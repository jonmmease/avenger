//! Polar plots. When the evaluator prepares a chart, each polar plot reads the part of the circle
//! that it draws, and each scale of `theta` and `radius` prepares a polar axis, which fits the
//! scale at each frame and draws when the scale's spec sets an axis. At each size, the polar plots
//! that share a scale take one outer radius: the largest at which each of them holds its circle,
//! its marks at points with their reach, and the labels of its axes. Each plot then draws the grid
//! of its axes, its marks at their angles and radii, and its axes, inside its box.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use arrow::array::ArrayRef;
use avenger_chart::{
    proto::{self, encoding, plot::Coord, polar_layer, Channel},
    resolve::{Resolved, ScaleGroup},
    Chart,
};
use avenger_color::ColorOrGradient;
use avenger_common::{
    types::{TextAlign, TextBaseline},
    value::ScalarOrArray,
};
use avenger_coords::{pieces, PlotCoords, PlotProjection, PolarProjection};
use avenger_guides::axis::{
    arc_path, upright, AxisStyle, PolarAxis, PolarAxisConfig, PolarAxisKind, PolarFrame,
    PolarLabel, PolarLabelOrient, PolarShape,
};
use avenger_scales::scales::{
    coerce::Coercer, fitting::FittingScale, linear::LinearScale, ConfiguredScale,
};
use avenger_scenegraph::marks::{
    arc::SceneArcMark,
    line::SceneLineMark,
    mark::SceneMark,
    path::ScenePathMark,
    text::{SceneTextMark, TextHalo, TextPaths},
};
use avenger_style::{ComputedStyle, ComputedStyles, Property};
use avenger_typst_label::{reads_reversed, LabelEngine, PathOffset, PathOverflow, TextBounds};
use lyon_path::{geom::euclid::Point2D, Path};

use crate::{
    encode::{at, numbers, scalar, LayerRows, Result},
    fit::Position,
    guides::axis_style,
    marks::{pattern_fills, row_patterns, stroke_width, symbol_mark, text_mark, Drawn, TextFormat},
    polyline::{polyline, rule_lines, series_lines, series_trails, Polyline},
    reach::Reach,
    style::{color as style_color, number as style_number, tree::GuideNodes, tree::StyleNodes},
    EvalError, EvalWarning,
};

/// How many times the fit of a circle measures its marks and its axes again at the radius that it
/// found, whose labels and ticks change with the radius.
const FIT_PASSES: usize = 4;

/// The opacity of the halo behind the labels of polar axes, which keeps them readable over the
/// grid and the data.
const HALO_OPACITY: f32 = 0.75;

/// How far the halo behind the labels of polar axes reaches past their glyphs, in pixels.
const HALO_WIDTH: f32 = 2.0;

/// The part of the circle that a polar plot draws: its angles, in degrees clockwise from the top,
/// and its inner radius, as a fraction of its outer radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Sector {
    start: f32,
    end: f32,
    inner: f32,
}

impl Sector {
    fn of(spec: Option<&proto::Polar>) -> Self {
        let spec = spec.cloned().unwrap_or_default();
        let start = spec.start_angle.unwrap_or(0.0);
        Self {
            start,
            end: spec.end_angle.unwrap_or(start + 360.0),
            inner: spec.inner_radius.unwrap_or(0.0),
        }
    }

    /// The part of the circle of `plot`, when it is a polar plot.
    pub(crate) fn of_plot(plot: &proto::Plot) -> Option<Self> {
        match plot.coord.as_ref()? {
            Coord::Polar(polar) => Some(Self::of(polar.options.as_ref())),
            _ => None,
        }
    }

    /// The start angle.
    pub(crate) fn start(&self) -> f32 {
        self.start
    }

    /// Whether the part is the whole circle, whose end is its start.
    fn full(&self) -> bool {
        PolarProjection::new([0.0, 0.0], [0.0, 1.0], [self.start, self.end])
            .period(0)
            .is_some()
    }

    /// The angle halfway between the start and the end.
    fn middle(&self) -> f32 {
        (self.start + self.end) / 2.0
    }

    /// The frame of the circle of `radius` about `center`, without corners.
    fn frame(&self, center: [f32; 2], radius: f32) -> PolarFrame {
        PolarFrame {
            center,
            inner_radius: radius * self.inner,
            outer_radius: radius,
            corners: Vec::new(),
        }
    }

    /// The box around the part of the circle of radius 1 about the origin.
    fn unit_box(&self) -> [[f32; 2]; 2] {
        self.frame([0.0, 0.0], 1.0).sector_box(self.start, self.end)
    }

    /// The aspect ratio of the box around the part of the circle, its width over its height,
    /// which the leaf of the plot keeps.
    pub(crate) fn aspect(&self) -> f32 {
        let [[left, top], [right, bottom]] = self.unit_box();
        (right - left) / (bottom - top)
    }
}

/// Whether `group` is a scale of `theta` over whole circles, whose range ends where it starts, so
/// each of its categories takes one equal step.
pub(crate) fn wraps(resolved: &Resolved, group: &ScaleGroup) -> bool {
    group.channel == Channel::Theta
        && group.members.iter().all(|member| {
            let plot = resolved.tree.layers[member.layer].plot;
            Sector::of_plot(resolved.tree.plots[plot].plot).is_some_and(|sector| sector.full())
        })
}

/// The polar axis of a scale, which fits the scale at each frame and draws when the scale's spec
/// sets an axis.
struct PolarAxisGuide {
    axis: PolarAxis,
    shown: bool,
    grid: bool,
}

/// What preparing the polar plots reads: the chart, its models, its style nodes and their computed
/// styles, the label engine, and the position scale of each position group.
pub(crate) struct PolarInputs<'a> {
    pub(crate) chart: &'a Chart,
    pub(crate) resolved: &'a Resolved<'a>,
    pub(crate) nodes: &'a StyleNodes,
    pub(crate) styles: &'a ComputedStyles,
    pub(crate) engine: &'a LabelEngine,
    pub(crate) positions: &'a [Option<Position>],
    /// The lumps of each group whose members read columns of lumps, by its position in
    /// `Resolved::scales`.
    pub(crate) lumps: &'a [Option<crate::lumps::Lumps>],
}

/// The polar plots of a chart and the axes of their scales, prepared without a size.
pub(crate) struct Polars {
    /// The polar plots, by their position in `Tree::plots`.
    plots: BTreeMap<usize, PolarPlot>,
    /// The polar plots that share a scale, which take one outer radius.
    groups: Vec<Vec<usize>>,
    /// The axis of each scale group of `theta` and `radius`, by its position in
    /// `Resolved::scales`.
    axes: BTreeMap<usize, PolarAxisGuide>,
}

/// A polar plot, prepared without a size.
struct PolarPlot {
    path: String,
    sector: Sector,
    /// The groups of its scales of `theta` and `radius`.
    theta: Option<usize>,
    radius: Option<usize>,
    /// Whether it clips its marks to its box, because a scale's domain can leave out data.
    clip: bool,
    /// Its marks at points, which its circle holds with their reach.
    points: Vec<Points>,
}

/// The rows of a mark at points: the values of `theta` and `radius`, each with whether it goes
/// through a scale, and how far the mark of each row reaches past its point.
struct Points {
    theta: Option<(ArrayRef, bool)>,
    radius: Option<(ArrayRef, bool)>,
    extent: PointExtent,
}

/// How far the mark of each row of points reaches past its point.
enum PointExtent {
    /// Toward the left, the right, the top, and the bottom, at every angle.
    Reach(Vec<Reach>),
    /// Texts whose reach turns with the angles of their points.
    Texts(PolarTexts),
}

impl PointExtent {
    fn len(&self) -> usize {
        match self {
            Self::Reach(reach) => reach.len(),
            Self::Texts(texts) => texts.boxes.len(),
        }
    }
}

