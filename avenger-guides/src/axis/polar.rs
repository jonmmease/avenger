//! Polar axes, prepared once and drawn at any size: an axis of the angle of a polar plot around
//! its rim, with spokes as its grid lines, and an axis of the radius along a spoke, or along the
//! start edge of part of a circle, with rings as its grid lines.
//!
//! A polar plot draws its axes inside its own box, so a polar axis has no parts for a layout.
//! A chart asks how far an axis reaches past the box of its circle and shrinks the circle until
//! the axis fits. Angles are in degrees clockwise from the top, as a clock reads, and positions
//! are in the plot's frame, with y down.

use std::sync::Arc;

use avenger_color::ColorOrGradient;
use avenger_common::{
    types::{StrokeCap, TextAlign, TextBaseline},
    value::{ScalarOrArray, ScalarOrArrayValue},
};
use avenger_scales::scales::{fitting::FittingScale, ConfiguredScale};
use avenger_scenegraph::marks::{
    group::SceneGroup,
    mark::SceneMark,
    path::ScenePathMark,
    rule::SceneRuleMark,
    text::{SceneTextMark, TextHalo, TextPaths},
};
use avenger_typst_label::{reads_reversed, LabelEngine, PathOffset, PathOverflow};
use lyon_path::{
    geom::{
        euclid::{Point2D, Vector2D},
        Angle, Arc as ArcSegment,
    },
    Path,
};

use super::{
    marks::{boxes_overlap, label_font, stride_shown, Labels, LABEL_GAP, TEXT_MARGIN},
    opts::{AxisConfig, AxisStyle},
    prepared::{Axis, PositionedLabels},
};
use crate::{axis::LabelThinning, error::AvengerGuidesError, GuideSide};

/// How far apart, in degrees, two angles may lie and still be one angle, as the end of a whole
/// circle is its start.
const SAME_ANGLE: f32 = 1e-3;

/// Which coordinate of a polar plot an axis shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolarAxisKind {
    /// The angle, around the rim, with spokes as grid lines.
    Theta,
    /// The radius, along a spoke, with rings as grid lines.
    Radius,
}

/// How the labels of a polar axis turn. A radial or tangential label turns half a turn where it
/// would read upside down.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PolarLabelOrient {
    /// Upright.
    Horizontal,
    /// Along the line from the center through the label.
    Radial,
    /// Across that line. Theta labels curve along a circle outside the rim, standing outward on
    /// both halves, unless one of them would sweep more than a quarter turn around that circle,
    /// and then they all stay straight. A radius label stays straight beside its spoke, where
    /// curving along its ring would slant each label differently.
    Tangential,
    /// This angle, in degrees clockwise.
    Angle(f32),
}

/// The shape of the rings of a radius axis and of the rim of a theta axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolarShape {
    Circle,
    /// Polygons whose corners are the frame's corners, the ticks of the theta axis, as the web
    /// and the rim of a radar chart.
    Polygon,
}

/// What a polar axis shows and how it looks, at any size.
///
/// A theta axis draws its ticks and labels outside the rim. A radius axis draws them along the
/// spoke at `angle` over a whole circle, on the clockwise side of the spoke, and along the start
/// edge over part of a circle, outside it, as the edge below a half circle from -90 to 90
/// degrees.
#[derive(Debug, Clone)]
pub struct PolarAxisConfig {
    pub kind: PolarAxisKind,
    /// Where the plot's range of angles starts, in degrees clockwise from the top.
    pub start_angle: f32,
    /// Where the plot's range of angles ends, past `start_angle` and at most a full turn later.
    pub end_angle: f32,
    /// The spoke of a radius axis over a whole circle, in degrees clockwise from the top, or
    /// `None` for the start angle. A radius axis over part of a circle runs along its start
    /// edge, and a theta axis takes no angle.
    pub angle: Option<f32>,
    pub shape: PolarShape,
    pub label_orient: PolarLabelOrient,
    /// The space between the end of a tick and its label, in pixels.
    pub label_padding: f32,
    /// A halo behind each label, so that a label reads over the grid and the data, or `None`
    /// for none. An array of colors gives each tick label its own, in the order of the ticks.
    pub label_halo: Option<TextHalo>,
    /// The pattern of numeric and temporal ticks, as for `AxisConfig::tick_format`.
    pub tick_format: String,
    /// Typst markup for each tick label, as for `AxisConfig::tick_label`.
    pub tick_label: Option<String>,
    /// The ticks, the labels, the colors, and the grid. `label_angle` and the title's settings
    /// do not apply, because `label_orient` turns the labels and a polar axis has no title.
    pub style: AxisStyle,
}

impl Default for PolarAxisConfig {
    fn default() -> Self {
        Self {
            kind: PolarAxisKind::Theta,
            start_angle: 0.0,
            end_angle: 360.0,
            angle: None,
            shape: PolarShape::Circle,
            label_orient: PolarLabelOrient::Horizontal,
            label_padding: TEXT_MARGIN,
            label_halo: None,
            tick_format: String::new(),
            tick_label: None,
            style: AxisStyle::default(),
        }
    }
}

impl PolarAxisConfig {
    /// Whether the range of angles is a whole circle.
    fn full(&self) -> bool {
        self.end_angle - self.start_angle >= 360.0 - SAME_ANGLE
    }

    /// The spoke that a radius axis runs along.
    fn radius_angle(&self) -> f32 {
        if self.full() {
            self.angle.unwrap_or(self.start_angle)
        } else {
            self.start_angle
        }
    }
}

