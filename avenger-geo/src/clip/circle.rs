//! Clipping to a small circle around the projection's center.
//!
//! Ported from d3-geo `src/clip/circle.js` and `circleStream` in
//! `src/circle.js` (ISC), <https://github.com/d3/d3-geo>

use super::{BufPoint, ClipOut, ClipPolicy, LineClipper};
use crate::math::{
    acos, cartesian, cartesian_add_in_place, cartesian_cross, cartesian_dot,
    cartesian_normalize_in_place, cartesian_scale, point_equal, spherical, EPSILON, HALF_PI, PI,
    RADIANS, TAU,
};
use crate::stream::GeoSink;
use std::cmp::Ordering;

/// Keeps the cap within `radius` radians of the rotated origin, d3's `clipAngle`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CirclePolicy {
    radius: f64,
    cr: f64,
    /// The step of the interpolated boundary, two degrees.
    delta: f64,
    small_radius: bool,
    not_hemisphere: bool,
}

impl CirclePolicy {
    /// The circle's angle from its center, in radians.
    pub(crate) fn radius(&self) -> f64 {
        self.radius
    }

    pub(crate) fn new(radius: f64) -> Self {
        let cr = radius.cos();
        CirclePolicy {
            radius,
            cr,
            delta: 2.0 * RADIANS,
            small_radius: cr > 0.0,
            not_hemisphere: cr.abs() > EPSILON,
        }
    }

    fn visible(&self, lambda: f64, phi: f64) -> bool {
        lambda.cos() * phi.cos() > self.cr
    }

    /// A 4-bit code for a point's place relative to the small circle's
    /// bounding box.
    fn code(&self, lambda: f64, phi: f64) -> u8 {
        let r = if self.small_radius {
            self.radius
        } else {
            PI - self.radius
        };
        let mut code = 0;
        if lambda < -r {
            code |= 1; // left
        } else if lambda > r {
            code |= 2; // right
        }
        if phi < -r {
            code |= 4; // below
        } else if phi > r {
            code |= 8; // above
        }
        code
    }

    /// Where the great circle from `a` to `b` meets the clip circle: the
    /// first meeting, or both when `two` and both lie between `a` and `b`.
    fn intersect(&self, a: [f64; 2], b: [f64; 2], two: bool) -> Intersection {
        let pa = cartesian(a[0], a[1]);
        let pb = cartesian(b[0], b[1]);
        // Two planes, n1.p = d1 and n2.p = d2, meet on the line
        // p(t) = c1 n1 + c2 n2 + t (n1 ⨯ n2).
        let n1 = [1.0, 0.0, 0.0];
        let n2 = cartesian_cross(&pa, &pb);
        let n2n2 = cartesian_dot(&n2, &n2);
        let n1n2 = n2[0];
        let determinant = n2n2 - n1n2 * n1n2;
        // Two polar points.
        if determinant == 0.0 {
            return if two {
                Intersection::None
            } else {
                Intersection::One(a)
            };
        }
        let c1 = self.cr * n2n2 / determinant;
        let c2 = -self.cr * n1n2 / determinant;
        let n1xn2 = cartesian_cross(&n1, &n2);
        let mut a_point = cartesian_scale(&n1, c1);
        cartesian_add_in_place(&mut a_point, &cartesian_scale(&n2, c2));

        // Solve |p(t)|^2 = 1.
        let u = n1xn2;
        let w = cartesian_dot(&a_point, &u);
        let uu = cartesian_dot(&u, &u);
        let t2 = w * w - uu * (cartesian_dot(&a_point, &a_point) - 1.0);
        if t2 < 0.0 {
            return Intersection::None;
        }
        let t = t2.sqrt();
        let mut q = cartesian_scale(&u, (-w - t) / uu);
        cartesian_add_in_place(&mut q, &a_point);
        let q = spherical(&q);
        if !two {
            return Intersection::One(q);
        }

        // Two intersection points.
        let (mut lambda0, mut lambda1) = (a[0], b[0]);
        let (mut phi0, mut phi1) = (a[1], b[1]);
        if lambda1 < lambda0 {
            std::mem::swap(&mut lambda0, &mut lambda1);
        }
        let delta = lambda1 - lambda0;
        let polar = (delta - PI).abs() < EPSILON;
        let meridian = polar || delta < EPSILON;
        if !polar && phi1 < phi0 {
            std::mem::swap(&mut phi0, &mut phi1);
        }
        // Check that the first point is between a and b.
        let between = if meridian {
            if polar {
                let limit = if (q[0] - lambda0).abs() < EPSILON {
                    phi0
                } else {
                    phi1
                };
                (phi0 + phi1 > 0.0) ^ (q[1] < limit)
            } else {
                phi0 <= q[1] && q[1] <= phi1
            }
        } else {
            (delta > PI) ^ (lambda0 <= q[0] && q[0] <= lambda1)
        };
        if between {
            let mut q1 = cartesian_scale(&u, (-w + t) / uu);
            cartesian_add_in_place(&mut q1, &a_point);
            Intersection::Two(q, spherical(&q1))
        } else {
            Intersection::None
        }
    }
}

