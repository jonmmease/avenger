//! What every axis draws, each in its part's frame: the domain line and ticks, the labels, the
//! title, and the grid.
//!
//! A part's frame puts 0 along the axis at the start of its length, and 0 across it at the
//! part's inner edge, nearest the plot, with the part extending outward.

use std::sync::Arc;

use avenger_color::ColorOrGradient;
use avenger_common::{
    types::{TextAlign, TextBaseline, TextSyntaxMode},
    value::ScalarOrArray,
};
use avenger_scenegraph::marks::{
    group::SceneGroup,
    rule::SceneRuleMark,
    text::{text_origin, SceneTextMark},
};
use avenger_typst_label::{LabelEngine, TextBounds};

use super::opts::{AxisStyle, LabelThinning};
use crate::guide::{GuideSide, Reach};

/// The gap between the end of a tick and its label.
pub(crate) const TEXT_MARGIN: f32 = 3.0;
/// How far the domain line reaches past each end of the axis, so that it covers the outer ticks.
pub(crate) const PIXEL_OFFSET: f32 = 0.5;
/// The space that tick labels keep from each other, in pixels.
pub(crate) const LABEL_GAP: f32 = 1.0;

/// Label text, how it reads, and the box of each label, measured once.
#[derive(Debug)]
pub(crate) struct Labels {
    pub(crate) text: Vec<String>,
    pub(crate) syntax: TextSyntaxMode,
    /// Each label's box, or `None` for a label that does not lay out and draws nothing.
    pub(crate) bounds: Vec<Option<TextBounds>>,
}

impl Labels {
    /// Labels in the style's label font, with the box that each one draws with.
    pub(crate) fn measure(
        text: Vec<String>,
        syntax: TextSyntaxMode,
        style: &AxisStyle,
        engine: &LabelEngine,
    ) -> Self {
        let mark = SceneTextMark {
            len: text.len() as u32,
            text: ScalarOrArray::new_array(text.clone()),
            text_syntax: syntax,
            ..label_font(style)
        };
        let bounds = mark
            .labels()
            .map(|label| engine.bounds(&label.label).ok())
            .collect();
        Self {
            text,
            syntax,
            bounds,
        }
    }

    pub(crate) fn empty() -> Self {
        Self {
            text: Vec::new(),
            syntax: TextSyntaxMode::Plain,
            bounds: Vec::new(),
        }
    }
}

/// One row of labels at a length: their positions along the axis, their distance across the
/// labels part, and their angle in degrees clockwise.
pub(crate) struct LabelRow {
    pub(crate) labels: Arc<Labels>,
    pub(crate) positions: Vec<f32>,
    pub(crate) distance: f32,
    pub(crate) angle: f32,
}

/// Separators between an outer level's bands: positions along the axis, from `inner` to
/// `outer` across the labels part.
pub(crate) struct Separators {
    pub(crate) positions: Vec<f32>,
    pub(crate) inner: f32,
    pub(crate) outer: f32,
}

/// The title's text, how it reads, and its box.
#[derive(Debug)]
pub(crate) struct Title {
    pub(crate) text: String,
    pub(crate) bounds: TextBounds,
}

/// How a label aligns to its position, as Vega-Lite chooses for an axis side and a label angle in
/// degrees clockwise: the label's end nearest its tick lies on the position, so that the label
/// extends away from the axis. Labels that run along the axis center on their ticks.
pub(crate) fn label_alignment(side: GuideSide, angle: f32) -> (TextAlign, TextBaseline) {
    let angle = angle.rem_euclid(360.0);
    match side {
        GuideSide::Top | GuideSide::Bottom => {
            let bottom = side == GuideSide::Bottom;
            let align = if angle % 180.0 == 0.0 {
                TextAlign::Center
            } else if (0.0 < angle && angle < 180.0) == bottom {
                TextAlign::Left
            } else {
                TextAlign::Right
            };
            let baseline = if (45.0 < angle && angle < 135.0) || (225.0 < angle && angle < 315.0) {
                TextBaseline::Middle
            } else if (angle <= 45.0 || 315.0 <= angle) == bottom {
                TextBaseline::Top
            } else {
                TextBaseline::Bottom
            };
            (align, baseline)
        }
        GuideSide::Left | GuideSide::Right => {
            let left = side == GuideSide::Left;
            let align = if (angle + 90.0) % 180.0 == 0.0 {
                TextAlign::Center
            } else if (90.0 < angle && angle < 270.0) == left {
                TextAlign::Left
            } else {
                TextAlign::Right
            };
            let baseline = if angle <= 45.0 || 315.0 <= angle || (135.0..=225.0).contains(&angle) {
                TextBaseline::Middle
            } else if (45.0 < angle && angle < 135.0) == left {
                TextBaseline::Top
            } else {
                TextBaseline::Bottom
            };
            (align, baseline)
        }
    }
}

