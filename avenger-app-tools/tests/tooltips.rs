//! Tooltips over a chart: the instance that a target finds at the pointer, its row as fields read
//! it, where its box stands, and its outline.

use std::sync::Arc;

use arrow_array::{ArrayRef, Date32Array, Float64Array, Int32Array, RecordBatch, StringArray};
use async_trait::async_trait;
use avenger_app::app::{AppBuild, AvengerApp, BuiltScene, Changes, Scene, Scenes};
use avenger_app::error::AvengerAppError;
use avenger_app_tools::tooltips::{InstanceTarget, TooltipAnchor, TooltipField, Tooltips};
use avenger_common::time::Instant;
use avenger_eventstream::manager::{EventStreamHandler, SceneTrees};
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{EventStreamConfig, Rebuild};
use avenger_eventstream::window::{
    ElementState, Key, WindowCursorMoved, WindowEvent, WindowKeyboardInput,
};
use avenger_format_datetime_d3::D3DateTimeFormatProvider;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_geometry::data::SceneData;
use avenger_geometry::query::Metric;
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::marks::group::{Clip, SceneGroup};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::rect::SceneRectMark;
use avenger_scenegraph::marks::symbol::SceneSymbolMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::LabelEngine;

const WIDTH: f32 = 200.0;
const HEIGHT: f32 = 100.0;
const CHART: &str = "chart";
const TOOLTIP: &str = "tooltip";

/// A chart of points along y = 50, the second drawn over the first, without rows for a mark of
/// decorations over the first, and a region in a clipped group, in a scene that starts at
/// `origin`. A key rebuilds its scene without its tree.
struct Chart {
    origin: [f32; 2],
}

/// The points' centers, whose discs have a radius of 5.
const POINTS: [[f32; 2]; 4] = [[30.0, 50.0], [33.0, 50.0], [60.0, 50.0], [190.0, 50.0]];

#[async_trait]
impl AppBuild<()> for Chart {
    /// A key rebuilds the chart without its tree.
    fn scenes(&self) -> Vec<Scene<()>> {
        let key = EventStreamConfig {
            types: vec![SceneGraphEventType::KeyPress],
            ..Default::default()
        };
        vec![Scene::new(CHART, 0).stream(key, Rerender)]
    }

    async fn build(&self, _: &mut (), scenes: &mut Scenes<'_>) -> Result<(), AvengerAppError> {
        if !scenes.requested(CHART) {
            return Ok(());
        }
        let points = SceneSymbolMark {
            name: "points".to_string(),
            len: POINTS.len() as u32,
            x: POINTS.map(|[x, _]| x).to_vec().into(),
            y: POINTS.map(|[_, y]| y).to_vec().into(),
            size: 100.0.into(),
            fill: avenger_color::ColorOrGradient::Color([0.2, 0.4, 0.8, 1.0]).into(),
            ..Default::default()
        };
        let decorations = SceneSymbolMark {
            name: "decorations".to_string(),
            x: 30.0.into(),
            y: 50.0.into(),
            size: 400.0.into(),
            ..Default::default()
        };
        let region = SceneRectMark {
            name: "regions".to_string(),
            x: 0.0.into(),
            y: 0.0.into(),
            width: Some(60.0.into()),
            height: Some(40.0.into()),
            ..Default::default()
        };
        // The region's group starts at (80, 60) and clips it to its left 50 pixels.
        let regions = SceneGroup {
            origin: [80.0, 60.0],
            clip: Clip::Rect {
                x: 0.0,
                y: 0.0,
                width: 50.0,
                height: 40.0,
            },
            marks: vec![region.into()],
            ..Default::default()
        };
        let scene = SceneGraph {
            marks: vec![points.into(), decorations.into(), regions.into()],
            width: WIDTH,
            height: HEIGHT,
            origin: self.origin,
        };
        let mut data = SceneData::new();
        data.insert("points", points_rows());
        let names: ArrayRef = Arc::new(StringArray::from(vec!["east"]));
        data.insert(
            "regions",
            Arc::new(RecordBatch::try_from_iter([("name", names)]).unwrap()),
        );
        scenes.set(CHART, BuiltScene { scene, data })
    }
}

