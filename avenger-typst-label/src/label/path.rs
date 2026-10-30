//! Labels on paths: a label laid out straight, then bent so that its text follows a curve.
//!
//! The bend moves each glyph cluster, rigidly, so that the middle of its advance lies on the path
//! and it turns to the path's direction there, as SVG's `textPath` places glyphs, and warps each
//! shape point by point. A math glyph that the rigid motion would set off the path, such as a wide
//! accent or a stretched brace along a tight curve, warps through its outline as a shape does.
//! Each line follows the path offset to its own baseline, so glyphs keep their advances there
//! instead of crowding inside a bend. The bent frame's top left is its box's, and the label's bent
//! state records where the path's origin lies in it, which places the label, and each line's
//! baseline as a curve, along which SVG can draw the label's runs as text.

use std::ops::Range;
use std::sync::Arc;

use kurbo::{BezPath, CubicBez, Line, ParamCurve as _, PathEl, PathSeg, QuadBez};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::bounds::LineBox;
use super::engine::CompiledLabel;
use super::frame::{
    Curve, CurveItem, FillRule, FrameItem, Geometry, GroupItem, LabelFrame, Point, Shape,
    Size, TextItem, Transform,
};
use super::outline::{bez_path, glyph_path};
use crate::typst_library::text::FontFlags;

/// A path that a label's text follows, in points, with the label's position as its origin.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TextPath {
    /// The path, whose first subpath the text follows.
    pub path: Curve,
    /// Where along the path the text's anchor lies.
    pub offset: PathOffset,
    /// Which point of the text lies at the offset.
    pub anchor: PathAnchor,
    /// Which part of the text lies on the path. Top and bottom are the text's own, so when
    /// `upright` reverses the path, `Bottom` still sets the text on top as it reads.
    pub baseline: PathBaseline,
    /// How far the text moves away from the path, in points, on the side where `baseline` puts
    /// it: toward the text's bottom for `Top` and `LineTop`, and toward its top for the others.
    /// Half a line's stroke width, plus any padding, clears the line from either side with one
    /// positive value. A negative shift moves the text toward the path.
    pub shift: f32,
    /// With `Some(angle)`, the text runs along the path reversed where it would read upside
    /// down once the caller turns the label by `angle` degrees clockwise, as [`reads_reversed`]
    /// decides at the middle of the text. The offset of a reversed path counts from its other
    /// end. With `None`, the text runs along the path as given.
    pub upright: Option<f32>,
    /// What happens to text that reaches past the path's ends.
    pub overflow: PathOverflow,
}

/// Where along a path a label's anchor lies.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum PathOffset {
    /// A fraction of the path's length, from its start.
    Fraction(f32),
    /// A distance along the path from its start, in points.
    Distance(f32),
}

/// Which point of a label's text lies at its path's offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum PathAnchor {
    /// The start of the text's box.
    Start,
    /// The middle of the text's box.
    Middle,
    /// The end of the text's box.
    End,
}

/// Which part of a label's text lies on its path: the parts of the label's box that a straight
/// label's baseline names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum PathBaseline {
    /// The first line's baseline, so descenders cross the path.
    Alphabetic,
    /// The top of the box, so the text hangs below the path.
    Top,
    /// The middle of the box, so the text is centered on the path.
    Middle,
    /// The bottom of the box, so the text sits on top of the path.
    Bottom,
    /// The top of the line box: the box with half the gap between lines above it.
    LineTop,
    /// The bottom of the line box: the box with half the gap between lines below it.
    LineBottom,
}

/// What happens to the text of a label on a path that reaches past the path's ends.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum PathOverflow {
    /// The text runs past the ends along the path's directions there.
    #[default]
    Extend,
    /// The label lays out nothing, so that a caller can draw it another way.
    Fit,
}

/// Whether text that runs along `direction`, turned by `angle` degrees clockwise, reads upside
/// down: whether it then points left. Text that points straight up or down reads either way,
/// and does not. With `upright`, a label on a path reverses when the path's direction at the
/// middle of its text reads upside down, where the middle lies half the label's straight width
/// past its start, along the path as given. A caller can therefore tell which labels reverse
/// without laying them out.
pub fn reads_reversed(direction: [f32; 2], angle: f32) -> bool {
    let (sin, cos) = angle.to_radians().sin_cos();
    let length = direction[0].hypot(direction[1]);
    direction[0] * cos - direction[1] * sin < -VERTICAL * length
}

/// How close to straight up or down, as a fraction of a direction's length, a direction reads
/// neither way.
const VERTICAL: f32 = 1e-4;

/// How far, in points, a flattened path may stray from the path.
const TOLERANCE: f64 = 0.05;

/// How close, in points, two points of a path lie to be one, and how short a segment is to have
/// no direction worth following.
const SAME_POINT: f64 = 1e-3;

/// The cosine of the smallest turn, about a degree, between two of a path's segments that makes a
/// corner, where glyphs turn at once instead of smoothly.
const CORNER_COSINE: f64 = 0.99985;

/// The longest straight piece, in points, that warping a shape keeps, so that a long underline
/// follows the path.
const WARP_STEP: f64 = 1.0;

/// How far, as a fraction of its size, a math glyph's ink may lie across the path from where the
/// bend would put it before the glyph warps through its outline. On a circle about three ems in
/// radius, stretched braces and wide accents stray past it, and a sum sign stays within it.
const STRAY: f64 = 0.03;

/// How far past an end of its path, in points, the text of a label that must fit may reach.
const FIT_SLACK: f64 = 0.5;

