use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

use avenger_color::{ColorOrGradient, Gradient};
use avenger_common::value::ScalarOrArray;
use itertools::izip;
use lyon_path::{
    builder::WithSvg,
    geom::{Angle, Vector},
    math::Point,
    path::BuilderImpl,
    Path,
};
use serde::{Deserialize, Serialize};

use super::mark::{default_interactive, SceneMark};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SceneArcMark {
    pub name: String,
    #[serde(default = "default_interactive")]
    pub interactive: bool,
    pub clip: bool,
    pub len: u32,
    pub gradients: Vec<Gradient>,
    pub x: ScalarOrArray<f32>,
    pub y: ScalarOrArray<f32>,
    pub start_angle: ScalarOrArray<f32>,
    pub end_angle: ScalarOrArray<f32>,
    pub outer_radius: ScalarOrArray<f32>,
    pub inner_radius: ScalarOrArray<f32>,
    pub pad_angle: ScalarOrArray<f32>,
    pub corner_radius: ScalarOrArray<f32>,
    pub fill: ScalarOrArray<ColorOrGradient>,
    pub stroke: ScalarOrArray<ColorOrGradient>,
    pub stroke_width: ScalarOrArray<f32>,
    pub indices: Option<Arc<Vec<usize>>>,
    pub zindex: Option<i32>,
}

impl Hash for SceneArcMark {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.interactive.hash(state);
        self.clip.hash(state);
        self.len.hash(state);
        self.gradients.hash(state);
        self.x.hash(state);
        self.y.hash(state);
        self.start_angle.hash(state);
        self.end_angle.hash(state);
        self.outer_radius.hash(state);
        self.inner_radius.hash(state);
        self.pad_angle.hash(state);
        self.corner_radius.hash(state);
        self.fill.hash(state);
        self.stroke.hash(state);
        self.stroke_width.hash(state);
        self.indices.hash(state);
        self.zindex.hash(state);
    }
}

impl SceneArcMark {
    pub fn x_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn y_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn start_angle_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.start_angle
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn end_angle_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.end_angle
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn outer_radius_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.outer_radius
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn inner_radius_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.inner_radius
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn pad_angle_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.pad_angle
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn corner_radius_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.corner_radius
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn fill_iter(&self) -> Box<dyn Iterator<Item = &ColorOrGradient> + '_> {
        self.fill.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn stroke_iter(&self) -> Box<dyn Iterator<Item = &ColorOrGradient> + '_> {
        self.stroke
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn stroke_width_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.stroke_width
            .as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn indices_iter(&self) -> Box<dyn Iterator<Item = usize> + '_> {
        if let Some(indices) = self.indices.as_ref() {
            Box::new(indices.iter().cloned())
        } else {
            Box::new(0..self.len as usize)
        }
    }

    pub fn transformed_path_iter(&self, origin: [f32; 2]) -> Box<dyn Iterator<Item = Path> + '_> {
        Box::new(
            izip!(
                self.x_iter(),
                self.y_iter(),
                self.start_angle_iter(),
                self.end_angle_iter(),
                self.outer_radius_iter(),
                self.inner_radius_iter(),
                self.pad_angle_iter(),
                self.corner_radius_iter()
            )
            .map(
                move |(
                    x,
                    y,
                    start_angle,
                    end_angle,
                    outer_radius,
                    inner_radius,
                    pad_angle,
                    corner_radius,
                )| {
                    build_arc_path(ArcGeometry {
                        x: *x + origin[0],
                        y: *y + origin[1],
                        start_angle: *start_angle,
                        end_angle: *end_angle,
                        outer_radius: *outer_radius,
                        inner_radius: *inner_radius,
                        pad_angle: *pad_angle,
                        corner_radius: *corner_radius,
                    })
                },
            ),
        )
    }
}

#[derive(Debug, Copy, Clone)]
struct ArcGeometry {
    x: f32,
    y: f32,
    start_angle: f32,
    end_angle: f32,
    outer_radius: f32,
    inner_radius: f32,
    pad_angle: f32,
    corner_radius: f32,
}

#[derive(Debug, Copy, Clone)]
struct CornerTangents {
    cx: f32,
    cy: f32,
    x01: f32,
    y01: f32,
    x11: f32,
    y11: f32,
}

