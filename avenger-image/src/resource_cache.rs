use std::{
    collections::{HashMap, HashSet},
    num::NonZeroUsize,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

#[cfg(not(target_arch = "wasm32"))]
use avenger_resource::PrefetchRetargetPlanner;
use avenger_resource::{
    RenderInvalidationReason, RenderInvalidationRequest, RenderInvalidationSink, ResourceKey,
    ResourceRequest, ResourceRequestPurpose, ResourceSource,
};
#[cfg(target_arch = "wasm32")]
use std::collections::VecDeque;

use crate::{
    error::AvengerImageError, fetcher::ImageFetcher, ImageResourceLease, ImageResourceResolver,
    ImageResourceState, RgbaImage,
};

pub const IMAGE_RESOURCE_KIND: &str = "image";
pub const DEFAULT_IMAGE_RESOURCE_CACHE_CAPACITY: usize = 512;
#[cfg(target_arch = "wasm32")]
const WASM_MAX_CONCURRENT_IMAGE_LOADS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageResourceLoadOptions {
    pub timeout: Option<Duration>,
    pub poll_interval: Duration,
}

impl Default for ImageResourceLoadOptions {
    fn default() -> Self {
        Self {
            timeout: Some(Duration::from_secs(30)),
            poll_interval: Duration::from_millis(10),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ImageResourceLoadError {
    #[error("image resource(s) missing: {0:?}")]
    Missing(Vec<ResourceKey>),
    #[error("image resource(s) failed: {0:?}")]
    Failed(Vec<(ResourceKey, String)>),
    #[error("timed out waiting for image resource(s): {0:?}")]
    TimedOut(Vec<ResourceKey>),
}

#[derive(Debug, Clone)]
enum CachedImageState {
    Ready(Arc<RgbaImage>),
    Failed(Arc<str>),
}

#[derive(Clone)]
struct CompletedImage {
    state: CachedImageState,
    source: Option<ResourceSource>,
    loaded_at: Instant,
    purpose: ResourceRequestPurpose,
}

struct PendingImage {
    id: u64,
    source: ResourceSource,
    purpose: ResourceRequestPurpose,
    stale: Option<CompletedImage>,
    allow_stale: bool,
}

#[derive(Default)]
struct RetainedImage {
    leases: usize,
    completed: Option<CompletedImage>,
}

#[cfg(target_arch = "wasm32")]
struct WasmQueuedImageLoad {
    request: ResourceRequest,
    request_id: u64,
}

struct ImageResourceCacheInner {
    states: lru::LruCache<ResourceKey, CompletedImage>,
    pending: HashMap<ResourceKey, PendingImage>,
    retained: HashMap<ResourceKey, RetainedImage>,
    generation: u64,
    next_request_id: u64,
    render_invalidation_sink: Option<Arc<dyn RenderInvalidationSink>>,
    #[cfg(target_arch = "wasm32")]
    queued_image_loads: VecDeque<WasmQueuedImageLoad>,
    #[cfg(target_arch = "wasm32")]
    active_image_loads: usize,
}

impl Default for ImageResourceCacheInner {
    fn default() -> Self {
        Self::new(default_image_resource_cache_capacity())
    }
}

impl ImageResourceCacheInner {
    fn new(capacity: NonZeroUsize) -> Self {
        Self {
            states: lru::LruCache::new(capacity),
            pending: HashMap::new(),
            retained: HashMap::new(),
            generation: 0,
            next_request_id: 0,
            render_invalidation_sink: None,
            #[cfg(target_arch = "wasm32")]
            queued_image_loads: VecDeque::new(),
            #[cfg(target_arch = "wasm32")]
            active_image_loads: 0,
        }
    }

    fn take_completed(&mut self, key: &ResourceKey) -> Option<CompletedImage> {
        self.retained
            .get_mut(key)
            .and_then(|entry| entry.completed.take())
            .or_else(|| self.states.pop(key))
    }

    fn put_completed(&mut self, key: ResourceKey, completed: CompletedImage) {
        if let Some(entry) = self.retained.get_mut(&key) {
            entry.completed = Some(completed);
            return;
        }
        // Speculative results cannot displace a required result. Leased results
        // are outside this LRU and are retained until their last owner releases.
        if completed.purpose == ResourceRequestPurpose::Prefetch
            && self.states.len() == self.states.cap().get()
        {
            let victim = self
                .states
                .iter()
                .rev()
                .find(|(_, entry)| entry.purpose == ResourceRequestPurpose::Prefetch)
                .map(|(key, _)| key.clone());
            let Some(victim) = victim else {
                return;
            };
            self.states.pop(&victim);
        }
        self.states.put(key, completed);
    }

    fn next_request_id(&mut self) -> u64 {
        self.next_request_id = self.next_request_id.wrapping_add(1);
        self.next_request_id
    }
}

/// Shared image-resource cache that loads requested images away from render calls.
///
/// Pending loads are independent of the completed-image LRU. Required working
/// sets can be retained with `ImageResourceResolver::retain_images`, including
/// sets larger than the LRU capacity. Leases explicitly own that additional
/// memory. Unleased required results are evictable by other required results.
/// Prefetch cannot evict required results. Native and browser hosts use bounded
/// fetch concurrency. Request keys identify a source whose content may change:
/// freshness is evaluated when a request is submitted, never during a read.
#[derive(Clone, Default)]
pub struct ImageResourceCache {
    inner: Arc<Mutex<ImageResourceCacheInner>>,
    fetcher: Option<Arc<dyn ImageFetcher>>,
    #[cfg(not(target_arch = "wasm32"))]
    scheduler: Arc<std::sync::OnceLock<Arc<crate::scheduler::FetchScheduler>>>,
}

impl ImageResourceCache {
    pub fn new() -> Self {
        Self::default()
    }

    fn from_parts(
        inner: Arc<Mutex<ImageResourceCacheInner>>,
        fetcher: Option<Arc<dyn ImageFetcher>>,
    ) -> Self {
        Self {
            inner,
            fetcher,
            #[cfg(not(target_arch = "wasm32"))]
            scheduler: Arc::new(std::sync::OnceLock::new()),
        }
    }

    pub fn with_capacity(capacity: NonZeroUsize) -> Self {
        Self::from_parts(
            Arc::new(Mutex::new(ImageResourceCacheInner::new(capacity))),
            None,
        )
    }

    pub fn with_fetcher(fetcher: Arc<dyn ImageFetcher>) -> Self {
        Self::from_parts(
            Arc::new(Mutex::new(ImageResourceCacheInner::default())),
            Some(fetcher),
        )
    }

    pub fn with_fetcher_and_capacity(
        fetcher: Arc<dyn ImageFetcher>,
        capacity: NonZeroUsize,
    ) -> Self {
        Self::from_parts(
            Arc::new(Mutex::new(ImageResourceCacheInner::new(capacity))),
            Some(fetcher),
        )
    }

    /// The lazily-built fetch scheduler (native only). Lazy so short-lived
    /// caches (per-export SVG/PDF renders) never spawn worker threads
    /// unless they actually fetch.
    #[cfg(not(target_arch = "wasm32"))]
    fn scheduler(&self) -> Arc<crate::scheduler::FetchScheduler> {
        self.scheduler
            .get_or_init(|| {
                let execute_inner = self.inner.clone();
                let execute_fetcher = self.fetcher.clone();
                let cancel_inner = self.inner.clone();
                let begin_inner = self.inner.clone();
                crate::scheduler::FetchScheduler::new(crate::scheduler::SchedulerHooks {
                    execute: Box::new(move |request, request_id| {
                        let key = request.key.clone();
                        let fetcher = execute_fetcher.clone().or_else(shared_default_fetcher);
                        let state = match load_resource_image(&request, fetcher) {
                            Ok(image) => CachedImageState::Ready(Arc::new(image)),
                            Err(error) => CachedImageState::Failed(Arc::from(error.to_string())),
                        };
                        finish_image_load(execute_inner.clone(), key, request_id, state);
                    }),
                    cancel_pending: Box::new(move |key, request_id| {
                        cancel_pending_entry(&cancel_inner, key, request_id);
                    }),
                    begin_request: Box::new(move |request| {
                        try_begin_request(&begin_inner, request)
                    }),
                })
            })
            .clone()
    }

    pub fn with_render_invalidation_sink(self, sink: Arc<dyn RenderInvalidationSink>) -> Self {
        self.set_render_invalidation_sink(Some(sink));
        self
    }

    pub fn set_render_invalidation_sink(&self, sink: Option<Arc<dyn RenderInvalidationSink>>) {
        self.inner
            .lock()
            .expect("image resource cache lock poisoned")
            .render_invalidation_sink = sink;
    }

    pub fn request(&self, request: ResourceRequest) {
        self.request_image(&request);
    }

    pub fn pending_count(&self) -> usize {
        self.inner
            .lock()
            .expect("image resource cache lock poisoned")
            .pending
            .len()
    }

    #[cfg(test)]
    fn set_state_for_testing(&self, key: ResourceKey, state: CachedImageState) {
        let sink = {
            let mut inner = self
                .inner
                .lock()
                .expect("image resource cache lock poisoned");
            inner.put_completed(
                key,
                CompletedImage {
                    state,
                    source: None,
                    loaded_at: Instant::now(),
                    purpose: ResourceRequestPurpose::Required,
                },
            );
            inner.generation = inner.generation.wrapping_add(1);
            inner.render_invalidation_sink.clone()
        };
        request_image_render_invalidation(sink);
    }

    #[cfg(test)]
    fn len_for_testing(&self) -> usize {
        self.inner
            .lock()
            .expect("image resource cache lock poisoned")
            .states
            .len()
    }
}

fn default_image_resource_cache_capacity() -> NonZeroUsize {
    NonZeroUsize::new(DEFAULT_IMAGE_RESOURCE_CACHE_CAPACITY)
        .expect("default image resource cache capacity must be non-zero")
}

/// Load required images and retain them until the returned lease is dropped.
pub fn load_image_resource_requests_blocking(
    resolver: &dyn ImageResourceResolver,
    requests: &[ResourceRequest],
    options: ImageResourceLoadOptions,
) -> Result<ImageResourceLease, ImageResourceLoadError> {
    let mut keys = Vec::new();
    for request in requests.iter().filter(|request| {
        request.kind.0 == IMAGE_RESOURCE_KIND && request.purpose == ResourceRequestPurpose::Required
    }) {
        if !keys.contains(&request.key) {
            keys.push(request.key.clone());
        }
    }

    let lease = resolver.retain_images(&keys);
    for request in requests.iter().filter(|request| {
        request.kind.0 == IMAGE_RESOURCE_KIND && request.purpose == ResourceRequestPurpose::Required
    }) {
        resolver.request_image(request);
    }
    if keys.is_empty() {
        return Ok(lease);
    }

    let started = Instant::now();
    loop {
        let mut pending = Vec::new();
        let mut missing = Vec::new();
        let mut failed = Vec::new();

        for key in &keys {
            match resolver.image_state(key) {
                ImageResourceState::Ready(_) => {}
                ImageResourceState::Pending => pending.push(key.clone()),
                ImageResourceState::Missing => missing.push(key.clone()),
                ImageResourceState::Failed(error) => failed.push((key.clone(), error.to_string())),
            }
        }

        if !failed.is_empty() {
            return Err(ImageResourceLoadError::Failed(failed));
        }
        if !missing.is_empty() {
            return Err(ImageResourceLoadError::Missing(missing));
        }
        if pending.is_empty() {
            return Ok(lease);
        }
        if options
            .timeout
            .is_some_and(|timeout| started.elapsed() >= timeout)
        {
            return Err(ImageResourceLoadError::TimedOut(pending));
        }

        thread::sleep(options.poll_interval);
    }
}

impl ImageResourceResolver for ImageResourceCache {
    fn image_state(&self, key: &ResourceKey) -> ImageResourceState {
        let mut inner = self
            .inner
            .lock()
            .expect("image resource cache lock poisoned");
        if let Some(pending) = inner.pending.get(key) {
            return if pending.allow_stale {
                pending
                    .stale
                    .as_ref()
                    .map(|entry| public_state(&entry.state))
                    .unwrap_or(ImageResourceState::Pending)
            } else {
                ImageResourceState::Pending
            };
        }
        if let Some(completed) = inner
            .retained
            .get(key)
            .and_then(|entry| entry.completed.as_ref())
        {
            return public_state(&completed.state);
        }
        inner
            .states
            .get(key)
            .map(|entry| public_state(&entry.state))
            .unwrap_or(ImageResourceState::Missing)
    }

    fn retain_images(&self, keys: &[ResourceKey]) -> ImageResourceLease {
        let keys: HashSet<_> = keys.iter().cloned().collect();
        {
            let mut inner = self
                .inner
                .lock()
                .expect("image resource cache lock poisoned");
            for key in &keys {
                let completed = inner.states.pop(key);
                let entry = inner.retained.entry(key.clone()).or_default();
                entry.leases += 1;
                if completed.is_some() {
                    entry.completed = completed;
                }
            }
        }
        let weak = Arc::downgrade(&self.inner);
        ImageResourceLease::new(move || {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            let mut inner = inner.lock().expect("image resource cache lock poisoned");
            for key in keys {
                let Some(entry) = inner.retained.get_mut(&key) else {
                    continue;
                };
                entry.leases -= 1;
                if entry.leases == 0 {
                    let entry = inner.retained.remove(&key).expect("leased entry exists");
                    if let Some(completed) = entry.completed {
                        inner.put_completed(key, completed);
                        inner.generation = inner.generation.wrapping_add(1);
                    }
                }
            }
        })
    }

    fn request_image(&self, request: &ResourceRequest) {
        match try_begin_request(&self.inner, request) {
            Some(request_id) => {
                #[cfg(not(target_arch = "wasm32"))]
                self.scheduler().enqueue(request.clone(), request_id);
                #[cfg(target_arch = "wasm32")]
                spawn_image_load(
                    self.inner.clone(),
                    self.fetcher.clone(),
                    request.clone(),
                    request_id,
                );
            }
            None => {
                // Already pending or ready. A queued prefetch whose tile
                // just became visible gets promoted to Required ordering.
                #[cfg(not(target_arch = "wasm32"))]
                if request.purpose == ResourceRequestPurpose::Required {
                    let is_pending = self
                        .inner
                        .lock()
                        .expect("image resource cache lock poisoned")
                        .pending
                        .contains_key(&request.key);
                    if is_pending {
                        self.scheduler().promote_to_required(&request.key);
                    }
                }
                #[cfg(target_arch = "wasm32")]
                if request.purpose == ResourceRequestPurpose::Required
                    && promote_queued_wasm_image_load(&self.inner, request)
                {
                    pump_wasm_image_loads(self.inner.clone(), self.fetcher.clone());
                }
            }
        }
    }

    fn generation(&self) -> u64 {
        self.inner
            .lock()
            .expect("image resource cache lock poisoned")
            .generation
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn update_focus(&self, cursor_canvas_px: [f32; 2]) {
        self.scheduler().update_focus(cursor_canvas_px);
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn set_gesture_active(&self, active: bool) {
        self.scheduler().set_gesture_active(active);
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn install_retarget_planners(&self, planners: Vec<Arc<dyn PrefetchRetargetPlanner>>) {
        self.scheduler().install_retarget_planners(planners);
    }
}

fn public_state(state: &CachedImageState) -> ImageResourceState {
    match state {
        CachedImageState::Ready(image) => ImageResourceState::Ready(image.clone()),
        CachedImageState::Failed(error) => ImageResourceState::Failed(error.clone()),
    }
}

/// Deduplicate pending work, or start a load when the source changed or expired.
fn try_begin_request(
    inner: &Arc<Mutex<ImageResourceCacheInner>>,
    request: &ResourceRequest,
) -> Option<u64> {
    if request.kind.0 != IMAGE_RESOURCE_KIND {
        return None;
    }
    let mut inner = inner.lock().expect("image resource cache lock poisoned");
    let replaces_visible_pending = inner
        .pending
        .get(&request.key)
        .is_some_and(|pending| pending.allow_stale && pending.stale.is_some());
    if let Some(pending) = inner.pending.get_mut(&request.key) {
        if pending.source == request.source {
            if request.purpose == ResourceRequestPurpose::Required {
                pending.purpose = ResourceRequestPurpose::Required;
            }
            let hides_stale =
                pending.allow_stale && !request.cache_policy.allow_stale && pending.stale.is_some();
            pending.allow_stale &= request.cache_policy.allow_stale;
            if hides_stale {
                inner.generation = inner.generation.wrapping_add(1);
                let sink = inner.render_invalidation_sink.clone();
                drop(inner);
                request_image_render_invalidation(sink);
            }
            return None;
        }
    }
    let mut completed = inner.take_completed(&request.key);
    let had_visible_image = completed
        .as_ref()
        .is_some_and(|entry| matches!(entry.state, CachedImageState::Ready(_)));
    if let Some(entry) = &mut completed {
        let same_source = entry
            .source
            .as_ref()
            .is_none_or(|source| source == &request.source);
        let fresh = request
            .cache_policy
            .max_age_seconds
            .is_none_or(|age| entry.loaded_at.elapsed() < Duration::from_secs(age));
        if same_source && fresh && matches!(entry.state, CachedImageState::Ready(_)) {
            if request.purpose == ResourceRequestPurpose::Required {
                entry.purpose = ResourceRequestPurpose::Required;
            }
            inner.put_completed(
                request.key.clone(),
                completed.expect("completed image exists"),
            );
            return None;
        }
        if !same_source || !matches!(entry.state, CachedImageState::Ready(_)) {
            completed = None;
        }
    }
    let hides_completed =
        had_visible_image && (completed.is_none() || !request.cache_policy.allow_stale);
    let id = inner.next_request_id();
    inner.pending.insert(
        request.key.clone(),
        PendingImage {
            id,
            source: request.source.clone(),
            purpose: request.purpose,
            stale: completed,
            allow_stale: request.cache_policy.allow_stale,
        },
    );
    inner.generation = inner.generation.wrapping_add(1);
    let sink = inner.render_invalidation_sink.clone();
    drop(inner);
    if hides_completed || replaces_visible_pending {
        request_image_render_invalidation(sink);
    }
    Some(id)
}

/// Cancel only the matching speculative request. Restore its previous image
/// and notify rendering if cancellation makes hidden stale pixels visible again.
#[cfg(not(target_arch = "wasm32"))]
fn cancel_pending_entry(
    inner: &Arc<Mutex<ImageResourceCacheInner>>,
    key: &ResourceKey,
    request_id: u64,
) {
    let mut inner = inner.lock().expect("image resource cache lock poisoned");
    if inner.pending.get(key).is_some_and(|pending| {
        pending.id == request_id && pending.purpose == ResourceRequestPurpose::Prefetch
    }) {
        let pending = inner.pending.remove(key).expect("pending entry exists");
        let restores_visible_image = !pending.allow_stale && pending.stale.is_some();
        if let Some(stale) = pending.stale {
            inner.put_completed(key.clone(), stale);
        }
        inner.generation = inner.generation.wrapping_add(1);
        let sink = inner.render_invalidation_sink.clone();
        drop(inner);
        if restores_visible_image {
            request_image_render_invalidation(sink);
        }
    }
}

/// One process-wide default fetcher so concurrent tile fetches share a
/// single HTTP client (connection pooling) instead of constructing one
/// per request.
#[cfg(not(target_arch = "wasm32"))]
fn shared_default_fetcher() -> Option<Arc<dyn ImageFetcher>> {
    static FETCHER: std::sync::OnceLock<Option<Arc<dyn ImageFetcher>>> = std::sync::OnceLock::new();
    FETCHER
        .get_or_init(|| crate::fetcher::make_image_fetcher().ok())
        .clone()
}

fn finish_image_load(
    inner: Arc<Mutex<ImageResourceCacheInner>>,
    key: ResourceKey,
    request_id: u64,
    state: CachedImageState,
) {
    let sink = {
        let mut inner = inner.lock().expect("image resource cache lock poisoned");
        if inner
            .pending
            .get(&key)
            .is_some_and(|pending| pending.id == request_id)
        {
            let pending = inner.pending.remove(&key).expect("pending entry exists");
            inner.put_completed(
                key,
                CompletedImage {
                    state,
                    source: Some(pending.source),
                    loaded_at: Instant::now(),
                    purpose: pending.purpose,
                },
            );
            inner.generation = inner.generation.wrapping_add(1);
            inner.render_invalidation_sink.clone()
        } else {
            None
        }
    };
    request_image_render_invalidation(sink);
}

#[cfg(target_arch = "wasm32")]
fn spawn_image_load(
    inner: Arc<Mutex<ImageResourceCacheInner>>,
    fetcher: Option<Arc<dyn ImageFetcher>>,
    request: ResourceRequest,
    request_id: u64,
) {
    {
        let mut inner = inner.lock().expect("image resource cache lock poisoned");
        inner.queued_image_loads.push_back(WasmQueuedImageLoad {
            request,
            request_id,
        });
    }
    pump_wasm_image_loads(inner, fetcher);
}

#[cfg(target_arch = "wasm32")]
fn pump_wasm_image_loads(
    inner: Arc<Mutex<ImageResourceCacheInner>>,
    fetcher: Option<Arc<dyn ImageFetcher>>,
) {
    loop {
        let Some(entry) = next_wasm_image_load(&inner) else {
            return;
        };
        let inner_for_task = inner.clone();
        let fetcher_for_task = fetcher.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let key = entry.request.key.clone();
            let state =
                match load_resource_image_wasm(&entry.request, fetcher_for_task.clone()).await {
                    Ok(image) => CachedImageState::Ready(Arc::new(image)),
                    Err(error) => CachedImageState::Failed(Arc::from(error.to_string())),
                };
            finish_image_load(inner_for_task.clone(), key, entry.request_id, state);
            {
                let mut inner = inner_for_task
                    .lock()
                    .expect("image resource cache lock poisoned");
                inner.active_image_loads = inner.active_image_loads.saturating_sub(1);
            }
            pump_wasm_image_loads(inner_for_task, fetcher_for_task);
        });
    }
}

#[cfg(target_arch = "wasm32")]
fn next_wasm_image_load(
    inner: &Arc<Mutex<ImageResourceCacheInner>>,
) -> Option<WasmQueuedImageLoad> {
    let mut inner = inner.lock().expect("image resource cache lock poisoned");
    if inner.active_image_loads >= WASM_MAX_CONCURRENT_IMAGE_LOADS
        || inner.queued_image_loads.is_empty()
    {
        return None;
    }
    let best = inner
        .queued_image_loads
        .iter()
        .enumerate()
        .fold(0usize, |best, (index, entry)| {
            if wasm_image_load_beats(entry, &inner.queued_image_loads[best]) {
                index
            } else {
                best
            }
        });
    inner.active_image_loads += 1;
    inner.queued_image_loads.remove(best)
}

#[cfg(target_arch = "wasm32")]
fn wasm_image_load_beats(candidate: &WasmQueuedImageLoad, current: &WasmQueuedImageLoad) -> bool {
    let candidate_required = candidate.request.purpose == ResourceRequestPurpose::Required;
    let current_required = current.request.purpose == ResourceRequestPurpose::Required;
    if candidate_required != current_required {
        return candidate_required;
    }
    if candidate.request.priority != current.request.priority {
        return candidate.request.priority > current.request.priority;
    }
    candidate.request_id < current.request_id
}

#[cfg(target_arch = "wasm32")]
fn promote_queued_wasm_image_load(
    inner: &Arc<Mutex<ImageResourceCacheInner>>,
    request: &ResourceRequest,
) -> bool {
    let mut inner = inner.lock().expect("image resource cache lock poisoned");
    let Some(index) = inner
        .queued_image_loads
        .iter()
        .position(|entry| entry.request.key == request.key)
    else {
        return false;
    };
    let Some(mut entry) = inner.queued_image_loads.remove(index) else {
        return false;
    };
    entry.request = request.clone();
    entry.request.purpose = ResourceRequestPurpose::Required;
    inner.queued_image_loads.push_back(entry);
    true
}

fn request_image_render_invalidation(sink: Option<Arc<dyn RenderInvalidationSink>>) {
    if let Some(sink) = sink {
        sink.request_render(RenderInvalidationRequest::now(
            RenderInvalidationReason::ResourceChanged { kind: "image" },
        ));
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn load_resource_image(
    request: &ResourceRequest,
    fetcher: Option<Arc<dyn ImageFetcher>>,
) -> Result<RgbaImage, AvengerImageError> {
    match &request.source {
        ResourceSource::Url { url } => RgbaImage::from_str(url, fetcher),
        ResourceSource::DataUri { data_uri } => RgbaImage::from_str(data_uri, fetcher),
        ResourceSource::Opaque { provider, id } => Err(AvengerImageError::InternalError(format!(
            "unsupported opaque image resource: {provider}/{id}"
        ))),
    }
}

#[cfg(target_arch = "wasm32")]
async fn load_resource_image_wasm(
    request: &ResourceRequest,
    fetcher: Option<Arc<dyn ImageFetcher>>,
) -> Result<RgbaImage, AvengerImageError> {
    match &request.source {
        ResourceSource::Url { url } => {
            if let Some(fetcher) = fetcher {
                RgbaImage::from_str(url, Some(fetcher))
            } else {
                fetch_browser_image(url).await
            }
        }
        ResourceSource::DataUri { data_uri } => RgbaImage::from_str(data_uri, fetcher),
        ResourceSource::Opaque { provider, id } => Err(AvengerImageError::InternalError(format!(
            "unsupported opaque image resource: {provider}/{id}"
        ))),
    }
}

#[cfg(target_arch = "wasm32")]
async fn fetch_browser_image(url: &str) -> Result<RgbaImage, AvengerImageError> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let window = web_sys::window().ok_or_else(|| {
        AvengerImageError::InternalError("browser image fetch requires a window".to_string())
    })?;
    let response_value = JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(|error| js_value_error("failed to fetch image", error))?;
    let response = response_value
        .dyn_into::<web_sys::Response>()
        .map_err(|_| {
            AvengerImageError::InternalError(format!(
                "failed to fetch image {url}: response was not a Response"
            ))
        })?;
    if !response.ok() {
        return Err(AvengerImageError::InternalError(format!(
            "failed to fetch image {url}: HTTP {}",
            response.status()
        )));
    }
    let buffer = response
        .array_buffer()
        .map_err(|error| js_value_error("failed to read image response", error))?;
    let buffer = JsFuture::from(buffer)
        .await
        .map_err(|error| js_value_error("failed to read image response", error))?;
    let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
    let image = image::load_from_memory(&bytes)?;
    Ok(RgbaImage::from_image(&image.into_rgba8()))
}

#[cfg(target_arch = "wasm32")]
fn js_value_error(context: &str, value: wasm_bindgen::JsValue) -> AvengerImageError {
    let message = value.as_string().unwrap_or_else(|| format!("{value:?}"));
    AvengerImageError::InternalError(format!("{context}: {message}"))
}

#[cfg(test)]
mod tests {
    use std::{num::NonZeroUsize, sync::Mutex};

    use avenger_resource::{
        RenderInvalidationHub, RenderInvalidationReason, ResourceCachePolicy, ResourceKind,
    };

    use super::*;

    const TINY_PNG_DATA_URI: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAG0lEQVR4nGO4o6b2XzX59X8GscVe/3+dEf0PAE8fCXZKLiUkAAAAAElFTkSuQmCC";

    fn image_request(key: &str, source: ResourceSource) -> ResourceRequest {
        ResourceRequest {
            key: ResourceKey::new(key),
            kind: ResourceKind::new("image"),
            source,
            priority: 0.0,
            cache_policy: ResourceCachePolicy::default(),
            purpose: ResourceRequestPurpose::Required,
            screen_center: None,
            prefetch_scope: None,
        }
    }

    fn wait_for_image_state(
        cache: &ImageResourceCache,
        key: &ResourceKey,
        is_done: impl Fn(&ImageResourceState) -> bool,
    ) -> ImageResourceState {
        let start = Instant::now();
        loop {
            let state = cache.image_state(key);
            if is_done(&state) {
                return state;
            }
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn capacity(value: usize) -> NonZeroUsize {
        NonZeroUsize::new(value).expect("test cache capacity must be non-zero")
    }

    fn ready_state(red: u8) -> CachedImageState {
        CachedImageState::Ready(Arc::new(RgbaImage {
            width: 1,
            height: 1,
            data: vec![red, 0, 0, 255],
        }))
    }

    #[test]
    fn cache_reports_missing_pending_and_ready_states() {
        let cache = ImageResourceCache::new();
        let key = ResourceKey::new("tiny");
        assert!(matches!(
            cache.image_state(&key),
            ImageResourceState::Missing
        ));

        cache.request(image_request(
            "tiny",
            ResourceSource::DataUri {
                data_uri: TINY_PNG_DATA_URI.to_string(),
            },
        ));

        let start = Instant::now();
        let ready = loop {
            match cache.image_state(&key) {
                ImageResourceState::Ready(image) => break image,
                ImageResourceState::Failed(error) => panic!("image load failed: {error}"),
                ImageResourceState::Pending | ImageResourceState::Missing => {
                    assert!(start.elapsed() < Duration::from_secs(5));
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        };

        assert_eq!((ready.width, ready.height), (2, 2));
        assert!(cache.generation() >= 2);
    }

    #[test]
    fn cache_failed_state_increments_generation() {
        let cache = ImageResourceCache::new();
        let before = cache.generation();
        cache.set_state_for_testing(
            ResourceKey::new("bad"),
            CachedImageState::Failed(Arc::from("nope")),
        );
        assert!(cache.generation() > before);
        assert!(matches!(
            cache.image_state(&ResourceKey::new("bad")),
            ImageResourceState::Failed(_)
        ));
    }

    #[test]
    fn cache_evicts_least_recently_used_resource() {
        let cache = ImageResourceCache::with_capacity(capacity(2));
        cache.set_state_for_testing(ResourceKey::new("a"), ready_state(1));
        cache.set_state_for_testing(ResourceKey::new("b"), ready_state(2));
        assert_eq!(cache.len_for_testing(), 2);

        assert!(matches!(
            cache.image_state(&ResourceKey::new("a")),
            ImageResourceState::Ready(_)
        ));
        cache.set_state_for_testing(ResourceKey::new("c"), ready_state(3));

        assert!(matches!(
            cache.image_state(&ResourceKey::new("a")),
            ImageResourceState::Ready(_)
        ));
        assert!(matches!(
            cache.image_state(&ResourceKey::new("b")),
            ImageResourceState::Missing
        ));
        assert!(matches!(
            cache.image_state(&ResourceKey::new("c")),
            ImageResourceState::Ready(_)
        ));
        assert_eq!(cache.len_for_testing(), 2);
    }

    #[test]
    fn pending_load_survives_completed_cache_eviction() {
        let cache = ImageResourceCache::with_capacity(capacity(1));
        let request = image_request(
            "old",
            ResourceSource::DataUri {
                data_uri: TINY_PNG_DATA_URI.into(),
            },
        );
        let id = try_begin_request(&cache.inner, &request).unwrap();
        cache.set_state_for_testing(ResourceKey::new("new"), ready_state(2));
        assert!(matches!(
            cache.image_state(&request.key),
            ImageResourceState::Pending
        ));
        finish_image_load(cache.inner.clone(), request.key.clone(), id, ready_state(1));
        assert!(matches!(
            cache.image_state(&request.key),
            ImageResourceState::Ready(_)
        ));
    }

    fn data_request(key: &str) -> ResourceRequest {
        image_request(
            key,
            ResourceSource::DataUri {
                data_uri: TINY_PNG_DATA_URI.into(),
            },
        )
    }

    #[test]
    fn prefetch_cannot_displace_required_pending_or_ready_images() {
        let cache = ImageResourceCache::with_capacity(capacity(1));
        let required = data_request("required");
        let required_id = try_begin_request(&cache.inner, &required).unwrap();
        let prefetch = ResourceRequest {
            purpose: ResourceRequestPurpose::Prefetch,
            ..data_request("prefetch")
        };
        let prefetch_id = try_begin_request(&cache.inner, &prefetch).unwrap();
        assert_eq!(cache.pending_count(), 2);
        assert!(matches!(
            cache.image_state(&required.key),
            ImageResourceState::Pending
        ));
        finish_image_load(
            cache.inner.clone(),
            required.key.clone(),
            required_id,
            ready_state(1),
        );
        finish_image_load(
            cache.inner.clone(),
            prefetch.key.clone(),
            prefetch_id,
            ready_state(2),
        );
        assert!(matches!(
            cache.image_state(&required.key),
            ImageResourceState::Ready(_)
        ));
        assert!(matches!(
            cache.image_state(&prefetch.key),
            ImageResourceState::Missing
        ));
    }

    #[test]
    fn blocking_load_retains_a_working_set_larger_than_the_lru() {
        let cache = ImageResourceCache::with_capacity(capacity(1));
        let requests = [data_request("a"), data_request("b"), data_request("c")];
        let loaded =
            load_image_resource_requests_blocking(&cache, &requests, Default::default()).unwrap();
        for request in &requests {
            assert!(matches!(
                cache.image_state(&request.key),
                ImageResourceState::Ready(_)
            ));
        }
        let second_owner = cache.retain_images(&[requests[0].key.clone()]);
        drop(loaded);
        assert!(matches!(
            cache.image_state(&requests[0].key),
            ImageResourceState::Ready(_)
        ));
        assert!(cache.len_for_testing() <= 1);
        drop(second_owner);
        assert_eq!(cache.len_for_testing(), 1);
    }

    #[test]
    fn expiry_refreshes_and_strict_requests_hide_stale_images() {
        let hub = RenderInvalidationHub::default();
        let cache = ImageResourceCache::new().with_render_invalidation_sink(Arc::new(hub.clone()));
        let notifications = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let callback_notifications = notifications.clone();
        let callback_cache = cache.clone();
        let _subscription = hub.subscribe(Arc::new(move |_| {
            // The callback can read the cache, so notification must follow unlock.
            let _ = callback_cache.image_state(&ResourceKey::new("image"));
            callback_notifications.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }));
        let mut request = data_request("image");
        let id = try_begin_request(&cache.inner, &request).unwrap();
        finish_image_load(cache.inner.clone(), request.key.clone(), id, ready_state(1));
        assert!(try_begin_request(&cache.inner, &request).is_none());
        request.cache_policy.max_age_seconds = Some(0);
        let refresh_id = try_begin_request(&cache.inner, &request).unwrap();
        assert!(
            matches!(cache.image_state(&request.key), ImageResourceState::Ready(image) if image.data[0] == 1)
        );
        let notifications_before = notifications.load(std::sync::atomic::Ordering::SeqCst);
        let generation_before_hiding = cache.generation();
        request.cache_policy.allow_stale = false;
        assert!(try_begin_request(&cache.inner, &request).is_none());
        assert!(cache.generation() > generation_before_hiding);
        assert_eq!(
            notifications.load(std::sync::atomic::Ordering::SeqCst),
            notifications_before + 1
        );
        assert!(matches!(
            cache.image_state(&request.key),
            ImageResourceState::Pending
        ));
        finish_image_load(
            cache.inner.clone(),
            request.key.clone(),
            refresh_id,
            ready_state(2),
        );
        assert!(
            matches!(cache.image_state(&request.key), ImageResourceState::Ready(image) if image.data[0] == 2)
        );
        request.cache_policy.max_age_seconds = Some(3600);
        assert!(try_begin_request(&cache.inner, &request).is_none());
    }

    #[test]
    fn cancelled_strict_refresh_restores_the_retained_image() {
        let hub = RenderInvalidationHub::default();
        let cache = ImageResourceCache::new().with_render_invalidation_sink(Arc::new(hub.clone()));
        let mut request = data_request("image");
        request.purpose = ResourceRequestPurpose::Prefetch;
        let _lease = cache.retain_images(std::slice::from_ref(&request.key));
        let id = try_begin_request(&cache.inner, &request).unwrap();
        finish_image_load(cache.inner.clone(), request.key.clone(), id, ready_state(1));
        request.cache_policy.max_age_seconds = Some(0);
        request.cache_policy.allow_stale = false;
        let refresh = try_begin_request(&cache.inner, &request).unwrap();
        assert!(matches!(
            cache.image_state(&request.key),
            ImageResourceState::Pending
        ));
        let generation = cache.generation();
        let before = hub.epoch();
        cancel_pending_entry(&cache.inner, &request.key, refresh);
        assert!(cache.generation() > generation);
        assert!(hub.epoch() > before);
        assert!(
            matches!(cache.image_state(&request.key), ImageResourceState::Ready(image) if image.data[0] == 1)
        );
        finish_image_load(
            cache.inner.clone(),
            request.key.clone(),
            refresh,
            ready_state(2),
        );
        assert!(
            matches!(cache.image_state(&request.key), ImageResourceState::Ready(image) if image.data[0] == 1)
        );
    }

    #[test]
    fn source_changes_supersede_pending_work_and_refresh_failures_are_visible() {
        let cache = ImageResourceCache::new();
        let mut request = data_request("image");
        let old = try_begin_request(&cache.inner, &request).unwrap();
        request.source = ResourceSource::Url {
            url: "https://fixture.invalid/changed".into(),
        };
        let new = try_begin_request(&cache.inner, &request).unwrap();
        finish_image_load(
            cache.inner.clone(),
            request.key.clone(),
            old,
            ready_state(1),
        );
        assert!(matches!(
            cache.image_state(&request.key),
            ImageResourceState::Pending
        ));
        finish_image_load(
            cache.inner.clone(),
            request.key.clone(),
            new,
            ready_state(2),
        );
        assert!(
            matches!(cache.image_state(&request.key), ImageResourceState::Ready(image) if image.data[0] == 2)
        );
        request.cache_policy.max_age_seconds = Some(0);
        let refresh = try_begin_request(&cache.inner, &request).unwrap();
        finish_image_load(
            cache.inner.clone(),
            request.key.clone(),
            refresh,
            CachedImageState::Failed(Arc::from("offline")),
        );
        assert!(matches!(
            cache.image_state(&request.key),
            ImageResourceState::Failed(_)
        ));
    }

    #[test]
    fn cache_requests_render_when_data_uri_becomes_ready() {
        let hub = RenderInvalidationHub::default();
        let invalidations = Arc::new(Mutex::new(Vec::new()));
        let invalidations_callback = invalidations.clone();
        let _subscription = hub.subscribe(Arc::new(move |invalidation| {
            invalidations_callback
                .lock()
                .expect("invalidations lock poisoned")
                .push(invalidation);
        }));
        let cache = ImageResourceCache::new().with_render_invalidation_sink(Arc::new(hub.clone()));
        let key = ResourceKey::new("tiny");

        cache.request(image_request(
            "tiny",
            ResourceSource::DataUri {
                data_uri: TINY_PNG_DATA_URI.to_string(),
            },
        ));

        let state = wait_for_image_state(&cache, &key, |state| {
            matches!(state, ImageResourceState::Ready(_))
        });
        assert!(matches!(state, ImageResourceState::Ready(_)));

        let invalidations = invalidations.lock().expect("invalidations lock poisoned");
        assert_eq!(invalidations.len(), 1);
        assert!(matches!(
            invalidations[0].reason,
            RenderInvalidationReason::ResourceChanged { kind: "image" }
        ));
    }

    #[test]
    fn cache_requests_render_when_resource_fails() {
        let hub = RenderInvalidationHub::default();
        let invalidations = Arc::new(Mutex::new(Vec::new()));
        let invalidations_callback = invalidations.clone();
        let _subscription = hub.subscribe(Arc::new(move |invalidation| {
            invalidations_callback
                .lock()
                .expect("invalidations lock poisoned")
                .push(invalidation);
        }));
        let cache = ImageResourceCache::new().with_render_invalidation_sink(Arc::new(hub.clone()));
        let key = ResourceKey::new("bad");

        cache.request(image_request(
            "bad",
            ResourceSource::Opaque {
                provider: "test".to_string(),
                id: "bad".to_string(),
            },
        ));

        let state = wait_for_image_state(&cache, &key, |state| {
            matches!(state, ImageResourceState::Failed(_))
        });
        assert!(matches!(state, ImageResourceState::Failed(_)));

        let invalidations = invalidations.lock().expect("invalidations lock poisoned");
        assert_eq!(invalidations.len(), 1);
    }

    #[test]
    fn cache_render_invalidation_callback_can_query_image_state() {
        let hub = RenderInvalidationHub::default();
        let cache = ImageResourceCache::new().with_render_invalidation_sink(Arc::new(hub.clone()));
        let key = ResourceKey::new("tiny");
        let cache_for_callback = cache.clone();
        let key_for_callback = key.clone();
        let callback_count = Arc::new(Mutex::new(0usize));
        let callback_count_for_callback = callback_count.clone();
        let _subscription = hub.subscribe(Arc::new(move |_| {
            let _state = cache_for_callback.image_state(&key_for_callback);
            *callback_count_for_callback
                .lock()
                .expect("callback count lock poisoned") += 1;
        }));

        cache.request(image_request(
            "tiny",
            ResourceSource::DataUri {
                data_uri: TINY_PNG_DATA_URI.to_string(),
            },
        ));

        let state = wait_for_image_state(&cache, &key, |state| {
            matches!(state, ImageResourceState::Ready(_))
        });
        assert!(matches!(state, ImageResourceState::Ready(_)));
        assert_eq!(
            *callback_count.lock().expect("callback count lock poisoned"),
            1
        );
    }

    #[test]
    fn blocking_loader_loads_data_uri_request() {
        let cache = ImageResourceCache::new();
        let request = image_request(
            "tiny",
            ResourceSource::DataUri {
                data_uri: TINY_PNG_DATA_URI.to_string(),
            },
        );

        let _loaded = load_image_resource_requests_blocking(
            &cache,
            &[request],
            ImageResourceLoadOptions {
                timeout: Some(Duration::from_secs(5)),
                poll_interval: Duration::from_millis(5),
            },
        )
        .unwrap();

        assert!(matches!(
            cache.image_state(&ResourceKey::new("tiny")),
            ImageResourceState::Ready(_)
        ));
    }

    #[test]
    fn blocking_loader_reports_failed_resource() {
        let cache = ImageResourceCache::new();
        let request = image_request(
            "bad",
            ResourceSource::Opaque {
                provider: "test".to_string(),
                id: "bad".to_string(),
            },
        );

        let err = load_image_resource_requests_blocking(
            &cache,
            &[request],
            ImageResourceLoadOptions {
                timeout: Some(Duration::from_secs(5)),
                poll_interval: Duration::from_millis(5),
            },
        )
        .unwrap_err();

        assert!(matches!(err, ImageResourceLoadError::Failed(failed) if failed.len() == 1));
    }

    #[test]
    fn blocking_loader_times_out_pending_resource() {
        let resolver = AlwaysPendingResolver::default();
        let request = image_request(
            "slow",
            ResourceSource::DataUri {
                data_uri: TINY_PNG_DATA_URI.to_string(),
            },
        );

        let err = load_image_resource_requests_blocking(
            &resolver,
            &[request],
            ImageResourceLoadOptions {
                timeout: Some(Duration::from_millis(1)),
                poll_interval: Duration::from_millis(1),
            },
        )
        .unwrap_err();

        assert!(
            matches!(err, ImageResourceLoadError::TimedOut(keys) if keys == vec![ResourceKey::new("slow")])
        );
        assert_eq!(resolver.requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn blocking_loader_reports_missing_resource() {
        let resolver = AlwaysMissingResolver;
        let request = image_request(
            "missing",
            ResourceSource::DataUri {
                data_uri: TINY_PNG_DATA_URI.to_string(),
            },
        );

        let err = load_image_resource_requests_blocking(
            &resolver,
            &[request],
            ImageResourceLoadOptions::default(),
        )
        .unwrap_err();

        assert!(
            matches!(err, ImageResourceLoadError::Missing(keys) if keys == vec![ResourceKey::new("missing")])
        );
    }

    struct AlwaysMissingResolver;

    impl ImageResourceResolver for AlwaysMissingResolver {
        fn image_state(&self, _key: &ResourceKey) -> ImageResourceState {
            ImageResourceState::Missing
        }

        fn request_image(&self, _request: &ResourceRequest) {}
    }

    #[test]
    fn blocking_loader_ignores_non_image_resources() {
        let resolver = AlwaysPendingResolver::default();
        let request = ResourceRequest {
            key: ResourceKey::new("other"),
            kind: ResourceKind::new("json"),
            source: ResourceSource::Opaque {
                provider: "test".to_string(),
                id: "other".to_string(),
            },
            priority: 0.0,
            cache_policy: ResourceCachePolicy::default(),
            purpose: ResourceRequestPurpose::Required,
            screen_center: None,
            prefetch_scope: None,
        };

        let _loaded = load_image_resource_requests_blocking(
            &resolver,
            &[request],
            ImageResourceLoadOptions::default(),
        )
        .unwrap();

        assert!(resolver.requests.lock().unwrap().is_empty());
    }

    #[test]
    fn blocking_loader_ignores_prefetch_image_resources() {
        let resolver = AlwaysPendingResolver::default();
        let request = ResourceRequest {
            key: ResourceKey::new("prefetch"),
            kind: ResourceKind::new("image"),
            source: ResourceSource::DataUri {
                data_uri: TINY_PNG_DATA_URI.to_string(),
            },
            priority: -1.0,
            cache_policy: ResourceCachePolicy::default(),
            purpose: ResourceRequestPurpose::Prefetch,
            screen_center: None,
            prefetch_scope: None,
        };

        let _loaded = load_image_resource_requests_blocking(
            &resolver,
            &[request],
            ImageResourceLoadOptions::default(),
        )
        .unwrap();

        assert!(resolver.requests.lock().unwrap().is_empty());
    }

    #[derive(Default)]
    struct AlwaysPendingResolver {
        requests: Mutex<Vec<ResourceKey>>,
    }

    impl ImageResourceResolver for AlwaysPendingResolver {
        fn image_state(&self, _key: &ResourceKey) -> ImageResourceState {
            ImageResourceState::Pending
        }

        fn request_image(&self, request: &ResourceRequest) {
            self.requests.lock().unwrap().push(request.key.clone());
        }
    }
}
