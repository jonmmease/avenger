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
    sync::Arc,
};

use datafusion::{
    arrow::datatypes::DataType, common::ScalarValue, dataframe::DataFrame, prelude::SessionContext,
};
use datafusion_proto::protobuf::LogicalPlanNode;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use avenger_chart_core::{
    AvengerChartError, AxisSpec, CompiledGuide, CompiledMark, CompiledParamSpec,
    CompiledSelectionSpec, CompiledStoreSpec, CompiledSubplotChildPlot, CompiledSubplotPayload,
    CoordMeasurement, CoordinateSystemTransform, EvaluationContext as CoreEvaluationContext,
    Legend, LogicalPlanNodeExt, ScaleRangeBinding, SerializableDataFrame, SerializableDataType,
    SerializableScalarMap, Theme, TimeContext, ToolMetadata, channel::strip_trailing_numbers,
};
use avenger_chart_scales::{ConfiguredScaleWithSpec, PlotScaleSpec as ScaleSpec, ScaleBuilder};

use crate::{
    event::ChartEventBinding,
    layout::{
        ChartResizePolicy, ChildFrameContentMeasurement, ChildFrameContentSolver,
        ContentAllocation, ContentLayout, ContentLayoutSolver, FrameAllocation, FrameDemand,
        LayoutSpec, SinglePlotContentMeasurement, SinglePlotContentSolver,
    },
};

pub use self::child_frame_container::ChildFrameContainerView;
pub(crate) use self::child_frame_container::{
    child_frame_container_overflow, child_frame_container_view_from_concat,
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
pub(crate) use self::domain_coordination::{ChildFrameDomainRequest, aggregate_domain_requests};
pub(crate) use self::layout_profile::{
    FacetCellProfileIndex, FacetCellRenderedComponentsProfileCapture, LayoutProfileSnapshot,
};
use self::legends::PreparedLegendPlan;
pub(crate) use self::mark_data_runtime::{
    LogicalMarkDataRequest, MarkDataRequest, PreparedMarkData, prepare_logical_mark_data,
    prepare_mark_data as prepare_mark_data_runtime,
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

#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventDatumFieldSpec {
    pub name: String,
    #[serde_as(as = "FromInto<SerializableDataType>")]
    pub data_type: DataType,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct CompiledColorbarOverlayMarks {
    pub(crate) channel_name: String,
    pub(crate) marks: Vec<Arc<dyn CompiledMark>>,
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

    pub(crate) fn infer_event_datum_fields(
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
        if requested.is_empty() {
            return Ok(Vec::new());
        }

        let mut types = IndexMap::new();
        collect_reserved_event_datum_types(&requested, &mut types);
        self.collect_event_datum_types(ctx, &requested, &mut types)?;
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

    fn collect_event_datum_types(
        &self,
        ctx: &SessionContext,
        requested: &BTreeSet<String>,
        out: &mut IndexMap<String, DataType>,
    ) -> Result<(), AvengerChartError> {
        if let Some(df) = self.data.as_ref().and_then(|node| {
            node.to_logical_plan(ctx)
                .ok()
                .map(|plan| DataFrame::new(ctx.state().clone(), plan))
        }) {
            collect_event_datum_types_from_schema(&df, requested, out);
        }

        for mark in &self.marks {
            if let Some(df) = mark.data_context().dataframe_with_context(ctx) {
                collect_event_datum_types_from_schema(&df, requested, out);
            }
            if let Some(subplot) = crate::concat::compiled_subplot(mark.as_ref()) {
                subplot
                    .compiled_subplot()
                    .collect_event_datum_types(ctx, requested, out)?;
            }
            if let Some(subplot) = crate::facet::marks::facet::facet_subplot_ref(mark.as_ref()) {
                subplot
                    .compiled_subplot()
                    .collect_event_datum_types(ctx, requested, out)?;
            }
            if let Some(subplot) = mark.as_positioned_subplot() {
                compiled_subplot_payload_child_plot(subplot.payload())
                    .collect_event_datum_types(ctx, requested, out)?;
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
        let scale_builder = Box::pin(scales::build_scale_builder_from_marks(
            &self.marks,
            &self.scale_specs,
            &self.coord_transform,
            &self.data,
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