fn build_arc_path(geometry: ArcGeometry) -> Path {
    const HALF_PI: f32 = std::f32::consts::FRAC_PI_2;
    const PI: f32 = std::f32::consts::PI;
    const TAU: f32 = std::f32::consts::TAU;
    const EPSILON: f32 = 1e-6;

    let mut r0 = geometry.inner_radius;
    let mut r1 = geometry.outer_radius;
    let a0 = geometry.start_angle - HALF_PI;
    let a1 = geometry.end_angle - HALF_PI;
    let da = (a1 - a0).abs();
    let clockwise = a1 > a0;

    if r1 < r0 {
        std::mem::swap(&mut r0, &mut r1);
    }

    let mut path_builder = Path::builder().with_svg();
    let center = Point::new(geometry.x, geometry.y);
    let point = |radius: f32, angle: f32| -> Point {
        Point::new(
            geometry.x + radius * angle.cos(),
            geometry.y + radius * angle.sin(),
        )
    };

    if r1 <= EPSILON {
        path_builder.move_to(center);
        path_builder.close();
        return path_builder.build();
    }

    if da > TAU - EPSILON {
        path_builder.move_to(point(r1, a0));
        append_canvas_arc(&mut path_builder, center, r1, a0, a1, !clockwise);

        if r0 > EPSILON {
            path_builder.move_to(point(r0, a1));
            append_canvas_arc(&mut path_builder, center, r0, a1, a0, clockwise);
        }

        path_builder.close();
        return path_builder.build();
    }

    let mut a01 = a0;
    let mut a11 = a1;
    let mut a00 = a0;
    let mut a10 = a1;
    let mut da0 = da;
    let mut da1 = da;
    let ap = geometry.pad_angle.abs() / 2.0;
    let rp = if ap > EPSILON {
        (r0 * r0 + r1 * r1).sqrt()
    } else {
        0.0
    };
    let rc = ((r1 - r0).abs() / 2.0).min(geometry.corner_radius.max(0.0));
    let mut rc0 = rc;
    let mut rc1 = rc;

    if rp > EPSILON {
        if r0 > EPSILON {
            let mut p0 = ((rp / r0) * ap.sin()).clamp(-1.0, 1.0).asin();
            da0 -= p0 * 2.0;
            if da0 > EPSILON {
                if !clockwise {
                    p0 = -p0;
                }
                a00 += p0;
                a10 -= p0;
            } else {
                da0 = 0.0;
                a00 = (a0 + a1) / 2.0;
                a10 = a00;
            }
        } else {
            da0 = 0.0;
            a00 = (a0 + a1) / 2.0;
            a10 = a00;
        }

        let mut p1 = ((rp / r1) * ap.sin()).clamp(-1.0, 1.0).asin();
        da1 -= p1 * 2.0;
        if da1 > EPSILON {
            if !clockwise {
                p1 = -p1;
            }
            a01 += p1;
            a11 -= p1;
        } else {
            da1 = 0.0;
            a01 = (a0 + a1) / 2.0;
            a11 = a01;
        }
    }

    let x01 = r1 * a01.cos();
    let y01 = r1 * a01.sin();
    let x10 = r0 * a10.cos();
    let y10 = r0 * a10.sin();

    if rc > EPSILON {
        let x11 = r1 * a11.cos();
        let y11 = r1 * a11.sin();
        let x00 = r0 * a00.cos();
        let y00 = r0 * a00.sin();

        if da < PI {
            if let Some([ix, iy]) = intersect(x01, y01, x00, y00, x11, y11, x10, y10) {
                let ax = x01 - ix;
                let ay = y01 - iy;
                let bx = x11 - ix;
                let by = y11 - iy;
                let a_len = (ax * ax + ay * ay).sqrt();
                let b_len = (bx * bx + by * by).sqrt();

                if a_len > EPSILON && b_len > EPSILON {
                    let cos_angle = ((ax * bx + ay * by) / (a_len * b_len)).clamp(-1.0, 1.0);
                    let kc = 1.0 / (cos_angle.acos() / 2.0).sin();
                    let lc = (ix * ix + iy * iy).sqrt();

                    if (kc - 1.0).abs() > EPSILON {
                        rc0 = rc.min((r0 - lc) / (kc - 1.0));
                    }
                    rc1 = rc.min((r1 - lc) / (kc + 1.0));
                } else {
                    rc0 = 0.0;
                    rc1 = 0.0;
                }
            } else {
                rc0 = 0.0;
                rc1 = 0.0;
            }
        }
    }

    if da1 <= EPSILON {
        path_builder.move_to(offset_point(geometry.x, geometry.y, x01, y01));
    } else if rc1 > EPSILON {
        let x11 = r1 * a11.cos();
        let y11 = r1 * a11.sin();
        let x00 = r0 * a00.cos();
        let y00 = r0 * a00.sin();
        let t0 = corner_tangents(x00, y00, x01, y01, r1, rc1, clockwise);
        let t1 = corner_tangents(x11, y11, x10, y10, r1, rc1, clockwise);

        path_builder.move_to(offset_point(
            geometry.x,
            geometry.y,
            t0.cx + t0.x01,
            t0.cy + t0.y01,
        ));

        if rc1 < rc {
            append_canvas_arc(
                &mut path_builder,
                offset_point(geometry.x, geometry.y, t0.cx, t0.cy),
                rc1,
                t0.y01.atan2(t0.x01),
                t1.y01.atan2(t1.x01),
                !clockwise,
            );
        } else {
            append_canvas_arc(
                &mut path_builder,
                offset_point(geometry.x, geometry.y, t0.cx, t0.cy),
                rc1,
                t0.y01.atan2(t0.x01),
                t0.y11.atan2(t0.x11),
                !clockwise,
            );
            append_canvas_arc(
                &mut path_builder,
                center,
                r1,
                (t0.cy + t0.y11).atan2(t0.cx + t0.x11),
                (t1.cy + t1.y11).atan2(t1.cx + t1.x11),
                !clockwise,
            );
            append_canvas_arc(
                &mut path_builder,
                offset_point(geometry.x, geometry.y, t1.cx, t1.cy),
                rc1,
                t1.y11.atan2(t1.x11),
                t1.y01.atan2(t1.x01),
                !clockwise,
            );
        }
    } else {
        path_builder.move_to(offset_point(geometry.x, geometry.y, x01, y01));
        append_canvas_arc(&mut path_builder, center, r1, a01, a11, !clockwise);
    }

    if r0 <= EPSILON || da0 <= EPSILON {
        path_builder.line_to(offset_point(geometry.x, geometry.y, x10, y10));
    } else if rc0 > EPSILON {
        let x11 = r1 * a11.cos();
        let y11 = r1 * a11.sin();
        let x00 = r0 * a00.cos();
        let y00 = r0 * a00.sin();
        let t0 = corner_tangents(x10, y10, x11, y11, r0, -rc0, clockwise);
        let t1 = corner_tangents(x01, y01, x00, y00, r0, -rc0, clockwise);

        path_builder.line_to(offset_point(
            geometry.x,
            geometry.y,
            t0.cx + t0.x01,
            t0.cy + t0.y01,
        ));

        if rc0 < rc {
            append_canvas_arc(
                &mut path_builder,
                offset_point(geometry.x, geometry.y, t0.cx, t0.cy),
                rc0,
                t0.y01.atan2(t0.x01),
                t1.y01.atan2(t1.x01),
                !clockwise,
            );
        } else {
            append_canvas_arc(
                &mut path_builder,
                offset_point(geometry.x, geometry.y, t0.cx, t0.cy),
                rc0,
                t0.y01.atan2(t0.x01),
                t0.y11.atan2(t0.x11),
                !clockwise,
            );
            append_canvas_arc(
                &mut path_builder,
                center,
                r0,
                (t0.cy + t0.y11).atan2(t0.cx + t0.x11),
                (t1.cy + t1.y11).atan2(t1.cx + t1.x11),
                clockwise,
            );
            append_canvas_arc(
                &mut path_builder,
                offset_point(geometry.x, geometry.y, t1.cx, t1.cy),
                rc0,
                t1.y11.atan2(t1.x11),
                t1.y01.atan2(t1.x01),
                !clockwise,
            );
        }
    } else {
        // Lyon derives the arc start from the current point on the inner circle.
        path_builder.line_to(offset_point(geometry.x, geometry.y, x10, y10));
        append_canvas_arc(&mut path_builder, center, r0, a10, a00, clockwise);
    }

    path_builder.close();
    path_builder.build()
}