/// The texts of a layer, whose reach turns with the angles of their points.
struct PolarTexts {
    /// The box of each text, at the largest font size that its encodings give it, and its
    /// rotation by `angle`, in degrees clockwise.
    boxes: Vec<(TextBounds, f32)>,
    placing: TextPlacing,
}

impl PolarTexts {
    fn of(rows: &LayerRows, text: TextFormat) -> Result<Self> {
        let len = rows.len();
        let sizes = rows.alternatives(Channel::Size, &numbers)?;
        let font_sizes: Vec<f32> = (0..len)
            .map(|row| match sizes.is_empty() {
                true => rows.style.font_size(),
                false => (sizes.iter().map(|sizes| at(sizes, row))).fold(0.0, f32::max),
            })
            .collect();
        let mut mark = text_mark(rows, text, scalar(0.0), scalar(0.0))?;
        mark.font_size = ScalarOrArray::new_array(font_sizes);
        let rotations = mark.angle.as_vec(len, None);
        let boxes = (mark.labels().zip(rotations))
            .map(|(label, rotation)| Ok((rows.engine.bounds(&label.label)?, rotation)))
            .collect::<Result<_>>()?;
        Ok(Self {
            boxes,
            placing: TextPlacing::of(rows.style),
        })
    }

    /// What the text of `row` takes at `angle` and `distance`, in outer radii, when the outer
    /// radius is `radius`.
    fn extents(&self, row: usize, angle: f32, distance: f32, radius: f32) -> Vec<Extent> {
        let (bounds, rotation) = &self.boxes[row];
        let size = [bounds.width, bounds.height];
        let circle = distance * radius;
        if self.placing.orient == proto::PolarTextOrient::Tangential && circle > 0.0 {
            return (self.placing)
                .curved(angle, circle, size)
                .extents(circle, distance);
        }
        let rotation = self.placing.rotation(angle, *rotation);
        let [[left, top], [right, bottom]] = (self.placing)
            .straight([0.0, 0.0], angle, circle, size, rotation)
            .bounds();
        vec![Extent {
            at: outward(angle).map(|value| value * distance),
            reach: [-left, right, -top, bottom],
        }]
    }
}

/// How the texts of a layer lie at their points: how they run, and which parts of them lie at the
/// radius and at the angle of each point, from the computed `text-orient`, `radius-side`, and
/// `theta-side` of the layer's `mark` node.
#[derive(Clone, Copy, Debug)]
struct TextPlacing {
    orient: proto::PolarTextOrient,
    radius: proto::RadiusSide,
    theta: proto::ThetaSide,
}

impl TextPlacing {
    fn of(style: &ComputedStyle) -> Self {
        Self {
            orient: match style.keyword(Property::TextOrient) {
                Some("radial") => proto::PolarTextOrient::Radial,
                Some("tangential") => proto::PolarTextOrient::Tangential,
                _ => proto::PolarTextOrient::Unspecified,
            },
            radius: match style.keyword(Property::RadiusSide) {
                Some("middle") => proto::RadiusSide::Middle,
                Some("inward") => proto::RadiusSide::Inward,
                _ => proto::RadiusSide::Outward,
            },
            theta: match style.keyword(Property::ThetaSide) {
                Some("clockwise") => proto::ThetaSide::Clockwise,
                Some("counterclockwise") => proto::ThetaSide::Counterclockwise,
                _ => proto::ThetaSide::Middle,
            },
        }
    }

    /// The rotation of a straight text at `angle` that its `angle` channel turns by `rotation`:
    /// along its spoke when it is radial.
    fn rotation(&self, angle: f32, rotation: f32) -> f32 {
        match self.orient {
            proto::PolarTextOrient::Radial => upright(angle - 90.0),
            _ => rotation,
        }
    }

    /// The box of a straight text of `size`, turned by `rotation`, beside `point`, at `angle` and
    /// `radius` pixels from the center: out along the spoke from the point by as far as the box
    /// reaches along it when the text lies outward, in when it lies inward, and so on around the
    /// circle when it lies clockwise or counterclockwise. A text at the center centers on it.
    fn straight(
        &self,
        point: [f32; 2],
        angle: f32,
        radius: f32,
        size: [f32; 2],
        rotation: f32,
    ) -> PolarLabel {
        if radius <= 0.0 {
            return PolarLabel {
                center: point,
                size,
                rotation,
            };
        }
        let along = match self.radius {
            proto::RadiusSide::Outward => 1.0,
            proto::RadiusSide::Inward => -1.0,
            _ => 0.0,
        };
        let around = match self.theta {
            proto::ThetaSide::Clockwise => 1.0,
            proto::ThetaSide::Counterclockwise => -1.0,
            _ => 0.0,
        };
        // Standing off the origin along a direction moves the box by as far as it reaches along
        // that direction.
        let shift = |direction: [f32; 2], sign: f32| {
            PolarLabel::standing_off([0.0, 0.0], direction, size, rotation)
                .center
                .map(|value| value * sign)
        };
        let [out, on] = [
            shift(outward(angle), along),
            shift(clockwise(angle), around),
        ];
        PolarLabel {
            center: [point[0] + out[0] + on[0], point[1] + out[1] + on[1]],
            size,
            rotation,
        }
    }

    /// Where a tangential text of `size` lies along the circle of radius `circle` pixels at
    /// `angle`.
    fn curved(&self, angle: f32, circle: f32, size: [f32; 2]) -> CurvedSpan {
        let [width, height] = size;
        let radii = match self.radius {
            proto::RadiusSide::Middle => [circle - height / 2.0, circle + height / 2.0],
            proto::RadiusSide::Inward => [circle - height, circle],
            _ => [circle, circle + height],
        };
        // The text sweeps its width at its nearest radius, which covers it.
        let sweep = (width / radii[0].max(1.0)).to_degrees().min(360.0);
        let angles = match self.theta {
            proto::ThetaSide::Clockwise => [angle, angle + sweep],
            proto::ThetaSide::Counterclockwise => [angle - sweep, angle],
            _ => [angle - sweep / 2.0, angle + sweep / 2.0],
        };
        CurvedSpan {
            angles,
            radii,
            reversed: reads_reversed([1.0, 0.0], (angles[0] + angles[1]) / 2.0),
            placing: *self,
        }
    }
}

/// The part of a ring that a tangential text covers: between two angles, in degrees clockwise
/// from the top, and two distances from the center, in pixels. A text whose middle would read
/// upside down runs counterclockwise, with its top toward the center.
#[derive(Clone, Copy, Debug)]
struct CurvedSpan {
    angles: [f32; 2],
    radii: [f32; 2],
    reversed: bool,
    placing: TextPlacing,
}

impl CurvedSpan {
    /// The alignment and the baseline of the text along its path, as it reads, which keep it to
    /// its sides of the angle and of the circle.
    fn anchor(&self) -> (TextAlign, TextBaseline) {
        let align =
            match (self.placing.theta, self.reversed) {
                (proto::ThetaSide::Clockwise, false)
                | (proto::ThetaSide::Counterclockwise, true) => TextAlign::Left,
                (proto::ThetaSide::Clockwise, true)
                | (proto::ThetaSide::Counterclockwise, false) => TextAlign::Right,
                _ => TextAlign::Center,
            };
        let baseline = match (self.placing.radius, self.reversed) {
            (proto::RadiusSide::Middle, _) => TextBaseline::Middle,
            (proto::RadiusSide::Inward, false) | (proto::RadiusSide::Outward, true) => {
                TextBaseline::Top
            }
            _ => TextBaseline::Bottom,
        };
        (align, baseline)
    }

