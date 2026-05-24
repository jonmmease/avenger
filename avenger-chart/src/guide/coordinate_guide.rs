//! Core trait for coordinate system guides

use std::{any::Any, collections::HashMap, sync::Arc};

use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{group::Clip, mark::SceneMark};
use datafusion::{common::ScalarValue, dataframe::DataFrame, prelude::SessionContext};
use indexmap::IndexMap;

use crate::{
    chart_core::{Axis, AxisPosition, GuideOverflowPhase},
    container::ChildFrameSharingPath,
    coords::CoordMeasurement,
    error::AvengerChartError,
    facet::evaluated_facet_tree::{AxisOwnershipMode, AxisVisibility, EvaluatedFacetTree},
    guide::{MeasurementResult, OverflowSpaceRequirement},
    layout::LayoutBounds,
    marks::CompiledMarkCore,
    plot::compiled::{CoordinationAxis, SharingLevel},
    theme::Theme,
};

/// Sharing context available while measuring or rendering coordinate guides.
#[derive(Clone, Copy)]
pub struct GuideSharingContext<'a> {
    facet_tree: &'a EvaluatedFacetTree,
    facet_path: &'a [ScalarValue],
    child_frame_sharing_path: &'a ChildFrameSharingPath,
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

    pub fn facet_path(&self) -> &'a [ScalarValue] {
        self.facet_path
    }

    pub fn is_root_facet_path(&self) -> bool {
        self.facet_path.is_empty()
    }

    pub fn child_frame_position_indices(&self) -> Vec<usize> {
        self.child_frame_sharing_path.position_indices()
    }

    pub fn child_frame_level_counts(&self) -> Vec<usize> {
        self.child_frame_sharing_path.level_counts()
    }

    pub(crate) fn child_frame_level_axes(&self) -> Vec<CoordinationAxis> {
        self.child_frame_sharing_path.level_axes()
    }

    pub(crate) fn child_frame_relevant_depth(&self, axis: CoordinationAxis) -> usize {
        self.child_frame_sharing_path
            .levels()
            .iter()
            .filter(|level| level.axis == axis)
            .count()
    }

    pub(crate) fn child_frame_sharing_path(&self) -> &'a ChildFrameSharingPath {
        self.child_frame_sharing_path
    }

    pub(crate) fn facet_tree(&self) -> &'a EvaluatedFacetTree {
        self.facet_tree
    }

    pub(crate) fn channel_axis_visibility_for_path_checked_with_mode(
        &self,
        axis_position: AxisPosition,
        sharing_level: u8,
        ownership_mode: AxisOwnershipMode,
    ) -> Option<AxisVisibility> {
        self.facet_tree
            .channel_axis_visibility_for_path_checked_with_mode(
                self.facet_path,
                axis_position,
                sharing_level,
                ownership_mode,
            )
    }

    pub(crate) fn facet_is_jagged_for_axis(&self, axis_position: AxisPosition) -> bool {
        self.facet_tree.is_jagged_for_axis(axis_position)
    }

    pub(crate) fn channel_domain_sharing_level(&self, channel: &str) -> SharingLevel {
        self.facet_tree.channel_domain_sharing_level_typed(channel)
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
    fn set_compiled_marks<M>(
        &mut self,
        compiled_marks: &[Arc<M>],
        session_context: &SessionContext,
    ) where
        M: CompiledMarkCore + ?Sized;

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
