//! The graticule: a map's meridians and parallels, labeled where they meet the
//! map's edge, as an axis labels its ticks.

use crate::error::AvengerGuidesGeoError;
use avenger_color::ColorOrGradient;
use avenger_common::types::{
    FontStyle, FontWeight, StrokeCap, StrokeJoin, TextAlign, TextBaseline, TextSyntaxMode,
};
use avenger_format::{FormatValues, TickSpacing};
use avenger_geo::{GeoSink, Graticule, GraticuleLine, GraticuleLineKind, Projector};
use avenger_scenegraph::marks::{
    group::SceneGroup,
    mark::SceneMark,
    path::ScenePathMark,
    text::{text_label, text_origin, text_style, SceneTextMark},
};
use avenger_typst_label::{escape_text, LabelEngine, LabelOptions, LabelValue};

/// A side of a map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapSide {
    Top,
    Bottom,
    Left,
    Right,
}

/// The graticule lines to draw, and where to label them.
#[derive(Debug, Clone)]
pub struct GraticuleConfig {
    pub graticule: Graticule,
    /// The sides that label the lines that reach them: meridians on the top and
    /// bottom, and parallels on the left and right. Without sides, the lines
    /// have no labels.
    pub label_sides: Vec<MapSide>,
    /// The pattern for each label's degrees, in the syntax of the text engine's
    /// number provider, such as `".1f"`. An empty pattern takes the provider's
    /// default.
    pub label_format: String,
    pub style: GraticuleStyle,
}

impl Default for GraticuleConfig {
    fn default() -> Self {
        Self {
            graticule: Graticule::default(),
            label_sides: vec![MapSide::Left, MapSide::Bottom],
            label_format: String::new(),
            style: GraticuleStyle::default(),
        }
    }
}

/// Graticule appearance.
#[derive(Debug, Clone)]
pub struct GraticuleStyle {
    pub stroke: [f32; 4],
    pub stroke_width: f32,
    pub label_color: [f32; 4],
    pub label_font_family: String,
    pub label_font_size: f32,
    pub label_font_weight: FontWeight,
    /// The distance between a line's end and its label.
    pub label_padding: f32,
}

impl Default for GraticuleStyle {
    fn default() -> Self {
        Self {
            stroke: [0.0, 0.0, 0.0, 0.18],
            stroke_width: 0.7,
            label_color: [0.0, 0.0, 0.0, 1.0],
            label_font_family: "sans-serif".to_string(),
            label_font_size: 10.0,
            label_font_weight: FontWeight::default(),
            label_padding: 4.0,
        }
    }
}

/// The graticule through `projector`: a group with a path mark of its lines and
/// a text mark of labels for each side in `config.label_sides`. Draw it before
/// the map's data.
///
/// A line is labeled on a side where it ends: on that edge of the projector's
/// clip extent when it has one, or else at the line's end nearest the side,
/// which lies on the projection's outline. Each label is the line's degrees,
/// formatted by the text engine's number provider, followed by `°` and N, S, E,
/// or W. While a side's labels overlap, every other one is left out, so the
/// rest stay evenly spaced, and then a label that overlaps one on an earlier
/// side is left out too.
///
/// # Errors
/// Returns an error when the graticule's configuration is invalid, or when the
/// text engine can't format or lay out a label.
pub fn make_graticule_marks(
    projector: &Projector,
    config: &GraticuleConfig,
    text_engine: &LabelEngine,
) -> Result<SceneGroup, AvengerGuidesGeoError> {
    let lines = config.graticule.graticule_lines()?;
    let mut marks: Vec<SceneMark> = Vec::new();
    let path = projector.project_to_path(&config.graticule.lines()?);
    if path.iter().next().is_some() {
        marks.push(
            ScenePathMark {
                path: path.into(),
                fill: ColorOrGradient::Color([0.0; 4]).into(),
                stroke: ColorOrGradient::Color(config.style.stroke).into(),
                stroke_width: Some(config.style.stroke_width),
                stroke_cap: StrokeCap::Round,
                stroke_join: StrokeJoin::Round,
                ..Default::default()
            }
            .into(),
        );
    }
    let mut placed = Vec::new();
    for &side in &config.label_sides {
        if let Some(labels) =
            side_labels(projector, &lines, side, config, text_engine, &mut placed)?
        {
            marks.push(labels.into());
        }
    }
    Ok(SceneGroup {
        interactive: Some(false),
        marks,
        ..Default::default()
    })
}

