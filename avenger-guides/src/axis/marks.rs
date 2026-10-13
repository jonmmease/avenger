//! The marks every axis draws: its grid, domain line, ticks, labels, and title.

use avenger_color::ColorOrGradient;
use avenger_common::{
    types::{FontStyle, TextAlign, TextBaseline, TextSyntaxMode},
    value::ScalarOrArray,
};
use avenger_geometry::marks::TextGeometryUtils;
use avenger_scenegraph::marks::{
    group::SceneGroup,
    rule::SceneRuleMark,
    text::{text_label, text_style, SceneTextMark},
};
use avenger_typst_label::{LabelEngine, LabelOptions};
use rstar::AABB;

use super::opts::{AxisConfig, AxisOrientation};
use crate::error::AvengerGuidesError;

/// The gap between the end of a tick and its label.
pub(crate) const TEXT_MARGIN: f32 = 3.0;
/// How far the domain line reaches past each end of the range, so that it covers the outer ticks.
const PIXEL_OFFSET: f32 = 0.5;

/// What an axis shows along its scale's range, in the axis's coordinates.
pub(crate) struct AxisParts<'a> {
    /// The scale's range.
    pub(crate) range: (f32, f32),
    /// Tick positions.
    pub(crate) ticks: Vec<f32>,
    pub(crate) labels: TickLabels,
    pub(crate) title: &'a str,
    /// The gap between the labels and the title.
    pub(crate) title_margin: f32,
}

/// Tick labels and their positions.
#[derive(Default)]
pub(crate) struct TickLabels {
    pub(crate) positions: Vec<f32>,
    pub(crate) text: Vec<String>,
    pub(crate) syntax: TextSyntaxMode,
}

/// An axis at `origin`: its grid behind the data marks, and its domain line, ticks, labels, and
/// title above them.
pub(crate) fn make_axis(
    parts: AxisParts,
    origin: [f32; 2],
    config: &AxisConfig,
    text_engine: &LabelEngine,
) -> Result<SceneGroup, AvengerGuidesError> {
    let style = &config.style;
    let AxisParts {
        range,
        ticks,
        labels,
        title,
        title_margin,
    } = parts;
    let mut marks = Vec::new();
    if config.grid {
        marks.push(make_grid(&ticks, config).into());
    }
    let mut elements = SceneGroup {
        zindex: Some(1),
        ..Default::default()
    };
    elements.marks.push(make_domain_line(range, config).into());
    elements.marks.push(make_ticks(&ticks, config).into());
    if style.labels_visible {
        elements.marks.push(make_tick_labels(labels, config).into());
    }
    if style.title_visible && !title.is_empty() {
        let envelope = elements.bounding_box(text_engine);
        let title = make_title(title, range, title_margin, &envelope, config, text_engine)?;
        elements.marks.push(title.into());
    }
    marks.push(elements.into());
    Ok(SceneGroup {
        origin,
        marks,
        ..Default::default()
    })
}

