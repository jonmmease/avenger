//! CompiledPlot - Immutable, serializable plot ready for rendering

mod child_frame_container;
pub(crate) mod child_frame_coordination;
mod child_frame_runtime;
mod child_frame_scope;
mod container_band_guide;
mod container_domain_sharing;
mod container_guide;
mod container_labels;
mod container_sharing;
mod coordination_scope;
mod domain_coordination;
mod layout_profile;
mod legends;
mod mark_data_runtime;
pub(crate) mod rendering;
pub mod scale_provider;
pub(crate) mod scales; // Made public so plot.rs can call build_scale_builder_from_marks
mod session;
mod titles;
mod validation;

use std::{
    any::Any,
    collections::{BTreeSet, HashMap},
    sync::{Arc, Mutex},
};

use datafusion::{
    arrow::datatypes::DataType,
    common::{DFSchema, ScalarValue},
    dataframe::DataFrame,
    logical_expr::{Expr, ExprSchemable, LogicalPlan},
    prelude::SessionContext,
};
use datafusion_proto::protobuf::LogicalPlanNode;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use avenger_chart_core::{
    AvengerChartError, AxisSpec, ChannelValue, CompiledDataContext, CompiledGuide, CompiledMark,
    CompiledParamSpec, CompiledSelectionSpec, CompiledStoreSpec, CompiledSubplotChildPlot,
    CompiledSubplotPayload, CoordMeasurement, CoordinateSystemTransform,
    EvaluationContext as CoreEvaluationContext, EventDatumFieldSpec, FacetDataScope, Legend,
    LogicalPlanNodeExt, MarkDataMode, ScaleInferenceHint, ScaleRangeBinding, SerializableDataFrame,
    SerializableScalarMap, Theme, TimeContext, ToolMetadata, channel::strip_trailing_numbers,
};
use avenger_chart_scales::{ConfiguredScaleWithSpec, PlotScaleSpec as ScaleSpec, ScaleBuilder};

use crate::{
    event::ChartEventBinding,
    facet::evaluated_facet_tree::EvaluatedFacetTree,
    layout::{
        ChartResizePolicy, ChildFrameContentMeasurement, ChildFrameContentSolver,
        ContentAllocation, ContentLayout, ContentLayoutSolver, FrameAllocation, FrameDemand,
        LayoutSpec, SinglePlotContentMeasurement, SinglePlotContentSolver,
    },
    render::EvaluationContext,
};

pub use self::child_frame_container::ChildFrameContainerView;
pub(crate) use self::child_frame_container::{
    ChildFrameRegion, child_frame_container_overflow, child_frame_container_view_from_concat,
    child_frame_container_view_from_positioned,
};
pub(crate) use self::child_frame_coordination::ChildFrameLayoutSlot;
pub(crate) use self::child_frame_runtime::{
    ChildFrameDataSelection, ChildFrameRuntime, PreparedChildFramePlot,
    fixed_child_plot_area_layout_spec, measure_child_frame_plot_with_builder,
};
pub(crate) use self::child_frame_scope::{
    ChildFrameKey, ChildFrameScopeKey, ChildFrameSharingLevel, ChildFrameSharingPath,
    ContainerPathSegment, container_path_without_facet_segments,
};
pub(crate) use self::container_band_guide::{
    ContainerBandGuideMeasurementConfig, ContainerBandGuideRenderConfig,
    measure_container_band_guide_slab, render_container_band_guide_slab,
};
pub(crate) use self::container_domain_sharing::{
    ChildFrameChannelDomainExtent, ChildFrameDomainSharingInput, apply_domain_group_to_key,
    child_frame_domain_sharing_levels_for_plot, coordinated_child_frame_domain_extents,
    extract_child_frame_shared_domain_extents,
};
pub(crate) use self::container_guide::{
    measure_child_frame_container_guide_overflow, render_child_frame_container_guide_labels,
};
pub(crate) use self::container_labels::ContainerLabelPlacement;
#[cfg(test)]
pub(crate) use self::container_labels::container_label_items_from_child_frame_container;
pub(crate) use self::container_sharing::{
    ContainerEdgeLevelProjection, EdgeOwnershipRequest, EdgeOwnershipScope, SharingGroupEdge,
    edge_ownership_scope_for_request, owner_for_scope, project_container_edge_levels,
    shared_path_key,
};
pub(crate) use self::coordination_scope::{CoordinationKind, CoordinationScopeKey};
pub(crate) use self::domain_coordination::union_domain_extents;
pub(crate) use self::domain_coordination::{ChildFrameDomainRequest, aggregate_domain_requests};
pub(crate) use self::layout_profile::{
    FacetCellProfileIndex, FacetCellRenderedComponentsProfileCapture, LayoutProfileSnapshot,
};
use self::legends::PreparedLegendPlan;
pub(crate) use self::mark_data_runtime::{
    BaseDataRequest, LogicalMarkDataRequest, MarkDataRequest, PreparedBaseData, PreparedMarkData,
    prepare_base_data, prepare_logical_mark_data, prepare_mark_data as prepare_mark_data_runtime,
};
pub use self::session::{
    EvaluationRequest, PlotSession, ScopedParamAssignment, ScopedParamStoreSnapshot,
    ScopedStoreAssignment, SelectionAssignment, SelectionStateUpdate, StoreStateUpdate,
};
pub(crate) use self::session::{
    GuideOverflowCacheHandle, LegendMeasurementCacheHandle, ScaleDomainCacheHandle,
    ScopedParamStore, ScopedSelectionStore, ScopedStoreState, TextMeasurementCacheHandle,
    TextMeasurementCacheKey,
};

