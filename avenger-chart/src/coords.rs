use std::{any::Any, collections::HashMap, sync::Arc};

use avenger_common::value::ScalarOrArray;
use avenger_scales::scales::ScaleImpl;
use datafusion::{common::ScalarValue, dataframe::DataFrame, prelude::SessionContext};
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

pub use crate::guide::OverflowSpaceRequirement;

use crate::{
    error::AvengerChartError,
    facet::{band_positions::BandPosition, coordination::CoordinationGroupKey},
    guide::CoordinateGuide,
    marks::CompiledMark,
    plot::compiled::ComponentsMeasurement,
    render::EvaluationContext,
    scales::{ConfiguredScaleWithSpec, domain_extent::DomainExtent},
    serialization::SerializableScalar,
};

/// Coordinated overflow values aggregated across ALL facets at the same nesting level.
///
/// This enables facet labels at the same nesting depth to be horizontally aligned
/// regardless of their parent facet's individual overflow requirements.
///
/// Contains both guide-only overflow (for label positioning) and total overflow
/// (including legends, for consistent spacing across facet cells).
#[derive(Default, Clone, Debug)]
pub struct CoordinatedOverflow {
    /// Overflow from guides only (axes, labels, ticks).
    /// Used for facet label positioning.
    pub guide: OverflowSpaceRequirement,

    /// Total overflow including legends.
    /// Used for consistent legend spacing across facet cells.
    pub total: OverflowSpaceRequirement,
}

