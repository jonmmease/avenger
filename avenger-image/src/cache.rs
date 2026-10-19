//! A resolver that fetches images in the background and keeps them in memory.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

#[cfg(not(target_arch = "wasm32"))]
use crate::disk::{DiskCache, DiskCacheOptions};
#[cfg(not(target_arch = "wasm32"))]
use crate::fetcher::{default_image_fetcher, FetchResponse, ImageFetcher, Validators};
use crate::resolver::{ImageLease, ImageResolver, ImageState};
use crate::RgbaImage;

/// The first wait before a failed image is fetched again. Each failure in a row doubles it.
const FIRST_RETRY: Duration = Duration::from_secs(1);
/// The longest wait before a failed image is fetched again, and the wait after a response that
/// the server won't change soon: a 4xx status, or bytes that don't decode.
const LAST_RETRY: Duration = Duration::from_secs(300);

/// How an [`ImageCache`] fetches and how much it keeps.
#[derive(Clone)]
pub struct ImageCacheOptions {
    /// The bytes of decoded images that the cache keeps beyond the leased ones. 128 MiB by
    /// default.
    pub budget: usize,
    /// The most images that fetch at once: 16 by default on native targets, each on its own
    /// thread, and 6 in the browser.
    pub max_fetches: usize,
    /// The fetcher, [`default_image_fetcher`] by default. The browser fetches with its own
    /// `fetch`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fetcher: Arc<dyn ImageFetcher>,
    /// The disk cache of fetched responses, none by default. The browser keeps its own HTTP
    /// cache.
    #[cfg(not(target_arch = "wasm32"))]
    pub disk: Option<DiskCacheOptions>,
}

impl Default for ImageCacheOptions {
    fn default() -> Self {
        Self {
            budget: 128 << 20,
            max_fetches: if cfg!(target_arch = "wasm32") { 6 } else { 16 },
            #[cfg(not(target_arch = "wasm32"))]
            fetcher: default_image_fetcher(),
            #[cfg(not(target_arch = "wasm32"))]
            disk: None,
        }
    }
}

/// An [`ImageResolver`] that fetches images in the background and keeps them in memory.
///
/// The cache fetches on a bounded pool: threads on native targets that start when it has
/// requests and exit when it has none, and the browser's `fetch` on the web. Images that
/// [`ImageResolver::image`] requests start before images that [`ImageResolver::prefetch`]
/// requests. Each image fetches once while the cache holds it.
///
/// Decoded images stay in memory within a byte budget, and the least recently used ones go
/// first. Leased images don't count toward the budget, and stay until their last lease drops.
/// When the last lease that holds an image drops, a fetch for it that hasn't started is dropped,
/// so a view that moves on doesn't wait for images it no longer shows.
///
/// A failed image fails until a wait passes: 1 second after its first failure, doubling with
/// each failure in a row to 5 minutes. A 4xx response, or bytes that don't decode, waits 5
/// minutes at once. The next request after the wait fetches it again.
///
/// On native targets, a disk cache can keep the fetched responses as their HTTP caching headers
/// allow, so a later run draws them without fetching them again. A fresh response draws without
/// a request. A stale one draws at once while its conditional request runs as a prefetch, unless
/// its headers say it must wait for that request. See [`DiskCacheOptions`].
///
/// Clones share their images.
#[derive(Clone)]
pub struct ImageCache {
    shared: Arc<Shared>,
}

pub(crate) struct Shared {
    state: Mutex<State>,
    on_change: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    options: ImageCacheOptions,
    #[cfg(not(target_arch = "wasm32"))]
    disk: Option<DiskCache>,
}

impl ImageCache {
    /// A cache with the default options.
    pub fn new() -> Self {
        Self::with_options(ImageCacheOptions::default())
    }

    pub fn with_options(options: ImageCacheOptions) -> Self {
        Self {
            shared: Arc::new(Shared {
                state: Mutex::new(State::default()),
                on_change: Mutex::new(None),
                #[cfg(not(target_arch = "wasm32"))]
                disk: options.disk.as_ref().map(DiskCache::new),
                options,
            }),
        }
    }

    /// Run `callback` when an image arrives, changes, or fails, outside the cache's lock, on the
    /// thread that fetched it. This replaces the previous callback.
    pub fn set_on_change(&self, callback: impl Fn() + Send + Sync + 'static) {
        *lock(&self.shared.on_change) = Some(Arc::new(callback));
    }