/// A label's box: its top left and bottom right.
type LabelBox = [[f32; 2]; 2];

/// The labels of the lines that meet `side`, without those that would overlap,
/// either each other or a box in `placed`, which their boxes join.
fn side_labels(
    projector: &Projector,
    lines: &[GraticuleLine],
    side: MapSide,
    config: &GraticuleConfig,
    text_engine: &LabelEngine,
    placed: &mut Vec<LabelBox>,
) -> Result<Option<SceneTextMark>, AvengerGuidesGeoError> {
    let kind = match side {
        MapSide::Top | MapSide::Bottom => GraticuleLineKind::Meridian,
        MapSide::Left | MapSide::Right => GraticuleLineKind::Parallel,
    };
    let frame = projector.clip_extent();
    let mut ends: Vec<(f64, [f64; 2])> = lines
        .iter()
        .filter(|line| line.kind == kind)
        .filter_map(|line| {
            let mut sink = LineEnds::default();
            projector.project_to_sink(&line.line, &mut sink);
            side_end(&sink.ends, side, frame).map(|end| (line.value, end))
        })
        .collect();
    if ends.is_empty() {
        return Ok(None);
    }
    let along = |end: &[f64; 2]| match side {
        MapSide::Top | MapSide::Bottom => end[0],
        MapSide::Left | MapSide::Right => end[1],
    };
    ends.sort_by(|a, b| along(&a.1).total_cmp(&along(&b.1)));

    // Each label keeps its own precision: 22.5° doesn't make 30° read 30.0°.
    let degrees: Vec<f64> = ends.iter().map(|(value, _)| value.abs()).collect();
    let formatted = text_engine.format(
        FormatValues::Numbers(&degrees),
        &config.label_format,
        TickSpacing::Varying,
    )?;

    let style = &config.style;
    let (align, baseline) = match side {
        MapSide::Left => (TextAlign::Right, TextBaseline::Middle),
        MapSide::Right => (TextAlign::Left, TextBaseline::Middle),
        MapSide::Top => (TextAlign::Center, TextBaseline::Bottom),
        MapSide::Bottom => (TextAlign::Center, TextBaseline::Top),
    };
    let options = LabelOptions {
        text: text_style(
            &style.label_font_family,
            style.label_font_size,
            style.label_font_weight,
            FontStyle::Normal,
            style.label_color,
        ),
        ..Default::default()
    };
    let pad = style.label_padding;
    let mut labels = Vec::new();
    for ((value, [ex, ey]), degrees) in ends.into_iter().zip(&formatted) {
        let text = label_text(kind, value, degrees);
        let (ex, ey) = (ex as f32, ey as f32);
        let position = match side {
            MapSide::Left => [ex - pad, ey],
            MapSide::Right => [ex + pad, ey],
            MapSide::Top => [ex, ey - pad],
            MapSide::Bottom => [ex, ey + pad],
        };
        let bounds = text_engine.bounds(&text_label(
            &text,
            TextSyntaxMode::TypstMarkup,
            options.clone(),
        ))?;
        let [left, top] = text_origin(&bounds, position, align, baseline);
        let label_box = [[left, top], [left + bounds.width, top + bounds.height]];
        labels.push((text, position, label_box));
    }
    // Vega's "parity" strategy: halve the labels until neighbors don't overlap.
    while labels
        .windows(2)
        .any(|pair| overlaps(&pair[0].2, &pair[1].2))
    {
        labels = labels.into_iter().step_by(2).collect();
    }
    labels.retain(|(_, _, label_box)| !placed.iter().any(|other| overlaps(label_box, other)));
    placed.extend(labels.iter().map(|(_, _, label_box)| *label_box));
    let (mut text, mut x, mut y) = (Vec::new(), Vec::new(), Vec::new());
    for (label, position, _) in labels {
        text.push(label);
        x.push(position[0]);
        y.push(position[1]);
    }
    Ok(Some(SceneTextMark {
        len: text.len() as u32,
        clip: false,
        text: text.into(),
        text_syntax: TextSyntaxMode::TypstMarkup,
        x: x.into(),
        y: y.into(),
        align: align.into(),
        baseline: baseline.into(),
        color: style.label_color.into(),
        font: style.label_font_family.clone().into(),
        font_size: style.label_font_size.into(),
        font_weight: style.label_font_weight.into(),
        ..Default::default()
    }))
}

