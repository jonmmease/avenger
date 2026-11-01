//! Interval and lasso brushes over a polar plot's projection select ranges and polygons of angle and
//! radius through the plot's theta and radius scales.

use std::sync::Arc;
use std::time::Duration;

use arrow_array::cast::AsArray;
use arrow_array::types::Int64Type;
use arrow_array::{ArrayRef, Int64Array, RecordBatch};
use async_trait::async_trait;
use avenger_app::app::{AppBuild, AvengerApp, BuiltScene, Scene, Scenes};
use avenger_app::error::AvengerAppError;
use avenger_app_tools::brushes::{
    Brush, Brushes, Instances, Interval, IntervalBrush, Lasso, LassoBrush,
};
use avenger_common::cursor::CursorIcon;
use avenger_common::time::Instant;
use avenger_common::types::SymbolShape;
use avenger_common::value::ScalarOrArray;
use avenger_coords::{PlotProjection, PolarProjection};
use avenger_eventstream::stream::Rebuild;
use avenger_eventstream::window::{
    ElementState, MouseButton, WindowCursorMoved, WindowEvent, WindowMouseInput,
};
use avenger_geometry::data::{Datum, SceneData};
use avenger_layout::Rect;
use avenger_scales::scales::linear::LinearScale;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::symbol::SceneSymbolMark;
use avenger_scenegraph::scene_graph::SceneGraph;

const PLOT: &str = "plot";
const BRUSHES: &str = "brushes";
const POINTS: &str = "points";

/// The square of the polar plot, whose circle of radius 100 fills it.
const AREA: Rect = Rect::new(100.0, 50.0, 200.0, 200.0);

struct Polar {
    /// Hours of a clock around the circle, from the top.
    theta: ConfiguredScale,
    /// From the center to the rim.
    radius: ConfiguredScale,
    /// Whether the plot has a projection.
    projected: bool,
    selection: Option<Interval>,
    settles: Vec<Option<Interval>>,
    lasso: Lasso<i64>,
    /// The points that the plot draws, each with its ID and its theta and radius.
    points: Vec<(i64, [f64; 2])>,
}

impl Default for Polar {
    fn default() -> Self {
        Self {
            theta: LinearScale::configured((0.0, 12.0), (0.0, 360.0)),
            radius: LinearScale::configured((0.0, 10.0), (0.0, 100.0)),
            projected: true,
            selection: None,
            settles: Vec::new(),
            lasso: Lasso::default(),
            points: Vec::new(),
        }
    }
}

/// The projection of the polar plot, a whole circle from the top.
fn polar() -> Arc<dyn PlotProjection> {
    Arc::new(PolarProjection::new(
        [100.0, 100.0],
        [0.0, 100.0],
        [0.0, 360.0],
    ))
}

fn projection(state: &Polar) -> Option<Arc<dyn PlotProjection>> {
    state.projected.then(polar)
}

/// The scene pixel at `angle` degrees clockwise from the top and `radius` pixels from the
/// center.
fn at(angle: f32, radius: f32) -> [f32; 2] {
    let (sin, cos) = angle.to_radians().sin_cos();
    [200.0 + radius * sin, 150.0 - radius * cos]
}

struct Plot;

#[async_trait]
impl AppBuild<Polar> for Plot {
    fn scenes(&self) -> Vec<Scene<Polar>> {
        vec![Scene::new(PLOT, 0)]
    }

