//! The mapping between an area's pixels and the domains of its scales.

use std::sync::Arc;

use arrow_array::{ArrayRef, Float64Array};
use avenger_layout::Rect;
use avenger_scales::error::AvengerScaleError;
use avenger_scales::scales::ConfiguredScale;

use super::draw::Pixels;

pub(crate) type ScaleOf<State, A> = Box<dyn Fn(&State, &A) -> ConfiguredScale + Send + Sync>;

/// The axes of an area that a brush covers.
pub(crate) type Axes = [Option<Axis>; 2];

/// Maps one axis of an area between pixels and its scale's domain. The scale's range, from low
/// to high, spans the area from its low edge to its high edge.
pub(crate) struct Axis {
    scale: ConfiguredScale,
    start: f32,
    length: f32,
    range: [f64; 2],
}

impl Axis {
    pub(crate) fn new(
        scale: ConfiguredScale,
        [start, end]: [f32; 2],
    ) -> Result<Self, AvengerScaleError> {
        let (a, b) = scale.numeric_interval_range()?;
        Ok(Self {
            scale,
            start,
            length: end - start,
            range: [a.min(b), a.max(b)],
        })
    }

    /// The domain value at `pixel`.
    pub(crate) fn value(&self, pixel: f32) -> Result<f64, AvengerScaleError> {
        let [low, high] = self.range;
        let fraction = (f64::from(pixel) - f64::from(self.start)) / f64::from(self.length);
        self.scale.invert_scalar(low + fraction * (high - low))
    }

    /// The pixel of the domain value `value`.
    pub(crate) fn pixel(&self, value: f64) -> Result<f32, AvengerScaleError> {
        let values: ArrayRef = Arc::new(Float64Array::from(vec![value]));
        let scaled = self.scale.scale_to_numeric(&values)?;
        let position = f64::from(scaled.first().copied().unwrap_or(f32::NAN));
        let [low, high] = self.range;
        Ok(self.start + ((position - low) / (high - low)) as f32 * self.length)
    }
}

/// The pixels that `area` spans along each axis.
pub(crate) fn spans(area: Rect) -> Pixels {
    [[area.x, area.right()], [area.y, area.bottom()]]
}
