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

#[test]
fn an_image_is_pending_until_it_arrives_and_the_callback_runs() {
    let fetcher = Arc::new(GateFetcher::default());
    let cache = cache(&fetcher, 1 << 20, 4);
    let changes = Arc::new(AtomicUsize::new(0));
    let counted = changes.clone();
    cache.set_on_change(move || {
        counted.fetch_add(1, Ordering::SeqCst);
    });
    assert!(matches!(cache.image("a"), ImageState::Pending));
    assert!(matches!(cache.image("a"), ImageState::Pending));
    fetcher.open();
    wait_until(|| is_ready(&cache, "a"));
    assert!(matches!(cache.image("a"), ImageState::Ready(image) if image.data == [1, 0, 0, 255]));
    assert_eq!(fetcher.fetched(), ["a"]);
    assert_eq!(changes.load(Ordering::SeqCst), 1);
}

/// The callback runs outside the cache's lock, so it can read the cache.
#[test]
fn the_callback_can_read_the_cache() {
    let fetcher = Arc::new(GateFetcher::default());
    fetcher.open();
    let cache = cache(&fetcher, 1 << 20, 4);
    let seen = Arc::new(Mutex::new(None));
    let (reader, written) = (cache.clone(), seen.clone());
    cache.set_on_change(move || {
        *lock(&written) = Some(reader.ready("a").is_some());
    });
    cache.image("a");
    wait_until(|| lock(&seen).is_some());
    assert_eq!(*lock(&seen), Some(true));
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
    let fetcher = Arc::new(GateFetcher::default());
    let cache = cache(&fetcher, 4, 4);
    cache.image("slow");
    wait_until(|| fetcher.fetched().len() == 1);
    {
        // Fill and churn the budget with ready images.
        let mut state = cache.shared.state();
        for url in ["x", "y", "z"] {
            state.entries.insert(url.to_string(), Entry::new());
            state.entries.get_mut(url).unwrap().status = Status::Fetching;
            state.fetching += 1;
            state.finish(
                url.to_string(),
                Ok(Arc::new(RgbaImage {
                    width: 1,
                    height: 1,
                    data: vec![0; 4],
                })),
                4,
            );
        }
    }
    fetcher.open();
    wait_until(|| is_ready(&cache, "slow"));
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
