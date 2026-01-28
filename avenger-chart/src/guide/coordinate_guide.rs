//! Core trait for coordinate system guides

use crate::theme::Theme;

use crate::axis::Axis;
use crate::error::AvengerChartError;
use crate::facet::evaluated_facet_tree::EvaluatedFacetTree;
use crate::guide::{MeasurementResult, OverflowSpaceRequirement};
use crate::layout::LayoutBounds;
use avenger_scenegraph::marks::mark::SceneMark;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Direction of faceting for determining which channel can be unified
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FacetDirection {
    /// Vertical stacking (row faceting) - can potentially unify y-axis
    Row,
    /// Horizontal arrangement (column faceting) - can potentially unify x-axis
    Column,
}

/// Information about which channel can be unified in faceting
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiableChannelInfo {
    /// The channel name (e.g., "y", "x", "r")
    pub channel: String,
    /// The title to use for the unified axis (extracted from marks)
    pub title: Option<String>,
}

/// Trait for visual guides in coordinate systems
///
/// A CoordinateGuide represents the visual reference elements for a coordinate system.
/// This includes both axes (configured at the channel level) and coordinate-specific
/// options (configured at the plot level).
pub trait CoordinateGuide: Clone + Default + Send + Sync {
    type Axis: Axis + Clone;

    /// Set axes that were configured at the channel level
    ///
    /// This is called during guide creation to apply all axis configurations
    /// from both plot-level and mark-level specifications.
    fn set_axes(&mut self, axes: HashMap<String, Self::Axis>);

    /// Set compiled marks for extracting default axis titles
    ///
    /// This is called during guide creation to provide access to compiled mark
    /// so that default axis titles can be extracted at render time.
    fn set_compiled_marks(
        &mut self,
        compiled_marks: Vec<std::sync::Arc<dyn crate::marks::CompiledMark>>,
        session_context: &datafusion::prelude::SessionContext,
    );

    fn update(&mut self, other: Self);

    fn build(self) -> Box<dyn CompiledGuide>;
}