/// What [`CirclePolicy::intersect`] finds.
enum Intersection {
    None,
    One([f64; 2]),
    Two([f64; 2], [f64; 2]),
}

impl ClipPolicy for CirclePolicy {
    type Line = CircleLine;

    fn point_visible(&self, lambda: f64, phi: f64) -> bool {
        self.visible(lambda, phi)
    }

    fn line_clipper(&self) -> CircleLine {
        CircleLine {
            policy: *self,
            point0: None,
            c0: 0,
            v0: false,
            v00: false,
            clean: 0,
        }
    }

    fn interpolate(
        &self,
        from: Option<&BufPoint>,
        to: Option<&BufPoint>,
        direction: i32,
        sink: &mut dyn GeoSink,
    ) {
        let (from, to) = (from.map(|p| [p.x, p.y]), to.map(|p| [p.x, p.y]));
        circle_stream(
            sink,
            self.radius,
            self.delta,
            f64::from(direction),
            from,
            to,
        );
    }

    fn start_point(&self) -> [f64; 2] {
        if self.small_radius {
            [0.0, -self.radius]
        } else {
            [-PI, self.radius - PI]
        }
    }

    /// d3's `compareIntersection`, as for the antimeridian.
    fn compare_intersection(&self, a: &BufPoint, b: &BufPoint) -> Ordering {
        fn key(p: &BufPoint) -> f64 {
            if p.x < 0.0 {
                p.y - HALF_PI - EPSILON
            } else {
                HALF_PI - p.y
            }
        }
        key(a).total_cmp(&key(b))
    }
}

/// Cuts a line into its visible segments (d3 `clipLine`).
pub(crate) struct CircleLine {
    policy: CirclePolicy,
    /// The previous point.
    point0: Option<[f64; 2]>,
    /// The previous point's code.
    c0: u8,
    /// Whether the previous point is visible.
    v0: bool,
    /// Whether the first point is visible.
    v00: bool,
    /// 1 when the line has no intersections.
    clean: u8,
}

impl LineClipper for CircleLine {
    fn line_start(&mut self, _sink: &mut ClipOut<'_>) {
        self.v00 = false;
        self.v0 = false;
        self.clean = 1;
    }

