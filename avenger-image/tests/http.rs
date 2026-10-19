//! Fetching over HTTP with a disk cache, against a local server.

mod support;

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use avenger_image::reqwest_fetcher::ReqwestImageFetcher;
use avenger_image::{DiskCacheOptions, ImageCache, ImageCacheOptions, ImageResolver, ImageState};
use support::{empty_dir, files, png, wait_until, Gate, Reply, TestServer};

fn cache(disk: Option<(&Path, u64)>, timeout: Duration) -> ImageCache {
    ImageCache::with_options(ImageCacheOptions {
        fetcher: Arc::new(ReqwestImageFetcher::new("avenger-tests/1.0", timeout).unwrap()),
        disk: disk.map(|(dir, budget)| DiskCacheOptions {
            dir: dir.to_path_buf(),
            budget,
        }),
        ..Default::default()
    })
}

fn disk_cache(dir: &Path) -> ImageCache {
    cache(Some((dir, 50_000_000)), Duration::from_secs(10))
}

/// The image's state once it's no longer pending.
fn settled(cache: &ImageCache, url: &str) -> ImageState {
    let mut state = cache.image(url);
    wait_until(|| {
        state = cache.image(url);
        !matches!(state, ImageState::Pending)
    });
    state
}

fn ready(cache: &ImageCache, url: &str) -> Arc<avenger_image::RgbaImage> {
    match settled(cache, url) {
        ImageState::Ready(image) => image,
        state => panic!("{url}: {state:?}"),
    }
}

/// The milliseconds since the Unix epoch.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// When the only stored response in `dir` goes stale, from its metadata file.
fn fresh_until(dir: &Path) -> i64 {
    let metas: Vec<_> = files(dir)
        .into_iter()
        .filter(|path| path.to_string_lossy().ends_with("~meta.json"))
        .collect();
    assert_eq!(metas.len(), 1, "{metas:?}");
    let meta: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&metas[0]).unwrap()).unwrap();
    meta["fresh_until"].as_i64().unwrap()
}

#[test]
fn the_user_agent_arrives() {
    let server = TestServer::start(|_| Reply::ok(png(10), &[]));
    let cache = cache(None, Duration::from_secs(10));
    assert_eq!(ready(&cache, &server.url("/a.png")).data, [10, 0, 0, 255]);
    assert_eq!(
        server.requests()[0].headers["user-agent"],
        "avenger-tests/1.0"
    );
}

#[test]
fn errors_and_slow_responses_fail_cleanly() {
    let server = TestServer::start(|request| match request.path.as_str() {
        "/error.png" => Reply::status(503, Vec::new(), &[]),
        "/missing.png" => Reply::status(404, Vec::new(), &[]),
        "/garbage.png" => Reply::ok(b"not an image".to_vec(), &[]),
        _ => Reply {
            delay: Duration::from_secs(2),
            ..Reply::ok(png(1), &[])
        },
    });
    let cache = cache(None, Duration::from_millis(200));
    for path in ["/error.png", "/missing.png", "/garbage.png", "/slow.png"] {
        let ImageState::Failed(message) = settled(&cache, &server.url(path)) else {
            panic!("{path} didn't fail");
        };
        assert!(!message.is_empty());
    }
}

#[test]
fn a_fresh_entry_needs_no_request_after_a_restart() {
    let server = TestServer::start(|_| Reply::ok(png(20), &[("Cache-Control", "max-age=3600")]));
    let dir = empty_dir("fresh");
    let url = server.url("/tile.png");
    ready(&disk_cache(&dir), &url);
    assert_eq!(ready(&disk_cache(&dir), &url).data, [20, 0, 0, 255]);
    assert_eq!(server.count("/tile.png"), 1);
}

/// A stale entry draws at once while its conditional request runs, and a 304 refreshes its
/// freshness without decoding the image again.
#[test]
fn a_stale_entry_draws_at_once_and_revalidates() {
    let gate = Arc::new(Gate::default());
    let waited = gate.clone();
    let server = TestServer::start(move |request| {
        if request.headers.get("if-none-match").map(String::as_str) == Some("\"v1\"") {
            waited.wait();
            Reply::status(304, Vec::new(), &[("Cache-Control", "max-age=3600")])
        } else {
            Reply::ok(
                png(30),
                &[("Cache-Control", "max-age=0"), ("ETag", "\"v1\"")],
            )
        }
    });
    let dir = empty_dir("stale");
    let url = server.url("/tile.png");
    ready(&disk_cache(&dir), &url);
    assert!(fresh_until(&dir) <= now_ms());

    let restarted = disk_cache(&dir);
    let image = ready(&restarted, &url);
    wait_until(|| server.count("/tile.png") == 2);
    gate.open();
    wait_until(|| fresh_until(&dir) > now_ms());
    let ImageState::Ready(after) = restarted.image(&url) else {
        panic!("the image didn't stay ready");
    };
    assert!(Arc::ptr_eq(&image, &after));
    // Fresh again, the next run sends no request.
    ready(&disk_cache(&dir), &url);
    assert_eq!(server.count("/tile.png"), 2);
}

