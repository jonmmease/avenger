//! Tooltip boxes lie beside their anchor on the first side that fits, stay inside their bounds, and
//! point their callout's tip at the anchor.

use std::sync::Arc;

use arrow_array::{ArrayRef, Float64Array, RecordBatch, StringArray};
use avenger_color::ColorOrGradient::Color;
use avenger_geometry::data::SceneData;
use avenger_geometry::focus::{FocusStrategy, Side};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::marks::{
    group::{Clip, SceneGroup},
    mark::SceneMark,
    rect::SceneRectMark,
    symbol::SceneSymbolMark,
};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_tooltip::draw::{
    highlight, instance_color, place, place_beside_pointer, Swatch, Tooltip, TooltipContent,
    TooltipField, TooltipRow, TooltipStyle,
};
use avenger_typst_label::{bundled_label_engine, LabelEngine};
use lyon_path::PathEvent;

const SIDES: [Side; 4] = [Side::Right, Side::Left, Side::Above, Side::Below];
const BOUNDS: [f32; 4] = [0.0, 0.0, 200.0, 100.0];

fn engine() -> LabelEngine {
    bundled_label_engine().with_number_formatting(Arc::new(
        avenger_format_number_d3::D3NumberFormatProvider::new(),
    ))
}

/// The box's fill, its outline's color, and its texts' colors: the header's and the values', then
/// the labels'.
fn colors(tooltip: &Tooltip) -> ([f32; 4], [f32; 4], [f32; 4], [f32; 4]) {
    let group = tooltip.marks(|_| [30.0, 50.0], BOUNDS, &engine()).unwrap();
    let paint = |paint: Option<&avenger_color::ColorOrGradient>| match paint {
        Some(Color(color)) => *color,
        paint => panic!("{paint:?} is not a color"),
    };
    let SceneMark::Path(outline) = &group.marks[0] else {
        panic!("the first mark is the box's outline");
    };
    let texts: Vec<[f32; 4]> = group
        .marks
        .iter()
        .filter_map(|mark| match mark {
            SceneMark::Text(text) => text.color.first().copied(),
            _ => None,
        })
        .collect();
    let [header, labels, ..] = texts[..] else {
        panic!("a header, labels, and values");
    };
    (
        paint(outline.fill.first()),
        paint(outline.stroke.first()),
        header,
        labels,
    )
}

/// A tint fades the box's fill toward its color and outlines the box in it. Dark text stays on
/// a light tinted fill, and on a dark one, the text turns white, with labels between the text
/// and the fill.
#[test]
fn a_tint_fades_the_fill_and_text_contrasts_with_it() {
    let style = TooltipStyle::default();
    let tinted = |tint, amount| Tooltip {
        header: Some("Setosa".to_string()),
        rows: vec![TooltipRow::new("length", "5.1")],
        style: TooltipStyle {
            tint_amount: amount,
            ..TooltipStyle::default()
        },
        tint: Some(tint),
        ..Default::default()
    };
    let (fill, stroke, text, _) = colors(&tinted([0.9, 0.0, 0.0, 0.1], 0.25));
    assert!(
        fill[0] > 0.9 && fill[1] < 0.95 && fill[1] > 0.5,
        "{fill:?} is a pale red"
    );
    assert_eq!(fill[3], style.fill[3]);
    assert_eq!(stroke, [0.9, 0.0, 0.0, 1.0]);
    assert_eq!(text, style.color);

    let (fill, _, text, label) = colors(&tinted([0.05, 0.1, 0.4, 1.0], 1.0));
    assert!(fill[2] < 0.5, "{fill:?} is the navy tint");
    assert_eq!(text, [1.0, 1.0, 1.0, 1.0]);
    assert!(
        label[0] < 1.0 && label[0] > fill[0],
        "{label:?} lies between"
    );
}

/// The first side that fits wins, and a box beside an anchor near an edge flips to the
/// opposite side.
#[test]
fn the_first_side_that_fits_wins() {
    let at = |point: [f32; 2]| move |_: Side| point;
    let placed = place(at([50.0, 50.0]), [40.0, 20.0], BOUNDS, &SIDES, 6.0);
    assert_eq!(placed.side, Side::Right);
    assert_eq!(placed.origin, [56.0, 40.0]);

    let placed = place(at([180.0, 50.0]), [40.0, 20.0], BOUNDS, &SIDES, 6.0);
    assert_eq!(placed.side, Side::Left);
    assert_eq!(placed.origin, [134.0, 40.0]);

    let placed = place(at([100.0, 50.0]), [40.0, 20.0], BOUNDS, &[Side::Above], 6.0);
    assert_eq!((placed.side, placed.origin), (Side::Above, [80.0, 24.0]));
}

