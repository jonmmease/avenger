//! A resolver that fetches images in the background and keeps them in memory.

#[cfg(not(target_arch = "wasm32"))]
use std::collections::BTreeSet;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
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
/// the server won't change soon: a 4xx status other than 408 and 429, or bytes that don't
/// decode.
const LAST_RETRY: Duration = Duration::from_secs(300);
/// The longest wait that a `Retry-After` header sets.
const LONGEST_RETRY_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
/// The fetch slots that only visible requests take, so an image that comes into view never
/// waits for a prefetch to finish.
const VISIBLE_SLOTS: usize = 2;

/// How an [`ImageCache`] fetches and how much it keeps.
#[derive(Clone)]
pub struct ImageCacheOptions {
    /// The bytes of decoded images that the cache keeps beyond the leased ones. 128 MiB by
    /// default.
    pub budget: usize,
    /// The most images that fetch at once: 16 by default on native targets, each on its own
    /// thread, and 6 in the browser. Prefetches leave two of them for visible requests.
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
/// requests, and prefetches leave two fetches free for them. Each image fetches once while the
/// cache holds it.
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
    /// Notified when a job finishes.
    finished: Condvar,
    /// The times that failed images can fetch again, and a thread that runs the change callback
    /// at each, on native targets.
    #[cfg(not(target_arch = "wasm32"))]
    retry: Arc<(Mutex<BTreeSet<Instant>>, Condvar)>,
    #[cfg(not(target_arch = "wasm32"))]
    retry_thread: std::sync::OnceLock<()>,
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
                finished: Condvar::new(),
                #[cfg(not(target_arch = "wasm32"))]
                retry: Arc::default(),
                #[cfg(not(target_arch = "wasm32"))]
                retry_thread: std::sync::OnceLock::new(),
                on_change: Mutex::new(None),
                #[cfg(not(target_arch = "wasm32"))]
                disk: options.disk.as_ref().map(DiskCache::new),
                options,
            }),
        }
    }

    /// Run `callback` when an image that [`ImageResolver::image`] asked for arrives, changes, or
    /// fails, and when a failed one can fetch again, outside the cache's lock, on the thread that
    /// fetched it. Images that only [`ImageResolver::prefetch`] asked for arrive without it, since
    /// no view draws them yet. This replaces the previous callback.
    pub fn set_on_change(&self, callback: impl Fn() + Send + Sync + 'static) {
        *lock(&self.shared.on_change) = Some(Arc::new(callback));
    }

    /// Request the images at `urls` and wait until each is ready or failed, for at most
    /// `timeout`, so an export or a test can draw them. Returns a lease that keeps them in
    /// memory, or the URLs still pending at the timeout.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn wait(&self, urls: &[&str], timeout: Duration) -> Result<ImageLease, WaitTimeout> {
        let lease = self.retain(urls);
        for url in urls {
            self.image(url);
        }
        let deadline = Instant::now() + timeout;
        let mut state = self.shared.state();
        loop {
            let pending: Vec<String> = urls
                .iter()
                .filter(|url| {
                    state.entries.get(**url).is_none_or(|entry| {
                        matches!(
                            entry.status,
                            Status::Unrequested | Status::Queued(_) | Status::Fetching
                        )
                    })
                })
                .map(|url| url.to_string())
                .collect();
            if pending.is_empty() {
                return Ok(lease);
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(WaitTimeout { pending });
            }
            state = self
                .shared
                .finished
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    fn request(&self, url: &str, class: Class) -> ImageState {
        let (state, wake) = self.shared.state().request(url, class);
        if let Some(at) = wake {
            self.shared.wake_at(at);
        }
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

/// The images that were still pending when [`ImageCache::wait`] timed out.
#[derive(Debug, Clone, thiserror::Error)]
#[error("timed out waiting for images: {pending:?}")]
pub struct WaitTimeout {
    pub pending: Vec<String>,
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
    /// Fetch an image that doesn't draw yet, for a request of this class.
    Fetch(Class),
    /// Revalidate the stale stored response that an image drew from, for the entry of this
    /// generation.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Revalidate(u64),
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
    /// How long the response asked the cache to wait before it requests the image again.
    pub(crate) retry_after: Option<Duration>,
}

impl Failure {
    pub(crate) fn passing(message: impl fmt::Display) -> Self {
        Self {
            message: message.to_string().into(),
            lasting: false,
            retry_after: None,
        }
    }

    pub(crate) fn lasting(message: impl fmt::Display) -> Self {
        Self {
            message: message.to_string().into(),
            lasting: true,
            retry_after: None,
        }
    }

    /// The failure of a response whose status isn't a success. A client error lasts, except a
    /// request timeout (408) and too many requests (429), which pass as server errors do.
    pub(crate) fn status(status: u16, url: &str) -> Self {
        let message = format!("HTTP status {status} for {url}");
        if (400..500).contains(&status) && !matches!(status, 408 | 429) {
            Self::lasting(message)
        } else {
            Self::passing(message)
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
    /// The jobs that run in each class.
    fetching: [usize; 2],
    /// The threads that run jobs, on native targets.
    #[cfg(not(target_arch = "wasm32"))]
    workers: usize,
    /// The entries created so far.
    generations: u64,
}

struct Entry {
    status: Status,
    leases: usize,
    /// The entry's tick in `recency`, when it's there.
    tick: Option<u64>,
    /// The failures in a row, which set the wait before the next fetch.
    failures: u32,
    revalidation: Revalidation,
    /// Whether [`ImageResolver::image`] asked for the image, so that its changes run the change
    /// callback.
    wanted: bool,
    /// Which entry this is for its URL, counted across the cache, so a revalidation that
    /// finishes after its entry was evicted leaves a newer entry alone.
    generation: u64,
}

impl Entry {
    fn new(generation: u64) -> Self {
        Self {
            status: Status::Unrequested,
            leases: 0,
            tick: None,
            failures: 0,
            revalidation: Revalidation::Idle,
            wanted: false,
            generation,
        }
    }

    /// The status of a failure, which waits before the next fetch: a second after the first
    /// failure in a row, doubling to five minutes, or five minutes for a lasting failure, or
    /// longer when the response's `Retry-After` asks, for at most a day.
    fn fail(&mut self, failure: Failure) -> Status {
        self.failures = self.failures.saturating_add(1);
        let backoff = if failure.lasting {
            LAST_RETRY
        } else {
            FIRST_RETRY
                .saturating_mul(1 << (self.failures - 1).min(16))
                .min(LAST_RETRY)
        };
        let asked = failure.retry_after.unwrap_or_default();
        let wait = backoff.max(asked.min(LONGEST_RETRY_AFTER));
        Status::Failed {
            message: failure.message,
            retry_at: Instant::now() + wait,
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
    /// A new entry's generation.
    fn generation(&mut self) -> u64 {
        self.generations += 1;
        self.generations
    }

    /// Answer a request, and queue its fetch when the cache holds no answer. Also returns when
    /// to run the change callback: a failed prefetch that a view now asks for wakes the cache
    /// when its wait passes, as a failure that a view asked for does.
    fn request(&mut self, url: &str, class: Class) -> (ImageState, Option<Instant>) {
        let generation = self.generation();
        let entry = self
            .entries
            .entry(url.to_string())
            .or_insert_with(|| Entry::new(generation));
        let newly_wanted = class == Class::Visible && !entry.wanted;
        entry.wanted |= class == Class::Visible;
        match &entry.status {
            Status::Ready(image) => {
                let image = image.clone();
                self.touch(url);
                return (ImageState::Ready(image), None);
            }
            Status::Failed { message, retry_at } if Instant::now() < *retry_at => {
                let wake = newly_wanted.then_some(*retry_at);
                return (ImageState::Failed(message.clone()), wake);
            }
            Status::Fetching | Status::Queued(Class::Visible) => {
                return (ImageState::Pending, None)
            }
            Status::Queued(Class::Prefetch) if class == Class::Prefetch => {
                return (ImageState::Pending, None)
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
        (ImageState::Pending, None)
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
    /// prefetch class, and that class runs at most `prefetches` jobs at once.
    fn next(&mut self, prefetches: usize) -> Option<(String, Job)> {
        for class in [Class::Visible, Class::Prefetch] {
            if class == Class::Prefetch && self.fetching[class as usize] >= prefetches {
                break;
            }
            while let Some(url) = self.queues[class as usize].pop_front() {
                let Some(entry) = self.entries.get_mut(&url) else {
                    continue;
                };
                if !entry.waits_in(class) {
                    continue;
                }
                let job = if matches!(entry.status, Status::Ready(_)) {
                    entry.revalidation = Revalidation::Running;
                    Job::Revalidate(entry.generation)
                } else {
                    entry.status = Status::Fetching;
                    Job::Fetch(class)
                };
                self.queued[class as usize] -= 1;
                self.fetching[class as usize] += 1;
                return Some((url, job));
            }
        }
        None
    }

    /// The jobs that run.
    fn running(&self) -> usize {
        self.fetching[Class::Visible as usize] + self.fetching[Class::Prefetch as usize]
    }

    /// The waiting jobs that can start now, with at most `prefetches` prefetch-class jobs
    /// running.
    #[cfg(not(target_arch = "wasm32"))]
    fn runnable(&self, prefetches: usize) -> usize {
        let free = prefetches.saturating_sub(self.fetching[Class::Prefetch as usize]);
        self.queued[Class::Visible as usize] + self.queued[Class::Prefetch as usize].min(free)
    }

    /// Record a fetch's result. An image that drew from a stale stored response queues its
    /// revalidation. Returns whether a view asked for the image, and when a failed image can
    /// fetch again.
    fn finish(
        &mut self,
        url: String,
        class: Class,
        result: Result<Arc<RgbaImage>, Failure>,
        stale: bool,
        budget: usize,
    ) -> (bool, Option<Instant>) {
        self.fetching[class as usize] -= 1;
        let Some(entry) = self.entries.get_mut(&url) else {
            return (false, None);
        };
        let revalidate = stale && result.is_ok();
        entry.status = match result {
            Ok(image) => {
                entry.failures = 0;
                Status::Ready(image)
            }
            Err(failure) => entry.fail(failure),
        };
        if revalidate {
            entry.revalidation = Revalidation::Queued;
        }
        let (unleased, wanted) = (entry.leases == 0, entry.wanted);
        let retry_at = match entry.status {
            Status::Failed { retry_at, .. } => Some(retry_at),
            _ => None,
        };
        if revalidate {
            self.enqueue(&url, Class::Prefetch);
        }
        if unleased {
            self.remember(&url);
            self.evict(budget);
        }
        (wanted, retry_at)
    }

    /// Record a revalidation's result: the image of a changed response, none when the stored
    /// response still holds or no new response arrived, or the failure of an image that the
    /// server no longer serves. Returns whether the image of a wanted entry changed, and when a
    /// failed image can fetch again.
    #[cfg(not(target_arch = "wasm32"))]
    fn finish_revalidation(
        &mut self,
        url: &str,
        generation: u64,
        result: Result<Option<Arc<RgbaImage>>, Failure>,
        budget: usize,
    ) -> (bool, Option<Instant>) {
        self.fetching[Class::Prefetch as usize] -= 1;
        let Some(entry) = self
            .entries
            .get_mut(url)
            .filter(|entry| entry.generation == generation)
        else {
            return (false, None);
        };
        entry.revalidation = Revalidation::Idle;
        let status = match result {
            Ok(None) => return (false, None),
            Ok(Some(image)) => Status::Ready(image),
            Err(failure) => entry.fail(failure),
        };
        let retry_at = match status {
            Status::Failed { retry_at, .. } => Some(retry_at),
            _ => None,
        };
        let (remembered, wanted) = (entry.tick.is_some(), entry.wanted);
        self.forget(url);
        if let Some(entry) = self.entries.get_mut(url) {
            entry.status = status;
        }
        if remembered {
            self.remember(url);
            self.evict(budget);
        }
        (wanted, retry_at)
    }

    fn retain(&mut self, urls: &[String]) {
        for url in urls {
            let generation = self.generation();
            let entry = self
                .entries
                .entry(url.clone())
                .or_insert_with(|| Entry::new(generation));
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

    /// The most prefetch-class jobs that run at once, which leaves [`VISIBLE_SLOTS`] for
    /// visible requests.
    fn prefetches(&self) -> usize {
        self.options
            .max_fetches
            .saturating_sub(VISIBLE_SLOTS)
            .max(1)
    }

    /// Record a fetch's result, and run the change callback for an image that a view asked for.
    /// A failed one runs it again when it can fetch again, so the renderer that draws it asks for
    /// it again.
    fn finish(
        self: &Arc<Self>,
        url: String,
        class: Class,
        result: Result<Arc<RgbaImage>, Failure>,
        stale: bool,
    ) {
        let budget = self.options.budget;
        let (wanted, retry_at) = self.state().finish(url, class, result, stale, budget);
        self.report(wanted, retry_at);
    }

    /// Wake the threads that wait for images, and, when a view asked for the image that a job
    /// settled, run the change callback, and run it again at `retry_at` for a failed image.
    fn report(self: &Arc<Self>, wanted: bool, retry_at: Option<Instant>) {
        self.finished.notify_all();
        if wanted {
            self.changed();
            if let Some(at) = retry_at {
                self.wake_at(at);
            }
        }
    }

    /// Run the change callback at `at`.
    #[cfg(not(target_arch = "wasm32"))]
    fn wake_at(self: &Arc<Self>, at: Instant) {
        let (deadlines, set) = &*self.retry;
        let mut deadlines = lock(deadlines);
        if deadlines.first().is_none_or(|first| at < *first) {
            set.notify_one();
        }
        deadlines.insert(at);
        drop(deadlines);
        self.retry_thread.get_or_init(|| {
            let (retry, shared) = (self.retry.clone(), Arc::downgrade(self));
            let _ = std::thread::Builder::new()
                .name("avenger-image-retry".to_string())
                .spawn(move || wake_at_deadlines(&retry, &shared));
        });
    }

    #[cfg(target_arch = "wasm32")]
    fn wake_at(self: &Arc<Self>, at: Instant) {
        use wasm_bindgen::{closure::Closure, JsCast};
        let shared = Arc::downgrade(self);
        let fire = Closure::once_into_js(move || {
            if let Some(shared) = shared.upgrade() {
                shared.changed();
            }
        });
        let delay = at.saturating_duration_since(Instant::now()).as_millis();
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                fire.unchecked_ref(),
                delay.min(i32::MAX as u128) as i32,
            );
        }
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
        let prefetches = self.prefetches();
        while state.workers < self.options.max_fetches
            && state.runnable(prefetches) > state.workers - state.running()
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
                match state.next(self.prefetches()) {
                    Some(next) => next,
                    None => {
                        state.workers -= 1;
                        return;
                    }
                }
            };
            // A job that panics, as a decoder can, fails its image instead of leaving it pending.
            match job {
                Job::Fetch(class) => {
                    let fetched =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.fetch(&url)))
                            .unwrap_or_else(|_| {
                                Err(Failure::lasting(format!("fetching {url} panicked")))
                            });
                    match fetched {
                        Ok((image, stale)) => self.finish(url, class, Ok(image), stale),
                        Err(failure) => self.finish(url, class, Err(failure), false),
                    }
                }
                Job::Revalidate(generation) => {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        self.revalidate(&url)
                    }))
                    .unwrap_or(Ok(None));
                    let budget = self.options.budget;
                    let (wanted, retry_at) = self
                        .state()
                        .finish_revalidation(&url, generation, result, budget);
                    self.report(wanted, retry_at);
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
    /// response changed, or none when the stored response still holds or no new one arrived. A
    /// passing failure, such as a server error, leaves the stored response to draw, as RFC 9111
    /// §4.3.3 allows, and a lasting one fails the image, since the server no longer serves the
    /// stored one.
    #[cfg(not(target_arch = "wasm32"))]
    fn revalidate(&self, url: &str) -> Result<Option<Arc<RgbaImage>>, Failure> {
        let Some(disk) = self.disk.as_ref() else {
            return Ok(None);
        };
        let Some(stored) = disk.load(url) else {
            return Ok(None);
        };
        let Ok(response) = self.send(url, &stored.meta.validators()) else {
            return Ok(None);
        };
        if response.status == 304 {
            disk.refresh(&stored.meta, &response);
            return Ok(None);
        }
        match self.accept(url, response) {
            Ok(image) => Ok(Some(image)),
            Err(failure) if failure.lasting => Err(failure),
            Err(_) => Ok(None),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn send(&self, url: &str, validators: &Validators) -> Result<FetchResponse, Failure> {
        self.options
            .fetcher
            .fetch(url, validators)
            .map_err(Failure::passing)
    }

    /// Decode a response, and store it on disk when it succeeds. A lasting failure, such as a
    /// 404 or a body that doesn't decode, removes the stored response, which the server no
    /// longer serves.
    #[cfg(not(target_arch = "wasm32"))]
    fn accept(&self, url: &str, response: FetchResponse) -> Result<Arc<RgbaImage>, Failure> {
        let decoded = if response.is_success() {
            RgbaImage::decode(url, &response.body)
                .map(Arc::new)
                .map_err(Failure::lasting)
        } else {
            let now = crate::disk::now_ms();
            Err(Failure {
                retry_after: crate::disk::retry_after(response.retry_after.as_deref(), now),
                ..Failure::status(response.status, url)
            })
        };
        if let Some(disk) = &self.disk {
            match &decoded {
                Ok(_) => disk.store(url, &response),
                Err(failure) if failure.lasting => disk.remove(url),
                Err(_) => {}
            }
        }
        decoded
    }

    /// Start a browser fetch for each waiting job, within the limit.
    #[cfg(target_arch = "wasm32")]
    fn pump(self: &Arc<Self>) {
        loop {
            let (url, class) = {
                let mut state = self.state();
                if state.running() >= self.options.max_fetches {
                    return;
                }
                match state.next(self.prefetches()) {
                    Some((url, Job::Fetch(class))) => (url, class),
                    // The browser keeps no stored responses, so nothing revalidates.
                    Some((url, Job::Revalidate(_))) => (url, Class::Prefetch),
                    None => return,
                }
            };
            let shared = self.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let result = crate::browser::fetch_image(&url).await.map(Arc::new);
                shared.finish(url, class, result, false);
                shared.pump();
            });
        }
    }
}

/// Run the cache's change callback at each deadline, once for the deadlines that pass together,
/// until the cache drops.
#[cfg(not(target_arch = "wasm32"))]
fn wake_at_deadlines(retry: &(Mutex<BTreeSet<Instant>>, Condvar), shared: &Weak<Shared>) {
    let (deadlines, set) = retry;
    let mut waiting = lock(deadlines);
    loop {
        if shared.strong_count() == 0 {
            return;
        }
        let now = Instant::now();
        // Wake now and then to notice that the cache dropped.
        let wait = match waiting.first() {
            Some(&at) if now >= at => {
                *waiting = waiting.split_off(&now);
                drop(waiting);
                if let Some(shared) = shared.upgrade() {
                    shared.changed();
                }
                waiting = lock(deadlines);
                continue;
            }
            Some(&at) => at - now,
            None => Duration::from_secs(60),
        };
        waiting = set
            .wait_timeout(waiting, wait)
            .unwrap_or_else(PoisonError::into_inner)
            .0;
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
