use std::{any::Any, collections::HashMap, sync::Arc};

use avenger_common::value::ScalarOrArray;
use avenger_scales::scales::ScaleImpl;
use datafusion::{common::ScalarValue, dataframe::DataFrame, prelude::SessionContext};

pub use crate::chart_core::{
    FacetAxis, OverflowSpaceRequirement, PaddingSpec, PlotGeometry, PointGeometry, SubplotGeometry,
    SubplotRect,
};

use crate::chart_core::ScaleRangeBinding;
use crate::{
    error::AvengerChartError,
    guide::CoordinateGuide,
    marks::CompiledMark,
    plot::compiled::ComponentsMeasurement,
    render::{CoordinationCheckpoint, EvaluationContext},
    scales::{ConfiguredScaleWithSpec, domain_extent::DomainExtent},
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
        let legend_top = (self.total.top - self.guide.top)
            .max(0.0)
            .max((other.total.top - other.guide.top).max(0.0));
        let legend_right = (self.total.right - self.guide.right)
            .max(0.0)
            .max((other.total.right - other.guide.right).max(0.0));
        let legend_bottom = (self.total.bottom - self.guide.bottom)
            .max(0.0)
            .max((other.total.bottom - other.guide.bottom).max(0.0));
        let legend_left = (self.total.left - self.guide.left)
            .max(0.0)
            .max((other.total.left - other.guide.left).max(0.0));

        self.guide.top = self.guide.top.max(other.guide.top);
        self.guide.bottom = self.guide.bottom.max(other.guide.bottom);
        self.guide.left = self.guide.left.max(other.guide.left);
        self.guide.right = self.guide.right.max(other.guide.right);

        self.total.top = self.guide.top + legend_top;
        self.total.right = self.guide.right + legend_right;
        self.total.bottom = self.guide.bottom + legend_bottom;
        self.total.left = self.guide.left + legend_left;
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
/// Coordinate measurement interface shared by all coordinate systems.
pub trait CoordMeasurement: Send + Sync + 'static {
    /// Downcast support for accessing concrete measurement types
    fn as_any(&self) -> &dyn Any;

    /// Mutable downcast support for coordination phase
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// Get coordinated overflow after coordination phase.
    /// Returns None for non-coordinatable measurements.
    fn coordinated_overflow(&self) -> Option<&CoordinatedOverflow> {
        None
    }

    /// Apply scale adjustments derived from coordinate measurement.
    ///
    /// This allows coordinate systems to adjust scale configurations based on
    /// measurement results. For example, FacetColumn updates the column scale
    /// with physical `padding_inner_px` computed from cell overflow measurements.
    ///
    /// # Arguments
    /// * `scales` - Mutable map of scales to update
    ///
    /// Default implementation: no-op (for coordinate systems that don't need scale updates)
    fn apply_scale_adjustments(&self, _scales: &mut HashMap<String, ConfiguredScaleWithSpec>) {
        // Default: no-op
    }

    /// Return a read-only child-frame container projection when this coordinate
    /// measurement owns measured child plots.
    ///
    /// Most coordinate systems are ordinary data coordinate systems and return
    /// `None`. Container coordinate systems such as facets, concat, and
    /// coordinate-positioned subplots override this hook so generic layout and
    /// debug code can inspect child frames without downcasting to built-in
    /// concrete measurement types.
    #[doc(hidden)]
    fn child_frame_container_view<'a>(
        &'a self,
        _measurement: &'a ComponentsMeasurement,
    ) -> Result<Option<crate::container::ChildFrameContainerView<'a>>, AvengerChartError> {
        Ok(None)
    }
}

/// Layout parameters coordinated across all facet-band nodes at the same depth.
///
/// Ensures subplot widths and gaps are consistent across all branches at each
/// nesting depth, even when branches have different overflow patterns or cell counts.
#[derive(Default, Clone, Debug)]
pub struct CoordinatedLayout {
    /// Physical gap between adjacent subplot plot areas.
    ///
    /// This value is written to band scales and explicit plot-area placement.
    pub padding_inner_px: f32,
    /// Virtual same-axis guide slot gap used for facet-guide alignment.
    ///
    /// This may be larger than `padding_inner_px` in deeply nested same-axis
    /// facet chains, where labels and guide titles need a common slot model but
    /// subplot plot areas should not inherit that extra guide-only space.
    pub guide_slot_gap_px: f32,
    pub outer_start: f32,
    pub outer_end: f32,
    pub n: usize,
}

impl CoordinatedLayout {
    pub fn merge(&mut self, other: &CoordinatedLayout) {
        self.padding_inner_px = self.padding_inner_px.max(other.padding_inner_px);
        self.guide_slot_gap_px = self.guide_slot_gap_px.max(other.guide_slot_gap_px);
        self.outer_start = self.outer_start.max(other.outer_start);
        self.outer_end = self.outer_end.max(other.outer_end);
        self.n = self.n.max(other.n);
    }
}

/// Empty measurement for coordinate systems that don't need measurement data.
///
/// Used by Cartesian, Polar, and other non-facet coordinate systems.
#[derive(Debug, Clone, Default)]
pub struct EmptyCoordMeasurement;