fn append_canvas_arc(
    path_builder: &mut WithSvg<BuilderImpl>,
    center: Point,
    radius: f32,
    start_angle: f32,
    end_angle: f32,
    anticlockwise: bool,
) {
    const TAU: f32 = std::f32::consts::TAU;
    const EPSILON: f32 = 1e-6;

    if radius <= EPSILON {
        return;
    }

    let mut sweep = end_angle - start_angle;
    if anticlockwise {
        if sweep > 0.0 {
            sweep -= TAU;
        }
    } else if sweep < 0.0 {
        sweep += TAU;
    }

    if sweep.abs() <= EPSILON {
        return;
    }

    path_builder.arc(
        center,
        Vector::new(radius, radius),
        Angle::radians(sweep),
        Angle::radians(0.0),
    );
}

fn offset_point(offset_x: f32, offset_y: f32, x: f32, y: f32) -> Point {
    Point::new(offset_x + x, offset_y + y)
}

#[allow(
    clippy::too_many_arguments,
    reason = "Keep the explicit inputs of the existing layout and rendering pipeline."
)]
fn intersect(
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    x3: f32,
    y3: f32,
) -> Option<[f32; 2]> {
    const EPSILON: f32 = 1e-6;

    let x10 = x1 - x0;
    let y10 = y1 - y0;
    let x32 = x3 - x2;
    let y32 = y3 - y2;
    let mut t = y32 * x10 - x32 * y10;
    if t * t < EPSILON {
        return None;
    }

    t = (x32 * (y0 - y2) - y32 * (x0 - x2)) / t;
    Some([x0 + t * x10, y0 + t * y10])
}

