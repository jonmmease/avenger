//! Clipping to a spherical polygon, the inside of a [`SphericalClip::Region`].
//!
//! Ported from d3-geo-polygon `src/clip/polygon.js`, `src/intersect.js`,
//! `src/cartesian.js`, and `src/polygonContains.js` (ISC),
//! <https://github.com/d3/d3-geo-polygon>. Its clip uses its own, older
//! `polygonContains`, which this module keeps for parity.
//!
//! [`SphericalClip::Region`]: crate::projector::SphericalClip::Region

use std::cmp::Ordering;
use std::sync::Arc;

use super::{BufPoint, ClipOut, ClipPolicy, LineClipper};
use crate::error::AvengerGeoError;
use crate::math::{
    acos, asin, cartesian, cartesian_cross, cartesian_dot, sign, spherical, EPSILON, EPSILON2,
    HALF_PI, PI, QUARTER_PI, RADIANS, TAU,
};
use crate::stream::GeoSink;
use geo_types::MultiPolygon;

/// One polygon's clip: its rings in radians, in d3's winding, and the great-circle segments of
/// its exterior.
#[derive(Debug, Clone)]
pub(crate) struct PolygonPolicy {
    polygon: Arc<Vec<Vec<[f64; 2]>>>,
    segments: Arc<Vec<Segment>>,
    start: [f64; 2],
}

/// A clip for each polygon of `region`, whose rings follow RFC 7946 winding, with its vertices
/// turned by `rotate`, which takes and gives radians.
///
/// # Errors
/// Returns an error for a region without polygons, a polygon with holes, which d3-geo-polygon
/// does not clip lines to, an exterior with fewer than three vertices, or a non-finite vertex.
pub(crate) fn prepare(
    region: &MultiPolygon<f64>,
    rotate: impl Fn(f64, f64) -> (f64, f64),
) -> Result<Vec<PolygonPolicy>, AvengerGeoError> {
    if region.0.is_empty() {
        return Err(AvengerGeoError::InvalidConfig(
            "a region clip needs a polygon".into(),
        ));
    }
    region
        .iter()
        .map(|polygon| {
            if !polygon.interiors().is_empty() {
                return Err(AvengerGeoError::InvalidConfig(
                    "a region clip's polygons cannot have holes".into(),
                ));
            }
            // A ring from `Polygon::new` repeats its first vertex at the end, and one that was
            // deserialized need not.
            let coords = &polygon.exterior().0;
            let coords = match (coords.first(), coords.last()) {
                (Some(first), Some(last)) if coords.len() > 1 && first == last => {
                    &coords[..coords.len() - 1]
                }
                _ => &coords[..],
            };
            if coords.len() < 3 {
                return Err(AvengerGeoError::InvalidConfig(
                    "a region clip's polygon needs three or more vertices".into(),
                ));
            }
            if !coords.iter().all(|c| c.x.is_finite() && c.y.is_finite()) {
                return Err(AvengerGeoError::InvalidConfig(
                    "a region clip's vertices must be finite".into(),
                ));
            }
            let turned = |coord: &geo_types::Coord<f64>| {
                let (lambda, phi) = rotate(coord.x * RADIANS, coord.y * RADIANS);
                [lambda, phi]
            };
            // d3's winding: the first point first, then the others reversed.
            let mut ring: Vec<[f64; 2]> = Vec::with_capacity(coords.len());
            ring.push(turned(&coords[0]));
            ring.extend(coords[1..].iter().rev().map(turned));
            ring.push(ring[0]);
            let points: Vec<[f64; 3]> = ring.iter().map(|p| cartesian(p[0], p[1])).collect();
            let segments = points
                .windows(2)
                .map(|pair| Segment::new(pair[0], pair[1]))
                .collect();
            Ok(PolygonPolicy {
                start: ring[0],
                polygon: Arc::new(vec![ring]),
                segments: Arc::new(segments),
            })
        })
        .collect()
}

impl ClipPolicy for PolygonPolicy {
    type Line = PolygonLine;

    fn point_visible(&self, lambda: f64, phi: f64) -> bool {
        polygon_contains(&self.polygon, [lambda, phi])
    }

    fn line_clipper(&self) -> PolygonLine {
        PolygonLine {
            segments: self.segments.clone(),
            polygon: self.polygon.clone(),
            point0: None,
            lambda00: 0.0,
            phi00: 0.0,
            v00: false,
            v0: false,
            clean: 0,
            line: Vec::new(),
            lines: Vec::new(),
        }
    }