impl TextPath {
    /// This path as the engine bends it for a label `width` points wide: reversed when `upright`
    /// reverses the text, with `upright` off, so that labels that differ only in their angles
    /// share their memos.
    pub(crate) fn resolved(&self, width: f32) -> TextPath {
        let reverse = self.upright.is_some_and(|angle| {
            let line = Polyline::new(&self.path);
            let middle = match self.anchor {
                PathAnchor::Start => f64::from(width) / 2.0,
                PathAnchor::Middle => 0.0,
                PathAnchor::End => -f64::from(width) / 2.0,
            };
            let (_, direction) = line.at(line.offset(self.offset) + middle);
            reads_reversed(direction.map(|value| value as f32), angle)
        });
        TextPath {
            path: if reverse { reversed(&self.path) } else { self.path.clone() },
            upright: None,
            ..self.clone()
        }
    }
}

/// The first subpath of `curve`, run from its end to its start. A closed subpath keeps its start
/// and runs around the other way.
fn reversed(curve: &Curve) -> Curve {
    let mut items = curve.0.iter();
    let Some(CurveItem::Move(start)) = items.next().copied() else {
        return curve.clone();
    };
    let mut points = vec![start];
    let mut segments = Vec::new();
    let mut closed = false;
    for item in items {
        match *item {
            CurveItem::Move(_) => break,
            CurveItem::Close => {
                closed = true;
                break;
            }
            CurveItem::Line(to) | CurveItem::Quad(_, to) | CurveItem::Cubic(_, _, to) => {
                segments.push(*item);
                points.push(to);
            }
        }
    }
    let back = |item: CurveItem, to: Point| match item {
        CurveItem::Quad(control, _) => CurveItem::Quad(control, to),
        CurveItem::Cubic(first, second, _) => CurveItem::Cubic(second, first, to),
        _ => CurveItem::Line(to),
    };
    let mut out = Vec::with_capacity(segments.len() + 3);
    if closed {
        out.push(CurveItem::Move(start));
        out.push(CurveItem::Line(points[points.len() - 1]));
    } else {
        out.push(CurveItem::Move(points[points.len() - 1]));
    }
    for (index, segment) in segments.iter().enumerate().rev() {
        out.push(back(*segment, points[index]));
    }
    if closed {
        out.push(CurveItem::Close);
    }
    Curve(out)
}

/// A path's first subpath flattened to points, with the distance along it to each, and whether
/// each point is a corner of the path, where its direction turns at once.
#[derive(Debug, Clone)]
struct Polyline {
    points: Vec<[f64; 2]>,
    lengths: Vec<f64>,
    corners: Vec<bool>,
}

impl Polyline {
    /// The flattened first subpath of `curve`. The points between the path's segments are
    /// corners where the segments' directions there differ. A path with no length is its start,
    /// or the origin, and the text runs from it to the right.
    fn new(curve: &Curve) -> Self {
        let mut points: Vec<[f64; 2]> = Vec::new();
        let mut corners: Vec<bool> = Vec::new();
        let push =
            |point: kurbo::Point, points: &mut Vec<[f64; 2]>, corners: &mut Vec<bool>| {
                let point = [point.x, point.y];
                let repeats = points.last().is_some_and(|last| {
                    (point[0] - last[0]).hypot(point[1] - last[1]) < SAME_POINT
                });
                if !repeats {
                    points.push(point);
                    corners.push(false);
                }
            };
        let mut start: Option<kurbo::Point> = None;
        let mut current = kurbo::Point::ZERO;
        // The direction in which the last segment ends.
        let mut leaving: Option<[f64; 2]> = None;
        for element in bez_path(curve).elements() {
            let segment = match *element {
                PathEl::MoveTo(to) => {
                    if start.is_some() {
                        break;
                    }
                    start = Some(to);
                    current = to;
                    push(to, &mut points, &mut corners);
                    continue;
                }
                PathEl::LineTo(to) => PathSeg::Line(Line::new(current, to)),
                PathEl::QuadTo(control, to) => {
                    PathSeg::Quad(QuadBez::new(current, control, to))
                }
                PathEl::CurveTo(a, b, to) => {
                    PathSeg::Cubic(CubicBez::new(current, a, b, to))
                }
                PathEl::ClosePath => match start {
                    Some(first) if first != current => {
                        PathSeg::Line(Line::new(current, first))
                    }
                    _ => break,
                },
            };
            // A segment too short to have a direction worth following, such as the line that a
            // builder can add to reach an arc's start, drops out.
            let Some((entering, leaves)) = ends(segment) else {
                current = segment.end();
                if matches!(element, PathEl::ClosePath) {
                    break;
                }
                continue;
            };
            if let (Some(before), Some(corner)) = (leaving, corners.last_mut()) {
                let turn = before[0] * entering[0] + before[1] * entering[1];
                *corner |= turn < CORNER_COSINE;
            }
            kurbo::flatten(
                [PathEl::MoveTo(segment.start()), segment.as_path_el()],
                TOLERANCE,
                |element| {
                    if let PathEl::LineTo(to) = element {
                        push(to, &mut points, &mut corners);
                    }
                },
            );
            leaving = Some(leaves);
            current = segment.end();
            if matches!(element, PathEl::ClosePath) {
                break;
            }
        }
        if points.is_empty() {
            points.push([0.0, 0.0]);
            corners.push(false);
        }
        Self::from_points(points, corners)
    }