/// Where a polar plot draws at one size: the center of its circle, its inner and outer radius
/// in pixels, and the corners of its polygon rings and rim.
#[derive(Debug, Clone, PartialEq)]
pub struct PolarFrame {
    pub center: [f32; 2],
    pub inner_radius: f32,
    pub outer_radius: f32,
    /// The angles of the corners of polygon rings and rims, the ticks of the theta axis from
    /// [`PolarAxis::tick_angles`], or empty for an axis whose shape is a circle.
    pub corners: Vec<f32>,
}

impl PolarFrame {
    /// The point at `angle` and `radius`.
    pub fn point(&self, angle: f32, radius: f32) -> [f32; 2] {
        let [x, y] = outward(angle);
        [self.center[0] + radius * x, self.center[1] + radius * y]
    }

    /// The box around the arc at the outer radius between `start_angle` and `end_angle`, and
    /// around the inner arc, or the center without a hole, as `[[left, top], [right, bottom]]`.
    /// A chart sizes a circle so that this box, with the reach of its axes past it, fits its
    /// plot.
    pub fn sector_box(&self, start_angle: f32, end_angle: f32) -> [[f32; 2]; 2] {
        let mut angles = vec![start_angle, end_angle];
        // Each quarter turn from the top between the angles is an extreme of the arc.
        let mut quarter = (start_angle / 90.0).ceil() * 90.0;
        while quarter <= end_angle {
            angles.push(quarter);
            quarter += 90.0;
        }
        let mut points: Vec<[f32; 2]> = angles
            .iter()
            .map(|angle| self.point(*angle, self.outer_radius))
            .collect();
        if self.inner_radius > 0.0 {
            points.extend(
                angles
                    .iter()
                    .map(|angle| self.point(*angle, self.inner_radius)),
            );
        } else {
            points.push(self.center);
        }
        let mut lower = [f32::INFINITY; 2];
        let mut upper = [f32::NEG_INFINITY; 2];
        for [x, y] in points {
            lower = [lower[0].min(x), lower[1].min(y)];
            upper = [upper[0].max(x), upper[1].max(y)];
        }
        [lower, upper]
    }
}

/// A box of text, turned about its center, that a polar plot places off an anchor: a polar axis
/// places each label out from its tick at the rim or beside the spoke, and a chart places a label
/// outside a pie out from its point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PolarLabel {
    pub center: [f32; 2],
    /// The width and the height.
    pub size: [f32; 2],
    /// How far the box turns about its center, in degrees clockwise, as a text mark's `angle`
    /// turns its text. It is not the angle of the box about the circle's center.
    pub rotation: f32,
}

impl PolarLabel {
    /// The box of `size`, turned by `rotation` degrees clockwise, that stands off `anchor` along the
    /// unit vector `away`: its center lies on the line from the anchor along `away`, and its edge
    /// nearest the anchor meets the anchor, so the box sits beside the anchor across a side and
    /// beyond it toward a corner. Along a zero vector, the box centers on the anchor.
    pub fn standing_off(anchor: [f32; 2], away: [f32; 2], size: [f32; 2], rotation: f32) -> Self {
        // Half the extent of the turned box along `away`.
        let (sin, cos) = rotation.to_radians().sin_cos();
        let distance = size[0] / 2.0 * (away[0] * cos + away[1] * sin).abs()
            + size[1] / 2.0 * (-away[0] * sin + away[1] * cos).abs();
        Self {
            center: offset(anchor, away, distance),
            size,
            rotation,
        }
    }

    /// The corners of the box, grown by `padding` on each side.
    pub fn corners(&self, padding: f32) -> [[f32; 2]; 4] {
        let (sin, cos) = self.rotation.to_radians().sin_cos();
        let [half_width, half_height] = self.size.map(|side| side / 2.0 + padding);
        [
            [-half_width, -half_height],
            [half_width, -half_height],
            [half_width, half_height],
            [-half_width, half_height],
        ]
        .map(|[x, y]| {
            [
                self.center[0] + x * cos - y * sin,
                self.center[1] + x * sin + y * cos,
            ]
        })
    }

    /// The bounds of the turned box, as `[[left, top], [right, bottom]]`.
    pub fn bounds(&self) -> [[f32; 2]; 2] {
        self.corners(0.0).into_iter().fold(
            [[f32::INFINITY; 2], [f32::NEG_INFINITY; 2]],
            |[lower, upper], [x, y]| {
                [
                    [lower[0].min(x), lower[1].min(y)],
                    [upper[0].max(x), upper[1].max(y)],
                ]
            },
        )
    }
}

/// The unit vector from the center toward `angle`.
fn outward(angle: f32) -> [f32; 2] {
    let (sin, cos) = angle.to_radians().sin_cos();
    [sin, -cos]
}

/// The unit vector a quarter turn clockwise from the direction of `angle`.
fn clockwise(angle: f32) -> [f32; 2] {
    let (sin, cos) = angle.to_radians().sin_cos();
    [cos, sin]
}

fn offset(point: [f32; 2], direction: [f32; 2], distance: f32) -> [f32; 2] {
    [
        point[0] + direction[0] * distance,
        point[1] + direction[1] * distance,
    ]
}

/// Whether two angles name one direction.
fn same_angle(a: f32, b: f32) -> bool {
    let difference = (a - b).rem_euclid(360.0);
    difference < SAME_ANGLE || 360.0 - difference < SAME_ANGLE
}

/// The scale kinds that a polar axis places ticks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScaleKind {
    Continuous,
    Band,
    Point,
}