    fn interpolate(
        &self,
        from: Option<&BufPoint>,
        to: Option<&BufPoint>,
        direction: i32,
        sink: &mut dyn GeoSink,
    ) {
        // Without crossings, the whole region: its rings, inside the polygon that the clip stage
        // or `sphere` opens. d3-geo-polygon opens a polygon here too, which nests it inside the
        // clip stage's, so a rectangle clip after it fails.
        let (Some(from), Some(to)) = (from, to) else {
            for ring in self.polygon.iter() {
                sink.line_start();
                for point in ring {
                    sink.point(point[0], point[1], None);
                }
                sink.line_end();
            }
            return;
        };
        let (Some(from_index), Some(to_index)) = (from.index, to.index) else {
            return;
        };
        let n = self.segments.len() as i64;
        let step = |index: i64| (index + i64::from(direction) + n) % n;
        let corner = |index: i64, sink: &mut dyn GeoSink| {
            let segment = &self.segments[index as usize];
            let point = spherical(if direction > 0 {
                &segment.to
            } else {
                &segment.from
            });
            sink.point(point[0], point[1], None);
        };
        if from_index != to_index {
            let mut index = from_index as i64;
            while index != to_index as i64 {
                corner(index, sink);
                index = step(index);
            }
        } else if from.t > to.t {
            for i in 0..n {
                corner((from_index as i64 + i * i64::from(direction) + n) % n, sink);
            }
        }
    }

    fn start_point(&self) -> [f64; 2] {
        self.start
    }

    /// d3-geo-polygon's `clipPolygonSort`: by clip edge, then along it.
    fn compare_intersection(&self, a: &BufPoint, b: &BufPoint) -> Ordering {
        match (a.index, b.index) {
            (Some(ai), Some(bi)) => ai.cmp(&bi).then(a.t.total_cmp(&b.t)),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        }
    }

    fn contains(&self, polygon: &[Vec<[f64; 2]>], point: [f64; 2]) -> bool {
        polygon_contains(polygon, point)
    }

    /// The region's rings, which stream their own lines.
    fn whole(&self, sink: &mut dyn GeoSink) {
        self.interpolate(None, None, 1, sink);
    }

    fn closes_coincident_segments(&self) -> bool {
        true
    }
}

/// A great-circle segment between two cartesian points (d3-geo-polygon
/// `intersectSegment`).
#[derive(Debug, Clone, Copy)]
struct Segment {
    from: [f64; 3],
    to: [f64; 3],
    normal: [f64; 3],
    from_normal: [f64; 3],
    to_normal: [f64; 3],
    l: f64,
}

impl Segment {
    fn new(from: [f64; 3], to: [f64; 3]) -> Self {
        let normal = cartesian_cross(&from, &to);
        Segment {
            from,
            to,
            normal,
            from_normal: cartesian_cross(&normal, &from),
            to_normal: cartesian_cross(&normal, &to),
            l: acos(cartesian_dot(&from, &to)),
        }
    }
}

fn cartesian_equal(a: &[f64; 3], b: &[f64; 3]) -> bool {
    let (dx, dy, dz) = (b[0] - a[0], b[1] - a[1], b[2] - a[2]);
    dx * dx + dy * dy + dz * dz < EPSILON2 * EPSILON2
}

fn normalize(d: [f64; 3]) -> [f64; 3] {
    let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    [d[0] / l, d[1] / l, d[2] / l]
}

/// Where segments `a` and `b` cross, if they do (d3-geo-polygon `intersect`).
fn intersect(a: &Segment, b: &Segment) -> Option<[f64; 3]> {
    if cartesian_equal(&a.from, &b.from) || cartesian_equal(&a.from, &b.to) {
        return Some(a.from);
    }
    if cartesian_equal(&a.to, &b.from) || cartesian_equal(&a.to, &b.to) {
        return Some(a.to);
    }
    // Slightly faster lookup when there is no intersection.
    let lc = if a.l + b.l < PI {
        (a.l + b.l).cos() - EPSILON
    } else {
        -1.0
    };
    if cartesian_dot(&a.from, &b.from) < lc
        || cartesian_dot(&a.from, &b.to) < lc
        || cartesian_dot(&a.to, &b.from) < lc
        || cartesian_dot(&a.to, &b.to) < lc
    {
        return None;
    }
    let axb = normalize(cartesian_cross(&a.normal, &b.normal));
    let a0 = cartesian_dot(&axb, &a.from_normal);
    let a1 = cartesian_dot(&axb, &a.to_normal);
    let b0 = cartesian_dot(&axb, &b.from_normal);
    let b1 = cartesian_dot(&axb, &b.to_normal);
    // The candidate lies on both segments, or the antipode does.
    if a0 >= 0.0 && a1 <= 0.0 && b0 >= 0.0 && b1 <= 0.0 {
        Some(axb)
    } else if a0 <= 0.0 && a1 >= 0.0 && b0 <= 0.0 && b1 >= 0.0 {
        Some([-axb[0], -axb[1], -axb[2]])
    } else {
        None
    }
}