    /// The circle of radius `circle` about the origin, centered at the top, that the text runs
    /// along: clockwise, or counterclockwise when it reads upside down otherwise.
    fn path(&self, circle: f32) -> Path {
        match self.reversed {
            true => arc_path([0.0, 0.0], circle, 180.0, -180.0),
            false => arc_path([0.0, 0.0], circle, -180.0, 180.0),
        }
    }

    /// The ends of the span and the extremes of its outer arc at each quarter turn between them,
    /// for a text on the circle of radius `circle` pixels at `distance` outer radii.
    fn extents(&self, circle: f32, distance: f32) -> Vec<Extent> {
        let [from, to] = self.angles;
        let [inner, outer] = [self.radii[0].max(0.0), self.radii[1]];
        let mut points = vec![(from, inner), (to, inner), (from, outer), (to, outer)];
        let mut quarter = (from / 90.0).ceil() * 90.0;
        while quarter < to {
            points.push((quarter, outer));
            quarter += 90.0;
        }
        points
            .into_iter()
            .map(|(angle, radius)| {
                let direction = outward(angle);
                let [x, y] = direction.map(|value| value * (radius - circle));
                Extent {
                    at: direction.map(|value| value * distance),
                    reach: [(-x).max(0.0), x.max(0.0), (-y).max(0.0), y.max(0.0)],
                }
            })
            .collect()
    }
}

/// Something that a plot holds with its circle: a point at `at`, in outer radii from the center,
/// and how far it reaches past the point toward the left, the right, the top, and the bottom, in
/// pixels.
struct Extent {
    at: [f32; 2],
    reach: Reach,
}

/// A polar plot at a size: its circle in the plot's frame, and its scales there.
pub(crate) struct PolarFit {
    sector: Sector,
    frame: PolarFrame,
    pub(crate) clip: bool,
    /// The scales of `theta` and `radius`, with their groups.
    pub(crate) theta: Option<(usize, ConfiguredScale)>,
    pub(crate) radius: Option<(usize, ConfiguredScale)>,
}

impl PolarFit {
    /// The plot's circle between the angles of its sector.
    fn circle(&self) -> PolarProjection {
        let Sector { start, end, .. } = self.sector;
        self.frame.projection(start, end)
    }

    /// The plot's coordinates: its circle, with the scale of `theta`, and the scale of `radius`,
    /// or without one, the map of the fractions of the outer radius that an unscaled radius holds
    /// to pixels from the center, so a value past 1 lies outside the circle.
    pub(crate) fn coords(&self) -> PlotCoords {
        let theta = self.theta.as_ref().map(|(_, scale)| scale.clone());
        let radius = (self.radius.as_ref().map(|(_, scale)| scale.clone())).unwrap_or_else(|| {
            let outer = f64::from(self.frame.outer_radius);
            LinearScale::configured((0.0, 1.0), (0.0, outer))
        });
        PlotCoords::new(Arc::new(self.circle()), [theta, Some(radius)])
    }
}

/// The value of the encoding of `channel` in `rows` for each row, and whether it goes through a
/// scale.
fn encoded(rows: &LayerRows, channel: Channel) -> Option<(ArrayRef, bool)> {
    let encoding = rows.encoding(channel)?;
    let column = |name: &str| {
        let column = rows.column(name)?;
        Some(crate::lumps::channel_values(&column, channel))
    };
    Some(match encoding.kind.as_ref()? {
        encoding::Kind::DirectField(name) => (column(name)?, false),
        encoding::Kind::ScaledField(name) => (column(name)?, true),
        encoding::Kind::DirectValue(value) => (rows.chart.value(value).clone(), false),
        encoding::Kind::ScaledValue(value) => (rows.chart.value(value).clone(), true),
    })
}

