//! Polar projections, whose positions are angles clockwise from the top and distances from the
//! center of a circle.

use crate::projection::PlotProjection;

/// How far apart, in degrees, the angles of a whole circle may lie.
const SAME_ANGLE: f32 = 1e-3;

/// The projection of a polar plot at one size: its circle's center, in pixels from the plot's
/// top-left corner, its inner and outer radius in pixels, and the start and end angles of its part
/// of the circle, in degrees clockwise from the top. Its range positions are an angle, in degrees
/// clockwise from the top, and a distance from the center, in pixels, which the ranges of the
/// scales of `theta` and `radius` hold. The angle of a whole circle wraps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PolarProjection {
    pub center: [f32; 2],
    /// The inner radius and the outer radius.
    pub radii: [f32; 2],
    /// The start angle and the end angle of the part of the circle.
    pub angles: [f32; 2],
}

impl PolarProjection {
    pub fn new(center: [f32; 2], radii: [f32; 2], angles: [f32; 2]) -> Self {
        Self {
            center,
            radii,
            angles,
        }
    }

    /// Whether the part is the whole circle, whose end is its start.
    fn full(&self) -> bool {
        self.angles[1] - self.angles[0] >= 360.0 - SAME_ANGLE
    }

    /// The pixel at `angle` degrees clockwise from the top and `radius` pixels from the center.
    fn point(&self, angle: f64, radius: f64) -> [f32; 2] {
        let (sin, cos) = angle.to_radians().sin_cos();
        let [x, y] = self.center.map(f64::from);
        [(x + radius * sin) as f32, (y - radius * cos) as f32]
    }
}

impl PlotProjection for PolarProjection {
    /// The angle and the distance of `pixel` from the center, with the angle within half a turn
    /// of the middle of the part of the circle, so the angles of a whole circle run from its
    /// start angle up to a turn past it, and an angle outside a part of the circle lies past its
    /// nearer end. Every pixel has a position, so a gesture that leaves the part of the circle
    /// clamps to its extent.
    fn position(&self, pixel: [f32; 2]) -> Option<[f64; 2]> {
        let [x, y] = [0, 1].map(|axis| f64::from(pixel[axis] - self.center[axis]));
        let low = f64::from(self.angles[0] + self.angles[1]) / 2.0 - 180.0;
        let angle = x.atan2(-y).to_degrees();
        Some([low + (angle - low).rem_euclid(360.0), x.hypot(y)])
    }

    fn pixel(&self, [angle, radius]: [f64; 2]) -> Option<[f32; 2]> {
        Some(self.point(angle, radius))
    }

    fn extent(&self) -> [[f64; 2]; 2] {
        [self.angles.map(f64::from), self.radii.map(f64::from)]
    }

    fn period(&self, dimension: usize) -> Option<f64> {
        (dimension == 0 && self.full()).then_some(360.0)
    }

    /// The clockwise direction for the angle and the outward direction for the radius, which hold
    /// at the center too.
    fn tangents(&self, [angle, _]: [f64; 2]) -> Option<[[f32; 2]; 2]> {
        let (sin, cos) = angle.to_radians().sin_cos();
        Some([[cos as f32, sin as f32], [sin as f32, -cos as f32]])
    }

    /// The part of the circle between the radii: for a whole circle, the outer circle and, with an
    /// inner radius, the inner circle wound the other way, and for a part, one ring along the
    /// outer arc and back along the inner arc, or through the center without an inner radius.
    fn outline(&self) -> Vec<Vec<[f32; 2]>> {
        let [start, end] = self.angles.map(f64::from);
        let [inner, outer] = self.radii.map(f64::from);
        let arc = |radius: f64, from: f64, to: f64| -> Vec<[f32; 2]> {
            self.path([from, radius], [to, radius]).concat()
        };
        if self.full() {
            let mut rings = vec![close(arc(outer, start, start + 360.0))];
            if inner > 0.0 {
                rings.push(close(arc(inner, start + 360.0, start)));
            }
            return rings;
        }
        let mut ring = arc(outer, start, end);
        if inner > 0.0 {
            ring.extend(arc(inner, end, start));
        } else {
            ring.push(self.center);
        }
        vec![ring]
    }

    /// The box of the arcs exactly: their ends and each quarter turn between them, at the outer
    /// radius and at the inner radius, or the center without one.
    fn bounds(&self) -> Option<[[f32; 2]; 2]> {
        let [start, end] = self.angles.map(f64::from);
        let mut angles = vec![start, end];
        let mut quarter = (start / 90.0).ceil() * 90.0;
        while quarter <= end {
            angles.push(quarter);
            quarter += 90.0;
        }
        let [inner, outer] = self.radii.map(f64::from);
        let mut points: Vec<[f32; 2]> = angles.iter().map(|a| self.point(*a, outer)).collect();
        if inner > 0.0 {
            points.extend(angles.iter().map(|a| self.point(*a, inner)));
        } else {
            points.push(self.center);
        }
        let mut bounds = [[f32::INFINITY; 2], [f32::NEG_INFINITY; 2]];
        for [x, y] in points {
            bounds = [
                [bounds[0][0].min(x), bounds[0][1].min(y)],
                [bounds[1][0].max(x), bounds[1][1].max(y)],
            ];
        }
        Some(bounds)
    }
}

/// `ring` without its last point where that point repeats its first, as a whole circle's path
/// ends where it starts.
fn close(mut ring: Vec<[f32; 2]>) -> Vec<[f32; 2]> {
    let repeats = match (ring.first(), ring.last()) {
        (Some(first), Some(last)) => {
            (first[0] - last[0]).abs() < 1e-3 && (first[1] - last[1]).abs() < 1e-3
        }
        _ => false,
    };
    if repeats && ring.len() > 1 {
        ring.pop();
    }
    ring
}
