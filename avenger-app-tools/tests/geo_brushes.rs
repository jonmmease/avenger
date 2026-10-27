//! Geographic box brushes select ranges of longitude and latitude through map projections, and
//! lasso brushes on maps select spherical polygons.

use std::f64::consts::PI;
use std::sync::Arc;
use std::time::Duration;

use arrow_array::cast::AsArray;
use arrow_array::types::Int64Type;
use arrow_array::{ArrayRef, Int64Array, RecordBatch};
use async_trait::async_trait;
use avenger_app::app::{AppBuild, AvengerApp, BuiltScene, Scene, Scenes};
use avenger_app::error::AvengerAppError;
use avenger_app_tools::brushes::{
    Brush, Brushes, GeoBox, GeoBoxBrush, Instances, Lasso, LassoBrush,
};
use avenger_common::cursor::CursorIcon;
use avenger_common::time::Instant;
use avenger_common::types::SymbolShape;
use avenger_common::value::ScalarOrArray;
use avenger_eventstream::stream::Rebuild;
use avenger_eventstream::window::{
    ElementState, MouseButton, WindowCursorMoved, WindowEvent, WindowMouseInput, WindowResizeEvent,
};
use avenger_geo::projections::{Equirectangular, Orthographic};
use avenger_geo::{polygon_contains, PanMode, Projection};
use avenger_geometry::data::{Datum, SceneData};
use avenger_layout::Rect;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::symbol::SceneSymbolMark;
use avenger_scenegraph::scene_graph::SceneGraph;

const MAP: &str = "map";
const BRUSHES: &str = "brushes";
const PLACES: &str = "places";

/// The map's area.
const AREA: Rect = Rect::new(20.0, 10.0, 360.0, 180.0);

/// An equirectangular map that fills the area at one pixel a degree.
fn flat() -> Projection {
    Projection::new(Equirectangular).with_view(180.0 / PI, [180.0, 90.0])
}

/// A globe of radius 80 at the area's center.
fn globe() -> Projection {
    Projection::new(Orthographic).with_view(80.0, [180.0, 90.0])
}

struct Map {
    projection: Option<Projection>,
    selection: Option<GeoBox>,
    settles: Vec<Option<GeoBox>>,
    lasso: Lasso<i64>,
    /// The places that the map draws, each with its ID.
    places: Vec<(i64, [f64; 2])>,
}

impl Map {
    fn new(projection: Projection) -> Self {
        Self {
            projection: Some(projection),
            selection: None,
            settles: Vec::new(),
            lasso: Lasso::default(),
            places: Vec::new(),
        }
    }
}

/// Whether one of a lasso's `polygons` contains `place`.
fn lassoed(polygons: &[Vec<[f64; 2]>], place: [f64; 2]) -> bool {
    (polygons.iter()).any(|ring| polygon_contains(std::slice::from_ref(ring), place))
}

/// The scene pixel of `[lon, lat]` through `projection`.
fn pixel(projection: &Projection, [lon, lat]: [f64; 2]) -> [f32; 2] {
    let (x, y) = projection.build().unwrap().project_point(lon, lat).unwrap();
    [AREA.x + x as f32, AREA.y + y as f32]
}

struct Places;

#[async_trait]
impl AppBuild<Map> for Places {
    fn scenes(&self) -> Vec<Scene<Map>> {
        vec![Scene::new(MAP, 0)]
    }

    async fn build(&self, state: &mut Map, scenes: &mut Scenes<'_>) -> Result<(), AvengerAppError> {
        if !scenes.requested(MAP) {
            return Ok(());
        }
        let projection = state.projection.clone().unwrap();
        let centers: Vec<[f32; 2]> = (state.places.iter())
            .map(|(_, place)| pixel(&projection, *place))
            .collect();
        let symbols = SceneSymbolMark {
            name: PLACES.to_string(),
            len: centers.len() as u32,
            x: ScalarOrArray::new_array(centers.iter().map(|c| c[0]).collect()),
            y: ScalarOrArray::new_array(centers.iter().map(|c| c[1]).collect()),
            shapes: vec![SymbolShape::Circle],
            size: 30.0.into(),
            ..Default::default()
        };
        let ids: ArrayRef = Arc::new(Int64Array::from_iter_values(
            state.places.iter().map(|(id, _)| *id),
        ));
        let mut data = SceneData::new();
        data.insert(
            PLACES,
            Arc::new(RecordBatch::try_from_iter([("id", ids)]).unwrap()),
        );
        let [width, height] = scenes.size();
        let scene = SceneGraph {
            marks: vec![symbols.into()],
            width,
            height,
            origin: [0.0; 2],
        };
        scenes.set(MAP, BuiltScene { scene, data })
    }
}