impl Polars {
    /// The polar plots of the chart, with the marks at points of their layers, from `layer_rows`
    /// with the reach of `reach`, and the axis of each scale of `theta` and `radius`.
    pub(crate) fn prepare<'r>(
        inputs: &PolarInputs,
        layer_rows: impl Fn(usize) -> LayerRows<'r>,
        reach: &[Option<Vec<Reach>>],
    ) -> Result<Self> {
        let resolved = inputs.resolved;
        let tree = &resolved.tree;
        let groups = &resolved.scales;
        let mut plots = BTreeMap::new();
        for (index, plot) in tree.plots.iter().enumerate() {
            let Some(sector) = Sector::of_plot(plot.plot) else {
                continue;
            };
            let scale = |channel: Channel| {
                groups.iter().position(|group| {
                    group.channel == channel
                        && (group.members.iter())
                            .any(|member| tree.layers[member.layer].plot == index)
                })
            };
            let (theta, radius) = (scale(Channel::Theta), scale(Channel::Radius));
            let mut points = Vec::new();
            for layer in plot.layers.clone() {
                let rows = layer_rows(layer);
                let extent = match (&tree.layers[layer].layer, &reach[layer]) {
                    (avenger_chart::layer::LayerRef::Polar(polar), reach) => {
                        match (polar.mark.as_ref(), reach) {
                            (Some(polar_layer::Mark::Text(text)), _) => {
                                let format = TextFormat {
                                    format: &text.format,
                                    syntax: text.syntax,
                                };
                                PointExtent::Texts(PolarTexts::of(&rows, format)?)
                            }
                            (Some(polar_layer::Mark::Symbol(_)), Some(reach)) => {
                                PointExtent::Reach(reach.clone())
                            }
                            _ => continue,
                        }
                    }
                    _ => continue,
                };
                points.push(Points {
                    theta: encoded(&rows, Channel::Theta),
                    radius: encoded(&rows, Channel::Radius),
                    extent,
                });
            }
            let clip = [theta, radius]
                .into_iter()
                .flatten()
                .any(|group| inputs.positions[group].as_ref().is_some_and(|p| p.clip));
            plots.insert(
                index,
                PolarPlot {
                    path: plot.path.clone(),
                    sector,
                    theta,
                    radius,
                    clip,
                    points,
                },
            );
        }

        // The plots that share a scale take one outer radius.
        let mut joined: Vec<BTreeSet<usize>> =
            plots.keys().map(|plot| BTreeSet::from([*plot])).collect();
        for group in groups {
            if !matches!(group.channel, Channel::Theta | Channel::Radius) {
                continue;
            }
            let group_plots: BTreeSet<usize> = (group.members.iter())
                .map(|member| tree.layers[member.layer].plot)
                .collect();
            let (shared, mut apart): (Vec<_>, Vec<_>) = joined
                .into_iter()
                .partition(|set| !set.is_disjoint(&group_plots));
            apart.push(shared.into_iter().flatten().collect());
            joined = apart;
        }
        joined.sort();

        let mut axes = BTreeMap::new();
        for (index, group) in groups.iter().enumerate() {
            if !matches!(group.channel, Channel::Theta | Channel::Radius) {
                continue;
            }
            let Some(plot) = (group.members.first())
                .map(|member| tree.layers[member.layer].plot)
                .filter(|plot| plots.contains_key(plot))
            else {
                continue;
            };
            let Some(position) = &inputs.positions[index] else {
                continue;
            };
            let guide = polar_axis(inputs, index, group, plot, plots[&plot].sector, position)?;
            axes.insert(index, guide);
        }
        Ok(Self {
            plots,
            groups: joined
                .into_iter()
                .map(|set| set.into_iter().collect())
                .collect(),
            axes,
        })
    }

    /// Each polar plot fitted at the size of its rectangle in `sizes`, by plot. The plots that
    /// share a scale take the smallest of their outer radii, so each draws a value of the scale at
    /// one distance from its center.
    pub(crate) fn fit(
        &self,
        sizes: &[[f32; 2]],
        warnings: &mut Vec<EvalWarning>,
    ) -> Result<BTreeMap<usize, PolarFit>> {
        let mut fits = BTreeMap::new();
        for group in &self.groups {
            let mut radius = f32::INFINITY;
            for plot in group {
                let polar = &self.plots[plot];
                let (fitted, fits) = self.outer_radius(polar, sizes[*plot])?;
                if !fits {
                    warnings.push(EvalWarning::Fit {
                        path: polar.path.clone(),
                        reason: "the plot is too small for the marks and the axis labels of its \
                                 circle, so they reach past its edges"
                            .to_string(),
                    });
                }
                radius = radius.min(fitted);
            }
            for plot in group {
                let polar = &self.plots[plot];
                let center = center(&self.extents(polar, radius)?, radius, sizes[*plot]);
                let frame = self.frame(polar, center, radius);
                let scale = |group: Option<usize>| {
                    group.map(|group| (group, self.axes[&group].axis.scale(&frame)))
                };
                fits.insert(
                    *plot,
                    PolarFit {
                        sector: polar.sector,
                        theta: scale(polar.theta),
                        radius: scale(polar.radius),
                        frame,
                        clip: polar.clip,
                    },
                );
            }
        }
        Ok(fits)
    }

    /// The frame of the circle of `plot` at `center` and `radius`, with the ticks of its theta
    /// axis as the corners of polygon rings and rims.
    fn frame(&self, plot: &PolarPlot, center: [f32; 2], radius: f32) -> PolarFrame {
        let mut frame = plot.sector.frame(center, radius);
        if let Some(theta) = plot.theta {
            frame.corners = self.axes[&theta].axis.tick_angles(&frame);
        }
        frame
    }

    /// The largest outer radius at which a plot of `size` holds the circle of `plot`, its marks
    /// at points, and the labels of its axes, and whether they fit. The labels and the positions
    /// of the marks change with the radius, so the fit measures them again at the radius that it
    /// finds.
    fn outer_radius(&self, plot: &PolarPlot, size: [f32; 2]) -> Result<(f32, bool)> {
        let [[left, top], [right, bottom]] = plot.sector.unit_box();
        let largest = (size[0] / (right - left))
            .min(size[1] / (bottom - top))
            .max(0.0);
        let mut radius = largest;
        let mut fits = true;
        for _ in 0..FIT_PASSES {
            let (next, next_fits) = fit_circle(&self.extents(plot, radius)?, size, largest);
            let settled = (next - radius).abs() < 0.01;
            (radius, fits) = (next, next_fits);
            if settled {
                break;
            }
        }
        Ok((radius, fits))
    }

    /// What `plot` holds at `radius`: the box of its circle, the reach of its shown axes past the
    /// box, and its marks at points with their reach.
    fn extents(&self, plot: &PolarPlot, radius: f32) -> Result<Vec<Extent>> {
        let radius = radius.max(1e-3);
        let frame = self.frame(plot, [0.0, 0.0], radius);
        let [[left, top], [right, bottom]] = plot.sector.unit_box();
        let [middle_x, middle_y] = [(left + right) / 2.0, (top + bottom) / 2.0];
        let mut extents = vec![
            Extent {
                at: [left, top],
                reach: [0.0; 4],
            },
            Extent {
                at: [right, bottom],
                reach: [0.0; 4],
            },
        ];
        for group in [plot.theta, plot.radius].into_iter().flatten() {
            let guide = &self.axes[&group];
            if !guide.shown {
                continue;
            }
            let [to_left, to_right, to_top, to_bottom] = guide.axis.reach(&frame);
            extents.extend([
                Extent {
                    at: [left, middle_y],
                    reach: [to_left, 0.0, 0.0, 0.0],
                },
                Extent {
                    at: [right, middle_y],
                    reach: [0.0, to_right, 0.0, 0.0],
                },
                Extent {
                    at: [middle_x, top],
                    reach: [0.0, 0.0, to_top, 0.0],
                },
                Extent {
                    at: [middle_x, bottom],
                    reach: [0.0, 0.0, 0.0, to_bottom],
                },
            ]);
        }
        let scale = |group: Option<usize>| group.map(|group| self.axes[&group].axis.scale(&frame));
        let (theta, radii) = (scale(plot.theta), scale(plot.radius));
        for points in &plot.points {
            let len = points.extent.len();
            let angles = match &points.theta {
                Some(values) => coordinates(values, theta.as_ref(), len)?,
                None => vec![plot.sector.middle(); len],
            };
            // The distance of each point from the center, in outer radii.
            let distances = match &points.radius {
                Some(values @ (_, true)) => coordinates(values, radii.as_ref(), len)?
                    .into_iter()
                    .map(|distance| distance / radius)
                    .collect(),
                Some(values) => coordinates(values, None, len)?,
                None => vec![0.0; len],
            };
            for (row, (angle, distance)) in angles.iter().zip(&distances).enumerate() {
                match &points.extent {
                    PointExtent::Reach(reach) => extents.push(Extent {
                        at: outward(*angle).map(|value| value * distance),
                        reach: reach[row],
                    }),
                    PointExtent::Texts(texts) => {
                        extents.extend(texts.extents(row, *angle, *distance, radius))
                    }
                }
            }
        }
        Ok(extents)
    }

    /// The shown axes of a fitted plot: its theta axis and its radius axis.
    fn shown(&self, fit: &PolarFit) -> [Option<&PolarAxisGuide>; 2] {
        [&fit.theta, &fit.radius].map(|scale| {
            (scale.as_ref())
                .map(|(group, _)| &self.axes[group])
                .filter(|guide| guide.shown)
        })
    }

    /// The grid lines of the axes of a fitted plot, behind its data: spokes at the ticks of
    /// `theta`, and rings at the ticks of `radius`.
    pub(crate) fn backdrop(&self, fit: &PolarFit) -> Vec<SceneMark> {
        (self.shown(fit).into_iter().flatten())
            .filter(|guide| guide.grid)
            .map(|guide| guide.axis.grid(&fit.frame).into())
            .collect()
    }

    /// The axes of a fitted plot, over its data, where the labels of the radius axis give way to
    /// those of the theta axis.
    pub(crate) fn overlay(&self, fit: &PolarFit) -> Vec<SceneMark> {
        let [theta, radius] = self.shown(fit);
        let mut marks: Vec<SceneMark> = Vec::new();
        if let Some(theta) = theta {
            marks.push(theta.axis.marks(&fit.frame).into());
        }
        if let Some(radius) = radius {
            let others: Vec<&PolarAxis> = theta.map(|theta| &theta.axis).into_iter().collect();
            marks.push(radius.axis.marks_avoiding(&fit.frame, &others).into());
        }
        marks
    }
}

/// The values of a point mark along a coordinate for each of `len` rows: through `scale` at the
/// center of a band when the values go through one, and as they are otherwise.
fn coordinates(
    (values, scaled): &(ArrayRef, bool),
    scale: Option<&ConfiguredScale>,
    len: usize,
) -> Result<Vec<f32>> {
    let converted = match (scaled, scale) {
        (true, Some(scale)) => {
            let offset = bandwidth(scale).map_or(0.0, |width| width / 2.0);
            (scale.scale_to_numeric(values)?).map(|value| value + offset)
        }
        _ => Coercer::default().to_numeric(values, None)?,
    };
    Ok(converted.to_scalar_if_len_one().as_vec(len, None))
}

/// The band of a band scale, in pixels or degrees.
fn bandwidth(scale: &ConfiguredScale) -> Option<f32> {
    match scale.scale_impl.scale_type() {
        "band" => avenger_scales::scales::band::bandwidth(&scale.config)
            .ok()
            .map(|width| width as f32),
        _ => None,
    }
}

/// A path of no length at the origin, along which a text centers on the origin.
fn origin_path() -> Path {
    let mut builder = Path::builder();
    builder.begin(Point2D::new(0.0, 0.0));
    builder.end(false);
    builder.build()
}

/// The unit vector from the center toward `angle`, in degrees clockwise from the top, with y
/// down.
fn outward(angle: f32) -> [f32; 2] {
    directions(angle)[1]
}