impl CoordMeasurement for EmptyCoordMeasurement {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    // Use default implementations for coordination methods (return None/empty)
}

/// Cell domain extent info collected before facet overflow measurement.
///
/// Contains all the information needed to group and aggregate domain extents
/// across cells at the appropriate channel-domain sharing level.
#[derive(Clone, Debug)]
pub struct CellDomainInfo {
    /// Full path to this cell (parent_path + cell_value)
    pub full_cell_path: Vec<ScalarValue>,
    /// Channel name (e.g., "x", "y")
    pub channel: String,
    /// Channel-domain sharing level (0=Free, N=Level(N), 255=Shared)
    pub domain_sharing_level: u8,
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
/// 5. Retargets subplot geometry and scale ranges affected by legend overflow
///    or layout coordination
///
/// Shared plot-scale domains are coordinated before facet overflow measurement,
/// so this pass only reconciles measured layout requirements.
pub async fn coordinate_overflow_for_guides(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
) -> Result<(), AvengerChartError> {
    crate::facet::coordination::coordinate_facet_measurement_tree(measurement, eval_ctx).await
}

pub async fn coordinate_overflow_for_guides_until(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
    checkpoint: CoordinationCheckpoint,
) -> Result<(), AvengerChartError> {
    crate::facet::coordination::coordinate_facet_measurement_tree_until(
        measurement,
        eval_ctx,
        checkpoint,
    )
    .await
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

/// Inputs for coordinate-system measurement.
///
/// This request object is the first boundary around coordinate measurement.
/// Keeping the inputs in one owned type makes the eventual crate split less
/// brittle: the request can be narrowed into stable views without changing
/// every coordinate-system implementation again.
#[derive(Clone, Copy)]
pub struct CoordMeasureRequest<'a> {
    scales: &'a HashMap<String, ConfiguredScaleWithSpec>,
    plot_width: f32,
    plot_height: f32,
    eval_ctx: &'a EvaluationContext,
    data: Option<&'a DataFrame>,
    compiled_marks: &'a [Arc<dyn CompiledMark>],
    facet_path: &'a [ScalarValue],
}

impl<'a> CoordMeasureRequest<'a> {
    pub(crate) fn new(
        scales: &'a HashMap<String, ConfiguredScaleWithSpec>,
        plot_width: f32,
        plot_height: f32,
        eval_ctx: &'a EvaluationContext,
        data: Option<&'a DataFrame>,
        compiled_marks: &'a [Arc<dyn CompiledMark>],
        facet_path: &'a [ScalarValue],
    ) -> Self {
        Self {
            scales,
            plot_width,
            plot_height,
            eval_ctx,
            data,
            compiled_marks,
            facet_path,
        }
    }

    pub(crate) fn scales(&self) -> &'a HashMap<String, ConfiguredScaleWithSpec> {
        self.scales
    }

    pub fn plot_width(&self) -> f32 {
        self.plot_width
    }

    pub fn plot_height(&self) -> f32 {
        self.plot_height
    }

    pub(crate) fn eval_ctx(&self) -> &'a EvaluationContext {
        self.eval_ctx
    }

    pub(crate) fn data(&self) -> Option<&'a DataFrame> {
        self.data
    }

    pub(crate) fn compiled_marks(&self) -> &'a [Arc<dyn CompiledMark>] {
        self.compiled_marks
    }

    pub(crate) fn facet_path(&self) -> &'a [ScalarValue] {
        self.facet_path
    }
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
                if let Some(expr) = channel_value.expr(session_context)
                    && !expr.column_refs().is_empty()
                {
                    return Some(col_name);
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
                if let Some(expr) = channel_value.expr(session_context)
                    && !expr.column_refs().is_empty()
                {
                    return Some(col_name);
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
        _request: CoordMeasureRequest<'_>,
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

    /// Get the default range binding for a coordinate channel.
    ///
    /// The binding records whether the range is dimension-dependent. It is used
    /// both for initial scale construction and for plot-area retargeting.
    fn default_range_binding(&self, _channel: &str) -> Option<ScaleRangeBinding> {
        None
    }

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
    ) -> Option<(f64, f64)> {
        self.default_range_binding(channel)
            .and_then(|binding| binding.resolve(plot_area_width, plot_area_height))
    }

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
    use crate::layout::BandPosition;
    use datafusion::common::ScalarValue;

    #[test]
    fn test_coordinate_transform_serialization() {
        // Create a coordinate system transform
        let cartesian = Cartesian;
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

    #[test]
    fn coordinated_overflow_merge_preserves_guide_and_legend_slabs() {
        let mut first = CoordinatedOverflow {
            guide: OverflowSpaceRequirement {
                right: 4.0,
                ..Default::default()
            },
            total: OverflowSpaceRequirement {
                right: 64.0,
                ..Default::default()
            },
        };
        let second = CoordinatedOverflow {
            guide: OverflowSpaceRequirement {
                right: 10.0,
                ..Default::default()
            },
            total: OverflowSpaceRequirement {
                right: 20.0,
                ..Default::default()
            },
        };

        first.merge(&second);

        assert_eq!(first.guide.right, 10.0);
        assert_eq!(first.total.right, 70.0);
    }
}
