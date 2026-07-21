//! CompiledPlot - Immutable, serializable plot ready for rendering

mod bake;
mod child_frame_container;
pub(crate) mod child_frame_coordination;
mod child_frame_runtime;
mod child_frame_scope;
mod container_band_guide;
mod container_domain_sharing;
mod container_guide;
mod container_labels;
mod container_sharing;
mod coordinate_domains;
mod coordination_scope;
mod domain_coordination;
mod layout_profile;
mod legends;
mod mark_data_runtime;
mod materialization;
mod native_runtime;
pub(crate) mod rendering;
pub mod scale_provider;
pub(crate) mod scales; // Made public so plot.rs can call build_scale_builder_from_marks
mod session;
mod titles;
mod validation;

use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet, HashMap},
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
    CompiledParamSpec, CompiledSelectionSpec, CompiledStateRegistry, CompiledStoreSpec,
    CompiledSubplotChildPlot, CompiledSubplotPayload, CompiledToolBehavior, CompiledViewScope,
    CoordMeasurement, CoordinateDomainResolvedState, CoordinateSystemTransform,
    EvaluationContext as CoreEvaluationContext, EventDatumFieldSpec, FacetDataScope,
    FormattingContext, Legend, LogicalPlanNodeExt, MarkDataMode, ParamRef, ScaleInferenceHint,
    ScaleRangeBinding, SelectionRef, SerializableDataFrame, SerializableScalarMap, StoreRef, Theme,
    TimeContext, ToolMetadata, channel::strip_trailing_numbers,
};
use avenger_chart_scales::{ConfiguredScaleWithSpec, PlotScaleSpec as ScaleSpec, ScaleBuilder};

