//! Integration test to verify external coordinate systems can be defined and used

use avenger_chart::{
    axis::Axis,
    coords::{CoordinateSystem, OverflowSpaceRequirement, TransformResult},
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

    fn transform_expressions(
        &self,
        channels: HashMap<String, Expr>,
    ) -> Result<TransformResult, AvengerChartError> {
        // Get the position channel expressions
        let x_expr = channels
            .get("iso_x")
            .ok_or_else(|| AvengerChartError::MissingChannelError("iso_x".to_string()))?;
        let y_expr = channels
            .get("iso_y")
            .ok_or_else(|| AvengerChartError::MissingChannelError("iso_y".to_string()))?;
        let z_expr = channels
            .get("iso_z")
            .ok_or_else(|| AvengerChartError::MissingChannelError("iso_z".to_string()))?;

        // Isometric projection formulas:
        // screen_x = (x - y) * cos(angle)
        // screen_y = (x + y) * sin(angle) - z
        let cos_angle = lit(self.angle.cos());
        let sin_angle = lit(self.angle.sin());

        let screen_x = (x_expr.clone() - y_expr.clone()) * cos_angle;
        let screen_y = (x_expr.clone() + y_expr.clone()) * sin_angle - z_expr.clone();

        Ok(TransformResult {
            x: screen_x,
            y: screen_y,
            depth: Some(z_expr.clone()), // Use z for depth ordering
        })
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
        _width: f32,
        _height: f32,
        _plot_area_ratio: f32,
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
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Custom cube rendering logic would go here
        // For this test, we just return an empty vector
        Ok(vec![])
    }

    fn default_channel_value(&self, channel: &str, _context: &avenger_chart::render_context::RenderContext) -> Option<ScalarValue> {
        match channel {
            "size" => Some(ScalarValue::Float32(Some(10.0))),
            "fill" => Some(ScalarValue::Utf8(Some("#3498db".to_string()))),
            _ => None,
        }
    }
}