    fn from_points(points: Vec<[f64; 2]>, corners: Vec<bool>) -> Self {
        let mut lengths = Vec::with_capacity(points.len());
        let mut total = 0.0;
        for (index, point) in points.iter().enumerate() {
            if index > 0 {
                let before = points[index - 1];
                total += (point[0] - before[0]).hypot(point[1] - before[1]);
            }
            lengths.push(total);
        }
        Self { points, lengths, corners }
    }

    fn length(&self) -> f64 {
        self.lengths[self.lengths.len() - 1]
    }

    /// The distance along the path that `offset` names.
    fn offset(&self, offset: PathOffset) -> f64 {
        match offset {
            PathOffset::Fraction(fraction) => f64::from(fraction) * self.length(),
            PathOffset::Distance(distance) => f64::from(distance),
        }
    }

    /// The unit direction of the segment that starts at `index`, or else of the next segment with
    /// length, or else of the last one before it, or to the right for a path with no length.
    fn direction(&self, index: usize) -> [f64; 2] {
        let last = self.points.len().saturating_sub(1);
        let from = index.min(last.saturating_sub(1));
        for segment in (from..last).chain((0..from).rev()) {
            let [a, b] = [self.points[segment], self.points[segment + 1]];
            let length = (b[0] - a[0]).hypot(b[1] - a[1]);
            if length > 0.0 {
                return [(b[0] - a[0]) / length, (b[1] - a[1]) / length];
            }
        }
        [1.0, 0.0]
    }

    /// Where along the points the distance `s` lies: a segment's index and the fraction of the
    /// way along it, past the first or last segment off the path's ends.
    fn locate(&self, s: f64) -> (usize, f64) {
        let segments = self.points.len().saturating_sub(1);
        if segments == 0 {
            return (0, 0.0);
        }
        let index =
            self.lengths.partition_point(|length| *length <= s).clamp(1, segments) - 1;
        let span = self.lengths[index + 1] - self.lengths[index];
        let along = if span > 0.0 { (s - self.lengths[index]) / span } else { 0.0 };
        (index, along)
    }

    /// The path's unit direction at the point `index`, on the segment `segment` that starts or
    /// ends there: the segment's own at a corner or an end of the path, and between its two
    /// segments' elsewhere, as the path's tangent lies between two chords of a curve.
    fn tangent_at(&self, index: usize, segment: usize) -> [f64; 2] {
        let last = self.points.len() - 1;
        if index == 0 || index == last || self.corners[index] {
            return self.direction(segment);
        }
        let [before, after] = [self.direction(index - 1), self.direction(index)];
        let sum = [before[0] + after[0], before[1] + after[1]];
        let length = sum[0].hypot(sum[1]);
        if length > 1e-12 {
            [sum[0] / length, sum[1] / length]
        } else {
            self.direction(segment)
        }
    }

    /// The point at the distance `s` along the path, and the path's unit direction there,
    /// which turns smoothly between the flattened points of a curve and at once at a corner.
    /// Off the path's ends, the point lies along the direction at the nearer end.
    fn at(&self, s: f64) -> ([f64; 2], [f64; 2]) {
        let total = self.length();
        let last = self.points.len() - 1;
        if s <= 0.0 || last == 0 {
            let direction = self.direction(0);
            let start = self.points[0];
            return (
                [start[0] + direction[0] * s, start[1] + direction[1] * s],
                direction,
            );
        }
        if s >= total {
            let direction = self.direction(last);
            let end = self.points[last];
            let past = s - total;
            return (
                [end[0] + direction[0] * past, end[1] + direction[1] * past],
                direction,
            );
        }
        let (index, along) = self.locate(s);
        let [a, b] = [self.points[index], self.points[index + 1]];
        let point = [a[0] + (b[0] - a[0]) * along, a[1] + (b[1] - a[1]) * along];
        let [from, to] =
            [self.tangent_at(index, index), self.tangent_at(index + 1, index)];
        let blend =
            [from[0] + (to[0] - from[0]) * along, from[1] + (to[1] - from[1]) * along];
        let length = blend[0].hypot(blend[1]);
        let direction = if length > 1e-12 {
            [blend[0] / length, blend[1] / length]
        } else {
            self.direction(index)
        };
        (point, direction)
    }

    /// The path as a curve through its points that a viewer follows as the bend does: a cubic
    /// segment between each two points that leaves and reaches them in the path's directions
    /// there, so a straight segment stays straight, with straight runs `before` and `after` its
    /// ends.
    fn curve(&self, before: f64, after: f64) -> Curve {
        let last = self.points.len() - 1;
        let point = |[x, y]: [f64; 2]| Point::new(x as f32, y as f32);
        let along = |from: [f64; 2], direction: [f64; 2], distance: f64| {
            [from[0] + direction[0] * distance, from[1] + direction[1] * distance]
        };
        let first = self.points[0];
        let mut items = vec![
            CurveItem::Move(point(along(first, self.direction(0), -before))),
            CurveItem::Line(point(first)),
        ];
        for index in 0..last {
            let [a, b] = [self.points[index], self.points[index + 1]];
            let third = (b[0] - a[0]).hypot(b[1] - a[1]) / 3.0;
            let [leave, reach] =
                [self.tangent_at(index, index), self.tangent_at(index + 1, index)];
            items.push(CurveItem::Cubic(
                point(along(a, leave, third)),
                point(along(b, reach, -third)),
                point(b),
            ));
        }
        items.push(CurveItem::Line(point(along(
            self.points[last],
            self.direction(last),
            after,
        ))));
        Curve(items)
    }

