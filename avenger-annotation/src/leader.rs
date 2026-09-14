//! Leaders: lines from labels to the points they annotate.
//!
//! [`leader`] builds one label's leader from the label's measured box, for code that places
//! labels and tests leaders before choosing them. [`make_leader_marks`] draws chosen leaders as
//! path marks, and [`make_text_leaders`] measures a text mark's labels and does both.

use avenger_color::ColorOrGradient;
use avenger_common::types::{StrokeCap, StrokeJoin, TextAlign, TextBaseline};
use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_scenegraph::marks::path::ScenePathMark;
use avenger_scenegraph::marks::stroke_dash::dash_paths;
use avenger_scenegraph::marks::text::{text_origin, SceneTextMark};
use avenger_typst_label::{LabelEngine, TextBounds};
use lyon_path::{math::point, Path};

use crate::error::AvengerAnnotationError;

const EPSILON: f32 = 1.0e-5;
const DEGENERATE_SEGMENT: f32 = 0.5;

/// How leaders look.
#[derive(Debug, Clone, PartialEq)]
pub struct LeaderStyle {
    /// Whether each leader is straight, elbowed or curved.
    pub shape: LeaderShape,
    /// The arrowhead at each leader's target, if any.
    pub arrow: LeaderArrow,
    /// Each arrowhead's length.
    pub arrow_length: f32,
    /// Each arrowhead's width.
    pub arrow_width: f32,
    /// The leaders' color.
    pub stroke: [f32; 4],
    /// The leaders' stroke width.
    pub stroke_width: f32,
    /// The leaders' line caps.
    pub stroke_cap: StrokeCap,
    /// The leaders' line joins.
    pub stroke_join: StrokeJoin,
    /// The leaders' dash pattern, or a solid line.
    pub stroke_dash: Option<Vec<f32>>,
    /// The gap between each label's box and its leader.
    pub label_padding: f32,
    /// The shortest leader that draws: a shorter one is left out.
    pub min_length: f32,
}

impl Default for LeaderStyle {
    fn default() -> Self {
        Self {
            shape: LeaderShape::Straight,
            arrow: LeaderArrow::None,
            arrow_length: 6.0,
            arrow_width: 5.0,
            stroke: [0.0, 0.0, 0.0, 0.7],
            stroke_width: 1.0,
            stroke_cap: StrokeCap::Round,
            stroke_join: StrokeJoin::Round,
            stroke_dash: None,
            label_padding: 2.0,
            min_length: 1.0,
        }
    }
}

/// The shape of a leader.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum LeaderShape {
    /// A straight line from the label's box.
    #[default]
    Straight,
    /// A line out of the middle of the label's side that turns toward the target.
    Elbow,
    /// A curve out of the middle of the label's side.
    Curved,
}

/// The arrowhead at a leader's target.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum LeaderArrow {
    /// No arrowhead.
    #[default]
    None,
    /// Two strokes that meet at the tip.
    Open,
    /// A filled triangle.
    Triangle,
}

/// A measured label at the position its text mark gives it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacedLabel {
    /// The label's box, as `LabelEngine::bounds` measures it.
    pub bounds: TextBounds,
    /// The point the label is placed at.
    pub position: [f32; 2],
    /// How the label's box lines up horizontally with its position.
    pub align: TextAlign,
    /// How the label's box lines up vertically with its position.
    pub baseline: TextBaseline,
    /// The label's rotation around its position, in degrees.
    pub angle: f32,
}

impl PlacedLabel {
    /// The corners of the label's box grown by `padding` on each side, in scene coordinates:
    /// the top left, top right, bottom right and bottom left corners before rotation.
    pub fn corners(&self, padding: f32) -> [[f32; 2]; 4] {
        let [left, top, right, bottom] = self.padded_box(padding);
        let angle = self.angle.to_radians();
        [[left, top], [right, top], [right, bottom], [left, bottom]]
            .map(|corner| rotate_around(corner, self.position, angle))
    }

    /// The label's box grown by `padding`, before rotation: `[left, top, right, bottom]`.
    fn padded_box(&self, padding: f32) -> [f32; 4] {
        let origin = text_origin(&self.bounds, self.position, self.align, self.baseline);
        [
            origin[0] - padding,
            origin[1] - padding,
            origin[0] + self.bounds.width + padding,
            origin[1] + self.bounds.height + padding,
        ]
    }
}

/// The point a leader aims at, and how far short of it the leader stops.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LeaderTarget {
    /// The point, such as a symbol's center.
    pub position: [f32; 2],
    /// How far short of the point the leader stops, such as the symbol's radius.
    pub radius: f32,
}

