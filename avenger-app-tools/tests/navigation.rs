//! Navigation gestures: drag to pan, scroll to zoom, pause or release to settle,
//! and double-click to restore the home view.

use std::time::Duration;

use arrow_array::{ArrayRef, TimestampMillisecondArray};
use async_trait::async_trait;
use avenger_app::app::{AppBuild, AppBuilder, AvengerApp, Scene, Scenes};
use avenger_app::error::AvengerAppError;
use avenger_app_tools::navigation::{KeyedMapPanZoom, KeyedPanZoom, MapPanZoom, PanZoom};
use avenger_common::cursor::CursorIcon;
use avenger_common::time::Instant;
use avenger_eventstream::stream::Rebuild;
use avenger_eventstream::window::{
    ElementState, MouseButton, MouseScrollDelta, WindowCursorMoved, WindowEvent, WindowMouseInput,
    WindowMouseWheel,
};
use avenger_geo::projections::{Equirectangular, Orthographic};
use avenger_geo::{PanMode, Projection};
use avenger_layout::Rect;
use avenger_scales::scales::linear::LinearScale;
use avenger_scales::scales::time::TimeScale;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::scene_graph::SceneGraph;
use std::sync::Arc;

const PLOT: &str = "plot";

/// The plot rectangle. The x scale spans its width and the y scale increases upward.
const AREA: Rect = Rect::new(50.0, 50.0, 200.0, 100.0);

/// The two panels of keyed navigation, side by side.
const LEFT: Rect = Rect::new(50.0, 50.0, 200.0, 100.0);
const RIGHT: Rect = Rect::new(280.0, 50.0, 200.0, 100.0);

/// One day in epoch milliseconds, the domain of the time scale.
const DAY: f64 = 86_400_000.0;

/// Map interaction rectangles.
const GLOBE: Rect = Rect::new(0.0, 0.0, 200.0, 200.0);
const WORLD: Rect = Rect::new(200.0, 0.0, 300.0, 200.0);

struct Views {
    x: ConfiguredScale,
    y: ConfiguredScale,
    globe: Option<Projection>,
    world: Option<Projection>,
    /// The x scales of the left and right panels.
    panels: [ConfiguredScale; 2],
    /// A time scale over one day across the plot.
    time: ConfiguredScale,
    /// The keys of the panels whose views settled, in order.
    settled: Vec<usize>,
    /// Records whether each plot build also rebuilt its tree.
    builds: Vec<bool>,
}

impl Default for Views {
    fn default() -> Self {
        Self {
            x: LinearScale::configured((0.0, 100.0), (0.0, 200.0)),
            y: LinearScale::configured((0.0, 10.0), (100.0, 0.0)),
            globe: Some(Projection::new(Orthographic).with_view(100.0, [100.0, 100.0])),
            world: Some(Projection::new(Equirectangular).with_view(50.0, [150.0, 100.0])),
            panels: [0, 1].map(|_| LinearScale::configured((0.0, 100.0), (0.0, 200.0))),
            time: {
                let instant = |millis: i64| -> ArrayRef {
                    Arc::new(TimestampMillisecondArray::from(vec![millis]))
                };
                TimeScale::configured((instant(0), instant(DAY as i64)), (0.0, 200.0))
            },
            settled: Vec::new(),
            builds: Vec::new(),
        }
    }
}

struct Plot;

#[async_trait]
impl AppBuild<Views> for Plot {
    fn scenes(&self) -> Vec<Scene<Views>> {
        vec![Scene::new(PLOT, 0)]
    }