use super::title::{PlotSubtitle, PlotTitle};

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct CompiledColorbarOverlayMarks {
    pub(crate) channel_name: String,
    pub(crate) marks: Vec<Arc<dyn CompiledMark>>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct CompiledMarkGroupState {
    #[serde(default)]
    pub(crate) id: Option<String>,
    #[serde(default)]
    pub(crate) parent_group_index: Option<usize>,
    #[serde(default)]
    pub(crate) scale_inference_hints: Vec<ScaleInferenceHint>,
    #[serde(default)]
    pub(crate) data: CompiledDataContext,
    #[serde(default)]
    pub(crate) data_mode: MarkDataMode,
    pub(crate) facet_data_scope: FacetDataScope,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(crate) struct MarkGroupDataCacheKey {
    pub(crate) plot_identity: usize,
    pub(crate) group_index: usize,
    pub(crate) facet_path: Vec<String>,
}

pub(crate) type MarkGroupDataCacheHandle =
    Arc<Mutex<HashMap<MarkGroupDataCacheKey, Arc<PreparedBaseData>>>>;

fn is_data_transparent_group(group: &CompiledMarkGroupState) -> bool {
    !group.data.has_explicit_data_source()
        && group.data.transforms().is_empty()
        && group.data_mode == MarkDataMode::Inherit
        && group.facet_data_scope == FacetDataScope::FILTERED
}

#[serde_as]
#[derive(Serialize, Deserialize)]
pub struct CompiledPlot {
    /// Coordinate system transform for position mapping
    pub(crate) coord_transform: Box<dyn CoordinateSystemTransform>,

    /// Guide renderer for axes/grids
    pub(crate) compiled_guide: Option<Arc<dyn CompiledGuide>>,

    /// Mark renderers
    pub(crate) marks: Vec<Arc<dyn CompiledMark>>,

    /// Compiled recursive mark-group metadata.
    #[serde(default)]
    pub(crate) mark_groups: Vec<CompiledMarkGroupState>,

    /// Nearest containing mark group for each compiled primitive mark.
    #[serde(default)]
    pub(crate) mark_group_index_by_mark: Vec<Option<usize>>,

    /// Axis specifications
    pub(crate) axis_specs: HashMap<String, AxisSpec>,

    /// Legends
    pub(crate) legends: IndexMap<String, Legend>,

    /// Compiled overlay marks keyed by legend channel name.
    #[serde(default)]
    pub(crate) legend_colorbar_overlays: Vec<CompiledColorbarOverlayMarks>,

    /// Layout specification
    pub(crate) layout_spec: LayoutSpec,

    /// Plot title
    pub(crate) title: Option<PlotTitle>,

    /// Plot subtitle
    pub(crate) subtitle: Option<PlotSubtitle>,

    /// Theme
    pub(crate) theme: Option<Arc<Theme>>,

    /// Time handling defaults for temporal transforms, scales, and guides.
    #[serde(default)]
    pub(crate) time_context: TimeContext,

    /// Mapping from scale names to coordinate channel
    pub(crate) scale_to_coord_channel: HashMap<String, String>,

    /// Scale specifications for building scales
    pub(crate) scale_specs: HashMap<String, ScaleSpec>,

    // Note: We intentionally do not persist a ScaleBuilder here. Scales are
    // rebuilt per evaluation using current params to ensure correctness for
    // paramized data queries and to keep direct vs serialized paths identical.
    /// Plot-level data for mark data inheritance
    #[serde_as(as = "Option<FromInto<SerializableDataFrame>>")]
    pub(crate) data: Option<LogicalPlanNode>,

    /// Default parameter values for prepared statements
    #[serde_as(as = "FromInto<SerializableScalarMap>")]
    pub(crate) default_params: IndexMap<String, ScalarValue>,

    /// Param specs (name, default, sharing scope) keyed by name in declaration order.
    ///
    /// This is the source of truth for parameter sharing. `default_params` is
    /// derived from it and retained for the existing flat-map accessors.
    #[serde(default)]
    pub(crate) param_specs: IndexMap<String, CompiledParamSpec>,

    /// Store specs keyed by name in declaration order.
    #[serde(default)]
    pub(crate) store_specs: IndexMap<String, CompiledStoreSpec>,

    /// Plot-level event bindings for chart apps.
    #[serde(default)]
    pub(crate) event_bindings: Vec<ChartEventBinding>,

    /// Datum columns requested by event bindings, with their compile-time types.
    #[serde(default)]
    pub(crate) event_datum_fields: Vec<EventDatumFieldSpec>,

    /// Coordinate readback columns exposed by event bindings, with their compile-time types.
    #[serde(default)]
    pub(crate) event_coord_fields: Vec<EventDatumFieldSpec>,

    /// Static selection specs registered by the author.
    #[serde(default)]
    pub(crate) selection_specs: IndexMap<String, CompiledSelectionSpec>,

    /// Param names whose values drive app cursor state rather than visual output.
    #[serde(default)]
    pub(crate) cursor_params: Vec<String>,

    /// Metadata for tools that expanded into this compiled plot.
    #[serde(default)]
    pub(crate) tool_metadata: Vec<ToolMetadata>,
}

impl CompiledPlot {
    fn mark_group_data_cache_key(
        &self,
        group_index: usize,
        facet_path: &[ScalarValue],
    ) -> MarkGroupDataCacheKey {
        MarkGroupDataCacheKey {
            plot_identity: self as *const Self as usize,
            group_index,
            facet_path: facet_path
                .iter()
                .map(|value| format!("{value:?}"))
                .collect(),
        }
    }

    pub(crate) fn mark_group_index_for_mark(&self, mark_index: usize) -> Option<usize> {
        self.mark_group_index_by_mark
            .get(mark_index)
            .copied()
            .flatten()
    }

    pub(crate) fn data_group_index_for_mark(&self, mark_index: usize) -> Option<usize> {
        let mut group_index = self.mark_group_index_for_mark(mark_index)?;
        loop {
            let group = self.mark_groups.get(group_index)?;
            if !is_data_transparent_group(group) {
                return Some(group_index);
            }
            group_index = group.parent_group_index?;
        }
    }

    pub(crate) fn scale_inference_hints_for_mark(
        &self,
        mark_index: usize,
    ) -> Result<Vec<ScaleInferenceHint>, AvengerChartError> {
        let Some(mut group_index) = self.mark_group_index_for_mark(mark_index) else {
            return Ok(Vec::new());
        };
        let mut ancestry = Vec::new();
        loop {
            let group = self.mark_groups.get(group_index).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Compiled mark group index {group_index} is out of bounds"
                ))
            })?;
            ancestry.push(group_index);
            if let Some(parent_index) = group.parent_group_index {
                group_index = parent_index;
            } else {
                break;
            }
        }
        ancestry.reverse();
        let mut hints = Vec::new();
        for group_index in ancestry {
            hints.extend(self.mark_groups[group_index].scale_inference_hints.clone());
        }
        Ok(hints)
    }

    pub(crate) async fn prepare_mark_group_base_data(
        &self,
        group_index: usize,
        eval_ctx: &EvaluationContext,
        provided_plot_df: Option<&DataFrame>,
        facet_path: &[ScalarValue],
    ) -> Result<Arc<PreparedBaseData>, AvengerChartError> {
        let key = self.mark_group_data_cache_key(group_index, facet_path);
        if let Some(cached) = eval_ctx
            .mark_group_data_cache
            .lock()
            .expect("mark-group data cache lock poisoned")
            .get(&key)
            .cloned()
        {
            return Ok(cached);
        }

        let group = self.mark_groups.get(group_index).ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Compiled mark group index {group_index} is out of bounds"
            ))
        })?;
        let inherited_base = if group.data.has_explicit_data_source() {
            None
        } else if let Some(parent_index) = group.parent_group_index {
            Some(
                Box::pin(self.prepare_mark_group_base_data(
                    parent_index,
                    eval_ctx,
                    provided_plot_df,
                    facet_path,
                ))
                .await?,
            )
        } else {
            None
        };

        let prepared = prepare_base_data(BaseDataRequest {
            data_context: &group.data,
            data_mode: group.data_mode,
            facet_data_scope: group.facet_data_scope,
            plot_data: self.data.as_ref(),
            provided_plot_df,
            inherited_base: inherited_base.as_deref(),
            facet_data_scope_context: Some(crate::facet::data_scope::FacetDataScopeContext::new(
                eval_ctx.facet_tree.as_ref(),
                eval_ctx.facet_data_root(),
                facet_path,
            )),
            eval_ctx,
        })
        .await?;
        let prepared = Arc::new(prepared);
        eval_ctx
            .mark_group_data_cache
            .lock()
            .expect("mark-group data cache lock poisoned")
            .insert(key, prepared.clone());
        Ok(prepared)
    }

    /// Get the theme or create default if not set
    pub fn get_theme(&self) -> Arc<Theme> {
        self.theme
            .clone()
            .unwrap_or_else(|| Arc::new(Theme::light()))
    }

    /// Get title if configured
    pub fn get_title(&self) -> Option<&PlotTitle> {
        self.title.as_ref()
    }

    /// Get subtitle if configured
    pub fn get_subtitle(&self) -> Option<&PlotSubtitle> {
        self.subtitle.as_ref()
    }

    /// Get layout spec
    pub fn get_layout_spec(&self) -> &LayoutSpec {
        &self.layout_spec
    }

    /// Get the app-facing resize policy for the compiled layout.
    pub fn resize_policy(&self) -> ChartResizePolicy {
        self.layout_spec.resize_policy()
    }

    /// Get default parameter values
    pub fn get_default_params(&self) -> &IndexMap<String, ScalarValue> {
        &self.default_params
    }

    /// Get the param specs (name, default, sharing) in declaration order.
    pub fn param_specs(&self) -> &IndexMap<String, CompiledParamSpec> {
        &self.param_specs
    }

    pub fn store_specs(&self) -> &IndexMap<String, CompiledStoreSpec> {
        &self.store_specs
    }

    /// Get plot-level event bindings.
    pub fn event_bindings(&self) -> &[ChartEventBinding] {
        &self.event_bindings
    }

    pub fn event_datum_types(&self) -> IndexMap<String, DataType> {
        self.event_datum_fields
            .iter()
            .map(|field| (field.name.clone(), field.data_type.clone()))
            .collect()
    }

    /// Infer app event-coordinate column types from invertible coordinate
    /// channel scale inputs.
    ///
    /// Continuous coordinates keep the app's default `Float64` event-column
    /// type. Categorical coordinates override that default with their domain
    /// value type, so nested band coordinates expose struct-valued readback with
    /// the original level field names.
    pub fn event_coord_types(
        &self,
        _ctx: &SessionContext,
    ) -> Result<IndexMap<String, DataType>, AvengerChartError> {
        Ok(self
            .event_coord_fields
            .iter()
            .map(|field| (field.name.clone(), field.data_type.clone()))
            .collect())
    }

    pub(crate) async fn infer_event_coord_fields(
        &self,
        ctx: &SessionContext,
    ) -> Result<Vec<EventDatumFieldSpec>, AvengerChartError> {
        let mut requested = BTreeSet::new();
        for binding in &self.event_bindings {
            requested.extend(
                crate::event::scan_chart_event_binding_interaction_columns(binding, ctx)?
                    .all_channels(),
            );
        }
        if requested.is_empty() {
            return Ok(Vec::new());
        }

        let mut types = IndexMap::new();
        self.collect_event_coord_types(ctx, &mut types, None, None)
            .await?;
        Ok(types
            .into_iter()
            .map(|(name, data_type)| EventDatumFieldSpec { name, data_type })
            .collect())
    }

    async fn collect_event_coord_types(
        &self,
        ctx: &SessionContext,
        out: &mut IndexMap<String, DataType>,
        inherited_plot_data: Option<&LogicalPlanNode>,
        inherited_store_specs: Option<&IndexMap<String, CompiledStoreSpec>>,
    ) -> Result<(), AvengerChartError> {
        let plot_data = self.data.as_ref().or(inherited_plot_data);
        let store_specs = if self.store_specs.is_empty() {
            inherited_store_specs.unwrap_or(&self.store_specs)
        } else {
            &self.store_specs
        };
        let eval_ctx = self.schema_inference_evaluation_context(ctx, Some(store_specs));
        let inherited_df = if self.data.is_none() {
            inherited_plot_data.and_then(|node| {
                node.to_logical_plan(ctx)
                    .ok()
                    .map(|plan| DataFrame::new(ctx.state().clone(), plan))
            })
        } else {
            None
        };
        collect_event_coord_types_from_marks(
            self.coord_transform.as_ref(),
            self,
            &self.marks,
            plot_data,
            inherited_df.as_ref(),
            ctx,
            &eval_ctx,
            out,
        )
        .await?;

        for mark in &self.marks {
            if let Some(subplot) = crate::concat::compiled_subplot(mark.as_ref()) {
                Box::pin(subplot.compiled_subplot().collect_event_coord_types(
                    ctx,
                    out,
                    plot_data,
                    Some(store_specs),
                ))
                .await?;
            }
            if let Some(subplot) = crate::facet::marks::facet::facet_subplot_ref(mark.as_ref()) {
                Box::pin(subplot.compiled_subplot().collect_event_coord_types(
                    ctx,
                    out,
                    plot_data,
                    Some(store_specs),
                ))
                .await?;
            }
            for payload in mark.child_plot_payloads() {
                Box::pin(
                    compiled_subplot_payload_child_plot(payload).collect_event_coord_types(
                        ctx,
                        out,
                        plot_data,
                        Some(store_specs),
                    ),
                )
                .await?;
            }
        }

        Ok(())
    }

    fn schema_inference_evaluation_context(
        &self,
        ctx: &SessionContext,
        store_specs: Option<&IndexMap<String, CompiledStoreSpec>>,
    ) -> EvaluationContext {
        let mut eval_ctx = EvaluationContext::new(
            self.get_theme(),
            Arc::new(ctx.clone()),
            self.default_params.clone(),
            Arc::new(EvaluatedFacetTree::empty()),
        )
        .with_time_context(self.time_context.clone());
        let store_specs = store_specs.unwrap_or(&self.store_specs);
        if !store_specs.is_empty() {
            eval_ctx = eval_ctx
                .with_scoped_store_state(Arc::new(ScopedStoreState::new(store_specs.clone())));
        }
        eval_ctx
    }

    pub(crate) async fn infer_event_datum_fields(
        &self,
        ctx: &SessionContext,
    ) -> Result<Vec<EventDatumFieldSpec>, AvengerChartError> {
        let mut requested = BTreeSet::new();
        for binding in &self.event_bindings {
            requested.extend(
                crate::event::scan_chart_event_binding_interaction_columns(binding, ctx)?
                    .current_datum,
            );
        }
        let explicit_requested = requested.clone();
        for mark in &self.marks {
            if let Some(details) = mark.state().details.as_ref() {
                requested.extend(details.iter().cloned());
            }
        }
        if requested.is_empty() {
            return Ok(Vec::new());
        }

        let mut types = IndexMap::new();
        collect_reserved_event_datum_types(&requested, &mut types);
        match self
            .collect_event_datum_types(ctx, &requested, &mut types, None, None)
            .await
        {
            Ok(()) => {}
            Err(AvengerChartError::InternalError(message))
                if explicit_requested.is_empty()
                    && message == "Mark expressions reference columns but no data is available" =>
            {
                return Ok(Vec::new());
            }
            Err(err) => return Err(err),
        }
        let missing = requested
            .iter()
            .filter(|field| !types.contains_key(*field))
            .cloned()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Event binding requested datum field(s) {} but no compiled mark or plot data source exposes them",
                missing.join(", ")
            )));
        }
        Ok(types
            .into_iter()
            .map(|(name, data_type)| EventDatumFieldSpec { name, data_type })
            .collect())
    }

    async fn collect_event_datum_types(
        &self,
        ctx: &SessionContext,
        requested: &BTreeSet<String>,
        out: &mut IndexMap<String, DataType>,
        inherited_plot_data: Option<&LogicalPlanNode>,
        inherited_store_specs: Option<&IndexMap<String, CompiledStoreSpec>>,
    ) -> Result<(), AvengerChartError> {
        let plot_data = self.data.as_ref().or(inherited_plot_data);
        let store_specs = if self.store_specs.is_empty() {
            inherited_store_specs.unwrap_or(&self.store_specs)
        } else {
            &self.store_specs
        };
        if let Some(df) = plot_data.and_then(|node| {
            node.to_logical_plan(ctx)
                .ok()
                .map(|plan| DataFrame::new(ctx.state().clone(), plan))
        }) {
            collect_event_datum_types_from_schema(&df, requested, out);
        }
        if let Some(guide) = self.compiled_guide.as_ref() {
            collect_event_datum_types_from_specs(guide.event_datum_field_specs(), requested, out)?;
        }

        let eval_ctx = self.schema_inference_evaluation_context(ctx, Some(store_specs));
        let inherited_df = if self.data.is_none() {
            inherited_plot_data.and_then(|node| {
                node.to_logical_plan(ctx)
                    .ok()
                    .map(|plan| DataFrame::new(ctx.state().clone(), plan))
            })
        } else {
            None
        };

        for mark in &self.marks {
            collect_event_datum_types_from_specs(mark.event_datum_field_specs(), requested, out)?;
            if let Some(store_data) = mark.data_context().store_data()
                && let Some(spec) = store_specs.get(&store_data.store_name)
            {
                collect_event_datum_types_from_store_spec(spec, requested, out);
            }
            let prepared_base = match self.data_group_index_for_mark(mark.state().mark_index()) {
                Some(group_index) => Some(
                    Box::pin(self.prepare_mark_group_base_data(
                        group_index,
                        &eval_ctx,
                        inherited_df.as_ref(),
                        &[],
                    ))
                    .await?,
                ),
                None => None,
            };
            let prepared = Box::pin(prepare_logical_mark_data(LogicalMarkDataRequest {
                mark: mark.as_ref(),
                plot_data,
                provided_plot_df: inherited_df.as_ref(),
                facet_data_scope: None,
                prepared_base: prepared_base.as_deref(),
                eval_ctx: &eval_ctx,
            }))
            .await?;
            if let Some(df) = prepared.dataframe.as_ref() {
                collect_event_datum_types_from_schema(&df, requested, out);
            }
            if let Some(subplot) = crate::concat::compiled_subplot(mark.as_ref()) {
                Box::pin(subplot.compiled_subplot().collect_event_datum_types(
                    ctx,
                    requested,
                    out,
                    plot_data,
                    Some(store_specs),
                ))
                .await?;
            }
            if let Some(subplot) = crate::facet::marks::facet::facet_subplot_ref(mark.as_ref()) {
                Box::pin(subplot.compiled_subplot().collect_event_datum_types(
                    ctx,
                    requested,
                    out,
                    plot_data,
                    Some(store_specs),
                ))
                .await?;
            }
            for payload in mark.child_plot_payloads() {
                Box::pin(
                    compiled_subplot_payload_child_plot(payload).collect_event_datum_types(
                        ctx,
                        requested,
                        out,
                        plot_data,
                        Some(store_specs),
                    ),
                )
                .await?;
            }
        }
        Ok(())
    }

    pub fn selection_specs(&self) -> &IndexMap<String, CompiledSelectionSpec> {
        &self.selection_specs
    }

    pub fn cursor_params(&self) -> &[String] {
        &self.cursor_params
    }

    pub fn tool_metadata(&self) -> &[ToolMetadata] {
        &self.tool_metadata
    }

    /// Get compiled mark renderers
    pub fn marks(&self) -> &[Arc<dyn CompiledMark>] {
        &self.marks
    }

    /// Build scales from an existing ScaleBuilder with specific dimensions.
    ///
    /// Reuses cached data queries from the provided builder, making it efficient
    /// for building scales multiple times (e.g., initial layout pass and final
    /// render pass).
    ///
    /// Typical usage:
    /// ```ignore
    /// // Build once (queries data)
    /// let builder = build_scale_builder_from_marks(...).await?;
    ///
    /// // Reuse multiple times (no queries)
    /// let scales1 = plot.build_scales_from_builder(&builder, 400.0, 300.0, ctx, params).await?;
    /// let scales2 = plot.build_scales_from_builder(&builder, 800.0, 600.0, ctx, params).await?;
    /// ```
    pub async fn build_scales_from_builder(
        &self,
        builder: &ScaleBuilder,
        plot_area_width: f32,
        plot_area_height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<HashMap<String, ConfiguredScaleWithSpec>, AvengerChartError> {
        // Build coordinate system range bindings map
        let mut coord_system_range_bindings = HashMap::<String, ScaleRangeBinding>::new();
        for channel in builder.channel_builders().keys() {
            let base = self
                .scale_to_coord_channel
                .get(channel)
                .map(String::as_str)
                .unwrap_or_else(|| strip_trailing_numbers(channel));
            if let Some(binding) = self.coord_transform.default_range_binding(base) {
                coord_system_range_bindings.insert(channel.clone(), binding);
            }
        }

        let theme = self.get_theme();
        let default_range_resolver = scales::default_range_for_compiled_marks(&self.marks);
        let built = Box::pin(builder.build_scales(
            plot_area_width,
            plot_area_height,
            &coord_system_range_bindings,
            &self.scale_specs,
            &default_range_resolver,
            theme.as_ref(),
            ctx,
            params,
        ))
        .await?;

        Ok(built)
    }

    /// Get scale specifications
    pub fn scale_specs(&self) -> &HashMap<String, ScaleSpec> {
        &self.scale_specs
    }

    /// Get legends
    pub fn legends(&self) -> &IndexMap<String, Legend> {
        &self.legends
    }

    /// Build configured scales for a provided DataFrame and plot-area dimensions.
    pub async fn build_scales_for_dataframe(
        &self,
        df: &DataFrame,
        plot_area_width: f32,
        plot_area_height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<HashMap<String, ConfiguredScaleWithSpec>, AvengerChartError> {
        let eval_ctx =
            CoreEvaluationContext::new(self.get_theme(), Arc::new(ctx.clone()), params.clone())
                .with_time_context(self.time_context.clone());
        let scale_builder = Box::pin(scales::build_scale_builder_from_compiled_plot(
            self,
            Some(df.clone()),
            &eval_ctx,
            self.get_theme().as_ref(),
        ))
        .await?;

        Box::pin(self.build_scales_from_builder(
            &scale_builder,
            plot_area_width,
            plot_area_height,
            ctx,
            params,
        ))
        .await
    }
}

fn collect_reserved_event_datum_types(
    requested: &BTreeSet<String>,
    out: &mut IndexMap<String, DataType>,
) {
    use avenger_chart_core::event::{
        LEGEND_BAND_CHANNEL_FIELD, LEGEND_CHANNEL_FIELD, LEGEND_ID_FIELD, LEGEND_INDEX_FIELD,
        LEGEND_LABEL_FIELD, LEGEND_NAME_FIELD, LEGEND_ORIENTATION_FIELD, LEGEND_SURFACE_KEY_FIELD,
        LEGEND_SURFACE_KIND_FIELD, LEGEND_VALUE_CHANNEL_FIELD, LEGEND_VALUE_FIELD,
    };

    for (name, data_type) in [
        (LEGEND_VALUE_FIELD, DataType::Utf8),
        (LEGEND_LABEL_FIELD, DataType::Utf8),
        (LEGEND_NAME_FIELD, DataType::Utf8),
        (LEGEND_CHANNEL_FIELD, DataType::Utf8),
        (LEGEND_INDEX_FIELD, DataType::Int64),
        (LEGEND_ID_FIELD, DataType::Utf8),
        (LEGEND_SURFACE_KEY_FIELD, DataType::Utf8),
        (LEGEND_SURFACE_KIND_FIELD, DataType::Utf8),
        (LEGEND_ORIENTATION_FIELD, DataType::Utf8),
        (LEGEND_VALUE_CHANNEL_FIELD, DataType::Utf8),
        (LEGEND_BAND_CHANNEL_FIELD, DataType::Utf8),
    ] {
        if requested.contains(name) && !out.contains_key(name) {
            out.insert(name.to_string(), data_type);
        }
    }
}

fn collect_event_datum_types_from_specs(
    specs: Vec<EventDatumFieldSpec>,
    requested: &BTreeSet<String>,
    out: &mut IndexMap<String, DataType>,
) -> Result<(), AvengerChartError> {
    for spec in specs {
        if !requested.contains(&spec.name) {
            continue;
        }
        if let Some(existing) = out.get(&spec.name) {
            if existing != &spec.data_type {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Event datum field '{}' was declared with conflicting types {:?} and {:?}",
                    spec.name, existing, spec.data_type
                )));
            }
            continue;
        }
        out.insert(spec.name, spec.data_type);
    }
    Ok(())
}