/// A polar axis prepared from a scale, a configuration and a text engine, without a size.
///
/// Every method takes the frame of the plot at a size and fits the scale there: a theta axis at
/// the length of its arc at the outer radius, and a radius axis at the length of its spoke. Its
/// ticks, its labels and the data's positions therefore agree at every size. The constructors
/// check the angles, the formats and the templates, so the methods do not fail.
#[derive(Debug)]
pub struct PolarAxis {
    /// The axis that places and labels the ticks along a line of the length of the arc or the
    /// spoke.
    axis: Axis,
    config: PolarAxisConfig,
    banded: bool,
}

/// The labels of an axis placed at one size: which label each is, and where it lies. An axis's
/// labels all stay straight, or all curve along their circle, so that a rim never mixes the two.
#[derive(Debug, Clone)]
enum PlacedLabels {
    Straight(Vec<(usize, PolarLabel)>),
    Curved(Vec<(usize, Band)>),
}

impl PlacedLabels {
    fn is_empty(&self) -> bool {
        match self {
            PlacedLabels::Straight(labels) => labels.is_empty(),
            PlacedLabels::Curved(bands) => bands.is_empty(),
        }
    }

    /// Each label's index and where it lies.
    fn shapes(&self) -> Vec<(usize, LabelShape)> {
        match self {
            PlacedLabels::Straight(labels) => labels
                .iter()
                .map(|(index, label)| (*index, LabelShape::Straight(*label)))
                .collect(),
            PlacedLabels::Curved(bands) => bands
                .iter()
                .map(|(index, band)| (*index, LabelShape::Curved(*band)))
                .collect(),
        }
    }

    /// The labels at the positions `shown` among these.
    fn select(&self, shown: &[usize]) -> Self {
        match self {
            PlacedLabels::Straight(labels) => {
                PlacedLabels::Straight(shown.iter().map(|at| labels[*at]).collect())
            }
            PlacedLabels::Curved(bands) => {
                PlacedLabels::Curved(shown.iter().map(|at| bands[*at]).collect())
            }
        }
    }

    /// Keeps the labels whose shapes `keep` accepts.
    fn retain(&mut self, keep: impl Fn(&LabelShape) -> bool) {
        match self {
            PlacedLabels::Straight(labels) => {
                labels.retain(|(_, label)| keep(&LabelShape::Straight(*label)))
            }
            PlacedLabels::Curved(bands) => {
                bands.retain(|(_, band)| keep(&LabelShape::Curved(*band)))
            }
        }
    }
}

/// Where a placed label lies: a turned box of straight text, or the band of a label that curves
/// along its circle. The labels of two axes can differ.
#[derive(Debug, Clone, Copy)]
enum LabelShape {
    Straight(PolarLabel),
    Curved(Band),
}

impl LabelShape {
    /// Convex pieces that cover the label, grown by `padding` on each side.
    fn pieces(&self, padding: f32) -> Vec<[[f32; 2]; 4]> {
        match self {
            LabelShape::Straight(label) => vec![label.corners(padding)],
            LabelShape::Curved(band) => band.pieces(padding),
        }
    }

    /// Points whose bounds are the label's bounds, grown by `padding` on each side.
    fn extremes(&self, padding: f32) -> Vec<[f32; 2]> {
        match self {
            LabelShape::Straight(label) => label.corners(padding).to_vec(),
            LabelShape::Curved(band) => band.extremes(padding),
        }
    }
}

/// The most, in degrees, that a curved label's text sweeps around its circle. A longer label
/// stays straight.
const MOST_SWEEP: f32 = 90.0;

/// The widest piece of a band that overlap tests compare, in degrees.
const PIECE_SWEEP: f32 = 10.0;

/// The part of a ring that a label curved along its circle covers: between two radii about the
/// frame's center and two angles about its middle. The label's text follows the arc at the inner
/// radius with its near edge there.
#[derive(Debug, Clone, Copy)]
struct Band {
    center: [f32; 2],
    inner: f32,
    outer: f32,
    /// The angle of the text's middle, in degrees clockwise from the top.
    middle: f32,
    /// Half the angle that the text sweeps, in degrees.
    half: f32,
}

impl Band {
    /// The band of text `size` wide and tall whose middle lies at `middle`, between `inner` and
    /// `inner` plus its height, or `None` when `inner` isn't positive or the text would sweep
    /// more than [`MOST_SWEEP`]. Its sweep is the text's width at the inner radius, which covers
    /// the text.
    fn new(center: [f32; 2], inner: f32, middle: f32, size: [f32; 2]) -> Option<Self> {
        let sweep = (size[0] / inner).to_degrees();
        (inner > 0.0 && sweep <= MOST_SWEEP).then_some(Self {
            center,
            inner,
            outer: inner + size[1],
            middle,
            half: sweep / 2.0,
        })
    }

    /// The part of the text on the arc, so that its near edge lies there on both halves. The arc
    /// runs left to right at its middle, so it reads upside down where the middle's angle turns
    /// it over, and upright reverses it there, with the text's top on the arc.
    fn baseline(&self) -> TextBaseline {
        match reads_reversed([1.0, 0.0], self.middle) {
            true => TextBaseline::Top,
            false => TextBaseline::Bottom,
        }
    }

    fn point(&self, angle: f32, radius: f32) -> [f32; 2] {
        offset(self.center, outward(angle), radius)
    }

    /// The band grown by `padding` on each side, in pieces of at most [`PIECE_SWEEP`]: each the
    /// four corners of a piece's inner and outer arcs, with the outer corners far enough out that
    /// the chord between them covers the arc.
    fn pieces(&self, padding: f32) -> Vec<[[f32; 2]; 4]> {
        let inner = (self.inner - padding).max(0.0);
        let half = self.half + (padding / self.inner.max(1.0)).to_degrees();
        let count = (2.0 * half / PIECE_SWEEP).ceil().max(1.0) as usize;
        let step = 2.0 * half / count as f32;
        (0..count)
            .map(|piece| {
                let from = self.middle - half + step * piece as f32;
                let to = from + step;
                let outer = (self.outer + padding) / (step / 2.0).to_radians().cos();
                [
                    self.point(from, inner),
                    self.point(from, outer),
                    self.point(to, outer),
                    self.point(to, inner),
                ]
            })
            .collect()
    }

