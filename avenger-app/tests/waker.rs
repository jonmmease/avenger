use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use avenger_app::app::{AppBuild, AvengerApp, Changes, Scene, Scenes};
use avenger_app::error::AvengerAppError;
use avenger_app::waker::{AppWaker, Refresh};
use avenger_eventstream::manager::{EventStreamHandler, SceneTrees};
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{DebounceConfig, EventStreamConfig, Rebuild};
use avenger_eventstream::window::{WindowCursorMoved, WindowEvent};
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
    /// The x positions of the moves that the handler saw.
    moves: Vec<f32>,
}

/// Draws one scene, counts builds, and optionally registers a `Record` stream.
struct Builder(Option<EventStreamConfig>);

#[async_trait]
impl AppBuild<State> for Builder {
    fn scenes(&self) -> Vec<Scene<State>> {
        let chart = Scene::new("chart", 0);
        vec![match &self.0 {
            Some(config) => chart.stream(config.clone(), Record),
            None => chart,
        }]
    }

    async fn build(
        &self,
        state: &mut State,
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError> {
        state.builds += 1;
        if state.request_during_build {
            if let Some(waker) = &state.waker {
                waker.request(Refresh::Rebuild);
            }
        }
        let scene = SceneGraph {
            marks: Vec::new(),
            width: 10.0,
            height: 10.0,
            origin: [0.0; 2],
        };
        scenes.set("chart", scene.into())
    }
}

/// Create a single-scene app with an optional `Record` stream.
async fn app(config: Option<EventStreamConfig>) -> AvengerApp<State> {
    let engine = avenger_typst_label::LabelEngine::new(Default::default());
    let builder = AvengerApp::builder(State::default(), Builder(config), engine);
    let mut app = builder.build().await.unwrap();
    let waker = app.waker();
    app.state_mut().waker = Some(waker);
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
    let mut app = app(None).await;
    let builds = app.state_mut().builds;
    let redraw = Changes {
        redraw: true,
        ..Default::default()
    };
    assert_eq!(app.refresh().await, Changes::default());
    app.waker().request(Refresh::Redraw);
    assert_eq!(app.refresh().await, redraw);
    assert_eq!(app.state_mut().builds, builds);
    app.waker().request(Refresh::Rebuild);
    assert_eq!(app.refresh().await.scenes, [0]);
    assert_eq!(app.state_mut().builds, builds + 1);
    assert_eq!(app.refresh().await, Changes::default());
}

/// A build clears the pending refresh, and a request during the build raises it again.
#[tokio::test(flavor = "current_thread")]
async fn a_request_during_a_build_raises_the_refresh_again() {
    let mut app = app(None).await;
    let waker = app.waker();
    let notified = counted(&waker);
    app.state_mut().request_during_build = true;
    waker.request(Refresh::Rebuild);
    assert_eq!(app.refresh().await.scenes, [0]);
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
    let mut app = app(None).await.with_images(cache.clone());
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
    assert!(app.refresh().await.redraw);
}

/// Records the moves that it sees, and asks to draw again.
struct Record;

#[async_trait]
impl EventStreamHandler<State> for Record {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut State,
        _: &SceneTrees<'_>,
    ) -> Rebuild {
        if let SceneGraphEvent::CursorMoved(event) = event {
            state.moves.push(event.position[0]);
        }
        Rebuild::scene("chart")
    }
}

/// A debounced stream's burst settles through the app's alarm: the handler sees the burst's last
/// event, the refresh builds the scene that the handler asks for, and the alarm rings at most
/// once for each time that it was set for.
#[tokio::test(flavor = "current_thread")]
async fn a_debounced_burst_settles_through_the_alarm() {
    let debounced = EventStreamConfig {
        types: vec![SceneGraphEventType::CursorMoved],
        debounce: Some(DebounceConfig::new(30)),
        ..Default::default()
    };
    let mut app = app(Some(debounced)).await;
    let notified = counted(&app.waker());
    let builds = app.state_mut().builds;
    let start = Instant::now();
    for (x, ms) in [(1.0, 0), (2.0, 10)] {
        let event = WindowEvent::CursorMoved(WindowCursorMoved { position: [x, 0.0] });
        app.update(&event, start + Duration::from_millis(ms)).await;
    }
    // Refresh each time the host hears from the waker, until the burst settles.
    let mut heard = 0;
    while app.state_mut().moves.is_empty() {
        assert!(start.elapsed() < Duration::from_secs(5), "timed out");
        let count = notified.load(Ordering::SeqCst);
        if count > heard {
            heard = count;
            app.refresh().await;
        } else {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    assert_eq!(app.state_mut().moves, [2.0]);
    assert_eq!(app.state_mut().builds, builds + 1);
    assert!(notified.load(Ordering::SeqCst) <= 2);
}