/// The unit vector a quarter turn clockwise from the direction of `angle`.
fn clockwise(angle: f32) -> [f32; 2] {
    directions(angle)[0]
}

/// The pixel directions in which the angle and the radius increase at `angle`: clockwise around
/// the circle, and outward from its center, as a polar projection's tangents give them.
fn directions(angle: f32) -> [[f32; 2]; 2] {
    let unit = PolarProjection::new([0.0, 0.0], [0.0, 1.0], [0.0, 360.0]);
    (unit.tangents([f64::from(angle), 1.0])).unwrap_or([[1.0, 0.0], [0.0, -1.0]])
}

/// The room that `extents` take on each side of the center along `axis`, 0 for x and 1 for y, at
/// `radius`: how far they reach before it and how far after it.
fn margins(extents: &[Extent], axis: usize, radius: f32) -> (f32, f32) {
    extents.iter().fold(
        (f32::NEG_INFINITY, f32::NEG_INFINITY),
        |(before, after), extent| {
            let at = extent.at[axis] * radius;
            (
                before.max(extent.reach[2 * axis] - at),
                after.max(extent.reach[2 * axis + 1] + at),
            )
        },
    )
}

/// The largest radius up to `largest`, the radius of the box alone, at which `extents` fit a plot
/// of `size`, and whether they fit at any radius. Where they fit at none, the box alone fits.
fn fit_circle(extents: &[Extent], size: [f32; 2], largest: f32) -> (f32, bool) {
    const SAMPLES: usize = 64;
    let fits = |radius: f32| {
        (0..2).all(|axis| {
            let (before, after) = margins(extents, axis, radius);
            before + after <= size[axis] + 1e-3
        })
    };
    let at = |step: usize| largest * step as f32 / SAMPLES as f32;
    // Each margin is the largest of lines in the radius, so the radii that fit form one interval:
    // the fit finds the largest sample that fits and refines it toward the next.
    let Some(step) = (0..=SAMPLES).rev().find(|step| fits(at(*step))) else {
        return (largest, false);
    };
    if step == SAMPLES {
        return (largest, true);
    }
    let (mut low, mut high) = (at(step), at(step + 1));
    for _ in 0..24 {
        let middle = (low + high) / 2.0;
        if fits(middle) {
            low = middle;
        } else {
            high = middle;
        }
    }
    (low, true)
}

/// The center at which `extents` at `radius` stand in the middle of a plot of `size`.
fn center(extents: &[Extent], radius: f32, size: [f32; 2]) -> [f32; 2] {
    [0, 1].map(|axis| {
        let (before, after) = margins(extents, axis, radius);
        (before + size[axis] - after) / 2.0
    })
}

/// The halo behind the labels of the axes of the plot at `path`: its computed `background`, or the
/// chart's when the plot's is transparent, at [`HALO_OPACITY`], or none over a transparent chart.
fn label_halo(inputs: &PolarInputs, path: &str) -> Option<TextHalo> {
    let plot = style_color(
        &inputs.styles[inputs.nodes.plot(path).plot],
        Property::Background,
    );
    let chart = style_color(&inputs.styles[inputs.nodes.chart], Property::Background);
    let [r, g, b, a] = if plot[3] > 0.0 { plot } else { chart };
    (a > 0.0).then(|| TextHalo {
        color: [r, g, b, a * HALO_OPACITY].into(),
        width: HALO_WIDTH,
    })
}

/// The polar axis of the scale group `group` of `theta` or `radius`, at `index`, whose first plot
/// is `plot` with the part of the circle `sector`, from the computed styles of its nodes. It draws
/// when the spec has an axis that the styles show.
fn polar_axis(
    inputs: &PolarInputs,
    index: usize,
    group: &ScaleGroup,
    plot: usize,
    sector: Sector,
    position: &Position,
) -> Result<PolarAxisGuide> {
    let styles = inputs.styles;
    let spec = group.spec.axis.clone().unwrap_or_default();
    let nodes = match inputs.nodes.guides.get(index) {
        Some(Some(GuideNodes::PolarAxis(nodes))) => Some(nodes),
        _ => None,
    };
    let shown = nodes.is_some_and(|nodes| !styles[nodes.axis].is_hidden());
    let mut config = PolarAxisConfig {
        kind: match group.channel {
            Channel::Theta => PolarAxisKind::Theta,
            _ => PolarAxisKind::Radius,
        },
        start_angle: sector.start,
        end_angle: sector.end,
        shape: match proto::Route::try_from(spec.route) {
            Ok(proto::Route::Canvas) => PolarShape::Polygon,
            _ => PolarShape::Circle,
        },
        tick_format: spec
            .labels
            .as_ref()
            .map_or_else(String::new, |labels| labels.format.clone()),
        tick_label: (spec.labels.as_ref())
            .map(|labels| labels.markup.clone())
            .filter(|markup| !markup.is_empty()),
        style: AxisStyle::default(),
        ..Default::default()
    };
    let mut grid = false;
    if let Some(nodes) = nodes {
        let axis = &styles[nodes.axis];
        config.style = axis_style(
            inputs.chart,
            axis,
            &styles[nodes.title],
            &styles[nodes.label],
            &styles[nodes.tick],
            &styles[nodes.rule],
            Some(&styles[nodes.grid]),
            spec.ticks.as_ref(),
            avenger_common::types::TextSyntaxMode::Plain,
        );
        // The computed `angle` of a radius axis runs from the plot's start angle.
        config.angle = Some(sector.start + style_number(axis, Property::Angle));
        config.label_padding = style_number(axis, Property::LabelPadding);
        config.label_orient = match axis.keyword(Property::LabelOrient) {
            Some("radial") => PolarLabelOrient::Radial,
            Some("tangential") => PolarLabelOrient::Tangential,
            _ => match style_number(axis, Property::LabelAngle) {
                0.0 => PolarLabelOrient::Horizontal,
                angle => PolarLabelOrient::Angle(angle),
            },
        };
        config.label_halo = label_halo(inputs, &inputs.resolved.tree.plots[plot].path);
        grid = !styles[nodes.grid].is_hidden();
    }
    // A band or a point axis labels the kept categories, the Other group, and the NULL
    // category of its lumps, and a NULL category in the style of the label's `null` node.
    if let Some(nodes) = nodes {
        config.style.category_labels = crate::guides::category_labels(
            inputs.chart,
            inputs.resolved,
            group,
            inputs.lumps[index].as_ref(),
            nodes.label,
            nodes.variants,
            styles,
            &config.tick_format,
            inputs.engine,
        )?;
    } else if let Some(lumps) = &inputs.lumps[index] {
        config.style.category_labels =
            Some(lumps.category_labels(&config.tick_format, None, None, inputs.engine)?);
    }
    let fitting: FittingScale = position.fitting.clone();
    let engine = inputs.engine;
    let axis = match fitting.scale().scale_impl.scale_type() {
        "band" => PolarAxis::band(fitting, &config, engine)?,
        "point" => PolarAxis::point(fitting, &config, engine)?,
        "nested_band" => {
            return Err(EvalError::Unsupported(format!(
                "a nested band scale of `{}` in a polar plot",
                group.channel.field_name()
            )))
        }
        _ => PolarAxis::continuous(fitting, &config, engine)?,
    };
    Ok(PolarAxisGuide {
        axis,
        shown,
        grid: shown && grid,
    })
}

/// An annular sector: its angles in degrees and its radii in pixels, in order, and the space
/// between each of its sides along a spoke and the spoke, in pixels.
struct Wedge {
    angles: [f32; 2],
    radii: [f32; 2],
    gap: f32,
}