#[test]
fn a_must_revalidate_entry_waits_for_its_response() {
    let gate = Arc::new(Gate::default());
    let waited = gate.clone();
    let server = TestServer::start(move |request| {
        if request.headers.contains_key("if-none-match") {
            waited.wait();
            Reply::status(304, Vec::new(), &[])
        } else {
            Reply::ok(
                png(40),
                &[
                    ("Cache-Control", "max-age=0, must-revalidate"),
                    ("ETag", "\"v1\""),
                ],
            )
        }
    });
    let dir = empty_dir("must-revalidate");
    let url = server.url("/tile.png");
    ready(&disk_cache(&dir), &url);

    let restarted = disk_cache(&dir);
    assert!(matches!(restarted.image(&url), ImageState::Pending));
    wait_until(|| server.count("/tile.png") == 2);
    std::thread::sleep(Duration::from_millis(50));
    assert!(matches!(restarted.image(&url), ImageState::Pending));
    gate.open();
    assert_eq!(ready(&restarted, &url).data, [40, 0, 0, 255]);
}

#[test]
fn no_store_writes_nothing() {
    let server = TestServer::start(|_| Reply::ok(png(50), &[("Cache-Control", "no-store")]));
    let dir = empty_dir("no-store");
    ready(&disk_cache(&dir), &server.url("/tile.png"));
    assert!(files(&dir).is_empty());
}

/// Past its budget, the disk cache removes the response that was least recently read or
/// written.
#[test]
fn the_budget_evicts_the_least_recently_used_entry() {
    let server = TestServer::start(|_| Reply::ok(png(60), &[("Cache-Control", "max-age=3600")]));
    let dir = empty_dir("budget");
    let (a, b, c) = (
        server.url("/a.png"),
        server.url("/b.png"),
        server.url("/c.png"),
    );
    ready(&disk_cache(&dir), &a);
    let entry: u64 = files(&dir)
        .iter()
        .map(|path| std::fs::metadata(path).unwrap().len())
        .sum();
    let budget = entry * 5 / 2;
    let limited = || cache(Some((&dir, budget)), Duration::from_secs(10));

    std::thread::sleep(Duration::from_millis(5));
    ready(&limited(), &b);
    std::thread::sleep(Duration::from_millis(5));
    // A read in a new run makes `a` the most recently used.
    ready(&limited(), &a);
    std::thread::sleep(Duration::from_millis(5));
    ready(&limited(), &c);
    assert_eq!(files(&dir).len(), 4);

    let next = limited();
    ready(&next, &a);
    ready(&next, &c);
    assert_eq!((server.count("/a.png"), server.count("/c.png")), (1, 1));
    ready(&next, &b);
    assert_eq!(server.count("/b.png"), 2);
}

/// A revalidation that the server answers with a 404 fails the image and removes the stored
/// response, so the next run fetches the image instead of drawing it. One answered with a 503
/// leaves the stored response to draw.
#[test]
fn a_revalidation_answered_404_removes_the_stored_response() {
    let removed = Arc::new(AtomicBool::new(false));
    let gate = Arc::new(Gate::default());
    let (gone, waited) = (removed.clone(), gate.clone());
    let server = TestServer::start(move |request| {
        let gone = gone.load(Ordering::SeqCst);
        if gone {
            waited.wait();
        }
        match (request.path.as_str(), gone) {
            ("/gone.png", true) => Reply::status(404, Vec::new(), &[]),
            ("/down.png", true) => Reply::status(503, Vec::new(), &[]),
            _ => Reply::ok(
                png(100),
                &[("Cache-Control", "max-age=0"), ("ETag", "\"v1\"")],
            ),
        }
    });
    let dir = empty_dir("revalidated-404");
    let (gone, down) = (server.url("/gone.png"), server.url("/down.png"));
    let first = disk_cache(&dir);
    ready(&first, &gone);
    ready(&first, &down);
    removed.store(true, Ordering::SeqCst);

    // The stale responses draw at once while they revalidate.
    let restarted = disk_cache(&dir);
    ready(&restarted, &gone);
    ready(&restarted, &down);
    gate.open();
    wait_until(|| matches!(restarted.image(&gone), ImageState::Failed(_)));
    wait_until(|| server.count("/down.png") == 2);
    assert!(matches!(restarted.image(&down), ImageState::Ready(_)));
    let stored = files(&dir);
    assert_eq!(stored.len(), 2, "{stored:?}");
    assert!(stored
        .iter()
        .all(|path| path.to_string_lossy().contains("down")));

    assert!(matches!(
        settled(&disk_cache(&dir), &gone),
        ImageState::Failed(_)
    ));
    assert_eq!(server.count("/gone.png"), 3);
}

/// A temporary file left by an interrupted write is never read as a stored response.
#[test]
fn a_leftover_temporary_file_is_ignored() {
    let server = TestServer::start(|_| Reply::ok(png(70), &[("Cache-Control", "max-age=3600")]));
    let dir = empty_dir("leftover");
    let url = server.url("/tile.png");
    ready(&disk_cache(&dir), &url);
    let stored = files(&dir);
    let body = stored
        .iter()
        .find(|path| !path.to_string_lossy().ends_with("~meta.json"))
        .unwrap()
        .clone();
    let mut temp = body.clone().into_os_string();
    temp.push("~tmp1-0");
    std::fs::write(&temp, b"partial").unwrap();

    // Beside a stored response, the temporary file isn't read.
    assert_eq!(ready(&disk_cache(&dir), &url).data, [70, 0, 0, 255]);
    assert_eq!(server.count("/tile.png"), 1);
    // In place of one, the response is fetched again.
    for path in stored {
        std::fs::remove_file(path).unwrap();
    }
    assert_eq!(ready(&disk_cache(&dir), &url).data, [70, 0, 0, 255]);
    assert_eq!(server.count("/tile.png"), 2);
}