/// The points' rows: an id, a value, which is null for the third point, and a date.
fn points_rows() -> Arc<RecordBatch> {
    let id: ArrayRef = Arc::new(Int32Array::from(vec![0, 1, 2, 3]));
    let value: ArrayRef = Arc::new(Float64Array::from(vec![
        Some(1234.5),
        Some(2.25),
        None,
        Some(7.0),
    ]));
    // Days since 1970-01-01: 2026-01-05 is 20458.
    let when: ArrayRef = Arc::new(Date32Array::from(vec![20458; 4]));
    Arc::new(RecordBatch::try_from_iter([("id", id), ("value", value), ("when", when)]).unwrap())
}

/// Asks for the chart's scene, but not its tree.
struct Rerender;

#[async_trait]
impl EventStreamHandler<()> for Rerender {
    async fn handle(&self, _: &SceneGraphEvent, _: &mut (), _: &SceneTrees<'_>) -> Rebuild {
        Rebuild::scene(CHART)
    }
}

fn engine() -> LabelEngine {
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
        .with_datetime_formatting(Arc::new(D3DateTimeFormatProvider::new()))
}

/// Tooltips over the chart that try `targets`.
fn tips(targets: impl IntoIterator<Item = InstanceTarget>) -> Tooltips {
    Tooltips::new(TOOLTIP, 10).over(targets)
}

fn app(tooltips: Tooltips) -> AvengerApp<()> {
    app_at([0.0; 2], tooltips)
}

/// An app of the chart at `origin`, under `tooltips`.
fn app_at(origin: [f32; 2], tooltips: Tooltips) -> AvengerApp<()> {
    let builder = AvengerApp::builder((), Chart { origin }, engine())
        .size([WIDTH, HEIGHT])
        .tool(tooltips);
    pollster::block_on(builder.build()).unwrap()
}

fn hover(app: &mut AvengerApp<()>, position: [f32; 2]) -> Changes {
    let moved = WindowEvent::CursorMoved(WindowCursorMoved { position });
    pollster::block_on(app.update(&moved, Instant::now()))
}

/// The marks named `name` in the tooltip's scene, depth first.
fn named<'a>(app: &'a AvengerApp<()>, name: &str) -> Vec<&'a SceneMark> {
    fn walk<'a>(marks: &'a [SceneMark], name: &str, found: &mut Vec<&'a SceneMark>) {
        for mark in marks {
            if mark.mark_name() == name {
                found.push(mark);
            }
            if let SceneMark::Group(group) = mark {
                walk(&group.marks, name, found);
            }
        }
    }
    let mut found = Vec::new();
    walk(&app.scenes()[1].marks, name, &mut found);
    found
}

/// The lines of the tooltip's text mark named `name`.
fn lines(app: &AvengerApp<()>, name: &str) -> Vec<String> {
    match named(app, name).first() {
        Some(SceneMark::Text(text)) => text.text.as_vec(text.len as usize, None),
        _ => Vec::new(),
    }
}

/// The tooltip's box, `[x, y, width, height]`, with its callout where the callout reaches past the
/// box's right or bottom edge, when it shows.
fn tooltip_box(app: &AvengerApp<()>) -> Option<[f32; 4]> {
    let Some(SceneMark::Group(group)) = named(app, "tooltip").first().copied() else {
        return None;
    };
    let SceneMark::Path(outline) = &group.marks[0] else {
        return None;
    };
    let path = outline.path.first()?;
    let points = path.iter().flat_map(|event| [event.from(), event.to()]);
    let (width, height) = points.fold((0.0f32, 0.0f32), |(w, h), p| (w.max(p.x), h.max(p.y)));
    Some([group.origin[0], group.origin[1], width, height])
}

