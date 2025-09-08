//! Integration test to verify external coordinate systems can be defined and used

use avenger_chart::{
    axis::Axis,
    coords::{CoordinateSystem, OverflowSpaceRequirement},
    define_common_mark_channels, define_position_channels,
    error::AvengerChartError,
    impl_mark_base, impl_mark_trait_common,
    marks::{ChannelValue, Mark, MarkState},
    render::Padding,
    scales::{Auto, Scale},
};
use avenger_scenegraph::marks::group::Clip;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::logical_expr::{lit, Expr};
use datafusion::scalar::ScalarValue;
use std::any::Any;
use std::collections::HashMap;
use std::marker::PhantomData;

/// A custom 3D isometric coordinate system defined in an external crate
/// This maps 3D coordinates (x, y, z) to 2D screen space using isometric projection
#[derive(Clone)]
pub struct Isometric {
    /// Angle for isometric projection (typically 30 degrees)
    angle: f64,
}

impl Default for Isometric {
    fn default() -> Self {
        Self {
            angle: std::f64::consts::PI / 6.0, // 30 degrees
        }
    }
}

impl Isometric {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Configuration for Isometric position channels (iso_x, iso_y, iso_z)
/// These channels support scales and axes
#[derive(Clone)]
pub struct IsometricPositionConfig {
    pub(crate) inner: ChannelValue,
    pub(crate) axis_config:
        Option<std::sync::Arc<dyn Fn(IsometricAxis) -> IsometricAxis + Send + Sync>>,
}

impl IsometricPositionConfig {
    /// Create a new position channel from a channel value
    pub fn new(value: ChannelValue) -> Self {
        Self {
            inner: value,
            axis_config: None,
        }
    }

    /// Configure the scale for this channel
    pub fn scale<F>(self, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        Self {
            inner: self.inner.scale(f),
            axis_config: self.axis_config,
        }
    }

    /// Configure the axis for this channel
    pub fn axis<F>(mut self, f: F) -> Self
    where
        F: Fn(IsometricAxis) -> IsometricAxis + Send + Sync + 'static,
    {
        self.axis_config = Some(std::sync::Arc::new(f));
        self
    }

    /// Get the inner ChannelValue
    pub fn into_inner(self) -> ChannelValue {
        self.inner
    }
}

// Implement the PositionConfig trait to work with the macro
impl avenger_chart::cartesian::channels::PositionConfig for IsometricPositionConfig {
    type Axis = IsometricAxis;

    fn new(value: ChannelValue) -> Self {
        Self {
            inner: value,
            axis_config: None,
        }
    }

    fn take_axis_config(
        self,
    ) -> (
        ChannelValue,
        Option<std::sync::Arc<dyn Fn(IsometricAxis) -> IsometricAxis + Send + Sync>>,
    ) {
        (self.inner, self.axis_config)
    }

    fn into_inner(self) -> ChannelValue {
        self.inner
    }
}

// Conversions from various types
impl From<ChannelValue> for IsometricPositionConfig {
    fn from(value: ChannelValue) -> Self {
        Self::new(value)
    }
}

impl From<datafusion::logical_expr::Expr> for IsometricPositionConfig {
    fn from(expr: datafusion::logical_expr::Expr) -> Self {
        Self::new(ChannelValue::from(expr))
    }
}

impl From<&str> for IsometricPositionConfig {
    fn from(s: &str) -> Self {
        Self::new(ChannelValue::from(s))
    }
}

/// Custom axis type for the isometric coordinate system
#[derive(Clone, Debug)]
pub struct IsometricAxis {
    pub channel: String,
    pub visible: bool,
}

impl Axis for IsometricAxis {
    fn clone_box(&self) -> Box<dyn Axis> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

#[async_trait::async_trait]
impl CoordinateSystem for Isometric {
    type Axis = IsometricAxis;

    fn required_channels(&self) -> &'static [&'static str] {
        &["iso_x", "iso_y", "iso_z"]
    }

    fn default_range(&self, channel: &str, width: f64, height: f64) -> Option<(f64, f64)> {
        match channel {
            "iso_x" => Some((0.0, width * 0.8)),
            "iso_y" => Some((0.0, height * 0.8)),
            "iso_z" => Some((0.0, height * 0.4)),
            _ => None,
        }
    }