    fn request(&self, url: &str, class: Class) -> ImageState {
        let state = self.shared.state().request(url, class);
        if matches!(state, ImageState::Pending) {
            self.shared.pump();
        }
        state
    }
}

impl Default for ImageCache {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for ImageCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.shared.state();
        f.debug_struct("ImageCache")
            .field("images", &state.entries.len())
            .field("bytes", &state.bytes)
            .finish_non_exhaustive()
    }
}

impl ImageResolver for ImageCache {
    fn image(&self, url: &str) -> ImageState {
        self.request(url, Class::Visible)
    }

    fn prefetch(&self, url: &str) {
        self.request(url, Class::Prefetch);
    }

    fn ready(&self, url: &str) -> Option<Arc<RgbaImage>> {
        let mut state = self.shared.state();
        let image = state.entries.get(url)?.status.image()?;
        state.touch(url);
        Some(image)
    }

    fn retain(&self, urls: &[&str]) -> ImageLease {
        let urls: Vec<String> = urls
            .iter()
            .copied()
            .collect::<HashSet<_>>()
            .into_iter()
            .map(str::to_string)
            .collect();
        self.shared.state().retain(&urls);
        ImageLease::new(Release {
            shared: Arc::downgrade(&self.shared),
            urls,
        })
    }
}

/// Releases a lease's images when the lease drops.
struct Release {
    shared: Weak<Shared>,
    urls: Vec<String>,
}

impl Drop for Release {
    fn drop(&mut self) {
        if let Some(shared) = self.shared.upgrade() {
            shared.state().release(&self.urls, shared.options.budget);
        }
    }
}

/// The two request classes: images a view draws now, and images it may draw soon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Visible = 0,
    Prefetch = 1,
}

/// What a fetch job does with a URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Job {
    /// Fetch an image that doesn't draw yet.
    Fetch,
    /// Revalidate the stale stored response that an image drew from.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Revalidate,
}

/// The revalidation of a ready image that drew from a stale stored response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Revalidation {
    Idle,
    Queued,
    Running,
}

/// Why a fetch failed, and whether the failure will last.
pub(crate) struct Failure {
    pub(crate) message: Arc<str>,
    /// Whether the server won't change its answer soon, so the image waits the longest retry.
    pub(crate) lasting: bool,
}

impl Failure {
    pub(crate) fn passing(message: impl fmt::Display) -> Self {
        Self {
            message: message.to_string().into(),
            lasting: false,
        }
    }

    pub(crate) fn lasting(message: impl fmt::Display) -> Self {
        Self {
            message: message.to_string().into(),
            lasting: true,
        }
    }
}

#[derive(Default)]
struct State {
    entries: HashMap<String, Entry>,
    /// The images that no lease holds, oldest use first, by the tick of their last use.
    recency: BTreeMap<u64, String>,
    tick: u64,
    /// The bytes of the images in `recency`.
    bytes: usize,
    /// The queued URLs of each class. A URL whose entry no longer waits in that class is skipped
    /// when it comes up.
    queues: [VecDeque<String>; 2],
    /// The number of URLs that wait in each class.
    queued: [usize; 2],
    /// The jobs that run.
    fetching: usize,
    /// The threads that run jobs, on native targets.
    #[cfg(not(target_arch = "wasm32"))]
    workers: usize,
}

struct Entry {
    status: Status,
    leases: usize,
    /// The entry's tick in `recency`, when it's there.
    tick: Option<u64>,
    /// The failures in a row, which set the wait before the next fetch.
    failures: u32,
    revalidation: Revalidation,
}

impl Entry {
    fn new() -> Self {
        Self {
            status: Status::Unrequested,
            leases: 0,
            tick: None,
            failures: 0,
            revalidation: Revalidation::Idle,
        }
    }

    /// Whether the entry waits in `class`'s queue.
    fn waits_in(&self, class: Class) -> bool {
        match self.status {
            Status::Queued(queued) => queued == class,
            Status::Ready(_) => {
                class == Class::Prefetch && self.revalidation == Revalidation::Queued
            }
            _ => false,
        }
    }
}