/// The topmost instance under the pointer among those with rows takes the tooltip, which shows
/// every column of its row without fields.
#[test]
fn an_exact_target_takes_the_topmost_instance_with_rows() {
    let mut app = app(tips([InstanceTarget::exact()]));
    hover(&mut app, [31.0, 50.0]);
    assert_eq!(lines(&app, "tooltip_titles"), ["id", "value", "when"]);
    assert_eq!(lines(&app, "tooltip_values")[0], "1");
    hover(&mut app, [5.0, 5.0]);
    assert!(named(&app, "tooltip").is_empty());
}

/// A nearest target takes the instance whose drawn edge lies nearest the pointer, within its
/// radius, which a metric can measure along x alone.
#[test]
fn a_nearest_target_looks_within_its_radius() {
    let mut app = app(tips([InstanceTarget::nearest(10.0)]));
    hover(&mut app, [60.0, 62.0]);
    assert_eq!(lines(&app, "tooltip_values")[0], "2");
    hover(&mut app, [60.0, 70.0]);
    assert!(named(&app, "tooltip").is_empty());

    let along_x = InstanceTarget::nearest(10.0).with_metric(Metric::AlongX);
    let mut app = crate::app(tips([along_x]));
    hover(&mut app, [60.0, 95.0]);
    assert_eq!(lines(&app, "tooltip_values")[0], "2");
}

/// Targets are tried in order, each among its marks, and the first that finds an instance shows
/// its tooltip.
#[test]
fn the_first_target_that_finds_an_instance_shows_it() {
    let targets = [
        InstanceTarget::nearest(30.0).with_marks(["points"]),
        InstanceTarget::exact().with_marks(["regions"]),
    ];
    let mut app = app(tips(targets));
    // Both targets find an instance here, so the first shows it.
    hover(&mut app, [85.0, 62.0]);
    assert_eq!(lines(&app, "tooltip_values")[0], "2");
    hover(&mut app, [125.0, 95.0]);
    assert_eq!(lines(&app, "tooltip_values"), ["east"]);
}

/// Fields format their values as an axis formats its ticks, with their labels' markup, and a null
/// value is empty. The header shows beside a swatch of the instance's color, unless the target is
/// without one.
#[test]
fn fields_read_as_axis_ticks_do() {
    let target = InstanceTarget::exact()
        .with_header(TooltipField::new("id").with_value_label("Point #value"))
        .with_fields([
            TooltipField::new("value")
                .with_title("Value")
                .with_value_format(",.1f"),
            TooltipField::new("when").with_value_format("%b %d"),
        ]);
    let mut app = app(tips([target.clone()]));
    hover(&mut app, [26.0, 50.0]);
    assert_eq!(lines(&app, "tooltip_titles"), ["Value", "when"]);
    let values = lines(&app, "tooltip_values");
    assert_eq!(values[0], "1,234.5");
    assert_eq!(values[1], "Jan 05");
    assert!(lines(&app, "tooltip_header")[0].contains("Point"));
    assert_eq!(named(&app, "tooltip_swatch").len(), 1);

    hover(&mut app, [60.0, 50.0]);
    assert_eq!(lines(&app, "tooltip_values")[0], "");

    let mut plain = crate::app(tips([target.without_swatch()]));
    hover(&mut plain, [26.0, 50.0]);
    assert!(named(&plain, "tooltip_swatch").is_empty());
}

/// A data-anchored box stands right of its instance, and left of it where the right side would
/// leave the window. A pointer-anchored box stands below and right of the pointer, and flips near
/// the window's edges.
#[test]
fn a_box_changes_sides_to_stay_inside_the_window() {
    let mut app = app(tips([InstanceTarget::exact().with_fields(["id"])]));
    hover(&mut app, [60.0, 50.0]);
    let [x, _, width, _] = tooltip_box(&app).unwrap();
    assert!(x > 65.0 && x + width <= WIDTH);
    // The box and its callout end at the point's left edge.
    hover(&mut app, [190.0, 50.0]);
    let [x, _, width, _] = tooltip_box(&app).unwrap();
    assert!(x + width <= 185.001);

    let pointer = InstanceTarget::exact().with_anchor(TooltipAnchor::Pointer);
    let mut app = app_with_pointer(pointer);
    hover(&mut app, [85.0, 62.0]);
    let [x, y, _, _] = tooltip_box(&app).unwrap();
    assert!(x > 85.0 && y > 62.0);
    hover(&mut app, [125.0, 95.0]);
    let [x, y, width, height] = tooltip_box(&app).unwrap();
    assert!(x + width < 125.0 && y + height < 95.0);
}