/// A box near the bounds' top moves down along its side to stay inside them, and its tip
/// stays on the anchor.
#[test]
fn the_box_stays_inside_the_bounds_and_the_tip_on_the_anchor() {
    let anchor = |side: Side| match side {
        Side::Right => [55.0, 4.0],
        _ => [45.0, 4.0],
    };
    let placed = place(anchor, [40.0, 30.0], BOUNDS, &SIDES, 6.0);
    assert_eq!(placed.side, Side::Right);
    assert_eq!(placed.origin, [61.0, 0.0]);
    assert_eq!(placed.tip, [55.0, 4.0]);
}

/// The points of a path.
fn points(path: &lyon_path::Path) -> Vec<[f32; 2]> {
    path.iter()
        .filter_map(|event| match event {
            PathEvent::Begin { at } => Some([at.x, at.y]),
            PathEvent::Line { to, .. } => Some([to.x, to.y]),
            _ => None,
        })
        .collect()
}

/// A tooltip's box outline reaches the anchor at its callout's tip, and lies beside it
/// otherwise.
#[test]
fn the_callout_points_at_the_anchor() {
    let tooltip = Tooltip {
        header: Some("Setosa".to_string()),
        rows: vec![TooltipRow::new("length", "5.1")],
        ..Default::default()
    };
    let group = tooltip.marks(|_| [30.0, 50.0], BOUNDS, &engine()).unwrap();
    let SceneMark::Path(outline) = &group.marks[0] else {
        panic!("the first mark is the box's outline");
    };
    let outline = points(outline.path.first().unwrap());
    let tip = [30.0 - group.origin[0], 50.0 - group.origin[1]];
    assert!(outline.contains(&tip), "{outline:?} has no tip at {tip:?}");
    assert_eq!(group.origin[0], 30.0 + tooltip.style.callout);
    let left = outline.iter().filter(|point| point[0] < 0.0).count();
    assert_eq!(left, 1, "only the tip lies left of the box");
}

/// A member's tooltip shows its row's columns, labeled by name and formatted by the text engine,
/// or the content's header and fields, with their labels and formats, and a field that names no
/// column fails. Its swatch is its symbol in its fill, and its color is its fill.
#[test]
fn a_members_tooltip_shows_its_row() {
    let points = SceneSymbolMark {
        name: "points".into(),
        len: 2,
        x: vec![20.0, 60.0].into(),
        y: 20.0.into(),
        fill: vec![Color([1.0, 0.0, 0.0, 1.0]), Color([0.0, 0.0, 1.0, 1.0])].into(),
        ..Default::default()
    };
    let scene = SceneGraph {
        marks: vec![points.into()],
        width: 100.0,
        height: 100.0,
        origin: [0.0; 2],
    };
    let length: ArrayRef = Arc::new(Float64Array::from(vec![5.1, 4.25]));
    let species: ArrayRef = Arc::new(StringArray::from(vec!["setosa", "virginica"]));
    let mut data = SceneData::new();
    data.insert(
        "points",
        Arc::new(RecordBatch::try_from_iter([("length", length), ("species", species)]).unwrap()),
    );
    let engine = engine();
    let tree = SceneGraphRTree::from_scene_graph_with_data(&scene, &data, &engine).unwrap();
    let focus = tree.focus([58.0, 22.0], &FocusStrategy::default()).unwrap();
    let member = &focus.members[0];

    let cells = |tooltip: &Tooltip| -> Vec<(String, String)> {
        let rows = tooltip.rows.iter();
        rows.map(|row| (row.label.clone(), row.value.clone()))
            .collect()
    };
    let tooltip = Tooltip::of_member(member, &TooltipContent::new(), &engine).unwrap();
    assert_eq!(tooltip.header, None);
    assert_eq!(
        cells(&tooltip),
        [
            ("length".to_string(), "4.25".to_string()),
            ("species".to_string(), "virginica".to_string())
        ]
    );
    let content = TooltipContent::new()
        .with_header("species")
        .with_fields([TooltipField::new("length")
            .with_label("Length")
            .with_format(".1f")]);
    let tooltip = Tooltip::of_member(member, &content, &engine).unwrap();
    assert_eq!(tooltip.header.as_deref(), Some("virginica"));
    assert_eq!(cells(&tooltip), [("Length".to_string(), "4.3".to_string())]);
    let missing = TooltipContent::new().with_fields(["width"]);
    assert!(Tooltip::of_member(member, &missing, &engine).is_err());

    let Some(Swatch::Symbol(swatch)) = Swatch::of(member, &scene) else {
        panic!("a symbol's swatch is a symbol");
    };
    assert_eq!(swatch.fill.first(), Some(&Color([0.0, 0.0, 1.0, 1.0])));
    assert_eq!(instance_color(member, &scene), Some([0.0, 0.0, 1.0, 1.0]));
}

/// A tooltip beside the pointer stands right of and below it by the style's offset, with no
/// callout: its outline stays inside its box.
#[test]
fn a_tooltip_beside_the_pointer_has_no_callout() {
    let tooltip = Tooltip {
        rows: vec![TooltipRow::new("length", "5.1")],
        ..Default::default()
    };
    let group = tooltip
        .marks_beside([30.0, 50.0], BOUNDS, &engine())
        .unwrap();
    assert_eq!(group.origin, [40.0, 60.0]);
    let SceneMark::Path(outline) = &group.marks[0] else {
        panic!("the first mark is the box's outline");
    };
    let outline = points(outline.path.first().unwrap());
    assert!(outline
        .iter()
        .all(|point| point[0] >= 0.0 && point[1] >= 0.0));
}

