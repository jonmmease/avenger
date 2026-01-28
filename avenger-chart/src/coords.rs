use crate::error::AvengerChartError;
use crate::facet::band_positions::BandPosition;
use crate::guide::CoordinateGuide;
pub use crate::guide::OverflowSpaceRequirement;
use crate::marks::CompiledMark;
use crate::serialization::SerializableScalar;
use avenger_common::value::ScalarOrArray;
use datafusion::common::ScalarValue;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

/// Coordinate-system-specific measurement data computed during the measure phase.
///
/// This trait allows coordinate systems (especially facets) to compute layout data
/// once during measurement and make it available to both guides and marks during rendering.
///
/// # Design
///
/// The `as_any()` method enables downcasting from `dyn CoordMeasurement` to the
/// concrete type. This pattern (same as `PlotGeometry`) preserves object safety
/// while allowing coordinate systems to use their specific measurement types.
pub trait CoordMeasurement: Send + Sync + 'static {
    /// Downcast support for accessing concrete measurement types
    fn as_any(&self) -> &dyn Any;
}

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

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubplotRect {
    /// Facet value for this subplot (e.g., "setosa" for species faceting)
    #[serde_as(as = "FromInto<SerializableScalar>")]
    pub value: ScalarValue,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl SubplotRect {
    pub fn new(value: ScalarValue, x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            value,
            x,
            y,
            width,
            height,
        }
    }

    pub fn scalar_value(&self) -> ScalarValue {
        self.value.clone()
    }
}

impl Default for SubplotRect {
    fn default() -> Self {
        SubplotRect::new(ScalarValue::Null, 0.0, 0.0, 0.0, 0.0)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubplotGeometry {
    pub rects: Vec<SubplotRect>,
}

impl SubplotGeometry {
    pub fn new(rects: Vec<SubplotRect>) -> Self {
        Self { rects }
    }

    pub fn count(&self) -> usize {
        self.rects.len()
    }

    pub fn rect_at(&self, index: usize) -> Option<&SubplotRect> {
        self.rects.get(index)
    }

    pub fn iter_rects(&self) -> impl Iterator<Item = &SubplotRect> {
        self.rects.iter()
    }

    pub fn from_band_positions(
        iter: impl IntoIterator<Item = BandPosition>,
        axis: FacetAxis,
        cross_extent: f32,
    ) -> Self {
        let rects = iter
            .into_iter()
            .map(|band| {
                let value = band.value.clone();
                let start = band.start();
                let bandwidth = band.bandwidth;
                match axis {
                    FacetAxis::Row => {
                        SubplotRect::new(value.clone(), 0.0, start, cross_extent, bandwidth)
                    }
                    FacetAxis::Column => {
                        SubplotRect::new(value, start, 0.0, bandwidth, cross_extent)
                    }
                }
            })
            .collect();
        Self { rects }
    }
}

#[typetag::serde]
impl PlotGeometry for SubplotGeometry {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FacetAxis {
    Row,
    Column,
}

/// Padding specification for coordinate system transforms
///
/// Facet coordinate systems need padding between subplots to accommodate overflow
/// from axes, legends, and other guides. This enum supports single-dimension
/// padding for FacetRow/FacetCol coordinate systems.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PaddingSpec {
    /// Single-dimension padding for row or column facets
    Single {
        /// Padding in pixels between subplots
        padding_px: f32,
        /// Overflow measurements for each subplot
        overflow: Vec<OverflowSpaceRequirement>,
    },
}

pub trait CoordinateSystem: Sized + Send + Sync + 'static {
    /// The guide type for this coordinate system
    ///
    /// This could be axes (Cartesian), geographic features (Geo),
    /// camera controls (3D), or no guide at all (ZeroD)
    type Guide: CoordinateGuide;

    // /// The plot geometry type produced by this coordinate system's transform
    // type PlotGeometry: PlotGeometry;

    /// Get the names of position channels required by this coordinate system
    fn required_channels(&self) -> &'static [&'static str];

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

#[async_trait::async_trait]
#[typetag::serde(tag = "type")]
pub trait CoordinateSystemTransform: Send + Sync {
    fn required_channels(&self) -> &'static [&'static str];

    /// Clone this transform into a new boxed instance
    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform>;