    async fn build(
        &self,
        state: &mut Polar,
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError> {
        if !scenes.requested(PLOT) {
            return Ok(());
        }
        let centers: Vec<[f32; 2]> = (state.points.iter())
            .map(|(_, [theta, radius])| at(*theta as f32 * 30.0, *radius as f32 * 10.0))
            .collect();
        let symbols = SceneSymbolMark {
            name: POINTS.to_string(),
            len: centers.len() as u32,
            x: ScalarOrArray::new_array(centers.iter().map(|c| c[0]).collect()),
            y: ScalarOrArray::new_array(centers.iter().map(|c| c[1]).collect()),
            shapes: vec![SymbolShape::Circle],
            size: 30.0.into(),
            ..Default::default()
        };
        let ids: ArrayRef = Arc::new(Int64Array::from_iter_values(
            state.points.iter().map(|(id, _)| *id),
        ));
        let mut data = SceneData::new();
        data.insert(
            POINTS,
            Arc::new(RecordBatch::try_from_iter([("id", ids)]).unwrap()),
        );
        let [width, height] = scenes.size();
        let scene = SceneGraph {
            marks: vec![symbols.into()],
            width,
            height,
            origin: [0.0; 2],
        };
        scenes.set(PLOT, BuiltScene { scene, data })
    }
}

/// `brush` over the polar plot, with one box.
fn boxes(brush: IntervalBrush<Polar>) -> IntervalBrush<Polar> {
    brush
        .area(|_| AREA)
        .projection(projection)
        .x_scale(|s| s.theta.clone())
        .y_scale(|s| s.radius.clone())
        .value(|s| s.selection)
        .on_change(|s, selection| {
            s.selection = selection;
            Rebuild::scene(PLOT)
        })
        .on_settle(|s, selection| {
            s.settles.push(selection);
            Rebuild::tree(PLOT)
        })
}

fn id(datum: &Datum) -> Option<i64> {
    let ids = datum.rows.column_by_name("id")?.as_primitive::<Int64Type>();
    Some(ids.value(datum.row))
}

fn lasso() -> LassoBrush<Polar, i64> {
    LassoBrush::new("lasso")
        .area(|_: &Polar| AREA)
        .projection(projection)
        .x_scale(|s| s.theta.clone())
        .y_scale(|s| s.radius.clone())
        .instances(Instances::marks([POINTS]).key(id))
        .value(|s| s.lasso.clone())
        .on_change(|s, lasso| {
            s.lasso = lasso;
            Rebuild::tree(PLOT)
        })
}

async fn app(state: Polar, brush: impl Into<Brush<Polar>>) -> AvengerApp<Polar> {
    let engine = avenger_typst_label::LabelEngine::new(Default::default());
    let brushes = Brushes::new(BRUSHES, 10).with(brush);
    let builder = AvengerApp::builder(state, Plot, engine)
        .size([400.0, 300.0])
        .tool(brushes);
    builder.build().await.unwrap()
}

async fn send(app: &mut AvengerApp<Polar>, events: impl IntoIterator<Item = WindowEvent>) {
    for event in events {
        std::thread::sleep(Duration::from_millis(10));
        app.update(&event, Instant::now()).await;
    }
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

/// A drag from `from` to `to` in moves of at most 10 pixels.
fn drag(from: [f32; 2], to: [f32; 2]) -> Vec<WindowEvent> {
    let mut events = vec![moved(from), mouse(ElementState::Pressed)];
    let steps = ((to[0] - from[0]).hypot(to[1] - from[1]) / 10.0)
        .ceil()
        .max(1.0) as usize;
    for step in 1..=steps {
        let t = step as f32 / steps as f32;
        events.push(moved([
            from[0] + t * (to[0] - from[0]),
            from[1] + t * (to[1] - from[1]),
        ]));
    }
    events.push(mouse(ElementState::Released));
    events
}

/// A drag through `corners` that releases at the first corner.
fn trace(corners: &[[f32; 2]]) -> Vec<WindowEvent> {
    let mut events = vec![moved(corners[0]), mouse(ElementState::Pressed)];
    for (from, to) in corners
        .iter()
        .zip(corners.iter().skip(1).chain([&corners[0]]))
    {
        let mut segment = drag(*from, *to);
        segment.retain(|event| matches!(event, WindowEvent::CursorMoved(_)));
        events.extend(segment.into_iter().skip(1));
    }
    events.push(mouse(ElementState::Released));
    events
}

/// Whether `a` and `b` select the same ranges, up to rounding.
fn same(a: Option<Interval>, b: Interval) -> bool {
    let axis = |a: Option<[f64; 2]>, b: Option<[f64; 2]>| match (a, b) {
        (Some(a), Some(b)) => (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4,
        (a, b) => a.is_none() && b.is_none(),
    };
    a.is_some_and(|a| axis(a.x, b.x) && axis(a.y, b.y))
}

/// The points of the drawn box's fill, in scene coordinates.
fn fill(app: &AvengerApp<Polar>) -> Vec<[f32; 2]> {
    let groups = app.scenes()[1].marks.iter().filter_map(|mark| match mark {
        SceneMark::Group(group) => Some(group),
        _ => None,
    });
    let path = groups
        .flat_map(|group| &group.marks)
        .find_map(|mark| match mark {
            SceneMark::Path(path) if path.name == "brush_box" => path.path.first().cloned(),
            _ => None,
        })
        .unwrap();
    let points = path.iter().map(|event| event.to());
    points.map(|p| [AREA.x + p.x, AREA.y + p.y]).collect()
}

/// A drag selects the hours and the radii between its press and its release, and draws an annular
/// sector whose points lie between the two radii.
#[tokio::test(flavor = "current_thread")]
async fn a_drag_selects_an_annular_sector() {
    let mut app = app(Polar::default(), boxes(IntervalBrush::xy("box"))).await;
    send(&mut app, drag(at(30.0, 40.0), at(60.0, 70.0))).await;
    let sector = Interval {
        x: Some([1.0, 2.0]),
        y: Some([4.0, 7.0]),
    };
    let selection = app.state().selection;
    assert!(same(selection, sector), "{selection:?}");
    assert_eq!(app.state().settles, [selection]);
    let fill = fill(&app);
    assert!(fill.len() > 8, "the sector's arcs bend");
    for [x, y] in fill {
        let radius = (x - 200.0).hypot(y - 150.0);
        assert!((40.0 - 1e-3..=70.0 + 1e-3).contains(&radius), "{radius}");
    }
}

/// A drag across the top of the circle selects a range of hours across the wrap, whose first
/// value exceeds its second.
#[tokio::test(flavor = "current_thread")]
async fn a_drag_across_the_top_selects_across_the_wrap() {
    let mut app = app(Polar::default(), boxes(IntervalBrush::xy("box"))).await;
    send(&mut app, drag(at(330.0, 50.0), at(30.0, 80.0))).await;
    let selection = app.state().selection.unwrap();
    let across = Interval {
        x: Some([11.0, 1.0]),
        y: Some([5.0, 8.0]),
    };
    assert!(same(Some(selection), across), "{selection:?}");
    assert!(selection.contains([11.5, 6.0]));
    assert!(selection.contains([0.5, 6.0]));
    assert!(!selection.contains([6.0, 6.0]));
}

/// A radius brush selects a ring, whose outer edge a drag outward moves, with the cursor of the
/// direction in which the edge moves.
#[tokio::test(flavor = "current_thread")]
async fn a_radius_brush_selects_a_ring() {
    let mut app = app(Polar::default(), boxes(IntervalBrush::y("ring"))).await;
    send(&mut app, drag(at(45.0, 30.0), at(45.0, 60.0))).await;
    let ring = Interval {
        x: None,
        y: Some([3.0, 6.0]),
    };
    assert!(same(app.state().selection, ring));

    send(&mut app, [moved(at(90.0, 60.0))]).await;
    assert_eq!(app.cursor(), CursorIcon::EwResize);
    send(&mut app, [moved(at(0.0, 60.0))]).await;
    assert_eq!(app.cursor(), CursorIcon::NsResize);
    send(&mut app, drag(at(180.0, 60.0), at(180.0, 90.0))).await;
    let wider = Interval {
        x: None,
        y: Some([3.0, 9.0]),
    };
    assert!(same(app.state().selection, wider));
}

/// A lasso selects the points inside its path, and reports its polygon in hours and radii.
#[tokio::test(flavor = "current_thread")]
async fn a_lasso_selects_in_hours_and_radii() {
    let state = Polar {
        points: vec![(1, [1.5, 5.0]), (2, [1.5, 9.0]), (3, [7.0, 5.0])],
        ..Polar::default()
    };
    let mut app = app(state, lasso()).await;
    let corners = [
        at(30.0, 30.0),
        at(60.0, 30.0),
        at(60.0, 70.0),
        at(30.0, 70.0),
    ];
    send(&mut app, trace(&corners)).await;
    let lasso = &app.state().lasso;
    assert_eq!(lasso.keys, [1]);
    let [polygon] = &lasso.polygons[..] else {
        panic!("the lasso holds one polygon");
    };
    for [theta, radius] in polygon {
        assert!((0.9..=2.1).contains(theta), "{theta}");
        assert!((2.5..=7.5).contains(radius), "{radius}");
    }
}

/// Whether one of `polygons` holds `point` on the plane, by the even-odd rule.
fn holds(polygons: &[Vec<[f64; 2]>], [x, y]: [f64; 2]) -> bool {
    polygons.iter().any(|ring| {
        let mut inside = false;
        for (index, &[xi, yi]) in ring.iter().enumerate() {
            let [xj, yj] = ring[(index + ring.len() - 1) % ring.len()];
            if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
                inside = !inside;
            }
        }
        inside
    })
}

/// A lasso across the top of the circle reports a polygon on each side of twelve o'clock, so a
/// planar test of hours and radii selects the points inside it, as its keys do.
#[tokio::test(flavor = "current_thread")]
async fn a_lasso_across_the_top_splits_at_twelve() {
    let state = Polar {
        points: vec![(1, [11.5, 5.0]), (2, [0.5, 5.0]), (3, [6.0, 5.0])],
        ..Polar::default()
    };
    let mut app = app(state, lasso()).await;
    let corners = [
        at(330.0, 30.0),
        at(30.0, 30.0),
        at(30.0, 70.0),
        at(330.0, 70.0),
    ];
    send(&mut app, trace(&corners)).await;
    let lasso = &app.state().lasso;
    let mut keys = lasso.keys.clone();
    keys.sort();
    assert_eq!(keys, [1, 2]);
    assert_eq!(lasso.polygons.len(), 2);
    assert!(holds(&lasso.polygons, [11.5, 5.0]) && holds(&lasso.polygons, [0.5, 5.0]));
    assert!(!holds(&lasso.polygons, [6.0, 5.0]));
}

/// A lasso around the center reports a polygon that runs from the center to its path at every
/// hour, and draws the disk that it holds.
#[tokio::test(flavor = "current_thread")]
async fn a_lasso_around_the_center_closes_through_it() {
    let state = Polar {
        points: vec![(1, [1.0, 2.0]), (2, [7.0, 2.0]), (3, [4.0, 9.0])],
        ..Polar::default()
    };
    let mut app = app(state, lasso()).await;
    let corners = [
        at(315.0, 50.0),
        at(45.0, 50.0),
        at(135.0, 50.0),
        at(225.0, 50.0),
    ];
    send(&mut app, trace(&corners)).await;
    let lasso = app.state().lasso.clone();
    let mut keys = lasso.keys.clone();
    keys.sort();
    assert_eq!(keys, [1, 2]);
    for hour in [0.5, 3.0, 6.0, 9.0, 11.5] {
        assert!(holds(&lasso.polygons, [hour, 1.0]), "{hour}");
        assert!(!holds(&lasso.polygons, [hour, 9.0]), "{hour}");
    }
    // The drawn lasso covers the center and every side of it.
    send(&mut app, [moved(at(0.0, 90.0))]).await;
    let groups = app.scenes()[1].marks.iter().filter_map(|mark| match mark {
        SceneMark::Group(group) => Some(group),
        _ => None,
    });
    let fill = groups
        .flat_map(|group| &group.marks)
        .find_map(|mark| match mark {
            SceneMark::Path(path) if path.name == "brush_lasso" => path.path.first().cloned(),
            _ => None,
        })
        .expect("the brush draws the lasso");
    let points: Vec<[f32; 2]> = fill
        .iter()
        .map(|event| [event.to().x, event.to().y])
        .collect();
    let reach = |pick: fn(&[f32; 2]) -> f32| points.iter().map(pick).fold(f32::MIN, f32::max);
    // About 35 pixels out at the sides of the square that it traces, from a center at 100, 100.
    assert!(
        reach(|p| p[0]) > 130.0 && reach(|p| -p[0]) > -70.0,
        "{points:?}"
    );
    assert!(
        reach(|p| p[1]) > 130.0 && reach(|p| -p[1]) > -70.0,
        "{points:?}"
    );
}

/// A plot without a projection, such as a 3D plot, takes no box.
#[tokio::test(flavor = "current_thread")]
async fn a_plot_without_a_projection_takes_no_box() {
    let state = Polar {
        projected: false,
        ..Polar::default()
    };
    let mut app = app(state, boxes(IntervalBrush::xy("box"))).await;
    send(&mut app, [moved(at(30.0, 40.0))]).await;
    assert_eq!(app.cursor(), CursorIcon::Default);
    send(&mut app, drag(at(30.0, 40.0), at(60.0, 70.0))).await;
    assert_eq!(app.state().selection, None);
}