fn app_with_pointer(target: InstanceTarget) -> AvengerApp<()> {
    app(tips([target.with_marks(["regions"])]))
}

/// The outline circles the instance's fill, in a group with its mark's clip in scene coordinates.
#[test]
fn the_outline_follows_its_instance_inside_its_clip() {
    let mut app = app(tips([InstanceTarget::exact().with_marks(["regions"])]));
    hover(&mut app, [100.0, 80.0]);
    let Some(SceneMark::Group(outline)) = named(&app, "tooltip_highlight").first().copied() else {
        panic!("the region has an outline");
    };
    assert_eq!(
        outline.clip,
        Clip::Rect {
            x: 80.0,
            y: 60.0,
            width: 50.0,
            height: 40.0
        }
    );
}

/// The tooltip's scene changes alone when the pointer moves, and not for a move to where the
/// pointer already is. It shows nothing while the chart's tree is stale, and takes no hits.
#[test]
fn the_tooltip_follows_the_pointer_over_current_trees() {
    let mut app = app(tips([InstanceTarget::nearest(10.0)]));
    assert_eq!(hover(&mut app, [60.0, 50.0]).scenes, [1]);
    assert!(hover(&mut app, [60.0, 50.0]).scenes.is_empty());
    let tree = SceneGraphRTree::from_scene_graph(&app.scenes()[1], app.text_engine());
    assert_eq!(tree.iter().count(), 0);

    let key = WindowEvent::KeyboardInput(WindowKeyboardInput {
        key: Key::Character('a'),
        state: ElementState::Pressed,
    });
    let changes = pollster::block_on(app.update(&key, Instant::now()));
    assert_eq!(changes.scenes, [0, 1]);
    assert!(named(&app, "tooltip").is_empty());
}

/// A chart whose scene starts at an origin draws its instances moved by it, as its tree and the
/// pointer place them, and the tooltip stands by them in that frame, with no origin of its own: its
/// outline clipped where the chart's clip lies, and its box beside the instance or the pointer.
#[test]
fn a_scene_origin_moves_the_tooltip_once() {
    let regions = InstanceTarget::exact()
        .with_marks(["regions"])
        .with_anchor(TooltipAnchor::Pointer);
    let mut app = app_at([50.0, 30.0], tips([regions]));
    // The region's visible part starts at (130, 90), and the box flips left of and above the
    // pointer to stay inside the 200 by 100 scene.
    hover(&mut app, [135.0, 92.0]);
    assert_eq!(app.scenes()[1].origin, [0.0, 0.0]);
    let Some(SceneMark::Group(outline)) = named(&app, "tooltip_highlight").first().copied() else {
        panic!("the region has an outline");
    };
    assert_eq!(
        outline.clip,
        Clip::Rect {
            x: 130.0,
            y: 90.0,
            width: 50.0,
            height: 40.0
        }
    );
    let [x, y, width, height] = tooltip_box(&app).unwrap();
    assert!((x + width - 125.0).abs() < 0.01 && (y + height - 82.0).abs() < 0.01);

    // The third point's disc lies right of x = 115, about (110, 80).
    let points = InstanceTarget::exact()
        .with_marks(["points"])
        .with_fields(["id"]);
    let mut app = app_at([50.0, 30.0], tips([points]));
    hover(&mut app, [110.0, 80.0]);
    assert_eq!(lines(&app, "tooltip_values"), ["2"]);
    let [x, _, _, _] = tooltip_box(&app).unwrap();
    assert!(x > 115.0 && x < 130.0);
}