pub(crate) fn compiled_subplot_payload_child_plot(
    payload: &CompiledSubplotPayload,
) -> &CompiledPlot {
    payload
        .compiled_child_plot()
        .as_any()
        .downcast_ref::<CompiledPlot>()
        .expect("subplot payload child plot is not an avenger-chart CompiledPlot")
}

fn collect_event_datum_types_from_schema(
    df: &DataFrame,
    requested: &BTreeSet<String>,
    out: &mut IndexMap<String, DataType>,
) {
    for field in df.schema().fields() {
        let name = field.name();
        if requested.contains(name) && !out.contains_key(name) {
            out.insert(name.clone(), field.data_type().clone());
        }
    }
}

fn collect_event_datum_types_from_store_spec(
    spec: &CompiledStoreSpec,
    requested: &BTreeSet<String>,
    out: &mut IndexMap<String, DataType>,
) {
    for field in &spec.fields {
        if requested.contains(&field.name) && !out.contains_key(&field.name) {
            out.insert(field.name.clone(), field.data_type.clone());
        }
    }
}

async fn collect_event_coord_types_from_marks(
    coord_transform: &dyn CoordinateSystemTransform,
    plot: &CompiledPlot,
    marks: &[Arc<dyn CompiledMark>],
    plot_data: Option<&LogicalPlanNode>,
    provided_plot_df: Option<&DataFrame>,
    ctx: &SessionContext,
    eval_ctx: &EvaluationContext,
    out: &mut IndexMap<String, DataType>,
) -> Result<(), AvengerChartError> {
    let invertible = coord_transform.interaction_invertible_channels();
    if invertible.is_empty() {
        return Ok(());
    }

    for mark in marks {
        let prepared_base = match plot.data_group_index_for_mark(mark.state().mark_index()) {
            Some(group_index) => Some(
                Box::pin(plot.prepare_mark_group_base_data(
                    group_index,
                    eval_ctx,
                    provided_plot_df,
                    &[],
                ))
                .await?,
            ),
            None => None,
        };
        let prepared = match Box::pin(prepare_logical_mark_data(LogicalMarkDataRequest {
            mark: mark.as_ref(),
            plot_data,
            provided_plot_df,
            facet_data_scope: None,
            prepared_base: prepared_base.as_deref(),
            eval_ctx,
        }))
        .await
        {
            Ok(prepared) => prepared,
            Err(AvengerChartError::InternalError(message))
                if message == "Mark expressions reference columns but no data is available" =>
            {
                continue;
            }
            Err(err) => return Err(err),
        };
        let mark_df = prepared.domain_dataframe.as_ref();
        let channels = &prepared.domain_channels;
        let scale_inference_hints =
            plot.scale_inference_hints_for_mark(mark.state().mark_index())?;

        for channel in &invertible {
            if out.contains_key(channel.as_str()) {
                continue;
            }
            let Some(data_type) = infer_event_coord_type_for_channel(
                coord_transform,
                mark.as_ref(),
                channels,
                mark_df,
                ctx,
                channel.as_str(),
                &scale_inference_hints,
            )?
            else {
                continue;
            };
            out.insert(channel.clone(), data_type);
        }
    }

    Ok(())
}