    /// The path moved `distance` toward the text's top, the left of its direction on screen,
    /// with mitered corners and without folds, and the distance along it that corresponds to
    /// `s` on this path.
    fn shifted(&self, distance: f64, s: f64) -> (Polyline, f64) {
        let count = self.points.len();
        let normal = |direction: [f64; 2]| [direction[1], -direction[0]];
        let mut points: Vec<[f64; 2]> = (0..count)
            .map(|index| {
                let after = normal(self.direction(index));
                let miter = if index == 0 || index + 1 == count {
                    after
                } else {
                    let before = normal(self.direction(index - 1));
                    let sum = [before[0] + after[0], before[1] + after[1]];
                    let scale = 1.0 + before[0] * after[0] + before[1] * after[1];
                    // A turn of nearly half a circle has no useful miter.
                    if scale > 0.1 { [sum[0] / scale, sum[1] / scale] } else { after }
                };
                let point = self.points[index];
                [point[0] + miter[0] * distance, point[1] + miter[1] * distance]
            })
            .collect();
        // A segment shorter than the move between its ends' miters would run backward, as a
        // builder's short line to an arc's start can, so its end stays at its start instead.
        for index in 1..count {
            let direction = self.direction(index - 1);
            let [before, after] = [points[index - 1], points[index]];
            let along = (after[0] - before[0]) * direction[0]
                + (after[1] - before[1]) * direction[1];
            if along < 0.0 {
                points[index] = before;
            }
        }
        let shifted = Polyline::from_points(points, self.corners.clone());
        let total = self.length();
        let corresponding = if s <= 0.0 || count == 1 {
            s
        } else if s >= total {
            shifted.length() + (s - total)
        } else {
            let (index, along) = self.locate(s);
            let [from, to] = [shifted.lengths[index], shifted.lengths[index + 1]];
            from + (to - from) * along
        };
        (shifted, corresponding)
    }
}

/// The unit directions in which `segment` starts and ends, by its first and last control points
/// that differ from its ends, or `None` for a segment whose control points and end lie within
/// [`SAME_POINT`] of its start, too short to have a direction worth following.
fn ends(segment: PathSeg) -> Option<([f64; 2], [f64; 2])> {
    let points: Vec<kurbo::Point> = match segment {
        PathSeg::Line(line) => vec![line.p0, line.p1],
        PathSeg::Quad(quad) => vec![quad.p0, quad.p1, quad.p2],
        PathSeg::Cubic(cubic) => vec![cubic.p0, cubic.p1, cubic.p2, cubic.p3],
    };
    let [first, last] = [points[0], points[points.len() - 1]];
    if points.iter().all(|point| (*point - first).hypot() < SAME_POINT) {
        return None;
    }
    let unit = |from: kurbo::Point, to: kurbo::Point| {
        let length = (to - from).hypot();
        (length > 1e-12).then(|| [(to.x - from.x) / length, (to.y - from.y) / length])
    };
    let start = points[1..].iter().find_map(|point| unit(first, *point))?;
    let end = points[..points.len() - 1]
        .iter()
        .rev()
        .find_map(|point| unit(*point, last))?;
    Some((start, end))
}

/// How a label lies along its path: the path's origin, and the lines and runs along which SVG can
/// draw the label's text. A label that lays out nothing has neither lines nor runs.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BentText {
    /// Where the origin of the path lies from the top left of the bent frame, which is the
    /// label's box.
    pub(crate) origin: Point,
    /// Each line's baseline as a curve that a viewer follows as the bend does, extended past the
    /// path's ends by as far as the text reaches, and the distance along it to the point of the
    /// text that lies at the path's offset.
    pub(crate) lines: Vec<(Curve, f32)>,
    /// The point of the straight box's width that lies at the path's offset.
    pub(crate) anchor_x: f32,
    /// The text items that stay text, on their lines' baselines.
    pub(crate) runs: Vec<BentRun>,
}

/// A run of straight text in a bent label, on the baseline of one of its lines.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BentRun {
    /// The straight item, and the transform from its coordinates to the straight box.
    pub(crate) item: TextItem,
    pub(crate) ts: Transform,
    /// The line whose baseline the run lies on.
    pub(crate) line: usize,
    /// The bent frame's text items that draw the run's clusters, as
    /// [`text_items`](LabelFrame::text_items) counts them.
    pub(crate) texts: Range<usize>,
}

/// A line of a label: the path offset to its baseline, and where the text's anchor lies along it.
struct LineCourse {
    /// The line's baseline in the label's box, in points from its top.
    baseline: f64,
    /// The line's box in the label's box, padded to at least the font size: its top and bottom.
    band: [f64; 2],
    path: Polyline,
    anchor: f64,
}

/// What maps a straight label's box onto its path.
struct Bend {
    lines: Vec<LineCourse>,
    /// The point of the box's width that lies at the anchor.
    anchor_x: f64,
}

impl Bend {
    /// The index of the line whose baseline lies nearest `y` in the box.
    fn nearest(&self, y: f64) -> usize {
        (0..self.lines.len())
            .min_by(|a, b| {
                let [a, b] = [a, b].map(|index| (self.lines[*index].baseline - y).abs());
                a.total_cmp(&b)
            })
            .expect("a label has a line")
    }

    /// The line whose baseline lies nearest `y` in the box.
    fn line(&self, y: f64) -> &LineCourse {
        &self.lines[self.nearest(y)]
    }