/// A box beside the pointer stands right of and below it, and on the pointer's other side along an
/// axis where it would leave the bounds, as Vega's tooltips stand.
#[test]
fn a_box_beside_the_pointer_flips_at_the_bounds() {
    let place = |pointer| place_beside_pointer(pointer, [40.0, 20.0], BOUNDS, [10.0, 10.0]);
    assert_eq!(place([50.0, 50.0]), [60.0, 60.0]);
    assert_eq!(place([190.0, 50.0]), [140.0, 60.0]);
    assert_eq!(place([50.0, 95.0]), [60.0, 65.0]);
    assert_eq!(place([190.0, 95.0]), [140.0, 65.0]);
}

/// A highlight outlines its member's drawn shape, a rect's corners, clipped as its mark is.
#[test]
fn a_highlight_outlines_its_member_within_its_clip() {
    let clip = Clip::Rect {
        x: 0.0,
        y: 0.0,
        width: 50.0,
        height: 50.0,
    };
    let rect = SceneRectMark {
        name: "region".into(),
        x: 10.0.into(),
        y: 10.0.into(),
        width: Some(20.0.into()),
        height: Some(30.0.into()),
        ..Default::default()
    };
    let scene = SceneGraph {
        marks: vec![SceneGroup {
            clip: clip.clone(),
            marks: vec![rect.into()],
            ..Default::default()
        }
        .into()],
        width: 100.0,
        height: 100.0,
        origin: [0.0; 2],
    };
    let values: ArrayRef = Arc::new(Float64Array::from(vec![1.0]));
    let mut data = SceneData::new();
    data.insert(
        "region",
        Arc::new(RecordBatch::try_from_iter([("value", values)]).unwrap()),
    );
    let tree = SceneGraphRTree::from_scene_graph_with_data(&scene, &data, &engine()).unwrap();
    let focus = tree.focus([20.0, 20.0], &FocusStrategy::Exact).unwrap();

    let group = highlight(&focus.members[0], [0.0, 0.0, 0.0, 1.0], 2.0);
    assert_eq!(group.clip, clip);
    let SceneMark::Path(outline) = &group.marks[0] else {
        panic!("a region's highlight is a path");
    };
    let corners = points(outline.path.first().unwrap());
    for corner in [[10.0, 10.0], [30.0, 10.0], [30.0, 40.0], [10.0, 40.0]] {
        assert!(
            corners.contains(&corner),
            "{corners:?} has no corner {corner:?}"
        );
    }
}

/// A group's tooltip has the header from its first member's row, and a row for each member,
/// labeled by its mark's name, with its swatch and its value.
#[test]
fn a_groups_tooltip_has_a_row_for_each_member() {
    use avenger_scenegraph::marks::line::{LineHit, SceneLineMark};
    let line = |name: &str, y: f32| -> SceneMark {
        SceneLineMark {
            name: name.into(),
            len: 3,
            x: vec![10.0, 50.0, 90.0].into(),
            y: y.into(),
            hit: LineHit::Vertices,
            ..Default::default()
        }
        .into()
    };
    let rows = |values: Vec<f64>| {
        let steps: ArrayRef = Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0]));
        let values: ArrayRef = Arc::new(Float64Array::from(values));
        Arc::new(RecordBatch::try_from_iter([("step", steps), ("value", values)]).unwrap())
    };
    let scene = SceneGraph {
        marks: vec![line("a", 20.0), line("b", 60.0)],
        width: 100.0,
        height: 100.0,
        origin: [0.0; 2],
    };
    let mut data = SceneData::new();
    data.insert("a", rows(vec![1.0, 2.0, 3.0]));
    data.insert("b", rows(vec![4.0, 5.0, 6.0]));
    let engine = engine();
    let tree = SceneGraphRTree::from_scene_graph_with_data(&scene, &data, &engine).unwrap();
    let strategy = FocusStrategy::GroupX {
        max_radius: 20.0,
        limit: None,
    };
    let focus = tree.focus([52.0, 40.0], &strategy).unwrap();
    let content = TooltipContent::new()
        .with_header(TooltipField::new("step").with_label("Step"))
        .with_fields([TooltipField::new("value").with_format(".1f")]);

    let tooltip = Tooltip::of_group(&focus, &content, &scene, &engine).unwrap();
    assert_eq!(tooltip.header.as_deref(), Some("Step 1"));
    let rows: Vec<(&str, &str, bool)> = tooltip
        .rows
        .iter()
        .map(|row| (row.label.as_str(), row.value.as_str(), row.swatch.is_some()))
        .collect();
    assert_eq!(rows, [("a", "2.0", true), ("b", "5.0", true)]);
}
