//! Polar projections, whose positions are angles clockwise from the top and distances from the
//! center of a circle.

use crate::projection::PlotProjection;

/// How far apart, in degrees, the angles of a whole circle may lie.
const SAME_ANGLE: f32 = 1e-3;

/// The largest step between the points of a path at a steady change of angle, in degrees.
const SWEEP_STEP: f64 = 2.0;

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

    /// The points from `from` to `to` at a steady change of angle and radius: an arc at one
    /// radius and a spoke at one angle.
    fn path(&self, from: [f64; 2], to: [f64; 2]) -> Vec<Vec<[f32; 2]>> {
        let steps = ((to[0] - from[0]).abs() / SWEEP_STEP).ceil().max(1.0) as usize;
        let points = (0..=steps).map(|step| {
            let along = step as f64 / steps as f64;
            let [angle, radius] = [0, 1].map(|axis| from[axis] + (to[axis] - from[axis]) * along);
            self.point(angle, radius)
        });
        vec![points.collect()]
    }
}
