//! Polar plots. When the evaluator prepares a chart, each polar plot reads the part of the circle
//! that it draws, and each scale of `theta` and `radius` prepares a polar axis, which fits the
//! scale at each size and draws the axis when the scale's spec sets one. At each size, the polar
//! plots that share a scale take one outer radius: the largest at which each of them holds its
//! circle, its marks at points with their reach, and the labels of its axes. Each plot then draws
//! the grid of its axes, its marks at their angles and radii, and its axes, inside its box.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use arrow::array::{ArrayRef, RecordBatch};
use avenger_color::ColorOrGradient;
use avenger_common::types::{TextAlign, TextBaseline};
use avenger_common::value::ScalarOrArray;
use avenger_coords::{PlotProjection, PolarProjection};
use avenger_guides::{
    axis::{
        arc_path, upright, PolarAxis, PolarAxisConfig, PolarAxisKind, PolarFrame, PolarLabel,
        PolarLabelOrient, PolarShape,
    },
    GuideSide,
};
use avenger_scales::scales::{coerce::Coercer, ConfiguredScale};
use avenger_scenegraph::marks::{
    arc::SceneArcMark,
    line::SceneLineMark,
    mark::SceneMark,
    path::ScenePathMark,
    text::{SceneTextMark, TextPaths},
};
use avenger_typst_label::{reads_reversed, LabelEngine, PathOffset, PathOverflow, TextBounds};
use lyon_path::{geom::euclid::Point2D, Path};

use avenger_chart::{
    proto::{self, coord, encoding, guide, mark, Channel},
    resolve::{PlotRef, ScaleGroup, Tree},
    Chart, FitKind,
};

use crate::{
    coord::{CoordSystem, FittedPlot, PlotUnit, ProjectionGuide, SystemInputs},
    encode::{self, at, bandwidth, pattern_fills, scalar, Drawn, Polyline, Projected, UnitRows},
    evaluator::{EvaluationWarning, PlotCoord, ScaleViews},
    fit::{self, Position, Reach},
    guides::{axis_config, numeric},
    scale::eval_error,
    EvalError, Theme,
};

type Result<T> = std::result::Result<T, EvalError>;

/// How far apart, in degrees, the angles of a whole circle may lie, as polar axes allow.
const SAME_ANGLE: f32 = 1e-3;

/// The largest step between the points of a segment at a steady change of angle, in degrees.
const STEP: f32 = 2.0;

/// How many times the fit of a circle measures its marks and its axes again at the radius that it
/// found, whose labels and ticks change with the radius.
const FIT_PASSES: usize = 4;

/// The part of the circle that a polar plot draws: its angles, in degrees clockwise from the top,
/// and its inner radius, as a fraction of its outer radius.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Sector {
    start: f32,
    end: f32,
    inner: f32,
}

impl Sector {
    fn of(spec: &proto::Polar) -> Self {
        let start = spec.start_angle.unwrap_or(0.0);
        Self {
            start,
            end: spec.end_angle.unwrap_or(start + 360.0),
            inner: spec.inner_radius.unwrap_or(0.0),
        }
    }

    /// The part of the circle of `plot`, when it is a polar plot.
    fn of_plot(plot: &PlotRef) -> Option<Self> {
        match plot.coord?.coord.as_ref()? {
            coord::Coord::Polar(polar) => Some(Self::of(polar)),
            _ => None,
        }
    }

    /// Whether the part is the whole circle, whose end is its start.
    fn full(&self) -> bool {
        self.end - self.start >= 360.0 - SAME_ANGLE
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
    fn aspect(&self) -> f32 {
        let [[left, top], [right, bottom]] = self.unit_box();
        (right - left) / (bottom - top)
    }
}

/// The polar plots of a chart and the axes of their scales, prepared without a size, with each
/// polar plot at its size at the last fit.
pub(crate) struct PolarPlots {
    /// The polar plots, by their index in the plots of the tree.
    plots: BTreeMap<usize, PolarPlot>,
    /// The polar plots that share a scale, which take one outer radius.
    groups: Vec<Vec<usize>>,
    /// The axis of each scale of `theta` and `radius`, by group.
    axes: BTreeMap<usize, PolarAxisGuide>,
    /// Each polar plot at its size at the last fit, by plot.
    fitted: BTreeMap<usize, PolarFit>,
}

/// The polar axis of a scale, which fits the scale at each size and draws when the scale's spec
/// sets an axis.
struct PolarAxisGuide {
    axis: PolarAxis,
    shown: bool,
    grid: bool,
}

/// A polar plot, prepared without a size.
struct PolarPlot {
    path: String,
    sector: Sector,
    /// The groups of its scales of `theta` and `radius`.
    theta: Option<usize>,
    radius: Option<usize>,
    /// Whether it clips its marks to its box, because a scale has an explicit domain that can
    /// leave out data.
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

/// The texts of a unit, whose reach turns with the angles of their points.
struct PolarTexts {
    /// The box of each text, at the largest font size that its encodings give it, and its
    /// rotation by `angle`, in degrees clockwise.
    boxes: Vec<(TextBounds, f32)>,
    placing: TextPlacing,
}

impl PolarTexts {
    fn of(rows: &UnitRows, text: &proto::TextMark) -> Result<Self> {
        let mark = fit::largest_text(rows, text)?;
        let rotations = mark.angle.as_vec(rows.len(), None);
        let boxes = mark
            .labels()
            .zip(rotations)
            .map(|(label, rotation)| {
                let bounds = rows
                    .engine
                    .bounds(&label.label)
                    .map_err(|error| eval_error("the text of a text mark", error))?;
                Ok((bounds, rotation))
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            boxes,
            placing: TextPlacing::of(text),
        })
    }