use crate::{
    bake::{BakedTableManifestEntry, PlotBakeReport},
    event::{ChartEventBinding, ChartParamChangeBinding},
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
    coordinate_domain_cell_for_builder, fixed_child_plot_area_layout_spec,
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
    ChildFrameChannelDomainExtent, ChildFrameCoordinateDomainCell,
    ChildFrameCoordinateDomainGroupKind, ChildFrameDomainSharingInput, apply_domain_group_to_key,
    child_frame_coordinate_domain_node_for_scale, child_frame_domain_sharing_levels_for_plot,
    coordinated_child_frame_domain_extents, extract_child_frame_domain_extents,
    resolve_child_frame_coordinate_domain_extents,
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
pub(crate) use self::coordinate_domains::{
    CoordinateDomainBuildPolicy, apply_coordinate_domain_overrides,
};
pub(crate) use self::coordination_scope::{CoordinationKind, CoordinationScopeKey};
pub(crate) use self::domain_coordination::union_domain_extents;
pub(crate) use self::domain_coordination::{ChildFrameDomainRequest, aggregate_domain_requests};
pub(crate) use self::layout_profile::{
    FacetCellProfileIndex, FacetCellRenderedComponentsProfileCapture, LayoutProfileSnapshot,
};
use self::legends::PreparedLegendPlan;
pub(crate) use self::mark_data_runtime::{
    BaseDataRequest, GroupViewDataCacheHandle, GroupViewMarkContext, LogicalMarkDataRequest,
    MarkDataRequest, PreparedBaseData, PreparedMarkData, prepare_base_data,
    prepare_logical_mark_data, prepare_mark_data as prepare_mark_data_runtime,
    schedule_view_materializations_for_mark,
};
pub(crate) use self::materialization::MaterializationCacheHandle;
pub(crate) use self::native_runtime::NativeWidgetEvaluationRuntime;
pub use self::native_runtime::{
    InMemoryNativeWidgetInstanceStore, NativeWidgetAttachmentEpoch, NativeWidgetCtx,
    NativeWidgetDispatchOutcome, NativeWidgetDocumentId, NativeWidgetEnvironment,
    NativeWidgetEvaluationIntent, NativeWidgetEvent, NativeWidgetEventRoute, NativeWidgetFactory,
    NativeWidgetFactoryContext, NativeWidgetFocusRequest, NativeWidgetHostCommandSink,
    NativeWidgetHostServices, NativeWidgetHostTransform, NativeWidgetInstance,
    NativeWidgetInstanceKey, NativeWidgetInstanceSlot, NativeWidgetInstanceStore,
    NativeWidgetMeasurement, NativeWidgetNamespace, NativeWidgetPartTheme, NativeWidgetPlotId,
    NativeWidgetRegistry, NativeWidgetRuntimeResources, NativeWidgetScene,
    NativeWidgetSlotInitError, NativeWidgetSlotTypeMismatch, NativeWidgetStateSnapshot,
    ResolvedNativeWidgetSpec,
};
#[cfg(test)]
pub(crate) use self::session::TextMeasurementCache;
pub use self::session::{
    ChartSessionSnapshot, EvaluationRequest, PlotSession, PlotSessionOptions,
    ResolvedScopedParamAssignment, ResolvedScopedStoreAssignment, ResolvedSelectionAssignment,
    ResolvedStateTransaction, ScopedParamAssignment, ScopedParamStoreSnapshot,
    ScopedStoreAssignment, SelectionAssignment, SelectionStateUpdate, StateMigrationReport,
    StoreStateUpdate,
};
pub(crate) use self::session::{
    GuideOverflowCacheHandle, LegendMeasurementCacheHandle, ScaleDomainCacheHandle,
    ScopedParamStore, ScopedSelectionStore, ScopedStoreState, TextMeasurementCacheHandle,
    TextMeasurementCacheKey, WidgetItemCacheHandle,
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
    /// Group view scope: the spec is lowered onto each child mark at compile
    /// time; the view-local data context stored here is the group's shared
    /// view chain, executed once per evaluation.
    #[serde(default)]
    pub(crate) view: Option<CompiledViewScope>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(crate) struct MarkGroupDataCacheKey {
    pub(crate) plot_identity: usize,
    pub(crate) group_index: usize,
    pub(crate) facet_path: Vec<String>,
}

pub(crate) type MarkGroupDataCacheHandle =
    Arc<Mutex<HashMap<MarkGroupDataCacheKey, Arc<PreparedBaseData>>>>;

pub(crate) struct ResolvedScaleSet {
    pub(crate) scales: HashMap<String, ConfiguredScaleWithSpec>,
    pub(crate) _coordinate_domains: CoordinateDomainResolvedState,
}

fn is_data_transparent_group(group: &CompiledMarkGroupState) -> bool {
    !group.data.has_explicit_data_source()
        && group.data.transforms().is_empty()
        && group.data_mode == MarkDataMode::Inherit
        && group.facet_data_scope == FacetDataScope::FILTERED
        && group.view.is_none()
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

    /// Renderer/eventstream lookup derived from opaque mark identity. Numeric
    /// paths are an internal scene transport detail, never an authoring target.
    #[serde(default)]
    pub(crate) mark_runtime_paths: BTreeMap<avenger_chart_core::MarkId, Vec<Vec<usize>>>,

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

    /// Formatting defaults for scales, guides, and retained markup labels.
    #[serde(default)]
    pub(crate) formatting_context: FormattingContext,

    /// Mapping from scale names to coordinate channel
    pub(crate) scale_to_coord_channel: HashMap<String, String>,

    /// Scale specifications for building scales
    pub(crate) scale_specs: HashMap<String, ScaleSpec>,

    /// Scale specifications owned by composed widgets, keyed first by widget id.
    ///
    /// Keeping each widget in its own namespace prevents identically named
    /// visual channels (for example, three independent `fill` scales) from
    /// coordinating accidentally. Widget scales never produce plot guides.
    #[serde(default)]
    pub(crate) widget_scale_specs: HashMap<String, HashMap<String, ScaleSpec>>,

    // Note: We intentionally do not persist a ScaleBuilder here. Scales are
    // rebuilt per evaluation using current params to ensure correctness for
    // paramized data queries and to keep direct vs serialized paths identical.
    /// Plot-level data for mark data inheritance
    #[serde_as(as = "Option<FromInto<SerializableDataFrame>>")]
    pub(crate) data: Option<LogicalPlanNode>,

    /// Default parameter values for prepared statements
    #[serde_as(as = "FromInto<SerializableScalarMap>")]
    pub(crate) default_params: IndexMap<String, ScalarValue>,

    /// Parameter specs keyed canonically by opaque identity, with a separate
    /// source-name index for authoring and host boundaries.
    #[serde(default)]
    pub(crate) param_specs: CompiledStateRegistry<ParamRef, CompiledParamSpec>,

    /// Store specs keyed canonically by opaque identity, with a separate
    /// source-name index for authoring and host boundaries.
    #[serde(default)]
    pub(crate) store_specs: CompiledStateRegistry<StoreRef, CompiledStoreSpec>,

    /// Plot-level event bindings for chart apps.
    #[serde(default)]
    pub(crate) event_bindings: Vec<ChartEventBinding>,

    /// Root/shared parameter-change reactions for chart apps.
    #[serde(default)]
    pub(crate) param_change_bindings: Vec<ChartParamChangeBinding>,

    /// Datum columns requested by event bindings, with their compile-time types.
    #[serde(default)]
    pub(crate) event_datum_fields: Vec<EventDatumFieldSpec>,

    /// Coordinate readback columns exposed by event bindings, with their compile-time types.
    #[serde(default)]
    pub(crate) event_coord_fields: Vec<EventDatumFieldSpec>,

    /// Selection specs keyed canonically by opaque identity, with a separate
    /// source-name index for authoring and host boundaries.
    #[serde(default)]
    pub(crate) selection_specs: CompiledStateRegistry<SelectionRef, CompiledSelectionSpec>,

    /// Metadata for tools that expanded into this compiled plot.
    #[serde(default)]
    pub(crate) tool_metadata: Vec<ToolMetadata>,

    /// Canonical resolved tool identities and exports retained after expansion.
    #[serde(default)]
    pub(crate) tool_behaviors: Vec<CompiledToolBehavior>,

    /// Positionless compiled widgets paired with their host placement.
    #[serde(default)]
    pub(crate) widgets: Vec<avenger_chart_core::CompiledWidgetAttachment>,

    /// Baked in-memory tables registered before decoding baked residual plans.
    ///
    /// No `skip_serializing_if` here: `CompiledPlot` round-trips through
    /// bincode, which is not self-describing, so conditionally skipped fields
    /// break deserialization of plots that were never baked.
    #[serde(default)]
    pub(crate) baked_tables: Vec<BakedTableManifestEntry>,

    /// Report from the bake that produced this compiled plot.
    #[serde(default)]
    pub(crate) bake_report: Option<PlotBakeReport>,
}

impl Clone for CompiledPlot {
    fn clone(&self) -> Self {
        Self {
            coord_transform: self.coord_transform.clone(),
            compiled_guide: self.compiled_guide.clone(),
            marks: self.marks.clone(),
            mark_groups: self.mark_groups.clone(),
            mark_group_index_by_mark: self.mark_group_index_by_mark.clone(),
            mark_runtime_paths: self.mark_runtime_paths.clone(),
            axis_specs: self.axis_specs.clone(),
            legends: self.legends.clone(),
            legend_colorbar_overlays: self.legend_colorbar_overlays.clone(),
            layout_spec: self.layout_spec.clone(),
            title: self.title.clone(),
            subtitle: self.subtitle.clone(),
            theme: self.theme.clone(),
            time_context: self.time_context.clone(),
            formatting_context: self.formatting_context.clone(),
            scale_to_coord_channel: self.scale_to_coord_channel.clone(),
            scale_specs: self.scale_specs.clone(),
            widget_scale_specs: self.widget_scale_specs.clone(),
            data: self.data.clone(),
            default_params: self.default_params.clone(),
            param_specs: self.param_specs.clone(),
            store_specs: self.store_specs.clone(),
            event_bindings: self.event_bindings.clone(),
            param_change_bindings: self.param_change_bindings.clone(),
            event_datum_fields: self.event_datum_fields.clone(),
            event_coord_fields: self.event_coord_fields.clone(),
            selection_specs: self.selection_specs.clone(),
            tool_metadata: self.tool_metadata.clone(),
            tool_behaviors: self.tool_behaviors.clone(),
            widgets: self.widgets.clone(),
            baked_tables: self.baked_tables.clone(),
            bake_report: self.bake_report.clone(),
        }
    }
}

impl CompiledPlot {
    pub fn mark_runtime_path_index(
        &self,
    ) -> &BTreeMap<avenger_chart_core::MarkId, Vec<Vec<usize>>> {
        &self.mark_runtime_paths
    }

    pub fn runtime_paths_for_mark_ids(
        &self,
        ids: &[avenger_chart_core::MarkId],
    ) -> Result<Vec<Vec<usize>>, AvengerChartError> {
        let mut paths = Vec::new();
        for id in ids {
            let resolved = self.mark_runtime_paths.get(id).ok_or_else(|| {
                AvengerChartError::InvalidArgument(format!(
                    "compiled mark identity '{}' has no runtime scene path",
                    id
                ))
            })?;
            paths.extend(resolved.iter().cloned());
        }
        paths.sort();
        paths.dedup();
        Ok(paths)
    }

    pub fn widgets(&self) -> &[avenger_chart_core::CompiledWidgetAttachment] {
        &self.widgets
    }

    pub(crate) fn validate_widget_frame_assignments(
        &self,
        assignments: &crate::render::WidgetFrameAssignments,
    ) -> Result<(), AvengerChartError> {
        fn collect(
            plot: &CompiledPlot,
            explicit: &mut BTreeSet<String>,
            guide: &mut BTreeSet<String>,
        ) {
            for attachment in &plot.widgets {
                let id = attachment.widget.id().to_string();
                match attachment.placement {
                    avenger_chart_core::WidgetPlacement::ExplicitFrame => {
                        explicit.insert(id);
                    }
                    avenger_chart_core::WidgetPlacement::Guide(_) => {
                        guide.insert(id);
                    }
                }
            }
            for mark in &plot.marks {
                for payload in mark.child_plot_payloads() {
                    collect(
                        compiled_subplot_payload_child_plot(payload),
                        explicit,
                        guide,
                    );
                }
            }
        }

        let mut explicit = BTreeSet::new();
        let mut guide = BTreeSet::new();
        collect(self, &mut explicit, &mut guide);
        for (id, _) in assignments.iter() {
            if guide.contains(id) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Widget frame assignment for '{id}' targets a guide-positioned widget"
                )));
            }
            if !explicit.contains(id) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Widget frame assignment references unknown explicit widget '{id}'"
                )));
            }
        }
        let missing = explicit
            .iter()
            .filter(|id| assignments.get(id).is_none())
            .cloned()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Missing explicit widget frame assignments for: {}",
                missing.join(", ")
            )));
        }
        Ok(())
    }

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

    /// The nearest enclosing group view scope for a mark, if any.
    pub(crate) fn group_view_for_mark(
        &self,
        mark_index: usize,
    ) -> Option<GroupViewMarkContext<'_>> {
        let mut group_index = self.mark_group_index_for_mark(mark_index);
        while let Some(index) = group_index {
            let group = self.mark_groups.get(index)?;
            if let Some(view) = group.view.as_ref() {
                return Some(GroupViewMarkContext {
                    plot_identity: self as *const Self as usize,
                    group_index: index,
                    scope: view,
                });
            }
            group_index = group.parent_group_index;
        }
        None
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

    pub(crate) fn scale_names_for_coord_channel(&self, coord_channel: &str) -> Vec<String> {
        let mut names = BTreeSet::new();
        for (scale_name, mapped_coord_channel) in &self.scale_to_coord_channel {
            if mapped_coord_channel == coord_channel {
                names.insert(scale_name.clone());
            }
        }
        for mark in &self.marks {
            for (channel_name, channel_value) in mark.data_context().channels() {
                if !self.coord_transform.channel_uses_scale(channel_name) {
                    continue;
                }
                let base_channel = strip_trailing_numbers(channel_name);
                if base_channel != coord_channel {
                    continue;
                }
                if let Some(scale_name) = channel_value.get_scale_name(channel_name) {
                    names.insert(scale_name);
                }
            }
        }
        if names.is_empty() && self.scale_specs.contains_key(coord_channel) {
            names.insert(coord_channel.to_string());
        }
        names.into_iter().collect()
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

    /// Append CSS to this compiled chart's theme for subsequent evaluations.
    ///
    /// Widget styles and intrinsic measurements are resolved per evaluation,
    /// so this does not require recompiling data plans or widget expansions.
    pub fn append_css(&mut self, css: &str) -> Result<(), AvengerChartError> {
        let theme = self.theme.get_or_insert_with(|| Arc::new(Theme::light()));
        Arc::make_mut(theme)
            .append_css(css)
            .map_err(AvengerChartError::InvalidArgument)
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

    /// Parameter registry. Specs are keyed by opaque ID; `get(name)` is the
    /// explicit author/host name-resolution adapter.
    pub fn param_specs(&self) -> &CompiledStateRegistry<ParamRef, CompiledParamSpec> {
        &self.param_specs
    }

    #[doc(hidden)]
    pub fn param_specs_mut(&mut self) -> &mut CompiledStateRegistry<ParamRef, CompiledParamSpec> {
        &mut self.param_specs
    }

    pub fn param_specs_by_id(&self) -> &IndexMap<ParamRef, CompiledParamSpec> {
        self.param_specs.by_id()
    }

    /// Store registry. Specs are keyed by opaque ID; `get(name)` is the
    /// explicit author/host name-resolution adapter.
    pub fn store_specs(&self) -> &CompiledStateRegistry<StoreRef, CompiledStoreSpec> {
        &self.store_specs
    }

    #[doc(hidden)]
    pub fn store_specs_mut(&mut self) -> &mut CompiledStateRegistry<StoreRef, CompiledStoreSpec> {
        &mut self.store_specs
    }

    pub fn store_specs_by_id(&self) -> &IndexMap<StoreRef, CompiledStoreSpec> {
        self.store_specs.by_id()
    }

    /// Report from the bake that produced this plot, if any.
    pub fn bake_report(&self) -> Option<&PlotBakeReport> {
        self.bake_report.as_ref()
    }

    /// Get plot-level event bindings.
    pub fn event_bindings(&self) -> &[ChartEventBinding] {
        &self.event_bindings
    }

    /// Resolve a host-generated event binding against this compiled plot's
    /// opaque state identities.
    ///
    /// Authored bindings are resolved during chart compilation. Native hosts
    /// may add a narrow binding after compilation (for example canvas-resize
    /// parameter updates) and must cross the same source-name adapter before
    /// the app runtime consumes it.
    pub fn resolve_host_event_binding(
        &self,
        binding: ChartEventBinding,
    ) -> Result<ChartEventBinding, AvengerChartError> {
        let param_specs = self
            .param_specs
            .iter()
            .map(|(name, spec)| (name.clone(), spec.clone()))
            .collect();
        let store_specs = self
            .store_specs
            .iter()
            .map(|(name, spec)| (name.clone(), spec.clone()))
            .collect();
        let selection_specs = self
            .selection_specs
            .iter()
            .map(|(name, spec)| (name.clone(), spec.clone()))
            .collect();
        super::plot::resolve_event_binding_state_targets(
            binding,
            &param_specs,
            &store_specs,
            &selection_specs,
        )
    }

    /// Get root/shared parameter-change reactions.
    pub fn param_change_bindings(&self) -> &[ChartParamChangeBinding] {
        &self.param_change_bindings
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
        inherited_store_specs: Option<&CompiledStateRegistry<StoreRef, CompiledStoreSpec>>,
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
        store_specs: Option<&CompiledStateRegistry<StoreRef, CompiledStoreSpec>>,
    ) -> EvaluationContext {
        let mut eval_ctx = EvaluationContext::new(
            self.get_theme(),
            Arc::new(ctx.clone()),
            self.default_params.clone(),
            Arc::new(EvaluatedFacetTree::empty()),
        )
        .with_time_context(self.time_context.clone())
        .with_formatting_context(self.formatting_context.clone());
        let store_specs_by_id = store_specs
            .map(|specs| {
                specs
                    .values()
                    .cloned()
                    .map(|spec| (spec.runtime_id.clone(), spec))
                    .collect()
            })
            .unwrap_or_else(|| self.store_specs_by_id().clone());
        if !store_specs_by_id.is_empty() {
            eval_ctx = eval_ctx
                .with_scoped_store_state(Arc::new(ScopedStoreState::new(store_specs_by_id)));
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
        for attachment in &self.widgets {
            let avenger_chart_core::CompiledWidget::Composed(widget) = &attachment.widget else {
                continue;
            };
            for mark in &widget.marks {
                if let Some(details) = mark.state().details.as_ref() {
                    requested.extend(details.iter().cloned());
                }
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
        inherited_store_specs: Option<&CompiledStateRegistry<StoreRef, CompiledStoreSpec>>,
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
                collect_event_datum_types_from_schema(df, requested, out);
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
            if let Some(cell) = crate::widget_cell::compiled_widget_cell(mark.as_ref())
                && let avenger_chart_core::CompiledWidget::Composed(widget) = cell.widget()
            {
                self.collect_composed_widget_event_datum_types(
                    widget,
                    ctx,
                    requested,
                    out,
                    store_specs,
                    &eval_ctx,
                )
                .await?;
            }
        }

        for attachment in &self.widgets {
            let avenger_chart_core::CompiledWidget::Composed(widget) = &attachment.widget else {
                continue;
            };
            self.collect_composed_widget_event_datum_types(
                widget,
                ctx,
                requested,
                out,
                store_specs,
                &eval_ctx,
            )
            .await?;
        }
        Ok(())
    }

    async fn collect_composed_widget_event_datum_types(
        &self,
        widget: &avenger_chart_core::CompiledComposedWidget,
        ctx: &SessionContext,
        requested: &BTreeSet<String>,
        out: &mut IndexMap<String, DataType>,
        store_specs: &CompiledStateRegistry<StoreRef, CompiledStoreSpec>,
        eval_ctx: &crate::render::EvaluationContext,
    ) -> Result<(), AvengerChartError> {
        let item_df = widget
            .items
            .as_ref()
            .and_then(|items| items.data.dataframe_with_context(ctx));
        if let Some(df) = item_df.as_ref() {
            collect_event_datum_types_from_schema(df, requested, out);
        }
        let prepared_base = item_df.clone().map(|dataframe| PreparedBaseData {
            dataframe: Some(dataframe),
            derived_scalars: Default::default(),
            facet_data_scope: FacetDataScope::FILTERED,
        });
        for mark in &widget.marks {
            collect_event_datum_types_from_specs(mark.event_datum_field_specs(), requested, out)?;
            if let Some(store_data) = mark.data_context().store_data()
                && let Some(spec) = store_specs.get(&store_data.store_name)
            {
                collect_event_datum_types_from_store_spec(spec, requested, out);
            }
            let prepared = Box::pin(prepare_logical_mark_data(LogicalMarkDataRequest {
                mark: mark.as_ref(),
                plot_data: None,
                provided_plot_df: None,
                facet_data_scope: None,
                prepared_base: prepared_base.as_ref(),
                eval_ctx,
            }))
            .await?;
            if let Some(df) = prepared.dataframe.as_ref() {
                collect_event_datum_types_from_schema(df, requested, out);
            }
        }
        Ok(())
    }

    pub fn selection_specs(&self) -> &CompiledStateRegistry<SelectionRef, CompiledSelectionSpec> {
        &self.selection_specs
    }

    #[doc(hidden)]
    pub fn selection_specs_mut(
        &mut self,
    ) -> &mut CompiledStateRegistry<SelectionRef, CompiledSelectionSpec> {
        &mut self.selection_specs
    }

    pub fn selection_specs_by_id(&self) -> &IndexMap<SelectionRef, CompiledSelectionSpec> {
        self.selection_specs.by_id()
    }

    pub fn tool_metadata(&self) -> &[ToolMetadata] {
        &self.tool_metadata
    }

    pub fn tool_behaviors(&self) -> &[CompiledToolBehavior] {
        &self.tool_behaviors
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
        self.build_scales_from_builder_with_coordinate_domain_policy(
            builder,
            plot_area_width,
            plot_area_height,
            ctx,
            params,
            CoordinateDomainBuildPolicy::RejectSharedOverrides,
        )
        .await
    }

    pub(crate) async fn build_scales_from_builder_without_coordinate_domains(
        &self,
        builder: &ScaleBuilder,
        plot_area_width: f32,
        plot_area_height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<HashMap<String, ConfiguredScaleWithSpec>, AvengerChartError> {
        self.build_scales_from_builder_inner(
            builder,
            plot_area_width,
            plot_area_height,
            ctx,
            params,
            None,
        )
        .await
        .map(|resolved| resolved.scales)
    }

    pub(crate) async fn build_scales_from_builder_with_coordinate_domain_policy(
        &self,
        builder: &ScaleBuilder,
        plot_area_width: f32,
        plot_area_height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
        build_policy: CoordinateDomainBuildPolicy,
    ) -> Result<HashMap<String, ConfiguredScaleWithSpec>, AvengerChartError> {
        self.build_scales_from_builder_inner(
            builder,
            plot_area_width,
            plot_area_height,
            ctx,
            params,
            Some(build_policy),
        )
        .await
        .map(|resolved| resolved.scales)
    }

    async fn build_scales_from_builder_inner(
        &self,
        builder: &ScaleBuilder,
        plot_area_width: f32,
        plot_area_height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
        build_policy: Option<CoordinateDomainBuildPolicy>,
    ) -> Result<ResolvedScaleSet, AvengerChartError> {
        let coordinate_domain_descriptors = if build_policy.is_some() {
            self.coordinate_domain_descriptors()
        } else {
            Default::default()
        };
        let materialized_builder =
            self.materialize_coordinate_domain_builder(builder, &coordinate_domain_descriptors)?;
        let builder = materialized_builder.as_ref().unwrap_or(builder);

        // Build coordinate system range bindings map
        let mut coord_system_range_bindings = HashMap::<String, ScaleRangeBinding>::new();
        for channel in builder.channel_builders().keys() {
            let base = self
                .coordinate_domain_channel_for_scale(&coordinate_domain_descriptors, channel)
                .or_else(|| self.scale_to_coord_channel.get(channel).map(String::as_str))
                .unwrap_or_else(|| strip_trailing_numbers(channel));
            if let Some(binding) = self.coord_transform.default_range_binding(base) {
                coord_system_range_bindings.insert(channel.clone(), binding);
            }
        }

        let theme = self.get_theme();
        let default_range_resolver = scales::default_range_for_compiled_marks(&self.marks);
        let mut built = Box::pin(builder.build_scales(
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

        let coordinate_domains = self.apply_coordinate_domain_provider(
            builder,
            &mut built,
            &coordinate_domain_descriptors,
            plot_area_width,
            plot_area_height,
            params,
            build_policy.unwrap_or(CoordinateDomainBuildPolicy::AllowSharedOverrides),
        )?;

        Ok(ResolvedScaleSet {
            scales: built,
            _coordinate_domains: coordinate_domains,
        })
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
        self.build_scales_for_dataframe_with_coordinate_domain_policy(
            df,
            plot_area_width,
            plot_area_height,
            ctx,
            params,
            CoordinateDomainBuildPolicy::RejectSharedOverrides,
        )
        .await
    }

    pub(crate) async fn build_scales_for_dataframe_with_coordinate_domain_policy(
        &self,
        df: &DataFrame,
        plot_area_width: f32,
        plot_area_height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
        build_policy: CoordinateDomainBuildPolicy,
    ) -> Result<HashMap<String, ConfiguredScaleWithSpec>, AvengerChartError> {
        let eval_ctx =
            CoreEvaluationContext::new(self.get_theme(), Arc::new(ctx.clone()), params.clone())
                .with_time_context(self.time_context.clone())
                .with_formatting_context(self.formatting_context.clone());
        let scale_builder = Box::pin(scales::build_scale_builder_from_compiled_plot(
            self,
            Some(df.clone()),
            &eval_ctx,
            self.get_theme().as_ref(),
        ))
        .await?;

        Box::pin(
            self.build_scales_from_builder_with_coordinate_domain_policy(
                &scale_builder,
                plot_area_width,
                plot_area_height,
                ctx,
                params,
                build_policy,
            ),
        )
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

    /// Resolved composed-widget styles and intrinsic sizes from this same
    /// measurement pass.
    pub(crate) widget_measurements: IndexMap<String, WidgetMeasurement>,
}

#[derive(Clone)]
pub(crate) struct WidgetMeasurement {
    pub(crate) position: Option<avenger_chart_core::LegendPosition>,
    pub(crate) declaration_order: u64,
    pub(crate) width: avenger_chart_core::ResolvedWidgetAxisSize,
    pub(crate) height: avenger_chart_core::ResolvedWidgetAxisSize,
    pub(crate) styles: Arc<avenger_chart_core::ResolvedWidgetStyleSet>,
    pub(crate) presentation: avenger_chart_core::WidgetPresentationState,
    pub(crate) prepared_items: Option<WidgetPreparedBaseData>,
    pub(crate) scales: HashMap<String, ConfiguredScaleWithSpec>,
}

#[derive(Clone)]
pub(crate) struct WidgetPreparedBaseData {
    pub(crate) base: PreparedBaseData,
    pub(crate) item_count: usize,
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
            .field(
                "widget_measurements",
                &format!("{} widgets", self.widget_measurements.len()),
            )
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

    /// Compiler-owned widget groups, one per composed attachment.
    pub widget_marks: Vec<avenger_scenegraph::marks::mark::SceneMark>,

    /// Typed, evaluation-local inputs associated with each composed widget.
    pub widget_runtime_inputs: IndexMap<String, IndexMap<String, datafusion::common::ScalarValue>>,

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