    /// Where the point `(x, y)` of the box lies on its line's path, and the path's direction
    /// there: along the line's offset path by its distance from the anchor, and off it by its
    /// height over the line's baseline.
    fn at(&self, line: &LineCourse, [x, y]: [f64; 2]) -> ([f64; 2], [f64; 2]) {
        let (point, direction) = line.path.at(line.anchor + x - self.anchor_x);
        let above = line.baseline - y;
        let point = [point[0] + direction[1] * above, point[1] - direction[0] * above];
        (point, direction)
    }
}

/// `compiled`, laid out straight at `font_size`, bent along `path`, whose upright test the
/// engine has resolved. A label that must fit and reaches past its path's ends lays out nothing.
pub(crate) fn bend(
    compiled: &CompiledLabel,
    path: &TextPath,
    font_size: f32,
) -> CompiledLabel {
    let straight = LineBox::new(&compiled.metrics, font_size);
    // The frame's top lies this far below the box's top.
    let top = f64::from(straight.top);
    let size = f64::from(font_size.max(1.0));
    let [width, height] = [f64::from(compiled.metrics.width), f64::from(straight.height)];
    let leading = f64::from(straight.leading);
    let reference = match path.baseline {
        PathBaseline::Alphabetic => f64::from(straight.ascent),
        PathBaseline::Top => 0.0,
        PathBaseline::Middle => height / 2.0,
        PathBaseline::Bottom => height,
        PathBaseline::LineTop => -leading / 2.0,
        PathBaseline::LineBottom => height + leading / 2.0,
    };
    let side = match path.baseline {
        PathBaseline::Top | PathBaseline::LineTop => -1.0,
        _ => 1.0,
    };
    let shift = side * f64::from(path.shift);
    let given = Polyline::new(&path.path);
    let offset = given.offset(path.offset);
    // The metrics' lines run top to bottom, and lines that share a baseline share its course.
    let mut lines: Vec<LineCourse> = compiled
        .metrics
        .lines
        .iter()
        .map(|line| {
            let baseline = f64::from(line.baseline) + top;
            let pad = (size - f64::from(line.bottom - line.top)).max(0.0) / 2.0;
            let band =
                [f64::from(line.top) + top - pad, f64::from(line.bottom) + top + pad];
            let (path, anchor) = given.shifted(reference - baseline + shift, offset);
            LineCourse { baseline, band, path, anchor }
        })
        .collect();
    lines.dedup_by(|a, b| a.baseline == b.baseline);
    let bend = Bend {
        lines,
        anchor_x: match path.anchor {
            PathAnchor::Start => 0.0,
            PathAnchor::Middle => width / 2.0,
            PathAnchor::End => width,
        },
    };

    let mut items: Vec<FrameItem> = Vec::new();
    let mut runs: Vec<BentRun> = Vec::new();
    let mut texts = 0;
    let mut extent = Extent::default();
    let mut fits = true;
    let fit = |line: &LineCourse, x: f64| {
        let s = line.anchor + x - bend.anchor_x;
        (-FIT_SLACK..=line.path.length() + FIT_SLACK).contains(&s)
    };
    compiled.frame.visit(
        Transform::translate(0.0, top as f32),
        &mut |ts, item| match item {
            FrameItem::Text(text) => {
                let first = texts;
                let math = text.font.0.font().info().flags.contains(FontFlags::MATH);
                // Whether every cluster stays text.
                let mut whole = true;
                for cluster in clusters(text) {
                    let start =
                        ts.pre_concat(Transform::translate(cluster.pen.x, cluster.pen.y));
                    let [p0, p1, ahead] = [
                        start.apply(Point::ZERO),
                        start.apply(Point::new(cluster.width, 0.0)),
                        start.apply(Point::new(1.0, 0.0)),
                    ]
                    .map(|point| [f64::from(point.x), f64::from(point.y)]);
                    let line = bend.line((p0[1] + p1[1]) / 2.0);
                    fits &= fit(line, p0[0]) && fit(line, p1[0]);
                    let middle = [(p0[0] + p1[0]) / 2.0, (p0[1] + p1[1]) / 2.0];
                    let (to, direction) = bend.at(line, middle);
                    let placed = turned(
                        middle,
                        [ahead[0] - p0[0], ahead[1] - p0[1]],
                        to,
                        direction,
                    );
                    let outline = math
                        .then(|| stray_outline(&cluster.item, start, placed, line, &bend))
                        .flatten();
                    if let Some(outline) = outline {
                        let curve = bent(&outline, Some(line), &bend, 0.0, &mut extent);
                        items.push(FrameItem::Shape(Shape {
                            geometry: Geometry::Curve(curve),
                            fill: Some(text.fill),
                            fill_rule: FillRule::NonZero,
                            stroke: None,
                        }));
                        whole = false;
                        continue;
                    }
                    for y in line.band {
                        for x in [p0[0], p1[0]] {
                            extent.add(apply(placed, [x, y]));
                        }
                    }
                    items.push(FrameItem::Group(GroupItem {
                        items: vec![(Point::ZERO, FrameItem::Text(cluster.item))],
                        transform: placed.pre_concat(start),
                    }));
                    texts += 1;
                }
                // A run that stays text and lies on its line's baseline can draw as text along
                // the line.
                let line = bend.nearest(f64::from(ts.ty));
                if whole && (bend.lines[line].baseline - f64::from(ts.ty)).abs() < 1e-3 {
                    runs.push(BentRun {
                        item: text.clone(),
                        ts,
                        line,
                        texts: first..texts,
                    });
                }
            }
            FrameItem::Shape(shape) => {
                let warped = warp(shape, ts, &bend, &mut extent);
                items.push(FrameItem::Shape(warped));
            }
            FrameItem::Group(_) => {}
        },
    );

    let empty = (path.overflow == PathOverflow::Fit && !fits) || extent.is_empty();
    if empty {
        return CompiledLabel {
            frame: LabelFrame { size: Size::default(), items: Vec::new() },
            bent: Some(Arc::new(BentText {
                origin: Point::ZERO,
                lines: Vec::new(),
                anchor_x: bend.anchor_x as f32,
                runs: Vec::new(),
            })),
            ..compiled.clone()
        };
    }
    let [left, top] = extent.min;
    let back = Transform::translate(-left as f32, -top as f32);
    let items = items
        .into_iter()
        .map(|item| (Point::ZERO, moved(item, back)))
        .collect();
    let size = Size::new((extent.max[0] - left) as f32, (extent.max[1] - top) as f32);
    // Each line's baseline, extended past the path's ends by as far as the text reaches.
    let lines = bend
        .lines
        .iter()
        .map(|line| {
            let start = line.anchor - bend.anchor_x;
            let before = (-start).max(0.0) + 1.0;
            let after = (start + width - line.path.length()).max(0.0) + 1.0;
            let curve = translated(&line.path.curve(before, after), back);
            (curve, (line.anchor + before) as f32)
        })
        .collect();
    CompiledLabel {
        frame: LabelFrame { size, items },
        bent: Some(Arc::new(BentText {
            origin: Point::new(-left as f32, -top as f32),
            lines,
            anchor_x: bend.anchor_x as f32,
            runs,
        })),
        ..compiled.clone()
    }
}