    /// The band's ends, and the extremes of its outer arc at each quarter turn between them,
    /// grown by `padding`.
    fn extremes(&self, padding: f32) -> Vec<[f32; 2]> {
        let [from, to] = [self.middle - self.half, self.middle + self.half];
        let [inner, outer] = [(self.inner - padding).max(0.0), self.outer + padding];
        let mut points = vec![
            self.point(from, inner),
            self.point(to, inner),
            self.point(from, outer),
            self.point(to, outer),
        ];
        let mut quarter = (from / 90.0).ceil() * 90.0;
        while quarter <= to {
            points.push(self.point(quarter, outer));
            quarter += 90.0;
        }
        points
    }

    /// Whether two bands about one center come closer than `gap`: across, between their radii,
    /// and around, between their angles.
    fn near(&self, other: &Band, gap: f32) -> bool {
        let across = self.inner < other.outer + gap && other.inner < self.outer + gap;
        let apart = ((self.middle - other.middle + 180.0).rem_euclid(360.0) - 180.0).abs();
        let gap_angle = (gap / self.inner.min(other.inner).max(1.0)).to_degrees();
        across && apart < self.half + other.half + gap_angle
    }

    /// The arc that the text follows: about the origin, centered at the top, as wide as the
    /// band, which the label's angle turns to its middle.
    fn arc(&self) -> Path {
        arc_path([0.0, 0.0], self.inner, -self.half, self.half)
    }
}

/// The ticks of an axis at one size, as angles or radii, and its labels, without those that
/// thinning drops.
struct Placed {
    ticks: Vec<f32>,
    labels: Option<Arc<Labels>>,
    placed: PlacedLabels,
}

impl PolarAxis {
    /// An axis for a scale with a numeric or temporal domain, such as a linear, log or time
    /// scale.
    pub fn continuous(
        scale: impl Into<FittingScale>,
        config: &PolarAxisConfig,
        engine: &LabelEngine,
    ) -> Result<Self, AvengerGuidesError> {
        Self::new(scale.into(), ScaleKind::Continuous, config, engine)
    }

    /// An axis for a band scale, with a tick and a label at the center of each band. Over a
    /// whole circle, the first band centers at the start angle.
    pub fn band(
        scale: impl Into<FittingScale>,
        config: &PolarAxisConfig,
        engine: &LabelEngine,
    ) -> Result<Self, AvengerGuidesError> {
        Self::new(scale.into(), ScaleKind::Band, config, engine)
    }

    /// An axis for a point scale, with a tick and a label at each point. Over a whole circle,
    /// the first point lies at the start angle.
    pub fn point(
        scale: impl Into<FittingScale>,
        config: &PolarAxisConfig,
        engine: &LabelEngine,
    ) -> Result<Self, AvengerGuidesError> {
        Self::new(scale.into(), ScaleKind::Point, config, engine)
    }

    fn new(
        scale: FittingScale,
        kind: ScaleKind,
        config: &PolarAxisConfig,
        engine: &LabelEngine,
    ) -> Result<Self, AvengerGuidesError> {
        let (start, end) = (config.start_angle, config.end_angle);
        if !(start.is_finite()
            && end.is_finite()
            && end > start
            && end - start <= 360.0 + SAME_ANGLE)
        {
            return Err(AvengerGuidesError::PolarAngles { start, end });
        }
        // The line axis fits its tick count to the spacing of labels across its direction: a
        // theta axis runs around the circle, and a radius axis along its spoke.
        let side = match config.kind {
            PolarAxisKind::Theta => GuideSide::Bottom,
            PolarAxisKind::Radius => {
                let [x, y] = outward(config.radius_angle());
                if y.abs() >= x.abs() {
                    GuideSide::Left
                } else {
                    GuideSide::Bottom
                }
            }
        };
        let line_config = AxisConfig {
            side,
            tick_format: config.tick_format.clone(),
            tick_label: config.tick_label.clone(),
            style: AxisStyle {
                label_angle: 0.0,
                title_visible: false,
                ..config.style.clone()
            },
        };
        let axis = match kind {
            ScaleKind::Continuous => Axis::continuous(scale, "", &line_config, engine)?,
            ScaleKind::Band => Axis::band(scale, "", &line_config, engine)?,
            ScaleKind::Point => Axis::point(scale, "", &line_config, engine)?,
        };
        Ok(Self {
            axis,
            config: config.clone(),
            banded: kind != ScaleKind::Continuous,
        })
    }

    /// The length of the line that the axis places its ticks along: the arc at the outer
    /// radius, or the spoke.
    fn length(&self, frame: &PolarFrame) -> f32 {
        let length = match self.config.kind {
            PolarAxisKind::Theta => {
                frame.outer_radius * (self.config.end_angle - self.config.start_angle).to_radians()
            }
            PolarAxisKind::Radius => frame.outer_radius - frame.inner_radius,
        };
        length.max(1e-3)
    }