/// A row's alignment, unless the style sets it, and how far its labels shift up the axis.
fn placement(side: GuideSide, angle: f32, style: &AxisStyle) -> (TextAlign, TextBaseline, f32) {
    let (align, baseline) = label_alignment(side, angle);
    let align = style.label_align.unwrap_or(align);
    let baseline = style.label_baseline.unwrap_or(baseline);
    // Numbers don't reach the full descent, so unrotated labels beside a vertical axis sit a
    // tenth of their size higher to look centered on their ticks.
    let shift =
        if side.is_vertical() && angle.rem_euclid(360.0) == 0.0 && baseline == TextBaseline::Middle
        {
            style.label_font_size * 0.1
        } else {
            0.0
        };
    (align, baseline, shift)
}

/// A text mark in the style's label font, with nothing else set.
pub(crate) fn label_font(style: &AxisStyle) -> SceneTextMark {
    SceneTextMark {
        clip: false,
        color: style.label_color.into(),
        font: style.label_font_family.clone().into(),
        font_size: style.label_font_size.into(),
        font_weight: style.label_font_weight.into(),
        ..Default::default()
    }
}

/// The four corners of a box laid out from `anchor`, turned `angle` degrees clockwise about it,
/// as avenger-geometry turns a text mark's box. The corners are laid out around the anchor
/// before it is added, so that a box's size does not change with its position.
fn corners(
    bounds: &TextBounds,
    anchor: [f32; 2],
    align: TextAlign,
    baseline: TextBaseline,
    angle: f32,
) -> [[f32; 2]; 4] {
    let [left, top] = text_origin(bounds, [0.0, 0.0], align, baseline);
    let (right, bottom) = (left + bounds.width, top + bounds.height);
    let offsets = [[left, top], [right, top], [right, bottom], [left, bottom]];
    // Quarter turns are exact.
    let (sin, cos) = match angle.rem_euclid(360.0) {
        0.0 => (0.0, 1.0),
        90.0 => (1.0, 0.0),
        180.0 => (0.0, -1.0),
        270.0 => (-1.0, 0.0),
        angle => angle.to_radians().sin_cos(),
    };
    offsets.map(|[dx, dy]| {
        [
            anchor[0] + dx * cos - dy * sin,
            anchor[1] + dx * sin + dy * cos,
        ]
    })
}

/// The domain line along the whole length, at the part's inner edge.
pub(crate) fn domain_mark(length: f32, side: GuideSide, style: &AxisStyle) -> SceneRuleMark {
    let [x, y] = side.point(-PIXEL_OFFSET, 0.0);
    let [x2, y2] = side.point(length + PIXEL_OFFSET, 0.0);
    SceneRuleMark {
        x: x.into(),
        x2: x2.into(),
        y: y.into(),
        y2: y2.into(),
        stroke: ColorOrGradient::Color(style.domain_color).into(),
        stroke_width: 1.0.into(),
        ..Default::default()
    }
}

/// Ticks at `positions`, from the inner edge outward by the tick length.
pub(crate) fn ticks_mark(positions: &[f32], side: GuideSide, style: &AxisStyle) -> SceneRuleMark {
    rules(positions, 0.0, style.tick_length, side, style.tick_color)
}