fn infer_event_coord_type_for_channel(
    coord_transform: &dyn CoordinateSystemTransform,
    mark: &dyn CompiledMark,
    channels: &IndexMap<String, ChannelValue>,
    dataframe: Option<&DataFrame>,
    ctx: &SessionContext,
    target_channel: &str,
    scale_inference_hints: &[ScaleInferenceHint],
) -> Result<Option<DataType>, AvengerChartError> {
    for (channel_name, channel_value) in channels {
        if !channel_maps_to_event_coord_scale(
            coord_transform,
            channel_name,
            channel_value,
            target_channel,
        ) {
            continue;
        }

        let Some(input_type) = infer_scale_input_data_type(channel_value, dataframe, ctx) else {
            continue;
        };
        let preferred = mark.preferred_scale_type(channel_name, &input_type);
        let hinted_preference = scale_inference_hints
            .iter()
            .find(|hint| hint.scale_name == target_channel)
            .map(|hint| hint.preference);
        let is_categorical_coord = matches!(
            hinted_preference,
            Some(avenger_chart_core::ScaleTypePreference::Band)
                | Some(avenger_chart_core::ScaleTypePreference::Point)
                | Some(avenger_chart_core::ScaleTypePreference::NestedBand)
        ) || matches!(
            preferred,
            Some(avenger_chart_core::ScaleTypePreference::Band)
                | Some(avenger_chart_core::ScaleTypePreference::Point)
                | Some(avenger_chart_core::ScaleTypePreference::NestedBand)
        ) || categorical_event_coord_type(&input_type);

        if is_categorical_coord {
            return Ok(Some(input_type));
        }
    }

    Ok(None)
}