impl Wedge {
    /// The sector between `angles` and `radii` with `inset` given up at each side. Its ends at
    /// one radius move in, but not past their middle, and an end at the center stays there. Its
    /// sides along spokes stand `inset` from their spokes, and its inner end lies no nearer the
    /// center than where they meet, so the gap between two slices of a pie is as wide at the
    /// center as at the edge.
    fn new(angles: [f32; 2], radii: [f32; 2], inset: f32) -> Self {
        let angles = [angles[0].min(angles[1]), angles[0].max(angles[1])];
        let (low, high) = (radii[0].min(radii[1]), radii[0].max(radii[1]));
        let inset = inset.max(0.0);
        let middle = (low + high) / 2.0;
        let inner = if low > 0.0 { low + inset } else { 0.0 };
        let (inner, outer) = if inner > high - inset {
            (middle, middle)
        } else {
            (inner, high - inset)
        };
        let half = (angles[1] - angles[0]).to_radians() / 2.0;
        let meet = if inset > 0.0 && half > 0.0 && half < std::f32::consts::PI {
            inset / half.sin()
        } else {
            0.0
        };
        Self {
            angles,
            radii: [inner.max(meet).min(outer), outer],
            gap: inset,
        }
    }

    /// The pad angle of an arc mark that keeps the sides `gap` from their spokes.
    fn pad_angle(&self) -> f32 {
        let pad_radius = self.radii[0].hypot(self.radii[1]);
        if self.gap > 0.0 && pad_radius > 0.0 {
            2.0 * (self.gap / pad_radius).min(1.0).asin()
        } else {
            0.0
        }
    }
}

/// The pieces in pixels of the segment between the positions `from` and `to` of `circle`: a
/// straight line between their pixels for the canvas route, and the arc between them otherwise.
fn segment(
    circle: &PolarProjection,
    from: [f64; 2],
    to: [f64; 2],
    canvas: bool,
) -> Vec<Vec<[f32; 2]>> {
    if canvas {
        return pieces([circle.pixel(from), circle.pixel(to)]);
    }
    circle.route(from, to)
}

/// Whether a route is the canvas route.
fn canvas(route: i32) -> bool {
    proto::Route::try_from(route) == Ok(proto::Route::Canvas)
}

/// The scene marks of a layer of a polar plot, whose mark is `mark`, in the plot fitted at its
/// size.
pub(crate) fn marks(
    rows: &LayerRows,
    mark: &polar_layer::Mark,
    fit: &PolarFit,
) -> Result<Vec<Drawn>> {
    use polar_layer::Mark;
    let drawer = Drawer { rows, fit };
    match mark {
        Mark::Symbol(_) => drawer.symbols(),
        Mark::Line(line) => drawer.lines(canvas(line.route)),
        Mark::Trail(trail) => drawer.trails(canvas(trail.route)),
        Mark::Area(area) => drawer.areas(area.orient, canvas(area.route)),
        Mark::Bar(bar) => drawer.bars(bar.orient, canvas(bar.route)),
        Mark::Rect(rect) => drawer.rects(canvas(rect.route)),
        Mark::Rule(rule) => drawer.rules(canvas(rule.route)),
        Mark::Text(text) => drawer.texts(TextFormat {
            format: &text.format,
            syntax: text.syntax,
        }),
    }
}

/// The rows of a layer of a polar plot fitted at its size, which draw its marks.
struct Drawer<'a, 'r> {
    rows: &'a LayerRows<'r>,
    fit: &'a PolarFit,
}

impl Drawer<'_, '_> {
    fn one(&self, mark: SceneMark) -> Vec<Drawn> {
        vec![Drawn {
            mark,
            rows: self.rows.table.clone(),
        }]
    }

    /// The value of `channel` for each row, an angle in degrees for `theta` and `theta2` and a
    /// radius in pixels for `radius` and `radius2`: through the scale of the pair, at its band
    /// position in a band when `centered`, or without a scale, degrees or fractions of the outer
    /// radius. `None` when the layer does not encode the channel.
    fn coordinates(&self, channel: Channel, centered: bool) -> Result<Option<Vec<f32>>> {
        let rows = self.rows;
        if !rows.has(channel) {
            return Ok(None);
        }
        let values = match centered {
            true => rows.position(channel)?,
            false => rows.numbers(channel)?.unwrap_or_else(|| scalar(0.0)),
        };
        let mut values = values.as_vec(rows.len(), None);
        let radius = matches!(channel, Channel::Radius | Channel::Radius2);
        if radius && rows.scale_for(channel).is_none() {
            for value in &mut values {
                *value *= self.fit.frame.outer_radius;
            }
        }
        Ok(Some(values))
    }

    /// The angle of each row, or the middle of the plot's angles without `theta`.
    fn angles(&self) -> Result<Vec<f32>> {
        Ok((self.coordinates(Channel::Theta, true)?)
            .unwrap_or_else(|| vec![self.fit.sector.middle(); self.rows.len()]))
    }

    /// The radius of each row, or the center without `radius`.
    fn radii(&self) -> Result<Vec<f32>> {
        Ok(
            (self.coordinates(Channel::Radius, true)?)
                .unwrap_or_else(|| vec![0.0; self.rows.len()]),
        )
    }

    fn symbols(&self) -> Result<Vec<Drawn>> {
        let (x, y): (Vec<f32>, Vec<f32>) = (self.angles()?.into_iter())
            .zip(self.radii()?)
            .map(|(angle, radius)| {
                let [x, y] = self.fit.frame.point(angle, radius);
                (x, y)
            })
            .unzip();
        let mark = symbol_mark(
            self.rows,
            ScalarOrArray::new_array(x),
            ScalarOrArray::new_array(y),
        )?;
        Ok(self.one(mark.into()))
    }

    /// The polyline through `series` in its order, at the angle and the radius of each row from
    /// `place`, along the canvas or the coordinate route. A series of one row is its point.
    fn polyline(
        &self,
        series: &[u32],
        place: impl Fn(usize) -> [f32; 2],
        canvas: bool,
    ) -> Polyline {
        let circle = self.fit.circle();
        polyline(
            series,
            |row| place(row).map(f64::from),
            |position| circle.pixel(position),
            |from, to| segment(&circle, from, to, canvas),
        )
    }

    /// The polyline of each series of a line or a trail through its rows along its route.
    fn series_polylines(&self, canvas: bool) -> Result<Vec<(Vec<u32>, Polyline)>> {
        let (angles, radii) = (self.angles()?, self.radii()?);
        Ok((self.rows.series()?.into_iter())
            .map(|series| {
                let polyline = self.polyline(&series, |row| [angles[row], radii[row]], canvas);
                (series, polyline)
            })
            .collect())
    }

    fn lines(&self, canvas: bool) -> Result<Vec<Drawn>> {
        series_lines(self.rows, self.series_polylines(canvas)?)
    }

    fn trails(&self, canvas: bool) -> Result<Vec<Drawn>> {
        series_trails(self.rows, self.series_polylines(canvas)?)
    }

    /// The baseline of a bar or an area along `channel`: zero when its scale's domain holds it,
    /// or the start of the scale's range, and without a scale, the start angle or the center.
    fn baseline(&self, channel: Channel) -> Result<f32> {
        match (self.rows.scale_for(channel), channel) {
            (Some(_), _) => self.rows.baseline(channel),
            (None, Channel::Theta) => Ok(self.fit.sector.start),
            (None, _) => Ok(0.0),
        }
    }