/// Separators between an outer level's bands, in the tick color.
pub(crate) fn separators_mark(
    separators: &Separators,
    side: GuideSide,
    style: &AxisStyle,
) -> SceneRuleMark {
    rules(
        &separators.positions,
        separators.inner,
        separators.outer,
        side,
        style.tick_color,
    )
}

/// Rules across the part at `positions`, from `inner` to `outer`.
fn rules(
    positions: &[f32],
    inner: f32,
    outer: f32,
    side: GuideSide,
    color: [f32; 4],
) -> SceneRuleMark {
    let along = ScalarOrArray::new_array(positions.to_vec());
    let start = side.outward() * inner;
    let end = side.outward() * outer;
    let (x, x2, y, y2) = if side.is_vertical() {
        (start.into(), end.into(), along.clone(), along)
    } else {
        (along.clone(), along, start.into(), end.into())
    };
    SceneRuleMark {
        len: positions.len() as u32,
        clip: false,
        x,
        x2,
        y,
        y2,
        stroke: ColorOrGradient::Color(color).into(),
        stroke_width: 1.0.into(),
        ..Default::default()
    }
}

/// A row of labels at their positions and distance, aligned to the end nearest their position
/// unless the style sets the alignment.
pub(crate) fn label_mark(row: &LabelRow, side: GuideSide, style: &AxisStyle) -> SceneTextMark {
    let (align, baseline, shift) = placement(side, row.angle, style);
    let anchors: Vec<[f32; 2]> = row
        .positions
        .iter()
        .map(|position| side.point(position - shift, row.distance))
        .collect();
    SceneTextMark {
        len: row.labels.text.len() as u32,
        text: ScalarOrArray::new_array(row.labels.text.clone()),
        x: ScalarOrArray::new_array(anchors.iter().map(|anchor| anchor[0]).collect()),
        y: ScalarOrArray::new_array(anchors.iter().map(|anchor| anchor[1]).collect()),
        align: align.into(),
        baseline: baseline.into(),
        angle: row.angle.into(),
        text_syntax: row.labels.syntax,
        ..label_font(style)
    }
}

/// The box of each label of `row` as it draws, or `None` for a label that draws nothing.
fn row_boxes(row: &LabelRow, side: GuideSide, style: &AxisStyle) -> Vec<Option<[[f32; 2]; 4]>> {
    let (align, baseline, shift) = placement(side, row.angle, style);
    row.positions
        .iter()
        .zip(&row.labels.bounds)
        .map(|(position, bounds)| {
            let anchor = side.point(position - shift, row.distance);
            bounds
                .as_ref()
                .map(|bounds| corners(bounds, anchor, align, baseline, row.angle))
        })
        .collect()
}

/// Whether a label of `row` comes closer than [`LABEL_GAP`] to the next label that draws.
pub(crate) fn row_overlaps(row: &LabelRow, side: GuideSide, style: &AxisStyle) -> bool {
    let boxes: Vec<_> = row_boxes(row, side, style).into_iter().flatten().collect();
    boxes
        .windows(2)
        .any(|pair| boxes_overlap(&pair[0], &pair[1], LABEL_GAP))
}

/// Drops the labels of `row` that overlap, as the style's label thinning chooses.
pub(crate) fn thin_row(row: &mut LabelRow, side: GuideSide, style: &AxisStyle) {
    if style.label_thinning == LabelThinning::None {
        return;
    }
    let boxes = row_boxes(row, side, style);
    let shown = stride_labels(&boxes, LABEL_GAP, false);
    if shown.len() < boxes.len() {
        let labels = &row.labels;
        row.labels = Arc::new(Labels {
            text: shown
                .iter()
                .map(|index| labels.text[*index].clone())
                .collect(),
            syntax: labels.syntax,
            bounds: shown.iter().map(|index| labels.bounds[*index]).collect(),
        });
        row.positions = shown.iter().map(|index| row.positions[*index]).collect();
    }
}