impl CoordinatedOverflow {
    /// Merge with another overflow context, keeping the maximum of each field.
    pub fn merge(&mut self, other: &Self) {
        self.guide.top = self.guide.top.max(other.guide.top);
        self.guide.bottom = self.guide.bottom.max(other.guide.bottom);
        self.guide.left = self.guide.left.max(other.guide.left);
        self.guide.right = self.guide.right.max(other.guide.right);

        self.total.top = self.total.top.max(other.total.top);
        self.total.bottom = self.total.bottom.max(other.total.bottom);
        self.total.left = self.total.left.max(other.total.left);
        self.total.right = self.total.right.max(other.total.right);
    }
}

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
///
/// # Coordination Support
///
/// The trait also provides methods for cross-facet coordination of overflow values.
/// This enables facet labels at the same nesting depth to be horizontally aligned
/// regardless of their individual subplot overflow requirements.
///
/// Default implementations return `None` or empty slices, making coordination opt-in
/// for coordinate systems that support it (facets) while being a no-op for those
/// that don't (Cartesian, Polar, etc.).
#[async_trait::async_trait]
pub trait CoordMeasurement: Send + Sync + 'static {
    /// Downcast support for accessing concrete measurement types
    fn as_any(&self) -> &dyn Any;

    /// Mutable downcast support for coordination phase
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// Get child measurements for traversal.
    /// Returns empty slice for non-facet coords.
    fn child_measurements(&self) -> &[ComponentsMeasurement] {
        &[]
    }

    /// Get mutable child measurements for coordination.
    fn child_measurements_mut(&mut self) -> &mut [ComponentsMeasurement] {
        &mut []
    }

    /// Get local overflow to contribute to guide alignment by nesting depth.
    /// Returns None for non-coordinatable measurements (e.g., EmptyCoordMeasurement).
    ///
    /// Implementations should compute this from their children's overflow values.
    fn local_overflow(&self) -> Option<CoordinatedOverflow> {
        None
    }

    /// Get coordinated overflow after coordination phase.
    /// Returns None for non-coordinatable measurements.
    fn coordinated_overflow(&self) -> Option<&CoordinatedOverflow> {
        None
    }

    /// Set coordinated overflow during distribution pass.
    /// Default: no-op for non-coordinatable measurements.
    fn set_coordinated_overflow(&mut self, _overflow: CoordinatedOverflow) {
        // Default: no-op
    }

    /// Apply scale adjustments derived from coordinate measurement.
    ///
    /// This allows coordinate systems to adjust scale configurations based on
    /// measurement results. For example, FacetColumn updates the column scale
    /// with `padding_inner_px` computed from cell overflow measurements.
    ///
    /// # Arguments
    /// * `scales` - Mutable map of scales to update
    ///
    /// Default implementation: no-op (for coordinate systems that don't need scale updates)
    fn apply_scale_adjustments(&self, _scales: &mut HashMap<String, ConfiguredScaleWithSpec>) {
        // Default: no-op
    }

    /// Returns the padding_inner_px if this is a facet coord with internal spacing.
    ///
    /// Used for propagating child facet padding to parent facets of the same orientation.
    /// When a parent FacetColumn contains nested FacetColumn children, the parent should
    /// use at least as much gap spacing as its children to maintain visual consistency.
    ///
    /// Default implementation: None (for non-facet coordinate systems)
    fn padding_inner_px(&self) -> Option<f32> {
        None
    }

    /// Update child measurement dimensions after second-pass layout.
    ///
    /// When the outer layout's second pass computes a different plot_area size
    /// (due to legend overflow), this method updates child subplot measurements
    /// to use the correct dimensions.
    ///
    /// # Arguments
    /// * `new_height` - The correct plot_area height from second-pass layout
    ///
    /// Default implementation: no-op (for non-facet coordinate systems)
    fn update_child_dimensions(&mut self, _new_height: f32) {
        // Default: no-op
    }

    /// Re-measure children after coordinated overflow is set.
    ///
    /// When legend overflow causes subplot height to change, this method
    /// re-measures child subplots with the corrected dimensions. This ensures
    /// that measurements passed to `build_plot_components` are accurate.
    ///
    /// Called by `coordinate_overflow_for_guides` after distributing coordinated values.
    ///
    /// Default implementation: no-op for non-facet coordinate systems.
    async fn apply_coordinated_overflow(
        &mut self,
        _eval_ctx: &EvaluationContext,
    ) -> Result<(), AvengerChartError> {
        Ok(())
    }

    /// Get the scale sharing level for a channel.
    ///
    /// Returns the sharing level (0=Free, N=Level(N), 255=Shared) for the given channel.
    /// This is used for axis visibility decisions in faceted charts.
    ///
    /// Default implementation: returns 255 (Shared), which preserves the original
    /// "edge-only" visibility behavior where labels only show on the outermost cells.
    fn channel_sharing_level(&self, _channel: &str) -> u8 {
        255 // Shared: default to current behavior
    }

    /// Get local layout parameters for coordinated sizing.
    ///
    /// Returns layout parameters (padding, outer offsets, cell count) that
    /// should be coordinated across all facets at the same nesting depth.
    ///
    /// Default implementation: None (for non-facet coordinate systems).
    fn local_layout(&self) -> Option<CoordinatedLayout> {
        None
    }

    /// Set coordinated layout parameters during distribution pass.
    ///
    /// After layout parameters have been aggregated across all facets at the
    /// same depth, this distributes the max values back so all branches use
    /// consistent sizing.
    ///
    /// Default implementation: no-op for non-facet coordinate systems.
    fn set_coordinated_layout(&mut self, _layout: CoordinatedLayout) {
        // Default: no-op
    }

    /// Collect cell domain extents for level-aware domain coordination.
    ///
    /// Facet implementations should iterate over their cells and push
    /// `CellDomainInfo` entries for each (cell, channel) pair that has
    /// domain extents to coordinate.
    ///
    /// Default implementation: no-op for non-facet coordinate systems.
    fn collect_cell_domain_extents(&self, _collector: &mut Vec<CellDomainInfo>) {
        // Default: no-op
    }

    /// Distribute unified domain extents back to cells.
    ///
    /// Facet implementations should look up coordinated extents for each
    /// (cell, channel) pair and store them for use during rendering.
    ///
    /// Default implementation: no-op for non-facet coordinate systems.
    fn distribute_cell_domain_extents(
        &mut self,
        _unified: &HashMap<(String, Vec<ScalarValue>), DomainExtent>,
    ) {
        // Default: no-op
    }

    /// Get the coordinated subplot width for propagation to children.
    ///
    /// After coordination adjusts this node's subplot width, this returns
    /// the coordinated width so it can be propagated to child FacetCol nodes.
    /// Children use this to update their `original_column_scale` range.
    ///
    /// Default implementation: None (for non-facet coordinate systems).
    fn coordinated_subplot_width(&self) -> Option<f32> {
        None
    }

    /// Set the parent's coordinated bandwidth on this node.
    ///
    /// When the parent FacetCol's coordination produces a different subplot width
    /// than what this node's `original_column_scale` was measured with, this method
    /// updates the scale range so that subsequent bandwidth computations are consistent
    /// across all branches at the same depth.
    ///
    /// Default implementation: no-op for non-facet coordinate systems.
    fn set_parent_bandwidth(&mut self, _bandwidth: f32) {
        // Default: no-op
    }

    /// Coordination grouping key for this node.
    ///
    /// Facet measurements should return a stable key containing depth/axis/field
    /// identity so cross-branch coordination is explicit and deterministic.
    ///
    /// Default implementation: `None`, which uses a depth-only fallback key.
    fn coordination_group_key(&self, _depth: usize) -> Option<CoordinationGroupKey> {
        None
    }
}

