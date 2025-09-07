use crate::axis::Axis;
use crate::error::AvengerChartError;
use avenger_scenegraph::marks::group::Clip;
use avenger_scenegraph::marks::mark::SceneMark;
use std::collections::HashMap;

/// Space requirements for coordinate system guides that overflow the plot area
#[derive(Debug, Clone)]
pub struct OverflowSpaceRequirement {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}

#[async_trait::async_trait]
pub trait CoordinateSystem: Sized + Send + Sync + 'static {
    /// The axis type for this coordinate system
    type Axis: Axis + Clone + 'static;

    /// Get the names of position channels required by this coordinate system
    fn required_channels(&self) -> &'static [&'static str];

    /// Get default range for a specific position channel based on inner plot dimensions
    /// Returns the range as a tuple of (start, end) values
    fn default_range(&self, channel: &str, width: f64, height: f64) -> Option<(f64, f64)>;

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
    /// * `width_estimate` - The estimated plot width
    /// * `height_estimate` - The estimated plot height
    /// * `theme` - The theme for rendering
    async fn measure_guide_overflow(
        &self,
        axes: HashMap<String, Self::Axis>,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        width_estimate: f32,
        height_estimate: f32,
        theme: &crate::theme::Theme,
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
        theme: &crate::theme::Theme,
    ) -> Result<Vec<SceneMark>, AvengerChartError>;

    /// Get default scale options for channels in this coordinate system
    /// Each coordinate system knows its own position channels and their optimal defaults
    fn default_scale_options(
        &self,
        _channel: &str,
        _scale_impl: &dyn avenger_scales::scales::ScaleImpl,
    ) -> HashMap<String, datafusion::logical_expr::Expr> {
        // Default implementation returns empty - each coord system overrides as needed
        HashMap::new()
    }

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

    /// Transform position channels to plot coordinates
    ///
    /// Takes position data in the coordinate system's native space (after scaling)
    /// and transforms it to x/y plot coordinates relative to the plot area origin.
    ///
    /// # Arguments
    /// * `position_channels` - Map of position channel names to their scaled data
    /// * `plot_width` - Width of the plot area
    /// * `plot_height` - Height of the plot area
    ///
    /// # Returns
    /// Tuple of (x, y) arrays in plot coordinates (screen space relative to plot area)
    fn transform_to_plot_coords(
        &self,
        position_channels: &std::collections::HashMap<
            &str,
            avenger_common::value::ScalarOrArray<f32>,
        >,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<
        (
            avenger_common::value::ScalarOrArray<f32>,
            avenger_common::value::ScalarOrArray<f32>,
        ),
        AvengerChartError,
    >;
}