/// The indices of the labels with `boxes` that show, as [`stride_shown`] picks them, where
/// labels keep `gap` apart. Each box is a label's four corners in order around it, or `None` for
/// a label that draws nothing.
pub(crate) fn stride_labels(boxes: &[Option<[[f32; 2]; 4]>], gap: f32, closed: bool) -> Vec<usize> {
    stride_shown(
        boxes.len(),
        |index| boxes[index].is_some(),
        |a, b| match (&boxes[a], &boxes[b]) {
            (Some(a), Some(b)) => !boxes_overlap(a, b, gap),
            _ => true,
        },
        closed,
    )
}

/// The indices of the `count` labels that show: every label that `draws`, or else every second,
/// every third, and so on from the first, at the smallest stride whose labels are `apart` from
/// their neighbors. Around a closed loop, such as a whole circle, the last label that shows
/// neighbors the first, and the stride divides the number of labels, so that the pattern stays
/// even all the way around.
pub(crate) fn stride_shown(
    count: usize,
    draws: impl Fn(usize) -> bool,
    apart: impl Fn(usize, usize) -> bool,
    closed: bool,
) -> Vec<usize> {
    let shown_at = |stride: usize| -> Vec<usize> {
        (0..count)
            .step_by(stride)
            .filter(|index| draws(*index))
            .collect()
    };
    let clear = |shown: &[usize]| {
        let neighbors = shown.windows(2).all(|pair| apart(pair[0], pair[1]));
        let around = !closed || shown.len() <= 2 || apart(shown[shown.len() - 1], shown[0]);
        neighbors && around
    };
    let stride = (1..count)
        .filter(|stride| !closed || count.is_multiple_of(*stride))
        .find(|stride| clear(&shown_at(*stride)))
        .unwrap_or(count.max(1));
    shown_at(stride)
}

/// Whether two boxes, each four corners in order around it, come closer than `gap`, by the
/// separating axis test.
pub(crate) fn boxes_overlap(a: &[[f32; 2]; 4], b: &[[f32; 2]; 4], gap: f32) -> bool {
    let edges = [a, b].map(|corners| {
        [corners[1], corners[3]].map(|end| [end[0] - corners[0][0], end[1] - corners[0][1]])
    });
    edges.iter().flatten().all(|edge| {
        let length = edge[0].hypot(edge[1]);
        if length == 0.0 {
            return true;
        }
        let axis = [edge[0] / length, edge[1] / length];
        let project = |corners: &[[f32; 2]; 4]| {
            corners
                .iter()
                .map(|point| point[0] * axis[0] + point[1] * axis[1])
                .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), value| {
                    (low.min(value), high.max(value))
                })
        };
        let ((a_low, a_high), (b_low, b_high)) = (project(a), project(b));
        a_low < b_high + gap && b_low < a_high + gap
    })
}

/// Extends `reach` to every drawn label of `row`, on a part of `length`.
pub(crate) fn label_reach(
    reach: &mut Reach,
    row: &LabelRow,
    side: GuideSide,
    length: f32,
    style: &AxisStyle,
) {
    let (align, baseline, shift) = placement(side, row.angle, style);
    for (position, bounds) in row.positions.iter().zip(&row.labels.bounds) {
        let Some(bounds) = bounds else {
            continue;
        };
        let anchor = side.point(position - shift, row.distance);
        for corner in corners(bounds, anchor, align, baseline, row.angle) {
            reach.add(side, length, corner);
        }
    }
}

/// The title's anchor, alignment and angle: centered on the length, `margin` past the part's
/// inner edge, reading up beside a left axis and down beside a right one.
fn title_placement(length: f32, margin: f32, side: GuideSide) -> ([f32; 2], TextBaseline, f32) {
    let anchor = side.point(length / 2.0, margin);
    let (baseline, angle) = match side {
        GuideSide::Left => (TextBaseline::LineBottom, -90.0),
        GuideSide::Right => (TextBaseline::LineBottom, 90.0),
        GuideSide::Top => (TextBaseline::Bottom, 0.0),
        GuideSide::Bottom => (TextBaseline::Top, 0.0),
    };
    (anchor, baseline, angle)
}

