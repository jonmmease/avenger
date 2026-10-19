use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Condvar;

use super::*;
use crate::error::AvengerImageError;
use crate::fetcher::FetchResponse;

/// One opaque pixel, encoded as PNG.
fn png(red: u8) -> Vec<u8> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(1, 1, image::Rgba([red, 0, 0, 255]))
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

/// A fetcher that serves each URL's response once a test opens its gate, and records the order
/// of its fetches and how many ran at once.
#[derive(Default)]
struct GateFetcher {
    open: Mutex<bool>,
    opened: Condvar,
    fetched: Mutex<Vec<String>>,
    running: AtomicUsize,
    most_running: AtomicUsize,
    /// The statuses to answer instead of a PNG, by URL.
    statuses: Mutex<HashMap<String, u16>>,
}

impl GateFetcher {
    fn open(&self) {
        *lock(&self.open) = true;
        self.opened.notify_all();
    }

    fn fetched(&self) -> Vec<String> {
        lock(&self.fetched).clone()
    }
}

impl ImageFetcher for GateFetcher {
    fn fetch(&self, url: &str, _: &Validators) -> Result<FetchResponse, AvengerImageError> {
        let running = self.running.fetch_add(1, Ordering::SeqCst) + 1;
        self.most_running.fetch_max(running, Ordering::SeqCst);
        lock(&self.fetched).push(url.to_string());
        let mut open = lock(&self.open);
        while !*open {
            open = self.opened.wait(open).unwrap();
        }
        drop(open);
        self.running.fetch_sub(1, Ordering::SeqCst);
        let status = lock(&self.statuses).get(url).copied().unwrap_or(200);
        Ok(FetchResponse {
            status,
            body: if status == 200 {
                png(url.len() as u8)
            } else {
                Vec::new()
            },
            ..Default::default()
        })
    }
}

fn cache(fetcher: &Arc<GateFetcher>, budget: usize, max_fetches: usize) -> ImageCache {
    ImageCache::with_options(ImageCacheOptions {
        budget,
        max_fetches,
        fetcher: fetcher.clone(),
        disk: None,
    })
}

/// Wait until `done` holds, for at most five seconds.
fn wait_until(mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < Duration::from_secs(5), "timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn is_ready(cache: &ImageCache, url: &str) -> bool {
    cache.ready(url).is_some()
}

/// The number of times the cache's change callback has run.
fn count_changes(cache: &ImageCache) -> Arc<AtomicUsize> {
    let changes = Arc::new(AtomicUsize::new(0));
    let counted = changes.clone();
    cache.set_on_change(move || {
        counted.fetch_add(1, Ordering::SeqCst);
    });
    changes
}

/// An image is pending until it arrives. Then the change callback runs, after the image is
/// ready and outside the cache's lock, so it can read the cache.
#[test]
fn an_image_is_pending_until_it_arrives_and_the_callback_runs() {
    let fetcher = Arc::new(GateFetcher::default());
    let cache = cache(&fetcher, 1 << 20, 4);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (reader, written) = (cache.clone(), seen.clone());
    cache.set_on_change(move || lock(&written).push(reader.ready("a").is_some()));
    assert!(matches!(cache.image("a"), ImageState::Pending));
    assert!(matches!(cache.image("a"), ImageState::Pending));
    fetcher.open();
    wait_until(|| !lock(&seen).is_empty());
    assert_eq!(*lock(&seen), [true]);
    assert!(matches!(cache.image("a"), ImageState::Ready(image) if image.data == [1, 0, 0, 255]));
    assert_eq!(fetcher.fetched(), ["a"]);
}

/// The budget keeps the most recently used images: a use of the oldest keeps it, and the next
/// oldest goes.
#[test]
fn the_budget_evicts_the_least_recently_used_image() {
    let fetcher = Arc::new(GateFetcher::default());
    fetcher.open();
    // Each image is one pixel, four bytes.
    let cache = cache(&fetcher, 8, 4);
    for url in ["a", "bb"] {
        cache.image(url);
        wait_until(|| is_ready(&cache, url));
    }
    assert!(cache.ready("a").is_some());
    cache.image("ccc");
    wait_until(|| is_ready(&cache, "ccc"));
    assert!(is_ready(&cache, "a"));
    assert!(!is_ready(&cache, "bb"));
}