    /// What the text of `row` takes at `angle` and `distance`, in outer radii, when the outer
    /// radius is `radius`.
    fn extents(&self, row: usize, angle: f32, distance: f32, radius: f32) -> Vec<Extent> {
        let (bounds, rotation) = &self.boxes[row];
        let size = [bounds.width, bounds.height];
        let circle = distance * radius;
        if self.placing.orient == proto::PolarTextOrient::Tangential && circle > 0.0 {
            return self
                .placing
                .curved(angle, circle, size)
                .extents(circle, distance);
        }
        let rotation = self.placing.rotation(angle, *rotation);
        let [[left, top], [right, bottom]] = self
            .placing
            .straight([0.0, 0.0], angle, circle, size, rotation)
            .bounds();
        vec![Extent {
            at: outward(angle).map(|value| value * distance),
            reach: [-left, right, -top, bottom],
        }]
    }
}

/// How the texts of a unit lie at their points: how they run, and which parts of them lie at the
/// radius and at the angle of each point.
#[derive(Clone, Copy, Debug)]
struct TextPlacing {
    orient: proto::PolarTextOrient,
    radius: proto::RadiusSide,
    theta: proto::ThetaSide,
}

impl TextPlacing {
    fn of(text: &proto::TextMark) -> Self {
        Self {
            orient: proto::PolarTextOrient::try_from(text.orient).unwrap_or_default(),
            radius: match proto::RadiusSide::try_from(text.radius_side) {
                Ok(proto::RadiusSide::Middle) => proto::RadiusSide::Middle,
                Ok(proto::RadiusSide::Inward) => proto::RadiusSide::Inward,
                _ => proto::RadiusSide::Outward,
            },
            theta: match proto::ThetaSide::try_from(text.theta_side) {
                Ok(proto::ThetaSide::Clockwise) => proto::ThetaSide::Clockwise,
                Ok(proto::ThetaSide::Counterclockwise) => proto::ThetaSide::Counterclockwise,
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
struct PolarFit {
    sector: Sector,
    frame: PolarFrame,
    clip: bool,
    /// The scales of `theta` and `radius`, with their groups.
    theta: Option<(usize, ConfiguredScale)>,
    radius: Option<(usize, ConfiguredScale)>,
}

/// A polar plot fitted at its size at the last fit.
struct FittedPolar<'a> {
    polars: &'a PolarPlots,
    fit: &'a PolarFit,
}

/// The value of the encoding of `channel` in `unit` for each row of `table`, and whether it goes
/// through a scale.
fn encoded(
    chart: &Chart,
    unit: &proto::Unit,
    channel: Channel,
    table: &RecordBatch,
) -> Option<(ArrayRef, bool)> {
    let encoding = unit
        .encodings
        .iter()
        .find(|encoding| encoding.channel == channel as i32)?;
    let values = match encoding.value.as_ref()? {
        encoding::Value::Column(name) => table.column_by_name(name)?.clone(),
        encoding::Value::Constant(index) | encoding::Value::ScaledConstant(index) => {
            chart.value(*index).clone()
        }
    };
    Some((values, encoding.scale.is_some()))
}

impl CoordSystem for PolarPlots {
    /// Whether `group` is a scale of `theta` over whole circles, whose range ends where it starts, so
    /// each of its categories takes one equal step.
    fn wraps(tree: &Tree, group: &ScaleGroup) -> bool {
        group.channel == Channel::Theta
            && group.members.iter().all(|member| {
                Sector::of_plot(&tree.plots[tree.units[member.unit].plot])
                    .is_some_and(|sector| sector.full())
            })
    }

    /// The polar plots of the tree, with the marks at points of `units`, and the axis of each
    /// scale of `theta` and `radius`, from the fitting scales.
    fn prepare(inputs: &SystemInputs, units: &[PlotUnit]) -> Result<Self> {
        let (tree, groups) = (inputs.tree, inputs.groups);
        let mut plots = BTreeMap::new();
        for (index, plot) in tree.plots.iter().enumerate() {
            let Some(sector) = Sector::of_plot(plot) else {
                continue;
            };
            let scale = |channel: Channel| {
                groups.iter().position(|group| {
                    group.channel == channel
                        && group
                            .members
                            .iter()
                            .any(|member| tree.units[member.unit].plot == index)
                })
            };
            let mut points = Vec::new();
            for unit in units.iter().filter(|unit| unit.plot == index) {
                let rows = &unit.rows;
                let extent = match (rows.mark, unit.reach) {
                    (mark::Mark::Text(text), _) => PointExtent::Texts(PolarTexts::of(rows, text)?),
                    (mark::Mark::Symbol(_), Some(reach)) => PointExtent::Reach(reach.to_vec()),
                    _ => continue,
                };
                let values = |channel| encoded(rows.chart, rows.unit, channel, &rows.table);
                points.push(Points {
                    theta: values(Channel::Theta),
                    radius: values(Channel::Radius),
                    extent,
                });
            }
            plots.insert(
                index,
                PolarPlot {
                    path: plot.path.clone(),
                    sector,
                    theta: scale(Channel::Theta),
                    radius: scale(Channel::Radius),
                    clip: false,
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
            let group_plots: BTreeSet<usize> = group
                .members
                .iter()
                .map(|member| tree.units[member.unit].plot)
                .collect();
            let (shared, mut apart): (Vec<_>, Vec<_>) = joined
                .into_iter()
                .partition(|set| !set.is_disjoint(&group_plots));
            apart.push(shared.into_iter().flatten().collect());
            joined = apart;
        }
        let mut polars = Self {
            plots,
            groups: joined
                .into_iter()
                .map(|set| set.into_iter().collect())
                .collect(),
            axes: BTreeMap::new(),
            fitted: BTreeMap::new(),
        };
        polars.rescale(inputs)?;
        Ok(polars)
    }

    /// Prepares the axis of each scale of `theta` and `radius` from the fitting scales, and
    /// whether each plot clips its marks, which change when the domain of a scale reads a
    /// selection.
    fn rescale(&mut self, inputs: &SystemInputs) -> Result<()> {
        let (chart, tree, groups) = (inputs.chart, inputs.tree, inputs.groups);
        let (positions, theme, engine) = (inputs.positions, inputs.theme, inputs.engine);
        for plot in self.plots.values_mut() {
            plot.clip = [plot.theta, plot.radius]
                .into_iter()
                .flatten()
                .any(|group| positions[&group].clip);
        }
        let mut axes = BTreeMap::new();
        for (index, group) in groups.iter().enumerate() {
            let kind = match group.channel {
                Channel::Theta => PolarAxisKind::Theta,
                Channel::Radius => PolarAxisKind::Radius,
                _ => continue,
            };
            let Some(sector) = group
                .members
                .first()
                .and_then(|member| self.plots.get(&tree.units[member.unit].plot))
                .map(|plot| plot.sector)
            else {
                continue;
            };
            axes.insert(
                index,
                polar_axis(
                    chart,
                    group,
                    &positions[&index],
                    kind,
                    sector,
                    theme,
                    engine,
                )?,
            );
        }
        self.axes = axes;
        Ok(())
    }

    /// A polar plot keeps the shape of the box around its part of the circle.
    fn guide(&self, plot: usize) -> ProjectionGuide {
        ProjectionGuide {
            aspect: self.plots.get(&plot).map(|plot| plot.sector.aspect()),
            slabs: Vec::new(),
        }
    }

    /// Fits each polar plot at the size of its rectangle in `sizes`. The plots that share a scale
    /// take the smallest of their outer radii, so each draws a value of the scale at one distance
    /// from its center.
    fn fit(
        &mut self,
        _positions: &BTreeMap<usize, Position>,
        _views: &mut ScaleViews,
        sizes: &[[f32; 2]],
        warnings: &mut Vec<EvaluationWarning>,
    ) -> Result<()> {
        let mut fits = BTreeMap::new();
        for group in &self.groups {
            let mut radius = f32::INFINITY;
            for plot in group {
                let polar = &self.plots[plot];
                let (fitted, fits) = self.outer_radius(polar, sizes[*plot])?;
                if !fits {
                    warnings.push(EvaluationWarning {
                        path: polar.path.clone(),
                        message: "the plot is too small for the marks and the axis labels of \
                                  its circle, so they reach past its edges"
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
        self.fitted = fits;
        Ok(())
    }

    fn fitted(&self, plot: usize) -> Option<Box<dyn FittedPlot + '_>> {
        Some(Box::new(FittedPolar {
            polars: self,
            fit: self.fitted.get(&plot)?,
        }))
    }
}

impl PolarPlots {
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
            scale
                .scale_to_numeric(values)
                .map_err(|error| eval_error("a position of a polar plot", error))?
                .map(|value| value + offset)
        }
        _ => Coercer::default()
            .to_numeric(values, None)
            .map_err(|error| eval_error("a position of a polar plot", error))?,
    };
    Ok(converted.to_scalar_if_len_one().as_vec(len, None))
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
    let (sin, cos) = angle.to_radians().sin_cos();
    [sin, -cos]
}

/// The unit vector a quarter turn clockwise from the direction of `angle`.
fn clockwise(angle: f32) -> [f32; 2] {
    let (sin, cos) = angle.to_radians().sin_cos();
    [cos, sin]
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

/// The polar axis of a scale of `theta` or `radius`, from its spec over the theme, which draws
/// when the spec sets an axis.
fn polar_axis(
    chart: &Chart,
    group: &ScaleGroup,
    position: &Position,
    kind: PolarAxisKind,
    sector: Sector,
    theme: &Theme,
    engine: &LabelEngine,
) -> Result<PolarAxisGuide> {
    let spec = match group
        .spec
        .guide
        .as_ref()
        .and_then(|guide| guide.guide.as_ref())
    {
        Some(guide::Guide::Axis(axis)) => Some(axis.clone()),
        _ => None,
    };
    let shown = spec.is_some();
    let spec = spec.unwrap_or_default();
    let scale_type = group.spec.r#type.as_str();
    let (line, grid) = axis_config(
        chart,
        &spec,
        GuideSide::Bottom,
        true,
        numeric(scale_type),
        theme,
    )?;
    let config = PolarAxisConfig {
        kind,
        start_angle: sector.start,
        end_angle: sector.end,
        angle: spec.angle,
        shape: match proto::Route::try_from(spec.route) {
            Ok(proto::Route::Canvas) => PolarShape::Polygon,
            _ => PolarShape::Circle,
        },
        label_orient: match spec.label_turn {
            Some(proto::axis::LabelTurn::LabelAngle(angle)) => PolarLabelOrient::Angle(angle),
            Some(proto::axis::LabelTurn::LabelOrient(orient)) => {
                match proto::PolarTextOrient::try_from(orient) {
                    Ok(proto::PolarTextOrient::Radial) => PolarLabelOrient::Radial,
                    Ok(proto::PolarTextOrient::Tangential) => PolarLabelOrient::Tangential,
                    _ => PolarLabelOrient::Horizontal,
                }
            }
            None => PolarLabelOrient::Horizontal,
        },
        label_halo: theme.polar_label_halo.clone(),
        tick_format: line.tick_format,
        tick_label: line.tick_label,
        style: line.style,
        ..Default::default()
    };
    let fitting = position.fitting.clone();
    let axis = match scale_type {
        "band" => PolarAxis::band(fitting, &config, engine),
        "point" => PolarAxis::point(fitting, &config, engine),
        "nested_band" => {
            return Err(EvalError::Evaluation(format!(
                "a polar plot draws no nested band scale of `{}`",
                group.channel.as_str_name()
            )))
        }
        _ => PolarAxis::continuous(fitting, &config, engine),
    }
    .map_err(|error| eval_error("a polar axis", error))?;
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

impl FittedPlot for FittedPolar<'_> {
    /// The scale of `theta` or `radius` of the group `group`.
    fn scale(&self, group: usize) -> Option<&ConfiguredScale> {
        [&self.fit.theta, &self.fit.radius]
            .into_iter()
            .flatten()
            .find(|(own, _)| *own == group)
            .map(|(_, scale)| scale)
    }

    /// Whether the plot clips its marks to its box.
    fn clips(&self) -> bool {
        self.fit.clip
    }

    /// The coordinate system of the plot: its scales and its circle.
    fn coord(&self) -> PlotCoord {
        PlotCoord::Polar {
            theta: self.fit.theta.as_ref().map(|(_, scale)| scale.clone()),
            radius: self.fit.radius.as_ref().map(|(_, scale)| scale.clone()),
            center: self.fit.frame.center,
            inner_radius: self.fit.frame.inner_radius,
            outer_radius: self.fit.frame.outer_radius,
        }
    }

    /// The plot's circle between the angles of its sector, whose range positions are degrees
    /// clockwise from the top and pixels from the center, as the scales of `theta` and `radius`
    /// take them.
    fn projection(&self, _size: [f32; 2]) -> Option<Arc<dyn PlotProjection>> {
        let Sector { start, end, .. } = self.fit.sector;
        Some(Arc::new(PolarProjection::new(
            self.fit.frame.center,
            [self.fit.frame.inner_radius, self.fit.frame.outer_radius],
            [start, end],
        )))
    }

    /// The grid lines of the plot's axes, behind its data: spokes at the ticks of `theta`, and
    /// rings at the ticks of `radius`.
    fn backdrop(&self) -> Result<Vec<SceneMark>> {
        Ok(self
            .shown()
            .into_iter()
            .flatten()
            .filter(|guide| guide.grid)
            .map(|guide| guide.axis.grid(&self.fit.frame).into())
            .collect())
    }

    /// The axes of the plot, over its data, where the labels of the radius axis give way to
    /// those of the theta axis.
    fn overlay(&self) -> Vec<SceneMark> {
        let [theta, radius] = self.shown();
        let mut marks: Vec<SceneMark> = Vec::new();
        if let Some(theta) = theta {
            marks.push(theta.axis.marks(&self.fit.frame).into());
        }
        if let Some(radius) = radius {
            let others: Vec<&PolarAxis> = theta.map(|theta| &theta.axis).into_iter().collect();
            marks.push(radius.axis.marks_avoiding(&self.fit.frame, &others).into());
        }
        marks
    }

    /// The scene marks of a unit of the plot. A symbol and a text stand at the point of their
    /// angle and radius.
    fn marks(&self, rows: &mut UnitRows) -> Result<Vec<Drawn>> {
        match rows.mark {
            mark::Mark::Text(text) => self.texts(rows, text),
            mark::Mark::Symbol(_) => {
                rows.projected = Some(self.points(rows)?);
                encode::marks(rows)
            }
            mark::Mark::Line(line) => {
                encode::series_lines(rows, self.series_polylines(rows, line.route)?)
            }
            mark::Mark::Trail(trail) => {
                encode::series_trails(rows, self.series_polylines(rows, trail.route)?)
            }
            mark::Mark::Area(area) => self.areas(rows, area),
            mark::Mark::Bar(bar) => self.bars(rows, bar),
            mark::Mark::Rect(rect) => self.rects(rows, rect),
            mark::Mark::Rule(rule) => self.rules(rows, rule.route),
            mark::Mark::Image(_) | mark::Mark::Path(_) | mark::Mark::Geoshape(_) => {
                Err(EvalError::Evaluation(format!(
                    "the polar plot of `{}` draws no image, path, or geoshape marks",
                    rows.name
                )))
            }
        }
    }
}

impl FittedPolar<'_> {
    /// The shown axes of the plot: its theta axis and its radius axis.
    fn shown(&self) -> [Option<&PolarAxisGuide>; 2] {
        [&self.fit.theta, &self.fit.radius].map(|scale| {
            scale
                .as_ref()
                .map(|(group, _)| &self.polars.axes[group])
                .filter(|guide| guide.shown)
        })
    }

    /// The value of `channel` for each row, an angle in degrees for `theta` and `theta2` and a
    /// radius in pixels for `radius` and `radius2`: through the scale of the pair, at the center
    /// of a band when `centered`, or without a scale, degrees or fractions of the outer radius.
    /// `None` when the unit does not encode the channel.
    fn coordinates(
        &self,
        rows: &UnitRows,
        channel: Channel,
        centered: bool,
    ) -> Result<Option<Vec<f32>>> {
        if !rows.has(channel) {
            return Ok(None);
        }
        let values = if centered {
            rows.position(channel)?
        } else {
            rows.numbers(channel)?.unwrap_or_else(|| scalar(0.0))
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
    fn angles(&self, rows: &UnitRows) -> Result<Vec<f32>> {
        Ok(self
            .coordinates(rows, Channel::Theta, true)?
            .unwrap_or_else(|| vec![self.fit.sector.middle(); rows.len()]))
    }

    /// The radius of each row, or the center without `radius`.
    fn radii(&self, rows: &UnitRows) -> Result<Vec<f32>> {
        Ok(self
            .coordinates(rows, Channel::Radius, true)?
            .unwrap_or_else(|| vec![0.0; rows.len()]))
    }

    /// The point of each row.
    fn points(&self, rows: &UnitRows) -> Result<Projected> {
        let (x, y) = self
            .angles(rows)?
            .into_iter()
            .zip(self.radii(rows)?)
            .map(|(angle, radius)| {
                let [x, y] = self.fit.frame.point(angle, radius);
                (x, y)
            })
            .unzip();
        Ok(Projected { x, y })
    }

    /// The text of each row beside its point, as its sides say: along the circle through the
    /// point when it is tangential, and otherwise straight, turned along its spoke when it is
    /// radial.
    fn texts(&self, rows: &UnitRows, text: &proto::TextMark) -> Result<Vec<Drawn>> {
        let (angles, radii) = (self.angles(rows)?, self.radii(rows)?);
        let placing = TextPlacing::of(text);
        let mark = match placing.orient {
            proto::PolarTextOrient::Tangential => {
                self.curved_texts(rows, text, placing, &angles, &radii)?
            }
            _ => self.straight_texts(rows, text, placing, &angles, &radii)?,
        };
        Ok(vec![Drawn {
            mark: mark.into(),
            rows: rows.table.clone(),
        }])
    }

    /// The straight texts of `rows` at `angles` and `radii`, each centered on its box as
    /// `placing` sets it.
    fn straight_texts(
        &self,
        rows: &UnitRows,
        text: &proto::TextMark,
        placing: TextPlacing,
        angles: &[f32],
        radii: &[f32],
    ) -> Result<SceneTextMark> {
        let mut mark = encode::text_mark(rows, text, scalar(0.0), scalar(0.0))?;
        let rotations = mark.angle.as_vec(rows.len(), None);
        let mut boxes = Vec::with_capacity(rows.len());
        for (row, label) in mark.labels().enumerate() {
            let bounds = rows
                .engine
                .bounds(&label.label)
                .map_err(|error| eval_error("the text of a text mark", error))?;
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

    /// The tangential texts of `rows` at `angles` and `radii`, as a polar axis curves its labels:
    /// each from the center, turned to the angle of its point, along a circle of its radius that
    /// starts and ends at the bottom and runs the way that the text reads upright, keeping to its
    /// sides of the angle and of the circle. A text at the center centers on it.
    fn curved_texts(
        &self,
        rows: &UnitRows,
        text: &proto::TextMark,
        placing: TextPlacing,
        angles: &[f32],
        radii: &[f32],
    ) -> Result<SceneTextMark> {
        let [x, y] = self.fit.frame.center;
        let mut mark = encode::text_mark(rows, text, scalar(x), scalar(y))?;
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
            let bounds = rows
                .engine
                .bounds(&label.label)
                .map_err(|error| eval_error("the text of a text mark", error))?;
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

    /// The points from the angle and the radius `from` to those of `to`, at a steady change of
    /// both.
    fn sweep(&self, from: [f32; 2], to: [f32; 2]) -> Vec<[f32; 2]> {
        let steps = ((to[0] - from[0]).abs() / STEP).ceil().max(1.0) as usize;
        (0..=steps)
            .map(|step| {
                let along = step as f32 / steps as f32;
                self.fit.frame.point(
                    from[0] + (to[0] - from[0]) * along,
                    from[1] + (to[1] - from[1]) * along,
                )
            })
            .collect()
    }

    /// The points of the segment from `from` to `to`, each an angle and a radius: its ends with
    /// the canvas route, and with the coordinate route, points at a steady change of angle and
    /// radius, the short way around a whole circle.
    fn segment(&self, from: [f32; 2], to: [f32; 2], canvas: bool) -> Vec<[f32; 2]> {
        if canvas {
            return vec![
                self.fit.frame.point(from[0], from[1]),
                self.fit.frame.point(to[0], to[1]),
            ];
        }
        let mut to = to;
        if self.fit.sector.full() {
            to[0] = from[0] + ((to[0] - from[0] + 180.0).rem_euclid(360.0) - 180.0);
        }
        self.sweep(from, to)
    }

    /// The polyline through `series` in its order, at the angle and the radius of each row from
    /// `place`, along `canvas` or the coordinate route.
    fn polyline(
        &self,
        series: &[u32],
        place: impl Fn(usize) -> [f32; 2],
        canvas: bool,
    ) -> Polyline {
        encode::polyline(
            series,
            place,
            |[angle, radius]| Some(self.fit.frame.point(angle, radius)),
            |from, to| vec![self.segment(from, to, canvas)],
        )
    }

    /// The polyline of each series of a line or a trail through its rows along `route`.
    fn series_polylines(&self, rows: &UnitRows, route: i32) -> Result<Vec<(Vec<u32>, Polyline)>> {
        let canvas = proto::Route::try_from(route) == Ok(proto::Route::Canvas);
        let (angles, radii) = (self.angles(rows)?, self.radii(rows)?);
        Ok(rows
            .series()?
            .into_iter()
            .map(|series| {
                let polyline = self.polyline(&series, |row| [angles[row], radii[row]], canvas);
                (series, polyline)
            })
            .collect())
    }

    /// Whether a bar or an area runs along `radius`: when its orient says so, or else when only
    /// its `theta` scale is continuous.
    fn horizontal(rows: &UnitRows, orient: i32) -> bool {
        let continuous = |channel| {
            rows.scale_for(channel).is_some_and(|scale| {
                FitKind::of(scale.scale_impl.scale_type()) == FitKind::Interval
            })
        };
        match proto::Orient::try_from(orient) {
            Ok(proto::Orient::Horizontal) => true,
            Ok(proto::Orient::Vertical) => false,
            _ => continuous(Channel::Theta) && !continuous(Channel::Radius),
        }
    }

    /// The baseline of a bar or an area along `channel`: zero when its scale's domain holds it,
    /// or the start of the scale's range, and without a scale, the start angle or the center.
    fn baseline(&self, rows: &UnitRows, channel: Channel) -> Result<f32> {
        match (rows.scale_for(channel), channel) {
            (Some(_), _) => rows.baseline(channel),
            (None, Channel::Theta) => Ok(self.fit.sector.start),
            (None, _) => Ok(0.0),
        }
    }

    /// The span of each row along `channel`: from its value to the value of `second`, across its
    /// band on a band scale without `second`, at its value on a continuous scale without
    /// `second`, and across `whole` without the channel.
    fn span(
        &self,
        rows: &UnitRows,
        channel: Channel,
        second: Channel,
        whole: [f32; 2],
    ) -> Result<Vec<[f32; 2]>> {
        let Some(start) = self.coordinates(rows, channel, rows.has(second))? else {
            return Ok(vec![whole; rows.len()]);
        };
        if let Some(end) = self.coordinates(rows, second, true)? {
            return Ok(start.into_iter().zip(end).map(|(a, b)| [a, b]).collect());
        }
        let width = rows.scale_for(channel).and_then(bandwidth).unwrap_or(0.0);
        Ok(start.into_iter().map(|a| [a, a + width]).collect())
    }

    /// The span of each row of a bar along its measure `channel`: from the value of `second`, or
    /// the baseline, to its value, or to the far end of `whole` without the channel.
    fn measure(
        &self,
        rows: &UnitRows,
        channel: Channel,
        second: Channel,
        whole: [f32; 2],
    ) -> Result<Vec<[f32; 2]>> {
        let len = rows.len();
        let end = self
            .coordinates(rows, channel, false)?
            .unwrap_or_else(|| vec![whole[1]; len]);
        let start = match self.coordinates(rows, second, false)? {
            Some(start) => start,
            None => vec![self.baseline(rows, channel)?; len],
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

    fn bars(&self, rows: &UnitRows, bar: &proto::BarMark) -> Result<Vec<Drawn>> {
        let (angles, radii) = if Self::horizontal(rows, bar.orient) {
            (
                self.measure(rows, Channel::Theta, Channel::Theta2, self.whole_angles())?,
                self.span(rows, Channel::Radius, Channel::Radius2, self.whole_radii())?,
            )
        } else {
            (
                self.span(rows, Channel::Theta, Channel::Theta2, self.whole_angles())?,
                self.measure(rows, Channel::Radius, Channel::Radius2, self.whole_radii())?,
            )
        };
        self.wedges(rows, angles, radii, bar.corner_radius, 0.0, bar.route)
    }

    fn rects(&self, rows: &UnitRows, rect: &proto::RectMark) -> Result<Vec<Drawn>> {
        let angles = self.span(rows, Channel::Theta, Channel::Theta2, self.whole_angles())?;
        let radii = self.span(rows, Channel::Radius, Channel::Radius2, self.whole_radii())?;
        self.wedges(
            rows,
            angles,
            radii,
            rect.corner_radius,
            rect.inset.unwrap_or(0.0),
            rect.route,
        )
    }

    /// The annular sector of each row between its angles and its radii: an arc mark, or with the
    /// canvas route, a path mark of polygons whose ends at one radius are straight. A path mark
    /// has one stroke width, so the polygons take the widest of their rows.
    fn wedges(
        &self,
        rows: &UnitRows,
        angles: Vec<[f32; 2]>,
        radii: Vec<[f32; 2]>,
        corner_radius: Option<f32>,
        inset: f32,
        route: i32,
    ) -> Result<Vec<Drawn>> {
        let wedges: Vec<Wedge> = angles
            .into_iter()
            .zip(radii)
            .map(|(angles, radii)| Wedge::new(angles, radii, inset))
            .collect();
        let (fill, stroke) = rows.paint(true)?;
        let has_stroke = rows.has(Channel::Stroke);
        let width = rows
            .numbers(Channel::StrokeWidth)?
            .unwrap_or_else(|| scalar(if has_stroke { 1.0 } else { 0.0 }));
        let fill_pattern = pattern_fills(rows.patterns(Channel::Pattern)?);
        let column =
            |value: fn(&Wedge) -> f32| ScalarOrArray::new_array(wedges.iter().map(value).collect());
        let mark: SceneMark = if proto::Route::try_from(route) == Ok(proto::Route::Canvas) {
            let widest = width
                .as_vec(rows.len(), None)
                .into_iter()
                .fold(0.0_f32, f32::max);
            ScenePathMark {
                name: rows.name.clone(),
                len: rows.len() as u32,
                path: ScalarOrArray::new_array(
                    wedges.iter().map(|wedge| self.polygon(wedge)).collect(),
                ),
                fill,
                stroke,
                stroke_width: Some(widest),
                fill_pattern,
                ..Default::default()
            }
            .into()
        } else {
            SceneArcMark {
                name: rows.name.clone(),
                len: rows.len() as u32,
                x: scalar(self.fit.frame.center[0]),
                y: scalar(self.fit.frame.center[1]),
                start_angle: column(|wedge| wedge.angles[0].to_radians()),
                end_angle: column(|wedge| wedge.angles[1].to_radians()),
                inner_radius: column(|wedge| wedge.radii[0]),
                outer_radius: column(|wedge| wedge.radii[1]),
                pad_angle: column(Wedge::pad_angle),
                corner_radius: scalar(corner_radius.unwrap_or(0.0)),
                fill,
                stroke,
                stroke_width: width,
                fill_pattern,
                ..Default::default()
            }
            .into()
        };
        Ok(vec![Drawn {
            mark,
            rows: rows.table.clone(),
        }])
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
    fn areas(&self, rows: &UnitRows, area: &proto::AreaMark) -> Result<Vec<Drawn>> {
        let horizontal = Self::horizontal(rows, area.orient);
        let canvas = proto::Route::try_from(area.route) == Ok(proto::Route::Canvas);
        let len = rows.len();
        let (angles, radii) = (self.angles(rows)?, self.radii(rows)?);
        let (channel, second) = if horizontal {
            (Channel::Theta, Channel::Theta2)
        } else {
            (Channel::Radius, Channel::Radius2)
        };
        let base = match self.coordinates(rows, second, true)? {
            Some(base) => base,
            None => vec![self.baseline(rows, channel)?; len],
        };
        let (fill, stroke) = rows.paint(true)?;
        let width = rows
            .numbers(Channel::StrokeWidth)?
            .unwrap_or_else(|| scalar(if rows.has(Channel::Stroke) { 1.0 } else { 0.0 }));
        let dash = rows.dashes(Channel::StrokeDash)?;
        let pattern = rows.patterns(Channel::Pattern)?;
        let mut drawn = Vec::new();
        for (index, series) in rows.series()?.into_iter().enumerate() {
            let first = series[0] as usize;
            let line = self.polyline(&series, |row| [angles[row], radii[row]], canvas);
            let reversed: Vec<u32> = series.iter().rev().copied().collect();
            let base_line = self.polyline(
                &reversed,
                |row| {
                    if horizontal {
                        [base[row], radii[row]]
                    } else {
                        [angles[row], base[row]]
                    }
                },
                canvas,
            );
            // The region runs out along the line and back along the base.
            let mut x: Vec<f32> = line.x.iter().chain(&base_line.x).copied().collect();
            let mut y: Vec<f32> = line.y.iter().chain(&base_line.y).copied().collect();
            let mut vertex_rows: Vec<u32> =
                line.rows.iter().chain(&base_line.rows).copied().collect();
            let mut builder = Path::builder();
            for (vertex, (x, y)) in x.iter().zip(&y).enumerate() {
                if vertex == 0 {
                    builder.begin(Point2D::new(*x, *y));
                } else {
                    builder.line_to(Point2D::new(*x, *y));
                }
            }
            if !x.is_empty() {
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
                        x: ScalarOrArray::new_array(x),
                        y: ScalarOrArray::new_array(y),
                        stroke: at(&stroke, first),
                        stroke_width,
                        stroke_dash: dash
                            .as_ref()
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
    fn rules(&self, rows: &UnitRows, route: i32) -> Result<Vec<Drawn>> {
        let canvas = proto::Route::try_from(route) == Ok(proto::Route::Canvas);
        let pairs = |channel, second, whole: [f32; 2]| -> Result<Vec<[f32; 2]>> {
            Ok(
                match (
                    self.coordinates(rows, channel, true)?,
                    self.coordinates(rows, second, true)?,
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
        let lines = angles
            .into_iter()
            .zip(radii)
            .enumerate()
            .map(|(row, ([from, to], [inner, outer]))| {
                let points = if from == to {
                    vec![
                        self.fit.frame.point(from, inner),
                        self.fit.frame.point(from, outer),
                    ]
                } else if inner == outer {
                    self.sweep([from, inner], [to, inner])
                } else {
                    self.segment([from, inner], [to, outer], canvas)
                };
                let mut polyline = Polyline::default();
                polyline.push_piece(&points, [row as u32; 2]);
                (row, polyline)
            })
            .collect();
        encode::rule_lines(rows, lines)
    }
}