fn channel_maps_to_event_coord_scale(
    coord_transform: &dyn CoordinateSystemTransform,
    channel_name: &str,
    channel_value: &ChannelValue,
    target_channel: &str,
) -> bool {
    if !coord_transform.channel_uses_scale(channel_name) {
        return false;
    }

    channel_value
        .get_scale_name(channel_name)
        .map(|scale_name| scale_name == target_channel)
        .unwrap_or(channel_name == target_channel)
}

fn infer_scale_input_data_type(
    channel_value: &ChannelValue,
    dataframe: Option<&DataFrame>,
    ctx: &SessionContext,
) -> Option<DataType> {
    let expr = channel_value.scale_input_expr(ctx)?;
    if let Some(df) = dataframe.filter(|df| !is_empty_relation(df)) {
        return infer_expr_data_type(&expr, df);
    }
    if expr.column_refs().is_empty() {
        expr.get_type(&DFSchema::empty()).ok()
    } else {
        None
    }
}

fn infer_expr_data_type(expr: &Expr, df: &DataFrame) -> Option<DataType> {
    match expr {
        Expr::Column(col) => df
            .schema()
            .field_with_unqualified_name(&col.name)
            .ok()
            .map(|field| field.data_type().clone()),
        _ => df
            .clone()
            .select(vec![expr.clone().alias("__event_coord_type")])
            .ok()
            .map(|projected| projected.schema().field(0).data_type().clone()),
    }
}