/// A leader, in scene coordinates.
#[derive(Debug, Clone)]
pub struct Leader {
    /// The leader's line, undashed.
    pub spine: Path,
    /// The arrowhead's tip and the two corners of its base, when the style has an arrowhead.
    pub arrowhead: Option<[[f32; 2]; 3]>,
}

/// The leader from `label` to `target`, or `None` when the target is inside the label's box
/// grown by `style.label_padding`, or too near it.
///
/// The leader runs from that box toward the target's position and stops `target.radius` short
/// of it, so an arrowhead points at the position. A straight leader starts where the line from
/// the box's center to the target crosses the box, and elbow and curved leaders start from the
/// middle of that side.
pub fn leader(label: &PlacedLabel, target: LeaderTarget, style: &LeaderStyle) -> Option<Leader> {
    leader_geometry(label, target, style).map(|geometry| Leader {
        spine: geometry.spine.to_path(),
        arrowhead: geometry.arrowhead,
    })
}

/// Draws `leaders`, `leaders[i]` for label `i` of `labels`, with the style they were built with.
///
/// The group holds a path mark of leader lines, with the style's dashes and open arrowheads,
/// and, for triangle arrowheads, a path mark of arrowheads. Both take the labels' name and
/// `interactive` flag, and instance `i` of each belongs to label `i`, so a pick on a leader
/// reports its label. A label without a leader has an empty path.
pub fn make_leader_marks(
    labels: &SceneTextMark,
    leaders: &[Option<Leader>],
    style: &LeaderStyle,
) -> Result<SceneGroup, AvengerAnnotationError> {
    let len = labels.len as usize;
    if leaders.len() != len {
        return Err(AvengerAnnotationError::LeaderCount {
            labels: len,
            leaders: leaders.len(),
        });
    }

    let mut lines = vec![Path::new(); len];
    let mut heads = vec![Path::new(); len];
    for (index, leader) in leaders.iter().enumerate() {
        let Some(leader) = leader else {
            continue;
        };
        let spine = match &style.stroke_dash {
            Some(dash) => dash_paths(std::iter::once(&leader.spine), dash),
            None => leader.spine.clone(),
        };
        lines[index] = match (style.arrow, leader.arrowhead) {
            // An open arrowhead's strokes join the line's, undashed.
            (LeaderArrow::Open, Some([tip, left, right])) => {
                let mut builder = Path::builder();
                builder.extend_from_paths(&[spine.as_slice()]);
                for corner in [left, right] {
                    builder.begin(point(corner[0], corner[1]));
                    builder.line_to(point(tip[0], tip[1]));
                    builder.end(false);
                }
                builder.build()
            }
            (LeaderArrow::Triangle, Some(points)) => {
                heads[index] = polygon(&points);
                spine
            }
            _ => spine,
        };
    }

    let color = |rgba: [f32; 4]| ScalarOrArray::new_scalar(ColorOrGradient::Color(rgba));
    let mut marks = vec![ScenePathMark {
        name: labels.name.clone(),
        interactive: labels.interactive,
        clip: labels.clip,
        len: labels.len,
        stroke_cap: style.stroke_cap,
        stroke_join: style.stroke_join,
        stroke_width: Some(style.stroke_width),
        path: ScalarOrArray::new_array(lines),
        fill: color([0.0; 4]),
        stroke: color(style.stroke),
        ..Default::default()
    }
    .into()];
    if style.arrow == LeaderArrow::Triangle {
        marks.push(
            ScenePathMark {
                name: labels.name.clone(),
                interactive: labels.interactive,
                clip: labels.clip,
                len: labels.len,
                stroke_width: None,
                path: ScalarOrArray::new_array(heads),
                fill: color(style.stroke),
                stroke: color([0.0; 4]),
                ..Default::default()
            }
            .into(),
        );
    }
    Ok(SceneGroup {
        name: labels.name.clone(),
        marks,
        zindex: labels.zindex,
        ..Default::default()
    })
}

/// Draws a leader from each label of `labels` to its target, `targets[i]` for label `i`.
///
/// Each label with a target is measured with `engine`, as renderers measure it, and gets the
/// leader that [`leader`] builds, drawn by [`make_leader_marks`]. A label without a target, or
/// that `labels.indices` skips, gets no leader.
pub fn make_text_leaders(
    labels: &SceneTextMark,
    targets: &[Option<LeaderTarget>],
    style: &LeaderStyle,
    engine: &LabelEngine,
) -> Result<SceneGroup, AvengerAnnotationError> {
    let len = labels.len as usize;
    if targets.len() != len {
        return Err(AvengerAnnotationError::LeaderCount {
            labels: len,
            leaders: targets.len(),
        });
    }

    let mut leaders = vec![None; len];
    for (index, label) in labels.indices_iter().zip(labels.labels()) {
        let Some(target) = targets[index] else {
            continue;
        };
        let placed = PlacedLabel {
            bounds: engine.bounds(&label.label)?,
            position: label.position,
            align: label.align,
            baseline: label.baseline,
            angle: label.angle,
        };
        leaders[index] = leader(&placed, target, style);
    }
    make_leader_marks(labels, &leaders, style)
}