    /// The span of each row along `channel`: from its value to the value of `second`, across its
    /// band on a band scale without `second`, at its value on a continuous scale without
    /// `second`, and across `whole` without the channel.
    fn span(&self, channel: Channel, second: Channel, whole: [f32; 2]) -> Result<Vec<[f32; 2]>> {
        let rows = self.rows;
        if !rows.has(channel) {
            return Ok(vec![whole; rows.len()]);
        }
        if let Some(end) = self.coordinates(second, true)? {
            let start = (self.coordinates(channel, true)?).expect("the layer encodes the channel");
            return Ok(start.into_iter().zip(end).map(|(a, b)| [a, b]).collect());
        }
        if let Some(bands) = rows.bands(channel)? {
            return Ok((0..rows.len())
                .map(|row| {
                    let (start, width) = at(&bands, row);
                    [start, start + width]
                })
                .collect());
        }
        let start = (self.coordinates(channel, false)?).expect("the layer encodes the channel");
        Ok(start.into_iter().map(|a| [a, a]).collect())
    }

    /// The span of each row of a bar along its measure `channel`: from the value of `second`, or
    /// the baseline, to its value, or to the far end of `whole` without the channel.
    fn measure(&self, channel: Channel, second: Channel, whole: [f32; 2]) -> Result<Vec<[f32; 2]>> {
        let len = self.rows.len();
        let end = (self.coordinates(channel, false)?).unwrap_or_else(|| vec![whole[1]; len]);
        let start = match self.coordinates(second, false)? {
            Some(start) => start,
            None => vec![self.baseline(channel)?; len],
        };
        Ok(start.into_iter().zip(end).map(|(a, b)| [a, b]).collect())
    }

    /// The angles of the plot, from its start to its end.
    fn whole_angles(&self) -> [f32; 2] {
        [self.fit.sector.start, self.fit.sector.end]
    }

    /// The radii of the plot, from its inner radius to its outer radius.
    fn whole_radii(&self) -> [f32; 2] {
        [self.fit.frame.inner_radius, self.fit.frame.outer_radius]
    }

    fn bars(&self, orient: i32, canvas: bool) -> Result<Vec<Drawn>> {
        let (angles, radii) = match self.rows.horizontal(orient) {
            true => (
                self.measure(Channel::Theta, Channel::Theta2, self.whole_angles())?,
                self.span(Channel::Radius, Channel::Radius2, self.whole_radii())?,
            ),
            false => (
                self.span(Channel::Theta, Channel::Theta2, self.whole_angles())?,
                self.measure(Channel::Radius, Channel::Radius2, self.whole_radii())?,
            ),
        };
        self.wedges(angles, radii, 0.0, canvas)
    }

    fn rects(&self, canvas: bool) -> Result<Vec<Drawn>> {
        let angles = self.span(Channel::Theta, Channel::Theta2, self.whole_angles())?;
        let radii = self.span(Channel::Radius, Channel::Radius2, self.whole_radii())?;
        let inset = style_number(self.rows.style, Property::Inset);
        self.wedges(angles, radii, inset, canvas)
    }

    /// The annular sector of each row between its angles and its radii: an arc mark, or with the
    /// canvas route, a path mark of polygons whose ends at one radius are straight. A path mark
    /// has one stroke width, so the polygons take the widest of their rows.
    fn wedges(
        &self,
        angles: Vec<[f32; 2]>,
        radii: Vec<[f32; 2]>,
        inset: f32,
        canvas: bool,
    ) -> Result<Vec<Drawn>> {
        let rows = self.rows;
        let wedges: Vec<Wedge> = (angles.into_iter().zip(radii))
            .map(|(angles, radii)| Wedge::new(angles, radii, inset))
            .collect();
        let (fill, stroke) = rows.paint()?;
        let width = match rows.numbers(Channel::StrokeWidth)? {
            Some(width) => width,
            None => scalar(stroke_width(rows)?.unwrap_or(0.0)),
        };
        let fill_pattern = pattern_fills(row_patterns(rows)?);
        let column =
            |value: fn(&Wedge) -> f32| ScalarOrArray::new_array(wedges.iter().map(value).collect());
        let mark: SceneMark = match canvas {
            true => {
                let widest = (width.as_vec(rows.len(), None).into_iter()).fold(0.0_f32, f32::max);
                ScenePathMark {
                    name: rows.name.clone(),
                    len: rows.len() as u32,
                    path: ScalarOrArray::new_array(
                        wedges.iter().map(|wedge| self.polygon(wedge)).collect(),
                    ),
                    fill,
                    stroke,
                    stroke_width: scalar(widest),
                    fill_pattern,
                    ..Default::default()
                }
                .into()
            }
            false => SceneArcMark {
                name: rows.name.clone(),
                len: rows.len() as u32,
                x: scalar(self.fit.frame.center[0]),
                y: scalar(self.fit.frame.center[1]),
                start_angle: column(|wedge| wedge.angles[0].to_radians()),
                end_angle: column(|wedge| wedge.angles[1].to_radians()),
                inner_radius: column(|wedge| wedge.radii[0]),
                outer_radius: column(|wedge| wedge.radii[1]),
                pad_angle: column(Wedge::pad_angle),
                corner_radius: scalar(style_number(rows.style, Property::CornerRadius)),
                fill,
                stroke,
                stroke_width: width,
                fill_pattern,
                ..Default::default()
            }
            .into(),
        };
        Ok(self.one(mark))
    }

    /// The polygon of `wedge` with straight ends at one radius, whose sides stand its gap inside
    /// their spokes.
    fn polygon(&self, wedge: &Wedge) -> Path {
        let corner = |angle: f32, radius: f32, side: f32| {
            let [x, y] = self.fit.frame.point(angle, radius);
            let [across_x, across_y] = clockwise(angle);
            Point2D::new(
                x + side * wedge.gap * across_x,
                y + side * wedge.gap * across_y,
            )
        };
        let [start, end] = wedge.angles;
        let [inner, outer] = wedge.radii;
        let mut builder = Path::builder();
        builder.begin(corner(start, outer, 1.0));
        builder.line_to(corner(end, outer, -1.0));
        builder.line_to(corner(end, inner, -1.0));
        builder.line_to(corner(start, inner, 1.0));
        builder.end(true);
        builder.build()
    }

    /// The region of each series between its line and its base, a path mark with the area's fill,
    /// and with a stroke, a line mark around the region, which draws its dash. A vertical area
    /// fills from `radius2`, or the baseline of `radius`, to `radius` along `theta`, and a
    /// horizontal one from `theta2`, or the baseline of `theta`, to `theta` along `radius`.
    fn areas(&self, orient: i32, canvas: bool) -> Result<Vec<Drawn>> {
        let rows = self.rows;
        let horizontal = rows.horizontal(orient);
        let len = rows.len();
        let (angles, radii) = (self.angles()?, self.radii()?);
        let (channel, second) = match horizontal {
            true => (Channel::Theta, Channel::Theta2),
            false => (Channel::Radius, Channel::Radius2),
        };
        let base = match self.coordinates(second, true)? {
            Some(base) => base,
            None => vec![self.baseline(channel)?; len],
        };
        let (fill, stroke) = rows.paint()?;
        let width = match rows.numbers(Channel::StrokeWidth)? {
            Some(width) => width,
            None => scalar(stroke_width(rows)?.unwrap_or(0.0)),
        };
        let dash = rows.dashes_or_style()?;
        let pattern = row_patterns(rows)?;
        let mut drawn = Vec::new();
        for (index, series) in rows.series()?.into_iter().enumerate() {
            let first = series[0] as usize;
            let line = self.polyline(&series, |row| [angles[row], radii[row]], canvas);
            let reversed: Vec<u32> = series.iter().rev().copied().collect();
            let base_line = self.polyline(
                &reversed,
                |row| match horizontal {
                    true => [base[row], radii[row]],
                    false => [angles[row], base[row]],
                },
                canvas,
            );
            // The region runs out along the line and back along the base.
            let mut x: Vec<f32> = line.x.iter().chain(&base_line.x).copied().collect();
            let mut y: Vec<f32> = line.y.iter().chain(&base_line.y).copied().collect();
            let mut vertex_rows: Vec<u32> =
                line.rows.iter().chain(&base_line.rows).copied().collect();
            let mut builder = Path::builder();
            let mut open = false;
            for (x, y) in x.iter().zip(&y) {
                if !x.is_finite() || !y.is_finite() {
                    continue;
                }
                if open {
                    builder.line_to(Point2D::new(*x, *y));
                } else {
                    builder.begin(Point2D::new(*x, *y));
                    open = true;
                }
            }
            if open {
                builder.end(true);
            }
            drawn.push(Drawn {
                mark: ScenePathMark {
                    name: format!("{}/{index}", rows.name),
                    len: 1,
                    path: scalar(builder.build()),
                    fill: scalar(at(&fill, first)),
                    fill_pattern: pattern_fills(
                        pattern.as_ref().map(|pattern| scalar(at(pattern, first))),
                    ),
                    stroke: scalar(ColorOrGradient::transparent()),
                    ..Default::default()
                }
                .into(),
                rows: rows.rows(&series[..1])?,
            });
            let stroke_width = at(&width, first);
            if stroke_width > 0.0 && !x.is_empty() {
                // The line around the region ends where it starts.
                x.push(x[0]);
                y.push(y[0]);
                vertex_rows.push(vertex_rows[0]);
                drawn.push(Drawn {
                    mark: SceneLineMark {
                        name: format!("{}/{index}/outline", rows.name),
                        len: x.len() as u32,
                        defined: ScalarOrArray::new_array(
                            x.iter().map(|value| value.is_finite()).collect(),
                        ),
                        x: ScalarOrArray::new_array(x),
                        y: ScalarOrArray::new_array(y),
                        stroke: at(&stroke, first),
                        stroke_width,
                        stroke_dash: (dash.as_ref())
                            .map(|dash| at(dash, first))
                            .filter(|dash| !dash.is_empty()),
                        ..Default::default()
                    }
                    .into(),
                    rows: rows.rows(&vertex_rows)?,
                });
            }
        }
        Ok(drawn)
    }