enum Status {
    /// A lease holds the URL, and nothing has requested it.
    Unrequested,
    Queued(Class),
    Fetching,
    Ready(Arc<RgbaImage>),
    Failed {
        message: Arc<str>,
        retry_at: Instant,
    },
}

impl Status {
    fn image(&self) -> Option<Arc<RgbaImage>> {
        match self {
            Status::Ready(image) => Some(image.clone()),
            _ => None,
        }
    }

    /// The bytes that the entry counts toward the budget while it's in `recency`.
    fn bytes(&self, url: &str) -> usize {
        match self {
            Status::Ready(image) => image.data.len(),
            Status::Failed { message, .. } => url.len() + message.len(),
            _ => 0,
        }
    }
}

impl State {
    fn request(&mut self, url: &str, class: Class) -> ImageState {
        let entry = self
            .entries
            .entry(url.to_string())
            .or_insert_with(Entry::new);
        match &entry.status {
            Status::Ready(image) => {
                let image = image.clone();
                self.touch(url);
                return ImageState::Ready(image);
            }
            Status::Failed { message, retry_at } if Instant::now() < *retry_at => {
                return ImageState::Failed(message.clone());
            }
            Status::Fetching | Status::Queued(Class::Visible) => return ImageState::Pending,
            Status::Queued(Class::Prefetch) if class == Class::Prefetch => {
                return ImageState::Pending
            }
            // The prefetch's place in its queue is skipped when it comes up.
            Status::Queued(Class::Prefetch) => self.queued[Class::Prefetch as usize] -= 1,
            Status::Unrequested | Status::Failed { .. } => {}
        }
        // A failure whose wait passed leaves `recency` while it fetches again.
        self.forget(url);
        if let Some(entry) = self.entries.get_mut(url) {
            entry.status = Status::Queued(class);
        }
        self.enqueue(url, class);
        ImageState::Pending
    }

    fn enqueue(&mut self, url: &str, class: Class) {
        self.queues[class as usize].push_back(url.to_string());
        self.queued[class as usize] += 1;
        // Drop the queue's skipped places once they outnumber its waiting ones, so a queue that
        // requests keep replacing stays as long as its requests.
        let queue = &mut self.queues[class as usize];
        if queue.len() > 64 && queue.len() > 2 * self.queued[class as usize] {
            let entries = &self.entries;
            let mut seen = HashSet::new();
            queue.retain(|url| {
                entries.get(url).is_some_and(|entry| entry.waits_in(class))
                    && seen.insert(url.clone())
            });
        }
    }

    /// The next job, visible requests first, marked as running. Revalidations run in the
    /// prefetch class.
    fn next(&mut self) -> Option<(String, Job)> {
        for class in [Class::Visible, Class::Prefetch] {
            while let Some(url) = self.queues[class as usize].pop_front() {
                let Some(entry) = self.entries.get_mut(&url) else {
                    continue;
                };
                if !entry.waits_in(class) {
                    continue;
                }
                let job = if matches!(entry.status, Status::Ready(_)) {
                    entry.revalidation = Revalidation::Running;
                    Job::Revalidate
                } else {
                    entry.status = Status::Fetching;
                    Job::Fetch
                };
                self.queued[class as usize] -= 1;
                self.fetching += 1;
                return Some((url, job));
            }
        }
        None
    }

    /// Record a fetch's result. An image that drew from a stale stored response queues its
    /// revalidation.
    fn finish(
        &mut self,
        url: String,
        result: Result<Arc<RgbaImage>, Failure>,
        stale: bool,
        budget: usize,
    ) {
        self.fetching -= 1;
        let Some(entry) = self.entries.get_mut(&url) else {
            return;
        };
        let revalidate = stale && result.is_ok();
        entry.status = match result {
            Ok(image) => {
                entry.failures = 0;
                Status::Ready(image)
            }
            Err(failure) => {
                entry.failures = entry.failures.saturating_add(1);
                let wait = if failure.lasting {
                    LAST_RETRY
                } else {
                    FIRST_RETRY
                        .saturating_mul(1 << (entry.failures - 1).min(16))
                        .min(LAST_RETRY)
                };
                Status::Failed {
                    message: failure.message,
                    retry_at: Instant::now() + wait,
                }
            }
        };
        if revalidate {
            entry.revalidation = Revalidation::Queued;
        }
        let unleased = entry.leases == 0;
        if revalidate {
            self.enqueue(&url, Class::Prefetch);
        }
        if unleased {
            self.remember(&url);
            self.evict(budget);
        }
    }

