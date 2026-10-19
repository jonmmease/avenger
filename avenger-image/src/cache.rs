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
use crate::fetcher::{default_image_fetcher, ImageFetcher, Validators};
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
}

impl Default for ImageCacheOptions {
    fn default() -> Self {
        Self {
            budget: 128 << 20,
            max_fetches: if cfg!(target_arch = "wasm32") { 6 } else { 16 },
            #[cfg(not(target_arch = "wasm32"))]
            fetcher: default_image_fetcher(),
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
/// Clones share their images.
#[derive(Clone)]
pub struct ImageCache {
    shared: Arc<Shared>,
}

pub(crate) struct Shared {
    state: Mutex<State>,
    on_change: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    options: ImageCacheOptions,
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
                options,
            }),
        }
    }

    /// Run `callback` when an image arrives or fails, outside the cache's lock, on the thread
    /// that fetched it. This replaces the previous callback.
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
    /// The queued URLs of each class. A URL whose entry is no longer queued in that class is
    /// skipped when it comes up.
    queues: [VecDeque<String>; 2],
    /// The number of URLs queued in each class.
    queued: [usize; 2],
    /// The fetches that run, and on native targets the threads that run them.
    fetching: usize,
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
}

impl Entry {
    fn new() -> Self {
        Self {
            status: Status::Unrequested,
            leases: 0,
            tick: None,
            failures: 0,
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
        // A failure that waited its turn leaves `recency` while it fetches again.
        self.forget(url);
        if let Some(entry) = self.entries.get_mut(url) {
            entry.status = Status::Queued(class);
        }
        self.queues[class as usize].push_back(url.to_string());
        self.queued[class as usize] += 1;
        self.compact(class);
        ImageState::Pending
    }

    /// The next URL to fetch, visible requests first, marked as fetching.
    fn next(&mut self) -> Option<String> {
        for class in [Class::Visible, Class::Prefetch] {
            while let Some(url) = self.queues[class as usize].pop_front() {
                let Some(entry) = self.entries.get_mut(&url) else {
                    continue;
                };
                if matches!(entry.status, Status::Queued(queued) if queued == class) {
                    entry.status = Status::Fetching;
                    self.queued[class as usize] -= 1;
                    self.fetching += 1;
                    return Some(url);
                }
            }
        }
        None
    }

    /// Drop the queue's skipped places once they outnumber its live ones, so a queue that
    /// requests keep replacing stays as long as its requests.
    fn compact(&mut self, class: Class) {
        let queue = &mut self.queues[class as usize];
        if queue.len() > 64 && queue.len() > 2 * self.queued[class as usize] {
            let entries = &self.entries;
            let mut seen = HashSet::new();
            queue.retain(|url| {
                entries.get(url).is_some_and(
                    |entry| matches!(entry.status, Status::Queued(queued) if queued == class),
                ) && seen.insert(url.clone())
            });
        }
    }

    fn finish(&mut self, url: String, result: Result<Arc<RgbaImage>, Failure>, budget: usize) {
        self.fetching -= 1;
        let Some(entry) = self.entries.get_mut(&url) else {
            return;
        };
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
        if entry.leases == 0 {
            self.remember(&url);
            self.evict(budget);
        }
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
                // A fetch that runs keeps its entry, and joins `recency` when it finishes.
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
            }
        }
    }
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }

    /// Record a fetch's result and run the change callback outside the lock.
    fn finish(&self, url: String, result: Result<Arc<RgbaImage>, Failure>) {
        self.state().finish(url, result, self.options.budget);
        let callback = lock(&self.on_change).clone();
        if let Some(callback) = callback {
            callback();
        }
    }

    /// Start a thread for each queued request that no idle thread will take, within the limit.
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

    /// Fetch queued requests until none is left, then exit.
    #[cfg(not(target_arch = "wasm32"))]
    fn work(self: Arc<Self>) {
        loop {
            let url = {
                let mut state = self.state();
                match state.next() {
                    Some(url) => url,
                    None => {
                        state.workers -= 1;
                        return;
                    }
                }
            };
            let result = self.fetch(&url);
            self.finish(url, result);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn fetch(&self, url: &str) -> Result<Arc<RgbaImage>, Failure> {
        if url.starts_with("data:") {
            return RgbaImage::from_str(url, None)
                .map(Arc::new)
                .map_err(Failure::lasting);
        }
        let response = self
            .options
            .fetcher
            .fetch(url, &Validators::default())
            .map_err(Failure::passing)?;
        if (400..500).contains(&response.status) {
            return Err(Failure::lasting(format!(
                "HTTP status {} for {url}",
                response.status
            )));
        }
        if !response.is_success() {
            return Err(Failure::passing(format!(
                "HTTP status {} for {url}",
                response.status
            )));
        }
        RgbaImage::decode(url, &response.body)
            .map(Arc::new)
            .map_err(Failure::lasting)
    }

    /// Start a browser fetch for each queued request, within the limit.
    #[cfg(target_arch = "wasm32")]
    fn pump(self: &Arc<Self>) {
        loop {
            let url = {
                let mut state = self.state();
                if state.fetching >= self.options.max_fetches {
                    return;
                }
                match state.next() {
                    Some(url) => url,
                    None => return,
                }
            };
            let shared = self.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let result = crate::browser::fetch_image(&url).await.map(Arc::new);
                shared.finish(url, result);
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