/// `brush` over the map, with one box.
fn boxes(brush: GeoBoxBrush<Map>) -> GeoBoxBrush<Map> {
    brush
        .area(|_| AREA)
        .projection(|s| s.projection.clone())
        .value(|s| s.selection)
        .on_change(|s, selection| {
            s.selection = selection;
            Rebuild::scene(MAP)
        })
        .on_settle(|s, selection| {
            s.settles.push(selection);
            Rebuild::tree(MAP)
        })
}

fn id(datum: &Datum) -> Option<i64> {
    let ids = datum.rows.column_by_name("id")?.as_primitive::<Int64Type>();
    Some(ids.value(datum.row))
}

fn lasso() -> LassoBrush<Map, i64> {
    LassoBrush::new("lasso")
        .area(|_: &Map| AREA)
        .projection(|s| s.projection.clone())
        .instances(Instances::marks([PLACES]).key(id))
        .value(|s| s.lasso.clone())
        .on_change(|s, lasso| {
            s.lasso = lasso;
            Rebuild::tree(MAP)
        })
}

async fn app(state: Map, brush: impl Into<Brush<Map>>) -> AvengerApp<Map> {
    let engine = avenger_typst_label::LabelEngine::new(Default::default());
    let brushes = Brushes::new(BRUSHES, 10).with(brush);
    let builder = AvengerApp::builder(state, Places, engine)
        .size([400.0, 200.0])
        .tool(brushes);
    builder.build().await.unwrap()
}

