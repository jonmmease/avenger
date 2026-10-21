//! Tooltip boxes lie beside their anchor on the first side that fits, stay inside their bounds, and
//! point their callout's tip at the anchor.

use std::sync::Arc;

use arrow_array::{ArrayRef, Float64Array, RecordBatch, StringArray};
use avenger_color::ColorOrGradient::Color;
use avenger_common::types::{TextAlign, TextSyntaxMode};
use avenger_geometry::data::SceneData;
use avenger_geometry::focus::{FocusQuery, FocusStrategy, Side};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::marks::{
    group::{Clip, SceneGroup},
    mark::SceneMark,
    rect::SceneRectMark,
    symbol::SceneSymbolMark,
};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_tooltip::draw::{
    highlight, instance_color, place, place_beside_pointer, Swatch, Tooltip, TooltipField,
    TooltipRow, TooltipStyle,
};
use avenger_tooltip::{GroupTarget, InstanceTarget};
use avenger_typst_label::{bind, bundled_label_engine, LabelEngine, LabelValue, LabelValues};
use lyon_path::PathEvent;

const SIDES: [Side; 4] = [Side::Right, Side::Left, Side::Above, Side::Below];
const BOUNDS: [f32; 4] = [0.0, 0.0, 200.0, 100.0];

fn engine() -> LabelEngine {
    bundled_label_engine().with_number_formatting(Arc::new(
        avenger_format_number_d3::D3NumberFormatProvider::new(),
    ))
}

/// The box's fill, its outline's color, and its texts' colors: the header's and the values', then
/// the titles'.
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
    let [header, titles, ..] = texts[..] else {
        panic!("a header, titles, and values");
    };
    (
        paint(outline.fill.first()),
        paint(outline.stroke.first()),
        header,
        titles,
    )
}