    /// One line for each rule, from (`theta`, `radius`) to (`theta2`, `radius2`). A rule at one
    /// angle is a spoke and a rule at one radius is a ring, whatever its route, and a rule
    /// without `theta` spans the plot's angles and one without `radius` its radii. A rule with
    /// both second values takes its route between its ends.
    fn rules(&self, canvas: bool) -> Result<Vec<Drawn>> {
        let rows = self.rows;
        let pairs = |channel, second, whole: [f32; 2]| -> Result<Vec<[f32; 2]>> {
            Ok(
                match (
                    self.coordinates(channel, true)?,
                    self.coordinates(second, true)?,
                ) {
                    (None, _) => vec![whole; rows.len()],
                    (Some(start), None) => start.into_iter().map(|a| [a, a]).collect(),
                    (Some(start), Some(end)) => {
                        start.into_iter().zip(end).map(|(a, b)| [a, b]).collect()
                    }
                },
            )
        };
        let angles = pairs(Channel::Theta, Channel::Theta2, self.whole_angles())?;
        let radii = pairs(Channel::Radius, Channel::Radius2, self.whole_radii())?;
        let circle = self.fit.circle();
        let mut lines = Vec::new();
        for (row, ([from, to], [inner, outer])) in angles.into_iter().zip(radii).enumerate() {
            let [from, to, inner, outer] = [from, to, inner, outer].map(f64::from);
            // A rule that holds its angle runs along a spoke, and one that holds its radius
            // along an arc.
            let pieces = if from == to {
                circle.iso_line(0, from, Some([inner, outer]))
            } else if inner == outer {
                circle.iso_line(1, inner, Some([from, to]))
            } else {
                segment(&circle, [from, inner], [to, outer], canvas)
            };
            let mut polyline = Polyline::default();
            for piece in pieces {
                polyline.push_piece(&piece, [row as u32; 2]);
            }
            lines.push((row, polyline));
        }
        rule_lines(rows, lines)
    }

    /// The text of each row beside its point, as its sides say: along the circle through the
    /// point when it is tangential, and otherwise straight, turned along its spoke when it is
    /// radial.
    fn texts(&self, text: TextFormat) -> Result<Vec<Drawn>> {
        let (angles, radii) = (self.angles()?, self.radii()?);
        let placing = TextPlacing::of(self.rows.style);
        let mark = match placing.orient {
            proto::PolarTextOrient::Tangential => {
                self.curved_texts(text, placing, &angles, &radii)?
            }
            _ => self.straight_texts(text, placing, &angles, &radii)?,
        };
        Ok(self.one(mark.into()))
    }

    /// The straight texts at `angles` and `radii`, each centered on its box as `placing` sets it.
    fn straight_texts(
        &self,
        text: TextFormat,
        placing: TextPlacing,
        angles: &[f32],
        radii: &[f32],
    ) -> Result<SceneTextMark> {
        let rows = self.rows;
        let mut mark = text_mark(rows, text, scalar(0.0), scalar(0.0))?;
        mark.align = scalar(TextAlign::Center);
        mark.baseline = scalar(TextBaseline::Middle);
        let rotations = mark.angle.as_vec(rows.len(), None);
        let mut boxes = Vec::with_capacity(rows.len());
        for (row, label) in mark.labels().enumerate() {
            let bounds = rows.engine.bounds(&label.label)?;
            boxes.push(placing.straight(
                self.fit.frame.point(angles[row], radii[row]),
                angles[row],
                radii[row],
                [bounds.width, bounds.height],
                placing.rotation(angles[row], rotations[row]),
            ));
        }
        mark.x = ScalarOrArray::new_array(boxes.iter().map(|placed| placed.center[0]).collect());
        mark.y = ScalarOrArray::new_array(boxes.iter().map(|placed| placed.center[1]).collect());
        mark.angle = ScalarOrArray::new_array(boxes.iter().map(|placed| placed.rotation).collect());
        Ok(mark)
    }

    /// The tangential texts at `angles` and `radii`, as a polar axis curves its labels: each from
    /// the center, turned to the angle of its point, along a circle of its radius that starts and
    /// ends at the bottom and runs the way that the text reads upright, keeping to its sides of
    /// the angle and of the circle. A text at the center centers on it.
    fn curved_texts(
        &self,
        text: TextFormat,
        placing: TextPlacing,
        angles: &[f32],
        radii: &[f32],
    ) -> Result<SceneTextMark> {
        let rows = self.rows;
        let [x, y] = self.fit.frame.center;
        let mut mark = text_mark(rows, text, scalar(x), scalar(y))?;
        mark.align = scalar(TextAlign::Center);
        mark.baseline = scalar(TextBaseline::Middle);
        let mut anchors = Vec::with_capacity(rows.len());
        let mut turns = Vec::with_capacity(rows.len());
        let mut paths = Vec::with_capacity(rows.len());
        for (row, label) in mark.labels().enumerate() {
            let (angle, radius) = (angles[row], radii[row]);
            if radius <= 0.0 {
                anchors.push((TextAlign::Center, TextBaseline::Middle));
                turns.push(0.0);
                paths.push(origin_path());
                continue;
            }
            let bounds = rows.engine.bounds(&label.label)?;
            let span = placing.curved(angle, radius, [bounds.width, bounds.height]);
            anchors.push(span.anchor());
            turns.push(angle);
            paths.push(span.path(radius));
        }
        mark.align = ScalarOrArray::new_array(anchors.iter().map(|anchor| anchor.0).collect());
        mark.baseline = ScalarOrArray::new_array(anchors.iter().map(|anchor| anchor.1).collect());
        mark.angle = ScalarOrArray::new_array(turns);
        mark.text_path = Some(TextPaths {
            path: ScalarOrArray::new_array(paths),
            offset: PathOffset::Fraction(0.5).into(),
            shift: 0.0.into(),
            upright: false,
            overflow: PathOverflow::Extend,
        });
        Ok(mark)
    }
}