/// A run of a text item's glyphs that share a cluster: as its own item, the pen position where
/// it starts in the item, and its advance.
struct Cluster {
    item: TextItem,
    pen: Point,
    width: f32,
}

/// The clusters of `text`, in visual order, each as an item of its own with its glyphs' ranges
/// in the cluster's text.
fn clusters(text: &TextItem) -> Vec<Cluster> {
    let mut clusters: Vec<Cluster> = Vec::new();
    let mut pen = Point::ZERO;
    let mut index = 0;
    while index < text.glyphs.len() {
        let range = text.glyphs[index].range.clone();
        let end = index
            + text.glyphs[index..]
                .iter()
                .take_while(|glyph| glyph.range == range)
                .count();
        let glyphs: Vec<_> = text.glyphs[index..end]
            .iter()
            .map(|glyph| {
                let mut glyph = glyph.clone();
                glyph.range =
                    glyph.range.start - range.start..glyph.range.end - range.start;
                glyph
            })
            .collect();
        let width = glyphs.iter().map(|glyph| glyph.x_advance).sum::<f32>() * text.size;
        let rise = glyphs.iter().map(|glyph| glyph.y_advance).sum::<f32>() * text.size;
        let sources = &text.glyphs[index..end];
        let source = sources.iter().map(|glyph| glyph.source.start).min().unwrap_or(0)
            ..sources.iter().map(|glyph| glyph.source.end).max().unwrap_or(0);
        clusters.push(Cluster {
            item: TextItem {
                font: text.font.clone(),
                size: text.size,
                fill: text.fill,
                text: text.text.get(range.clone()).unwrap_or_default().to_string(),
                glyphs,
                source,
            },
            pen,
            width,
        });
        pen = Point::new(pen.x + width, pen.y - rise);
        index = end;
    }
    clusters
}

/// The rigid motion that takes a cluster, whose baseline runs along `straight` through `middle`,
/// the middle of its advance, in the straight box, to `to` on the path, turned to `direction`.
fn turned(
    middle: [f64; 2],
    straight: [f64; 2],
    to: [f64; 2],
    direction: [f64; 2],
) -> Transform {
    let turn = direction[1].atan2(direction[0]) - straight[1].atan2(straight[0]);
    let (sin, cos) = turn.sin_cos();
    Transform::translate(to[0] as f32, to[1] as f32)
        .pre_concat(Transform {
            sx: cos as f32,
            ky: sin as f32,
            kx: -sin as f32,
            sy: cos as f32,
            tx: 0.0,
            ty: 0.0,
        })
        .pre_concat(Transform::translate(-middle[0] as f32, -middle[1] as f32))
}

/// The outline of `cluster`, which `start` draws in the straight box, as contours there, if the
/// rigid motion `placed` would set a corner of its ink farther across the path than [`STRAY`] of
/// its size from where the bend along `line` puts that corner, and every glyph of it has an
/// outline. Distances along the path don't count, since the bend stretches or squeezes text along
/// the path by its height over the baseline.
fn stray_outline(
    cluster: &TextItem,
    start: Transform,
    placed: Transform,
    line: &LineCourse,
    bend: &Bend,
) -> Option<Contours> {
    let mut path = BezPath::new();
    for (origin, glyph) in cluster.positioned_glyphs() {
        path.extend(glyph_path(cluster, origin, glyph)?.elements().iter().copied());
    }
    let outline = contours(&path, start);
    let mut ink = Extent::default();
    for point in outline.iter().flat_map(|(points, _)| points) {
        ink.add(*point);
    }
    if ink.is_empty() {
        return None;
    }
    let [min, max] = [ink.min, ink.max];
    let stray = [min, [max[0], min[1]], max, [min[0], max[1]]]
        .into_iter()
        .map(|corner| {
            let rigid = apply(placed, corner);
            let (bent, direction) = bend.at(line, corner);
            ((rigid[0] - bent[0]) * direction[1] - (rigid[1] - bent[1]) * direction[0])
                .abs()
        })
        .fold(0.0, f64::max);
    (stray > STRAY * f64::from(cluster.size)).then_some(outline)
}