    async fn build(
        &self,
        state: &mut Views,
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError> {
        if !scenes.requested(PLOT) {
            return Ok(());
        }
        state.builds.push(scenes.requested_tree(PLOT));
        let [width, height] = scenes.size();
        let scene = SceneGraph {
            marks: Vec::new(),
            width,
            height,
            origin: [0.0; 2],
        };
        scenes.set(PLOT, scene.into())
    }
}

/// Create a plot app with the supplied tools and clear its build history.
async fn app(tools: impl FnOnce(AppBuilder<Views>) -> AppBuilder<Views>) -> AvengerApp<Views> {
    let engine = avenger_typst_label::LabelEngine::new(Default::default());
    let builder = AvengerApp::builder(Views::default(), Plot, engine).size([500.0, 300.0]);
    let mut app = tools(builder).build().await.unwrap();
    app.state_mut().builds.clear();
    app
}

fn pan_zoom() -> PanZoom<Views> {
    PanZoom::xy(PLOT, |_| AREA, |s: &mut Views| &mut s.x, |s| &mut s.y)
}

/// Send events 10 ms apart so the 8 ms throttle accepts each move.
async fn send(app: &mut AvengerApp<Views>, events: impl IntoIterator<Item = WindowEvent>) {
    for event in events {
        std::thread::sleep(Duration::from_millis(10));
        app.update(&event, Instant::now()).await;
    }
}

/// Wait past the settle delay and dispatch due events.
async fn settle(app: &mut AvengerApp<Views>) {
    std::thread::sleep(Duration::from_millis(150));
    app.refresh().await;
}

fn moved(position: [f32; 2]) -> WindowEvent {
    WindowEvent::CursorMoved(WindowCursorMoved { position })
}

fn mouse(state: ElementState) -> WindowEvent {
    WindowEvent::MouseInput(WindowMouseInput {
        state,
        button: MouseButton::Left,
    })
}

fn drag(from: [f32; 2], to: [f32; 2]) -> [WindowEvent; 4] {
    [
        moved(from),
        mouse(ElementState::Pressed),
        moved(to),
        mouse(ElementState::Released),
    ]
}

fn double_click() -> [WindowEvent; 4] {
    [
        mouse(ElementState::Pressed),
        mouse(ElementState::Released),
        mouse(ElementState::Pressed),
        mouse(ElementState::Released),
    ]
}

fn wheel(lines: f32) -> WindowEvent {
    WindowEvent::MouseWheel(WindowMouseWheel {
        delta: MouseScrollDelta::LineDelta(0.0, lines),
    })
}

/// Return domain values at a window position using the plot scales.
fn values(state: &Views, [x, y]: [f32; 2]) -> [f64; 2] {
    let x = state.x.invert_scalar(f64::from(x - AREA.x)).unwrap();
    let y = state.y.invert_scalar(f64::from(y - AREA.y)).unwrap();
    [x, y]
}

fn width(scale: &ConfiguredScale) -> f64 {
    let (start, end) = scale.numeric_interval_domain().unwrap();
    end - start
}

/// Compare values within the precision of scale inversion through f32 coordinates.
fn assert_close(actual: [f64; 2], expected: [f64; 2]) {
    let gap = (actual[0] - expected[0]).hypot(actual[1] - expected[1]);
    assert!(gap < 1e-5, "{actual:?} isn't {expected:?}");
}

fn assert_same(actual: &Option<Projection>, expected: &Projection) {
    let actual = actual.as_ref().unwrap();
    assert_eq!(
        (actual.rotate, actual.view),
        (expected.rotate, expected.view)
    );
}

/// A drag preserves the values under the pointer and rebuilds the tree on release.
#[tokio::test(flavor = "current_thread")]
async fn a_drag_keeps_the_values_under_the_press_under_the_pointer() {
    let mut app = app(|builder| builder.tool(pan_zoom())).await;
    let press = [100.0, 80.0];
    let pressed = values(app.state(), press);
    send(&mut app, drag(press, [140.0, 100.0])).await;
    assert_close(values(app.state(), [140.0, 100.0]), pressed);
    assert_eq!(app.state().builds, [false, true]);
}

/// Wheel zoom preserves the value under the pointer, caps movement at two notches,
/// and rebuilds the tree after settling.
#[tokio::test(flavor = "current_thread")]
async fn the_wheel_zooms_about_the_pointer_by_at_most_two_notches() {
    let mut app = app(|builder| builder.tool(pan_zoom())).await;
    let at = [100.0, 80.0];
    let under = values(app.state(), at);
    send(&mut app, [moved(at), wheel(1.0)]).await;
    assert!((width(&app.state().x) - 100.0 / 1.2).abs() < 1e-9);
    send(&mut app, [wheel(5.0)]).await;
    assert!((width(&app.state().x) - 100.0 / 1.2_f64.powi(3)).abs() < 1e-9);
    assert_close(values(app.state(), at), under);
    settle(&mut app).await;
    assert_eq!(app.state().builds, [false, false, true]);
}

/// A pause rebuilds the drag's tree. Releasing without further movement requests no rebuild.
#[tokio::test(flavor = "current_thread")]
async fn a_paused_drag_rests_its_view() {
    let mut app = app(|builder| builder.tool(pan_zoom())).await;
    let [hover, press, to, release] = drag([100.0, 80.0], [140.0, 100.0]);
    send(&mut app, [hover, press, to]).await;
    settle(&mut app).await;
    assert_eq!(app.state().builds, [false, true]);
    send(&mut app, [release]).await;
    assert_eq!(app.state().builds, [false, true]);
}

/// Navigation requests an open hand over its area and a closed hand throughout a drag.
/// Releasing outside the area restores the default cursor.
#[tokio::test(flavor = "current_thread")]
async fn the_cursor_grabs_over_the_area_and_during_a_drag() {
    let mut app = app(|builder| builder.tool(pan_zoom())).await;
    let [hover, press, off, release] = drag([100.0, 80.0], [300.0, 200.0]);
    send(&mut app, [hover]).await;
    assert_eq!(app.cursor(), CursorIcon::Grab);
    send(&mut app, [press, off]).await;
    assert_eq!(app.cursor(), CursorIcon::Grabbing);
    send(&mut app, [release]).await;
    assert_eq!(app.cursor(), CursorIcon::Default);
}

/// A pending hover timer does not settle a later wheel event early.
#[tokio::test(flavor = "current_thread")]
async fn a_pointer_pause_does_not_settle_the_wheel() {
    let mut app =
        app(|builder| builder.tool(pan_zoom().with_settle_delay(Duration::from_millis(400)))).await;
    send(&mut app, [moved([100.0, 80.0])]).await;
    std::thread::sleep(Duration::from_millis(300));
    send(&mut app, [wheel(1.0)]).await;
    std::thread::sleep(Duration::from_millis(130));
    app.refresh().await;
    assert_eq!(app.state().builds, [false]);
}

/// A double-click restores the scales' domains at the tool's first gesture, with their tree.
#[tokio::test(flavor = "current_thread")]
async fn a_double_click_restores_the_first_domains() {
    let mut app = app(|builder| builder.tool(pan_zoom())).await;
    let first = app.state().x.numeric_interval_domain().unwrap();
    send(&mut app, drag([100.0, 80.0], [140.0, 100.0])).await;
    send(&mut app, double_click()).await;
    assert_eq!(app.state().x.numeric_interval_domain().unwrap(), first);
    assert_eq!(app.state().builds, [false, true, true]);
}

/// With a home, a double-click restores the home scales' domains, whatever the view was at the
/// tool's first gesture.
#[tokio::test(flavor = "current_thread")]
async fn a_double_click_restores_the_home_domains() {
    let home = |_: &Views| {
        [
            Some(LinearScale::configured((0.0, 100.0), (0.0, 200.0))),
            None,
        ]
    };
    let mut app = app(|builder| builder.tool(pan_zoom().with_home(home))).await;
    // Another tool, such as a box zoom, changes the view before the first gesture.
    app.state_mut().x = app.state().x.clone().with_domain_interval((20.0, 40.0));
    app.state_mut().y = app.state().y.clone().with_domain_interval((2.0, 4.0));
    send(&mut app, [moved([100.0, 80.0])]).await;
    send(&mut app, double_click()).await;
    assert_eq!(
        app.state().x.numeric_interval_domain().unwrap(),
        (0.0, 100.0)
    );
    assert_eq!(app.state().y.numeric_interval_domain().unwrap(), (2.0, 4.0));
}

/// Two maps on one scene pan independently in their own areas and modes.
#[tokio::test(flavor = "current_thread")]
async fn each_map_pans_in_its_own_area_and_mode() {
    let mut app = app(|builder| {
        builder
            .tool(MapPanZoom::new(
                PLOT,
                |_| GLOBE,
                |s: &mut Views| s.globe.as_mut(),
                PanMode::Sphere,
            ))
            .tool(MapPanZoom::new(
                PLOT,
                |_| WORLD,
                |s: &mut Views| s.world.as_mut(),
                PanMode::Longitude,
            ))
    })
    .await;
    let globe = app.state().globe.clone().unwrap();
    let world = app.state().world.clone().unwrap();

    send(&mut app, drag([100.0, 100.0], [130.0, 90.0])).await;
    let panned = globe.pan(PanMode::Sphere, [100.0, 100.0], [130.0, 90.0]);
    assert_same(&app.state().globe, &panned.unwrap());
    assert_same(&app.state().world, &world);

    send(&mut app, drag([350.0, 100.0], [330.0, 110.0])).await;
    let panned = world.pan(PanMode::Longitude, [150.0, 100.0], [130.0, 110.0]);
    assert_same(&app.state().world, &panned.unwrap());
}

/// One keyed tool pans each map in its own area and mode, the wheel zooms only the map under the
/// pointer, and a double-click restores only its own map.
#[tokio::test(flavor = "current_thread")]
async fn keyed_maps_move_only_their_own_projections() {
    let maps = KeyedMapPanZoom::new(
        PLOT,
        |_| vec![(0, GLOBE), (1, WORLD)],
        |s: &mut Views, key: &usize| match key {
            0 => s.globe.as_mut(),
            _ => s.world.as_mut(),
        },
        |_, key| match key {
            0 => PanMode::Sphere,
            _ => PanMode::Longitude,
        },
    );
    let mut app = app(|builder| builder.tool(maps)).await;
    let globe = app.state().globe.clone().unwrap();
    let world = app.state().world.clone().unwrap();

    send(&mut app, drag([100.0, 100.0], [130.0, 90.0])).await;
    let panned = globe.pan(PanMode::Sphere, [100.0, 100.0], [130.0, 90.0]);
    let panned = panned.unwrap();
    assert_same(&app.state().globe, &panned);
    assert_same(&app.state().world, &world);

    send(&mut app, [moved([350.0, 100.0]), wheel(1.0)]).await;
    let zoomed = world.zoom(PanMode::Longitude, [150.0, 100.0], 1.2);
    let zoomed = zoomed.unwrap();
    assert_same(&app.state().world, &zoomed);
    assert_same(&app.state().globe, &panned);

    send(&mut app, [moved([100.0, 100.0])]).await;
    send(&mut app, double_click()).await;
    assert_same(&app.state().globe, &globe);
    assert_same(&app.state().world, &zoomed);
}

/// Wheel zoom stops at the maximum scale without rebuilding on further zoom-in events.
#[tokio::test(flavor = "current_thread")]
async fn the_wheel_stops_at_the_scale_limits() {
    let mut app = app(|builder| {
        let world = MapPanZoom::new(
            PLOT,
            |_| WORLD,
            |s: &mut Views| s.world.as_mut(),
            PanMode::Longitude,
        );
        builder.tool(world.with_scale_limits(|_, _| (50.0, 65.0)))
    })
    .await;
    let scale = |app: &AvengerApp<Views>| app.state().world.as_ref().unwrap().view.unwrap().scale;
    send(&mut app, [moved([350.0, 100.0]), wheel(1.0)]).await;
    assert!((scale(&app) - 60.0).abs() < 1e-9);
    send(&mut app, [wheel(1.0), wheel(1.0)]).await;
    assert!((scale(&app) - 65.0).abs() < 1e-9);
    assert_eq!(app.state().builds, [false, false]);
}

/// A tool that pans the x scale of each panel and records the panels whose views settle.
fn panels() -> KeyedPanZoom<Views, usize> {
    KeyedPanZoom::x(
        PLOT,
        |_| vec![(0, LEFT), (1, RIGHT)],
        |s: &mut Views, key: &usize| s.panels.get_mut(*key),
    )
    .on_settle(|s, key| {
        s.settled.push(*key);
        Rebuild::none()
    })
}

fn panel_domains(app: &AvengerApp<Views>) -> [(f64, f64); 2] {
    app.state()
        .panels
        .each_ref()
        .map(|scale| scale.numeric_interval_domain().unwrap())
}

/// A drag pans only the scale of the panel that it starts in, the wheel zooms only the scale of
/// the panel under the pointer, and a double-click restores only its own panel.
#[tokio::test(flavor = "current_thread")]
async fn keyed_areas_move_only_their_own_scales() {
    let mut app = app(|builder| builder.tool(panels())).await;
    send(&mut app, drag([100.0, 80.0], [150.0, 80.0])).await;
    let [left, right] = panel_domains(&app);
    assert_close([left.0, left.1], [-25.0, 75.0]);
    assert_eq!(right, (0.0, 100.0));

    send(&mut app, [moved([380.0, 80.0]), wheel(1.0)]).await;
    let [left_now, right] = panel_domains(&app);
    assert_eq!(left_now, left);
    assert!((right.1 - right.0 - 100.0 / 1.2).abs() < 1e-9);

    send(&mut app, [moved([100.0, 80.0])]).await;
    send(&mut app, double_click()).await;
    let [left, right_now] = panel_domains(&app);
    assert_eq!(left, (0.0, 100.0));
    assert_eq!(right_now, right);
}

/// Each gesture that changed a view settles one time with its key: a release, a stopped wheel,
/// and a double-click. A click that pans nothing settles nothing.
#[tokio::test(flavor = "current_thread")]
async fn settled_views_report_their_keys() {
    let mut app = app(|builder| builder.tool(panels())).await;
    // The click stands apart from the next press, so the two are no double-click.
    send(&mut app, drag([200.0, 120.0], [202.0, 120.0])).await;
    assert!(app.state().settled.is_empty());
    send(&mut app, drag([100.0, 80.0], [150.0, 80.0])).await;
    assert_eq!(app.state().settled, [0]);
    send(&mut app, [moved([380.0, 80.0]), wheel(1.0)]).await;
    settle(&mut app).await;
    assert_eq!(app.state().settled, [0, 1]);
    send(&mut app, [moved([100.0, 80.0])]).await;
    send(&mut app, double_click()).await;
    assert_eq!(app.state().settled, [0, 1, 0]);
}

/// Bounds stop a pan at their ends and a zoom out at their width, and a minimum span stops a
/// zoom in.
#[tokio::test(flavor = "current_thread")]
async fn bounds_and_a_minimum_span_limit_the_view() {
    let limited = PanZoom::x(PLOT, |_| AREA, |s: &mut Views| &mut s.x)
        .bounds(|_| [Some([0.0, 100.0]), None])
        .min_span([Some(20.0), None]);
    let mut app = app(|builder| builder.tool(limited)).await;
    let domain = |app: &AvengerApp<Views>| app.state().x.numeric_interval_domain().unwrap();
    // The domain already spans the bounds, so it can neither pan nor zoom out.
    send(&mut app, drag([100.0, 80.0], [150.0, 80.0])).await;
    assert_eq!(domain(&app), (0.0, 100.0));
    send(&mut app, [moved([100.0, 80.0]), wheel(-1.0)]).await;
    assert_eq!(domain(&app), (0.0, 100.0));
    // Zooming in stops at a span of 20: five turns of two notches would shrink it to 16.
    send(&mut app, [2.0; 6].map(wheel)).await;
    let (start, end) = domain(&app);
    assert!((end - start - 20.0).abs() < 1e-9, "{start} {end}");
    // A pan toward larger values, of 64 units at 10 px per unit, stops at the upper bound.
    send(&mut app, drag([200.0, 80.0], [-440.0, 80.0])).await;
    let (start, end) = domain(&app);
    assert!(
        (end - 100.0).abs() < 1e-9 && (start - 80.0).abs() < 1e-9,
        "{start} {end}"
    );
}

/// A drag pans a time scale and keeps the instant under the press under the pointer.
#[tokio::test(flavor = "current_thread")]
async fn a_drag_pans_a_time_scale() {
    let mut app =
        app(|builder| builder.tool(PanZoom::x(PLOT, |_| AREA, |s: &mut Views| &mut s.time))).await;
    let at = |app: &AvengerApp<Views>, x: f32| {
        app.state()
            .time
            .invert_scalar(f64::from(x - AREA.x))
            .unwrap()
    };
    let pressed = at(&app, 100.0);
    send(&mut app, drag([100.0, 80.0], [150.0, 80.0])).await;
    assert!((at(&app, 150.0) - pressed).abs() < 1.0);
    // A quarter of the plot is a quarter of the day.
    let moved = app.state().time.invert_scalar(0.0).unwrap();
    assert!((moved + DAY / 4.0).abs() < 1.0, "{moved}");
}