    /// Record a revalidation's result: the image of a changed response, or none when the stored
    /// response still holds or no new response arrived. Returns whether the image changed.
    #[cfg(not(target_arch = "wasm32"))]
    fn finish_revalidation(
        &mut self,
        url: &str,
        image: Option<Arc<RgbaImage>>,
        budget: usize,
    ) -> bool {
        self.fetching -= 1;
        let Some(entry) = self.entries.get_mut(url) else {
            return false;
        };
        entry.revalidation = Revalidation::Idle;
        let Some(image) = image else {
            return false;
        };
        let remembered = entry.tick.is_some();
        self.forget(url);
        if let Some(entry) = self.entries.get_mut(url) {
            entry.status = Status::Ready(image);
        }
        if remembered {
            self.remember(url);
            self.evict(budget);
        }
        true
    }

    fn retain(&mut self, urls: &[String]) {
        for url in urls {
            let entry = self.entries.entry(url.clone()).or_insert_with(Entry::new);
            entry.leases += 1;
            self.forget(url);
        }
    }

    fn release(&mut self, urls: &[String], budget: usize) {
        for url in urls {
            let Some(entry) = self.entries.get_mut(url) else {
                continue;
            };
            entry.leases -= 1;
            if entry.leases > 0 {
                continue;
            }
            match entry.status {
                Status::Unrequested => {
                    self.entries.remove(url);
                }
                Status::Queued(class) => {
                    self.queued[class as usize] -= 1;
                    self.entries.remove(url);
                }
                // A fetch that runs keeps its entry, which joins `recency` when it finishes.
                Status::Fetching => {}
                Status::Ready(_) | Status::Failed { .. } => self.remember(url),
            }
        }
        self.evict(budget);
    }

    /// Mark an entry in `recency` as just used.
    fn touch(&mut self, url: &str) {
        if self
            .entries
            .get(url)
            .is_some_and(|entry| entry.tick.is_some())
        {
            self.forget(url);
            self.remember(url);
        }
    }

    /// Add an unleased, finished entry to `recency` as just used.
    fn remember(&mut self, url: &str) {
        self.tick += 1;
        let tick = self.tick;
        if let Some(entry) = self.entries.get_mut(url) {
            if entry.tick.is_none() {
                entry.tick = Some(tick);
                self.bytes += entry.status.bytes(url);
                self.recency.insert(tick, url.to_string());
            }
        }
    }

    /// Remove an entry from `recency`, when it's there.
    fn forget(&mut self, url: &str) {
        if let Some(entry) = self.entries.get_mut(url) {
            if let Some(tick) = entry.tick.take() {
                self.bytes -= entry.status.bytes(url);
                self.recency.remove(&tick);
            }
        }
    }