/// A pending image has no pixels to evict, and it arrives while others fill the budget.
#[test]
fn a_pending_image_survives_eviction() {
    let pixel = || {
        Arc::new(RgbaImage {
            width: 1,
            height: 1,
            data: vec![0; 4],
        })
    };
    let mut state = State::default();
    state.request("slow", Class::Visible);
    assert_eq!(state.next(1).unwrap().0, "slow");
    // Fill and churn the budget of one image with ready images.
    for url in ["x", "y", "z"] {
        state.request(url, Class::Visible);
        state.next(1).unwrap();
        state.finish(url.to_string(), Class::Visible, Ok(pixel()), false, 4);
    }
    assert!(matches!(state.entries["slow"].status, Status::Fetching));
    state.finish("slow".to_string(), Class::Visible, Ok(pixel()), false, 4);
    assert!(state.entries["slow"].status.image().is_some());
}

/// Leased images stay in memory beyond the budget, and the budget applies once the lease drops.
#[test]
fn leases_hold_more_than_the_budget() {
    let fetcher = Arc::new(GateFetcher::default());
    fetcher.open();
    let cache = cache(&fetcher, 4, 4);
    let urls = ["a", "bb", "ccc"];
    let lease = cache.retain(&urls);
    for url in urls {
        cache.image(url);
    }
    wait_until(|| urls.iter().all(|url| is_ready(&cache, url)));
    assert_eq!(cache.shared.state().bytes, 0);
    drop(lease);
    let ready: Vec<_> = urls.iter().filter(|url| is_ready(&cache, url)).collect();
    assert_eq!(ready.len(), 1);
    assert_eq!(cache.shared.state().bytes, 4);
}

#[test]
fn ready_never_requests() {
    let fetcher = Arc::new(GateFetcher::default());
    let cache = cache(&fetcher, 1 << 20, 4);
    assert!(cache.ready("a").is_none());
    let _lease = cache.retain(&["a"]);
    assert!(cache.ready("a").is_none());
    assert!(fetcher.fetched().is_empty());
    assert!(cache.shared.state().queues.iter().all(VecDeque::is_empty));
}

/// No more fetches run at once than the limit, and the rest wait their turn.
#[test]
fn fetches_stay_within_the_limit() {
    let fetcher = Arc::new(GateFetcher::default());
    let cache = cache(&fetcher, 1 << 20, 2);
    let urls = ["a", "b", "c", "d", "e"];
    for url in urls {
        cache.image(url);
    }
    wait_until(|| fetcher.fetched().len() == 2);
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(fetcher.fetched().len(), 2);
    fetcher.open();
    wait_until(|| urls.iter().all(|url| is_ready(&cache, url)));
    assert_eq!(fetcher.most_running.load(Ordering::SeqCst), 2);
    // The threads exit once nothing is queued.
    wait_until(|| cache.shared.state().workers == 0);
}

/// A prefetch waits behind visible requests, even ones that came after it.
#[test]
fn a_prefetch_waits_behind_visible_requests() {
    let fetcher = Arc::new(GateFetcher::default());
    let cache = cache(&fetcher, 1 << 20, 1);
    cache.image("busy");
    wait_until(|| fetcher.fetched().len() == 1);
    cache.prefetch("later");
    cache.image("now");
    // A visible request for a queued prefetch moves it ahead.
    cache.prefetch("promoted");
    cache.image("promoted");
    fetcher.open();
    wait_until(|| fetcher.fetched().len() == 4);
    assert_eq!(fetcher.fetched(), ["busy", "now", "promoted", "later"]);
}

/// When the last lease that holds a queued URL drops, its fetch is dropped. A request without a
/// lease still runs.
#[test]
fn a_queued_fetch_drops_with_its_last_lease() {
    let fetcher = Arc::new(GateFetcher::default());
    let cache = cache(&fetcher, 1 << 20, 1);
    cache.image("busy");
    wait_until(|| fetcher.fetched().len() == 1);
    let first = cache.retain(&["leased", "kept"]);
    let second = cache.retain(&["kept"]);
    cache.image("leased");
    cache.image("kept");
    cache.image("unleased");
    drop(first);
    fetcher.open();
    wait_until(|| is_ready(&cache, "unleased"));
    assert_eq!(fetcher.fetched(), ["busy", "kept", "unleased"]);
    assert!(matches!(cache.image("leased"), ImageState::Pending));
    drop(second);
}

