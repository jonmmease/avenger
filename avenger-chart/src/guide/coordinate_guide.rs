//! Core trait for coordinate system guides

use std::{any::Any, collections::HashMap, sync::Arc};

use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{group::Clip, mark::SceneMark};
use datafusion::{common::ScalarValue, dataframe::DataFrame, prelude::SessionContext};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{
    axis::Axis,
    cartesian::axis::AxisPosition,
    container::ChildFrameSharingPath,
    coords::CoordMeasurement,
    error::AvengerChartError,
    facet::evaluated_facet_tree::EvaluatedFacetTree,
    guide::{MeasurementResult, OverflowSpaceRequirement},
    layout::LayoutBounds,
    marks::CompiledMark,
    theme::Theme,
};

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

/// Sharing context available while measuring or rendering coordinate guides.
#[derive(Clone, Copy)]
pub struct GuideSharingContext<'a> {
    pub(crate) facet_tree: &'a EvaluatedFacetTree,
    pub(crate) facet_path: &'a [ScalarValue],
    pub(crate) child_frame_sharing_path: &'a ChildFrameSharingPath,
}

impl<'a> GuideSharingContext<'a> {
    pub(crate) fn new(
        facet_tree: &'a EvaluatedFacetTree,
        facet_path: &'a [ScalarValue],
        child_frame_sharing_path: &'a ChildFrameSharingPath,
    ) -> Self {
        Self {
            facet_tree,
            facet_path,
            child_frame_sharing_path,
        }
    }
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
        compiled_marks: Vec<Arc<dyn CompiledMark>>,
        session_context: &SessionContext,
    );

    fn update(&mut self, other: Self);

    fn build(self) -> Box<dyn CompiledGuide>;
}

/// Which overflow contract a guide should use while measuring frame demand.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuideOverflowPhase {
    /// Initial/local measurement before facet coordination has produced a contract.
    Measurement,
    /// Final realization after facet coordination has produced a contract.
    Final,
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
    /// * `sharing_context` - Guide sharing context for visibility decisions.
    ///   It includes the facet path/tree and any generic child-frame sharing
    ///   path for concat-like containers.
    /// * `coord_measurement` - Optional coordinate measurement data from `coord_transform.measure()`.
    ///   **This is purely for efficiency** - when provided, implementations may use pre-computed
    ///   data (e.g., subplot measurements for facet guides) to avoid redundant computation.
    ///   The result must be identical whether or not this is provided.
    async fn measure_overflow(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        theme: &Theme,
        params: &IndexMap<String, ScalarValue>,
        data_override: Option<&DataFrame>,
        ctx: &SessionContext,
        sharing_context: GuideSharingContext<'_>,
        coord_measurement: Option<&dyn CoordMeasurement>,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError>;

    /// Measure overflow for a specific layout phase.
    ///
    /// Most guides have no coordinated-vs-local distinction and can use the
    /// default implementation. Facet guides override this so final frame
    /// solving reserves coordinated guide slots before geometry is solved.
    async fn measure_overflow_for_phase(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        theme: &Theme,
        params: &IndexMap<String, ScalarValue>,
        data_override: Option<&DataFrame>,
        ctx: &SessionContext,
        sharing_context: GuideSharingContext<'_>,
        coord_measurement: Option<&dyn CoordMeasurement>,
        _phase: GuideOverflowPhase,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        self.measure_overflow(
            scales,
            plot_width,
            plot_height,
            theme,
            params,
            data_override,
            ctx,
            sharing_context,
            coord_measurement,
        )
        .await
    }

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
        scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        theme: &Theme,
        params: &IndexMap<String, ScalarValue>,
        data_override: Option<&DataFrame>,
        ctx: &SessionContext,
        sharing_context: GuideSharingContext<'_>,
        coord_measurement: Option<&dyn CoordMeasurement>,
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
            sharing_context,
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
        scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        theme: &Theme,
        params: &IndexMap<String, ScalarValue>,
        data_override: Option<&DataFrame>,
        ctx: &SessionContext,
        sharing_context: GuideSharingContext<'_>,
        coord_measurement: Option<&dyn CoordMeasurement>,
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
                sharing_context,
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
    /// * `sharing_context` - Pre-computed sharing state for facet and generic
    ///   child-frame visibility decisions.
    /// * `coord_measurement` - Coordinate measurement data from `coord_transform.measure()`.
    ///   For facet guides, this provides subplot measurements including overflow data
    ///   needed for proper label positioning. For non-facet guides, this is `EmptyCoordMeasurement`.
    async fn evaluate(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &LayoutBounds,
        guide_overflow: &OverflowSpaceRequirement,
        theme: &Theme,
        params: &IndexMap<String, ScalarValue>,
        ctx: &SessionContext,
        data_override: Option<&DataFrame>,
        sharing_context: GuideSharingContext<'_>,
        coord_measurement: &dyn CoordMeasurement,
    ) -> Result<Vec<SceneMark>, AvengerChartError>;

    /// Get the clipping region for the coordinate system
    ///
    /// Returns the appropriate clip region for marks in this coordinate system.
    /// This is used to ensure marks don't overflow the plot area.
    fn get_clip(
        &self,
        plot_width: f32,
        plot_height: f32,
        scales: &HashMap<String, ConfiguredScale>,
    ) -> Clip;

    /// Determine which channel axis can be unified when this subplot is used in faceting.
    fn facet_unifiable_channel(
        &self,
        _facet_direction: FacetDirection,
        _marks: &[Arc<dyn CompiledMark>],
        _session_context: &SessionContext,
    ) -> Option<UnifiableChannelInfo> {
        // Default: no unification
        None
    }

    /// Query the position of an axis by channel name
    fn axis_position(&self, _channel: &str) -> Option<AxisPosition> {
        // Default: no position info available
        None
    }

    /// Check if this guide suppresses the specified channel's axis title
    fn unifies_channel(&self, _channel: &str) -> bool {
        // Default: guides render their own axis titles
        false
    }

    /// Get this guide as Any for downcasting to concrete types
    fn as_any(&self) -> &dyn Any;
}