fn corner_tangents(
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    radius: f32,
    corner_radius: f32,
    clockwise: bool,
) -> CornerTangents {
    let x01 = x0 - x1;
    let y01 = y0 - y1;
    let lo = if clockwise {
        corner_radius
    } else {
        -corner_radius
    } / (x01 * x01 + y01 * y01).sqrt();
    let ox = lo * y01;
    let oy = -lo * x01;
    let x11 = x0 + ox;
    let y11 = y0 + oy;
    let x10 = x1 + ox;
    let y10 = y1 + oy;
    let x00 = (x11 + x10) / 2.0;
    let y00 = (y11 + y10) / 2.0;
    let dx = x10 - x11;
    let dy = y10 - y11;
    let d2 = dx * dx + dy * dy;
    let r = radius - corner_radius;
    let determinant = x11 * y10 - x10 * y11;
    let d = if dy < 0.0 { -1.0 } else { 1.0 }
        * (0.0_f32.max(r * r * d2 - determinant * determinant)).sqrt();
    let mut cx0 = (determinant * dy - dx * d) / d2;
    let mut cy0 = (-determinant * dx - dy * d) / d2;
    let cx1 = (determinant * dy + dx * d) / d2;
    let cy1 = (-determinant * dx + dy * d) / d2;
    let dx0 = cx0 - x00;
    let dy0 = cy0 - y00;
    let dx1 = cx1 - x00;
    let dy1 = cy1 - y00;

    if dx0 * dx0 + dy0 * dy0 > dx1 * dx1 + dy1 * dy1 {
        cx0 = cx1;
        cy0 = cy1;
    }

    CornerTangents {
        cx: cx0,
        cy: cy0,
        x01: -ox,
        y01: -oy,
        x11: cx0 * (radius / r - 1.0),
        y11: cy0 * (radius / r - 1.0),
    }
}

impl Default for SceneArcMark {
    fn default() -> Self {
        Self {
            name: "arc_mark".to_string(),
            interactive: true,
            clip: true,
            len: 1,
            gradients: vec![],
            x: ScalarOrArray::new_scalar(0.0),
            y: ScalarOrArray::new_scalar(0.0),
            start_angle: ScalarOrArray::new_scalar(0.0),
            end_angle: ScalarOrArray::new_scalar(0.7),
            outer_radius: ScalarOrArray::new_scalar(10.0),
            inner_radius: ScalarOrArray::new_scalar(0.0),
            pad_angle: ScalarOrArray::new_scalar(0.0),
            corner_radius: ScalarOrArray::new_scalar(0.0),
            fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])),
            stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            stroke_width: ScalarOrArray::new_scalar(0.0),
            indices: None,
            zindex: None,
        }
    }
}

impl From<SceneArcMark> for SceneMark {
    fn from(mark: SceneArcMark) -> Self {
        SceneMark::Arc(mark)
    }
}

#[cfg(test)]
mod tests {
    use avenger_common::value::ScalarOrArray;
    use lyon_path::Event;

    use super::*;

    #[test]
    fn annular_sector_connects_outer_and_inner_edges() {
        for (start, end) in [
            (0.0, std::f32::consts::FRAC_PI_2),
            (std::f32::consts::FRAC_PI_2, 0.0),
        ] {
            let mark = SceneArcMark {
                start_angle: ScalarOrArray::new_scalar(start),
                end_angle: ScalarOrArray::new_scalar(end),
                inner_radius: ScalarOrArray::new_scalar(5.0),
                outer_radius: ScalarOrArray::new_scalar(10.0),
                ..Default::default()
            };
            let center = Point::new(17.0, 23.0);
            let path = mark
                .transformed_path_iter([center.x, center.y])
                .next()
                .unwrap();
            let angle = end - std::f32::consts::FRAC_PI_2;
            let expected_outer = center + Vector::new(10.0 * angle.cos(), 10.0 * angle.sin());
            let expected_inner = center + Vector::new(5.0 * angle.cos(), 5.0 * angle.sin());
            assert!(path.iter().any(|event| matches!(event, Event::Line { from, to }
                if from.distance_to(expected_outer) < 1e-4 && to.distance_to(expected_inner) < 1e-4)));
        }
    }

