//! Cartesian projections, whose positions are the pixels of a plot's rectangle.

use crate::projection::PlotProjection;

/// The projection of a Cartesian plot of `size`, whose range positions are its pixels from its
/// top-left corner, as the plot's scales of `x` and `y` take them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CartesianProjection {
    pub size: [f32; 2],
}

impl CartesianProjection {
    pub fn new(size: [f32; 2]) -> Self {
        Self { size }
    }
}

impl PlotProjection for CartesianProjection {
    /// Every pixel has a position, so a gesture that leaves the plot clamps to its extent.
    fn position(&self, pixel: [f32; 2]) -> Option<[f64; 2]> {
        Some(pixel.map(f64::from))
    }

    fn pixel(&self, position: [f64; 2]) -> Option<[f32; 2]> {
        Some(position.map(|value| value as f32))
    }

    fn extent(&self) -> [[f64; 2]; 2] {
        self.size.map(|length| [0.0, f64::from(length)])
    }

    fn is_linear(&self) -> bool {
        true
    }

    fn path(&self, from: [f64; 2], to: [f64; 2]) -> Vec<Vec<[f32; 2]>> {
        vec![vec![
            from.map(|value| value as f32),
            to.map(|value| value as f32),
        ]]
    }

    fn polygon(&self, ring: &[[f64; 2]]) -> Vec<Vec<[f32; 2]>> {
        if ring.len() < 3 {
            return Vec::new();
        }
        vec![(ring.iter())
            .map(|position| position.map(|value| value as f32))
            .collect()]
    }
}