/// The title centered on a part of `length`, `margin` past its inner edge.
pub(crate) fn title_mark(
    text: &str,
    length: f32,
    margin: f32,
    side: GuideSide,
    style: &AxisStyle,
) -> SceneTextMark {
    let (anchor, baseline, angle) = title_placement(length, margin, side);
    SceneTextMark {
        clip: false,
        len: 1,
        text: text.to_string().into(),
        x: anchor[0].into(),
        y: anchor[1].into(),
        align: TextAlign::Center.into(),
        baseline: baseline.into(),
        angle: angle.into(),
        color: style.title_color.into(),
        font: style.title_font_family.clone().into(),
        font_size: style.title_font_size.into(),
        font_weight: style.title_font_weight.into(),
        text_syntax: style.title_syntax,
        ..Default::default()
    }
}

/// How far the title reaches on a part of `length`.
pub(crate) fn title_reach(title: &Title, length: f32, margin: f32, side: GuideSide) -> Reach {
    let (anchor, baseline, angle) = title_placement(length, margin, side);
    let mut reach = Reach::default();
    for corner in corners(&title.bounds, anchor, TextAlign::Center, baseline, angle) {
        reach.add(side, length, corner);
    }
    reach
}

/// Grid lines at `positions` across a plot of `size`, behind the data marks. They aren't
/// interactive, so that they never take a hit from the marks they cross.
pub(crate) fn grid_group(
    positions: &[f32],
    side: GuideSide,
    size: [f32; 2],
    style: &AxisStyle,
) -> SceneGroup {
    let [width, height] = size;
    let along = ScalarOrArray::new_array(positions.to_vec());
    let (x, x2, y, y2) = if side.is_vertical() {
        (0.0.into(), width.into(), along.clone(), along)
    } else {
        (along.clone(), along, 0.0.into(), height.into())
    };
    let grid = SceneRuleMark {
        len: positions.len() as u32,
        clip: false,
        x,
        x2,
        y,
        y2,
        stroke: ColorOrGradient::Color(style.grid_color).into(),
        stroke_width: style.grid_width.into(),
        ..Default::default()
    };
    SceneGroup {
        interactive: Some(false),
        zindex: Some(-1),
        marks: vec![grid.into()],
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use avenger_geometry::{marks::TextGeometryUtils, rtree::SceneGraphRTree};
    use avenger_scenegraph::{marks::symbol::SceneSymbolMark, scene_graph::SceneGraph};

    use super::*;

    const SIDES: [GuideSide; 4] = [
        GuideSide::Left,
        GuideSide::Right,
        GuideSide::Top,
        GuideSide::Bottom,
    ];

    /// One label at 50 along the axis, a margin into the labels part, at `angle`.
    fn row(text: &str, angle: f32, style: &AxisStyle) -> LabelRow {
        LabelRow {
            labels: Arc::new(Labels::measure(
                vec![text.to_string()],
                TextSyntaxMode::Plain,
                style,
                &avenger_typst_label::bundled_label_engine(),
            )),
            positions: vec![50.0],
            distance: TEXT_MARGIN,
            angle,
        }
    }

    /// Vega-Lite's `defaultLabelAlign` and `defaultLabelBaseline` (`compile/axis/properties.ts`,
    /// v6.4.2), written out line by line. Where they return `null`, Vega's default for the side
    /// applies: centered labels on top and bottom axes, and middle labels on left and right ones.
    #[allow(
        clippy::manual_range_contains,
        reason = "The comparisons follow Vega-Lite's source."
    )]
    fn vega_lite(side: GuideSide, angle: f32) -> (TextAlign, TextBaseline) {
        let angle = ((angle % 360.0) + 360.0) % 360.0;
        let is_x = matches!(side, GuideSide::Top | GuideSide::Bottom);
        let start_angle = if is_x { 0.0 } else { 90.0 };
        let main_side = if is_x {
            GuideSide::Bottom
        } else {
            GuideSide::Left
        };
        let align = if (angle + start_angle) % 180.0 == 0.0 {
            TextAlign::Center
        } else if (start_angle < angle && angle < 180.0 + start_angle) == (side == main_side) {
            TextAlign::Left
        } else {
            TextAlign::Right
        };
        let baseline = if is_x {
            if (45.0 < angle && angle < 135.0) || (225.0 < angle && angle < 315.0) {
                TextBaseline::Middle
            } else if (angle <= 45.0 || 315.0 <= angle) == (side == GuideSide::Top) {
                TextBaseline::Bottom
            } else {
                TextBaseline::Top
            }
        } else if angle <= 45.0 || 315.0 <= angle || (135.0 <= angle && angle <= 225.0) {
            TextBaseline::Middle
        } else if (45.0 <= angle && angle <= 135.0) == (side == GuideSide::Left) {
            TextBaseline::Top
        } else {
            TextBaseline::Bottom
        };
        (align, baseline)
    }

    #[test]
    fn labels_align_as_vega_lite_aligns_them() {
        for side in SIDES {
            for angle in (-720..=1440).map(|half| half as f32 / 2.0) {
                assert_eq!(
                    label_alignment(side, angle),
                    vega_lite(side, angle),
                    "{side:?} {angle}"
                );
            }
        }
        use {TextAlign as A, TextBaseline as B};
        let bottom = |angle| label_alignment(GuideSide::Bottom, angle);
        assert_eq!(bottom(0.0), (A::Center, B::Top));
        assert_eq!(bottom(-45.0), (A::Right, B::Top));
        assert_eq!(bottom(-90.0), (A::Right, B::Middle));
        assert_eq!(bottom(90.0), (A::Left, B::Middle));
        let left = |angle| label_alignment(GuideSide::Left, angle);
        assert_eq!(left(0.0), (A::Right, B::Middle));
        assert_eq!(left(90.0), (A::Center, B::Top));
        assert_eq!(left(-90.0), (A::Center, B::Bottom));
    }

    /// At every angle, labels stay outside the domain line, a tick length inside the labels
    /// part, and begin at their position, a margin past the part's inner edge.
    #[test]
    fn rotated_labels_extend_away_from_the_axis() {
        let engine = avenger_typst_label::bundled_label_engine();
        let style = AxisStyle::default();
        for side in SIDES {
            for angle in (0..360).step_by(15).map(|angle| angle as f32) {
                let mark = label_mark(&row("Category label", angle, &style), side, &style);
                let mut reach = Reach::default();
                label_reach(
                    &mut reach,
                    &row("Category label", angle, &style),
                    side,
                    100.0,
                    &style,
                );
                let group = SceneGroup {
                    marks: vec![mark.into()],
                    ..Default::default()
                };
                let bounds = group.bounding_box(&engine);
                let (lower, upper) = (bounds.lower(), bounds.upper());
                // The near edge, measured outward from the inner edge, inside the 1-pixel
                // margin that the bounding box adds.
                let near = match side {
                    GuideSide::Left => -upper[0],
                    GuideSide::Right => lower[0],
                    GuideSide::Top => -upper[1],
                    GuideSide::Bottom => lower[1],
                } + 1.0;
                assert!(
                    -style.tick_length < near && near <= TEXT_MARGIN + 0.01,
                    "{side:?} {angle}: near edge {near}"
                );
                // The far edge is the reach that the labels part reports.
                let far = match side {
                    GuideSide::Left => -lower[0],
                    GuideSide::Right => upper[0],
                    GuideSide::Top => -lower[1],
                    GuideSide::Bottom => upper[1],
                } - 1.0;
                assert!((far - reach.across).abs() < 0.01, "{side:?} {angle}");
            }
        }
    }

    #[test]
    fn styles_set_alignment_and_only_unrotated_vertical_labels_shift() {
        let style = AxisStyle {
            label_align: Some(TextAlign::Left),
            label_baseline: Some(TextBaseline::Alphabetic),
            ..Default::default()
        };
        let mark = label_mark(&row("1", 0.0, &style), GuideSide::Bottom, &style);
        assert_eq!(mark.align, TextAlign::Left.into());
        assert_eq!(mark.baseline, TextBaseline::Alphabetic.into());

        // Labels beside a vertical axis sit a tenth of their size higher.
        let y = |angle| {
            let style = AxisStyle::default();
            let mark = label_mark(&row("1", angle, &style), GuideSide::Left, &style);
            mark.y.as_vec(1, None)[0]
        };
        assert_eq!(y(0.0), 50.0 - 1.2);
        assert_eq!(y(30.0), 50.0);
    }

    #[test]
    fn grid_behind_a_symbol_cannot_replace_it_as_the_top_hit() {
        let grid = grid_group(
            &[50.0],
            GuideSide::Left,
            [100.0, 100.0],
            &AxisStyle::default(),
        );

        let point = SceneSymbolMark {
            name: "point".into(),
            x: 50.0.into(),
            y: 50.0.into(),
            size: 64.0.into(),
            ..Default::default()
        };
        let scene = SceneGraph {
            // Keep the guide later in document order to reproduce the prior
            // conflict between its scene path and its negative visual z-index.
            marks: vec![point.into(), grid.into()],
            width: 100.0,
            height: 100.0,
            origin: [0.0, 0.0],
        };
        let rtree =
            SceneGraphRTree::from_scene_graph(&scene, &avenger_typst_label::bundled_label_engine());
        let topmost = avenger_geometry::query::Query::new(
            avenger_geometry::query::QueryShape::Point([50.0, 50.0]),
            avenger_geometry::query::Relation::Intersects,
        );
        assert_eq!(
            rtree
                .query(&topmost)
                .first()
                .expect("point at grid intersection")
                .mark_instance
                .name,
            "point"
        );
    }

    /// Boxes 10 high from each `[left, right]`, along a line.
    fn spans(spans: &[[f32; 2]]) -> Vec<Option<[[f32; 2]; 4]>> {
        spans
            .iter()
            .map(|[left, right]| Some([[*left, 0.0], [*right, 0.0], [*right, 10.0], [*left, 10.0]]))
            .collect()
    }

    /// The third label overlaps the fourth, so every second label shows. Labels that each overlap
    /// their neighbors but not the ones past them take every second label too.
    #[test]
    fn overlapping_labels_thin_to_the_smallest_stride_that_keeps_them_apart() {
        let boxes = spans(&[
            [0.0, 5.0],
            [10.0, 15.0],
            [20.0, 35.0],
            [30.0, 35.0],
            [40.0, 45.0],
        ]);
        assert_eq!(stride_labels(&boxes, LABEL_GAP, false), [0, 2, 4]);

        let crowded: Vec<[f32; 2]> = (0..8)
            .map(|i| [i as f32 * 10.0, i as f32 * 10.0 + 15.0])
            .collect();
        assert_eq!(
            stride_labels(&spans(&crowded), LABEL_GAP, false),
            [0, 2, 4, 6]
        );

        // A label that draws nothing neighbors no label and shows nothing.
        let mut gapped = spans(&[[0.0, 15.0], [10.0, 25.0], [30.0, 35.0]]);
        gapped[1] = None;
        assert_eq!(stride_labels(&gapped, LABEL_GAP, false), [0, 2]);
    }

    /// Around a closed loop, the last label neighbors the first, and a stride divides the number
    /// of labels.
    #[test]
    fn thinning_around_a_loop_keeps_the_pattern_even() {
        // The sixth label overlaps the first.
        let mut around = spans(&[
            [0.0, 5.0],
            [10.0, 15.0],
            [20.0, 25.0],
            [30.0, 35.0],
            [40.0, 45.0],
        ]);
        around.extend(spans(&[[3.0, 8.0]]));
        assert_eq!(stride_labels(&around, LABEL_GAP, false).len(), 6);
        assert_eq!(stride_labels(&around, LABEL_GAP, true), [0, 2, 4]);

        // Eight crowded labels around a loop take a stride of 2, which divides 8, and five take a
        // stride of 5, since no smaller stride divides it.
        let crowded = |count: usize| {
            let ranges: Vec<[f32; 2]> = (0..count)
                .map(|i| [i as f32 * 10.0, i as f32 * 10.0 + 15.0])
                .collect();
            stride_labels(&spans(&ranges), LABEL_GAP, true)
        };
        assert_eq!(crowded(8), [0, 2, 4, 6]);
        assert_eq!(crowded(5), [0]);
    }
}
