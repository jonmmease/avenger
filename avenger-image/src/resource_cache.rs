use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use avenger_resource::{
    RenderInvalidationReason, RenderInvalidationRequest, RenderInvalidationSink, ResourceKey,
    ResourceRequest, ResourceSource,
};

use crate::{
    error::AvengerImageError, fetcher::ImageFetcher, ImageResourceResolver, ImageResourceState,
    RgbaImage,
};

pub const IMAGE_RESOURCE_KIND: &str = "image";

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
    Pending,
    Ready(Arc<RgbaImage>),
    Failed(Arc<str>),
}

#[derive(Default)]
struct ImageResourceCacheInner {
    states: HashMap<ResourceKey, CachedImageState>,
    generation: u64,
    render_invalidation_sink: Option<Arc<dyn RenderInvalidationSink>>,
}

/// Shared image-resource cache that loads requested images away from render calls.
///
/// Native builds use a background thread per uncached request. Browser hosts can
/// still use the same `ImageResourceResolver` trait with their own wasm fetcher
/// until this cache grows a wasm-specific executor.
#[derive(Clone, Default)]
pub struct ImageResourceCache {
    inner: Arc<Mutex<ImageResourceCacheInner>>,
    fetcher: Option<Arc<dyn ImageFetcher>>,
}

impl ImageResourceCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_fetcher(fetcher: Arc<dyn ImageFetcher>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ImageResourceCacheInner::default())),
            fetcher: Some(fetcher),
        }
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
            .states
            .values()
            .filter(|state| matches!(state, CachedImageState::Pending))
            .count()
    }

    #[cfg(test)]
    fn set_state_for_testing(&self, key: ResourceKey, state: CachedImageState) {
        let sink = {
            let mut inner = self
                .inner
                .lock()
                .expect("image resource cache lock poisoned");
            inner.states.insert(key, state);
            inner.generation = inner.generation.wrapping_add(1);
            inner.render_invalidation_sink.clone()
        };
        request_image_render_invalidation(sink);
    }
}

pub fn load_image_resource_requests_blocking(
    resolver: &dyn ImageResourceResolver,
    requests: &[ResourceRequest],
    options: ImageResourceLoadOptions,
) -> Result<(), ImageResourceLoadError> {
    let mut keys = Vec::new();
    for request in requests
        .iter()
        .filter(|request| request.kind.0 == IMAGE_RESOURCE_KIND)
    {
        resolver.request_image(request);
        if !keys.contains(&request.key) {
            keys.push(request.key.clone());
        }
    }

    if keys.is_empty() {
        return Ok(());
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
            return Ok(());
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
        self.inner
            .lock()
            .expect("image resource cache lock poisoned")
            .states
            .get(key)
            .map(|state| match state {
                CachedImageState::Pending => ImageResourceState::Pending,
                CachedImageState::Ready(image) => ImageResourceState::Ready(image.clone()),
                CachedImageState::Failed(error) => ImageResourceState::Failed(error.clone()),
            })
            .unwrap_or(ImageResourceState::Missing)
    }

    fn request_image(&self, request: &ResourceRequest) {
        let should_spawn = {
            let mut inner = self
                .inner
                .lock()
                .expect("image resource cache lock poisoned");
            if matches!(
                inner.states.get(&request.key),
                Some(CachedImageState::Pending | CachedImageState::Ready(_))
            ) {
                false
            } else {
                inner
                    .states
                    .insert(request.key.clone(), CachedImageState::Pending);
                inner.generation = inner.generation.wrapping_add(1);
                true
            }
        };

        if should_spawn {
            spawn_image_load(self.inner.clone(), self.fetcher.clone(), request.clone());
        }
    }

    fn generation(&self) -> u64 {
        self.inner
            .lock()
            .expect("image resource cache lock poisoned")
            .generation
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn_image_load(
    inner: Arc<Mutex<ImageResourceCacheInner>>,
    fetcher: Option<Arc<dyn ImageFetcher>>,
    request: ResourceRequest,
) {
    std::thread::spawn(move || {
        let key = request.key.clone();
        let state = match load_resource_image(&request, fetcher) {
            Ok(image) => CachedImageState::Ready(Arc::new(image)),
            Err(error) => CachedImageState::Failed(Arc::from(error.to_string())),
        };
        let sink = {
            let mut inner = inner.lock().expect("image resource cache lock poisoned");
            inner.states.insert(key, state);
            inner.generation = inner.generation.wrapping_add(1);
            inner.render_invalidation_sink.clone()
        };
        request_image_render_invalidation(sink);
    });
}

#[cfg(target_arch = "wasm32")]
fn spawn_image_load(
    inner: Arc<Mutex<ImageResourceCacheInner>>,
    _fetcher: Option<Arc<dyn ImageFetcher>>,
    request: ResourceRequest,
) {
    let sink = {
        let mut inner = inner.lock().expect("image resource cache lock poisoned");
        inner.states.insert(
            request.key,
            CachedImageState::Failed(Arc::from(
                "ImageResourceCache does not provide a wasm image loader; supply a browser resolver",
            )),
        );
        inner.generation = inner.generation.wrapping_add(1);
        inner.render_invalidation_sink.clone()
    };
    request_image_render_invalidation(sink);
}

fn request_image_render_invalidation(sink: Option<Arc<dyn RenderInvalidationSink>>) {
    if let Some(sink) = sink {
        sink.request_render(RenderInvalidationRequest::now(
            RenderInvalidationReason::ResourceChanged { kind: "image" },
        ));
    }
}

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

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

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

        load_image_resource_requests_blocking(
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
        };

        load_image_resource_requests_blocking(
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
