use crate::error::AvengerChartError;
pub use crate::guide::OverflowSpaceRequirement;
use crate::guide::{CoordinateGuideBuilder, CoordinateGuideRender};
use crate::marks::MarkRenderer;
use crate::theme::Theme;
use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::marks::group::Clip;
use avenger_scenegraph::marks::mark::SceneMark;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[typetag::serde(tag = "type")]
pub trait PlotGeometry: Send + Sync + 'static {
    fn as_any(&self) -> &dyn std::any::Any;
}

/// Geometry type for point-based coordinate systems (Cartesian, Polar, ZeroD)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PointGeometry {
    pub x: ScalarOrArray<f32>,
    pub y: ScalarOrArray<f32>,
}

#[typetag::serde]
impl PlotGeometry for PointGeometry {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[async_trait::async_trait]
pub trait CoordinateSystem: Sized + Send + Sync + 'static {
    /// The guide type for this coordinate system
    ///
    /// This could be axes (Cartesian), geographic features (Geo),
    /// camera controls (3D), or no guide at all (ZeroD)
    type Guide: CoordinateGuideRender + CoordinateGuideBuilder;

    /// The plot geometry type produced by this coordinate system's transform
    type PlotGeometry: PlotGeometry;

    /// Get the names of position channels required by this coordinate system
    fn required_channels(&self) -> &'static [&'static str];

    /// Get default range for a specific position channel based on inner plot dimensions
    /// Returns the range as a tuple of (start, end) values
    fn default_range(&self, channel: &str, width: f64, height: f64) -> Option<(f64, f64)>;

    /// Create default guide for this coordinate system
    ///
    /// # Arguments
    /// * `axes` - Axes that were configured at the channel level
    /// * `scales` - The scale registry containing all configured scales
    /// * `marks` - The mark renderers in the plot, used to extract information for guide configuration
    ///
    /// # Returns
    /// The default guide configuration for this coordinate system with axes set
    fn create_default_guide(
        &self,
        axes: HashMap<String, <Self::Guide as CoordinateGuideBuilder>::Axis>,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        marks: &[Arc<dyn MarkRenderer>],
    ) -> Self::Guide;

    /// Configure the guide with user-provided settings
    ///
    /// This allows users to customize coordinate-level options
    fn configure_guide<F>(&self, guide: Self::Guide, f: F) -> Self::Guide
    where
        F: FnOnce(Self::Guide) -> Self::Guide,
    {
        f(guide)
    }

    /// Create default axes for channels that don't have explicit axis configuration
    ///
    /// This is called during plot construction to create axes for position channels
    /// that have scales but no user-provided axis configuration.
    fn create_default_axes(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        marks: &[Arc<dyn MarkRenderer>],
    ) -> HashMap<String, <Self::Guide as CoordinateGuideBuilder>::Axis>;

    /// Measure how much space the coordinate system's guide needs outside the plot area
    ///
    /// This delegates to the Guide's measure_overflow method
    async fn measure_guide_overflow(
        &self,
        guide: &Self::Guide,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        width_estimate: f32,
        height_estimate: f32,
        theme: &dyn Theme,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        guide
            .measure_overflow(scales, width_estimate, height_estimate, theme)
            .await
    }

    /// Render the guide for this coordinate system
    ///
    /// This delegates to the Guide's render method
    async fn render_guide(
        &self,
        guide: &Self::Guide,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &crate::layout::LayoutBounds,
        theme: &dyn crate::theme::Theme,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        guide
            .render(scales, plot_width, plot_height, plot_bounds, theme)
            .await
    }

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

    /// Transform position channels to coordinate system geometry
    ///
    /// Takes position data in the coordinate system's native space (after scaling)
    /// and transforms it to the coordinate system's geometry type.
    ///
    /// # Arguments
    /// * `position_channels` - Map of position channel names to their scaled data
    /// * `plot_width` - Width of the plot area
    /// * `plot_height` - Height of the plot area
    ///
    /// # Returns
    /// The coordinate system's plot geometry type containing transformed positions
    fn transform(
        &self,
        position_channels: &std::collections::HashMap<
            &str,
            avenger_common::value::ScalarOrArray<f32>,
        >,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<Self::PlotGeometry, AvengerChartError>;

    /// Create a boxed coordinate system transform for use with MarkRenderer
    ///
    /// This creates a type-erased version of the coordinate system that can be
    /// used by the serializable MarkRenderer implementations.
    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform>;
}

/// Helper function to extract channel title from mark encodings
///
/// This looks through the marks to find a meaningful column name for the given channel,
/// which can be used as a default title for axes or legends.
///
/// # Arguments
/// * `marks` - The marks in the plot
/// * `channel` - The channel name to extract a title for
///
/// # Returns
/// An optional string containing the column name if found
pub fn extract_channel_title_from_marks(
    marks: &[Arc<dyn MarkRenderer>],
    channel: &str,
) -> Option<String> {
    // Look through marks to find a column name for this channel
    for mark in marks {
        if let Some(channel_value) = mark.data_context().channels().get(channel) {
            // Try to get column name if this references actual data
            if let Some(col_name) = channel_value.as_column_name() {
                // Only use if it references actual columns
                if let Some(expr) = channel_value.expr() {
                    if !expr.column_refs().is_empty() {
                        return Some(col_name);
                    }
                }
            }
        }
    }

    // For interval marks, also check the secondary channel (x2, y2)
    // if the primary channel didn't have a meaningful column
    let secondary_channel = match channel {
        "x" => "x2",
        "y" => "y2",
        _ => return None,
    };

    for mark in marks {
        if let Some(channel_value) = mark.data_context().channels().get(secondary_channel) {
            // Try to get column name if this references actual data
            if let Some(col_name) = channel_value.as_column_name() {
                // Only use if it references actual columns
                if let Some(expr) = channel_value.expr() {
                    if !expr.column_refs().is_empty() {
                        return Some(col_name);
                    }
                }
            }
        }
    }

    None
}

pub trait CoordinateSystemTransform {
    fn required_channels(&self) -> &'static [&'static str];

    /// Transform position channels to coordinate system geometry
    ///
    /// Takes position data in the coordinate system's native space (after scaling)
    /// and transforms it to the coordinate system's geometry type.
    ///
    /// # Arguments
    /// * `position_channels` - Map of position channel names to their scaled data
    /// * `plot_width` - Width of the plot area
    /// * `plot_height` - Height of the plot area
    ///
    /// # Returns
    /// The coordinate system's plot geometry type containing transformed positions
    fn transform(
        &self,
        position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<Box<dyn PlotGeometry>, AvengerChartError>;
}
