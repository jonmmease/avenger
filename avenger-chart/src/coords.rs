use crate::error::AvengerChartError;
use crate::guide::CoordinateGuideBuilder;
pub use crate::guide::OverflowSpaceRequirement;
use crate::marks::CompiledMark;
use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::marks::group::Clip;
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
    type Guide: CoordinateGuideBuilder;

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
        marks: &[Arc<dyn CompiledMark>],
    ) -> Self::Guide;

    /// Create default axes for channels that don't have explicit axis configuration
    ///
    /// This is called during plot construction to create axes for position channels
    /// that have scales but no user-provided axis configuration.
    fn create_default_axes(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        marks: &[Arc<dyn CompiledMark>],
        session_context: &datafusion::prelude::SessionContext,
    ) -> HashMap<String, <Self::Guide as CoordinateGuideBuilder>::Axis>;

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

    /// Create a boxed coordinate system transform for use with CompiledMark
    ///
    /// This creates a type-erased version of the coordinate system that can be
    /// used by the serializable CompiledMark implementations.
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
/// * `session_context` - The session context for expression evaluation
///
/// # Returns
/// An optional string containing the column name if found
pub fn extract_channel_title_from_marks(
    marks: &[Arc<dyn CompiledMark>],
    channel: &str,
    session_context: &datafusion::prelude::SessionContext,
) -> Option<String> {
    // Look through marks to find a column name for this channel
    for mark in marks {
        if let Some(channel_value) = mark.data_context().channels().get(channel) {
            // Try to get column name if this references actual data
            if let Some(col_name) = channel_value.as_column_name(session_context) {
                // Only use if it references actual columns
                if let Some(expr) = channel_value.expr(session_context) {
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
            if let Some(col_name) = channel_value.as_column_name(session_context) {
                // Only use if it references actual columns
                if let Some(expr) = channel_value.expr(session_context) {
                    if !expr.column_refs().is_empty() {
                        return Some(col_name);
                    }
                }
            }
        }
    }

    None
}

#[typetag::serde(tag = "type")]
pub trait CoordinateSystemTransform: Send + Sync {
    fn required_channels(&self) -> &'static [&'static str];

    /// Clone this transform into a new boxed instance
    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform>;

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

    /// Get the default range for a coordinate channel
    ///
    /// Returns the default range for a given channel based on plot dimensions.
    /// This is used for positional scales like x, y, r, theta.
    ///
    /// # Arguments
    /// * `channel` - The channel name (e.g., "x", "y", "r", "theta")
    /// * `plot_area_width` - Width of the plot area
    /// * `plot_area_height` - Height of the plot area
    ///
    /// # Returns
    /// The default range as (min, max) or None if not a coordinate channel
    fn default_range(
        &self,
        channel: &str,
        plot_area_width: f64,
        plot_area_height: f64,
    ) -> Option<(f64, f64)>;

    /// Get default scale options for a coordinate channel
    ///
    /// Returns coordinate-specific scale options that should be applied
    /// to scales for this channel.
    ///
    /// # Arguments
    /// * `channel` - The channel name
    /// * `scale_type` - The type of scale being configured
    ///
    /// # Returns
    /// Map of option names to their values
    fn default_scale_options(
        &self,
        channel: &str,
        scale_type: &str,
    ) -> HashMap<String, datafusion::scalar::ScalarValue>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartesian::Cartesian;

    #[test]
    fn test_coordinate_transform_serialization() {
        // Create a coordinate system transform
        let cartesian = Cartesian::default();
        let transform: Box<dyn CoordinateSystemTransform> = Box::new(cartesian);

        // Serialize it
        let json = serde_json::to_string(&transform).unwrap();
        assert!(json.contains("\"type\":\"Cartesian\""));

        // Deserialize it
        let deserialized: Box<dyn CoordinateSystemTransform> = serde_json::from_str(&json).unwrap();

        // Check that required channels match
        assert_eq!(deserialized.required_channels(), &["x", "y"]);
    }
}