fn categorical_event_coord_type(data_type: &DataType) -> bool {
    match data_type {
        DataType::Struct(_) | DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => true,
        DataType::Dictionary(_, value_type) => categorical_event_coord_type(value_type.as_ref()),
        _ => false,
    }
}

fn is_empty_relation(df: &DataFrame) -> bool {
    matches!(df.logical_plan(), LogicalPlan::EmptyRelation(_))
}

pub(crate) fn compiled_subplot_payload_child_plot_arc(
    payload: &CompiledSubplotPayload,
) -> Arc<CompiledPlot> {
    match Arc::clone(payload.compiled_child_plot())
        .into_any_arc()
        .downcast::<CompiledPlot>()
    {
        Ok(compiled) => compiled,
        Err(_) => panic!("subplot payload child plot is not an avenger-chart CompiledPlot"),
    }
}

#[typetag::serde]
impl CompiledSubplotChildPlot for CompiledPlot {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

/// Measurement results from `measure_plot_components`
///
/// This captures all the computation needed for layout coordination without
/// actually rendering any marks. The render pass uses this to avoid re-measuring.
///
/// Overflow info is available via `layout.overflow` (guide only) and
/// `layout.total_overflow` (guide + legends).
#[derive(Clone)]
pub struct ComponentsMeasurement {
    /// Coordinate-system-specific measurement data (e.g., facet cell layout)
    ///
    /// This is computed by top-level coordinate measurement dispatch and
    /// contains layout data that's available to both guides and marks during
    /// rendering. For facet coordinate systems, this includes cell positions,
    /// subplot measurements, and computed padding. For non-layout coordinate
    /// systems, this is an `EmptyCoordMeasurement`.
    pub coord_measurement: Box<dyn CoordMeasurement>,

