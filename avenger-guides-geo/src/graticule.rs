//! The graticule: a map's meridians and parallels, labeled where they meet the
//! map's edge, as an axis labels its ticks.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use crate::error::AvengerGuidesGeoError;
use avenger_color::ColorOrGradient;
use avenger_common::types::{
    FontStyle, FontWeight, StrokeCap, StrokeJoin, TextAlign, TextBaseline, TextSyntaxMode,
};
use avenger_format::{FormatValues, TickSpacing};
use avenger_geo::{
    AvengerGeoError, FitSummary, GeoSink, GraticuleLine, GraticuleLineKind, Projection, Projector,
};
use avenger_guides::{GuideSide, PartExtent};
use avenger_scenegraph::marks::{
    group::SceneGroup,
    path::ScenePathMark,
    text::{text_label, text_origin, text_style, SceneTextMark},
};
use avenger_typst_label::{escape_text, LabelEngine, LabelOptions, LabelValue, TextBounds};
use geo_types::MultiLineString;

/// The most placements that a graticule keeps, one for each map size.
const KEPT_PLACEMENTS: usize = 8;

/// The smallest width and height that a map is fitted in, so that a map without area still has
/// a projector.
const MIN_SIZE: f64 = 1e-6;

/// A map's edge, where its lines are labeled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MapFrame {
    /// The projection's outline, fitted in the map's rectangle, as a world map's is. A line is
    /// labeled at its end nearest the side, on the outline.
    #[default]
    Outline,
    /// The map's rectangle, which the map fills and is clipped to, as a regional map is. A line
    /// is labeled where it crosses the side's edge.
    Rectangle,
}

/// The graticule lines to draw, the map's frame, and where to label the lines.
#[derive(Debug, Clone)]
pub struct GraticuleConfig {
    pub graticule: avenger_geo::Graticule,
    pub frame: MapFrame,
    /// The sides that label the lines that reach them: meridians on the top and
    /// bottom, and parallels on the left and right. Without sides, the lines
    /// have no labels.
    pub label_sides: Vec<GuideSide>,
    /// The pattern for each label's degrees, in the syntax of the text engine's
    /// number provider, such as `".1f"`. An empty pattern takes the provider's
    /// default.
    pub label_format: String,
    pub style: GraticuleStyle,
}

