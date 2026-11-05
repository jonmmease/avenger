//! Cartesian projections, whose positions are the pixels of a plot's rectangle.

use arrow::array::Float32Array;
use arrow::buffer::{NullBuffer, ScalarBuffer};

use crate::positions::Positions;
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

    /// The positions themselves, which reuse the buffer of a Float32 dimension, null where either
    /// dimension is null.
    fn pixels(&self, positions: [Positions<'_>; 2]) -> [Float32Array; 2] {
        let [x, y] = positions;
        assert_eq!(
            x.len(),
            y.len(),
            "the dimensions of positions have one length"
        );
        let nulls = NullBuffer::union(x.nulls(), y.nulls());
        [x, y].map(|positions| {
            let values: ScalarBuffer<f32> = match positions {
                Positions::F32(array) => array.values().clone(),
                Positions::F64(array) => array.values().iter().map(|v| *v as f32).collect(),
            };
            Float32Array::new(values, nulls.clone())
        })
    }

    fn extent(&self) -> [[f64; 2]; 2] {
        self.size.map(|length| [0.0, f64::from(length)])
    }

    fn is_linear(&self) -> bool {
        true
    }

    /// The pixel axes: positions are pixels, so x grows rightward and y downward.
    fn tangents(&self, _position: [f64; 2]) -> Option<[[f32; 2]; 2]> {
        Some([[1.0, 0.0], [0.0, 1.0]])
    }
}