/// A leader's line and arrowhead, in scene coordinates.
#[derive(Debug, Clone, PartialEq)]
struct Geometry {
    spine: Spine,
    /// The arrowhead's tip and the two corners of its base.
    arrowhead: Option<[[f32; 2]; 3]>,
}

/// A leader's line.
#[derive(Debug, Clone, PartialEq)]
enum Spine {
    Line {
        start: [f32; 2],
        end: [f32; 2],
    },
    Polyline {
        points: Vec<[f32; 2]>,
    },
    Cubic {
        start: [f32; 2],
        ctrl1: [f32; 2],
        ctrl2: [f32; 2],
        end: [f32; 2],
    },
}

/// The side of a label's box that a leader leaves from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitSide {
    Left,
    Right,
    Top,
    Bottom,
}

impl Spine {
    fn to_path(&self) -> Path {
        let mut builder = Path::builder();
        match self {
            Spine::Line { start, end } => {
                builder.begin(point(start[0], start[1]));
                builder.line_to(point(end[0], end[1]));
                builder.end(false);
            }
            Spine::Polyline { points } => {
                if let Some(first) = points.first() {
                    builder.begin(point(first[0], first[1]));
                    for p in &points[1..] {
                        builder.line_to(point(p[0], p[1]));
                    }
                    builder.end(false);
                }
            }
            Spine::Cubic {
                start,
                ctrl1,
                ctrl2,
                end,
            } => {
                builder.begin(point(start[0], start[1]));
                builder.cubic_bezier_to(
                    point(ctrl1[0], ctrl1[1]),
                    point(ctrl2[0], ctrl2[1]),
                    point(end[0], end[1]),
                );
                builder.end(false);
            }
        }
        builder.build()
    }
}

/// A closed path through `points`.
fn polygon(points: &[[f32; 2]]) -> Path {
    let mut builder = Path::builder();
    builder.begin(point(points[0][0], points[0][1]));
    for p in &points[1..] {
        builder.line_to(point(p[0], p[1]));
    }
    builder.close();
    builder.build()
}

/// A label's leader, or none when the target is inside the label's padded box or too near it.
fn leader_geometry(
    label: &PlacedLabel,
    target: LeaderTarget,
    style: &LeaderStyle,
) -> Option<Geometry> {
    let target_radius = target.radius.max(0.0);
    let min_length = style.min_length.max(0.0);
    let [left, top, right, bottom] = label.padded_box(style.label_padding.max(0.0));

    // The leader is built in the label's unrotated frame, then rotated with it.
    let angle = label.angle.to_radians();
    let target = rotate_around(target.position, label.position, -angle);

    if contains_point(left, right, top, bottom, target) {
        return None;
    }

    let center = [(left + right) * 0.5, (top + bottom) * 0.5];
    let (ray_start, side) = box_ray_intersection(left, right, top, bottom, center, target);
    let start = match style.shape {
        LeaderShape::Straight => ray_start,
        LeaderShape::Elbow | LeaderShape::Curved => side_center(left, right, top, bottom, side),
    };
    let remaining_gap = length(sub(target, start)) - target_radius;
    if remaining_gap <= EPSILON || remaining_gap < min_length {
        return None;
    }

    let (spine, tip, tangent, effective_arrow_length) =
        build_spine(style, start, target, target_radius, side);
    let arrowhead = build_arrowhead(
        style.arrow,
        tip,
        tangent,
        effective_arrow_length,
        style.arrow_width,
    );

    Some(Geometry {
        spine: rotate_spine(spine, label.position, angle),
        arrowhead: arrowhead
            .map(|points| points.map(|corner| rotate_around(corner, label.position, angle))),
    })
}