    fn point(&mut self, lambda: f64, phi: f64, _m: Option<f64>, sink: &mut ClipOut<'_>) {
        let policy = self.policy;
        let point1 = [lambda, phi];
        let v = policy.visible(lambda, phi);
        let c = if policy.small_radius {
            if v {
                0
            } else {
                policy.code(lambda, phi)
            }
        } else if v {
            policy.code(lambda + if lambda < 0.0 { PI } else { -PI }, phi)
        } else {
            0
        };
        if self.point0.is_none() {
            self.v00 = v;
            self.v0 = v;
            if v {
                sink.line_start();
            }
        }
        // d3 marks `point1` here when its intersection is degenerate, but it
        // streams `point1` without the mark, so the mark has no effect.
        if v != self.v0 {
            self.clean = 0;
            let point0 = self
                .point0
                .expect("a previous point when visibility changes");
            if v {
                // Outside going in.
                sink.line_start();
                let point2 = one(policy.intersect(point1, point0, false), point1);
                sink.point(point2[0], point2[1], None);
                self.point0 = Some(point2);
            } else {
                // Inside going out.
                let point2 = one(policy.intersect(point0, point1, false), point0);
                sink.point(point2[0], point2[1], Some(2.0));
                sink.line_end();
                self.point0 = Some(point2);
            }
        } else if let Some(point0) = self.point0 {
            if policy.not_hemisphere && (policy.small_radius ^ v) && (c & self.c0) == 0 {
                // The codes differ, or are both zero, so the segment may cross
                // the small circle twice.
                if let Intersection::Two(t0, t1) = policy.intersect(point1, point0, true) {
                    self.clean = 0;
                    if policy.small_radius {
                        sink.line_start();
                        sink.point(t0[0], t0[1], None);
                        sink.point(t1[0], t1[1], None);
                        sink.line_end();
                    } else {
                        sink.point(t1[0], t1[1], None);
                        sink.line_end();
                        sink.line_start();
                        sink.point(t0[0], t0[1], Some(3.0));
                    }
                }
            }
        }
        if v && self
            .point0
            .is_none_or(|point0| !point_equal(point0, point1))
        {
            sink.point(point1[0], point1[1], None);
        }
        self.point0 = Some(point1);
        self.v0 = v;
        self.c0 = c;
    }

    fn line_end(&mut self, sink: &mut ClipOut<'_>) {
        if self.v0 {
            sink.line_end();
        }
        self.point0 = None;
    }

    fn clean(&self) -> u8 {
        self.clean | (u8::from(self.v00 && self.v0) << 1)
    }
}

/// The single intersection, or `fallback` when there is none.
fn one(intersection: Intersection, fallback: [f64; 2]) -> [f64; 2] {
    match intersection {
        Intersection::One(point) => point,
        _ => fallback,
    }
}

/// Streams the circle of `radius` radians around [0, 0], from `t0` to `t1` in
/// `direction`, or the whole circle without them, every `delta` radians
/// (d3 `circleStream`).
pub(crate) fn circle_stream(
    sink: &mut dyn GeoSink,
    radius: f64,
    delta: f64,
    direction: f64,
    t0: Option<[f64; 2]>,
    t1: Option<[f64; 2]>,
) {
    if delta == 0.0 {
        return;
    }
    let cos_radius = radius.cos();
    let sin_radius = radius.sin();
    let step = direction * delta;
    let (mut t0, t1) = match (t0, t1) {
        (Some(from), Some(to)) => {
            let mut t0 = circle_radius(cos_radius, from);
            let t1 = circle_radius(cos_radius, to);
            if if direction > 0.0 { t0 < t1 } else { t0 > t1 } {
                t0 += direction * TAU;
            }
            (t0, t1)
        }
        _ => (radius + direction * TAU, radius - step / 2.0),
    };
    while if direction > 0.0 { t0 > t1 } else { t0 < t1 } {
        let point = spherical(&[cos_radius, -sin_radius * t0.cos(), -sin_radius * t0.sin()]);
        sink.point(point[0], point[1], None);
        t0 -= step;
    }
}

/// The signed angle of a point relative to [cos_radius, 0, 0].
fn circle_radius(cos_radius: f64, point: [f64; 2]) -> f64 {
    let mut point = cartesian(point[0], point[1]);
    point[0] -= cos_radius;
    cartesian_normalize_in_place(&mut point);
    let radius = acos(-point[1]);
    ((if -point[2] < 0.0 { -radius } else { radius }) + TAU - EPSILON) % TAU
}
