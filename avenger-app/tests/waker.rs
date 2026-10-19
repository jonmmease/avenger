use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use avenger_app::app::{AvengerApp, Refreshed, SceneGraphBuilder};
use avenger_app::error::AvengerAppError;
use avenger_app::waker::{AppWaker, Refresh};
use avenger_image::{
    FetchResponse, ImageCache, ImageCacheOptions, ImageFetcher, ImageResolver, Validators,
};
use avenger_scenegraph::scene_graph::SceneGraph;

#[derive(Clone, Default)]
struct State {
    waker: Option<AppWaker>,
    builds: usize,
    /// Whether a build requests a rebuild, as background work that the build starts would.
    request_during_build: bool,
}

struct Builder;

#[async_trait]
impl SceneGraphBuilder<State> for Builder {
    async fn build(&self, state: &mut State) -> Result<SceneGraph, AvengerAppError> {
        state.builds += 1;
        if state.request_during_build {
            if let Some(waker) = &state.waker {
                waker.request(Refresh::Rebuild);
            }
        }
        Ok(SceneGraph {
            marks: Vec::new(),
            width: 10.0,
            height: 10.0,
            origin: [0.0; 2],
        })
    }
}

async fn app() -> AvengerApp<State> {
    let mut app = AvengerApp::try_new(
        State::default(),
        Arc::new(Builder),
        Vec::new(),
        avenger_typst_label::LabelEngine::new(Default::default()),
    )
    .await
    .unwrap();
    let waker = app.waker();
    app.app_state_mut().waker = Some(waker);
    app
}

fn counted(waker: &AppWaker) -> Arc<AtomicUsize> {
    let count = Arc::new(AtomicUsize::new(0));
    let counter = count.clone();
    waker.set_notify(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    count
}

/// Requests raise the pending refresh, the host hears only when it rises, and a rebuild wins
/// over a redraw.
#[test]
fn the_host_hears_when_the_pending_refresh_rises() {
    let waker = AppWaker::new();
    let notified = counted(&waker);
    let count = || notified.load(Ordering::SeqCst);
    waker.request(Refresh::Redraw);
    waker.request(Refresh::Redraw);
    assert_eq!(count(), 1);
    waker.clone().request(Refresh::Rebuild);
    waker.request(Refresh::Redraw);
    assert_eq!(count(), 2);
    assert_eq!(waker.take(), Some(Refresh::Rebuild));
    assert_eq!(waker.take(), None);
    waker.request(Refresh::Redraw);
    assert_eq!(count(), 3);
}

/// A refresh does what the pending request asks: nothing, a redraw, or a new scene.
#[tokio::test(flavor = "current_thread")]
async fn a_refresh_reports_a_redraw_or_a_new_scene() {
    let mut app = app().await;
    let builds = app.app_state_mut().builds;
    assert!(matches!(app.refresh().await.unwrap(), Refreshed::Nothing));
    app.waker().request(Refresh::Redraw);
    assert!(matches!(app.refresh().await.unwrap(), Refreshed::Redraw));
    assert_eq!(app.app_state_mut().builds, builds);
    app.waker().request(Refresh::Rebuild);
    assert!(matches!(app.refresh().await.unwrap(), Refreshed::Scene(_)));
    assert_eq!(app.app_state_mut().builds, builds + 1);
    assert!(matches!(app.refresh().await.unwrap(), Refreshed::Nothing));
}

/// A build clears the pending refresh, and a request during the build raises it again.
#[tokio::test(flavor = "current_thread")]
async fn a_request_during_a_build_raises_the_refresh_again() {
    let mut app = app().await;
    let waker = app.waker();
    let notified = counted(&waker);
    app.app_state_mut().request_during_build = true;
    waker.request(Refresh::Rebuild);
    assert!(matches!(app.refresh().await.unwrap(), Refreshed::Scene(_)));
    assert_eq!(notified.load(Ordering::SeqCst), 2);
    assert_eq!(waker.take(), Some(Refresh::Rebuild));
}

/// A fetcher that serves a one-pixel PNG for every URL.
struct PixelFetcher;

impl ImageFetcher for PixelFetcher {
    fn fetch(
        &self,
        _url: &str,
        _: &Validators,
    ) -> Result<FetchResponse, avenger_image::error::AvengerImageError> {
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        Ok(FetchResponse {
            status: 200,
            body: png.into_inner(),
            ..Default::default()
        })
    }
}

/// Images that arrive through the app's cache before the host refreshes wake the host once, with
/// a redraw.
#[tokio::test(flavor = "current_thread")]
async fn a_burst_of_arrivals_wakes_the_host_once() {
    let cache = ImageCache::with_options(ImageCacheOptions {
        fetcher: Arc::new(PixelFetcher),
        ..Default::default()
    });
    let mut app = app().await.with_images(cache.clone());
    let notified = counted(&app.waker());
    let urls = [
        "https://a.test/1.png",
        "https://a.test/2.png",
        "https://a.test/3.png",
    ];
    for url in urls {
        cache.image(url);
    }
    let start = Instant::now();
    while !urls.iter().all(|url| cache.ready(url).is_some()) {
        assert!(start.elapsed() < Duration::from_secs(5), "timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(notified.load(Ordering::SeqCst), 1);
    assert!(matches!(app.refresh().await.unwrap(), Refreshed::Redraw));
}