/// Builds a leader from `start` toward `target`, with its tip, the direction it arrives in, and
/// the arrowhead length that fits. Its last segment points at the target's center, and the tip
/// lies on that segment's line, `target_radius` short of the center, so an arrowhead aims at
/// the center whatever the shape.
fn build_spine(
    style: &LeaderStyle,
    start: [f32; 2],
    target: [f32; 2],
    target_radius: f32,
    side: ExitSide,
) -> (Spine, [f32; 2], [f32; 2], f32) {
    let (arrow, arrow_length, arrow_width) = (style.arrow, style.arrow_length, style.arrow_width);
    // The last segment, from `from` toward the target's center: the tip, the direction, the
    // arrowhead length that fits, and where the line ends under the arrowhead.
    let arrive = |from: [f32; 2]| {
        let to_target = sub(target, from);
        let tangent = normalize_or(to_target, [1.0, 0.0]);
        let tip = sub(target, scale(tangent, target_radius));
        let effective_arrow_length = effective_arrow_length(
            arrow,
            arrow_length,
            arrow_width,
            length(to_target) - target_radius,
        );
        let end = spine_end(tip, tangent, arrow, effective_arrow_length, arrow_width);
        (tip, tangent, effective_arrow_length, end)
    };

    match style.shape {
        LeaderShape::Straight => {
            let (tip, tangent, effective_arrow_length, end) = arrive(start);
            (
                Spine::Line { start, end },
                tip,
                tangent,
                effective_arrow_length,
            )
        }
        LeaderShape::Elbow => {
            // The second leg lines up with the target's center.
            let bend = match side {
                ExitSide::Left | ExitSide::Right => [target[0], start[1]],
                ExitSide::Top | ExitSide::Bottom => [start[0], target[1]],
            };
            if length(sub(bend, start)) < DEGENERATE_SEGMENT
                || length(sub(target, bend)) - target_radius < DEGENERATE_SEGMENT
            {
                let (tip, tangent, effective_arrow_length, end) = arrive(start);
                (
                    Spine::Line { start, end },
                    tip,
                    tangent,
                    effective_arrow_length,
                )
            } else {
                let (tip, tangent, effective_arrow_length, end) = arrive(bend);
                let mut points = vec![start, bend];
                if length(sub(end, bend)) > EPSILON {
                    points.push(end);
                }
                (
                    Spine::Polyline { points },
                    tip,
                    tangent,
                    effective_arrow_length,
                )
            }
        }
        LeaderShape::Curved => {
            let distance = length(sub(target, start)) - target_radius;
            let control = (distance * 0.35).clamp(8.0, 48.0).min(distance * 0.5);
            let ctrl1 = add(start, scale(outward_normal(side), control));
            let (tip, tangent, effective_arrow_length, end) = arrive(ctrl1);
            let ctrl2 = sub(end, scale(tangent, control));
            (
                Spine::Cubic {
                    start,
                    ctrl1,
                    ctrl2,
                    end,
                },
                tip,
                tangent,
                effective_arrow_length,
            )
        }
    }
}

/// The arrowhead length that fits on a last segment `available_length` long.
fn effective_arrow_length(
    arrow: LeaderArrow,
    arrow_length: f32,
    arrow_width: f32,
    available_length: f32,
) -> f32 {
    if arrow == LeaderArrow::Triangle && arrow_length > EPSILON && arrow_width > EPSILON {
        arrow_length.min((available_length - DEGENERATE_SEGMENT).max(0.0))
    } else {
        arrow_length
    }
}

/// Where the line ends: at the tip, or under a triangle arrowhead at its base.
fn spine_end(
    tip: [f32; 2],
    tangent: [f32; 2],
    arrow: LeaderArrow,
    arrow_length: f32,
    arrow_width: f32,
) -> [f32; 2] {
    if arrow == LeaderArrow::Triangle && arrow_length > EPSILON && arrow_width > EPSILON {
        sub(tip, scale(tangent, arrow_length))
    } else {
        tip
    }
}

/// The arrowhead's tip and the two corners of its base, or none when the style has no arrowhead
/// or it has no size.
fn build_arrowhead(
    arrow: LeaderArrow,
    tip: [f32; 2],
    tangent: [f32; 2],
    arrow_length: f32,
    arrow_width: f32,
) -> Option<[[f32; 2]; 3]> {
    let arrow_length = arrow_length.max(0.0);
    let arrow_width = arrow_width.max(0.0);
    if arrow == LeaderArrow::None || arrow_length <= EPSILON || arrow_width <= EPSILON {
        return None;
    }

    let tangent = normalize_or(tangent, [1.0, 0.0]);
    let normal = [-tangent[1], tangent[0]];
    let base_center = sub(tip, scale(tangent, arrow_length));
    let left_base = add(base_center, scale(normal, arrow_width * 0.5));
    let right_base = sub(base_center, scale(normal, arrow_width * 0.5));
    Some([tip, left_base, right_base])
}

fn rotate_spine(spine: Spine, center: [f32; 2], angle: f32) -> Spine {
    let rotate = |p| rotate_around(p, center, angle);
    match spine {
        Spine::Line { start, end } => Spine::Line {
            start: rotate(start),
            end: rotate(end),
        },
        Spine::Polyline { points } => Spine::Polyline {
            points: points.into_iter().map(rotate).collect(),
        },
        Spine::Cubic {
            start,
            ctrl1,
            ctrl2,
            end,
        } => Spine::Cubic {
            start: rotate(start),
            ctrl1: rotate(ctrl1),
            ctrl2: rotate(ctrl2),
            end: rotate(end),
        },
    }
}