    async fn measure(
        &self,
        _scales: &std::collections::HashMap<String, crate::scales::ConfiguredScaleWithSpec>,
        _plot_width: f32,
        _plot_height: f32,
        _eval_ctx: &crate::render::EvaluationContext,
        _data: Option<&datafusion::dataframe::DataFrame>,
        _compiled_marks: &[Arc<dyn CompiledMark>],
        _facet_path: &[ScalarValue],
    ) -> Result<Option<Box<dyn CoordMeasurement>>, AvengerChartError> {
        // Default: no coordinate-level measurement needed
        Ok(None)
    }

    /// Return a new transform updated with measured padding and overflow data.
    ///
    /// # Arguments
    /// * `spec` - Padding specification (Single for row/col facets, Grid for grid facets)
    ///
    /// Default implementation returns an unchanged clone (for non-facet coordinates).
    fn with_measured_padding(&self, spec: &PaddingSpec) -> Box<dyn CoordinateSystemTransform> {
        let _ = spec;
        self.clone_box()
    }

    /// Transform position channels to coordinate system geometry
    ///
    /// Takes position data in the coordinate system's native space (after scaling)
    /// and transforms it to the coordinate system's geometry type.
    ///
    /// # Arguments
    /// * `position_channels` - Map of position channel names to their scaled data
    /// * `position_values` - Optional source ScalarValue for each position (faceting only)
    /// * `plot_width` - Width of the plot area
    /// * `plot_height` - Height of the plot area
    ///
    /// For facet coordinates (FacetRow, FacetColumn), `position_values` provides
    /// the original domain values that were scaled to produce `position_channels`. This enables
    /// SubplotRect.value to store the actual facet key (e.g., "setosa", "versicolor").
    ///
    /// For point-based coordinates (Cartesian, Polar, ZeroD), this parameter is unused.
    ///
    /// # Returns
    /// The coordinate system's plot geometry type containing transformed positions
    fn transform(
        &self,
        position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        position_values: Option<&HashMap<&str, Vec<datafusion::common::ScalarValue>>>,
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
    /// * `scale_impl` - The scale implementation being configured
    ///
    /// # Returns
    /// Map of option names to their values
    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn avenger_scales::scales::ScaleImpl,
    ) -> HashMap<String, datafusion::scalar::ScalarValue>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartesian::Cartesian;
    use crate::facet::band_positions::BandPosition;
    use datafusion::common::ScalarValue;

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

    #[test]
    fn test_subplot_geometry_row_helpers() {
        let band_positions = vec![
            BandPosition::new(ScalarValue::from("A"), 0.0, 20.0),
            BandPosition::new(ScalarValue::from("B"), 20.0, 20.0),
        ];

        let geometry =
            SubplotGeometry::from_band_positions(band_positions.clone(), FacetAxis::Row, 100.0);

        assert_eq!(geometry.count(), 2);
        let first = geometry.rect_at(0).unwrap();
        assert_eq!(first.value, ScalarValue::from("A"));
        assert_eq!(first.x, 0.0);
        assert_eq!(first.y, 0.0);
        assert_eq!(first.width, 100.0);
        assert_eq!(first.height, 20.0);

        let second = geometry.rect_at(1).unwrap();
        assert_eq!(second.value, ScalarValue::from("B"));
        assert_eq!(second.y, 20.0);

        let collected: Vec<_> = geometry.iter_rects().collect();
        assert_eq!(collected.len(), 2);
    }

    #[test]
    fn test_subplot_geometry_column_helpers() {
        let band_positions = vec![
            BandPosition::new(ScalarValue::from("L"), 5.0, 15.0),
            BandPosition::new(ScalarValue::from("R"), 20.0, 15.0),
        ];

        let geometry =
            SubplotGeometry::from_band_positions(band_positions.clone(), FacetAxis::Column, 80.0);

        let first = geometry.rect_at(0).unwrap();
        assert_eq!(first.value, ScalarValue::from("L"));
        assert_eq!(first.x, 5.0);
        assert_eq!(first.y, 0.0);
        assert_eq!(first.width, 15.0);
        assert_eq!(first.height, 80.0);

        let second = geometry.rect_at(1).unwrap();
        assert_eq!(second.value, ScalarValue::from("R"));
        assert_eq!(second.x, 20.0);
    }
}