    /// The angle or the radius at `position` along the line of `length`, where a theta axis
    /// turns back by `turn` along the line, as [`Self::turn`] gives it.
    fn coordinate(&self, frame: &PolarFrame, length: f32, turn: f32, position: f32) -> f32 {
        let config = &self.config;
        match config.kind {
            PolarAxisKind::Theta => {
                let sweep = config.end_angle - config.start_angle;
                config.start_angle + (position - turn) / length * sweep
            }
            PolarAxisKind::Radius => frame.inner_radius + position,
        }
    }

    /// How far along its line a theta axis turns back so that, over a whole circle, the first
    /// tick of a band or a point scale lies at the start angle: the position of that tick among
    /// `ticks`, and 0 otherwise.
    fn turn(&self, ticks: &[f32]) -> f32 {
        match ticks.first() {
            Some(first) if self.banded && self.closed() => *first,
            _ => 0.0,
        }
    }

    /// Whether the axis is a continuous theta around a whole circle, whose end is its start, so
    /// its last tick and label repeat its first.
    fn ends_at_start(&self) -> bool {
        self.closed() && !self.banded
    }

    /// The scale with its range set for the frame, in degrees clockwise from the top for a
    /// theta axis and in pixels from the center for a radius axis, and its domain fitted there,
    /// for the chart's data.
    pub fn scale(&self, frame: &PolarFrame) -> ConfiguredScale {
        let length = self.length(frame);
        let scale = self.axis.scale(length);
        let Ok((start, end)) = scale.numeric_interval_range() else {
            return scale;
        };
        // Only the bands of a theta axis around a whole circle turn, so only they need ticks.
        let turn = match self.banded && self.closed() {
            true => self.turn(&self.line_ticks(frame, length).0),
            false => 0.0,
        };
        let coordinate =
            |position: f64| f64::from(self.coordinate(frame, length, turn, position as f32));
        scale.with_range_interval((coordinate(start), coordinate(end)))
    }

    /// The angles of the ticks of a theta axis at the frame, for the corners of polygon rings
    /// and rims, or none for a radius axis.
    pub fn tick_angles(&self, frame: &PolarFrame) -> Vec<f32> {
        match self.config.kind {
            PolarAxisKind::Theta => self.placed(frame).ticks,
            PolarAxisKind::Radius => Vec::new(),
        }
    }

    /// The ticks and the labels at the frame.
    fn placed(&self, frame: &PolarFrame) -> Placed {
        let config = &self.config;
        let length = self.length(frame);
        let closed = self.closed();
        let (positions, row) = self.line_ticks(frame, length);
        let turn = self.turn(&positions);
        let mut ticks: Vec<f32> = positions
            .iter()
            .map(|position| self.coordinate(frame, length, turn, *position))
            .collect();
        if self.ends_at_start() && ticks.len() >= 2 && same_angle(ticks[0], ticks[ticks.len() - 1])
        {
            ticks.pop();
        }
        let Some((labels, label_positions)) = row else {
            return Placed {
                ticks,
                labels: None,
                placed: PlacedLabels::Straight(Vec::new()),
            };
        };
        let mut placed = self.place(frame, length, turn, &labels, &label_positions);
        if config.style.label_thinning == LabelThinning::Stride {
            let shapes = placed.shapes();
            let shown = stride_shown(
                shapes.len(),
                |_| true,
                |a, b| !overlaps(&shapes[a].1, &shapes[b].1, LABEL_GAP),
                closed,
            );
            placed = placed.select(&shown);
        }
        Placed {
            ticks,
            labels: Some(labels),
            placed,
        }
    }

    /// Whether the axis's labels run around a whole circle, where the last neighbors the first.
    fn closed(&self) -> bool {
        self.config.kind == PolarAxisKind::Theta && self.config.full()
    }

    /// The ticks and the labels with their positions along the line of `length`. A continuous
    /// axis takes coarser ticks where its labels overlap around the rim or along the spoke at the
    /// frame, and does not turn.
    fn line_ticks(&self, frame: &PolarFrame, length: f32) -> (Vec<f32>, Option<PositionedLabels>) {
        let overlap = |labels: &Labels, positions: &[f32]| {
            crowded(
                &self.place(frame, length, 0.0, labels, positions),
                self.closed(),
            )
        };
        self.axis.ticks_and_labels(length, &overlap)
    }

    /// The labels at `positions` along the line of `length`, placed at the frame: those that lay
    /// out, without the label at the end of a whole circle of a continuous theta, which is its
    /// start. Tangential theta labels curve along their circle together, or, when one of them
    /// would sweep too far around it, all stay straight.
    fn place(
        &self,
        frame: &PolarFrame,
        length: f32,
        turn: f32,
        labels: &Labels,
        positions: &[f32],
    ) -> PlacedLabels {
        let ends_at_start = self.ends_at_start();
        // Each label that lays out: its index, its angle or radius, and its width and height.
        let mut spots: Vec<(usize, f32, [f32; 2])> = Vec::new();
        for (index, position) in positions.iter().enumerate() {
            let Some(bounds) = labels.bounds.get(index).copied().flatten() else {
                continue;
            };
            let at = self.coordinate(frame, length, turn, *position);
            if ends_at_start && spots.iter().any(|(_, other, _)| same_angle(*other, at)) {
                continue;
            }
            spots.push((index, at, [bounds.width, bounds.height]));
        }
        let config = &self.config;
        let curves = config.label_orient == PolarLabelOrient::Tangential
            && config.kind == PolarAxisKind::Theta;
        let bands: Option<Vec<Band>> = curves
            .then(|| {
                spots
                    .iter()
                    .map(|(_, at, size)| self.curved(frame, *at, *size))
                    .collect()
            })
            .flatten();
        match bands {
            Some(bands) => {
                PlacedLabels::Curved(spots.iter().map(|(index, ..)| *index).zip(bands).collect())
            }
            None => PlacedLabels::Straight(
                spots
                    .iter()
                    .map(|(index, at, size)| (*index, self.straight(frame, *at, *size)))
                    .collect(),
            ),
        }
    }