    #[test]
    fn transformed_path_iter_applies_pad_angle_to_arc_outer_start() {
        let unpadded = SceneArcMark {
            start_angle: ScalarOrArray::new_scalar(0.0),
            end_angle: ScalarOrArray::new_scalar(std::f32::consts::FRAC_PI_2),
            inner_radius: ScalarOrArray::new_scalar(5.0),
            outer_radius: ScalarOrArray::new_scalar(10.0),
            pad_angle: ScalarOrArray::new_scalar(0.0),
            ..Default::default()
        };
        let padded = SceneArcMark {
            pad_angle: ScalarOrArray::new_scalar(0.2),
            ..unpadded.clone()
        };

        let unpadded_path = unpadded.transformed_path_iter([0.0, 0.0]).next().unwrap();
        let padded_path = padded.transformed_path_iter([0.0, 0.0]).next().unwrap();

        let unpadded_outer_start = first_move_to(&unpadded_path);
        let padded_outer_start = first_move_to(&padded_path);
        assert!(unpadded_outer_start.x.abs() <= 1e-4);
        assert!(padded_outer_start.x.abs() > 0.1);
        assert!((unpadded_outer_start.x - padded_outer_start.x).abs() > 0.1);
    }

    #[test]
    fn full_circle_arc_does_not_apply_pad_angle() {
        let unpadded = SceneArcMark {
            start_angle: ScalarOrArray::new_scalar(0.0),
            end_angle: ScalarOrArray::new_scalar(std::f32::consts::TAU),
            inner_radius: ScalarOrArray::new_scalar(5.0),
            outer_radius: ScalarOrArray::new_scalar(10.0),
            pad_angle: ScalarOrArray::new_scalar(0.0),
            ..Default::default()
        };
        let padded = SceneArcMark {
            pad_angle: ScalarOrArray::new_scalar(0.4),
            ..unpadded.clone()
        };

        let unpadded_path = unpadded.transformed_path_iter([0.0, 0.0]).next().unwrap();
        let padded_path = padded.transformed_path_iter([0.0, 0.0]).next().unwrap();

        let unpadded_outer_start = first_move_to(&unpadded_path);
        let padded_outer_start = first_move_to(&padded_path);
        assert!((unpadded_outer_start.x - padded_outer_start.x).abs() <= 1e-4);
        assert!((unpadded_outer_start.y - padded_outer_start.y).abs() <= 1e-4);
    }

    #[test]
    fn transformed_path_iter_applies_corner_radius_to_arc_outer_start() {
        let sharp = SceneArcMark {
            start_angle: ScalarOrArray::new_scalar(0.0),
            end_angle: ScalarOrArray::new_scalar(std::f32::consts::FRAC_PI_2),
            inner_radius: ScalarOrArray::new_scalar(4.0),
            outer_radius: ScalarOrArray::new_scalar(12.0),
            corner_radius: ScalarOrArray::new_scalar(0.0),
            ..Default::default()
        };
        let rounded = SceneArcMark {
            corner_radius: ScalarOrArray::new_scalar(3.0),
            ..sharp.clone()
        };

        let sharp_path = sharp.transformed_path_iter([0.0, 0.0]).next().unwrap();
        let rounded_path = rounded.transformed_path_iter([0.0, 0.0]).next().unwrap();
        let sharp_outer_start = first_move_to(&sharp_path);
        let rounded_outer_start = first_move_to(&rounded_path);

        let dx = sharp_outer_start.x - rounded_outer_start.x;
        let dy = sharp_outer_start.y - rounded_outer_start.y;
        assert!((dx * dx + dy * dy).sqrt() > 0.1);
        assert!(quadratic_count(&rounded_path) > quadratic_count(&sharp_path));
    }

    fn first_move_to(path: &lyon_path::Path) -> lyon_path::math::Point {
        path.iter()
            .find_map(|event| match event {
                Event::Begin { at } => Some(at),
                _ => None,
            })
            .expect("arc path should include a move command")
    }

    fn quadratic_count(path: &lyon_path::Path) -> usize {
        path.iter()
            .filter(|event| matches!(event, Event::Quadratic { .. }))
            .count()
    }
}