    fn transform_to_plot_coords(
        &self,
        position_channels: &HashMap<&str, avenger_common::value::ScalarOrArray<f32>>,
        _plot_width: f32,
        _plot_height: f32,
    ) -> Result<(avenger_common::value::ScalarOrArray<f32>, avenger_common::value::ScalarOrArray<f32>), AvengerChartError> {
        use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
        
        // Get the position channel values
        let x = position_channels
            .get("iso_x")
            .ok_or_else(|| AvengerChartError::MissingChannelError("iso_x".to_string()))?;
        let y = position_channels
            .get("iso_y")
            .ok_or_else(|| AvengerChartError::MissingChannelError("iso_y".to_string()))?;
        let z = position_channels
            .get("iso_z")
            .ok_or_else(|| AvengerChartError::MissingChannelError("iso_z".to_string()))?;

        // Isometric projection formulas:
        // screen_x = (x - y) * cos(angle)
        // screen_y = (x + y) * sin(angle) - z
        let cos_angle = self.angle.cos() as f32;
        let sin_angle = self.angle.sin() as f32;

        // Compute screen coordinates based on scalar/array combinations
        let screen_x = match (x.value(), y.value()) {
            (ScalarOrArrayValue::Scalar(x_val), ScalarOrArrayValue::Scalar(y_val)) => {
                ScalarOrArray::new_scalar((x_val - y_val) * cos_angle)
            }
            (ScalarOrArrayValue::Array(x_arr), ScalarOrArrayValue::Scalar(y_val)) => {
                let result: Vec<f32> = x_arr.iter().map(|x| (x - y_val) * cos_angle).collect();
                ScalarOrArray::new_array(result)
            }
            (ScalarOrArrayValue::Scalar(x_val), ScalarOrArrayValue::Array(y_arr)) => {
                let result: Vec<f32> = y_arr.iter().map(|y| (x_val - y) * cos_angle).collect();
                ScalarOrArray::new_array(result)
            }
            (ScalarOrArrayValue::Array(x_arr), ScalarOrArrayValue::Array(y_arr)) => {
                let result: Vec<f32> = x_arr
                    .iter()
                    .zip(y_arr.iter())
                    .map(|(x, y)| (x - y) * cos_angle)
                    .collect();
                ScalarOrArray::new_array(result)
            }
        };

        let screen_y = match (x.value(), y.value(), z.value()) {
            (ScalarOrArrayValue::Scalar(x_val), ScalarOrArrayValue::Scalar(y_val), ScalarOrArrayValue::Scalar(z_val)) => {
                ScalarOrArray::new_scalar((x_val + y_val) * sin_angle - z_val)
            }
            _ => {
                // For simplicity in this test, just handle the scalar case
                // A full implementation would handle all combinations
                let x_vals = x.as_vec(1, None);
                let y_vals = y.as_vec(1, None);
                let z_vals = z.as_vec(1, None);
                
                let result: Vec<f32> = (0..x_vals.len())
                    .map(|i| (x_vals[i] + y_vals[i]) * sin_angle - z_vals[i])
                    .collect();
                ScalarOrArray::new_array(result)
            }
        };

        Ok((screen_x, screen_y))
    }

    fn create_default_axes(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _marks: &[Box<dyn avenger_chart::marks::Mark<Self>>],
    ) -> HashMap<String, Self::Axis>
    where
        Self: Sized,
    {
        let mut axes = HashMap::new();

        // Create axes for each channel that has a scale
        for channel in ["iso_x", "iso_y", "iso_z"] {
            if scales.contains_key(channel) {
                axes.insert(
                    channel.to_string(),
                    IsometricAxis {
                        channel: channel.to_string(),
                        visible: true,
                    },
                );
            }
        }

        axes
    }

    async fn measure_guide_overflow(
        &self,
        _axes: HashMap<String, Self::Axis>,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _width_estimate: f32,
        _height_estimate: f32,
        _theme: &avenger_chart::theme::Theme,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        // For simplicity, assume isometric axes don't overflow
        Ok(OverflowSpaceRequirement {
            top: 0.0,
            bottom: 0.0,
            left: 0.0,
            right: 0.0,
        })
    }

    async fn render_axes(
        &self,
        _axes: &HashMap<String, Self::Axis>,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        _padding: &Padding,
        _theme: &avenger_chart::theme::Theme,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // For this test, we don't need to actually render axes
        Ok(vec![])
    }

    fn get_clip(
        &self,
        plot_width: f32,
        plot_height: f32,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> Clip {
        // Use rectangular clipping for isometric view
        Clip::Rect {
            x: 0.0,
            y: 0.0,
            width: plot_width,
            height: plot_height,
        }
    }
}

/// A custom cube mark for the isometric coordinate system
pub struct Cube<C: CoordinateSystem> {
    state: MarkState<C>,
    _phantom: PhantomData<C>,
}

// Use the exported macro for base implementation
impl_mark_base!(Cube);

// Define common channels
define_common_mark_channels! {
    Cube {
        fill: {},
        stroke: {},
        opacity: {},
        size: {},
    }
}

// Define position channels for Isometric Cube using the macro
define_position_channels! {
    Cube<Isometric> {
        iso_x: {
            required: true,
            with_config: IsometricPositionConfig,
        },
        iso_y: {
            required: true,
            with_config: IsometricPositionConfig,
        },
        iso_z: {
            required: true,
            with_config: IsometricPositionConfig,
        }
    }
}

// Implement the Mark trait for Isometric
impl Mark<Isometric> for Cube<Isometric> {
    impl_mark_trait_common!(Cube, Isometric, "cube");

    fn render_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
        _context: &avenger_chart::render_context::RenderContext,
        _coord: &Isometric,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Custom cube rendering logic would go here
        // For this test, we just return an empty vector
        Ok(vec![])
    }

    fn default_channel_value(
        &self,
        channel: &str,
        _context: &avenger_chart::render_context::RenderContext,
    ) -> Option<ScalarValue> {
        match channel {
            "size" => Some(ScalarValue::Float32(Some(10.0))),
            "fill" => Some(ScalarValue::Utf8(Some("#3498db".to_string()))),
            _ => None,
        }
    }
}