    /// The band of a tangential theta label at the angle `at`, of width and height `size`, along
    /// a circle past the end of its tick by the label padding, with its near edge on that circle
    /// on both halves, or `None` when it would sweep more than a quarter turn.
    fn curved(&self, frame: &PolarFrame, at: f32, size: [f32; 2]) -> Option<Band> {
        let radius = frame.outer_radius + self.label_reach();
        Band::new(frame.center, radius, at, size)
    }

    /// How far a label stands past the axis line: the tick and the label padding.
    fn label_reach(&self) -> f32 {
        self.config.style.tick_length.max(0.0) + self.config.label_padding
    }

    /// The straight label at `at`, of width and height `size`: its angle from its orientation,
    /// standing off the end of its tick by the label padding, out from the rim or beside the
    /// spoke.
    fn straight(&self, frame: &PolarFrame, at: f32, size: [f32; 2]) -> PolarLabel {
        let config = &self.config;
        let reach = self.label_reach();
        // The direction that the label stands from its anchor, and the direction that a radial
        // label reads.
        let (anchor, away, along) = match config.kind {
            PolarAxisKind::Theta => {
                let away = outward(at);
                (frame.point(at, frame.outer_radius + reach), away, away)
            }
            PolarAxisKind::Radius => {
                let spoke = config.radius_angle();
                let away = self.radius_side();
                (
                    offset(frame.point(spoke, at), away, reach),
                    away,
                    outward(spoke),
                )
            }
        };
        let rotation = match config.label_orient {
            PolarLabelOrient::Horizontal => 0.0,
            PolarLabelOrient::Angle(angle) => angle,
            PolarLabelOrient::Radial => upright(along[1].atan2(along[0]).to_degrees()),
            PolarLabelOrient::Tangential => upright(along[1].atan2(along[0]).to_degrees() + 90.0),
        };
        PolarLabel::standing_off(anchor, away, size, rotation)
    }

    /// The side of its spoke that a radius axis draws its ticks and labels on: the clockwise
    /// side over a whole circle, and outside the start edge over part of one.
    fn radius_side(&self) -> [f32; 2] {
        let [x, y] = clockwise(self.config.radius_angle());
        if self.config.full() {
            [x, y]
        } else {
            [-x, -y]
        }
    }

    /// The ticks, their line, and the labels at the frame, in the plot's frame.
    pub fn marks(&self, frame: &PolarFrame) -> SceneGroup {
        self.marks_of(frame, self.placed(frame))
    }

    /// The marks at the frame, without the labels that overlap a label of one of `others`, as
    /// a radius axis hides the label at the rim that would cover the label of the theta axis
    /// at its spoke. Labels keep the width of the wider halo apart, so that neither halo covers
    /// the other label.
    pub fn marks_avoiding(&self, frame: &PolarFrame, others: &[&PolarAxis]) -> SceneGroup {
        let mut placed = self.placed(frame);
        let avoided: Vec<(LabelShape, f32)> = others
            .iter()
            .flat_map(|other| {
                let halo = other.halo_width();
                other
                    .placed(frame)
                    .placed
                    .shapes()
                    .into_iter()
                    .map(move |(_, shape)| (shape, halo))
            })
            .collect();
        let halo = self.halo_width();
        placed.placed.retain(|shape| {
            avoided
                .iter()
                .all(|(other, width)| !overlaps(shape, other, LABEL_GAP + halo.max(*width)))
        });
        self.marks_of(frame, placed)
    }

    /// The width of the labels' halo, or 0 for none.
    fn halo_width(&self) -> f32 {
        self.config
            .label_halo
            .as_ref()
            .map_or(0.0, |halo| halo.width.max(0.0))
    }

    fn marks_of(&self, frame: &PolarFrame, placed: Placed) -> SceneGroup {
        let config = &self.config;
        let style = &config.style;
        let mut marks: Vec<SceneMark> = Vec::new();

        // The line of the axis: the rim of a theta axis, and the spoke of a radius axis.
        match config.kind {
            PolarAxisKind::Theta => {
                let rim = self.ring(frame, frame.outer_radius);
                marks.push(stroke_paths("domain", vec![rim], style.domain_color, 1.0).into());
            }
            PolarAxisKind::Radius => {
                let spoke = config.radius_angle();
                let start = frame.point(spoke, frame.inner_radius);
                let end = frame.point(spoke, frame.outer_radius);
                marks.push(rules("domain", &[(start, end)], style.domain_color, 1.0).into());
            }
        }

        if style.tick_length > 0.0 && !placed.ticks.is_empty() {
            let segments = self.tick_segments(frame, &placed.ticks);
            marks.push(rules("ticks", &segments, style.tick_color, 1.0).into());
        }

        if let (Some(labels), false) = (&placed.labels, placed.placed.is_empty()) {
            let mut text = label_text(labels, &placed.placed, style);
            text.halo = config
                .label_halo
                .as_ref()
                .map(|halo| placed_halo(halo, &placed.placed));
            marks.push(text.into());
        }
        SceneGroup {
            marks,
            ..Default::default()
        }
    }

