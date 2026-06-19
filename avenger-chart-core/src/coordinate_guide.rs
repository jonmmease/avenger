//! Core trait for coordinate system guides.

use std::{any::Any, collections::HashMap, sync::Arc};

use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{group::Clip, mark::SceneMark};
use datafusion::{common::ScalarValue, dataframe::DataFrame, prelude::SessionContext};
use indexmap::IndexMap;

use crate::{
    AvengerChartError, Axis, AxisGuideVisibilityConfig, AxisPosition, CompiledMarkCore,
    CoordMeasurement, CoordinationAxis, DerivedScalarMap, DerivedScalarsByChannel,
    EventDatumFieldSpec, GuideEventDatumRows, GuideOverflowPhase, LayoutBounds, MeasurementResult,
    OverflowSpaceRequirement, SharingLevel, Theme,
    guide_sharing::{
        AxisOwnershipMode, AxisVisibility, ChildFrameGuideSharingView, FacetGuideSharingView,
    },
};

/// CoordinationScope context available while measuring or rendering coordinate guides.
#[derive(Clone, Copy)]
pub struct GuideSharingContext<'a> {
    facet_view: &'a dyn FacetGuideSharingView,
    facet_path: &'a [ScalarValue],
    child_frame_view: &'a dyn ChildFrameGuideSharingView,
    derived_scalars_by_channel: Option<&'a DerivedScalarsByChannel>,
}

impl<'a> GuideSharingContext<'a> {
    #[doc(hidden)]
    pub fn new(
        facet_view: &'a dyn FacetGuideSharingView,
        facet_path: &'a [ScalarValue],
        child_frame_view: &'a dyn ChildFrameGuideSharingView,
    ) -> Self {
        Self {
            facet_view,
            facet_path,
            child_frame_view,
            derived_scalars_by_channel: None,
        }
    }

    #[doc(hidden)]
    pub fn with_derived_scalars(
        mut self,
        derived_scalars_by_channel: &'a DerivedScalarsByChannel,
    ) -> Self {
        self.derived_scalars_by_channel = Some(derived_scalars_by_channel);
        self
    }

    #[doc(hidden)]
    pub fn derived_scalars_for_channel(&self, channel: &str) -> Option<&'a DerivedScalarMap> {
        self.derived_scalars_by_channel
            .and_then(|scalars| scalars.get(channel))
    }

    pub fn facet_path(&self) -> &'a [ScalarValue] {
        self.facet_path
    }

    pub fn is_root_facet_path(&self) -> bool {
        self.facet_path.is_empty()
    }

    pub fn child_frame_position_indices(&self) -> Vec<usize> {
        self.child_frame_view.position_indices()
    }

    pub fn child_frame_level_counts(&self) -> Vec<usize> {
        self.child_frame_view.level_counts()
    }

    #[doc(hidden)]
    pub fn child_frame_level_axes(&self) -> Vec<CoordinationAxis> {
        self.child_frame_view.level_axes()
    }

    #[doc(hidden)]
    pub fn child_frame_relevant_depth(&self, axis: CoordinationAxis) -> usize {
        self.child_frame_view.relevant_depth(axis)
    }

    #[doc(hidden)]
    pub fn child_frame_axis_guide_visibility_config(
        &self,
        axis: CoordinationAxis,
    ) -> AxisGuideVisibilityConfig {
        self.child_frame_view
            .axis_guide_visibility_config_for_axis(axis)
    }

    #[doc(hidden)]
    pub fn with_facet_path<'b>(&self, facet_path: &'b [ScalarValue]) -> GuideSharingContext<'b>
    where
        'a: 'b,
    {
        GuideSharingContext {
            facet_view: self.facet_view,
            facet_path,
            child_frame_view: self.child_frame_view,
            derived_scalars_by_channel: self.derived_scalars_by_channel,
        }
    }

    #[doc(hidden)]
    pub fn channel_axis_visibility_for_path_checked_with_mode(
        &self,
        axis_position: AxisPosition,
        sharing_level: u8,
        ownership_mode: AxisOwnershipMode,
    ) -> Option<AxisVisibility> {
        self.facet_view
            .channel_axis_visibility_for_path_checked_with_mode(
                self.facet_path,
                axis_position,
                sharing_level,
                ownership_mode,
            )
    }

    #[doc(hidden)]
    pub fn channel_axis_visibility_for_path_checked(
        &self,
        axis_position: AxisPosition,
        sharing_level: u8,
    ) -> Option<AxisVisibility> {
        self.facet_view.channel_axis_visibility_for_path_checked(
            self.facet_path,
            axis_position,
            sharing_level,
        )
    }

    #[doc(hidden)]
    pub fn facet_is_jagged_for_axis(&self, axis_position: AxisPosition) -> bool {
        self.facet_view.is_jagged_for_axis(axis_position)
    }

    #[doc(hidden)]
    pub fn channel_domain_sharing_level(&self, channel: &str) -> SharingLevel {
        self.facet_view.channel_domain_sharing_level(channel)
    }

    #[doc(hidden)]
    pub fn axis_guide_visibility_config(
        &self,
        axis_position: AxisPosition,
    ) -> Option<AxisGuideVisibilityConfig> {
        self.facet_view
            .axis_guide_visibility_config_for_path(self.facet_path, axis_position)
    }

    #[doc(hidden)]
    pub fn facet_guide_labels_visible(
        &self,
        axis_position: AxisPosition,
        sharing_level: u8,
    ) -> bool {
        if self.facet_path.is_empty() {
            return true;
        }

        self.facet_view
            .channel_axis_visibility_for_path_checked(self.facet_path, axis_position, sharing_level)
            .map(|visibility| visibility.show_labels)
            .unwrap_or(true)
    }

    #[doc(hidden)]
    pub fn facet_guide_title_visible(&self, axis_position: AxisPosition) -> bool {
        if self.facet_path.is_empty() {
            return true;
        }

        self.facet_view
            .channel_axis_visibility_for_path_checked(
                self.facet_path,
                axis_position,
                SharingLevel::GLOBAL.raw(),
            )
            .map(|visibility| visibility.show_title)
            .unwrap_or(true)
    }

    #[doc(hidden)]
    pub fn effective_edge_indices_for_values(
        &self,
        values: &[ScalarValue],
    ) -> Option<(usize, usize)> {
        self.facet_view
            .effective_edge_indices_for_values_at_path(self.facet_path, values)
    }
}

