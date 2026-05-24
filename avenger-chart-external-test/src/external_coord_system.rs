//! Integration test to verify external coordinate systems can be defined and used

use std::{any::Any, collections::HashMap, marker::PhantomData, sync::Arc};

use async_trait::async_trait;
use avenger_chart::{
    coords::{CoordinateSystem, CoordinateSystemTransform},
    define_position_channels,
    guide::{CoordinateGuide, GuideSharingContext},
    impl_mark_trait_common,
    marks::{CompiledMark, Mark},
    render::RenderContext,
};
use avenger_chart_core::{
    define_common_mark_channels, impl_mark_base, AvengerChartError, Axis, ChannelDescriptor,
    ChannelValue, CompiledDataContext, CompiledMarkState, CoordMeasurement, CoordinateSystemCore,
    CoordinateSystemTransformCore, MarkState, OverflowSpaceRequirement, PlotGeometry,
    PointGeometry, PositionConfig,
};
use avenger_chart_scales::{Auto, Scale, ScaleChannelValue};
use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::{arrow::record_batch::RecordBatch, scalar::ScalarValue};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

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
impl PositionConfig for IsometricPositionConfig {
    type Axis = IsometricAxis;

    fn new(value: ChannelValue) -> Self {
        Self {
            inner: value,
            axis_config: None,
        }
    }