    /// The grid lines at the frame, in the plot's frame, behind its data: a spoke at each angle
    /// of a theta axis from the inner radius to the outer radius, and a ring at each radius of
    /// a radius axis.
    pub fn grid(&self, frame: &PolarFrame) -> SceneGroup {
        let style = &self.config.style;
        let ticks = self.placed(frame).ticks;
        let mark: SceneMark = match self.config.kind {
            PolarAxisKind::Theta => {
                let spokes: Vec<([f32; 2], [f32; 2])> = ticks
                    .iter()
                    .map(|angle| {
                        (
                            frame.point(*angle, frame.inner_radius),
                            frame.point(*angle, frame.outer_radius),
                        )
                    })
                    .collect();
                rules("grid", &spokes, style.grid_color, style.grid_width).into()
            }
            PolarAxisKind::Radius => {
                let rings = ticks
                    .iter()
                    .map(|radius| self.ring(frame, *radius))
                    .collect();
                stroke_paths("grid", rings, style.grid_color, style.grid_width).into()
            }
        };
        SceneGroup {
            interactive: Some(false),
            marks: vec![mark],
            ..Default::default()
        }
    }

    /// How far the axis reaches past the box of its part of the circle at the frame, toward
    /// the left, the right, the top, and the bottom, in pixels: the box around the outer arc
    /// between the plot's angles and the inner arc, or the center, that a chart sizes its
    /// circle to fit.
    pub fn reach(&self, frame: &PolarFrame) -> [f32; 4] {
        let config = &self.config;
        let [[left, top], [right, bottom]] = frame.sector_box(config.start_angle, config.end_angle);
        let mut reach = [0.0_f32; 4];
        let mut add = |[x, y]: [f32; 2]| {
            reach[0] = reach[0].max(left - x);
            reach[1] = reach[1].max(x - right);
            reach[2] = reach[2].max(top - y);
            reach[3] = reach[3].max(y - bottom);
        };
        let placed = self.placed(frame);
        for (_, end) in self.tick_segments(frame, &placed.ticks) {
            add(end);
        }
        let padding = self.halo_width();
        for (_, shape) in placed.placed.shapes() {
            for point in shape.extremes(padding) {
                add(point);
            }
        }
        reach
    }

    /// The tick at each of `ticks`, from where it meets the axis to its end: outward from the
    /// rim at each angle of a theta axis, and across the spoke at each radius of a radius axis.
    fn tick_segments(&self, frame: &PolarFrame, ticks: &[f32]) -> Vec<([f32; 2], [f32; 2])> {
        let length = self.config.style.tick_length.max(0.0);
        match self.config.kind {
            PolarAxisKind::Theta => ticks
                .iter()
                .map(|angle| {
                    (
                        frame.point(*angle, frame.outer_radius),
                        frame.point(*angle, frame.outer_radius + length),
                    )
                })
                .collect(),
            PolarAxisKind::Radius => {
                let spoke = self.config.radius_angle();
                let away = self.radius_side();
                ticks
                    .iter()
                    .map(|radius| {
                        let start = frame.point(spoke, *radius);
                        (start, offset(start, away, length))
                    })
                    .collect()
            }
        }
    }

    /// The ring at `radius` in the axis's shape: an arc, or a polygon through the frame's
    /// corners.
    fn ring(&self, frame: &PolarFrame, radius: f32) -> Path {
        match self.config.shape {
            PolarShape::Circle => self.arc(frame, radius),
            PolarShape::Polygon => self.polygon(frame, radius),
        }
    }

    /// The arc at `radius` from the start angle to the end angle, or the whole circle.
    fn arc(&self, frame: &PolarFrame, radius: f32) -> Path {
        let config = &self.config;
        let mut builder = Path::builder();
        if config.full() {
            builder.add_circle(
                Point2D::new(frame.center[0], frame.center[1]),
                radius,
                lyon_path::Winding::Positive,
            );
            return builder.build();
        }
        arc_path(frame.center, radius, config.start_angle, config.end_angle)
    }

    /// The polygon at `radius` through the frame's corners between the start angle and the end
    /// angle, closed over a whole circle, or the arc without corners.
    fn polygon(&self, frame: &PolarFrame, radius: f32) -> Path {
        let config = &self.config;
        let (start, end) = (config.start_angle, config.end_angle);
        let full = config.full();
        let mut angles: Vec<f32> = frame
            .corners
            .iter()
            .map(|angle| start + (angle - start).rem_euclid(360.0))
            .filter(|angle| full || *angle <= end + SAME_ANGLE)
            .collect();
        if angles.len() < 2 {
            return self.arc(frame, radius);
        }
        angles.sort_by(f32::total_cmp);
        angles.dedup_by(|a, b| same_angle(*a, *b));
        if !full {
            if !same_angle(angles[0], start) {
                angles.insert(0, start);
            }
            if !same_angle(angles[angles.len() - 1], end) {
                angles.push(end);
            }
        }
        let mut builder = Path::builder();
        let first = frame.point(angles[0], radius);
        builder.begin(Point2D::new(first[0], first[1]));
        for angle in &angles[1..] {
            let [x, y] = frame.point(*angle, radius);
            builder.line_to(Point2D::new(x, y));
        }
        builder.end(full);
        builder.build()
    }
}

/// The arc of `radius` about `center` from `start` to `end`, in degrees clockwise from the top,
/// as cubic segments.
fn arc_path(center: [f32; 2], radius: f32, start: f32, end: f32) -> Path {
    let mut builder = Path::builder();
    let from = offset(center, outward(start), radius);
    builder.begin(Point2D::new(from[0], from[1]));
    // Lyon measures angles from the x axis toward y, which runs down, so a quarter turn before
    // the angle from the top.
    let arc = ArcSegment {
        center: Point2D::new(center[0], center[1]),
        radii: Vector2D::new(radius, radius),
        start_angle: Angle::degrees(start - 90.0),
        sweep_angle: Angle::degrees(end - start),
        x_rotation: Angle::zero(),
    };
    arc.for_each_cubic_bezier(&mut |curve| {
        builder.cubic_bezier_to(curve.ctrl1, curve.ctrl2, curve.to);
    });
    builder.end(false);
    builder.build()
}