/// A new prefetch lease, taken before the old one drops, keeps the prefetches the two share and
/// drops the queued ones that only the old one held.
#[test]
fn replacing_a_prefetch_lease_drops_what_it_no_longer_holds() {
    let fetcher = Arc::new(GateFetcher::default());
    let cache = cache(&fetcher, 1 << 20, 1);
    cache.image("busy");
    wait_until(|| fetcher.fetched().len() == 1);
    let old = cache.retain(&["left", "shared"]);
    cache.prefetch("left");
    cache.prefetch("shared");
    let new = cache.retain(&["shared", "entered"]);
    cache.prefetch("entered");
    drop(old);
    fetcher.open();
    wait_until(|| is_ready(&cache, "entered"));
    assert_eq!(fetcher.fetched(), ["busy", "shared", "entered"]);
    drop(new);
}

/// A failure waits before the next fetch: a second from the first failure, doubling, to five
/// minutes, and five minutes at once for a 4xx response.
#[test]
fn failures_back_off() {
    let fetcher = Arc::new(GateFetcher::default());
    fetcher.open();
    lock(&fetcher.statuses).extend([("down".to_string(), 503), ("gone".to_string(), 404)]);
    let cache = cache(&fetcher, 1 << 20, 4);
    let failed = |url: &str| matches!(cache.image(url), ImageState::Failed(_));
    let wait = |url: &str| {
        let state = cache.shared.state();
        let Status::Failed { retry_at, .. } = state.entries[url].status else {
            panic!("{url} didn't fail");
        };
        retry_at.saturating_duration_since(Instant::now())
    };
    // Make a failure's wait pass.
    let expire = |url: &str| {
        let mut state = cache.shared.state();
        if let Status::Failed { retry_at, .. } = &mut state.entries.get_mut(url).unwrap().status {
            *retry_at = Instant::now();
        }
    };

    cache.image("gone");
    wait_until(|| failed("gone"));
    assert!(wait("gone") > Duration::from_secs(299));

    for expected in [1, 2, 4] {
        cache.image("down");
        wait_until(|| failed("down"));
        let seconds = wait("down").as_secs_f64();
        assert!(seconds > expected as f64 - 0.5 && seconds <= expected as f64);
        // A request during the wait doesn't fetch again.
        let fetches = fetcher.fetched().len();
        assert!(failed("down"));
        assert_eq!(fetcher.fetched().len(), fetches);
        expire("down");
    }
    for _ in 0..12 {
        cache.image("down");
        wait_until(|| failed("down"));
        expire("down");
    }
    cache.image("down");
    wait_until(|| failed("down"));
    assert!(wait("down") > Duration::from_secs(299));
}

/// A wait returns once its images are ready or failed, or names the ones still pending at its
/// timeout.
#[test]
fn a_wait_ends_when_its_images_settle() {
    let fetcher = Arc::new(GateFetcher::default());
    lock(&fetcher.statuses).insert("gone".to_string(), 404);
    let cache = cache(&fetcher, 1 << 20, 4);
    let timeout = cache
        .wait(&["a", "gone"], Duration::from_millis(30))
        .unwrap_err();
    let mut pending = timeout.pending;
    pending.sort();
    assert_eq!(pending, ["a", "gone"]);
    let opener = fetcher.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(20));
        opener.open();
    });
    let lease = cache.wait(&["a", "gone"], Duration::from_secs(5)).unwrap();
    assert!(is_ready(&cache, "a"));
    assert!(matches!(cache.image("gone"), ImageState::Failed(_)));
    drop(lease);
}

/// When a failure's wait passes, the cache runs its change callback again, so a renderer that
/// draws the image asks for it again without another event. Each failure's wait ends with its
/// own wake, however the waits overlap.
#[test]
fn every_failure_wakes_the_cache_when_its_wait_passes() {
    let fetcher = Arc::new(GateFetcher::default());
    fetcher.open();
    for url in ["a", "b"] {
        lock(&fetcher.statuses).insert(url.to_string(), 503);
    }
    let cache = cache(&fetcher, 1 << 20, 4);
    let changes = count_changes(&cache);
    cache.image("a");
    wait_until(|| changes.load(Ordering::SeqCst) == 1);
    assert!(matches!(cache.image("a"), ImageState::Failed(_)));
    std::thread::sleep(Duration::from_millis(300));
    cache.image("b");
    wait_until(|| changes.load(Ordering::SeqCst) == 2);
    // A's wait ends a second after its failure, and b's 0.3 seconds later.
    wait_until(|| changes.load(Ordering::SeqCst) == 4);
    // Past its wait, a request fetches the image again.
    assert!(matches!(cache.image("a"), ImageState::Pending));
}