/// Layout parameters coordinated across all FacetCol nodes at the same depth.
///
/// Ensures subplot widths and gaps are consistent across all branches at each
/// nesting depth, even when branches have different overflow patterns or cell counts.
#[derive(Default, Clone, Debug)]
pub struct CoordinatedLayout {
    pub padding_inner_px: f32,
    pub outer_left: f32,
    pub outer_right: f32,
    pub n: usize,
}

impl CoordinatedLayout {
    pub fn merge(&mut self, other: &CoordinatedLayout) {
        self.padding_inner_px = self.padding_inner_px.max(other.padding_inner_px);
        self.outer_left = self.outer_left.max(other.outer_left);
        self.outer_right = self.outer_right.max(other.outer_right);
        self.n = self.n.max(other.n);
    }
}

/// Empty measurement for coordinate systems that don't need measurement data.
///
/// Used by Cartesian, Polar, and other non-facet coordinate systems.
#[derive(Debug, Clone, Default)]
pub struct EmptyCoordMeasurement;

#[async_trait::async_trait]
impl CoordMeasurement for EmptyCoordMeasurement {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    // Use default implementations for coordination methods (return None/empty)
}

/// Cell domain extent info collected during Pass 1.
///
/// Contains all the information needed to group and aggregate domain extents
/// across cells at the appropriate sharing level.
#[derive(Clone)]
pub struct CellDomainInfo {
    /// Full path to this cell (parent_path + cell_value)
    pub full_cell_path: Vec<ScalarValue>,
    /// Channel name (e.g., "x", "y")
    pub channel: String,
    /// Scale sharing level for this channel (0=Free, N=Level(N), 255=Shared)
    pub sharing_level: u8,
    /// Facet depth (1-based) for sharing comparison
    pub facet_depth: u8,
    /// The domain extent
    pub extent: DomainExtent,
}

/// This is the main entry point for overflow coordination. It:
/// 1. Collects overflow values by nesting depth
/// 2. Computes max overflow at each depth level
/// 3. Distributes coordinated values to all facets
/// 4. Coordinates layout parameters (padding, cell count) by depth
/// 5. Coordinates domain extents for level-aware scale sharing
/// 6. Re-measures any subplots affected by legend overflow or layout coordination
///
/// This ensures measurements are correct before `build_plot_components` is called.
pub async fn coordinate_overflow_for_guides(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
) -> Result<(), AvengerChartError> {
    crate::facet::coordination::coordinate_facet_measurement_tree(measurement, eval_ctx).await
}

#[typetag::serde(tag = "type")]
pub trait PlotGeometry: Send + Sync + 'static {
    fn as_any(&self) -> &dyn Any;
}

/// Geometry type for point-based coordinate systems (Cartesian, Polar, ZeroD)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PointGeometry {
    pub x: ScalarOrArray<f32>,
    pub y: ScalarOrArray<f32>,
}

#[typetag::serde]
impl PlotGeometry for PointGeometry {
    fn as_any(&self) -> &dyn Any {
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
    fn as_any(&self) -> &dyn Any {
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
    session_context: &SessionContext,
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
        _scales: &HashMap<String, ConfiguredScaleWithSpec>,
        _plot_width: f32,
        _plot_height: f32,
        _eval_ctx: &EvaluationContext,
        _data: Option<&DataFrame>,
        _compiled_marks: &[Arc<dyn CompiledMark>],
        _facet_path: &[ScalarValue],
    ) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
        // Default: return empty measurement for non-facet coordinate systems
        Ok(Box::new(EmptyCoordMeasurement))
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
        scale_impl: &dyn ScaleImpl,
    ) -> HashMap<String, ScalarValue>;
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