    /// Scales for rendering
    pub scales: std::collections::HashMap<String, ConfiguredScaleWithSpec>,

    /// Plot area dimensions
    pub plot_area_width: f32,
    pub plot_area_height: f32,

    /// Canvas size
    pub canvas_size: (f32, f32),

    /// Clip region for data marks
    pub clip: avenger_scenegraph::marks::group::Clip,

    /// Layout solution (contains overflow, total_overflow, legends/titles positioning)
    pub layout: crate::render::LayoutSolution,

    /// Allocation granted to this measured chart frame by its parent.
    pub frame_allocation: FrameAllocation,

    /// Merged params (defaults + provided + canvas dimensions)
    pub params: indexmap::IndexMap<String, datafusion::common::ScalarValue>,

    /// Prepared legend plan used by both layout and render phases
    pub(crate) legend_plan: PreparedLegendPlan,
}

impl std::fmt::Debug for ComponentsMeasurement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComponentsMeasurement")
            .field("coord_measurement", &"<coord_measurement>")
            .field("scales", &format!("{} scales", self.scales.len()))
            .field("plot_area_width", &self.plot_area_width)
            .field("plot_area_height", &self.plot_area_height)
            .field("canvas_size", &self.canvas_size)
            .field("layout", &"LayoutSolution")
            .field("frame_allocation", &self.frame_allocation)
            .field("legend_plan", &"<legend_plan>")
            .finish()
    }
}