/// A failed prefetch wakes nothing, but once a view asks for it, its wait ends with a wake, so
/// the view asks for it again.
#[test]
fn a_failed_prefetch_that_a_view_asks_for_wakes_the_cache() {
    let fetcher = Arc::new(GateFetcher::default());
    fetcher.open();
    lock(&fetcher.statuses).insert("down".to_string(), 503);
    let cache = cache(&fetcher, 1 << 20, 4);
    let changes = count_changes(&cache);
    cache.prefetch("down");
    wait_until(|| {
        matches!(
            cache.shared.state().entries["down"].status,
            Status::Failed { .. }
        )
    });
    assert!(matches!(cache.image("down"), ImageState::Failed(_)));
    wait_until(|| changes.load(Ordering::SeqCst) == 1);
    assert!(matches!(cache.image("down"), ImageState::Pending));
}

/// Prefetches leave two fetches for visible requests, so an image that comes into view starts
/// while prefetches wait.
#[test]
fn prefetches_leave_fetches_for_visible_requests() {
    let fetcher = Arc::new(GateFetcher::default());
    let cache = cache(&fetcher, 1 << 20, 4);
    for url in ["p1", "p2", "p3", "p4"] {
        cache.prefetch(url);
    }
    wait_until(|| fetcher.fetched().len() == 2);
    std::thread::sleep(Duration::from_millis(20));
    // The two fetches start together, in either order.
    let mut running = fetcher.fetched();
    running.sort();
    assert_eq!(running, ["p1", "p2"]);
    cache.image("seen");
    wait_until(|| fetcher.fetched().len() == 3);
    assert_eq!(fetcher.fetched()[2], "seen");
    fetcher.open();
    wait_until(|| {
        ["p1", "p2", "p3", "p4", "seen"]
            .iter()
            .all(|url| is_ready(&cache, url))
    });
}

/// A revalidation that finishes after its entry was evicted leaves the URL's newer entry alone.
#[test]
fn a_revalidation_of_an_evicted_entry_leaves_the_newer_one() {
    let mut state = State::default();
    let image = Arc::new(RgbaImage {
        width: 1,
        height: 1,
        data: vec![0; 4],
    });
    state.request("a", Class::Visible);
    let (_, job) = state.next(1).unwrap();
    assert_eq!(job, Job::Fetch(Class::Visible));
    state.finish(
        "a".to_string(),
        Class::Visible,
        Ok(image.clone()),
        true,
        1 << 20,
    );
    let (_, job) = state.next(1).unwrap();
    let Job::Revalidate(generation) = job else {
        panic!("expected a revalidation");
    };
    state.evict(0);
    assert!(state.entries.is_empty());
    state.request("a", Class::Visible);
    assert_eq!(
        state.finish_revalidation("a", generation, Ok(Some(image)), 1 << 20),
        (false, None)
    );
    assert!(matches!(
        state.entries["a"].status,
        Status::Queued(Class::Visible)
    ));
    assert_eq!((state.fetching, state.queued), ([0, 0], [1, 0]));
}

/// A prefetch arrives without the change callback, since no view draws it yet, and a prefetch
/// that a view asks for while it fetches runs the callback when it arrives.
#[test]
fn only_images_that_a_view_asks_for_run_the_callback() {
    let fetcher = Arc::new(GateFetcher::default());
    let cache = cache(&fetcher, 1 << 20, 4);
    let changes = count_changes(&cache);
    cache.prefetch("later");
    cache.prefetch("asked");
    wait_until(|| fetcher.fetched().len() == 2);
    assert!(matches!(cache.image("asked"), ImageState::Pending));
    fetcher.open();
    wait_until(|| is_ready(&cache, "later") && is_ready(&cache, "asked"));
    wait_until(|| changes.load(Ordering::SeqCst) == 1);
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(changes.load(Ordering::SeqCst), 1);
}