/// Whether `p` lies on segment `a` (d3-geo-polygon `intersectPointOnLine`).
fn point_on_line(p: &[f64; 3], a: &Segment) -> bool {
    let a0 = cartesian_dot(p, &a.from_normal);
    let a1 = cartesian_dot(p, &a.to_normal);
    let on = cartesian_dot(p, &a.normal);
    on.abs() < EPSILON2 && (a0 > -EPSILON2 && a1 < EPSILON2 || a0 < EPSILON2 && a1 > -EPSILON2)
}

/// The geodesic distance between two cartesian points.
fn distance(a: &[f64; 3], b: &[f64; 3]) -> f64 {
    let axb = cartesian_cross(a, b);
    cartesian_dot(&axb, &axb).sqrt().atan2(cartesian_dot(a, b))
}

/// The direction of d3-geo-polygon's deterministic nudge.
fn randsign(i: usize, j: i32) -> f64 {
    sign((100.0 * i as f64 + f64::from(j)).sin())
}

/// A point of a clipped line, which a crossing marks with its clip edge and its distance along it.
#[derive(Debug, Clone, Copy)]
struct LinePoint {
    lambda: f64,
    phi: f64,
    crossing: Option<(usize, f64)>,
}

/// Cuts a line into its parts inside the polygon (d3-geo-polygon `clipLine`).
pub(crate) struct PolygonLine {
    segments: Arc<Vec<Segment>>,
    polygon: Arc<Vec<Vec<[f64; 2]>>>,
    point0: Option<[f64; 3]>,
    lambda00: f64,
    phi00: f64,
    v00: bool,
    v0: bool,
    clean: u8,
    line: Vec<LinePoint>,
    lines: Vec<Vec<LinePoint>>,
}

impl PolygonLine {
    fn add(&mut self, mut lambda: f64, mut phi: f64, close: bool) {
        // Move away from ±180°: https://github.com/d3/d3-geo/pull/108#issuecomment-323798937
        if lambda.cos() == -1.0 {
            lambda -= sign(lambda.sin()) * 1e-5;
        }
        if close {
            lambda = self.lambda00;
            phi = self.phi00;
        }
        let mut point = cartesian(lambda * 0.9999999999, phi + 1e-14);
        let mut v = self.v0;
        let segments = self.segments.clone();
        if let Some(point0) = self.point0 {
            struct Crossing {
                lambda: f64,
                phi: f64,
                distance: f64,
                index: usize,
                t: f64,
            }
            let mut crossings: Vec<Crossing> = Vec::new();
            let mut segment = Segment::new(point0, point);
            let (mut i, mut j) = (0usize, 100i32);
            while i < segments.len() && j > 0 {
                let s = &segments[i];
                if let Some(intersection) = intersect(&segment, s) {
                    if cartesian_equal(&intersection, &point0)
                        || cartesian_equal(&intersection, &point)
                        || cartesian_equal(&intersection, &s.from)
                        || cartesian_equal(&intersection, &s.to)
                    {
                        // Nudge the point off the clip edge, and start again.
                        let t = 1e-4;
                        let r = randsign(i, j);
                        lambda = ((lambda + 3.0 * PI + r * t) % TAU) - PI;
                        phi = (HALF_PI - t).min((t - HALF_PI).max(phi + r * t));
                        point = cartesian(lambda, phi);
                        segment = Segment::new(point0, point);
                        i = 0;
                        j -= 1;
                        crossings.clear();
                        continue;
                    }
                    let sph = spherical(&intersection);
                    crossings.push(Crossing {
                        lambda: sph[0],
                        phi: sph[1],
                        distance: distance(&point0, &intersection),
                        index: i,
                        t: distance(&s.from, &intersection),
                    });
                }
                i += 1;
            }
            if !crossings.is_empty() {
                self.clean = 0;
                crossings.sort_by(|a, b| a.distance.total_cmp(&b.distance));
                for crossing in &crossings {
                    v = !v;
                    let point = LinePoint {
                        lambda: crossing.lambda,
                        phi: crossing.phi,
                        crossing: Some((crossing.index, crossing.t)),
                    };
                    if v {
                        self.line = vec![point];
                    } else {
                        self.line.push(point);
                        self.lines.push(std::mem::take(&mut self.line));
                    }
                }
            }
            if v {
                self.line.push(LinePoint {
                    lambda,
                    phi,
                    crossing: None,
                });
            }
        } else {
            let (mut i, mut j) = (0usize, 100i32);
            while i < segments.len() && j > 0 {
                if point_on_line(&point, &segments[i]) {
                    let t = 1e-4;
                    let r = randsign(i, j);
                    lambda = ((lambda + 3.0 * PI + r * t) % TAU) - PI;
                    phi = (HALF_PI - 1e-4).min((1e-4 - HALF_PI).max(phi + r * t));
                    point = cartesian(lambda, phi);
                    i = 0;
                    j -= 1;
                    continue;
                }
                i += 1;
            }
            self.lambda00 = lambda;
            self.phi00 = phi;
            v = polygon_contains(&self.polygon, [lambda, phi]);
            self.v00 = v;
            if v {
                self.line = vec![LinePoint {
                    lambda,
                    phi,
                    crossing: None,
                }];
            }
        }
        self.point0 = Some(point);
        self.v0 = v;
    }
}

