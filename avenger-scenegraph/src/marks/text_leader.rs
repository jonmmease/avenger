use avenger_text::{
    measurement::TextBounds,
    types::{TextAlign, TextBaseline},
};

use avenger_common::types::{SceneTextLeaderArrow, SceneTextLeaderShape};

const EPSILON: f32 = 1.0e-5;
const DEGENERATE_SEGMENT: f32 = 0.5;

#[derive(Debug, Clone, Copy)]
pub struct TextLeaderGeometryInput<'a> {
    pub target: [f32; 2],
    pub label_anchor: [f32; 2],
    pub angle_degrees: f32,
    pub text_bounds: &'a TextBounds,
    pub align: &'a TextAlign,
    pub baseline: &'a TextBaseline,
    pub label_padding: f32,
    pub target_radius: f32,
    pub min_length: f32,
    pub shape: SceneTextLeaderShape,
    pub arrow: SceneTextLeaderArrow,
    pub arrow_length: f32,
    pub arrow_width: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextLeaderGeometry {
    pub spine: TextLeaderPath,
    pub arrowhead: Option<TextLeaderArrowhead>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TextLeaderPath {
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

#[derive(Debug, Clone, PartialEq)]
pub enum TextLeaderArrowhead {
    Open {
        left: [[f32; 2]; 2],
        right: [[f32; 2]; 2],
    },
    Triangle {
        points: [[f32; 2]; 3],
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitSide {
    Left,
    Right,
    Top,
    Bottom,
}

impl TextLeaderPath {
    pub fn points(&self) -> Vec<[f32; 2]> {
        match self {
            TextLeaderPath::Line { start, end } => vec![*start, *end],
            TextLeaderPath::Polyline { points } => points.clone(),
            TextLeaderPath::Cubic {
                start,
                ctrl1,
                ctrl2,
                end,
            } => vec![*start, *ctrl1, *ctrl2, *end],
        }
    }
}

pub fn compute_text_leader_geometry(
    input: TextLeaderGeometryInput<'_>,
) -> Option<TextLeaderGeometry> {
    let padding = input.label_padding.max(0.0);
    let target_radius = input.target_radius.max(0.0);
    let min_length = input.min_length.max(0.0);

    let text_origin =
        input
            .text_bounds
            .calculate_origin(input.label_anchor, input.align, input.baseline);
    let left = text_origin[0] - padding;
    let right = text_origin[0] + input.text_bounds.width + padding;
    let top = text_origin[1] - padding;
    let bottom = text_origin[1] + input.text_bounds.height + padding;

    let angle = input.angle_degrees.to_radians();
    let target_local = rotate_around(input.target, input.label_anchor, -angle);

    if contains_point(left, right, top, bottom, target_local) {
        return None;
    }

    let center = [(left + right) * 0.5, (top + bottom) * 0.5];
    let (ray_start, side) = box_ray_intersection(left, right, top, bottom, center, target_local)?;
    let start = match input.shape {
        SceneTextLeaderShape::Straight => ray_start,
        SceneTextLeaderShape::Elbow | SceneTextLeaderShape::Curved => {
            side_center(left, right, top, bottom, side)
        }
    };
    let to_target = sub(target_local, start);
    let distance_to_target = length(to_target);
    if distance_to_target <= EPSILON {
        return None;
    }
    let target_direction = scale(to_target, 1.0 / distance_to_target);
    let tip = sub(target_local, scale(target_direction, target_radius));
    if length(sub(tip, start)) < min_length {
        return None;
    }

    let (mut spine, tangent, effective_arrow_length) = build_spine(
        input.shape,
        start,
        tip,
        side,
        input.arrow,
        input.arrow_length,
        input.arrow_width,
    );
    let arrowhead = build_arrowhead(
        input.arrow,
        tip,
        tangent,
        effective_arrow_length,
        input.arrow_width,
    );

    spine = transform_path_to_scene(spine, input.label_anchor, angle);
    let arrowhead = arrowhead
        .map(|arrowhead| transform_arrowhead_to_scene(arrowhead, input.label_anchor, angle));

    Some(TextLeaderGeometry { spine, arrowhead })
}

fn build_spine(
    shape: SceneTextLeaderShape,
    start: [f32; 2],
    tip: [f32; 2],
    side: ExitSide,
    arrow: SceneTextLeaderArrow,
    arrow_length: f32,
    arrow_width: f32,
) -> (TextLeaderPath, [f32; 2], f32) {
    let raw_tangent = normalize_or(sub(tip, start), [1.0, 0.0]);

    match shape {
        SceneTextLeaderShape::Straight => {
            let tangent = normalize_or(sub(tip, start), raw_tangent);
            let effective_arrow_length =
                effective_arrow_length(arrow, arrow_length, arrow_width, length(sub(tip, start)));
            let end = spine_end(tip, tangent, arrow, effective_arrow_length, arrow_width);
            (
                TextLeaderPath::Line { start, end },
                tangent,
                effective_arrow_length,
            )
        }
        SceneTextLeaderShape::Elbow => {
            let bend = match side {
                ExitSide::Left | ExitSide::Right => [tip[0], start[1]],
                ExitSide::Top | ExitSide::Bottom => [start[0], tip[1]],
            };
            if length(sub(bend, start)) < DEGENERATE_SEGMENT
                || length(sub(tip, bend)) < DEGENERATE_SEGMENT
            {
                let tangent = normalize_or(sub(tip, start), raw_tangent);
                let effective_arrow_length = effective_arrow_length(
                    arrow,
                    arrow_length,
                    arrow_width,
                    length(sub(tip, start)),
                );
                let end = spine_end(tip, tangent, arrow, effective_arrow_length, arrow_width);
                (
                    TextLeaderPath::Line { start, end },
                    tangent,
                    effective_arrow_length,
                )
            } else {
                let tangent = normalize_or(sub(tip, bend), raw_tangent);
                let effective_arrow_length = effective_arrow_length(
                    arrow,
                    arrow_length,
                    arrow_width,
                    length(sub(tip, bend)),
                );
                let end = spine_end(tip, tangent, arrow, effective_arrow_length, arrow_width);
                let mut points = vec![start, bend];
                if length(sub(end, bend)) > EPSILON {
                    points.push(end);
                }
                (
                    TextLeaderPath::Polyline { points },
                    tangent,
                    effective_arrow_length,
                )
            }
        }
        SceneTextLeaderShape::Curved => {
            let distance = length(sub(tip, start));
            let control = (distance * 0.35).clamp(8.0, 48.0).min(distance * 0.5);
            let outward = outward_normal(side);
            let ctrl1 = add(start, scale(outward, control));
            let tangent = normalize_or(sub(tip, ctrl1), raw_tangent);
            let effective_arrow_length =
                effective_arrow_length(arrow, arrow_length, arrow_width, distance);
            let end = spine_end(tip, tangent, arrow, effective_arrow_length, arrow_width);
            let ctrl2 = sub(end, scale(tangent, control));
            (
                TextLeaderPath::Cubic {
                    start,
                    ctrl1,
                    ctrl2,
                    end,
                },
                tangent,
                effective_arrow_length,
            )
        }
    }
}

fn effective_arrow_length(
    arrow: SceneTextLeaderArrow,
    arrow_length: f32,
    arrow_width: f32,
    available_length: f32,
) -> f32 {
    if matches!(arrow, SceneTextLeaderArrow::Triangle)
        && arrow_length > EPSILON
        && arrow_width > EPSILON
    {
        arrow_length.min((available_length - DEGENERATE_SEGMENT).max(0.0))
    } else {
        arrow_length
    }
}

fn spine_end(
    tip: [f32; 2],
    tangent: [f32; 2],
    arrow: SceneTextLeaderArrow,
    arrow_length: f32,
    arrow_width: f32,
) -> [f32; 2] {
    if matches!(arrow, SceneTextLeaderArrow::Triangle)
        && arrow_length > EPSILON
        && arrow_width > EPSILON
    {
        sub(tip, scale(tangent, arrow_length))
    } else {
        tip
    }
}

fn build_arrowhead(
    arrow: SceneTextLeaderArrow,
    tip: [f32; 2],
    tangent: [f32; 2],
    arrow_length: f32,
    arrow_width: f32,
) -> Option<TextLeaderArrowhead> {
    let arrow_length = arrow_length.max(0.0);
    let arrow_width = arrow_width.max(0.0);
    if arrow_length <= EPSILON || arrow_width <= EPSILON {
        return None;
    }

    let tangent = normalize_or(tangent, [1.0, 0.0]);
    let normal = [-tangent[1], tangent[0]];
    let base_center = sub(tip, scale(tangent, arrow_length));
    let left_base = add(base_center, scale(normal, arrow_width * 0.5));
    let right_base = sub(base_center, scale(normal, arrow_width * 0.5));

    match arrow {
        SceneTextLeaderArrow::None => None,
        SceneTextLeaderArrow::Open => Some(TextLeaderArrowhead::Open {
            left: [left_base, tip],
            right: [right_base, tip],
        }),
        SceneTextLeaderArrow::Triangle => Some(TextLeaderArrowhead::Triangle {
            points: [tip, left_base, right_base],
        }),
    }
}

fn transform_path_to_scene(path: TextLeaderPath, center: [f32; 2], angle: f32) -> TextLeaderPath {
    match path {
        TextLeaderPath::Line { start, end } => TextLeaderPath::Line {
            start: rotate_around(start, center, angle),
            end: rotate_around(end, center, angle),
        },
        TextLeaderPath::Polyline { points } => TextLeaderPath::Polyline {
            points: points
                .into_iter()
                .map(|point| rotate_around(point, center, angle))
                .collect(),
        },
        TextLeaderPath::Cubic {
            start,
            ctrl1,
            ctrl2,
            end,
        } => TextLeaderPath::Cubic {
            start: rotate_around(start, center, angle),
            ctrl1: rotate_around(ctrl1, center, angle),
            ctrl2: rotate_around(ctrl2, center, angle),
            end: rotate_around(end, center, angle),
        },
    }
}

fn transform_arrowhead_to_scene(
    arrowhead: TextLeaderArrowhead,
    center: [f32; 2],
    angle: f32,
) -> TextLeaderArrowhead {
    match arrowhead {
        TextLeaderArrowhead::Open { left, right } => TextLeaderArrowhead::Open {
            left: [
                rotate_around(left[0], center, angle),
                rotate_around(left[1], center, angle),
            ],
            right: [
                rotate_around(right[0], center, angle),
                rotate_around(right[1], center, angle),
            ],
        },
        TextLeaderArrowhead::Triangle { points } => TextLeaderArrowhead::Triangle {
            points: [
                rotate_around(points[0], center, angle),
                rotate_around(points[1], center, angle),
                rotate_around(points[2], center, angle),
            ],
        },
    }
}

fn contains_point(left: f32, right: f32, top: f32, bottom: f32, point: [f32; 2]) -> bool {
    point[0] >= left && point[0] <= right && point[1] >= top && point[1] <= bottom
}

fn box_ray_intersection(
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
    center: [f32; 2],
    target: [f32; 2],
) -> Option<([f32; 2], ExitSide)> {
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

    Some((point, side))
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
    let sin = angle.sin();
    let cos = angle.cos();
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
    use avenger_text::measurement::TextBounds;

    use super::*;

    fn bounds() -> TextBounds {
        TextBounds {
            width: 40.0,
            height: 10.0,
            ascent: 8.0,
            descent: 2.0,
            line_height: 12.0,
        }
    }

    fn input(target: [f32; 2], dx: f32, dy: f32) -> TextLeaderGeometryInput<'static> {
        TextLeaderGeometryInput {
            target,
            label_anchor: [target[0] + dx, target[1] + dy],
            angle_degrees: 0.0,
            text_bounds: Box::leak(Box::new(bounds())),
            align: &TextAlign::Center,
            baseline: &TextBaseline::Middle,
            label_padding: 2.0,
            target_radius: 0.0,
            min_length: 1.0,
            shape: SceneTextLeaderShape::Straight,
            arrow: SceneTextLeaderArrow::None,
            arrow_length: 6.0,
            arrow_width: 5.0,
        }
    }

    #[test]
    fn suppresses_when_target_inside_padded_box() {
        let geometry = compute_text_leader_geometry(input([10.0, 10.0], 0.0, 0.0));
        assert!(geometry.is_none());
    }

    #[test]
    fn builds_straight_leader_to_target() {
        let geometry = compute_text_leader_geometry(input([10.0, 10.0], 80.0, 0.0)).unwrap();
        assert!(matches!(geometry.spine, TextLeaderPath::Line { .. }));
    }

    #[test]
    fn target_radius_shortens_tip() {
        let mut input = input([10.0, 10.0], 80.0, 0.0);
        input.target_radius = 10.0;
        let geometry = compute_text_leader_geometry(input).unwrap();
        let TextLeaderPath::Line { end, .. } = geometry.spine else {
            panic!("expected line");
        };
        assert!((end[0] - 20.0).abs() < 0.01);
    }

    #[test]
    fn builds_open_arrowhead() {
        let mut input = input([10.0, 10.0], 80.0, 0.0);
        input.arrow = SceneTextLeaderArrow::Open;
        let geometry = compute_text_leader_geometry(input).unwrap();
        assert!(matches!(
            geometry.arrowhead,
            Some(TextLeaderArrowhead::Open { .. })
        ));
    }

    #[test]
    fn builds_elbow_polyline() {
        let mut input = input([10.0, 10.0], 80.0, 50.0);
        input.shape = SceneTextLeaderShape::Elbow;
        let geometry = compute_text_leader_geometry(input).unwrap();
        assert!(matches!(geometry.spine, TextLeaderPath::Polyline { .. }));
    }

    #[test]
    fn elbow_leader_starts_from_center_of_selected_side() {
        let mut input = input([10.0, 10.0], 80.0, 50.0);
        input.shape = SceneTextLeaderShape::Elbow;

        let geometry = compute_text_leader_geometry(input).unwrap();
        let start = match &geometry.spine {
            TextLeaderPath::Polyline { points } => points[0],
            _ => panic!("expected elbow polyline"),
        };

        assert_close_point(start, [90.0, 53.0]);
    }

    #[test]
    fn elbow_triangle_arrowhead_sits_on_spine_end() {
        let mut input = input([10.0, 10.0], 80.0, 50.0);
        input.shape = SceneTextLeaderShape::Elbow;
        input.arrow = SceneTextLeaderArrow::Triangle;
        input.arrow_length = 12.0;
        input.arrow_width = 10.0;

        let geometry = compute_text_leader_geometry(input).unwrap();
        let spine_end = match &geometry.spine {
            TextLeaderPath::Polyline { points } => *points.last().unwrap(),
            _ => panic!("expected elbow polyline"),
        };
        let arrowhead = geometry.arrowhead.unwrap();
        let TextLeaderArrowhead::Triangle { points } = arrowhead else {
            panic!("expected triangle arrowhead");
        };
        let base_center = [
            (points[1][0] + points[2][0]) * 0.5,
            (points[1][1] + points[2][1]) * 0.5,
        ];

        assert_close_point(base_center, spine_end);
    }

    #[test]
    fn curved_triangle_arrowhead_uses_cubic_endpoint_tangent() {
        let mut input = input([10.0, 10.0], 80.0, 20.0);
        input.shape = SceneTextLeaderShape::Curved;
        input.arrow = SceneTextLeaderArrow::Triangle;
        input.arrow_length = 12.0;
        input.arrow_width = 10.0;

        let geometry = compute_text_leader_geometry(input).unwrap();
        let TextLeaderPath::Cubic {
            start, ctrl2, end, ..
        } = geometry.spine
        else {
            panic!("expected cubic leader");
        };
        let arrowhead = geometry.arrowhead.unwrap();
        let TextLeaderArrowhead::Triangle { points } = arrowhead else {
            panic!("expected triangle arrowhead");
        };
        let base_center = [
            (points[1][0] + points[2][0]) * 0.5,
            (points[1][1] + points[2][1]) * 0.5,
        ];
        let curve_tangent = normalize_or(sub(end, ctrl2), [1.0, 0.0]);
        let arrow_tangent = normalize_or(sub(points[0], base_center), [1.0, 0.0]);
        let straight_tangent = normalize_or(sub(points[0], start), [1.0, 0.0]);

        assert_close_point(base_center, end);
        assert!(dot(curve_tangent, arrow_tangent) > 0.999);
        assert!(dot(straight_tangent, arrow_tangent) < 0.995);
    }

    #[test]
    fn builds_curved_cubic() {
        let mut input = input([10.0, 10.0], 80.0, 50.0);
        input.shape = SceneTextLeaderShape::Curved;
        let geometry = compute_text_leader_geometry(input).unwrap();
        assert!(matches!(geometry.spine, TextLeaderPath::Cubic { .. }));
    }

    fn assert_close_point(actual: [f32; 2], expected: [f32; 2]) {
        assert!(
            (actual[0] - expected[0]).abs() < 0.01,
            "x mismatch: {actual:?} != {expected:?}"
        );
        assert!(
            (actual[1] - expected[1]).abs() < 0.01,
            "y mismatch: {actual:?} != {expected:?}"
        );
    }

    fn dot(a: [f32; 2], b: [f32; 2]) -> f32 {
        a[0] * b[0] + a[1] * b[1]
    }
}
