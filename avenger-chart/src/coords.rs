use std::{collections::HashMap, sync::Arc};

use datafusion::{common::ScalarValue, dataframe::DataFrame, prelude::SessionContext};

pub use crate::chart_core::{
    CoordMeasurement, CoordinateSystemTransformCore, CoordinatedLayout, CoordinatedOverflow,
    EmptyCoordMeasurement, FacetAxis, OverflowSpaceRequirement, PaddingSpec, PlotGeometry,
    PointGeometry, SubplotGeometry, SubplotRect,
};

use crate::facet::coord::{FacetBandCoordMeasurement, FacetBandProbeMeasurement};
use crate::{
    error::AvengerChartError,
    guide::CoordinateGuide,
    marks::CompiledMark,
    plot::compiled::ComponentsMeasurement,
    render::{CoordinationCheckpoint, EvaluationContext},
    scales::{ConfiguredScaleWithSpec, domain_extent::DomainExtent},
};

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

pub(crate) fn apply_coord_measurement_scale_adjustments(
    measurement: &dyn CoordMeasurement,
    scales: &mut HashMap<String, ConfiguredScaleWithSpec>,
) {
    if let Some(facet_band) = measurement
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurement>()
    {
        facet_band.apply_scale_adjustments(scales);
    } else if let Some(facet_probe) = measurement
        .as_any()
        .downcast_ref::<FacetBandProbeMeasurement>()
    {
        facet_probe.apply_scale_adjustments(scales);
    }
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

#[typetag::serde(tag = "type")]
#[async_trait::async_trait]
pub trait CoordinateSystemTransform: CoordinateSystemTransformCore {
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