    /// Remove the least recently used images until the rest fit the budget.
    fn evict(&mut self, budget: usize) {
        while self.bytes > budget {
            let Some((_, url)) = self.recency.pop_first() else {
                break;
            };
            if let Some(entry) = self.entries.remove(&url) {
                self.bytes -= entry.status.bytes(&url);
                if entry.revalidation == Revalidation::Queued {
                    self.queued[Class::Prefetch as usize] -= 1;
                }
            }
        }
    }
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }

    /// Record a fetch's result and run the change callback.
    fn finish(&self, url: String, result: Result<Arc<RgbaImage>, Failure>, stale: bool) {
        self.state().finish(url, result, stale, self.options.budget);
        self.changed();
    }

    /// Run the change callback, outside the lock.
    fn changed(&self) {
        let callback = lock(&self.on_change).clone();
        if let Some(callback) = callback {
            callback();
        }
    }

    /// Start a thread for each waiting job that no idle thread will take, within the limit.
    #[cfg(not(target_arch = "wasm32"))]
    fn pump(self: &Arc<Self>) {
        let mut state = self.state();
        while state.workers < self.options.max_fetches
            && state.queued[0] + state.queued[1] > state.workers - state.fetching
        {
            state.workers += 1;
            let shared = self.clone();
            let spawned = std::thread::Builder::new()
                .name("avenger-image-fetch".to_string())
                .spawn(move || shared.work());
            if spawned.is_err() {
                state.workers -= 1;
                break;
            }
        }
    }

    /// Run waiting jobs until none is left, then exit.
    #[cfg(not(target_arch = "wasm32"))]
    fn work(self: Arc<Self>) {
        loop {
            let (url, job) = {
                let mut state = self.state();
                match state.next() {
                    Some(next) => next,
                    None => {
                        state.workers -= 1;
                        return;
                    }
                }
            };
            match job {
                Job::Fetch => match self.fetch(&url) {
                    Ok((image, stale)) => self.finish(url, Ok(image), stale),
                    Err(failure) => self.finish(url, Err(failure), false),
                },
                Job::Revalidate => {
                    let image = self.revalidate(&url);
                    let budget = self.options.budget;
                    if self.state().finish_revalidation(&url, image, budget) {
                        self.changed();
                    }
                }
            }
        }
    }

    /// Fetch an image, from the disk cache when it holds the image's response. Returns the image
    /// and whether it drew from a stale stored response that needs revalidation.
    #[cfg(not(target_arch = "wasm32"))]
    fn fetch(&self, url: &str) -> Result<(Arc<RgbaImage>, bool), Failure> {
        if url.starts_with("data:") {
            return RgbaImage::from_str(url, None)
                .map(|image| (Arc::new(image), false))
                .map_err(Failure::lasting);
        }
        if let Some(stored) = self.disk.as_ref().and_then(|disk| disk.load(url)) {
            // A stored response that doesn't decode is fetched again.
            let decoded = || RgbaImage::decode(url, &stored.body).ok().map(Arc::new);
            let fresh = stored.meta.is_fresh();
            if fresh || !stored.meta.must_revalidate() {
                if let Some(image) = decoded() {
                    return Ok((image, !fresh));
                }
            } else {
                let response = self.send(url, &stored.meta.validators())?;
                if response.status != 304 {
                    return self.accept(url, response).map(|image| (image, false));
                }
                if let Some(disk) = &self.disk {
                    disk.refresh(&stored.meta, &response);
                }
                if let Some(image) = decoded() {
                    return Ok((image, false));
                }
            }
        }
        let response = self.send(url, &Validators::default())?;
        self.accept(url, response).map(|image| (image, false))
    }

    /// Send the conditional request of a stale stored response. Returns the new image when the
    /// response changed, or none when the stored response still holds or no new one arrived.
    #[cfg(not(target_arch = "wasm32"))]
    fn revalidate(&self, url: &str) -> Option<Arc<RgbaImage>> {
        let disk = self.disk.as_ref()?;
        let stored = disk.load(url)?;
        let response = self.send(url, &stored.meta.validators()).ok()?;
        if response.status == 304 {
            disk.refresh(&stored.meta, &response);
            return None;
        }
        self.accept(url, response).ok()
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn send(&self, url: &str, validators: &Validators) -> Result<FetchResponse, Failure> {
        self.options
            .fetcher
            .fetch(url, validators)
            .map_err(Failure::passing)
    }

    /// Decode a response, and store it on disk when it succeeds.
    #[cfg(not(target_arch = "wasm32"))]
    fn accept(&self, url: &str, response: FetchResponse) -> Result<Arc<RgbaImage>, Failure> {
        let status = response.status;
        if (400..500).contains(&status) {
            return Err(Failure::lasting(format!("HTTP status {status} for {url}")));
        }
        if !response.is_success() {
            return Err(Failure::passing(format!("HTTP status {status} for {url}")));
        }
        let image = RgbaImage::decode(url, &response.body)
            .map(Arc::new)
            .map_err(Failure::lasting)?;
        if let Some(disk) = &self.disk {
            disk.store(url, &response);
        }
        Ok(image)
    }

    /// Start a browser fetch for each waiting job, within the limit.
    #[cfg(target_arch = "wasm32")]
    fn pump(self: &Arc<Self>) {
        loop {
            let url = {
                let mut state = self.state();
                if state.fetching >= self.options.max_fetches {
                    return;
                }
                match state.next() {
                    Some((url, _)) => url,
                    None => return,
                }
            };
            let shared = self.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let result = crate::browser::fetch_image(&url).await.map(Arc::new);
                shared.finish(url, result, false);
                shared.pump();
            });
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
