use crate::axis::AxisTrait;
use crate::error::AvengerChartError;
use avenger_scenegraph::marks::group::Clip;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::logical_expr::Expr;
use std::collections::HashMap;

/// Space requirements for coordinate system guides that overflow the plot area
#[derive(Debug, Clone)]
pub struct OverflowSpaceRequirement {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}

/// Result of coordinate transformation
#[derive(Debug, Clone)]
pub struct TransformResult {
    /// X coordinate expression in screen space
    pub x: Expr,
    /// Y coordinate expression in screen space
    pub y: Expr,
    /// Optional depth/z-order expression for 3D effects or layering
    pub depth: Option<Expr>,
}

#[async_trait::async_trait]
pub trait CoordinateSystem: Sized + Send + Sync + 'static {
    /// The axis type for this coordinate system
    type Axis: AxisTrait + Clone + 'static;

    /// Get the names of position channels required by this coordinate system
    fn required_channels(&self) -> &'static [&'static str];

    /// Get default range for a specific position channel based on inner plot dimensions
    /// Returns the range as a tuple of (start, end) values
    fn default_range(&self, channel: &str, width: f64, height: f64) -> Option<(f64, f64)>;

    /// Transform position channel expressions to screen coordinates
    ///
    /// # Arguments
    /// * `channels` - Map from channel name (e.g., "r", "theta") to expressions
    ///   that compute the scaled values for those channels
    ///
    /// # Returns
    /// Result containing TransformResult or error if required channels are missing
    fn transform_expressions(
        &self,
        channels: HashMap<String, Expr>,
    ) -> Result<TransformResult, AvengerChartError>;

    /// Create default axes for all channels that have scales
    ///
    /// # Arguments
    /// * `scales` - The scale registry containing all configured scales
    /// * `marks` - The marks in the plot, used to extract column names for titles
    ///
    /// # Returns
    /// A map of channel names to default axis configurations
    fn create_default_axes(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        marks: &[Box<dyn crate::marks::Mark<Self>>],
    ) -> HashMap<String, Self::Axis>
    where
        Self: Sized;

    /// Measure how much space the coordinate system's guides need outside the plot area
    ///
    /// This measures overflow of axes, labels, and other visual guides beyond the
    /// initial plot area bounds.
    ///
    /// # Arguments
    /// * `axes` - The axes to measure
    /// * `scales` - The configured scales
    /// * `width` - The total canvas width
    /// * `height` - The total canvas height  
    /// * `plot_area_ratio` - The initial ratio of plot area to canvas (typically 0.8)
    async fn measure_guide_overflow(
        &self,
        axes: HashMap<String, Self::Axis>,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        width: f32,
        height: f32,
        plot_area_ratio: f32,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError>;

    /// Render all axes for this coordinate system
    ///
    /// # Arguments
    /// * `axes` - Map of all axes (configured + defaults) to render
    /// * `scales` - The scale registry containing all configured scales
    /// * `plot_width` - Width of the plot area
    /// * `plot_height` - Height of the plot area
    /// * `padding` - Padding around the plot area
    ///
    /// # Returns
    /// A vector of SceneMark objects representing the rendered axes
    async fn render_axes(
        &self,
        axes: &HashMap<String, Self::Axis>,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        padding: &crate::render::Padding,
    ) -> Result<Vec<SceneMark>, AvengerChartError>;

    /// Get clipping specification for this coordinate system
    ///
    /// Different coordinate systems may use different clipping regions.
    /// For example, Cartesian uses rectangular clipping while Polar uses circular.
    fn get_clip(
        &self,
        plot_width: f32,
        plot_height: f32,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> Clip;

    /// Prepare the scalar batch with any coordinate-specific data
    ///
    /// This allows coordinate systems to inject additional scalar values
    /// that marks might need for rendering (e.g., polar center coordinates).
    fn prepare_scalar_batch(
        &self,
        batch: datafusion::arrow::record_batch::RecordBatch,
        _plot_width: f32,
        _plot_height: f32,
    ) -> Result<datafusion::arrow::record_batch::RecordBatch, AvengerChartError> {
        // Default implementation: return batch unchanged
        Ok(batch)
    }
}