fn apply(transform: Transform, [x, y]: [f64; 2]) -> [f64; 2] {
    let point = transform.apply(Point::new(x as f32, y as f32));
    [f64::from(point.x), f64::from(point.y)]
}

/// `shape`, drawn through `ts` in the straight box, with each point mapped onto the path, as a
/// curve of short lines in the bent label.
fn warp(shape: &Shape, ts: Transform, bend: &Bend, extent: &mut Extent) -> Shape {
    let curve = match &shape.geometry {
        Geometry::Line(to) => {
            Curve(vec![CurveItem::Move(Point::ZERO), CurveItem::Line(*to)])
        }
        Geometry::Rect(size) => Curve(vec![
            CurveItem::Move(Point::ZERO),
            CurveItem::Line(Point::new(size.x, 0.0)),
            CurveItem::Line(Point::new(size.x, size.y)),
            CurveItem::Line(Point::new(0.0, size.y)),
            CurveItem::Close,
        ]),
        Geometry::Curve(curve) => curve.clone(),
    };
    let reach = shape
        .stroke
        .as_ref()
        .map_or(0.0, |stroke| f64::from(stroke.thickness) / 2.0);
    let curve = bent(&contours(&bez_path(&curve), ts), None, bend, reach, extent);
    Shape { geometry: Geometry::Curve(curve), ..shape.clone() }
}

/// The contours of a path in the straight box, flattened: each one's points, and whether it
/// closes.
type Contours = Vec<(Vec<[f64; 2]>, bool)>;

/// The contours of `path`, drawn through `ts` in the straight box.
fn contours(path: &BezPath, ts: Transform) -> Contours {
    let mut contours: Contours = Vec::new();
    let place = |point: kurbo::Point| {
        let point = ts.apply(Point::new(point.x as f32, point.y as f32));
        [f64::from(point.x), f64::from(point.y)]
    };
    kurbo::flatten(path, TOLERANCE, |element| match element {
        PathEl::MoveTo(to) => contours.push((vec![place(to)], false)),
        PathEl::LineTo(to) => {
            if let Some((points, _)) = contours.last_mut() {
                points.push(place(to));
            }
        }
        PathEl::ClosePath => {
            if let Some((points, closed)) = contours.last_mut() {
                *closed = true;
                if let Some(first) = points.first().copied() {
                    points.push(first);
                }
            }
        }
        _ => {}
    });
    contours
}

/// `contours` with each point mapped onto the path, along `line`, or else along the line nearest
/// each contour's middle, as a curve of short lines in the bent label. `extent` takes the points
/// grown by `reach`, how far a stroke reaches past them.
fn bent(
    contours: &Contours,
    line: Option<&LineCourse>,
    bend: &Bend,
    reach: f64,
    extent: &mut Extent,
) -> Curve {
    let mut items = Vec::new();
    for (points, closed) in contours {
        let line = line.unwrap_or_else(|| {
            let middle =
                points.iter().map(|point| point[1]).sum::<f64>() / points.len() as f64;
            bend.line(middle)
        });
        let mut warped: Vec<[f64; 2]> = Vec::new();
        for (index, point) in points.iter().enumerate() {
            if index > 0 {
                let before = points[index - 1];
                let steps = ((point[0] - before[0]).hypot(point[1] - before[1])
                    / WARP_STEP)
                    .ceil()
                    .max(1.0) as usize;
                for step in 1..steps {
                    let along = step as f64 / steps as f64;
                    let between = [
                        before[0] + (point[0] - before[0]) * along,
                        before[1] + (point[1] - before[1]) * along,
                    ];
                    warped.push(bend.at(line, between).0);
                }
            }
            warped.push(bend.at(line, *point).0);
        }
        if *closed {
            warped.pop();
        }
        for point in &warped {
            extent.add([point[0] - reach, point[1] - reach]);
            extent.add([point[0] + reach, point[1] + reach]);
        }
        let point = |[x, y]: [f64; 2]| Point::new(x as f32, y as f32);
        for (index, warped) in warped.iter().enumerate() {
            items.push(if index == 0 {
                CurveItem::Move(point(*warped))
            } else {
                CurveItem::Line(point(*warped))
            });
        }
        if *closed {
            items.push(CurveItem::Close);
        }
    }
    Curve(items)
}

/// `item`, moved by `back` in the bent label.
fn moved(item: FrameItem, back: Transform) -> FrameItem {
    match item {
        FrameItem::Group(group) => FrameItem::Group(GroupItem {
            transform: back.pre_concat(group.transform),
            ..group
        }),
        FrameItem::Shape(shape) => match shape.geometry {
            Geometry::Curve(curve) => FrameItem::Shape(Shape {
                geometry: Geometry::Curve(translated(&curve, back)),
                ..shape
            }),
            _ => FrameItem::Shape(shape),
        },
        FrameItem::Text(text) => FrameItem::Text(text),
    }
}

/// `curve` with its points moved by `transform`.
pub(crate) fn translated(curve: &Curve, transform: Transform) -> Curve {
    let point = |point: Point| transform.apply(point);
    Curve(
        curve
            .0
            .iter()
            .map(|item| match *item {
                CurveItem::Move(to) => CurveItem::Move(point(to)),
                CurveItem::Line(to) => CurveItem::Line(point(to)),
                CurveItem::Quad(control, to) => {
                    CurveItem::Quad(point(control), point(to))
                }
                CurveItem::Cubic(a, b, to) => {
                    CurveItem::Cubic(point(a), point(b), point(to))
                }
                CurveItem::Close => CurveItem::Close,
            })
            .collect(),
    )
}

