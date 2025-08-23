//! Integration test to verify external coordinate systems can be defined and used

use avenger_chart::{
    axis::AxisTrait,
    coords::{CoordinateSystem, OverflowSpaceRequirement, TransformResult},
    define_common_mark_channels, define_position_mark_channels,
    error::AvengerChartError,
    impl_mark_base, impl_mark_trait_common,
    marks::{ChannelType, Mark, MarkState},
    plot::Plot,
    render::Padding,
};
use avenger_scenegraph::marks::group::Clip;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::logical_expr::{Expr, col, lit};
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

impl Isometric {
    pub fn new() -> Self {
        Self {
            angle: std::f64::consts::PI / 6.0, // 30 degrees
        }
    }
}

/// Custom axis type for the isometric coordinate system
#[derive(Clone, Debug)]
pub struct IsometricAxis {
    pub channel: String,
    pub visible: bool,
}

impl AxisTrait for IsometricAxis {
    fn clone_box(&self) -> Box<dyn AxisTrait> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
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
        fill: { type: ChannelType::Color },
        stroke: { type: ChannelType::Color },
        opacity: { type: ChannelType::Numeric },
        size: { type: ChannelType::Numeric },
    }
}

// Define position channels for Isometric
define_position_mark_channels! {
    Cube<Isometric> {
        iso_x: { type: ChannelType::Numeric, required: true },
        iso_y: { type: ChannelType::Numeric, required: true },
        iso_z: { type: ChannelType::Numeric, required: true },
    }
}

// Implement the Mark trait for Isometric
impl Mark<Isometric> for Cube<Isometric> {
    impl_mark_trait_common!(Cube, Isometric, "cube");

    fn render_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Custom cube rendering logic would go here
        // For this test, we just return an empty vector
        Ok(vec![])
    }

    fn default_channel_value(&self, channel: &str) -> Option<ScalarValue> {
        match channel {
            "size" => Some(ScalarValue::Float32(Some(10.0))),
            "fill" => Some(ScalarValue::Utf8(Some("#3498db".to_string()))),
            _ => None,
        }
    }
}

#[test]
fn test_external_coord_system_can_be_created() {
    // Create a custom coordinate system
    let iso = Isometric::new();

    // Verify required channels
    assert_eq!(iso.required_channels(), &["iso_x", "iso_y", "iso_z"]);

    // Test default ranges
    assert_eq!(iso.default_range("iso_x", 100.0, 100.0), Some((0.0, 80.0)));
    assert_eq!(iso.default_range("iso_y", 100.0, 100.0), Some((0.0, 80.0)));
    assert_eq!(iso.default_range("iso_z", 100.0, 100.0), Some((0.0, 40.0)));
}

#[test]
fn test_external_coord_system_transform() {
    // Create a custom coordinate system
    let iso = Isometric::new();

    // Create channel expressions
    let mut channels = HashMap::new();
    channels.insert("iso_x".to_string(), col("x"));
    channels.insert("iso_y".to_string(), col("y"));
    channels.insert("iso_z".to_string(), col("z"));

    // Transform should succeed
    let result = iso.transform_expressions(channels);
    assert!(result.is_ok());

    let transform = result.unwrap();
    assert!(transform.depth.is_some()); // Should have depth for 3D
}

#[test]
fn test_external_mark_with_external_coord() {
    // Create a plot with custom coordinate system
    let _plot = Plot::new(Isometric::new());

    // Create a custom mark for the custom coordinate system
    let cube = Cube::<Isometric>::new()
        .iso_x("x_pos")
        .iso_y("y_pos")
        .iso_z("z_pos")
        .fill("category")
        .size(15.0);

    // Verify state access works
    assert_eq!(cube.mark_type(), "cube");
    assert_eq!(cube.state().zindex, None);

    // Verify channels are properly defined
    let channels = cube.supported_channels();
    let channel_names: Vec<_> = channels.iter().map(|c| c.name).collect();
    assert!(channel_names.contains(&"iso_x"));
    assert!(channel_names.contains(&"iso_y"));
    assert!(channel_names.contains(&"iso_z"));
    assert!(channel_names.contains(&"fill"));
    assert!(channel_names.contains(&"size"));
}

#[test]
fn test_external_coord_in_plot() {
    // Create a plot with custom coordinate system
    let plot = Plot::new(Isometric::new());

    // Create a custom mark
    let cube = Cube::<Isometric>::new().iso_x("x").iso_y("y").iso_z("z");

    // Add the mark to the plot - this tests that the types work correctly
    let plot_with_mark = plot.mark(cube);

    // The plot should accept our custom mark without issue
    assert!(plot_with_mark.marks().len() > 0);
}

#[test]
fn test_external_coord_axes() {
    let iso = Isometric::new();
    let scales = HashMap::new();
    let marks: Vec<Box<dyn Mark<Isometric>>> = vec![];

    // Create default axes
    let axes = iso.create_default_axes(&scales, &marks);

    // Should be empty since we have no scales
    assert_eq!(axes.len(), 0);

    // Add a scale and try again
    let mut scales = HashMap::new();
    scales.insert(
        "iso_x".to_string(),
        avenger_scales::scales::linear::LinearScale::configured((0.0, 100.0), (0.0, 500.0)),
    );

    let axes = iso.create_default_axes(&scales, &marks);
    assert_eq!(axes.len(), 1);
    assert!(axes.contains_key("iso_x"));
}