fn contains_point(left: f32, right: f32, top: f32, bottom: f32, point: [f32; 2]) -> bool {
    point[0] >= left && point[0] <= right && point[1] >= top && point[1] <= bottom
}

/// Where the ray from the box's center to the target leaves the box, and through which side.
fn box_ray_intersection(
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
    center: [f32; 2],
    target: [f32; 2],
) -> ([f32; 2], ExitSide) {
    let half_width = ((right - left) * 0.5).abs().max(EPSILON);
    let half_height = ((bottom - top) * 0.5).abs().max(EPSILON);
    let dx = target[0] - center[0];
    let dy = target[1] - center[1];

    let side = if (dx / half_width).abs() >= (dy / half_height).abs() {
        if dx >= 0.0 {
            ExitSide::Right
        } else {
            ExitSide::Left
        }
    } else if dy >= 0.0 {
        ExitSide::Bottom
    } else {
        ExitSide::Top
    };

    let point = match side {
        ExitSide::Left => {
            let t = if dx.abs() <= EPSILON {
                0.0
            } else {
                (left - center[0]) / dx
            };
            [left, (center[1] + dy * t).clamp(top, bottom)]
        }
        ExitSide::Right => {
            let t = if dx.abs() <= EPSILON {
                0.0
            } else {
                (right - center[0]) / dx
            };
            [right, (center[1] + dy * t).clamp(top, bottom)]
        }
        ExitSide::Top => {
            let t = if dy.abs() <= EPSILON {
                0.0
            } else {
                (top - center[1]) / dy
            };
            [(center[0] + dx * t).clamp(left, right), top]
        }
        ExitSide::Bottom => {
            let t = if dy.abs() <= EPSILON {
                0.0
            } else {
                (bottom - center[1]) / dy
            };
            [(center[0] + dx * t).clamp(left, right), bottom]
        }
    };

    (point, side)
}

fn side_center(left: f32, right: f32, top: f32, bottom: f32, side: ExitSide) -> [f32; 2] {
    match side {
        ExitSide::Left => [left, (top + bottom) * 0.5],
        ExitSide::Right => [right, (top + bottom) * 0.5],
        ExitSide::Top => [(left + right) * 0.5, top],
        ExitSide::Bottom => [(left + right) * 0.5, bottom],
    }
}

fn outward_normal(side: ExitSide) -> [f32; 2] {
    match side {
        ExitSide::Left => [-1.0, 0.0],
        ExitSide::Right => [1.0, 0.0],
        ExitSide::Top => [0.0, -1.0],
        ExitSide::Bottom => [0.0, 1.0],
    }
}

fn rotate_around(point: [f32; 2], center: [f32; 2], angle: f32) -> [f32; 2] {
    if angle.abs() <= EPSILON {
        return point;
    }
    let (sin, cos) = angle.sin_cos();
    let translated = sub(point, center);
    [
        center[0] + translated[0] * cos - translated[1] * sin,
        center[1] + translated[0] * sin + translated[1] * cos,
    ]
}

fn add(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] + b[0], a[1] + b[1]]
}