impl Default for GraticuleConfig {
    fn default() -> Self {
        Self {
            graticule: avenger_geo::Graticule::default(),
            frame: MapFrame::default(),
            label_sides: vec![GuideSide::Left, GuideSide::Bottom],
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

/// A graticule prepared from an unfitted projection, a summary of what the map shows, a
/// configuration and a text engine, without a size.
///
/// Every method takes the map's size, or the length of one of its sides, and fits the
/// projection to it, so the labels' sizes, the drawing and the data's positions agree at every
/// size. A side's length gives the map's size at [`Graticule::aspect`], where the fitted map
/// fills its rectangle. The constructor checks the configuration, formats and measures the
/// labels, and fits the projection once, so the methods do not fail.
///
/// Each label is the line's degrees, formatted by the text engine's number provider, followed
/// by `°` and N, S, E, or W. While a side's labels overlap, every other one is left out, so the
/// rest stay evenly spaced. Then a label that overlaps one on an earlier side in
/// [`GraticuleConfig::label_sides`] is left out too.
#[derive(Debug)]
pub struct Graticule {
    /// The projection as the caller gave it, which each size fits.
    projection: Projection,
    summary: FitSummary,
    aspect: f32,
    frame: MapFrame,
    label_sides: Vec<GuideSide>,
    style: GraticuleStyle,
    /// The graticule's lines.
    lines: MultiLineString<f64>,
    /// Each line's label, in the order of `lines`, or none when no side is labeled.
    labels: Vec<LineLabel>,
    /// The most recent placements, newest first.
    kept: Mutex<VecDeque<Arc<Placement>>>,
}

/// A line's label, with the kind of line it labels.
#[derive(Debug)]
struct LineLabel {
    kind: GraticuleLineKind,
    text: String,
    bounds: TextBounds,
}

/// The graticule at one map size, in the map's frame.
#[derive(Debug)]
struct Placement {
    size: [f32; 2],
    lines: SceneGroup,
    /// The labels of each side, in the order of `label_sides`.
    sides: Vec<Vec<Placed>>,
}

/// A label placed at a line's end.
#[derive(Debug)]
struct Placed {
    /// The index of the label in `labels`.
    label: usize,
    position: [f32; 2],
    bounds: LabelBox,
}

/// A label's box: its top left and bottom right.
type LabelBox = [[f32; 2]; 2];

impl Graticule {
    /// A graticule for maps through `projection`, fitted to the geometry that `summary`
    /// summarizes. The projection's view, if any, is replaced at each size.
    ///
    /// # Errors
    /// Returns an error when the projection has a clip extent, which the graticule sets under
    /// [`MapFrame::Rectangle`], when the summary has no finite, positive aspect, when
    /// `label_sides` lists a side twice, when the graticule's configuration is invalid, when the
    /// text engine can't format or lay out a label, or when the projection can't be fitted and
    /// built.
    pub fn new(
        projection: &Projection,
        summary: &FitSummary,
        config: &GraticuleConfig,
        engine: &LabelEngine,
    ) -> Result<Self, AvengerGuidesGeoError> {
        if projection.clip_extent.is_some() {
            return Err(AvengerGuidesGeoError::InvalidConfig(
                "the graticule fits the projection at each size, so the projection takes no \
                 clip extent"
                    .into(),
            ));
        }
        let aspect = summary
            .aspect()
            .map(|aspect| aspect as f32)
            .filter(|aspect| aspect.is_finite() && *aspect > 0.0)
            .ok_or_else(|| {
                AvengerGuidesGeoError::InvalidConfig(
                    "the summary's bounds need a finite, positive width and height".into(),
                )
            })?;
        for (index, side) in config.label_sides.iter().enumerate() {
            if config.label_sides[..index].contains(side) {
                return Err(AvengerGuidesGeoError::InvalidConfig(format!(
                    "label_sides lists {side:?} twice"
                )));
            }
        }
        let lines = config.graticule.graticule_lines()?;
        let labels = if config.label_sides.is_empty() {
            Vec::new()
        } else {
            line_labels(&lines, config, engine)?
        };
        let graticule = Self {
            projection: projection.clone(),
            summary: *summary,
            aspect,
            frame: config.frame,
            label_sides: config.label_sides.clone(),
            style: config.style.clone(),
            lines: MultiLineString(lines.into_iter().map(|line| line.line).collect()),
            labels,
            kept: Mutex::new(VecDeque::new()),
        };
        // Every size fits and builds like this one, so the methods do not fail.
        graticule.try_projector([100.0 * aspect, 100.0])?;
        Ok(graticule)
    }

    /// The map's width over its height, at which the fitted map fills its rectangle. A layout
    /// keeps a map's leaf at this ratio with `Leaf::aspect`.
    pub fn aspect(&self) -> f32 {
        self.aspect
    }

    /// The labels' thickness and overhang on `side` of a map whose side is `length` long, and
    /// the lengths where they hold, which is only `length`.
    ///
    /// The thickness is how far the labels reach outward past the map's edge. Labels at the ends
    /// of lines inside the edge, as on a world map's outline, take no thickness there. A side
    /// that isn't in [`GraticuleConfig::label_sides`] is empty at every length, and every side
    /// is empty at a length that isn't positive.
    pub fn extent(&self, side: GuideSide, length: f32) -> PartExtent {
        let Some(index) = self.side_index(side) else {
            return PartExtent {
                thickness: 0.0,
                overhang: [0.0; 2],
                holds: 0.0..=f32::INFINITY,
            };
        };
        let placement = self.placement(self.size_at(side, length));
        let edge = edge_origin(side, placement.size);
        let (mut across, mut start, mut end) = (0.0_f32, 0.0_f32, 0.0_f32);
        for label in &placement.sides[index] {
            for [x, y] in label.bounds {
                let (along, outward) = side.along_across([x - edge[0], y - edge[1]]);
                across = across.max(outward);
                start = start.max(-along);
                end = end.max(along - length);
            }
        }
        PartExtent {
            thickness: across,
            overhang: [start, end],
            holds: length..=length,
        }
    }

    /// The labels on `side` of a map whose side is `length` long, in the side's slab's frame, or
    /// `None` when the side has no labels at that length.
    ///
    /// Along the side, 0 is the start of its slab. Across it, 0 is the map's edge, and the slab
    /// extends outward: to positive coordinates below or right of the map, and to negative ones
    /// above or left of it.
    pub fn part(&self, side: GuideSide, length: f32) -> Option<SceneGroup> {
        let index = self.side_index(side)?;
        let placement = self.placement(self.size_at(side, length));
        let [x, y] = edge_origin(side, placement.size);
        let mark = self.text_mark(side, &placement.sides[index], [-x, -y])?;
        Some(SceneGroup {
            marks: vec![mark.into()],
            ..Default::default()
        })
    }

    /// The origin for a side's labels in the slab with rectangle `slot`, `[x, y, width,
    /// height]`: where the slab's inner edge, on the map's edge, meets its start.
    pub fn origin(&self, side: GuideSide, slot: [f32; 4]) -> [f32; 2] {
        side.slot_origin(slot)
    }

    /// The projection fitted to a map of `size`, with its top-left corner at 0, for the map's
    /// data. Under [`MapFrame::Rectangle`], it clips to the map's rectangle. A map without area
    /// shrinks to its corner.
    pub fn projector(&self, size: [f32; 2]) -> Projector {
        self.try_projector(size)
            .expect("the constructor fitted and built the projection")
    }

    /// The graticule's lines across a map of `size`, in the map's frame, behind its data.
    pub fn lines(&self, size: [f32; 2]) -> SceneGroup {
        self.placement(size).lines.clone()
    }

    /// The labels of every side in the frame of a map of `size`, with its top-left corner at 0,
    /// for a map without a layout.
    pub fn marks(&self, size: [f32; 2]) -> SceneGroup {
        let placement = self.placement(size);
        let marks = self
            .label_sides
            .iter()
            .zip(&placement.sides)
            .filter_map(|(&side, labels)| self.text_mark(side, labels, [0.0; 2]))
            .map(Into::into)
            .collect();
        SceneGroup {
            marks,
            ..Default::default()
        }
    }

    fn side_index(&self, side: GuideSide) -> Option<usize> {
        self.label_sides.iter().position(|labeled| *labeled == side)
    }

    /// The size of a map whose `side` is `length` long.
    fn size_at(&self, side: GuideSide, length: f32) -> [f32; 2] {
        if side.is_vertical() {
            [length * self.aspect, length]
        } else {
            [length, length / self.aspect]
        }
    }

    fn try_projector(&self, size: [f32; 2]) -> Result<Projector, AvengerGeoError> {
        // A fit needs a finite, positive range.
        let [width, height] = size.map(|v| f64::from(v).max(MIN_SIZE).min(f64::from(f32::MAX)));
        let rect = [[0.0, 0.0], [width, height]];
        let mut projection = self.projection.clone();
        projection.fit(rect, &self.summary)?;
        if self.frame == MapFrame::Rectangle {
            projection.clip_extent = Some(rect);
        }
        projection.build()
    }

    /// The kept placement at `size`, or a new one.
    fn placement(&self, size: [f32; 2]) -> Arc<Placement> {
        {
            let kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(placement) = kept.iter().find(|placement| placement.size == size) {
                return placement.clone();
            }
        }
        // Place without the lock, which projecting never needs.
        let placement = Arc::new(self.place(size));
        let mut kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
        kept.push_front(placement.clone());
        kept.truncate(KEPT_PLACEMENTS);
        placement
    }

    /// The lines and the labels of every side at `size`. A size without area has no labels.
    fn place(&self, size: [f32; 2]) -> Placement {
        let projector = self.projector(size);
        let lines = self.lines_group(&projector);
        let has_area = size.iter().all(|v| *v > 0.0);
        if self.labels.is_empty() || !has_area {
            return Placement {
                size,
                lines,
                sides: self.label_sides.iter().map(|_| Vec::new()).collect(),
            };
        }
        let ends: Vec<Vec<[f64; 2]>> = self
            .lines
            .iter()
            .map(|line| {
                let mut sink = LineEnds::default();
                projector.project_to_sink(line, &mut sink);
                sink.ends
            })
            .collect();
        let frame = projector.clip_extent();
        let mut boxes = Vec::new();
        let sides = self
            .label_sides
            .iter()
            .map(|&side| self.side_labels(side, &ends, frame, &mut boxes))
            .collect();
        Placement { size, lines, sides }
    }

    fn lines_group(&self, projector: &Projector) -> SceneGroup {
        let path = projector.project_to_path(&self.lines);
        let marks = if path.iter().next().is_some() {
            vec![ScenePathMark {
                path: path.into(),
                fill: ColorOrGradient::Color([0.0; 4]).into(),
                stroke: ColorOrGradient::Color(self.style.stroke).into(),
                stroke_width: Some(self.style.stroke_width),
                stroke_cap: StrokeCap::Round,
                stroke_join: StrokeJoin::Round,
                ..Default::default()
            }
            .into()]
        } else {
            Vec::new()
        };
        SceneGroup {
            interactive: Some(false),
            marks,
            ..Default::default()
        }
    }

    /// The labels of the lines that meet `side`, given each line's `ends`, without those that
    /// would overlap, either each other or a box in `boxes`, which their boxes join.
    fn side_labels(
        &self,
        side: GuideSide,
        ends: &[Vec<[f64; 2]>],
        frame: Option<[[f64; 2]; 2]>,
        boxes: &mut Vec<LabelBox>,
    ) -> Vec<Placed> {
        let kind = if side.is_vertical() {
            GraticuleLineKind::Parallel
        } else {
            GraticuleLineKind::Meridian
        };
        let mut found: Vec<(usize, [f64; 2])> = self
            .labels
            .iter()
            .zip(ends)
            .enumerate()
            .filter(|(_, (label, _))| label.kind == kind)
            .filter_map(|(index, (_, ends))| side_end(ends, side, frame).map(|end| (index, end)))
            .collect();
        let along = |end: &[f64; 2]| if side.is_vertical() { end[1] } else { end[0] };
        found.sort_by(|a, b| along(&a.1).total_cmp(&along(&b.1)));

        let (align, baseline) = alignment(side);
        let pad = self.style.label_padding;
        let placed: Vec<Placed> = found
            .into_iter()
            .map(|(label, [x, y])| {
                let (x, y) = (x as f32, y as f32);
                let position = match side {
                    GuideSide::Left => [x - pad, y],
                    GuideSide::Right => [x + pad, y],
                    GuideSide::Top => [x, y - pad],
                    GuideSide::Bottom => [x, y + pad],
                };
                let bounds = &self.labels[label].bounds;
                let [left, top] = text_origin(bounds, position, align, baseline);
                Placed {
                    label,
                    position,
                    bounds: [[left, top], [left + bounds.width, top + bounds.height]],
                }
            })
            .collect();
        let mut placed = thin_by_parity(placed);
        placed.retain(|label| !boxes.iter().any(|other| overlaps(&label.bounds, other)));
        boxes.extend(placed.iter().map(|label| label.bounds));
        placed
    }

    /// A text mark of `labels` on `side`, moved by `offset`, or `None` without labels.
    fn text_mark(
        &self,
        side: GuideSide,
        labels: &[Placed],
        offset: [f32; 2],
    ) -> Option<SceneTextMark> {
        if labels.is_empty() {
            return None;
        }
        let (align, baseline) = alignment(side);
        let style = &self.style;
        let text: Vec<String> = labels
            .iter()
            .map(|label| self.labels[label.label].text.clone())
            .collect();
        let coordinate = |axis: usize| -> Vec<f32> {
            labels
                .iter()
                .map(|label| label.position[axis] + offset[axis])
                .collect()
        };
        Some(SceneTextMark {
            len: labels.len() as u32,
            clip: false,
            text: text.into(),
            text_syntax: TextSyntaxMode::TypstMarkup,
            x: coordinate(0).into(),
            y: coordinate(1).into(),
            align: align.into(),
            baseline: baseline.into(),
            color: style.label_color.into(),
            font: style.label_font_family.clone().into(),
            font_size: style.label_font_size.into(),
            font_weight: style.label_font_weight.into(),
            ..Default::default()
        })
    }
}

/// Each line's label, formatted and measured.
fn line_labels(
    lines: &[GraticuleLine],
    config: &GraticuleConfig,
    engine: &LabelEngine,
) -> Result<Vec<LineLabel>, AvengerGuidesGeoError> {
    // Each label keeps its own precision: 22.5° doesn't make 30° read 30.0°.
    let degrees: Vec<f64> = lines.iter().map(|line| line.value.abs()).collect();
    let formatted = engine.format(
        FormatValues::Numbers(&degrees),
        &config.label_format,
        TickSpacing::Varying,
    )?;
    let style = &config.style;
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
    lines
        .iter()
        .zip(&formatted)
        .map(|(line, degrees)| {
            let text = label_text(line.kind, line.value, degrees);
            let bounds = engine.bounds(&text_label(
                &text,
                TextSyntaxMode::TypstMarkup,
                options.clone(),
            ))?;
            Ok(LineLabel {
                kind: line.kind,
                text,
                bounds,
            })
        })
        .collect()
}

/// A side's label alignment: away from the map's edge, and centered along it.
fn alignment(side: GuideSide) -> (TextAlign, TextBaseline) {
    match side {
        GuideSide::Left => (TextAlign::Right, TextBaseline::Middle),
        GuideSide::Right => (TextAlign::Left, TextBaseline::Middle),
        GuideSide::Top => (TextAlign::Center, TextBaseline::Bottom),
        GuideSide::Bottom => (TextAlign::Center, TextBaseline::Top),
    }
}

/// Where `side`'s edge of a map of `size` starts, in the map's frame.
fn edge_origin(side: GuideSide, size: [f32; 2]) -> [f32; 2] {
    match side {
        GuideSide::Left | GuideSide::Top => [0.0, 0.0],
        GuideSide::Right => [size[0], 0.0],
        GuideSide::Bottom => [0.0, size[1]],
    }
}

/// Of the ends of a line's pieces, the one nearest `side`: on that edge of
/// `frame` when there is one.
fn side_end(ends: &[[f64; 2]], side: GuideSide, frame: Option<[[f64; 2]; 2]>) -> Option<[f64; 2]> {
    let (axis, nearest_is_max) = match side {
        GuideSide::Left => (0, false),
        GuideSide::Right => (0, true),
        GuideSide::Top => (1, false),
        GuideSide::Bottom => (1, true),
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

/// Vega's "parity" strategy: halve the labels, in their order along the side, until no two
/// overlap. Line ends on a curved frame stand at different depths, so a label can overlap one
/// past its neighbor.
fn thin_by_parity(mut placed: Vec<Placed>) -> Vec<Placed> {
    let overlapping = |placed: &[Placed]| {
        placed.iter().enumerate().any(|(i, a)| {
            placed[i + 1..]
                .iter()
                .any(|b| overlaps(&a.bounds, &b.bounds))
        })
    };
    while overlapping(&placed) {
        placed = placed.into_iter().step_by(2).collect();
    }
    placed
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_geo::projections::EqualEarth;

    /// Labels down a side, where the first and third overlap past the second, which stands
    /// apart, thin until no two overlap.
    #[test]
    fn parity_thins_labels_that_overlap_past_a_neighbor() {
        let label = |label: usize, x: f32, y: f32| Placed {
            label,
            position: [x, y],
            bounds: [[x - 20.0, y - 6.0], [x, y + 6.0]],
        };
        let placed = vec![
            label(0, 0.0, 0.0),
            label(1, -100.0, 4.0),
            label(2, 0.0, 8.0),
        ];
        let kept: Vec<usize> = thin_by_parity(placed)
            .iter()
            .map(|label| label.label)
            .collect();
        assert_eq!(kept, [0]);
    }

    #[test]
    fn placements_are_kept_by_size_up_to_the_limit() {
        let projection = Projection::new(EqualEarth);
        let summary = projection.summarize_sphere().unwrap();
        let engine = avenger_typst_label::bundled_label_engine().with_number_formatting(Arc::new(
            avenger_format_number_d3::D3NumberFormatProvider::new(),
        ));
        let graticule =
            Graticule::new(&projection, &summary, &GraticuleConfig::default(), &engine).unwrap();
        let first = graticule.placement([300.0, 150.0]);
        assert!(Arc::ptr_eq(&first, &graticule.placement([300.0, 150.0])));
        for width in 0..2 * KEPT_PLACEMENTS {
            graticule.extent(GuideSide::Bottom, 100.0 + width as f32);
        }
        assert_eq!(graticule.kept.lock().unwrap().len(), KEPT_PLACEMENTS);
        assert!(!Arc::ptr_eq(&first, &graticule.placement([300.0, 150.0])));
    }
}