/// Of the ends of a line's pieces, the one nearest `side`: on that edge of
/// `frame` when there is one.
fn side_end(ends: &[[f64; 2]], side: MapSide, frame: Option<[[f64; 2]; 2]>) -> Option<[f64; 2]> {
    let (axis, nearest_is_max) = match side {
        MapSide::Left => (0, false),
        MapSide::Right => (0, true),
        MapSide::Top => (1, false),
        MapSide::Bottom => (1, true),
    };
    let by_axis = |a: &&[f64; 2], b: &&[f64; 2]| a[axis].total_cmp(&b[axis]);
    let end = *if nearest_is_max {
        ends.iter().max_by(by_axis)
    } else {
        ends.iter().min_by(by_axis)
    }?;
    let Some(frame) = frame else {
        return Some(end);
    };
    let edge = frame[usize::from(nearest_is_max)][axis];
    ((end[axis] - edge).abs() <= 1e-6 * (1.0 + edge.abs())).then_some(end)
}

/// A label's markup: the formatted degrees, `°`, and the hemisphere.
fn label_text(kind: GraticuleLineKind, value: f64, degrees: &LabelValue) -> String {
    let degrees = match degrees {
        LabelValue::Markup(markup) => markup.clone(),
        LabelValue::Str(text) => escape_text(text),
        degrees => unreachable!("formatted numbers are text or markup, not {degrees:?}"),
    };
    let hemisphere = match kind {
        // The prime meridian, the equator, and the antimeridian name no hemisphere.
        _ if value.abs() < 1e-9 => "",
        GraticuleLineKind::Meridian if value.abs() >= 180.0 => "",
        GraticuleLineKind::Meridian if value > 0.0 => "E",
        GraticuleLineKind::Meridian => "W",
        GraticuleLineKind::Parallel if value > 0.0 => "N",
        GraticuleLineKind::Parallel => "S",
    };
    format!("{degrees}°{hemisphere}")
}

fn overlaps(a: &LabelBox, b: &LabelBox) -> bool {
    a[0][0] < b[1][0] && b[0][0] < a[1][0] && a[0][1] < b[1][1] && b[0][1] < a[1][1]
}

/// The first and last points of each streamed line piece.
#[derive(Default)]
struct LineEnds {
    ends: Vec<[f64; 2]>,
    first: Option<[f64; 2]>,
    last: Option<[f64; 2]>,
}

impl GeoSink for LineEnds {
    fn point(&mut self, x: f64, y: f64, _m: Option<f64>) {
        if x.is_finite() && y.is_finite() {
            self.first.get_or_insert([x, y]);
            self.last = Some([x, y]);
        }
    }
    fn line_start(&mut self) {
        self.first = None;
        self.last = None;
    }
    fn line_end(&mut self) {
        self.ends.extend(self.first.take());
        self.ends.extend(self.last.take());
    }
    fn polygon_start(&mut self) {}
    fn polygon_end(&mut self) {}
}