/// How a label aligns to its position, as Vega-Lite chooses for an axis side and a label angle in
/// degrees clockwise: the label's end nearest its tick lies on the position, so that the label
/// extends away from the axis. Labels that run along the axis center on their ticks.
pub(crate) fn label_alignment(
    orientation: AxisOrientation,
    angle: f32,
) -> (TextAlign, TextBaseline) {
    let angle = angle.rem_euclid(360.0);
    match orientation {
        AxisOrientation::Top | AxisOrientation::Bottom => {
            let bottom = orientation == AxisOrientation::Bottom;
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
        AxisOrientation::Left | AxisOrientation::Right => {
            let left = orientation == AxisOrientation::Left;
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

/// Grid lines across the plot at the ticks, behind the data marks. They aren't interactive, so
/// that they never take a hit from the marks they cross.
fn make_grid(ticks: &[f32], config: &AxisConfig) -> SceneGroup {
    let [width, height] = config.dimensions;
    let positions = ScalarOrArray::new_array(ticks.to_vec());
    let (x, x2, y, y2) = match config.orientation {
        AxisOrientation::Left | AxisOrientation::Right => {
            (0.0.into(), width.into(), positions.clone(), positions)
        }
        AxisOrientation::Top | AxisOrientation::Bottom => {
            (positions.clone(), positions, 0.0.into(), height.into())
        }
    };
    let grid = SceneRuleMark {
        len: ticks.len() as u32,
        clip: false,
        x,
        x2,
        y,
        y2,
        stroke: ColorOrGradient::Color(config.style.grid_color).into(),
        stroke_width: config.style.grid_width.into(),
        ..Default::default()
    };
    SceneGroup {
        interactive: Some(false),
        zindex: Some(-1),
        marks: vec![grid.into()],
        ..Default::default()
    }
}

fn make_domain_line(range: (f32, f32), config: &AxisConfig) -> SceneRuleMark {
    let start = range.0.min(range.1) - PIXEL_OFFSET;
    let end = range.0.max(range.1) + PIXEL_OFFSET;
    let offset = axis_offset(config);
    let (x, x2, y, y2) = match config.orientation {
        AxisOrientation::Left | AxisOrientation::Right => (offset, offset, start, end),
        AxisOrientation::Top | AxisOrientation::Bottom => (start, end, offset, offset),
    };
    SceneRuleMark {
        x: x.into(),
        x2: x2.into(),
        y: y.into(),
        y2: y2.into(),
        stroke: ColorOrGradient::Color(config.style.domain_color).into(),
        stroke_width: 1.0.into(),
        ..Default::default()
    }
}

/// Where the domain line lies across the axis: at the plot's edge on the axis's side.
fn axis_offset(config: &AxisConfig) -> f32 {
    match config.orientation {
        AxisOrientation::Left | AxisOrientation::Top => 0.0,
        AxisOrientation::Right => config.dimensions[0],
        AxisOrientation::Bottom => config.dimensions[1],
    }
}

/// The sign of the direction away from the plot, across the axis.
fn outward(orientation: AxisOrientation) -> f32 {
    match orientation {
        AxisOrientation::Left | AxisOrientation::Top => -1.0,
        AxisOrientation::Right | AxisOrientation::Bottom => 1.0,
    }
}

fn make_ticks(ticks: &[f32], config: &AxisConfig) -> SceneRuleMark {
    let positions = ScalarOrArray::new_array(ticks.to_vec());
    let start = axis_offset(config);
    let end = start + outward(config.orientation) * config.style.tick_length;
    let (x, x2, y, y2) = match config.orientation {
        AxisOrientation::Left | AxisOrientation::Right => {
            (start.into(), end.into(), positions.clone(), positions)
        }
        AxisOrientation::Top | AxisOrientation::Bottom => {
            (positions.clone(), positions, start.into(), end.into())
        }
    };
    SceneRuleMark {
        len: ticks.len() as u32,
        clip: false,
        x,
        x2,
        y,
        y2,
        stroke: ColorOrGradient::Color(config.style.tick_color).into(),
        stroke_width: 1.0.into(),
        ..Default::default()
    }
}

/// Labels a margin past the tick ends, aligned to the end nearest their tick unless the style
/// sets the alignment.
fn make_tick_labels(labels: TickLabels, config: &AxisConfig) -> SceneTextMark {
    let style = &config.style;
    let (align, baseline) = label_alignment(config.orientation, style.label_angle);
    let align = style.label_align.unwrap_or(align);
    let baseline = style.label_baseline.unwrap_or(baseline);
    let across =
        axis_offset(config) + outward(config.orientation) * (style.tick_length + TEXT_MARGIN);
    let vertical = matches!(
        config.orientation,
        AxisOrientation::Left | AxisOrientation::Right
    );
    // Numbers don't reach the full descent, so unrotated labels beside a vertical axis sit a
    // tenth of their size higher to look centered on their ticks.
    let shift = if vertical
        && style.label_angle.rem_euclid(360.0) == 0.0
        && baseline == TextBaseline::Middle
    {
        style.label_font_size * 0.1
    } else {
        0.0
    };
    let along = ScalarOrArray::new_array(labels.positions.iter().map(|p| p - shift).collect());
    let (x, y) = if vertical {
        (across.into(), along)
    } else {
        (along, across.into())
    };
    SceneTextMark {
        clip: false,
        len: labels.text.len() as u32,
        text: ScalarOrArray::new_array(labels.text),
        x,
        y,
        align: align.into(),
        baseline: baseline.into(),
        angle: style.label_angle.into(),
        color: ColorOrGradient::Color(style.label_color).into(),
        font: style.label_font_family.clone().into(),
        font_size: style.label_font_size.into(),
        font_weight: style.label_font_weight.into(),
        text_syntax: labels.syntax,
        ..Default::default()
    }
}

/// The title, centered on the range, a margin past the labels and ticks in `envelope`.
fn make_title(
    title: &str,
    range: (f32, f32),
    margin: f32,
    envelope: &AABB<[f32; 2]>,
    config: &AxisConfig,
    text_engine: &LabelEngine,
) -> Result<SceneTextMark, AvengerGuidesError> {
    let style = &config.style;
    // Lay the title out once here, so that markup that doesn't typeset is the axis's error.
    text_engine.bounds(&text_label(
        title,
        style.title_syntax,
        LabelOptions {
            text: text_style(
                &style.title_font_family,
                style.title_font_size,
                style.title_font_weight,
                FontStyle::Normal,
                style.title_color,
            ),
            ..Default::default()
        },
    ))?;
    let mid = (range.0 + range.1) / 2.0;
    let (x, y, baseline, angle) = match config.orientation {
        AxisOrientation::Left => (
            envelope.lower()[0] - margin,
            mid,
            TextBaseline::LineBottom,
            -90.0,
        ),
        AxisOrientation::Right => (
            envelope.upper()[0] + margin,
            mid,
            TextBaseline::LineBottom,
            90.0,
        ),
        AxisOrientation::Top => (mid, envelope.lower()[1] - margin, TextBaseline::Bottom, 0.0),
        AxisOrientation::Bottom => (mid, envelope.upper()[1] + margin, TextBaseline::Top, 0.0),
    };
    Ok(SceneTextMark {
        clip: false,
        len: 1,
        text: title.to_string().into(),
        x: x.into(),
        y: y.into(),
        align: TextAlign::Center.into(),
        baseline: baseline.into(),
        angle: angle.into(),
        color: ColorOrGradient::Color(style.title_color).into(),
        font: style.title_font_family.clone().into(),
        font_size: style.title_font_size.into(),
        font_weight: style.title_font_weight.into(),
        text_syntax: style.title_syntax,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use avenger_format::NumberFormatProvider;
    use avenger_format_number_d3::D3NumberFormatProvider;
    use avenger_geometry::rtree::SceneGraphRTree;
    use avenger_scenegraph::{marks::symbol::SceneSymbolMark, scene_graph::SceneGraph};

    use super::super::opts::AxisStyle;
    use super::*;

    const SIDES: [AxisOrientation; 4] = [
        AxisOrientation::Left,
        AxisOrientation::Right,
        AxisOrientation::Top,
        AxisOrientation::Bottom,
    ];

    fn config(orientation: AxisOrientation, style: AxisStyle) -> AxisConfig {
        AxisConfig {
            orientation,
            dimensions: [100.0, 100.0],
            grid: false,
            tick_format: D3NumberFormatProvider::new().prepare(",").unwrap().into(),
            tick_label: None,
            style,
        }
    }

    fn labels(text: &str) -> TickLabels {
        TickLabels {
            positions: vec![50.0],
            text: vec![text.to_string()],
            syntax: TextSyntaxMode::Plain,
        }
    }

    /// Vega-Lite's `defaultLabelAlign` and `defaultLabelBaseline` (`compile/axis/properties.ts`,
    /// v6.4.2), written out line by line. Where they return `null`, Vega's default for the side
    /// applies: centered labels on top and bottom axes, and middle labels on left and right ones.
    #[allow(
        clippy::manual_range_contains,
        reason = "The comparisons follow Vega-Lite's source."
    )]
    fn vega_lite(orientation: AxisOrientation, angle: f32) -> (TextAlign, TextBaseline) {
        let angle = ((angle % 360.0) + 360.0) % 360.0;
        let is_x = matches!(orientation, AxisOrientation::Top | AxisOrientation::Bottom);
        let start_angle = if is_x { 0.0 } else { 90.0 };
        let main_orient = if is_x {
            AxisOrientation::Bottom
        } else {
            AxisOrientation::Left
        };
        let align = if (angle + start_angle) % 180.0 == 0.0 {
            TextAlign::Center
        } else if (start_angle < angle && angle < 180.0 + start_angle)
            == (orientation == main_orient)
        {
            TextAlign::Left
        } else {
            TextAlign::Right
        };
        let baseline = if is_x {
            if (45.0 < angle && angle < 135.0) || (225.0 < angle && angle < 315.0) {
                TextBaseline::Middle
            } else if (angle <= 45.0 || 315.0 <= angle) == (orientation == AxisOrientation::Top) {
                TextBaseline::Bottom
            } else {
                TextBaseline::Top
            }
        } else if angle <= 45.0 || 315.0 <= angle || (135.0 <= angle && angle <= 225.0) {
            TextBaseline::Middle
        } else if (45.0 <= angle && angle <= 135.0) == (orientation == AxisOrientation::Left) {
            TextBaseline::Top
        } else {
            TextBaseline::Bottom
        };
        (align, baseline)
    }

    #[test]
    fn labels_align_as_vega_lite_aligns_them() {
        for orientation in SIDES {
            for angle in (-720..=1440).map(|half| half as f32 / 2.0) {
                assert_eq!(
                    label_alignment(orientation, angle),
                    vega_lite(orientation, angle),
                    "{orientation:?} {angle}"
                );
            }
        }
        use {TextAlign as A, TextBaseline as B};
        let bottom = |angle| label_alignment(AxisOrientation::Bottom, angle);
        assert_eq!(bottom(0.0), (A::Center, B::Top));
        assert_eq!(bottom(-45.0), (A::Right, B::Top));
        assert_eq!(bottom(-90.0), (A::Right, B::Middle));
        assert_eq!(bottom(90.0), (A::Left, B::Middle));
        let left = |angle| label_alignment(AxisOrientation::Left, angle);
        assert_eq!(left(0.0), (A::Right, B::Middle));
        assert_eq!(left(90.0), (A::Center, B::Top));
        assert_eq!(left(-90.0), (A::Center, B::Bottom));
    }

    /// At every angle, labels stay outside the domain line and begin at their position, a margin
    /// past the end of their tick.
    #[test]
    fn rotated_labels_extend_away_from_the_axis() {
        let engine = avenger_typst_label::bundled_label_engine();
        for orientation in SIDES {
            for angle in (0..360).step_by(15).map(|angle| angle as f32) {
                let config = config(
                    orientation,
                    AxisStyle {
                        label_angle: angle,
                        ..Default::default()
                    },
                );
                let mark = make_tick_labels(labels("Category label"), &config);
                let group = SceneGroup {
                    marks: vec![mark.into()],
                    ..Default::default()
                };
                let bounds = group.bounding_box(&engine);
                let (lower, upper) = (bounds.lower(), bounds.upper());
                // The near edge, measured outward from the domain line.
                let near = match orientation {
                    AxisOrientation::Left => -upper[0],
                    AxisOrientation::Right => lower[0] - 100.0,
                    AxisOrientation::Top => -upper[1],
                    AxisOrientation::Bottom => lower[1] - 100.0,
                };
                let anchor = config.style.tick_length + TEXT_MARGIN;
                assert!(
                    0.0 < near && near <= anchor + 0.01,
                    "{orientation:?} {angle}: near edge {near}"
                );
            }
        }
    }

    #[test]
    fn labels_keep_their_margin_past_the_ticks() {
        for orientation in SIDES {
            for tick_length in [5.0, 25.0] {
                let config = config(
                    orientation,
                    AxisStyle {
                        tick_length,
                        ..Default::default()
                    },
                );
                let ticks = make_ticks(&[50.0], &config);
                let labels = make_tick_labels(labels("1"), &config);
                let (tick_end, label) = match orientation {
                    AxisOrientation::Left | AxisOrientation::Right => {
                        (ticks.x2.as_vec(1, None)[0], labels.x.as_vec(1, None)[0])
                    }
                    AxisOrientation::Top | AxisOrientation::Bottom => {
                        (ticks.y2.as_vec(1, None)[0], labels.y.as_vec(1, None)[0])
                    }
                };
                let clearance = outward(orientation) * (label - tick_end);
                assert_eq!(clearance, TEXT_MARGIN, "{orientation:?} {tick_length}");
            }
        }
    }

    #[test]
    fn styles_set_alignment_and_only_level_labels_shift() {
        let style = AxisStyle {
            label_align: Some(TextAlign::Left),
            label_baseline: Some(TextBaseline::Alphabetic),
            ..Default::default()
        };
        let mark = make_tick_labels(labels("1"), &config(AxisOrientation::Bottom, style));
        assert_eq!(mark.align, TextAlign::Left.into());
        assert_eq!(mark.baseline, TextBaseline::Alphabetic.into());

        // Level labels beside a vertical axis sit a tenth of their size higher.
        let y = |angle| {
            let style = AxisStyle {
                label_angle: angle,
                ..Default::default()
            };
            let mark = make_tick_labels(labels("1"), &config(AxisOrientation::Left, style));
            mark.y.as_vec(1, None)[0]
        };
        assert_eq!(y(0.0), 50.0 - 1.2);
        assert_eq!(y(30.0), 50.0);
    }

    #[test]
    fn grid_behind_a_symbol_cannot_replace_it_as_the_top_hit() {
        let config = config(AxisOrientation::Left, AxisStyle::default());
        let grid = make_grid(&[50.0], &config);

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
        assert_eq!(
            rtree
                .pick_top_mark_at_point(&[50.0, 50.0])
                .expect("point at grid intersection")
                .name,
            "point"
        );
    }
}