#[async_trait::async_trait]
#[typetag::serde(tag = "type")]
pub trait CompiledGuide: Send + Sync + 'static {
    /// Measure how much space this guide needs outside the plot area
    ///
    /// # Arguments
    /// * `data_override` - Optional DataFrame to use instead of compiled data.
    ///   This enables nested facets to pass filtered data to inner guides at runtime.
    ///   When Some, guides should use this data. When None, use compiled data.
    /// * `facet_tree` - The evaluated facet tree for visibility decisions.
    /// * `facet_path` - Path of values identifying the cell in the facet grid (e.g., `["East", "Eng"]`).
    ///   Empty slice when not in a facet cell.
    ///   Used for visibility-aware overflow: interior cells may hide axis labels.
    ///   The tree converts this to indices internally for visibility checks.
    /// * `coord_measurement` - Optional coordinate measurement data from `coord_transform.measure()`.
    ///   **This is purely for efficiency** - when provided, implementations may use pre-computed
    ///   data (e.g., subplot measurements for facet guides) to avoid redundant computation.
    ///   The result must be identical whether or not this is provided.
    async fn measure_overflow(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        theme: &Theme,
        params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
        data_override: Option<&datafusion::dataframe::DataFrame>,
        ctx: &datafusion::prelude::SessionContext,
        facet_tree: &crate::facet::evaluated_facet_tree::EvaluatedFacetTree,
        facet_path: &[datafusion::common::ScalarValue],
        coord_measurement: Option<&dyn crate::coords::CoordMeasurement>,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError>;

    /// Measure only the intrinsic subplot overflow, excluding facet-level decorative content
    ///
    /// For facet guides (FacetRow, FacetCol), this returns only the overflow needed by
    /// the Cartesian subplots (axes, tick labels), WITHOUT adding space for facet labels,
    /// titles, or unified axis titles. This is used when measuring nested facets to avoid
    /// double-counting facet-level spacing.
    ///
    /// For non-facet guides (Cartesian, Polar), this is equivalent to `measure_overflow()`.
    async fn measure_intrinsic_overflow(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        theme: &Theme,
        params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
        data_override: Option<&datafusion::dataframe::DataFrame>,
        ctx: &datafusion::prelude::SessionContext,
        facet_tree: &crate::facet::evaluated_facet_tree::EvaluatedFacetTree,
        facet_path: &[datafusion::common::ScalarValue],
        coord_measurement: Option<&dyn crate::coords::CoordMeasurement>,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        // Default implementation: same as measure_overflow()
        self.measure_overflow(
            scales,
            plot_width,
            plot_height,
            theme,
            params,
            data_override,
            ctx,
            facet_tree,
            facet_path,
            coord_measurement,
        )
        .await
    }

    /// Measure overflow with internal self-coordination
    ///
    /// This method enables facet guides to perform their own internal two-pass measurement,
    /// computing both overflow AND spacing needs (e.g., inter-row/col gaps).
    ///
    /// # Default Implementation
    /// Delegates to `measure_overflow()` and wraps the result in a `MeasurementResult`
    /// with empty `spacing_needs`. Non-facet guides (Cartesian, Polar) use this default.
    ///
    /// # Arguments
    /// * `coord_measurement` - Optional, purely for efficiency (see `measure_overflow` docs).
    async fn measure_with_coordination(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        theme: &Theme,
        params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
        data_override: Option<&datafusion::dataframe::DataFrame>,
        ctx: &datafusion::prelude::SessionContext,
        facet_tree: &crate::facet::evaluated_facet_tree::EvaluatedFacetTree,
        facet_path: &[datafusion::common::ScalarValue],
        coord_measurement: Option<&dyn crate::coords::CoordMeasurement>,
    ) -> Result<MeasurementResult, AvengerChartError> {
        // Default: measure once, return with empty spacing_needs
        let overflow = self
            .measure_overflow(
                scales,
                plot_width,
                plot_height,
                theme,
                params,
                data_override,
                ctx,
                facet_tree,
                facet_path,
                coord_measurement,
            )
            .await?;
        Ok(MeasurementResult::new(overflow))
    }

    /// Evaluate this guide to scene marks
    ///
    /// # Arguments
    /// * `data_override` - Optional DataFrame to use instead of compiled data.
    ///   This enables nested facets to pass filtered data to inner guides at runtime.
    ///   When Some, guides should use this data. When None, use compiled data.
    /// * `facet_tree` - Pre-computed facet structure for visibility decisions.
    /// * `facet_path` - Path of values identifying the cell in the facet grid (e.g., `["East", "Eng"]`).
    ///   Empty slice when not in a facet cell.
    /// * `coord_measurement` - Coordinate measurement data from `coord_transform.measure()`.
    ///   For facet guides, this provides subplot measurements including overflow data
    ///   needed for proper label positioning. For non-facet guides, this is `EmptyCoordMeasurement`.
    async fn evaluate(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &LayoutBounds,
        theme: &Theme,
        params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
        ctx: &datafusion::prelude::SessionContext,
        data_override: Option<&datafusion::dataframe::DataFrame>,
        facet_tree: &EvaluatedFacetTree,
        facet_path: &[datafusion::common::ScalarValue],
        coord_measurement: &dyn crate::coords::CoordMeasurement,
    ) -> Result<Vec<SceneMark>, AvengerChartError>;

    /// Get the clipping region for the coordinate system
    ///
    /// Returns the appropriate clip region for marks in this coordinate system.
    /// This is used to ensure marks don't overflow the plot area.
    fn get_clip(
        &self,
        plot_width: f32,
        plot_height: f32,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> avenger_scenegraph::marks::group::Clip;

    /// Determine which channel axis can be unified when this subplot is used in faceting.
    fn facet_unifiable_channel(
        &self,
        _facet_direction: FacetDirection,
        _marks: &[std::sync::Arc<dyn crate::marks::CompiledMark>],
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Option<UnifiableChannelInfo> {
        // Default: no unification
        None
    }

    /// Query the position of an axis by channel name
    fn axis_position(&self, _channel: &str) -> Option<crate::cartesian::axis::AxisPosition> {
        // Default: no position info available
        None
    }

    /// Check if this guide suppresses the specified channel's axis title
    fn unifies_channel(&self, _channel: &str) -> bool {
        // Default: guides render their own axis titles
        false
    }

    /// Get this guide as Any for downcasting to concrete types
    fn as_any(&self) -> &dyn std::any::Any;
}
