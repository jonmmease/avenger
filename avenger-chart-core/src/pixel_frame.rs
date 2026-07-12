//! Pixel-coordinate frame for chart widgets and other screen-space marks.

use std::{any::Any, collections::HashMap};

use avenger_common::value::ScalarOrArray;
use avenger_scales::scales::ScaleImpl;
use datafusion::common::ScalarValue;
use serde::{Deserialize, Serialize};

use crate::{
    AvengerChartError, CoordinateSystem, CoordinateSystemCore, CoordinateSystemTransform,
    CoordinateSystemTransformCore, NoGuide, PlotGeometry, PointGeometry,
};

/// A frame whose positional channel values are already logical pixels.
///
/// `x`, `y`, `x2`, and `y2` bypass scale evaluation. Non-position channels
/// retain the normal chart scale pipeline.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct PixelFrame;

impl PixelFrame {
    pub const fn new() -> Self {
        Self
    }
}

impl CoordinateSystemCore for PixelFrame {
    fn required_channels(&self) -> &'static [&'static str] {
        &["x", "y"]
    }
}

impl CoordinateSystem for PixelFrame {
    type Guide = NoGuide;

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(*self)
    }
}

impl CoordinateSystemTransformCore for PixelFrame {
    fn required_channels(&self) -> &'static [&'static str] {
        &["x", "y"]
    }

    fn channel_uses_scale(&self, channel: &str) -> bool {
        !matches!(channel, "x" | "y" | "x2" | "y2")
    }

    fn transform(
        &self,
        position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        _position_values: Option<&HashMap<&str, Vec<ScalarValue>>>,
        _plot_width: f32,
        _plot_height: f32,
    ) -> Result<Box<dyn PlotGeometry>, AvengerChartError> {
        let x = position_channels.get("x").cloned().ok_or_else(|| {
            AvengerChartError::InternalError("Missing x pixel position channel".to_string())
        })?;
        let y = position_channels.get("y").cloned().ok_or_else(|| {
            AvengerChartError::InternalError("Missing y pixel position channel".to_string())
        })?;
        Ok(Box::new(PointGeometry { x, y }))
    }

    fn default_scale_options(
        &self,
        _channel: &str,
        _scale_impl: &dyn ScaleImpl,
    ) -> HashMap<String, ScalarValue> {
        HashMap::new()
    }
}

#[typetag::serde]
impl CoordinateSystemTransform for PixelFrame {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(*self)
    }
}

#[cfg(test)]
mod tests {
    use avenger_common::value::ScalarOrArrayValue;

    use super::*;

    #[test]
    fn positional_channels_bypass_scales_only() {
        let frame = PixelFrame;
        for channel in ["x", "y", "x2", "y2"] {
            assert!(!frame.channel_uses_scale(channel));
        }
        for channel in ["fill", "stroke", "opacity", "size", "text"] {
            assert!(frame.channel_uses_scale(channel));
        }
    }

    #[test]
    fn transform_passes_pixels_through() {
        let frame = PixelFrame;
        let channels = HashMap::from([
            ("x", ScalarOrArray::new_array(vec![3.0, 17.5])),
            ("y", ScalarOrArray::new_array(vec![4.0, 22.25])),
        ]);
        let geometry = frame.transform(&channels, None, 800.0, 600.0).unwrap();
        let points = geometry.as_any().downcast_ref::<PointGeometry>().unwrap();
        assert!(matches!(
            points.x.value(),
            ScalarOrArrayValue::Array(values) if values.as_slice() == [3.0, 17.5]
        ));
        assert!(matches!(
            points.y.value(),
            ScalarOrArrayValue::Array(values) if values.as_slice() == [4.0, 22.25]
        ));
    }

    #[test]
    fn boxed_transform_round_trips() {
        let transform: Box<dyn CoordinateSystemTransform> = Box::new(PixelFrame);
        let bytes = bincode::serialize(&transform).unwrap();
        let decoded: Box<dyn CoordinateSystemTransform> = bincode::deserialize(&bytes).unwrap();
        assert!(decoded.as_any().is::<PixelFrame>());
    }
}