/// Trait for visual guides in coordinate systems.
///
/// A `CoordinateGuide` represents the visual reference elements for a
/// coordinate system. This includes both axes configured at the channel level
/// and coordinate-specific options configured at the plot level.
pub trait CoordinateGuide: Clone + Default + Send + Sync {
    type Axis: Axis + Clone + Default;

    /// Set axes that were configured at the channel level.
    fn set_axes(&mut self, axes: HashMap<String, Self::Axis>);

    /// Provide compiled marks so default axis titles can be extracted.
    fn set_compiled_marks<M>(
        &mut self,
        compiled_marks: &[Arc<M>],
        session_context: &SessionContext,
    ) where
        M: CompiledMarkCore + ?Sized;

    fn update(&mut self, other: Self);

    fn build(self) -> Box<dyn CompiledGuide>;
}

#[allow(clippy::too_many_arguments)]
#[async_trait::async_trait]
#[typetag::serde(tag = "type")]
pub trait CompiledGuide: Send + Sync + 'static {
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

    /// Return a guide-defined sharing discriminator for overflow-cache reuse.
    ///
    /// The default keeps cache keys tied to the concrete facet path. Coordinate
    /// guides may return a discriminator when multiple physical facet paths are
    /// known to have equivalent guide visibility and overflow behavior.
    fn overflow_cache_discriminator(
        &self,
        _sharing_context: GuideSharingContext<'_>,
        _phase: GuideOverflowPhase,
    ) -> Option<String> {
        None
    }

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

    /// Event datum fields generated by this guide in addition to source-data fields.
    fn event_datum_field_specs(&self) -> Vec<EventDatumFieldSpec> {
        Vec::new()
    }

    /// Logical datum rows associated with rendered guide scene marks.
    fn event_datum_rows(
        &self,
        _guide_marks: &[SceneMark],
        _plot_width: f32,
        _plot_height: f32,
        _params: &IndexMap<String, ScalarValue>,
        _ctx: &SessionContext,
        _coord_measurement: &dyn CoordMeasurement,
    ) -> Result<Vec<GuideEventDatumRows>, AvengerChartError> {
        Ok(Vec::new())
    }

    fn get_clip(
        &self,
        plot_width: f32,
        plot_height: f32,
        scales: &HashMap<String, ConfiguredScale>,
    ) -> Clip;

    fn axis_position(&self, _channel: &str) -> Option<AxisPosition> {
        None
    }

    fn unifies_channel(&self, _channel: &str) -> bool {
        false
    }

    fn as_any(&self) -> &dyn Any;
}