impl ComponentsMeasurement {
    pub(crate) fn refresh_frame_allocation_rect(&mut self) {
        self.frame_allocation.rect.width = self.canvas_size.0;
        self.frame_allocation.rect.height = self.canvas_size.1;
    }

    pub(crate) fn sync_canvas_size_from_layout(&mut self) {
        self.canvas_size = self.layout.canvas_size;
        self.refresh_frame_allocation_rect();
    }

    pub(crate) fn frame_demand(&self) -> FrameDemand {
        FrameDemand::from_guide_and_rendered_envelope(
            self.layout.overflow.clone(),
            self.layout.total_overflow.clone(),
        )
    }

    pub(crate) fn content_allocation(&self) -> ContentAllocation {
        ContentAllocation::new(self.frame_allocation, *self.layout.plot_area_bounds())
    }

    pub(crate) fn content_layout(&self) -> Result<ContentLayout, AvengerChartError> {
        let allocation = self.content_allocation();
        let frame_demand = self.frame_demand();

        if let Some(container) = self.child_frame_container_view()? {
            let solver = ChildFrameContentSolver;
            let child_frame_allocations = container.child_frame_allocations();
            let demand = solver.measure_content_demand(
                &allocation,
                &ChildFrameContentMeasurement {
                    frame_demand,
                    child_frame_allocations,
                },
            )?;
            let plan = solver.coordinate_content(&allocation, &demand)?;
            solver.realize_content(allocation, demand, plan)
        } else {
            let solver = SinglePlotContentSolver;
            let demand = solver.measure_content_demand(
                &allocation,
                &SinglePlotContentMeasurement { frame_demand },
            )?;
            let plan = solver.coordinate_content(&allocation, &demand)?;
            solver.realize_content(allocation, demand, plan)
        }
    }
}

/// Extended components returned from plot evaluation
///
/// This structure supports both measurement and rendering modes,
/// and includes all plot components (data, guides, legends, titles).
#[derive(Clone)]
pub struct PlotComponents {
    /// Data mark scene graph elements
    pub data_marks: Vec<avenger_scenegraph::marks::mark::SceneMark>,

    /// Guide mark scene graph elements (axes, grids)
    pub guide_marks: Vec<avenger_scenegraph::marks::mark::SceneMark>,

    /// Legend scene graph elements
    pub legend_marks: Vec<avenger_scenegraph::marks::mark::SceneMark>,

    /// Title scene graph elements
    pub title_marks: Vec<avenger_scenegraph::marks::mark::SceneMark>,

    /// Subtitle scene graph elements
    pub subtitle_marks: Vec<avenger_scenegraph::marks::mark::SceneMark>,

    /// Plot area bounds (data rectangle)
    pub plot_bounds: crate::layout::LayoutBounds,

    /// Clip region for data marks
    pub clip: avenger_scenegraph::marks::group::Clip,

    /// Size dimensions used for this evaluation
    pub size: (f32, f32),

    /// Whether `size` represents canvas dimensions (true) or plot area dimensions (false)
    pub size_is_canvas: bool,

    /// Debug marks (layout visualization) - these are in absolute canvas coordinates
    pub debug_marks: Vec<avenger_scenegraph::marks::mark::SceneMark>,

    /// Interaction scopes produced by this plot, in local scene coordinates.
    ///
    /// Container renderers (facets, concat, positioned subplots) translate child
    /// scope bounds by the same origin used for the child scene group, the same
    /// way they translate child scene marks.
    pub interaction_scopes: Vec<crate::render::EvaluatedInteractionScope>,

    /// Event datum rows produced by data marks, keyed by local scene-mark path.
    pub event_datums: Vec<crate::render::EvaluatedEventDatumRows>,

    /// Event datum rows produced by frame chrome, keyed by component-local scene-mark path.
    pub chrome_event_datums: Vec<crate::render::EvaluatedEventDatumRows>,
}