    fn take_axis_config(self) -> (ChannelValue, Option<IsometricAxis>) {
        // For now, return a default axis if config exists
        // In a real implementation, you'd apply the config function
        let axis = self.axis_config.map(|config_fn| {
            // Apply the config function to a default axis
            config_fn(IsometricAxis {
                channel: "".to_string(),
                visible: true,
            })
        });
        (self.inner, axis)
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
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IsometricAxis {
    pub channel: String,
    pub visible: bool,
}

#[typetag::serde]
impl Axis for IsometricAxis {
    fn update(&mut self, other: &dyn Axis) {
        if let Some(other_iso) = other.as_any().downcast_ref::<IsometricAxis>() {
            self.visible = other_iso.visible;
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn box_clone(&self) -> Box<dyn Axis> {
        Box::new(self.clone())
    }
}

/// Options for Isometric coordinate system
#[derive(Clone, Debug)]
pub struct IsometricOptions {
    /// Angle for isometric projection (typically 30 degrees)
    pub angle: f64,
}

impl Default for IsometricOptions {
    fn default() -> Self {
        Self {
            angle: std::f64::consts::PI / 6.0, // 30 degrees
        }
    }
}

/// Guide for Isometric coordinate system
#[derive(Clone, Debug)]
pub struct IsometricGuide {
    /// Axes configured at the channel level
    pub axes: HashMap<String, IsometricAxis>,
    /// Coordinate-system-level options
    pub options: IsometricOptions,
}

impl Default for IsometricGuide {
    fn default() -> Self {
        Self::new()
    }
}

impl IsometricGuide {
    pub fn new() -> Self {
        Self {
            axes: HashMap::new(),
            options: IsometricOptions::default(),
        }
    }
}

/// Compiled version of the isometric guide for rendering
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledIsometricGuide {
    axes: HashMap<String, IsometricAxis>,
}

#[async_trait::async_trait]
#[typetag::serde]
impl avenger_chart::guide::CompiledGuide for CompiledIsometricGuide {
    async fn measure_overflow(
        &self,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        _theme: &avenger_chart::theme::Theme,
        _params: &IndexMap<String, ScalarValue>,
        _data_override: Option<&datafusion::dataframe::DataFrame>,
        _ctx: &datafusion::prelude::SessionContext,
        _sharing_context: GuideSharingContext<'_>,
        _coord_measurement: Option<&dyn CoordMeasurement>,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        Ok(OverflowSpaceRequirement {
            top: 0.0,
            bottom: 0.0,
            left: 0.0,
            right: 0.0,
        })
    }

    async fn evaluate(
        &self,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        _plot_bounds: &avenger_chart::layout::LayoutBounds,
        _guide_overflow: &OverflowSpaceRequirement,
        _theme: &avenger_chart::theme::Theme,
        _params: &IndexMap<String, ScalarValue>,
        _ctx: &datafusion::prelude::SessionContext,
        _data_override: Option<&datafusion::dataframe::DataFrame>,
        _sharing_context: GuideSharingContext<'_>,
        _coord_measurement: &dyn CoordMeasurement,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Ok(vec![])
    }

    fn get_clip(
        &self,
        plot_width: f32,
        plot_height: f32,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> avenger_scenegraph::marks::group::Clip {
        avenger_scenegraph::marks::group::Clip::Rect {
            x: 0.0,
            y: 0.0,
            width: plot_width,
            height: plot_height,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl CoordinateGuide for IsometricGuide {
    type Axis = IsometricAxis;

    fn set_axes(&mut self, axes: HashMap<String, Self::Axis>) {
        self.axes = axes;
    }

    fn set_compiled_marks(
        &mut self,
        _compiled_marks: Vec<std::sync::Arc<dyn CompiledMark>>,
        _session_context: &datafusion::prelude::SessionContext,
    ) {
        // Store mark renderers if needed for axis titles
    }

    fn update(&mut self, other: Self) {
        self.axes = other.axes;
        self.options = other.options;
    }

    fn build(self) -> Box<dyn avenger_chart::guide::CompiledGuide> {
        // For this example, we'll just return a dummy compiled guide
        Box::new(CompiledIsometricGuide { axes: self.axes })
    }
}

impl CoordinateSystemCore for Isometric {
    fn required_channels(&self) -> &'static [&'static str] {
        &["iso_x", "iso_y", "iso_z"]
    }
}

impl CoordinateSystem for Isometric {
    type Guide = IsometricGuide;

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(IsometricTransform { angle: self.angle })
    }
}

/// Transform implementation for the isometric coordinate system
#[derive(Clone, Serialize, Deserialize)]
struct IsometricTransform {
    angle: f64,
}

impl CoordinateSystemTransformCore for IsometricTransform {
    fn required_channels(&self) -> &'static [&'static str] {
        &["iso_x", "iso_y", "iso_z"]
    }

    fn default_range(
        &self,
        channel: &str,
        plot_area_width: f64,
        plot_area_height: f64,
    ) -> Option<(f64, f64)> {
        match channel {
            "iso_x" => Some((0.0, plot_area_width * 0.8)),
            "iso_y" => Some((0.0, plot_area_height * 0.8)),
            "iso_z" => Some((0.0, plot_area_height * 0.4)),
            _ => None,
        }
    }

    fn default_scale_options(
        &self,
        _channel: &str,
        _scale_impl: &dyn avenger_scales::scales::ScaleImpl,
    ) -> HashMap<String, ScalarValue> {
        // Return default scale options for this coordinate system
        HashMap::new()
    }

    fn transform(
        &self,
        position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        _position_values: Option<&HashMap<&str, Vec<ScalarValue>>>,
        _plot_width: f32,
        _plot_height: f32,
    ) -> Result<Box<dyn PlotGeometry>, AvengerChartError> {
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
            (
                ScalarOrArrayValue::Scalar(x_val),
                ScalarOrArrayValue::Scalar(y_val),
                ScalarOrArrayValue::Scalar(z_val),
            ) => ScalarOrArray::new_scalar((x_val + y_val) * sin_angle - z_val),
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

        Ok(Box::new(PointGeometry {
            x: screen_x,
            y: screen_y,
        }))
    }
}

#[async_trait::async_trait]
#[typetag::serde]
impl CoordinateSystemTransform for IsometricTransform {
    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

/// A custom cube mark for the isometric coordinate system
pub struct Cube<C> {
    pub(crate) state: MarkState,
    pub(crate) _phantom: PhantomData<C>,
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
#[async_trait]
impl Mark<Isometric> for Cube<Isometric> {
    impl_mark_trait_common!(Cube);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledIsometricCube {
            state: compiled_state,
        }))
    }
}

/// Compiled version of Cube mark for rendering
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledIsometricCube {
    pub(crate) state: CompiledMarkState,
}

#[typetag::serde]
#[async_trait]
impl CompiledMark for CompiledIsometricCube {
    fn state(&self) -> &CompiledMarkState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut CompiledMarkState {
        &mut self.state
    }

    fn data_context(&self) -> &CompiledDataContext {
        &self.state.data
    }

    fn mark_type(&self) -> &str {
        "cube"
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        vec![
            ChannelDescriptor {
                name: "iso_x",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "iso_y",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "iso_z",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "size",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "fill",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "stroke",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "opacity",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
        ]
    }

    async fn render_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
        _context: &RenderContext,
        _coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Custom cube rendering logic would go here
        // For this test, we just return an empty vector
        Ok(vec![])
    }

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        match channel {
            "size" => Some(ScalarValue::Float32(Some(10.0))),
            "fill" => Some(ScalarValue::Utf8(Some("#3498db".to_string()))),
            "stroke" => Some(ScalarValue::Utf8(Some("#000000".to_string()))),
            "opacity" => Some(ScalarValue::Float32(Some(1.0))),
            _ => None,
        }
    }
}