/// `angle`, in degrees clockwise, turned half a turn when a label at it would read upside down.
fn upright(angle: f32) -> f32 {
    let angle = (angle + 180.0).rem_euclid(360.0) - 180.0;
    if angle > 90.0 {
        angle - 180.0
    } else if angle <= -90.0 {
        angle + 180.0
    } else {
        angle
    }
}

/// Whether two labels come closer than `gap`: two curved labels about one center by their bands,
/// and otherwise by the pieces that cover them.
fn overlaps(a: &LabelShape, b: &LabelShape, gap: f32) -> bool {
    match (a, b) {
        (LabelShape::Curved(a), LabelShape::Curved(b)) if a.center == b.center => a.near(b, gap),
        (a, b) => {
            let theirs = b.pieces(0.0);
            a.pieces(0.0)
                .iter()
                .any(|mine| theirs.iter().any(|theirs| boxes_overlap(mine, theirs, gap)))
        }
    }
}

/// Whether a label comes closer than [`LABEL_GAP`] to the next, or, around a closed loop, the
/// last to the first.
fn crowded(placed: &PlacedLabels, closed: bool) -> bool {
    let shapes = placed.shapes();
    let neighbors = shapes
        .windows(2)
        .any(|pair| overlaps(&pair[0].1, &pair[1].1, LABEL_GAP));
    let around = closed
        && shapes.len() > 2
        && overlaps(&shapes[shapes.len() - 1].1, &shapes[0].1, LABEL_GAP);
    neighbors || around
}

/// A rule mark of `segments`.
fn rules(
    name: &str,
    segments: &[([f32; 2], [f32; 2])],
    color: [f32; 4],
    width: f32,
) -> SceneRuleMark {
    let column = |pick: fn(&([f32; 2], [f32; 2])) -> f32| -> ScalarOrArray<f32> {
        ScalarOrArray::new_array(segments.iter().map(pick).collect())
    };
    SceneRuleMark {
        name: name.to_string(),
        len: segments.len() as u32,
        x: column(|segment| segment.0[0]),
        y: column(|segment| segment.0[1]),
        x2: column(|segment| segment.1[0]),
        y2: column(|segment| segment.1[1]),
        stroke: ColorOrGradient::Color(color).into(),
        stroke_width: width.into(),
        stroke_cap: StrokeCap::Butt.into(),
        ..Default::default()
    }
}

/// A path mark that strokes `paths` and fills none of them.
fn stroke_paths(name: &str, paths: Vec<Path>, color: [f32; 4], width: f32) -> ScenePathMark {
    ScenePathMark {
        name: name.to_string(),
        len: paths.len() as u32,
        path: ScalarOrArray::new_array(paths),
        fill: ColorOrGradient::Color([0.0; 4]).into(),
        stroke: ColorOrGradient::Color(color).into(),
        stroke_width: Some(width),
        ..Default::default()
    }
}

/// The halo of the placed labels: one color for all of them, or the color of each label's tick.
fn placed_halo(halo: &TextHalo, placed: &PlacedLabels) -> TextHalo {
    let color = match halo.color.value() {
        ScalarOrArrayValue::Scalar(_) => halo.color.clone(),
        ScalarOrArrayValue::Array(colors) => ScalarOrArray::new_array(
            placed
                .shapes()
                .iter()
                .map(|(index, _)| colors.get(*index).copied().unwrap_or([0.0; 4]))
                .collect(),
        ),
    };
    TextHalo {
        color,
        width: halo.width,
    }
}

/// The placed labels' text: straight labels centered on their centers and turned by their angles,
/// or curved labels along the arcs of their bands.
fn label_text(labels: &Labels, placed: &PlacedLabels, style: &AxisStyle) -> SceneTextMark {
    fn column<T>(placed: &[(usize, T)], pick: impl Fn(&T) -> f32) -> ScalarOrArray<f32> {
        ScalarOrArray::new_array(placed.iter().map(|(_, shape)| pick(shape)).collect())
    }
    let shapes = placed.shapes();
    let text = SceneTextMark {
        name: "labels".to_string(),
        len: shapes.len() as u32,
        text: ScalarOrArray::new_array(
            shapes
                .iter()
                .map(|(index, _)| labels.text[*index].clone())
                .collect(),
        ),
        text_syntax: labels.syntax,
        align: TextAlign::Center.into(),
        ..label_font(style)
    };
    match placed {
        PlacedLabels::Straight(straight) => SceneTextMark {
            x: column(straight, |label| label.center[0]),
            y: column(straight, |label| label.center[1]),
            angle: column(straight, |label| label.rotation),
            baseline: TextBaseline::Middle.into(),
            ..text
        },
        // A curved label lies at its circle's center, turned by the angle of its middle, which
        // turns its arc there.
        PlacedLabels::Curved(bands) => SceneTextMark {
            x: column(bands, |band| band.center[0]),
            y: column(bands, |band| band.center[1]),
            angle: column(bands, |band| band.middle),
            baseline: ScalarOrArray::new_array(
                bands.iter().map(|(_, band)| band.baseline()).collect(),
            ),
            text_path: Some(TextPaths {
                path: ScalarOrArray::new_array(bands.iter().map(|(_, band)| band.arc()).collect()),
                offset: PathOffset::Fraction(0.5).into(),
                shift: 0.0.into(),
                upright: true,
                overflow: PathOverflow::Extend,
            }),
            ..text
        },
    }
}