fn sub(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn scale(v: [f32; 2], s: f32) -> [f32; 2] {
    [v[0] * s, v[1] * s]
}

fn length(v: [f32; 2]) -> f32 {
    (v[0] * v[0] + v[1] * v[1]).sqrt()
}

fn normalize_or(v: [f32; 2], fallback: [f32; 2]) -> [f32; 2] {
    let len = length(v);
    if len <= EPSILON {
        fallback
    } else {
        [v[0] / len, v[1] / len]
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use avenger_scenegraph::marks::mark::SceneMark;
    use lyon_path::Event;

    use super::*;

    const BOUNDS: TextBounds = TextBounds {
        width: 40.0,
        height: 10.0,
        ascent: 8.0,
        leading: 2.0,
    };

    /// A centered 40 × 10 label at `position`, turned by `angle` degrees.
    fn placed(position: [f32; 2], angle: f32) -> PlacedLabel {
        PlacedLabel {
            bounds: BOUNDS,
            position,
            align: TextAlign::Center,
            baseline: TextBaseline::Middle,
            angle,
        }
    }

    /// The leader to `target` of a centered 40 × 10 label offset from it by `(dx, dy)`.
    fn geometry(
        target: [f32; 2],
        radius: f32,
        dx: f32,
        dy: f32,
        style: &LeaderStyle,
    ) -> Option<Geometry> {
        leader_geometry(
            &placed([target[0] + dx, target[1] + dy], 0.0),
            LeaderTarget {
                position: target,
                radius,
            },
            style,
        )
    }

    fn style(shape: LeaderShape, arrow: LeaderArrow) -> LeaderStyle {
        LeaderStyle {
            shape,
            arrow,
            ..Default::default()
        }
    }

    #[test]
    fn suppresses_when_target_inside_padded_box() {
        assert!(geometry([10.0, 10.0], 0.0, 0.0, 0.0, &LeaderStyle::default()).is_none());
    }

    #[test]
    fn builds_straight_leader_to_target() {
        let geometry = geometry([10.0, 10.0], 0.0, 80.0, 0.0, &LeaderStyle::default()).unwrap();
        assert!(matches!(geometry.spine, Spine::Line { .. }));
    }

    #[test]
    fn suppresses_when_target_radius_reaches_label() {
        for radius in [58.0, 60.0, 100.0] {
            assert!(geometry([10.0, 10.0], radius, 80.0, 0.0, &LeaderStyle::default()).is_none());
        }
    }

    #[test]
    fn target_radius_shortens_tip() {
        let Spine::Line { end, .. } =
            geometry([10.0, 10.0], 10.0, 80.0, 0.0, &LeaderStyle::default())
                .unwrap()
                .spine
        else {
            panic!("expected line");
        };
        assert!((end[0] - 20.0).abs() < 0.01);
    }

    #[test]
    fn builds_open_arrowhead() {
        let style = style(LeaderShape::Straight, LeaderArrow::Open);
        let geometry = geometry([10.0, 10.0], 0.0, 80.0, 0.0, &style).unwrap();
        assert!(geometry.arrowhead.is_some());
    }

    #[test]
    fn builds_elbow_polyline() {
        let style = style(LeaderShape::Elbow, LeaderArrow::None);
        let geometry = geometry([10.0, 10.0], 0.0, 80.0, 50.0, &style).unwrap();
        assert!(matches!(geometry.spine, Spine::Polyline { .. }));
        assert!(geometry.arrowhead.is_none());
    }

    #[test]
    fn elbow_leader_starts_from_center_of_selected_side() {
        let style = style(LeaderShape::Elbow, LeaderArrow::None);
        let Spine::Polyline { points } = geometry([10.0, 10.0], 0.0, 80.0, 50.0, &style)
            .unwrap()
            .spine
        else {
            panic!("expected elbow polyline");
        };
        assert_close_point(points[0], [90.0, 53.0]);
    }

    #[test]
    fn elbow_triangle_arrowhead_sits_on_spine_end() {
        let style = LeaderStyle {
            shape: LeaderShape::Elbow,
            arrow: LeaderArrow::Triangle,
            arrow_length: 12.0,
            arrow_width: 10.0,
            ..Default::default()
        };
        let geometry = geometry([10.0, 10.0], 0.0, 80.0, 50.0, &style).unwrap();
        let Spine::Polyline { points: spine } = &geometry.spine else {
            panic!("expected elbow polyline");
        };
        let Some(points) = geometry.arrowhead else {
            panic!("expected triangle arrowhead");
        };
        assert_close_point(midpoint(points[1], points[2]), *spine.last().unwrap());
    }

    #[test]
    fn curved_triangle_arrowhead_uses_cubic_endpoint_tangent() {
        let style = LeaderStyle {
            shape: LeaderShape::Curved,
            arrow: LeaderArrow::Triangle,
            arrow_length: 12.0,
            arrow_width: 10.0,
            ..Default::default()
        };
        let geometry = geometry([10.0, 10.0], 0.0, 80.0, 20.0, &style).unwrap();
        let Spine::Cubic {
            start, ctrl2, end, ..
        } = geometry.spine
        else {
            panic!("expected cubic leader");
        };
        let Some(points) = geometry.arrowhead else {
            panic!("expected triangle arrowhead");
        };
        let base_center = midpoint(points[1], points[2]);
        let curve_tangent = normalize_or(sub(end, ctrl2), [1.0, 0.0]);
        let arrow_tangent = normalize_or(sub(points[0], base_center), [1.0, 0.0]);
        let straight_tangent = normalize_or(sub(points[0], start), [1.0, 0.0]);

        assert_close_point(base_center, end);
        assert!(dot(curve_tangent, arrow_tangent) > 0.999);
        assert!(dot(straight_tangent, arrow_tangent) < 0.995);
    }

    #[test]
    fn builds_curved_cubic() {
        let style = style(LeaderShape::Curved, LeaderArrow::None);
        let geometry = geometry([10.0, 10.0], 0.0, 80.0, 50.0, &style).unwrap();
        assert!(matches!(geometry.spine, Spine::Cubic { .. }));
    }

    /// Whatever the shape, side and rotation, an arrowhead aims at the target's center, with
    /// its tip the target radius short of it.
    #[test]
    fn arrowheads_point_at_the_target_center() {
        let target = [10.0, 10.0];
        for shape in [
            LeaderShape::Straight,
            LeaderShape::Elbow,
            LeaderShape::Curved,
        ] {
            for (dx, dy, angle) in [
                (80.0, 50.0, 0.0),
                (110.0, 116.0, 0.0),
                (-90.0, 40.0, 0.0),
                (30.0, -70.0, 0.0),
                (60.0, -70.0, 30.0),
            ] {
                let style = style(shape, LeaderArrow::Triangle);
                let geometry = leader_geometry(
                    &placed([target[0] + dx, target[1] + dy], angle),
                    LeaderTarget {
                        position: target,
                        radius: 7.0,
                    },
                    &style,
                )
                .unwrap();
                let Some(points) = geometry.arrowhead else {
                    panic!("expected triangle arrowhead");
                };
                let axis = normalize_or(sub(points[0], midpoint(points[1], points[2])), [1.0, 0.0]);
                let to_center = sub(target, points[0]);
                let miss = to_center[0] * axis[1] - to_center[1] * axis[0];
                let case = format!("{shape:?} leader to a label at ({dx}, {dy}), {angle}°");
                assert!(miss.abs() < 1e-3, "{case} misses the center by {miss}");
                assert!(
                    dot(to_center, axis) > 0.0,
                    "{case} points away from the center"
                );
                assert!((length(to_center) - 7.0).abs() < 1e-3, "{case}");
            }
        }
    }

    /// The corners are the padded box that leaders start from, turned with the label.
    #[test]
    fn corners_turn_with_the_label() {
        let corners = placed([100.0, 50.0], 0.0).corners(2.0);
        for (corner, expected) in
            corners
                .into_iter()
                .zip([[78.0, 43.0], [122.0, 43.0], [122.0, 57.0], [78.0, 57.0]])
        {
            assert_close_point(corner, expected);
        }
        let turned = placed([100.0, 50.0], 90.0).corners(2.0);
        assert_close_point(turned[0], [107.0, 28.0]);
        assert_close_point(turned[2], [93.0, 72.0]);
    }

    /// Three labels named "notes", at x = 100, 200 and 300.
    fn notes(indices: Option<Vec<usize>>) -> SceneTextMark {
        SceneTextMark {
            name: "notes".to_string(),
            len: 3,
            text: ScalarOrArray::new_array(vec!["a".into(), "b".into(), "c".into()]),
            x: ScalarOrArray::new_array(vec![100.0, 200.0, 300.0]),
            y: 100.0.into(),
            indices: indices.map(Arc::new),
            ..Default::default()
        }
    }

    fn at(position: [f32; 2]) -> Option<LeaderTarget> {
        Some(LeaderTarget {
            position,
            radius: 0.0,
        })
    }

    /// The path marks of the three labels' leaders to `targets`.
    fn leaders(
        targets: &[Option<LeaderTarget>],
        style: &LeaderStyle,
        indices: Option<Vec<usize>>,
    ) -> SceneGroup {
        make_text_leaders(
            &notes(indices),
            targets,
            style,
            &avenger_typst_label::bundled_label_engine(),
        )
        .unwrap()
    }

    fn path_mark(mark: &SceneMark) -> &ScenePathMark {
        let SceneMark::Path(mark) = mark else {
            panic!("expected path mark");
        };
        mark
    }

    fn subpaths(path: &Path) -> usize {
        path.iter()
            .filter(|event| matches!(event, Event::Begin { .. }))
            .count()
    }

    /// Label `i`'s leader is instance `i`, named for the labels. A label without a target, one
    /// that the mark skips, or one whose target is inside its box has an empty path.
    #[test]
    fn each_label_gets_its_leader_as_its_instance() {
        let group = leaders(
            &[at([60.0, 40.0]), at([200.0, 97.0]), None],
            &LeaderStyle::default(),
            None,
        );
        assert_eq!(group.marks.len(), 1);
        let lines = path_mark(&group.marks[0]);
        assert_eq!(lines.name, "notes");
        let paths = lines.path.as_vec(3, None);
        assert_eq!(paths.iter().map(subpaths).collect::<Vec<_>>(), [1, 0, 0]);

        let skipped = leaders(
            &[at([60.0, 40.0]), at([160.0, 40.0]), at([260.0, 40.0])],
            &LeaderStyle::default(),
            Some(vec![0, 2]),
        );
        let paths = path_mark(&skipped.marks[0]).path.as_vec(3, None);
        assert_eq!(paths.iter().map(subpaths).collect::<Vec<_>>(), [1, 0, 1]);
    }

    /// `make_text_leaders` draws the leader that `leader` builds from the measured label.
    #[test]
    fn text_leaders_draw_the_leader_of_each_measured_label() {
        let engine = avenger_typst_label::bundled_label_engine();
        let notes = notes(None);
        let label = notes.labels().next().unwrap();
        let placed = PlacedLabel {
            bounds: engine.bounds(&label.label).unwrap(),
            position: label.position,
            align: label.align,
            baseline: label.baseline,
            angle: label.angle,
        };
        let target = LeaderTarget {
            position: [60.0, 40.0],
            radius: 3.0,
        };
        let style = LeaderStyle::default();
        let built = leader(&placed, target, &style).unwrap();
        let group =
            make_text_leaders(&notes, &[Some(target), None, None], &style, &engine).unwrap();
        let drawn = &path_mark(&group.marks[0]).path.as_vec(3, None)[0];
        assert_eq!(
            drawn.iter().collect::<Vec<_>>(),
            built.spine.iter().collect::<Vec<_>>()
        );
    }

    /// Triangle arrowheads get their own mark, filled with the leader color, and open ones
    /// join the line.
    #[test]
    fn arrowheads_draw_with_their_leaders() {
        let targets = [at([60.0, 40.0]), at([160.0, 40.0]), at([260.0, 40.0])];
        let triangle = style(LeaderShape::Straight, LeaderArrow::Triangle);
        let group = leaders(&targets, &triangle, None);
        assert_eq!(group.marks.len(), 2);
        let heads = path_mark(&group.marks[1]);
        assert_eq!(
            heads.fill.as_vec(1, None)[0],
            ColorOrGradient::Color(triangle.stroke)
        );
        assert!(heads
            .path
            .as_vec(3, None)
            .iter()
            .all(|path| subpaths(path) == 1));

        let open = leaders(
            &targets,
            &style(LeaderShape::Straight, LeaderArrow::Open),
            None,
        );
        assert_eq!(open.marks.len(), 1);
        let paths = path_mark(&open.marks[0]).path.as_vec(3, None);
        assert!(paths.iter().all(|path| subpaths(path) == 3));
    }

    /// Leaders take part in hit tests when their labels do.
    #[test]
    fn leaders_take_their_labels_interactivity() {
        let labels = SceneTextMark {
            text: "a".to_string().into(),
            interactive: false,
            ..Default::default()
        };
        let style = style(LeaderShape::Straight, LeaderArrow::Triangle);
        let group = make_text_leaders(
            &labels,
            &[at([100.0, 100.0])],
            &style,
            &avenger_typst_label::bundled_label_engine(),
        )
        .unwrap();
        assert_eq!(group.marks.len(), 2);
        assert!(group.marks.iter().all(|mark| !path_mark(mark).interactive));
    }

    /// A leader's line stays solid until `make_leader_marks` dashes it.
    #[test]
    fn marks_dash_the_leader_line() {
        let dashed = LeaderStyle {
            stroke_dash: Some(vec![2.0, 2.0]),
            ..Default::default()
        };
        let target = LeaderTarget {
            position: [20.0, 40.0],
            radius: 0.0,
        };
        let built = leader(&placed([100.0, 100.0], 0.0), target, &dashed).unwrap();
        assert_eq!(subpaths(&built.spine), 1);
        let labels = SceneTextMark {
            len: 1,
            ..Default::default()
        };
        let group = make_leader_marks(&labels, &[Some(built)], &dashed).unwrap();
        assert!(subpaths(&path_mark(&group.marks[0]).path.as_vec(1, None)[0]) > 1);
    }

    #[test]
    fn needs_one_entry_per_label() {
        let labels = SceneTextMark {
            len: 2,
            ..Default::default()
        };
        let style = LeaderStyle::default();
        let error = make_text_leaders(
            &labels,
            &[at([0.0, 0.0])],
            &style,
            &avenger_typst_label::bundled_label_engine(),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            AvengerAnnotationError::LeaderCount {
                labels: 2,
                leaders: 1
            }
        ));
        let error = make_leader_marks(&labels, &[None, None, None], &style).unwrap_err();
        assert!(matches!(
            error,
            AvengerAnnotationError::LeaderCount {
                labels: 2,
                leaders: 3
            }
        ));
    }

    fn assert_close_point(actual: [f32; 2], expected: [f32; 2]) {
        assert!(
            (actual[0] - expected[0]).abs() < 0.01 && (actual[1] - expected[1]).abs() < 0.01,
            "{actual:?} != {expected:?}"
        );
    }

    fn midpoint(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
        [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5]
    }

    fn dot(a: [f32; 2], b: [f32; 2]) -> f32 {
        a[0] * b[0] + a[1] * b[1]
    }
}
