use std::{collections::HashMap, sync::Arc};

use datafusion::{common::ScalarValue, dataframe::DataFrame};

use avenger_chart_cartesian::Cartesian;
pub use avenger_chart_core::{
    CoordMeasurement, CoordinateSystem, CoordinateSystemCore, CoordinateSystemTransform,
    CoordinateSystemTransformCore, CoordinatedLayout, CoordinatedOverflow, EmptyCoordMeasurement,
    FacetAxis, OverflowSpaceRequirement, PaddingSpec, PlotGeometry, PointGeometry, SubplotGeometry,
    SubplotRect, extract_channel_title_from_marks,
};

use crate::facet::coord::{FacetBandCoordMeasurement, FacetBandProbeMeasurement};
use crate::{
    error::AvengerChartError,
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

/// Measure coordinate-system-specific layout state for built-in layout-aware
/// coordinates.
///
/// External coordinate transforms default to no measurement. Facet, concat, and
/// coordinate-positioned Cartesian subplot measurement stay in the top-level
/// crate because they depend on full plot/layout runtime state.
pub(crate) async fn measure_coordinate_system_transform(
    transform: &dyn CoordinateSystemTransform,
    request: CoordMeasureRequest<'_>,
) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
    let any = transform.as_any();

    if any.is::<Cartesian>() {
        return crate::cartesian::positioned_subplot::measure_cartesian_positioned_subplots(
            request.scales(),
            request.plot_width(),
            request.plot_height(),
            request.eval_ctx(),
            request.data(),
            request.compiled_marks(),
            request.facet_path(),
        )
        .await;
    }

    if any.is::<crate::concat::HConcat>() {
        return crate::concat::measure_concat_coord_system(
            crate::layout::BandDirection::Horizontal,
            request.plot_width(),
            request.plot_height(),
            request.eval_ctx(),
            request.data(),
            request.compiled_marks(),
            request.facet_path(),
        )
        .await;
    }

    if any.is::<crate::concat::VConcat>() {
        return crate::concat::measure_concat_coord_system(
            crate::layout::BandDirection::Vertical,
            request.plot_width(),
            request.plot_height(),
            request.eval_ctx(),
            request.data(),
            request.compiled_marks(),
            request.facet_path(),
        )
        .await;
    }

    if any.is::<crate::facet::coord::FacetRow>() {
        return crate::facet::coord::measure_facet_row(
            request.scales(),
            request.plot_width(),
            request.eval_ctx(),
            request.data(),
            request.compiled_marks(),
            request.facet_path(),
        )
        .await;
    }

    if any.is::<crate::facet::coord::FacetColumn>() {
        return crate::facet::coord::measure_facet_column(
            request.scales(),
            request.plot_height(),
            request.eval_ctx(),
            request.data(),
            request.compiled_marks(),
            request.facet_path(),
        )
        .await;
    }

    Ok(Box::new(EmptyCoordMeasurement))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cartesian::Cartesian, polar::Polar};

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

        let polar = Polar::new();
        let transform: Box<dyn CoordinateSystemTransform> = Box::new(polar);

        let json = serde_json::to_string(&transform).unwrap();
        assert!(json.contains("\"type\":\"Polar\""));

        let deserialized: Box<dyn CoordinateSystemTransform> = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.required_channels(), &["r", "theta"]);
    }
}