/// A tint fades the box's fill toward its color and outlines the box in it. Dark text stays on
/// a light tinted fill, and on a dark one, the text turns white, with titles between the text
/// and the fill. A style without a tint keeps its colors.
#[test]
fn a_tint_fades_the_fill_and_text_contrasts_with_it() {
    let style = TooltipStyle::default();
    let tinted = |tint, amount| Tooltip {
        header: Some("Setosa".to_string()),
        rows: vec![TooltipRow::new("length", "5.1")],
        style: TooltipStyle {
            tint: amount,
            ..TooltipStyle::default()
        },
        tint: Some(tint),
        ..Default::default()
    };
    let (fill, stroke, text, _) = colors(&tinted([0.9, 0.0, 0.0, 0.1], Some(0.25)));
    assert!(
        fill[0] > 0.9 && fill[1] < 0.95 && fill[1] > 0.5,
        "{fill:?} is a pale red"
    );
    assert_eq!(fill[3], style.fill[3]);
    assert_eq!(stroke, [0.9, 0.0, 0.0, 1.0]);
    assert_eq!(text, style.color);

    let (fill, _, text, title) = colors(&tinted([0.05, 0.1, 0.4, 1.0], Some(1.0)));
    assert!(fill[2] < 0.5, "{fill:?} is the navy tint");
    assert_eq!(text, [1.0, 1.0, 1.0, 1.0]);
    assert!(
        title[0] < 1.0 && title[0] > fill[0],
        "{title:?} lies between"
    );

    let (fill, stroke, text, _) = colors(&tinted([0.05, 0.1, 0.4, 1.0], None));
    assert_eq!(
        (fill, stroke, text),
        (style.fill, style.stroke, style.color)
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

/// A member's tooltip shows its row's columns, titled by name and formatted by the text engine,
/// or the target's header and fields, with their titles, formats, and labels, and a field that
/// names no column fails. Its swatch is its symbol in its fill, and its color is its fill.
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
    let focus = tree
        .focus(FocusQuery::at([58.0, 22.0], FocusStrategy::default()))
        .unwrap();
    let member = &focus.members[0];

    let cells = |tooltip: &Tooltip| -> Vec<(String, String)> {
        let rows = tooltip.rows.iter();
        rows.map(|row| (row.title.clone(), row.value.clone()))
            .collect()
    };
    let tooltip = Tooltip::of_member(member, &InstanceTarget::default(), &engine).unwrap();
    assert_eq!(tooltip.header, None);
    assert_eq!(
        cells(&tooltip),
        [
            ("length".to_string(), "4.25".to_string()),
            ("species".to_string(), "virginica".to_string())
        ]
    );
    let target = InstanceTarget::default()
        .with_header("species")
        .with_fields([TooltipField::new("length")
            .with_title("Length")
            .with_value_format(".1f")]);
    let tooltip = Tooltip::of_member(member, &target, &engine).unwrap();
    assert_eq!(tooltip.header.as_deref(), Some("virginica"));
    assert_eq!(cells(&tooltip), [("Length".to_string(), "4.3".to_string())]);
    let labeled = InstanceTarget::default().with_fields([TooltipField::new("length")
        .with_value_format(".1f")
        .with_value_label("*#value* cm")]);
    let tooltip = Tooltip::of_member(member, &labeled, &engine).unwrap();
    let value = LabelValues::from([("value".into(), LabelValue::Str("4.3".into()))]);
    let label = bind("*#value* cm", &value).unwrap();
    assert_eq!(tooltip.rows[0].value, label);
    assert_eq!(tooltip.rows[0].value_syntax, TextSyntaxMode::TypstMarkup);
    let missing = InstanceTarget::default().with_fields(["width"]);
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
    let focus = tree
        .focus(FocusQuery::at([20.0, 20.0], FocusStrategy::Exact))
        .unwrap();

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
/// titled by its entry's title, with its swatch and its value: the target's value field, or the
/// first column but the header's. A member that isn't one of the entries is titled by its mark's
/// name, and a group target needs a title for each mark.
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
    let focus = tree.focus(FocusQuery::at([52.0, 40.0], strategy)).unwrap();
    let target = GroupTarget::new(["a", "b"], ["Alpha", "Beta"])
        .unwrap()
        .with_header(TooltipField::new("step").with_title("Step"));

    let cells = |tooltip: &Tooltip| -> Vec<(String, String, bool)> {
        let rows = tooltip.rows.iter();
        rows.map(|row| (row.title.clone(), row.value.clone(), row.swatch.is_some()))
            .collect()
    };

    let formatted = target
        .clone()
        .with_value(TooltipField::new("value").with_value_format(".1f"));
    let tooltip = Tooltip::of_group(&focus, &formatted, &scene, &engine).unwrap();
    assert_eq!(tooltip.header.as_deref(), Some("Step 1"));
    let expected = [("Alpha", "2.0", true), ("Beta", "5.0", true)];
    assert_eq!(
        cells(&tooltip),
        expected.map(|(t, v, s)| (t.into(), v.into(), s))
    );

    let tooltip = Tooltip::of_group(&focus, &target, &scene, &engine).unwrap();
    let expected = [("Alpha", "2", true), ("Beta", "5", true)];
    assert_eq!(
        cells(&tooltip),
        expected.map(|(t, v, s)| (t.into(), v.into(), s))
    );

    let only_a = GroupTarget::new(["a"], ["Alpha"]).unwrap();
    let tooltip = Tooltip::of_group(&focus, &only_a, &scene, &engine).unwrap();
    assert_eq!(tooltip.rows[1].title, "b");
    assert!(GroupTarget::new(["a", "b"], ["Alpha"]).is_err());
}

/// Values align right in their column by default, so numbers line up at their last digits, or
/// left or centered as the style says.
#[test]
fn values_align_as_the_style_says() {
    let values = |align: Option<TextAlign>| {
        let mut style = TooltipStyle::default();
        if let Some(align) = align {
            style.value_align = align;
        }
        let tooltip = Tooltip {
            rows: vec![TooltipRow::new("a", "1"), TooltipRow::new("b", "-100.25")],
            style,
            ..Default::default()
        };
        let group = tooltip.marks_beside([0.0, 0.0], BOUNDS, &engine()).unwrap();
        group
            .marks
            .iter()
            .find_map(|mark| match mark {
                SceneMark::Text(text) if text.text_iter().any(|text| text == "1") => {
                    Some((*text.x.first().unwrap(), *text.align.first().unwrap()))
                }
                _ => None,
            })
            .expect("the values' text")
    };
    let (right, align) = values(None);
    assert_eq!(align, TextAlign::Right);
    let (left, _) = values(Some(TextAlign::Left));
    let (center, _) = values(Some(TextAlign::Center));
    assert!(
        left < center && center < right,
        "{left} < {center} < {right}"
    );
}