async fn send(app: &mut AvengerApp<Map>, events: impl IntoIterator<Item = WindowEvent>) {
    for event in events {
        std::thread::sleep(Duration::from_millis(2));
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
fn same(a: Option<GeoBox>, b: GeoBox) -> bool {
    let axis = |a: Option<[f64; 2]>, b: Option<[f64; 2]>| match (a, b) {
        (Some(a), Some(b)) => (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6,
        (a, b) => a.is_none() && b.is_none(),
    };
    a.is_some_and(|a| axis(a.lon, b.lon) && axis(a.lat, b.lat))
}

/// The bounds of the drawn box's fill, `[left, top, right, bottom]`, from the map's top-left
/// corner.
fn bounds(app: &AvengerApp<Map>) -> [f32; 4] {
    let groups = app.scenes()[1].marks.iter().filter_map(|mark| match mark {
        SceneMark::Group(group) => Some(group),
        _ => None,
    });
    let fill = groups
        .flat_map(|group| &group.marks)
        .find_map(|mark| match mark {
            SceneMark::Path(path) if path.name == "brush_box" => path.path.first().cloned(),
            _ => None,
        })
        .unwrap();
    let mut bounds = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for point in fill.iter().map(|event| event.to()) {
        bounds = [
            bounds[0].min(point.x),
            bounds[1].min(point.y),
            bounds[2].max(point.x),
            bounds[3].max(point.y),
        ];
    }
    bounds
}

/// On an equirectangular map, a drag reports the ranges between its corners, and latitude and
/// longitude brushes report one range.
#[tokio::test(flavor = "current_thread")]
async fn a_drag_reports_the_ranges_of_its_corners() {
    let cases = [
        (
            GeoBoxBrush::lon_lat("box"),
            GeoBox {
                lon: Some([-30.0, 40.0]),
                lat: Some([-10.0, 20.0]),
            },
        ),
        (
            GeoBoxBrush::lat("band"),
            GeoBox {
                lon: None,
                lat: Some([-10.0, 20.0]),
            },
        ),
        (
            GeoBoxBrush::lon("slice"),
            GeoBox {
                lon: Some([-30.0, 40.0]),
                lat: None,
            },
        ),
    ];
    for (brush, expected) in cases {
        let mut app = app(Map::new(flat()), boxes(brush)).await;
        let from = pixel(&flat(), [-30.0, 20.0]);
        let to = pixel(&flat(), [40.0, -10.0]);
        send(&mut app, drag(from, to)).await;
        let selection = app.state().selection;
        assert!(same(selection, expected), "{selection:?} for {expected:?}");
        assert_eq!(app.state().settles, [selection]);
    }
}

/// A drag east across the antimeridian selects the longitudes that it crossed, so the range's
/// first value exceeds its second.
#[tokio::test(flavor = "current_thread")]
async fn a_drag_across_the_antimeridian() {
    let centered = flat().with_rotate([180.0, 0.0, 0.0]);
    let mut app = app(
        Map::new(centered.clone()),
        boxes(GeoBoxBrush::lon_lat("box")),
    )
    .await;
    let from = pixel(&centered, [170.0, 10.0]);
    let to = pixel(&centered, [-170.0, -10.0]);
    send(&mut app, drag(from, to)).await;
    let selection = app.state().selection.unwrap();
    assert!(same(
        Some(selection),
        GeoBox {
            lon: Some([170.0, -170.0]),
            lat: Some([-10.0, 10.0]),
        }
    ));
    assert!(selection.contains([180.0, 0.0]));
    assert!(!selection.contains([0.0, 0.0]));
}

/// On a globe, a press off the sphere draws no box and shows no crosshair.
#[tokio::test(flavor = "current_thread")]
async fn a_press_off_the_globe_draws_nothing() {
    let mut app = app(Map::new(globe()), boxes(GeoBoxBrush::lon_lat("box"))).await;
    let off = [AREA.x + 10.0, AREA.y + 10.0];
    send(&mut app, [moved(off)]).await;
    assert_eq!(app.cursor(), CursorIcon::Default);
    send(&mut app, drag(off, pixel(&globe(), [0.0, 0.0]))).await;
    assert_eq!(app.state().selection, None);

    send(&mut app, [moved(pixel(&globe(), [0.0, 0.0]))]).await;
    assert_eq!(app.cursor(), CursorIcon::Crosshair);
}

/// A latitude band that reaches past the top of the map reports the north pole.
#[tokio::test(flavor = "current_thread")]
async fn a_band_to_the_pole() {
    let mut app = app(Map::new(flat()), boxes(GeoBoxBrush::lat("band"))).await;
    let from = pixel(&flat(), [0.0, 60.0]);
    send(&mut app, drag(from, [from[0], AREA.y - 20.0])).await;
    let band = GeoBox {
        lon: None,
        lat: Some([60.0, 90.0]),
    };
    assert!(same(app.state().selection, band));
}

/// A pan of the projection draws the box again over the same places.
#[tokio::test(flavor = "current_thread")]
async fn a_pan_keeps_the_box_on_its_geography() {
    let mut app = app(Map::new(flat()), boxes(GeoBoxBrush::lon_lat("box"))).await;
    let from = pixel(&flat(), [-30.0, 20.0]);
    let to = pixel(&flat(), [40.0, -10.0]);
    send(&mut app, drag(from, to)).await;
    let [left, top, right, bottom] = bounds(&app);
    assert!((left - 150.0).abs() < 0.01 && (right - 220.0).abs() < 0.01);
    assert!((top - 70.0).abs() < 0.01 && (bottom - 100.0).abs() < 0.01);

    app.state_mut().projection =
        Some(Projection::new(Equirectangular).with_view(180.0 / PI, [210.0, 90.0]));
    let resize = WindowEvent::WindowResize(WindowResizeEvent {
        size: [400.0, 200.0],
    });
    send(&mut app, [resize]).await;
    let [left, _, right, _] = bounds(&app);
    assert!((left - 180.0).abs() < 0.01 && (right - 250.0).abs() < 0.01);
}

/// A lasso on a globe selects the places inside its path, in either direction, and reports a
/// polygon that holds the places inside the path and not the rest of the sphere.
#[tokio::test(flavor = "current_thread")]
async fn a_lasso_selects_a_spherical_polygon() {
    let corners = [[185.0, 75.0], [230.0, 75.0], [230.0, 115.0], [185.0, 115.0]];
    let reversed: Vec<[f32; 2]> = corners.iter().rev().copied().collect();
    for path in [corners.to_vec(), reversed] {
        let mut state = Map::new(globe());
        state.places = vec![(1, [0.0, 0.0]), (2, [10.0, 10.0]), (3, [50.0, 0.0])];
        let mut app = app(state, lasso()).await;
        send(&mut app, trace(&path)).await;
        let lasso = app.state().lasso.clone();
        let mut keys = lasso.keys.clone();
        keys.sort();
        assert_eq!(keys, [1, 2]);
        assert!(lassoed(&lasso.polygons, [5.0, 5.0]));
        assert!(!lassoed(&lasso.polygons, [50.0, 0.0]));
        assert!(!lassoed(&lasso.polygons, [180.0, 0.0]));
    }
}

/// A drag past the edge of a flat map stops at the map's edge, where longitudes would wrap.
#[tokio::test(flavor = "current_thread")]
async fn a_drag_past_the_map_stops_at_its_edge() {
    let mut app = app(Map::new(flat()), boxes(GeoBoxBrush::lon_lat("box"))).await;
    let from = pixel(&flat(), [150.0, 10.0]);
    send(&mut app, drag(from, [AREA.right() + 60.0, from[1] + 20.0])).await;
    let [west, east] = app.state().selection.unwrap().lon.unwrap();
    assert!((west - 150.0).abs() < 1e-6);
    assert!(east > 170.0 && east <= 180.0, "east {east}");
}

/// A lasso mostly off a globe's limb still holds the places inside it, in either direction.
#[tokio::test(flavor = "current_thread")]
async fn a_lasso_off_the_limb() {
    let corners = [[250.0, 80.0], [320.0, 80.0], [320.0, 120.0], [250.0, 120.0]];
    let reversed: Vec<[f32; 2]> = corners.iter().rev().copied().collect();
    for path in [corners.to_vec(), reversed] {
        let mut app = app(Map::new(globe()), lasso()).await;
        send(&mut app, trace(&path)).await;
        let polygons = app.state().lasso.polygons.clone();
        // Longitude 50 lies 61 pixels right of the globe's center, inside the path.
        assert!(lassoed(&polygons, [50.0, 0.0]));
        assert!(!lassoed(&polygons, [-120.0, 0.0]));
    }
}

/// A drag across a flat map in one move selects the longitudes that it crossed, as a drag in
/// many moves does.
#[tokio::test(flavor = "current_thread")]
async fn a_fast_drag_selects_the_longitudes_that_it_crossed() {
    let mut app = app(Map::new(flat()), boxes(GeoBoxBrush::lon_lat("box"))).await;
    let from = pixel(&flat(), [-150.0, 20.0]);
    let to = pixel(&flat(), [150.0, -20.0]);
    let release = mouse(ElementState::Released);
    send(
        &mut app,
        [
            moved(from),
            mouse(ElementState::Pressed),
            moved(to),
            release,
        ],
    )
    .await;
    let crossed = GeoBox {
        lon: Some([-150.0, 150.0]),
        lat: Some([-20.0, 20.0]),
    };
    assert!(same(app.state().selection, crossed));
}

/// A lasso around a whole globe selects the places on it, though its path never crosses the
/// globe, and its polygon is the globe's visible half.
#[tokio::test(flavor = "current_thread")]
async fn a_lasso_around_the_globe() {
    let mut state = Map::new(globe());
    state.places = vec![(1, [0.0, 0.0]), (2, [30.0, 20.0])];
    let mut app = app(state, lasso()).await;
    let around = [[110.0, 15.0], [290.0, 15.0], [290.0, 185.0], [110.0, 185.0]];
    send(&mut app, trace(&around)).await;
    let lasso = &app.state().lasso;
    let mut keys = lasso.keys.clone();
    keys.sort();
    assert_eq!(keys, [1, 2]);
    assert!(lassoed(&lasso.polygons, [30.0, 20.0]));
    assert!(!lassoed(&lasso.polygons, [180.0, 0.0]));
}

/// A lasso keeps the map's view at its press, so a view change during the drag leaves its keys
/// inside its polygon.
#[tokio::test(flavor = "current_thread")]
async fn a_view_change_during_a_lasso() {
    let mut state = Map::new(flat());
    state.places = vec![(1, [-25.0, 0.0])];
    let mut app = app(state, lasso()).await;
    let press = mouse(ElementState::Pressed);
    send(
        &mut app,
        [moved([170.0, 70.0]), press, moved([230.0, 70.0])],
    )
    .await;
    let projection = app.state().projection.as_ref().unwrap();
    let zoomed = projection
        .zoom(PanMode::Translate, [210.0, 60.0], 1.44)
        .unwrap();
    app.state_mut().projection = Some(zoomed);
    let size = [400.0, 200.0];
    send(
        &mut app,
        [WindowEvent::WindowResize(WindowResizeEvent { size })],
    )
    .await;
    let rest = [[230.0, 130.0], [170.0, 130.0], [170.0, 70.0]].map(moved);
    send(
        &mut app,
        rest.into_iter().chain([mouse(ElementState::Released)]),
    )
    .await;
    let lasso = &app.state().lasso;
    assert_eq!(lasso.keys, [1]);
    assert!(lassoed(&lasso.polygons, [-25.0, 0.0]));
}