/// The smallest box around points.
struct Extent {
    min: [f64; 2],
    max: [f64; 2],
}

impl Default for Extent {
    fn default() -> Self {
        Self {
            min: [f64::INFINITY; 2],
            max: [f64::NEG_INFINITY; 2],
        }
    }
}

impl Extent {
    fn add(&mut self, [x, y]: [f64; 2]) {
        if x.is_finite() && y.is_finite() {
            self.min = [self.min[0].min(x), self.min[1].min(y)];
            self.max = [self.max[0].max(x), self.max[1].max(y)];
        }
    }

    fn is_empty(&self) -> bool {
        !(self.min[0] <= self.max[0] && self.min[1] <= self.max[1])
    }
}

/// What a path's memos key on: all of it, with its numbers as bits.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PathKey {
    curve: Vec<(u8, [u32; 6])>,
    offset: (u8, u32),
    anchor: PathAnchor,
    baseline: PathBaseline,
    shift: u32,
    upright: Option<u32>,
    overflow: PathOverflow,
}

impl PathKey {
    pub(crate) fn new(path: &TextPath) -> Self {
        // Destructured in full, so that a new field can't be left out of the key.
        let TextPath {
            path: curve,
            offset,
            anchor,
            baseline,
            shift,
            upright,
            overflow,
        } = path;
        let bits = |points: &[Point]| {
            let mut out = [0_u32; 6];
            for (index, point) in points.iter().enumerate() {
                out[2 * index] = point.x.to_bits();
                out[2 * index + 1] = point.y.to_bits();
            }
            out
        };
        Self {
            curve: curve
                .0
                .iter()
                .map(|item| match *item {
                    CurveItem::Move(to) => (0, bits(&[to])),
                    CurveItem::Line(to) => (1, bits(&[to])),
                    CurveItem::Quad(control, to) => (2, bits(&[control, to])),
                    CurveItem::Cubic(a, b, to) => (3, bits(&[a, b, to])),
                    CurveItem::Close => (4, [0; 6]),
                })
                .collect(),
            offset: match *offset {
                PathOffset::Fraction(fraction) => (0, fraction.to_bits()),
                PathOffset::Distance(distance) => (1, distance.to_bits()),
            },
            anchor: *anchor,
            baseline: *baseline,
            shift: shift.to_bits(),
            upright: upright.map(f32::to_bits),
            overflow: *overflow,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(points: &[[f32; 2]]) -> Curve {
        let mut items = vec![CurveItem::Move(Point::new(points[0][0], points[0][1]))];
        items
            .extend(points[1..].iter().map(|[x, y]| CurveItem::Line(Point::new(*x, *y))));
        Curve(items)
    }

    /// The rule reads a direction by where it points once turned: left reads upside down, and
    /// straight up or down reads either way.
    #[test]
    fn text_reads_reversed_when_it_points_left() {
        assert!(!reads_reversed([1.0, 0.0], 0.0));
        assert!(reads_reversed([-1.0, 0.0], 0.0));
        assert!(reads_reversed([1.0, 0.0], 120.0));
        assert!(!reads_reversed([1.0, 0.0], 60.0));
        assert!(!reads_reversed([1.0, 0.0], 90.0));
        assert!(!reads_reversed([1.0, 0.0], 270.0));
    }

    /// Reversing runs the first subpath backward: an open one from its end, and a closed one
    /// around the other way from its start, with each curve's control points swapped.
    #[test]
    fn paths_reverse_from_their_other_end() {
        let open = Curve(vec![
            CurveItem::Move(Point::new(0.0, 0.0)),
            CurveItem::Line(Point::new(10.0, 0.0)),
            CurveItem::Cubic(
                Point::new(12.0, 0.0),
                Point::new(14.0, 2.0),
                Point::new(14.0, 4.0),
            ),
        ]);
        assert_eq!(
            reversed(&open).0,
            [
                CurveItem::Move(Point::new(14.0, 4.0)),
                CurveItem::Cubic(
                    Point::new(14.0, 2.0),
                    Point::new(12.0, 0.0),
                    Point::new(10.0, 0.0)
                ),
                CurveItem::Line(Point::new(0.0, 0.0)),
            ]
        );
        let mut closed = line(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]);
        closed.0.push(CurveItem::Close);
        assert_eq!(
            reversed(&closed).0,
            [
                CurveItem::Move(Point::new(0.0, 0.0)),
                CurveItem::Line(Point::new(10.0, 10.0)),
                CurveItem::Line(Point::new(10.0, 0.0)),
                CurveItem::Line(Point::new(0.0, 0.0)),
                CurveItem::Close,
            ]
        );
    }

    /// A path moved toward the text's top keeps its corners mitered, and the distance at a
    /// point of the path corresponds to the same point of the moved path.
    #[test]
    fn shifted_paths_keep_their_corners() {
        let path = Polyline::new(&line(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]));
        let (shifted, s) = path.shifted(2.0, 10.0);
        // Up is toward the top of text that runs right, and the corner moves up and right.
        assert_eq!(shifted.points, [[0.0, -2.0], [12.0, -2.0], [12.0, 10.0]]);
        assert!((s - 12.0).abs() < 1e-9);
        // Past the end, the distance runs on along the last direction.
        let (_, past) = path.shifted(2.0, 25.0);
        assert!((past - 29.0).abs() < 1e-9);
    }
}