impl LineClipper for PolygonLine {
    fn line_start(&mut self, _out: &mut ClipOut<'_>) {
        self.point0 = None;
        self.clean = 1;
        self.line.clear();
    }

    fn point(&mut self, lambda: f64, phi: f64, _m: Option<f64>, _out: &mut ClipOut<'_>) {
        self.add(lambda, phi, false);
    }

    fn close(&mut self, lambda: f64, phi: f64, _out: &mut ClipOut<'_>) {
        self.add(lambda, phi, true);
    }

    fn line_end(&mut self, out: &mut ClipOut<'_>) {
        if self.v0 {
            self.lines.push(std::mem::take(&mut self.line));
        }
        for line in std::mem::take(&mut self.lines) {
            out.line_start();
            for point in line {
                match point.crossing {
                    Some((index, t)) => out.crossing(point.lambda, point.phi, index, t),
                    None => out.point(point.lambda, point.phi, None),
                }
            }
            out.line_end();
        }
    }

    fn clean(&self) -> u8 {
        self.clean | (u8::from(self.v00 && self.v0) << 1)
    }
}

/// Whether `polygon`, in radians in d3's winding, contains `point`: d3-geo-polygon's
/// `polygonContains`, an older d3-geo version without its longitude and pole handling.
fn polygon_contains(polygon: &[Vec<[f64; 2]>], point: [f64; 2]) -> bool {
    let [lambda, phi] = point;
    let normal = [lambda.sin(), -lambda.cos(), 0.0];
    let mut angle = 0.0_f64;
    let mut winding: i64 = 0;
    let mut sum = 0.0_f64;
    for ring in polygon {
        let m = ring.len();
        if m == 0 {
            continue;
        }
        let mut point0 = ring[m - 1];
        let mut lambda0 = point0[0];
        let phi0 = point0[1] / 2.0 + QUARTER_PI;
        let mut sin_phi0 = phi0.sin();
        let mut cos_phi0 = phi0.cos();
        for &point1 in ring {
            let lambda1 = point1[0];
            let phi1 = point1[1] / 2.0 + QUARTER_PI;
            let sin_phi1 = phi1.sin();
            let cos_phi1 = phi1.cos();
            let delta = lambda1 - lambda0;
            let sign = if delta >= 0.0 { 1.0 } else { -1.0 };
            let abs_delta = sign * delta;
            let antimeridian = abs_delta > PI;
            let k = sin_phi0 * sin_phi1;
            sum += (k * sign * abs_delta.sin()).atan2(cos_phi0 * cos_phi1 + k * abs_delta.cos());
            angle += if antimeridian {
                delta + sign * TAU
            } else {
                delta
            };
            // Are the longitudes either side of the point's meridian, and the latitudes smaller
            // than its parallel?
            if antimeridian ^ (lambda0 >= lambda) ^ (lambda1 >= lambda) {
                let arc = normalize(cartesian_cross(
                    &cartesian(point0[0], point0[1]),
                    &cartesian(point1[0], point1[1]),
                ));
                let intersection = normalize(cartesian_cross(&normal, &arc));
                let phi_arc = if antimeridian ^ (delta >= 0.0) {
                    -1.0
                } else {
                    1.0
                } * asin(intersection[2]);
                if phi > phi_arc || (phi == phi_arc && (arc[0] != 0.0 || arc[1] != 0.0)) {
                    winding += if antimeridian ^ (delta >= 0.0) { 1 } else { -1 };
                }
            }
            lambda0 = lambda1;
            sin_phi0 = sin_phi1;
            cos_phi0 = cos_phi1;
            point0 = point1;
        }
    }
    ((angle < -EPSILON) || (angle < EPSILON && sum < -EPSILON)) ^ (winding & 1 != 0)
}
