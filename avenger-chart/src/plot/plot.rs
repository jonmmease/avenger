//! Plot builder for creating visualizations

use std::{
    any::Any,
    collections::{HashMap, HashSet},
    sync::Arc,
};

use datafusion::{common::ScalarValue, dataframe::DataFrame, prelude::lit};
use datafusion_proto::protobuf::LogicalPlanNode;
use indexmap::IndexMap;

use avenger_chart_core::{
    AvengerChartError, Axis, AxisGuideVisibilityPolicy, AxisSpec, ChartTool, CompileContext,
    CompiledDataContext, CompiledMark, CompiledMarkState, CompiledParamSpec, CompiledSelectionSpec,
    CompiledSubplotChildPlot, CoordinateGuide, CoordinateSystem, CoordinateSystemTransformCore,
    CoordinationScope, DataContext, DomainCoordination, DomainCoordinationGroup, FormattingContext,
    IntoExpr, IntoPlotMark, Legend, LegendSurfaceKind, Mark, MarkDataMode, MarkState, Param,
    PlotMark, PlotMarkKind, RepeatContext, RepeatDomainCoordination, RepeatVariable,
    ScaleInferenceHint, SceneGeometryTarget, Selection, SelectionSceneQuery, SelectionUpdate,
    Store, SubplotChildPlotSpec, Theme, TimeContext, compile_selections, validate_structural_id,
};
use avenger_chart_marks::Subplot;
use avenger_chart_scales::{PlotScaleSpec as ScaleSpec, serialization::LogicalPlanNodeExt};

use crate::{
    concat::{ConcatOrigin, GridConcat, HConcat, VConcat, WrapConcat},
    event::{ChartEventBinding, ChartEventStream, rewrite_reserved_event_binding_local_datums},
    layout::{CanvasConstraint, LayoutSpec, Margins, PlotConstraint, SizeMode},
    legend::ColorbarOverlay,
    repeat::{RepeatColumns, RepeatGrid, RepeatResolvedChildPlotSpec, RepeatRows, RepeatWrap},
    serialization::serializable_expr_from_expr,
    tools::{ToolCompileContext, discover_tool_scale_targets},
};

use super::{
    compiled::{CompiledColorbarOverlayMarks, CompiledPlot},
    title::{PlotSubtitle, PlotTitle},
};

#[derive(Clone)]
pub struct Plot<C: CoordinateSystem> {
    coord_system: C,

    /// Recursive plot elements stored until compilation.
    marks: Vec<PlotMark<C>>,

    /// Plot-level data for mark inheritance
    pub(crate) data: Option<DataFrame>,

    /// Plot-level scale configurations (set via .scale())
    pub(crate) scale_specs: HashMap<String, ScaleSpec>,

    /// Plot-level legend configurations (set via .legend())
    pub(crate) legends: IndexMap<String, Legend>,

    /// Layout specification for sizing and margins
    pub(crate) layout_spec: LayoutSpec,

    /// Optional plot title rendered by the layout system
    pub(crate) title: Option<PlotTitle>,

    /// Optional plot subtitle rendered by the layout system
    pub(crate) subtitle: Option<PlotSubtitle>,

    /// Theme for visual styling
    pub(crate) theme: Option<Arc<Theme>>,

    /// Time handling defaults for temporal transforms, scales, and guides.
    pub(crate) time_context: TimeContext,

    /// Formatting defaults for scales, guides, and retained markup labels.
    pub(crate) formatting_context: FormattingContext,

    /// Guide configuration
    pub(crate) guide_config: Option<C::Guide>,

    /// Parameters that can be used in expressions, with their sharing scope.
    pub(crate) param_specs: Vec<CompiledParamSpec>,

    /// Plot-level event bindings that patch params in chart apps
    pub(crate) event_bindings: Vec<ChartEventBinding>,

    /// Plot-level selections that event bindings can update and marks can read.
    pub(crate) selections: Vec<Selection>,

    /// Plot-level stores that event bindings can mutate and marks can read.
    pub(crate) stores: Vec<Store>,

    /// Param names whose values drive app cursor state instead of chart visuals.
    pub(crate) cursor_params: Vec<String>,

    /// Authoring-time tools that expand during compilation.
    pub(crate) tools: Vec<Arc<dyn ChartTool<C>>>,
}

#[async_trait::async_trait]
impl<C> SubplotChildPlotSpec for Plot<C>
where
    C: CoordinateSystem + Clone + 'static,
{
    fn clone_box(&self) -> Box<dyn SubplotChildPlotSpec> {
        Box::new(self.clone())
    }

    fn has_plot_level_data(&self) -> bool {
        self.data.is_some()
    }

    async fn compile_boxed(
        &self,
        session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledSubplotChildPlot>, AvengerChartError> {
        Ok(Arc::new(self.clone().compile(session_context).await?))
    }

    async fn compile_boxed_with_context(
        &self,
        session_context: &datafusion::prelude::SessionContext,
        compile_context: Option<CompileContext<'_>>,
    ) -> Result<Arc<dyn CompiledSubplotChildPlot>, AvengerChartError> {
        let tool_context = compile_context.and_then(ToolCompileContext::downcast);
        Ok(Arc::new(
            self.clone()
                .compile_with_tool_context(session_context, tool_context, false)
                .await?,
        ))
    }
}

impl<C: CoordinateSystem> Plot<C> {
    pub fn with_coord(coord_system: C) -> Self {
        Plot {
            coord_system,
            marks: Vec::new(),
            data: None,
            scale_specs: HashMap::new(),
            legends: IndexMap::new(),
            layout_spec: LayoutSpec::default(),
            title: None,
            subtitle: None,
            theme: None,
            time_context: TimeContext::default(),
            formatting_context: FormattingContext::default(),
            guide_config: None,
            param_specs: Vec::new(),
            event_bindings: Vec::new(),
            selections: Vec::new(),
            stores: Vec::new(),
            cursor_params: Vec::new(),
            tools: Vec::new(),
        }
    }
}

impl Plot<crate::concat::GridConcat> {
    pub fn rows(mut self, rows: usize) -> Self {
        self.coord_system = self.coord_system.clone().rows(rows);
        self
    }

    pub fn columns(mut self, columns: usize) -> Self {
        self.coord_system = self.coord_system.clone().columns(columns);
        self
    }

    /// Per-column plot-area sizing. See [`crate::concat::TrackSizing`].
    pub fn column_widths(
        mut self,
        widths: impl IntoIterator<Item = crate::concat::TrackSizing>,
    ) -> Self {
        self.coord_system = self.coord_system.clone().column_widths(widths);
        self
    }

    /// Per-row plot-area sizing. See [`crate::concat::TrackSizing`].
    pub fn row_heights(
        mut self,
        heights: impl IntoIterator<Item = crate::concat::TrackSizing>,
    ) -> Self {
        self.coord_system = self.coord_system.clone().row_heights(heights);
        self
    }

    pub fn axis_guide_visibility(mut self, policy: AxisGuideVisibilityPolicy) -> Self {
        self.coord_system = self.coord_system.clone().axis_guide_visibility(policy);
        self
    }
}

impl Plot<crate::concat::HConcat> {
    /// Per-column plot-area sizing, one entry per child. See
    /// [`crate::concat::TrackSizing`].
    pub fn widths(mut self, widths: impl IntoIterator<Item = crate::concat::TrackSizing>) -> Self {
        self.coord_system = self.coord_system.clone().widths(widths);
        self
    }
}

impl Plot<crate::concat::VConcat> {
    /// Per-row plot-area sizing, one entry per child. See
    /// [`crate::concat::TrackSizing`].
    pub fn heights(
        mut self,
        heights: impl IntoIterator<Item = crate::concat::TrackSizing>,
    ) -> Self {
        self.coord_system = self.coord_system.clone().heights(heights);
        self
    }
}

impl Plot<crate::concat::WrapConcat> {
    pub fn columns(mut self, expr: impl IntoExpr) -> Self {
        self.coord_system = self.coord_system.clone().columns(expr);
        self
    }

    pub fn responsive_columns(mut self, width: impl IntoExpr) -> Self {
        self.coord_system = self.coord_system.clone().responsive_columns(width);
        self
    }

    pub fn axis_guide_visibility(mut self, policy: AxisGuideVisibilityPolicy) -> Self {
        self.coord_system = self.coord_system.clone().axis_guide_visibility(policy);
        self
    }
}

impl Plot<RepeatColumns> {
    pub fn columns(mut self, columns: impl IntoIterator<Item = RepeatVariable>) -> Self {
        self.coord_system.set_columns(columns.into_iter().collect());
        self
    }

    pub fn cell<P>(mut self, cell: P) -> Self
    where
        P: SubplotChildPlotSpec + 'static,
    {
        self.coord_system.set_cell(Box::new(cell));
        self
    }

    pub fn cell_when<P>(mut self, predicate: impl IntoExpr, cell: P) -> Self
    where
        P: SubplotChildPlotSpec + 'static,
    {
        self.coord_system.add_cell_when(predicate, Box::new(cell));
        self
    }

    pub fn with_repeat_domain_coordination(mut self, mode: RepeatDomainCoordination) -> Self {
        self.coord_system.set_domain_coordination(mode);
        self
    }
}

impl Plot<RepeatRows> {
    pub fn rows(mut self, rows: impl IntoIterator<Item = RepeatVariable>) -> Self {
        self.coord_system.set_rows(rows.into_iter().collect());
        self
    }

    pub fn cell<P>(mut self, cell: P) -> Self
    where
        P: SubplotChildPlotSpec + 'static,
    {
        self.coord_system.set_cell(Box::new(cell));
        self
    }

    pub fn cell_when<P>(mut self, predicate: impl IntoExpr, cell: P) -> Self
    where
        P: SubplotChildPlotSpec + 'static,
    {
        self.coord_system.add_cell_when(predicate, Box::new(cell));
        self
    }

    pub fn with_repeat_domain_coordination(mut self, mode: RepeatDomainCoordination) -> Self {
        self.coord_system.set_domain_coordination(mode);
        self
    }
}

impl Plot<RepeatGrid> {
    pub fn rows(mut self, rows: impl IntoIterator<Item = RepeatVariable>) -> Self {
        self.coord_system.set_rows(rows.into_iter().collect());
        self
    }

    pub fn columns(mut self, columns: impl IntoIterator<Item = RepeatVariable>) -> Self {
        self.coord_system.set_columns(columns.into_iter().collect());
        self
    }

    pub fn cell<P>(mut self, cell: P) -> Self
    where
        P: SubplotChildPlotSpec + 'static,
    {
        self.coord_system.set_cell(Box::new(cell));
        self
    }

    pub fn cell_when<P>(mut self, predicate: impl IntoExpr, cell: P) -> Self
    where
        P: SubplotChildPlotSpec + 'static,
    {
        self.coord_system.add_cell_when(predicate, Box::new(cell));
        self
    }

    pub fn matrix_domains(mut self) -> Self {
        self.coord_system.matrix_domains(CoordinationScope::Shared);
        self
    }

    pub fn matrix_domains_with_scope(mut self, scope: CoordinationScope) -> Self {
        self.coord_system.matrix_domains(scope);
        self
    }

    pub fn axis_guide_visibility(mut self, policy: AxisGuideVisibilityPolicy) -> Self {
        self.coord_system.axis_guide_visibility(policy);
        self
    }

    pub fn matrix_axes(mut self) -> Self {
        self.coord_system.matrix_axes();
        self
    }

    pub fn with_repeat_domain_coordination(mut self, mode: RepeatDomainCoordination) -> Self {
        self.coord_system.set_domain_coordination(mode);
        self
    }
}

impl Plot<RepeatWrap> {
    pub fn items(mut self, items: impl IntoIterator<Item = RepeatVariable>) -> Self {
        self.coord_system.set_items(items.into_iter().collect());
        self
    }

    pub fn columns(mut self, expr: impl IntoExpr) -> Self {
        self.coord_system.set_columns(expr);
        self
    }

    pub fn responsive_columns(mut self, width: impl IntoExpr) -> Self {
        self.coord_system.set_responsive_columns(width);
        self
    }

    pub fn cell<P>(mut self, cell: P) -> Self
    where
        P: SubplotChildPlotSpec + 'static,
    {
        self.coord_system.set_cell(Box::new(cell));
        self
    }

    pub fn cell_when<P>(mut self, predicate: impl IntoExpr, cell: P) -> Self
    where
        P: SubplotChildPlotSpec + 'static,
    {
        self.coord_system.add_cell_when(predicate, Box::new(cell));
        self
    }

    pub fn item_domains(mut self) -> Self {
        self.coord_system.item_domains(CoordinationScope::Shared);
        self
    }

    pub fn item_domains_with_scope(mut self, scope: CoordinationScope) -> Self {
        self.coord_system.item_domains(scope);
        self
    }

    pub fn with_repeat_domain_coordination(mut self, mode: RepeatDomainCoordination) -> Self {
        self.coord_system.set_domain_coordination(mode);
        self
    }
}

impl<C: CoordinateSystem + Default> Default for Plot<C> {
    fn default() -> Self {
        Self::with_coord(C::default())
    }
}

impl<C: CoordinateSystem + Default> Plot<C> {
    pub fn new() -> Self {
        Self::default()
    }
}

impl<C: CoordinateSystem> Plot<C> {
    /// Compile this plot into a renderable form (consuming self)
    pub async fn compile(
        self,
        session_context: &datafusion::prelude::SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let root_tool_context = ToolCompileContext::root(self.time_context.clone());
        Box::pin(self.compile_with_tool_context(session_context, Some(&root_tool_context), true))
            .await
    }

    pub(crate) async fn compile_with_tool_context(
        self,
        session_context: &datafusion::prelude::SessionContext,
        inherited_tool_context: Option<&ToolCompileContext>,
        is_root: bool,
    ) -> Result<CompiledPlot, AvengerChartError> {
        match try_lower_repeat_plot(self, session_context)? {
            MaybeLoweredRepeatPlot::Lowered(lowered) => {
                return Box::pin(lowered.compile_with_tool_context(
                    session_context,
                    inherited_tool_context,
                    is_root,
                ))
                .await;
            }
            MaybeLoweredRepeatPlot::Original(plot) => {
                return Box::pin(plot.compile_without_repeat_lowering(
                    session_context,
                    inherited_tool_context,
                    is_root,
                ))
                .await;
            }
        }
    }

    async fn compile_without_repeat_lowering(
        self,
        session_context: &datafusion::prelude::SessionContext,
        inherited_tool_context: Option<&ToolCompileContext>,
        is_root: bool,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let inherited_time_context = inherited_tool_context
            .map(|context| context.time_context())
            .cloned()
            .unwrap_or_default();
        let effective_time_context = self
            .time_context
            .resolved_with_parent(&inherited_time_context);
        let inherited_formatting_context = inherited_tool_context
            .map(|context| context.formatting_context())
            .cloned()
            .unwrap_or_default();
        let effective_formatting_context = self
            .formatting_context
            .resolved_with_parent(&inherited_formatting_context);
        let tool_context = ToolCompileContext::from_parent(inherited_tool_context)
            .with_time_context(effective_time_context.clone())
            .with_formatting_context(effective_formatting_context.clone());
        let coord_system = if let Some(repeat_context) = tool_context.repeat_context() {
            self.coord_system.resolve_repeat(repeat_context)?
        } else {
            self.coord_system.clone()
        };
        let mut pre_tool_axis_specs: HashMap<String, AxisSpec> = HashMap::new();
        let mut pre_tool_legends: IndexMap<String, Legend> = self.legends.clone();
        let mut pre_tool_scale_specs: HashMap<String, ScaleSpec> = self.scale_specs.clone();
        let mut pre_tool_scale_to_coord_channel: HashMap<String, String> = HashMap::new();
        let pre_tool_flat_marks = flatten_plot_marks(&self.marks, tool_context.repeat_context())?;
        let pre_tool_mark_states =
            resolve_mark_states(&pre_tool_flat_marks.marks, tool_context.repeat_context())?;
        let pre_tool_coord_system = coord_system.resolve_from_mark_states(&pre_tool_mark_states)?;
        pre_tool_coord_system.validate()?;
        let pre_tool_coord_transform = pre_tool_coord_system.create_transform();
        crate::plot::channel::extract_axis_configs_from_coordinate(
            &pre_tool_coord_system,
            &mut pre_tool_axis_specs,
        );
        for mark_state in &pre_tool_mark_states {
            crate::plot::channel::extract_channel_configs_from_state(
                mark_state,
                session_context,
                pre_tool_coord_transform.as_ref(),
                &mut pre_tool_axis_specs,
                &mut pre_tool_legends,
                &mut pre_tool_scale_specs,
                &mut pre_tool_scale_to_coord_channel,
            )?;
        }
        crate::plot::channel::extract_axis_configs_from_coordinate(
            &pre_tool_coord_system,
            &mut pre_tool_axis_specs,
        );
        let pre_tool_scale_coordination = scale_domain_coordinations_from_states(
            &pre_tool_mark_states,
            pre_tool_coord_transform.as_ref(),
        )?;
        let tool_scale_targets = discover_tool_scale_targets(
            pre_tool_coord_transform.as_ref(),
            &pre_tool_scale_to_coord_channel,
            &pre_tool_scale_coordination,
        )?;
        let tool_coordinate_metrics = pre_tool_coord_transform
            .domain_provider()
            .map(|provider| {
                provider
                    .domain_descriptors()
                    .into_iter()
                    .flat_map(|descriptor| descriptor.metrics)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let active_tool_expansions = tool_context.expand_local_tools(
            &self.tools,
            &tool_scale_targets,
            &tool_coordinate_metrics,
        )?;
        tool_context.register_local_stores(&self.stores)?;
        if !is_root {
            tool_context.register_local_event_bindings(&self.event_bindings)?;
        }
        let erased_tool_context: CompileContext<'_> = &tool_context;

        let mut selection_specs: IndexMap<String, CompiledSelectionSpec> =
            compile_selections(&self.selections)?;

        // Start with plot-level configurations
        let mut axis_specs: HashMap<String, AxisSpec> = HashMap::new();
        let mut legends: IndexMap<String, Legend> = self.legends.clone();
        let mut scale_specs: HashMap<String, ScaleSpec> = self.scale_specs.clone();
        let mut scale_to_coord_channel: HashMap<String, String> = HashMap::new();

        let mut marks = self.marks;
        for active in &active_tool_expansions {
            marks.extend(
                active
                    .expansion
                    .marks
                    .iter()
                    .cloned()
                    .map(PlotMark::from_mark_arc),
            );
        }
        let flat_marks = flatten_plot_marks(&marks, tool_context.repeat_context())?;
        let resolved_mark_states =
            resolve_mark_states(&flat_marks.marks, tool_context.repeat_context())?;
        let coord_system = coord_system.resolve_from_mark_states(&resolved_mark_states)?;
        coord_system.validate()?;
        let coord_transform = coord_system.create_transform();

        // 1. Extract and merge channel configs from all marks with proper SessionContext
        crate::plot::channel::extract_axis_configs_from_coordinate(&coord_system, &mut axis_specs);
        for mark_state in &resolved_mark_states {
            crate::plot::channel::extract_channel_configs_from_state(
                mark_state,
                session_context,
                coord_transform.as_ref(),
                &mut axis_specs,
                &mut legends,
                &mut scale_specs,
                &mut scale_to_coord_channel,
            )?;
        }
        crate::plot::channel::extract_axis_configs_from_coordinate(&coord_system, &mut axis_specs);

        let scale_coordination = scale_domain_coordinations_from_states(
            &resolved_mark_states,
            coord_transform.as_ref(),
        )?;
        tool_context.apply_scale_edits(
            &active_tool_expansions,
            coord_transform.as_ref(),
            &scale_to_coord_channel,
            &mut scale_specs,
            &scale_coordination,
        )?;

        // 2. Compile all marks. Aggregate channels are intentionally prepared at
        // runtime so faceted marks aggregate after mark-level data scope has been
        // resolved.
        let mut compiled_marks: Vec<Arc<dyn CompiledMark>> = Vec::new();
        for (mark_index, (m, mark_state)) in flat_marks
            .marks
            .iter()
            .zip(resolved_mark_states.iter())
            .enumerate()
        {
            // Get the DataFrame (or use plot-level data)
            let df_opt = if mark_state.data_mode == MarkDataMode::Unit {
                None
            } else if flat_marks.mark_group_indices[mark_index].is_some() {
                mark_state.data.dataframe().cloned()
            } else {
                mark_state
                    .data
                    .dataframe()
                    .cloned()
                    .or_else(|| self.data.clone())
            };

            let compiled_state = CompiledMarkState::from_mark_state(mark_state, df_opt)
                .with_mark_index(mark_index)
                .with_public_target_path(flat_marks.public_target_paths[mark_index].clone());
            let compiled_mark = m
                .compile_with_context(compiled_state, session_context, Some(erased_tool_context))
                .await?;
            compiled_marks.push(compiled_mark);
        }
        let mark_groups = compile_mark_group_states(&flat_marks.group_states);
        crate::plot::channel::extract_channel_configs_from_compiled_domain_channels(
            &compiled_marks,
            coord_transform.as_ref(),
            &mut axis_specs,
            &mut legends,
            &mut scale_specs,
            &mut scale_to_coord_channel,
        )?;

        // 3. Build guide renderer - either from config or default
        let mut guide = if let Some(config) = &self.guide_config {
            config.clone()
        } else {
            // Create default guide for the coordinate system
            C::Guide::default()
        };

        // We need to populate axes from axis_specs before building
        // Create axes map for the guide
        let mut guide_axes = HashMap::new();

        // Add user-specified axes from axis_specs
        for (channel, axis_spec) in &axis_specs {
            let AxisSpec::Local(axis_config) = axis_spec;
            // The axis_config is already the correct type for this coordinate system
            // We need to downcast it to the specific axis type for the guide
            // This is safe because the axis type matches the coordinate system
            if let Some(typed_axis) = axis_config
                .as_any()
                .downcast_ref::<<C::Guide as CoordinateGuide>::Axis>()
            {
                guide_axes.insert(channel.clone(), typed_axis.clone());
            }
        }

        // Set the axes on the guide
        guide.set_axes(guide_axes);

        // Pass compiled mark metadata to the guide so it can extract titles
        // without owning the render-capable compiled mark collection.
        guide.set_compiled_marks(&compiled_marks, session_context);

        let compiled_guide = Arc::from(guide.build());

        // 4. Prepare serialized logical plan for plot-level data (for rebuilds)
        let data_plan_node = match &self.data {
            Some(df) => {
                let plan = df.logical_plan().clone();
                Some(LogicalPlanNode::from_logical_plan(&plan).map_err(|e| {
                    AvengerChartError::InternalError(format!(
                        "Failed to serialize logical plan: {}",
                        e
                    ))
                })?)
            }
            None => None,
        };

        // Build param specs (preserving declaration order) and reject duplicate
        // names regardless of whether they came from add_param or
        // add_param_with_sharing.
        let mut param_source_specs = self.param_specs.clone();
        let mut store_source_specs = Vec::new();
        let legend_colorbar_overlays = compile_colorbar_overlays(&legends, session_context).await?;
        for legend in legends.values_mut() {
            legend.colorbar_overlays.clear();
        }
        let legend_event_bindings = legend_event_bindings(&legends, session_context)?;
        if !is_root {
            tool_context.register_local_legend_event_bindings(&legend_event_bindings)?;
        }

        let mut event_bindings = if is_root {
            self.event_bindings
                .iter()
                .cloned()
                .map(ChartEventBinding::with_plot_surface_target)
                .collect()
        } else {
            Vec::new()
        };
        if is_root {
            event_bindings.extend(legend_event_bindings);
        }
        let cursor_params = self.cursor_params.clone();
        let mut tool_metadata = Vec::new();
        if is_root {
            let artifacts = tool_context.finalize_root()?;
            param_source_specs.extend(artifacts.param_specs);
            store_source_specs.extend(artifacts.store_specs);
            for spec in artifacts.selection_specs {
                if selection_specs.contains_key(&spec.id) {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "Duplicate plot selection '{}'",
                        spec.id
                    )));
                }
                selection_specs.insert(spec.id.clone(), spec);
            }
            event_bindings.extend(artifacts.event_bindings);
            tool_metadata.extend(artifacts.metadata);
        }

        event_bindings = event_bindings
            .into_iter()
            .map(|binding| rewrite_reserved_event_binding_local_datums(binding, session_context))
            .collect::<Result<_, AvengerChartError>>()?;

        for binding in &event_bindings {
            binding.validate()?;
        }
        event_bindings = event_bindings
            .into_iter()
            .map(|binding| {
                resolve_event_binding_mark_targets(binding, &flat_marks.mark_target_registry)
            })
            .collect::<Result<_, AvengerChartError>>()?;

        let mut param_specs: IndexMap<String, CompiledParamSpec> = IndexMap::new();
        for spec in &param_source_specs {
            if param_specs.contains_key(&spec.name) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Duplicate plot parameter '{}'",
                    spec.name
                )));
            }
            param_specs.insert(spec.name.clone(), spec.clone());
        }

        // Derive the flat default-param map for existing callers from the specs.
        let default_params: IndexMap<String, ScalarValue> = param_specs
            .values()
            .map(|spec| (spec.name.clone(), spec.default.clone()))
            .collect();

        let mut store_specs = IndexMap::new();
        for spec in store_source_specs {
            if store_specs.contains_key(&spec.name) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Duplicate plot store '{}'",
                    spec.name
                )));
            }
            store_specs.insert(spec.name.clone(), spec);
        }

        // 5. Build CompiledPlot (we do not store a persistent ScaleBuilder; it is
        // rebuilt per evaluation using current params for correctness.)
        let mut compiled = CompiledPlot {
            coord_transform,
            compiled_guide: Some(compiled_guide),
            marks: compiled_marks,
            mark_groups,
            mark_group_index_by_mark: flat_marks.mark_group_indices,
            axis_specs,
            legends,
            legend_colorbar_overlays,
            layout_spec: self.layout_spec,
            title: self.title,
            subtitle: self.subtitle,
            theme: self.theme,
            time_context: effective_time_context,
            formatting_context: effective_formatting_context,
            scale_to_coord_channel,
            scale_specs,
            data: data_plan_node,
            default_params,
            param_specs,
            store_specs,
            event_bindings,
            event_datum_fields: Vec::new(),
            event_coord_fields: Vec::new(),
            selection_specs,
            cursor_params,
            tool_metadata,
        };
        compiled.event_datum_fields = compiled.infer_event_datum_fields(session_context).await?;
        compiled.event_coord_fields = compiled.infer_event_coord_fields(session_context).await?;

        // 6. Validate scoped raw-domain params are shared at least as broadly as
        // the scales they drive (catches Free/Level pan misconfigurations early).
        compiled.validate_coordinate_domain_metrics()?;
        compiled.validate_scoped_raw_domain_sharing(session_context)?;
        compiled.validate_transform_output_scale_sharing()?;

        Ok(compiled)
    }

    /// Get a reference to the coordinate system
    pub fn coord_system(&self) -> &C {
        &self.coord_system
    }

    pub fn mark<M>(mut self, mark: M) -> Self
    where
        M: IntoPlotMark<C>,
    {
        // Just store plot elements - config extraction happens during compile().
        self.marks.extend(mark.into_plot_marks());
        self
    }

    /// Set plot-level data that can be inherited by marks
    pub fn data(mut self, data: DataFrame) -> Self {
        self.data = Some(data);
        self
    }

    /// Add a parameter that can be used in plot expressions.
    ///
    /// The parameter is globally shared (`CoordinationScope::Shared`): one
    /// value across every facet cell. Use [`Plot::add_param_with_sharing`]
    /// to register a parameter with a finer-grained facet sharing scope.
    pub fn add_param(mut self, param: Param) -> Self {
        self.param_specs.push(CompiledParamSpec::shared(&param));
        self
    }

    /// Add multiple parameters at once, all globally shared.
    pub fn add_params(mut self, params: impl IntoIterator<Item = Param>) -> Self {
        self.param_specs
            .extend(params.into_iter().map(|p| CompiledParamSpec::shared(&p)));
        self
    }

    /// Add a parameter with an explicit facet sharing scope.
    ///
    /// `CoordinationScope::Free`/`Level(0)` gives one value per leaf coordinate scope,
    /// `CoordinationScope::Level(N)` shares per logical ancestor `N` levels up, and
    /// `CoordinationScope::Shared` keeps one global value.
    pub fn add_param_with_sharing(mut self, param: Param, sharing: CoordinationScope) -> Self {
        self.param_specs
            .push(CompiledParamSpec::new(&param, sharing));
        self
    }

    /// Add a plot-level event binding that can patch one or more params in chart apps.
    pub fn event_binding(mut self, binding: ChartEventBinding) -> Self {
        self.event_bindings.push(binding);
        self
    }

    /// Add multiple plot-level event bindings.
    pub fn event_bindings(mut self, bindings: impl IntoIterator<Item = ChartEventBinding>) -> Self {
        self.event_bindings.extend(bindings);
        self
    }

    /// Register a plot-level selection.
    pub fn add_selection(mut self, selection: Selection) -> Self {
        self.selections.push(selection);
        self
    }

    /// Register a plot-level mutable store.
    pub fn add_store(mut self, store: Store) -> Self {
        self.stores.push(store);
        self
    }

    /// Register multiple plot-level mutable stores.
    pub fn add_stores(mut self, stores: impl IntoIterator<Item = Store>) -> Self {
        self.stores.extend(stores);
        self
    }

    /// Mark a parameter as app cursor state.
    ///
    /// Event bindings may patch this param with `ev::cursor(...)`; chart apps
    /// apply cursor-only patches without rebuilding the chart scene.
    pub fn cursor_param(mut self, param: impl Into<String>) -> Self {
        self.cursor_params.push(param.into());
        self
    }

    /// Add an authoring-time chart tool.
    pub fn tool<T: ChartTool<C>>(mut self, tool: T) -> Self {
        self.tools.push(Arc::new(tool));
        self
    }

    /// Add multiple authoring-time chart tools of the same concrete type.
    pub fn tools<T: ChartTool<C>>(mut self, tools: impl IntoIterator<Item = T>) -> Self {
        self.tools.extend(
            tools
                .into_iter()
                .map(|tool| Arc::new(tool) as Arc<dyn ChartTool<C>>),
        );
        self
    }

    /// Get the layout specification
    pub fn get_layout_spec(&self) -> &LayoutSpec {
        &self.layout_spec
    }

    // ====== Layout API ======

    /// Set fixed canvas dimensions (traditional mode).
    ///
    /// For faceted charts, this selects canvas-fit facet sizing.
    /// The plot area is solved to fit within the provided canvas.
    ///
    /// Accepts numeric literals (e.g., `800.0`), `Expr` values, or column references via `col()`
    pub fn canvas_size<W, H>(mut self, width: W, height: H) -> Self
    where
        W: IntoExpr,
        H: IntoExpr,
    {
        let width_expr = width.into_expr();
        let height_expr = height.into_expr();

        self.layout_spec.canvas = SizeMode::Fixed {
            width: serializable_expr_from_expr(width_expr, "canvas width"),
            height: serializable_expr_from_expr(height_expr, "canvas height"),
        };
        self
    }

    /// Set canvas sizing constraint for responsive layouts
    pub fn canvas_constraint(mut self, constraint: CanvasConstraint) -> Self {
        self.layout_spec.canvas = constraint.into();
        self
    }

    /// Set fixed plot area dimensions (data-first mode).
    ///
    /// For non-facet charts, this fixes the top-level plot area.
    /// For top-level faceted charts, this selects plot-area-sized sizing where
    /// `width`/`height` are interpreted as per-leaf-subplot plot-area dimensions and
    /// the root canvas grows to fit the facet tree.
    ///
    /// Accepts numeric literals (e.g., `400.0`), `Expr` values, or column references via `col()`
    pub fn plot_size<W, H>(mut self, width: W, height: H) -> Self
    where
        W: IntoExpr,
        H: IntoExpr,
    {
        let width_expr = width.into_expr();
        let height_expr = height.into_expr();

        self.layout_spec.plot_area = SizeMode::Fixed {
            width: serializable_expr_from_expr(width_expr, "plot width"),
            height: serializable_expr_from_expr(height_expr, "plot height"),
        };
        self
    }

    /// Set plot area sizing constraint for responsive layouts
    pub fn plot_constraint(mut self, constraint: PlotConstraint) -> Self {
        self.layout_spec.plot_area = match constraint {
            PlotConstraint::Auto => SizeMode::Auto,
            PlotConstraint::Width(w) => {
                SizeMode::Width(serializable_expr_from_expr(w, "plot width constraint"))
            }
            PlotConstraint::Height(h) => {
                SizeMode::Height(serializable_expr_from_expr(h, "plot height constraint"))
            }
        };
        self
    }

    /// Set margins
    pub fn margins(mut self, margins: Margins) -> Self {
        self.layout_spec.margins = margins;
        self
    }

    /// Set the theme for the plot
    pub fn theme(mut self, theme: Theme) -> Self {
        self.theme = Some(Arc::new(theme));
        self
    }

    /// Set time handling defaults for temporal transforms, scales, and guides.
    pub fn time_context(mut self, time_context: TimeContext) -> Self {
        self.time_context = time_context;
        self
    }

    /// Set formatting defaults for scales, guides, and retained markup labels.
    pub fn formatting_context(mut self, formatting_context: FormattingContext) -> Self {
        self.formatting_context = formatting_context;
        self
    }

    /// Configure the guide (coordinate system visual elements like axes and background)
    pub fn configure_guide(mut self, guide: C::Guide) -> Self {
        self.guide_config = match self.guide_config {
            Some(existing) => {
                let mut updated = existing.clone();
                updated.update(guide);
                Some(updated)
            }
            None => Some(guide),
        };
        self
    }

    /// Access the configured theme (or default if not set)
    pub fn get_theme(&self) -> Arc<Theme> {
        self.theme
            .clone()
            .unwrap_or_else(|| Arc::new(Theme::light()))
    }
}

enum MaybeLoweredRepeatPlot<C: CoordinateSystem> {
    Original(Box<Plot<C>>),
    Lowered(Box<LoweredRepeatPlot>),
}

enum LoweredRepeatPlot {
    Columns(Box<Plot<HConcat>>),
    Rows(Box<Plot<VConcat>>),
    Grid(Box<Plot<GridConcat>>),
    Wrap(Box<Plot<WrapConcat>>),
}

impl LoweredRepeatPlot {
    async fn compile_with_tool_context(
        self,
        session_context: &datafusion::prelude::SessionContext,
        inherited_tool_context: Option<&ToolCompileContext>,
        is_root: bool,
    ) -> Result<CompiledPlot, AvengerChartError> {
        match self {
            Self::Columns(plot) => {
                Box::pin(plot.compile_with_tool_context(
                    session_context,
                    inherited_tool_context,
                    is_root,
                ))
                .await
            }
            Self::Rows(plot) => {
                Box::pin(plot.compile_with_tool_context(
                    session_context,
                    inherited_tool_context,
                    is_root,
                ))
                .await
            }
            Self::Grid(plot) => {
                Box::pin(plot.compile_with_tool_context(
                    session_context,
                    inherited_tool_context,
                    is_root,
                ))
                .await
            }
            Self::Wrap(plot) => {
                Box::pin(plot.compile_with_tool_context(
                    session_context,
                    inherited_tool_context,
                    is_root,
                ))
                .await
            }
        }
    }
}

fn try_lower_repeat_plot<C: CoordinateSystem>(
    plot: Plot<C>,
    session_context: &datafusion::prelude::SessionContext,
) -> Result<MaybeLoweredRepeatPlot<C>, AvengerChartError> {
    if let Some(repeat) = (&plot.coord_system as &dyn Any)
        .downcast_ref::<RepeatColumns>()
        .cloned()
    {
        return Ok(MaybeLoweredRepeatPlot::Lowered(Box::new(
            LoweredRepeatPlot::Columns(Box::new(lower_repeat_columns_plot(
                plot,
                repeat,
                session_context,
            )?)),
        )));
    }
    if let Some(repeat) = (&plot.coord_system as &dyn Any)
        .downcast_ref::<RepeatRows>()
        .cloned()
    {
        return Ok(MaybeLoweredRepeatPlot::Lowered(Box::new(
            LoweredRepeatPlot::Rows(Box::new(lower_repeat_rows_plot(
                plot,
                repeat,
                session_context,
            )?)),
        )));
    }
    if let Some(repeat) = (&plot.coord_system as &dyn Any)
        .downcast_ref::<RepeatGrid>()
        .cloned()
    {
        return Ok(MaybeLoweredRepeatPlot::Lowered(Box::new(
            LoweredRepeatPlot::Grid(Box::new(lower_repeat_grid_plot(
                plot,
                repeat,
                session_context,
            )?)),
        )));
    }
    if let Some(repeat) = (&plot.coord_system as &dyn Any)
        .downcast_ref::<RepeatWrap>()
        .cloned()
    {
        return Ok(MaybeLoweredRepeatPlot::Lowered(Box::new(
            LoweredRepeatPlot::Wrap(Box::new(lower_repeat_wrap_plot(
                plot,
                repeat,
                session_context,
            )?)),
        )));
    }

    Ok(MaybeLoweredRepeatPlot::Original(Box::new(plot)))
}

#[allow(clippy::type_complexity)]
struct RepeatPlotParts<C: CoordinateSystem> {
    data: Option<DataFrame>,
    scale_specs: HashMap<String, ScaleSpec>,
    legends: IndexMap<String, Legend>,
    layout_spec: LayoutSpec,
    title: Option<PlotTitle>,
    subtitle: Option<PlotSubtitle>,
    theme: Option<Arc<Theme>>,
    time_context: TimeContext,
    formatting_context: FormattingContext,
    param_specs: Vec<CompiledParamSpec>,
    event_bindings: Vec<ChartEventBinding>,
    selections: Vec<Selection>,
    stores: Vec<Store>,
    cursor_params: Vec<String>,
    _phantom: std::marker::PhantomData<fn() -> C>,
}

fn split_repeat_plot<C: CoordinateSystem>(
    plot: Plot<C>,
    kind: &str,
) -> Result<RepeatPlotParts<C>, AvengerChartError> {
    let Plot {
        coord_system: _,
        marks,
        data,
        scale_specs,
        legends,
        layout_spec,
        title,
        subtitle,
        theme,
        time_context,
        formatting_context,
        guide_config,
        param_specs,
        event_bindings,
        selections,
        stores,
        cursor_params,
        tools,
    } = plot;

    if !marks.is_empty() {
        return Err(AvengerChartError::InvalidArgument(format!(
            "{kind} uses `.cell(...)` for its repeated child plot and does not support root-level marks"
        )));
    }
    if guide_config.is_some() {
        return Err(AvengerChartError::InvalidArgument(format!(
            "{kind} does not support root-level guide configuration in this repeat stage"
        )));
    }
    if !tools.is_empty() {
        return Err(AvengerChartError::InvalidArgument(format!(
            "{kind} does not support root-level tools in this repeat stage; attach tools to the repeated cell plot"
        )));
    }

    Ok(RepeatPlotParts {
        data,
        scale_specs,
        legends,
        layout_spec,
        title,
        subtitle,
        theme,
        time_context,
        formatting_context,
        param_specs,
        event_bindings,
        selections,
        stores,
        cursor_params,
        _phantom: std::marker::PhantomData,
    })
}

fn finish_lowered_repeat_plot<C, P>(
    parts: RepeatPlotParts<C>,
    coord_system: P,
    marks: Vec<Arc<dyn Mark<P>>>,
) -> Plot<P>
where
    C: CoordinateSystem,
    P: CoordinateSystem,
{
    Plot {
        coord_system,
        marks: marks.into_iter().map(PlotMark::from_mark_arc).collect(),
        data: parts.data,
        scale_specs: parts.scale_specs,
        legends: parts.legends,
        layout_spec: parts.layout_spec,
        title: parts.title,
        subtitle: parts.subtitle,
        theme: parts.theme,
        time_context: parts.time_context,
        formatting_context: parts.formatting_context,
        guide_config: None,
        param_specs: parts.param_specs,
        event_bindings: parts.event_bindings,
        selections: parts.selections,
        stores: parts.stores,
        cursor_params: parts.cursor_params,
        tools: Vec::new(),
    }
}

#[derive(Clone)]
struct AuthoringMarkGroupState {
    id: Option<String>,
    parent_group_index: Option<usize>,
    scale_inference_hints: Vec<ScaleInferenceHint>,
    data: DataContext,
    data_mode: MarkDataMode,
    facet_data_scope: avenger_chart_core::FacetDataScope,
}

struct FlattenedPlotMarks<C: CoordinateSystem> {
    marks: Vec<Arc<dyn Mark<C>>>,
    group_states: Vec<AuthoringMarkGroupState>,
    mark_group_indices: Vec<Option<usize>>,
    public_target_paths: Vec<Option<String>>,
    /// Root marks and children of idless groups share this unqualified id
    /// namespace even when the child id is not exposed as a public target.
    unqualified_mark_ids: HashSet<String>,
    mark_target_registry: MarkTargetRegistry,
}

impl<C: CoordinateSystem> FlattenedPlotMarks<C> {
    fn reserve_unqualified_mark_id(&mut self, id: &str) -> Result<(), AvengerChartError> {
        if self.unqualified_mark_ids.insert(id.to_string()) {
            Ok(())
        } else {
            Err(AvengerChartError::InvalidArgument(format!(
                "Duplicate mark id '{id}' among sibling marks"
            )))
        }
    }
}

#[derive(Clone, Debug, Default)]
struct MarkTargetRegistry {
    paths: HashMap<String, Vec<Vec<usize>>>,
}

impl MarkTargetRegistry {
    fn insert(
        &mut self,
        target: String,
        mark_paths: Vec<Vec<usize>>,
    ) -> Result<(), AvengerChartError> {
        if self.paths.contains_key(&target) {
            let message = if target.contains('.') {
                format!("Duplicate mark target path '{target}'")
            } else {
                format!("Duplicate mark id '{target}' among sibling marks")
            };
            return Err(AvengerChartError::InvalidArgument(message));
        }
        self.paths.insert(target, mark_paths);
        Ok(())
    }

    fn resolve(&self, target: &str) -> Result<Vec<Vec<usize>>, AvengerChartError> {
        self.paths.get(target).cloned().ok_or_else(|| {
            AvengerChartError::InvalidArgument(format!("Unknown mark target '{target}'"))
        })
    }
}

fn flatten_plot_marks<C: CoordinateSystem>(
    elements: &[PlotMark<C>],
    repeat_context: Option<&RepeatContext>,
) -> Result<FlattenedPlotMarks<C>, AvengerChartError> {
    let empty_repeat_context;
    let repeat_context = match repeat_context {
        Some(repeat_context) => repeat_context,
        None => {
            empty_repeat_context = RepeatContext::default();
            &empty_repeat_context
        }
    };
    let mut flat = FlattenedPlotMarks {
        marks: Vec::new(),
        group_states: Vec::new(),
        mark_group_indices: Vec::new(),
        public_target_paths: Vec::new(),
        unqualified_mark_ids: HashSet::new(),
        mark_target_registry: MarkTargetRegistry::default(),
    };
    flatten_plot_mark_elements(elements, None, &[], repeat_context, &mut flat)?;
    Ok(flat)
}

fn flatten_plot_mark_elements<C: CoordinateSystem>(
    elements: &[PlotMark<C>],
    parent_group_index: Option<usize>,
    public_path_prefix: &[String],
    repeat_context: &RepeatContext,
    flat: &mut FlattenedPlotMarks<C>,
) -> Result<Vec<usize>, AvengerChartError> {
    let mut descendant_mark_indices = Vec::new();
    for element in elements {
        match element.kind() {
            PlotMarkKind::InvalidArgument(message) => {
                return Err(AvengerChartError::InvalidArgument(message.clone()));
            }
            PlotMarkKind::Primitive(mark) => {
                if let Some(id) = mark.state().id.as_deref() {
                    validate_structural_id("mark", id)?;
                    if public_path_prefix.is_empty() {
                        flat.reserve_unqualified_mark_id(id)?;
                    }
                }
                if parent_group_index.is_some() {
                    let state = mark.state();
                    if state.has_explicit_data_source() {
                        return Err(AvengerChartError::InvalidArgument(
                            "Primitive child marks inside MarkGroup may not set explicit data or data_store"
                                .to_string(),
                        ));
                    }
                    if state.data_mode == MarkDataMode::Unit {
                        return Err(AvengerChartError::InvalidArgument(
                            "Primitive child marks inside MarkGroup may not use unit_data()"
                                .to_string(),
                        ));
                    }
                }
                let mark_index = flat.marks.len();
                let public_target_path = mark_public_target_path(
                    mark.state().id.as_deref(),
                    parent_group_index,
                    public_path_prefix,
                );
                flat.marks.push(mark.clone());
                flat.mark_group_indices.push(parent_group_index);
                flat.public_target_paths.push(public_target_path.clone());
                if let Some(public_target_path) = public_target_path {
                    flat.mark_target_registry
                        .insert(public_target_path, vec![vec![mark_index]])?;
                }
                descendant_mark_indices.push(mark_index);
            }
            PlotMarkKind::Group(group) => {
                group.validate_id()?;
                if group.children().is_empty() {
                    return Err(AvengerChartError::InvalidArgument(
                        "MarkGroup must contain at least one child mark or group".to_string(),
                    ));
                }
                let group_public_path_prefix =
                    group_public_path_prefix(public_path_prefix, group.id_ref());
                let group_index = flat.group_states.len();
                flat.group_states.push(AuthoringMarkGroupState {
                    id: group.id_ref().map(ToString::to_string),
                    parent_group_index,
                    scale_inference_hints: group.scale_inference_hints().to_vec(),
                    data: group.data_context().resolve_repeat(repeat_context)?,
                    data_mode: group.data_mode(),
                    facet_data_scope: group.facet_data_scope_value(),
                });
                let group_descendants = flatten_plot_mark_elements(
                    group.children(),
                    Some(group_index),
                    &group_public_path_prefix,
                    repeat_context,
                    flat,
                )?;
                if group.id_ref().is_some() && !group_public_path_prefix.is_empty() {
                    let group_target = group_public_path_prefix.join(".");
                    let paths = group_descendants
                        .iter()
                        .map(|mark_index| vec![*mark_index])
                        .collect();
                    flat.mark_target_registry.insert(group_target, paths)?;
                }
                descendant_mark_indices.extend(group_descendants);
            }
        }
    }
    Ok(descendant_mark_indices)
}

fn group_public_path_prefix(parent: &[String], group_id: Option<&str>) -> Vec<String> {
    let mut prefix = parent.to_vec();
    if let Some(group_id) = group_id {
        prefix.push(group_id.to_string());
    }
    prefix
}

fn mark_public_target_path(
    mark_id: Option<&str>,
    parent_group_index: Option<usize>,
    public_path_prefix: &[String],
) -> Option<String> {
    let mark_id = mark_id?;
    if public_path_prefix.is_empty() {
        parent_group_index.is_none().then(|| mark_id.to_string())
    } else {
        let mut segments = public_path_prefix.to_vec();
        segments.push(mark_id.to_string());
        Some(segments.join("."))
    }
}

fn resolve_event_binding_mark_targets(
    mut binding: ChartEventBinding,
    registry: &MarkTargetRegistry,
) -> Result<ChartEventBinding, AvengerChartError> {
    if let Some(mut between) = binding.between.take() {
        between.start = resolve_event_stream_mark_targets(between.start, registry)?;
        between.end = resolve_event_stream_mark_targets(between.end, registry)?;
        binding.between = Some(between);
    }
    for assignment in &mut binding.selection_assignments {
        assignment.update =
            resolve_selection_update_scene_query_mark_targets(assignment.update.clone(), registry)?;
    }
    Ok(binding)
}

fn resolve_event_stream_mark_targets(
    stream: ChartEventStream,
    registry: &MarkTargetRegistry,
) -> Result<ChartEventStream, AvengerChartError> {
    let paths = resolve_mark_target_paths(stream.mark_ids(), registry)?;
    Ok(if paths.is_empty() {
        stream
    } else {
        stream.with_resolved_mark_paths(paths)
    })
}

fn resolve_selection_update_scene_query_mark_targets(
    update: SelectionUpdate,
    registry: &MarkTargetRegistry,
) -> Result<SelectionUpdate, AvengerChartError> {
    Ok(match update {
        SelectionUpdate::ReplaceAllFromSceneQuery { query } => {
            SelectionUpdate::ReplaceAllFromSceneQuery {
                query: resolve_selection_scene_query_mark_targets(query, registry)?,
            }
        }
        SelectionUpdate::ReplaceFromSceneQueryInScope { query } => {
            SelectionUpdate::ReplaceFromSceneQueryInScope {
                query: resolve_selection_scene_query_mark_targets(query, registry)?,
            }
        }
        SelectionUpdate::UpsertFromSceneQuery { query } => SelectionUpdate::UpsertFromSceneQuery {
            query: resolve_selection_scene_query_mark_targets(query, registry)?,
        },
        SelectionUpdate::ToggleFromSceneQuery { query } => SelectionUpdate::ToggleFromSceneQuery {
            query: resolve_selection_scene_query_mark_targets(query, registry)?,
        },
        other => other,
    })
}

fn resolve_selection_scene_query_mark_targets(
    mut query: SelectionSceneQuery,
    registry: &MarkTargetRegistry,
) -> Result<SelectionSceneQuery, AvengerChartError> {
    query.query.target = resolve_scene_geometry_target_mark_targets(query.query.target, registry)?;
    Ok(query)
}

fn resolve_scene_geometry_target_mark_targets(
    target: SceneGeometryTarget,
    registry: &MarkTargetRegistry,
) -> Result<SceneGeometryTarget, AvengerChartError> {
    let paths = resolve_mark_target_paths(target.mark_ids(), registry)?;
    Ok(if paths.is_empty() {
        target
    } else {
        target.with_resolved_mark_paths(paths)
    })
}

fn resolve_mark_target_paths(
    targets: &[String],
    registry: &MarkTargetRegistry,
) -> Result<Vec<Vec<usize>>, AvengerChartError> {
    let mut paths = Vec::new();
    for target in targets {
        paths.extend(registry.resolve(target)?);
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn compile_mark_group_states(
    groups: &[AuthoringMarkGroupState],
) -> Vec<super::compiled::CompiledMarkGroupState> {
    groups
        .iter()
        .map(|group| {
            let data = if let Some(store_data) = group.data.store_data_ref() {
                CompiledDataContext::new_store_data(
                    store_data.clone(),
                    group.data.transforms().to_vec(),
                    group.data.channels().clone(),
                )
            } else {
                CompiledDataContext::new(
                    group.data.dataframe().cloned(),
                    group.data.transforms().to_vec(),
                    group.data.channels().clone(),
                )
            };
            super::compiled::CompiledMarkGroupState {
                id: group.id.clone(),
                parent_group_index: group.parent_group_index,
                scale_inference_hints: group.scale_inference_hints.clone(),
                data,
                data_mode: group.data_mode,
                facet_data_scope: group.facet_data_scope,
            }
        })
        .collect()
}

fn lower_repeat_columns_plot<C: CoordinateSystem>(
    plot: Plot<C>,
    repeat: RepeatColumns,
    session_context: &datafusion::prelude::SessionContext,
) -> Result<Plot<HConcat>, AvengerChartError> {
    let variables = resolve_repeat_variables(
        "RepeatColumns",
        "columns",
        repeat.columns_config(),
        session_context,
    )?;
    let column_count = variables.len();
    let marks = variables
        .into_iter()
        .enumerate()
        .map(|(column_index, column)| {
            let key = format!("repeat_col:{}", column.id);
            let id = format!("repeat_col_{}", column.id);
            let label = column.title.clone();
            let repeat_context = RepeatContext::new()
                .with_column(column, column_index, column_count)
                .with_domain_coordination(repeat.domain_coordination_config().clone());
            let cell = repeat.cell_templates().select(
                "RepeatColumns",
                &key,
                &repeat_context,
                session_context,
            )?;
            Ok(Arc::new(
                Subplot::<HConcat>::new(RepeatResolvedChildPlotSpec::new(cell, repeat_context))
                    .key(key)
                    .id(id)
                    .label(label),
            ) as Arc<dyn Mark<HConcat>>)
        })
        .collect::<Result<Vec<_>, AvengerChartError>>()?;
    let parts = split_repeat_plot(plot, "RepeatColumns")?;
    Ok(finish_lowered_repeat_plot(
        parts,
        HConcat::new().with_origin(ConcatOrigin::RepeatColumns),
        marks,
    ))
}

fn lower_repeat_rows_plot<C: CoordinateSystem>(
    plot: Plot<C>,
    repeat: RepeatRows,
    session_context: &datafusion::prelude::SessionContext,
) -> Result<Plot<VConcat>, AvengerChartError> {
    let variables =
        resolve_repeat_variables("RepeatRows", "rows", repeat.rows_config(), session_context)?;
    let row_count = variables.len();
    let marks = variables
        .into_iter()
        .enumerate()
        .map(|(row_index, row)| {
            let key = format!("repeat_row:{}", row.id);
            let id = format!("repeat_row_{}", row.id);
            let label = row.title.clone();
            let repeat_context = RepeatContext::new()
                .with_row(row, row_index, row_count)
                .with_domain_coordination(repeat.domain_coordination_config().clone());
            let cell = repeat.cell_templates().select(
                "RepeatRows",
                &key,
                &repeat_context,
                session_context,
            )?;
            Ok(Arc::new(
                Subplot::<VConcat>::new(RepeatResolvedChildPlotSpec::new(cell, repeat_context))
                    .key(key)
                    .id(id)
                    .label(label),
            ) as Arc<dyn Mark<VConcat>>)
        })
        .collect::<Result<Vec<_>, AvengerChartError>>()?;
    let parts = split_repeat_plot(plot, "RepeatRows")?;
    Ok(finish_lowered_repeat_plot(
        parts,
        VConcat::new().with_origin(ConcatOrigin::RepeatRows),
        marks,
    ))
}

fn lower_repeat_grid_plot<C: CoordinateSystem>(
    plot: Plot<C>,
    repeat: RepeatGrid,
    session_context: &datafusion::prelude::SessionContext,
) -> Result<Plot<GridConcat>, AvengerChartError> {
    let rows =
        resolve_repeat_variables("RepeatGrid", "rows", repeat.rows_config(), session_context)?;
    let columns = resolve_repeat_variables(
        "RepeatGrid",
        "columns",
        repeat.columns_config(),
        session_context,
    )?;
    let row_count = rows.len();
    let column_count = columns.len();
    let mut marks = Vec::new();
    for (row_index, row) in rows.iter().cloned().enumerate() {
        for (column_index, column) in columns.iter().cloned().enumerate() {
            let key = format!("repeat_cell:{}:{}", row.id, column.id);
            let id = format!("repeat_cell_{}_{}", row.id, column.id);
            let repeat_context = RepeatContext::new()
                .with_row(row.clone(), row_index, row_count)
                .with_column(column.clone(), column_index, column_count)
                .with_domain_coordination(repeat.domain_coordination_config().clone())
                .with_matrix_axis_defaults(repeat.matrix_axis_defaults());
            let cell = repeat.cell_templates().select(
                "RepeatGrid",
                &key,
                &repeat_context,
                session_context,
            )?;
            marks.push(Arc::new(
                Subplot::<GridConcat>::new(RepeatResolvedChildPlotSpec::new(cell, repeat_context))
                    .key(key)
                    .id(id)
                    .grid_cell(row_index, column_index),
            ) as Arc<dyn Mark<GridConcat>>);
        }
    }
    let parts = split_repeat_plot(plot, "RepeatGrid")?;
    Ok(finish_lowered_repeat_plot(
        parts,
        GridConcat::new()
            .rows(row_count)
            .columns(column_count)
            .with_axis_guide_visibility_config(repeat.axis_guide_visibility_config())
            .with_origin(ConcatOrigin::RepeatGrid),
        marks,
    ))
}

fn lower_repeat_wrap_plot<C: CoordinateSystem>(
    plot: Plot<C>,
    repeat: RepeatWrap,
    session_context: &datafusion::prelude::SessionContext,
) -> Result<Plot<WrapConcat>, AvengerChartError> {
    let variables = resolve_repeat_variables(
        "RepeatWrap",
        "items",
        repeat.items_config(),
        session_context,
    )?;
    let item_count = variables.len();
    let marks = variables
        .into_iter()
        .enumerate()
        .map(|(item_index, item)| {
            let key = format!("repeat_item:{}", item.id);
            let id = format!("repeat_item_{}", item.id);
            let label = item.title.clone();
            let repeat_context = RepeatContext::new()
                .with_item(item, item_index, item_count)
                .with_domain_coordination(repeat.domain_coordination_config().clone());
            let cell = repeat.cell_templates().select(
                "RepeatWrap",
                &key,
                &repeat_context,
                session_context,
            )?;
            Ok(Arc::new(
                Subplot::<WrapConcat>::new(RepeatResolvedChildPlotSpec::new(cell, repeat_context))
                    .key(key)
                    .id(id)
                    .label(label),
            ) as Arc<dyn Mark<WrapConcat>>)
        })
        .collect::<Result<Vec<_>, AvengerChartError>>()?;
    let parts = split_repeat_plot(plot, "RepeatWrap")?;
    Ok(finish_lowered_repeat_plot(
        parts,
        WrapConcat::new()
            .with_column_mode(repeat.column_mode_config().clone())
            .with_origin(ConcatOrigin::RepeatWrap),
        marks,
    ))
}

fn resolve_repeat_variables(
    kind: &str,
    axis: &str,
    variables: &[RepeatVariable],
    session_context: &datafusion::prelude::SessionContext,
) -> Result<Vec<avenger_chart_core::ResolvedRepeatVariable>, AvengerChartError> {
    if variables.is_empty() {
        return Err(AvengerChartError::InvalidArgument(format!(
            "{kind} requires at least one repeat {axis} variable"
        )));
    }
    let mut ids = HashSet::new();
    let mut resolved = Vec::with_capacity(variables.len());
    for variable in variables {
        let variable = variable.resolve(session_context)?;
        if !ids.insert(variable.id.clone()) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "{kind} has duplicate repeat {axis} variable id '{}'",
                variable.id
            )));
        }
        resolved.push(variable);
    }
    Ok(resolved)
}

fn scale_domain_coordinations_from_states(
    states: &[MarkState],
    coord_transform: &dyn CoordinateSystemTransformCore,
) -> Result<HashMap<String, DomainCoordination>, AvengerChartError> {
    let state_refs = states.iter().collect::<Vec<_>>();
    scale_domain_coordinations_from_state_refs(&state_refs, coord_transform)
}

fn scale_domain_coordinations_from_state_refs(
    states: &[&MarkState],
    coord_transform: &dyn CoordinateSystemTransformCore,
) -> Result<HashMap<String, DomainCoordination>, AvengerChartError> {
    let mut result: HashMap<String, DomainCoordination> = HashMap::new();
    let mut merge_coordination = |channel_name: &str,
                                  channel_value: &avenger_chart_core::ChannelValue|
     -> Result<(), AvengerChartError> {
        if !coord_transform.channel_uses_scale(channel_name) {
            return Ok(());
        }
        let Some(scale_name) = channel_value.get_scale_name(channel_name) else {
            return Ok(());
        };
        let Some(coordination) = channel_value.get_domain_coordination() else {
            return Ok(());
        };
        let coordination = coordination.clone();
        match result.get_mut(&scale_name) {
            Some(existing) => {
                if existing.group != coordination.group {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "scale '{scale_name}' has incompatible domain groups"
                    )));
                }
                if coordination.scope.to_level() > existing.scope.to_level() {
                    existing.scope = coordination.scope;
                }
            }
            None => {
                result.insert(scale_name, coordination);
            }
        }
        Ok(())
    };

    for state in states {
        for (channel_name, channel_value) in state.data.channels() {
            merge_coordination(channel_name, channel_value)?;
        }
    }
    Ok(result)
}

fn resolve_mark_states<C: CoordinateSystem>(
    marks: &[Arc<dyn Mark<C>>],
    repeat_context: Option<&RepeatContext>,
) -> Result<Vec<MarkState>, AvengerChartError> {
    let empty_repeat_context;
    let repeat_context = match repeat_context {
        Some(repeat_context) => repeat_context,
        None => {
            empty_repeat_context = RepeatContext::default();
            &empty_repeat_context
        }
    };
    marks
        .iter()
        .map(|mark| {
            let mut state = mark.state().resolve_repeat(repeat_context)?;
            apply_repeat_matrix_axis_defaults::<C>(&mut state, repeat_context)?;
            Ok(state)
        })
        .collect()
}

fn apply_repeat_matrix_axis_defaults<C: CoordinateSystem>(
    state: &mut MarkState,
    repeat_context: &RepeatContext,
) -> Result<(), AvengerChartError>
where
    <C::Guide as CoordinateGuide>::Axis: 'static,
{
    if !repeat_context.matrix_axis_defaults {
        return Ok(());
    }

    let updates = state
        .data
        .channels()
        .iter()
        .filter_map(|(channel, value)| {
            if value.get_axis_config().is_some() || state.axis_configs.contains_key(channel) {
                return None;
            }
            repeat_matrix_axis_title(channel, value, repeat_context)
                .map(|title| (channel.clone(), value.clone(), title))
        })
        .collect::<Vec<_>>();

    for (channel, value, title) in updates {
        let mut axis = <C::Guide as CoordinateGuide>::Axis::default();
        if axis.set_default_title_expr(lit(title))? {
            state.data = state
                .data
                .clone()
                .with_channel_value(&channel, value.with_boxed_axis_config(Box::new(axis)));
        }
    }

    Ok(())
}

fn repeat_matrix_axis_title(
    channel: &str,
    value: &avenger_chart_core::ChannelValue,
    repeat_context: &RepeatContext,
) -> Option<String> {
    let variable = match channel {
        "x" => repeat_context.column.as_ref()?,
        "y" => repeat_context.row.as_ref()?,
        _ => return None,
    };
    let coordination = value.get_domain_coordination()?;
    if coordination.group == DomainCoordinationGroup::Named(variable.id.clone()) {
        Some(variable.title.clone())
    } else {
        None
    }
}

fn legend_event_bindings(
    legends: &IndexMap<String, Legend>,
    session_context: &datafusion::prelude::SessionContext,
) -> Result<Vec<ChartEventBinding>, AvengerChartError> {
    let mut bindings = Vec::new();
    for (channel_name, legend) in legends {
        legend.validate_event_surface()?;
        for binding in &legend.event_bindings {
            let binding = binding.clone().with_legend_surface_target(
                vec![channel_name.clone()],
                vec![
                    LegendSurfaceKind::DiscreteItem,
                    LegendSurfaceKind::ContinuousColorbar,
                ],
            );
            let binding = rewrite_reserved_event_binding_local_datums(binding, session_context)?;
            bindings.push(binding);
        }
    }
    Ok(bindings)
}

async fn compile_colorbar_overlays(
    legends: &IndexMap<String, Legend>,
    session_context: &datafusion::prelude::SessionContext,
) -> Result<Vec<CompiledColorbarOverlayMarks>, AvengerChartError> {
    let mut compiled = Vec::new();
    for (channel_name, legend) in legends {
        if legend.colorbar_overlays.is_empty() {
            continue;
        }
        let mut marks = Vec::new();
        for overlay in &legend.colorbar_overlays {
            let Some(overlay) = overlay.downcast_ref::<ColorbarOverlay>() else {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Legend '{}' has an unsupported colorbar overlay type",
                    channel_name
                )));
            };
            marks.extend(overlay.compile(session_context).await?);
        }
        compiled.push(CompiledColorbarOverlayMarks {
            channel_name: channel_name.clone(),
            marks,
        });
    }
    Ok(compiled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartesian::Cartesian;
    use crate::concat::{GridConcat, HConcat, VConcat, WrapConcat, compiled_subplot};
    use crate::event::{ChartEventBinding, ChartEventStream, ChartEventType};
    use crate::facet::coord::{FacetColumn, FacetWrap};
    use crate::facet::marks::{FacetColumnSubplotChannels, FacetWrapSubplotChannels};
    use crate::plot::{EvaluationRequest, SelectionAssignment, SelectionStateUpdate};
    use crate::repeat::{RepeatColumns, RepeatGrid, RepeatRows, RepeatWrap};
    use crate::tools::ToolCompileContext;
    use crate::zerod::ZeroDCoord;
    use avenger_chart_cartesian::{
        CartesianAxis, CartesianRectPositionChannels, CartesianSymbolPositionChannels,
    };
    use avenger_chart_core::{
        AxisGuideVisibilityPolicy, CompiledDataTransform, DataTransform,
        DataTransformCompileContext, DataTransformExecutionContext, DataTransformResult,
        DefaultLogicalExprNodeExt, DomainCoordinationGroup, FormattingContext, IntoPlotMark,
        MarkGroup, PlotMark, RepeatContext, RepeatDomainCoordination, RepeatVariable,
        ResolvedRepeatVariable, ResolvedSelectionClauseScope, ScaleChannelConfig,
        ScaleInferenceHint, ScaleTypePreference, SceneGeometryQuery, SceneQueryDatumField,
        Selection, SelectionClause, SelectionClauseUpdate, SelectionEqualityDimensionValue,
        SelectionPredicateSpec, SelectionPredicateUpdate, SelectionSceneQuery, SelectionUpdate,
        StoreRow, StoreUpdate, SubplotDataSource, collect_repeat_placeholder_kinds, repeat,
        simplify_to_scalar_sync,
    };
    use avenger_chart_marks::{Rect, Subplot, Symbol};
    use avenger_chart_parallel::{
        Parallel, ParallelLine, ParallelSymbol,
        event::{
            PARALLEL_DIMENSION_ID_FIELD, PARALLEL_SURFACE_KIND_DIMENSION_TITLE,
            PARALLEL_SURFACE_KIND_FIELD, PARALLEL_SURFACE_KIND_POINT, PARALLEL_TITLE_FIELD,
            parallel_dimension_id, parallel_surface_kind, parallel_title,
        },
        generated_dimension_channel,
    };
    use avenger_chart_tools::PanScrollZoom;
    use avenger_chart_transforms::{Bin, Calculate, Filter};
    use avenger_scenegraph::marks::{
        line::SceneLineMark,
        mark::{MarkInstance, SceneMark},
    };
    use datafusion::{
        arrow::{
            array::Float64Array,
            datatypes::{DataType, Field, Schema},
            record_batch::RecordBatch,
        },
        functions_aggregate::expr_fn::count,
        prelude::{SessionContext, col, lit},
    };
    use datafusion_proto::protobuf::LogicalExprNode;
    use serde::{Deserialize, Serialize};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::sync::Mutex;

    fn resolved_repeat(id: &str) -> ResolvedRepeatVariable {
        ResolvedRepeatVariable {
            id: id.to_string(),
            expr: col(id),
            title: id.to_string(),
            type_hint: None,
        }
    }

    fn column_repeat_context(id: &str, index: usize, count: usize) -> RepeatContext {
        RepeatContext::new().with_column(resolved_repeat(id), index, count)
    }

    fn repeat_vars(names: &[&str]) -> Vec<RepeatVariable> {
        names
            .iter()
            .map(|name| RepeatVariable::new(*name, col(*name)).title(format!("Title {name}")))
            .collect()
    }

    fn repeated_column_cell() -> Plot<Cartesian> {
        Plot::<Cartesian>::new().mark(Symbol::new().x(repeat::column()).y(lit(1.0)).size(64.0))
    }

    fn repeated_row_cell() -> Plot<Cartesian> {
        Plot::<Cartesian>::new().mark(Symbol::new().x(lit(1.0)).y(repeat::row()).size(64.0))
    }

    #[tokio::test]
    async fn compile_preserves_formatting_context_number_locale() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let number_locale_spec = avenger_text::NumberLocaleSpec {
            decimal: Some("~".to_string()),
            group: Some("_".to_string()),
            ..Default::default()
        };
        let datetime_locale_spec = avenger_text::DateTimeLocaleSpec {
            date_patterns: Some(avenger_text::LengthsSpec {
                long: Some("y'~'MM'~'dd".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let compiled = Plot::<Cartesian>::new()
            .formatting_context(
                FormattingContext::new()
                    .number_locale("custom")
                    .number_locale_spec("custom", number_locale_spec.clone())
                    .datetime_locale("custom-datetime")
                    .datetime_timezone("America/New_York")
                    .datetime_locale_spec("custom-datetime", datetime_locale_spec.clone()),
            )
            .mark(Symbol::new().x(lit(1.0)).y(lit(2.0)))
            .compile(&ctx)
            .await?;

        assert_eq!(
            compiled.formatting_context.resolved_number_locale(),
            "custom"
        );
        assert_eq!(
            compiled
                .formatting_context
                .number_locale_specs()
                .get("custom"),
            Some(&number_locale_spec)
        );
        assert_eq!(
            compiled.formatting_context.resolved_datetime_locale(),
            "custom-datetime"
        );
        assert_eq!(
            compiled.formatting_context.resolved_datetime_timezone(),
            "America/New_York"
        );
        assert_eq!(
            compiled
                .formatting_context
                .datetime_locale_specs()
                .get("custom-datetime"),
            Some(&datetime_locale_spec)
        );

        let serialized =
            bincode::serialize(&compiled).expect("serialize compiled plot with formatting context");
        let decoded: CompiledPlot = bincode::deserialize(&serialized)
            .expect("deserialize compiled plot with formatting context");
        assert_eq!(
            decoded.formatting_context.resolved_number_locale(),
            "custom"
        );
        assert_eq!(
            decoded
                .formatting_context
                .number_locale_specs()
                .get("custom"),
            Some(&number_locale_spec)
        );
        assert_eq!(
            decoded.formatting_context.resolved_datetime_locale(),
            "custom-datetime"
        );
        assert_eq!(
            decoded.formatting_context.resolved_datetime_timezone(),
            "America/New_York"
        );
        assert_eq!(
            decoded
                .formatting_context
                .datetime_locale_specs()
                .get("custom-datetime"),
            Some(&datetime_locale_spec)
        );
        Ok(())
    }

    fn repeated_grid_cell() -> Plot<Cartesian> {
        Plot::<Cartesian>::new().mark(
            Symbol::new()
                .x(repeat::column())
                .y(repeat::row())
                .size(64.0),
        )
    }

    fn collect_scene_line_marks<'a>(marks: &'a [SceneMark], out: &mut Vec<&'a SceneLineMark>) {
        for mark in marks {
            match mark {
                SceneMark::Line(line) => out.push(line),
                SceneMark::Group(group) => collect_scene_line_marks(&group.marks, out),
                _ => {}
            }
        }
    }

    fn diagonal_histogram_count_shared_cell() -> Plot<Cartesian> {
        Plot::<Cartesian>::new().mark(Rect::new().transform(
            Bin::new(repeat::column()).maxbins(5),
            |mark, bin| {
                mark.x(bin.start())
                    .x2(bin.end())
                    .y_with(lit(0.0), |c| {
                        c.with_domain_group("hist_count").share_domain()
                    })
                    .y2_with(count(lit(1)), |c| {
                        c.with_domain_group("hist_count").share_domain()
                    })
            },
        ))
    }

    fn constant_y_grid_cell(value: f64) -> Plot<Cartesian> {
        Plot::<Cartesian>::new().mark(Symbol::new().x(repeat::column()).y(lit(value)).size(64.0))
    }

    struct TestCompoundMark;

    impl IntoPlotMark<Cartesian> for TestCompoundMark {
        fn into_plot_marks(self) -> Vec<PlotMark<Cartesian>> {
            vec![
                PlotMark::from_mark(Symbol::new().id("compound_symbol").x(lit(1.0)).y(lit(2.0))),
                PlotMark::from_mark(Rect::new().id("compound_rect").x(lit(0.0)).y(lit(0.0))),
            ]
        }
    }

    static COUNTING_GROUP_TRANSFORM_APPLIES: AtomicUsize = AtomicUsize::new(0);
    static COUNTING_GROUP_TRANSFORM_TEST_LOCK: Mutex<()> = Mutex::const_new(());

    #[derive(Clone)]
    struct CountingGroupTransform;

    impl DataTransform for CountingGroupTransform {
        type Output = ();

        fn into_compiled_and_output(
            self,
            _ctx: DataTransformCompileContext,
        ) -> Result<(Box<dyn CompiledDataTransform>, Self::Output), AvengerChartError> {
            Ok((Box::new(CompiledCountingGroupTransform), ()))
        }
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct CompiledCountingGroupTransform;

    #[typetag::serde(name = "test_mark_group_counting")]
    #[async_trait::async_trait]
    impl CompiledDataTransform for CompiledCountingGroupTransform {
        fn clone_box(&self) -> Box<dyn CompiledDataTransform> {
            Box::new(self.clone())
        }

        async fn apply(
            &self,
            dataframe: datafusion::dataframe::DataFrame,
            _ctx: &DataTransformExecutionContext<'_>,
        ) -> Result<DataTransformResult, AvengerChartError> {
            COUNTING_GROUP_TRANSFORM_APPLIES.fetch_add(1, Ordering::SeqCst);
            Ok(DataTransformResult::dataframe(dataframe))
        }
    }

    async fn xy_dataframe(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
        ctx.sql("SELECT * FROM (VALUES (1.0, 2.0), (2.0, 4.0), (3.0, 6.0)) AS t(x, y)")
            .await
            .expect("dataframe")
    }

    #[tokio::test]
    async fn mark_group_flattening_preserves_author_order() {
        let ctx = SessionContext::new();
        let compiled = Plot::<Cartesian>::new()
            .mark(Symbol::new().id("a").x(lit(1.0)).y(lit(1.0)))
            .mark(
                MarkGroup::new()
                    .id("summary")
                    .mark(Symbol::new().id("b").x(lit(2.0)).y(lit(2.0)))
                    .mark(Rect::new().id("c").x(lit(3.0)).y(lit(3.0))),
            )
            .mark(TestCompoundMark)
            .compile(&ctx)
            .await
            .expect("compile");

        let ids = compiled
            .marks
            .iter()
            .map(|mark| mark.state().id.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                Some("a".to_string()),
                Some("b".to_string()),
                Some("c".to_string()),
                Some("compound_symbol".to_string()),
                Some("compound_rect".to_string()),
            ]
        );
        assert_eq!(compiled.mark_groups.len(), 1);
        assert_eq!(
            compiled.mark_group_index_by_mark,
            vec![None, Some(0), Some(0), None, None]
        );
        let public_paths = compiled
            .marks
            .iter()
            .map(|mark| mark.state().public_target_path.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            public_paths,
            vec![
                Some("a".to_string()),
                Some("summary.b".to_string()),
                Some("summary.c".to_string()),
                Some("compound_symbol".to_string()),
                Some("compound_rect".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn nested_mark_group_records_nearest_parent_group() {
        let ctx = SessionContext::new();
        let compiled = Plot::<Cartesian>::new()
            .mark(
                MarkGroup::new().id("outer").mark(
                    MarkGroup::new()
                        .id("inner")
                        .mark(Symbol::new().id("leaf").x(lit(1.0)).y(lit(1.0))),
                ),
            )
            .compile(&ctx)
            .await
            .expect("compile");

        assert_eq!(compiled.mark_groups.len(), 2);
        assert_eq!(compiled.mark_groups[0].parent_group_index, None);
        assert_eq!(compiled.mark_groups[1].parent_group_index, Some(0));
        assert_eq!(compiled.mark_group_index_by_mark, vec![Some(1)]);
        assert_eq!(
            compiled.marks[0].state().public_target_path.as_deref(),
            Some("outer.inner.leaf")
        );
    }

    #[tokio::test]
    async fn mark_group_event_targets_resolve_to_public_paths() {
        let ctx = SessionContext::new();
        let compiled = Plot::<Cartesian>::new()
            .mark(
                MarkGroup::new()
                    .id("manual_box_plot")
                    .mark(Symbol::new().id("box").x(lit(1.0)).y(lit(1.0)))
                    .mark(Symbol::new().id("outliers").x(lit(2.0)).y(lit(2.0))),
            )
            .event_binding(ChartEventBinding::on_between_end(
                ChartEventStream::on(ChartEventType::MouseDown).mark("manual_box_plot.outliers"),
                ChartEventStream::on(ChartEventType::MouseUp),
            ))
            .compile(&ctx)
            .await
            .expect("compile");

        let binding = compiled.event_bindings().first().expect("event binding");
        let between = binding.between.as_ref().expect("between binding");
        assert_eq!(
            between.start.resolved_mark_paths(),
            Some(&[vec![1usize]][..])
        );
        assert_eq!(
            compiled.marks[1].state().public_target_path.as_deref(),
            Some("manual_box_plot.outliers")
        );
    }

    #[tokio::test]
    async fn root_primitive_event_target_resolves_to_mark_path() {
        let ctx = SessionContext::new();
        let compiled = Plot::<Cartesian>::new()
            .mark(Symbol::new().id("points").x(lit(1.0)).y(lit(1.0)))
            .event_binding(ChartEventBinding::on_between_end(
                ChartEventStream::on(ChartEventType::MouseDown).mark("points"),
                ChartEventStream::on(ChartEventType::MouseUp),
            ))
            .compile(&ctx)
            .await
            .expect("compile");

        let binding = compiled.event_bindings().first().expect("event binding");
        let between = binding.between.as_ref().expect("between binding");
        assert_eq!(
            between.start.resolved_mark_paths(),
            Some(&[vec![0usize]][..])
        );
    }

    #[tokio::test]
    async fn mark_group_root_target_resolves_to_descendants() {
        let ctx = SessionContext::new();
        let compiled = Plot::<Cartesian>::new()
            .mark(
                MarkGroup::new()
                    .id("manual_box_plot")
                    .mark(Symbol::new().id("box").x(lit(1.0)).y(lit(1.0)))
                    .mark(Symbol::new().id("outliers").x(lit(2.0)).y(lit(2.0))),
            )
            .event_binding(ChartEventBinding::on_between_end(
                ChartEventStream::on(ChartEventType::MouseDown).mark("manual_box_plot"),
                ChartEventStream::on(ChartEventType::MouseUp),
            ))
            .compile(&ctx)
            .await
            .expect("compile");

        let binding = compiled.event_bindings().first().expect("event binding");
        let between = binding.between.as_ref().expect("between binding");
        assert_eq!(
            between.start.resolved_mark_paths(),
            Some(&[vec![0usize], vec![1usize]][..])
        );
    }

    #[tokio::test]
    async fn nested_local_mark_target_without_root_group_is_invalid() {
        let ctx = SessionContext::new();
        let err = match Plot::<Cartesian>::new()
            .mark(MarkGroup::new().mark(Symbol::new().id("outliers").x(lit(1.0)).y(lit(1.0))))
            .event_binding(ChartEventBinding::on_between_end(
                ChartEventStream::on(ChartEventType::MouseDown).mark("outliers"),
                ChartEventStream::on(ChartEventType::MouseUp),
            ))
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("unrooted nested target should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("Unknown mark target 'outliers'"));
    }

    #[tokio::test]
    async fn same_local_child_ids_under_different_roots_are_valid_targets() {
        let ctx = SessionContext::new();
        let compiled = Plot::<Cartesian>::new()
            .mark(
                MarkGroup::new()
                    .id("a")
                    .mark(Symbol::new().id("outliers").x(lit(1.0)).y(lit(1.0))),
            )
            .mark(
                MarkGroup::new()
                    .id("b")
                    .mark(Symbol::new().id("outliers").x(lit(2.0)).y(lit(2.0))),
            )
            .event_binding(ChartEventBinding::on_between_end(
                ChartEventStream::on(ChartEventType::MouseDown).marks(["a.outliers", "b.outliers"]),
                ChartEventStream::on(ChartEventType::MouseUp),
            ))
            .compile(&ctx)
            .await
            .expect("compile");

        let binding = compiled.event_bindings().first().expect("event binding");
        let between = binding.between.as_ref().expect("between binding");
        assert_eq!(
            between.start.resolved_mark_paths(),
            Some(&[vec![0usize], vec![1usize]][..])
        );
        assert_eq!(
            compiled
                .marks
                .iter()
                .map(|mark| mark.state().public_target_path.clone())
                .collect::<Vec<_>>(),
            vec![
                Some("a.outliers".to_string()),
                Some("b.outliers".to_string())
            ]
        );
    }

    #[tokio::test]
    async fn duplicate_child_target_paths_under_same_root_error() {
        let ctx = SessionContext::new();
        let err = match Plot::<Cartesian>::new()
            .mark(
                MarkGroup::new()
                    .id("a")
                    .mark(Symbol::new().id("outliers").x(lit(1.0)).y(lit(1.0)))
                    .mark(Symbol::new().id("outliers").x(lit(2.0)).y(lit(2.0))),
            )
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("duplicate nested public target should fail"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("Duplicate mark target path 'a.outliers'"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn scene_query_mark_targets_resolve_to_public_paths() {
        let ctx = SessionContext::new();
        let compiled = Plot::<Cartesian>::new()
            .mark(
                MarkGroup::new()
                    .id("manual_box_plot")
                    .mark(Symbol::new().id("box").x(lit(1.0)).y(lit(1.0)))
                    .mark(Symbol::new().id("outliers").x(lit(2.0)).y(lit(2.0))),
            )
            .add_selection(Selection::new("picked"))
            .event_binding(
                ChartEventBinding::on(ChartEventType::Click).set_selection(
                    "picked",
                    SelectionUpdate::replace_all_from_scene_query(SelectionSceneQuery::new(
                        SceneGeometryQuery::rect(lit(0.0), lit(0.0), lit(10.0), lit(10.0))
                            .mark("manual_box_plot.outliers"),
                    )),
                ),
            )
            .compile(&ctx)
            .await
            .expect("compile");

        let binding = compiled.event_bindings().first().expect("event binding");
        let SelectionUpdate::ReplaceAllFromSceneQuery { query } =
            &binding.selection_assignments[0].update
        else {
            panic!("expected scene query update");
        };
        assert_eq!(
            query.query.target.resolved_mark_paths(),
            Some(&[vec![1usize]][..])
        );
    }

    #[tokio::test]
    async fn empty_mark_group_is_invalid() {
        let ctx = SessionContext::new();
        let err = match Plot::<Cartesian>::new()
            .mark(MarkGroup::new())
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("empty group should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("MarkGroup must contain"), "{err}");
    }

    #[tokio::test]
    async fn primitive_group_child_cannot_set_explicit_data() {
        let ctx = SessionContext::new();
        let data = xy_dataframe(&ctx).await;
        let err = match Plot::<Cartesian>::new()
            .mark(MarkGroup::new().mark(Symbol::new().data(data).x(col("x")).y(col("y"))))
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("explicit child data should fail"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("Primitive child marks inside MarkGroup may not set explicit data"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn flattened_group_child_ids_must_be_unique() {
        let ctx = SessionContext::new();
        let err = match Plot::<Cartesian>::new()
            .mark(Symbol::new().id("duplicate").x(lit(1.0)).y(lit(1.0)))
            .mark(MarkGroup::new().mark(Symbol::new().id("duplicate").x(lit(2.0)).y(lit(2.0))))
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("duplicate ids should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("Duplicate mark id"), "{err}");
    }

    #[tokio::test]
    async fn two_marks_in_one_group_share_transform_preparation() {
        let _guard = COUNTING_GROUP_TRANSFORM_TEST_LOCK.lock().await;
        let ctx = SessionContext::new();
        let data = xy_dataframe(&ctx).await;
        COUNTING_GROUP_TRANSFORM_APPLIES.store(0, Ordering::SeqCst);
        let compiled = Plot::<Cartesian>::new()
            .data(data.clone())
            .mark(
                MarkGroup::new().transform_no_output(CountingGroupTransform, |group| {
                    group
                        .mark(Symbol::new().x(col("x")).y(col("y")))
                        .mark(Symbol::new().x(col("x")).y(col("y")))
                }),
            )
            .compile(&ctx)
            .await
            .expect("compile");

        let _scales = compiled
            .build_scales_for_dataframe(&data, 320.0, 240.0, &ctx, compiled.get_default_params())
            .await
            .expect("build scales");
        assert_eq!(COUNTING_GROUP_TRANSFORM_APPLIES.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn sibling_groups_prepare_independent_transform_branches() {
        let _guard = COUNTING_GROUP_TRANSFORM_TEST_LOCK.lock().await;
        let ctx = SessionContext::new();
        let data = xy_dataframe(&ctx).await;
        COUNTING_GROUP_TRANSFORM_APPLIES.store(0, Ordering::SeqCst);
        let branch = || {
            MarkGroup::<Cartesian>::new().transform_no_output(CountingGroupTransform, |group| {
                group.mark(Symbol::new().x(col("x")).y(col("y")))
            })
        };
        let compiled = Plot::<Cartesian>::new()
            .data(data.clone())
            .mark(branch())
            .mark(branch())
            .compile(&ctx)
            .await
            .expect("compile");

        let _scales = compiled
            .build_scales_for_dataframe(&data, 320.0, 240.0, &ctx, compiled.get_default_params())
            .await
            .expect("build scales");
        assert_eq!(COUNTING_GROUP_TRANSFORM_APPLIES.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn data_transparent_groups_do_not_shadow_child_mark_data() {
        let ctx = SessionContext::new();
        let compiled = Plot::<Cartesian>::new()
            .mark(
                MarkGroup::new()
                    .id("transparent")
                    .scale_inference_hint(ScaleInferenceHint::new("x", ScaleTypePreference::Point))
                    .mark(Symbol::new().id("leaf").x(col("x")).y(col("y"))),
            )
            .compile(&ctx)
            .await
            .expect("compile");

        assert_eq!(compiled.mark_group_index_for_mark(0), Some(0));
        assert_eq!(compiled.data_group_index_for_mark(0), None);
        assert_eq!(
            compiled.scale_inference_hints_for_mark(0).expect("hints"),
            vec![ScaleInferenceHint::new("x", ScaleTypePreference::Point)]
        );
        assert_eq!(
            compiled.marks[0].state().public_target_path.as_deref(),
            Some("transparent.leaf")
        );

        let data = xy_dataframe(&ctx).await;
        let _scales = compiled
            .build_scales_for_dataframe(&data, 320.0, 240.0, &ctx, compiled.get_default_params())
            .await
            .expect("transparent group does not hide plot data");
    }

    #[tokio::test]
    async fn data_transparent_nested_group_uses_parent_data_group() {
        let _guard = COUNTING_GROUP_TRANSFORM_TEST_LOCK.lock().await;
        let ctx = SessionContext::new();
        let data = xy_dataframe(&ctx).await;
        let compiled = Plot::<Cartesian>::new()
            .data(data.clone())
            .mark(
                MarkGroup::new().transform_no_output(CountingGroupTransform, |group| {
                    group.mark(
                        MarkGroup::new()
                            .id("transparent")
                            .mark(Symbol::new().id("leaf").x(col("x")).y(col("y"))),
                    )
                }),
            )
            .compile(&ctx)
            .await
            .expect("compile");

        assert_eq!(compiled.mark_group_index_for_mark(0), Some(1));
        assert_eq!(compiled.data_group_index_for_mark(0), Some(0));
        let _scales = compiled
            .build_scales_for_dataframe(&data, 320.0, 240.0, &ctx, compiled.get_default_params())
            .await
            .expect("build scales");
    }

    #[tokio::test]
    async fn nested_explicit_group_data_resets_parent_inheritance() {
        let _guard = COUNTING_GROUP_TRANSFORM_TEST_LOCK.lock().await;
        let ctx = SessionContext::new();
        let parent_data = xy_dataframe(&ctx).await;
        let child_data = ctx
            .sql("SELECT * FROM (VALUES (10.0, 20.0), (30.0, 40.0)) AS t(x, y)")
            .await
            .expect("child dataframe");
        COUNTING_GROUP_TRANSFORM_APPLIES.store(0, Ordering::SeqCst);
        let compiled = Plot::<Cartesian>::new()
            .data(parent_data.clone())
            .mark(
                MarkGroup::new().transform_no_output(CountingGroupTransform, |group| {
                    group.mark(
                        MarkGroup::new()
                            .data(child_data)
                            .mark(Symbol::new().x(col("x")).y(col("y"))),
                    )
                }),
            )
            .compile(&ctx)
            .await
            .expect("compile");

        let _scales = compiled
            .build_scales_for_dataframe(
                &parent_data,
                320.0,
                240.0,
                &ctx,
                compiled.get_default_params(),
            )
            .await
            .expect("build scales");
        assert_eq!(COUNTING_GROUP_TRANSFORM_APPLIES.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn compiled_plot_serializes_mark_group_metadata() {
        let ctx = SessionContext::new();
        let compiled = Plot::<Cartesian>::new()
            .mark(
                MarkGroup::new()
                    .id("outer")
                    .scale_inference_hint(ScaleInferenceHint::new("x", ScaleTypePreference::Point))
                    .mark(
                        MarkGroup::new()
                            .id("inner")
                            .scale_inference_hint(ScaleInferenceHint::new(
                                "y",
                                ScaleTypePreference::Band,
                            ))
                            .mark(Symbol::new().id("leaf").x(lit(1.0)).y(lit(1.0))),
                    ),
            )
            .compile(&ctx)
            .await
            .expect("compile");

        assert_eq!(
            compiled.scale_inference_hints_for_mark(0).expect("hints"),
            vec![
                ScaleInferenceHint::new("x", ScaleTypePreference::Point),
                ScaleInferenceHint::new("y", ScaleTypePreference::Band),
            ]
        );
        let bytes = bincode::serialize(&compiled).expect("serialize");
        let restored: CompiledPlot = bincode::deserialize(&bytes).expect("deserialize");
        assert_eq!(restored.mark_groups.len(), 2);
        assert_eq!(restored.mark_groups[0].id.as_deref(), Some("outer"));
        assert_eq!(
            restored.mark_groups[0].scale_inference_hints,
            vec![ScaleInferenceHint::new("x", ScaleTypePreference::Point)]
        );
        assert_eq!(restored.mark_groups[1].id.as_deref(), Some("inner"));
        assert_eq!(restored.mark_groups[1].parent_group_index, Some(0));
        assert_eq!(
            restored.mark_groups[1].scale_inference_hints,
            vec![ScaleInferenceHint::new("y", ScaleTypePreference::Band)]
        );
        assert_eq!(restored.mark_group_index_by_mark, vec![Some(1)]);
        assert_eq!(
            restored.scale_inference_hints_for_mark(0).expect("hints"),
            vec![
                ScaleInferenceHint::new("x", ScaleTypePreference::Point),
                ScaleInferenceHint::new("y", ScaleTypePreference::Band),
            ]
        );
    }

    #[tokio::test]
    async fn compiled_plot_serializes_single_mark_group_metadata() {
        let ctx = SessionContext::new();
        let compiled = Plot::<Cartesian>::new()
            .mark(
                MarkGroup::new()
                    .id("group")
                    .scale_inference_hint(ScaleInferenceHint::new("y", ScaleTypePreference::Band))
                    .mark(Symbol::new().id("leaf").x(lit(1.0)).y(lit(1.0))),
            )
            .compile(&ctx)
            .await
            .expect("compile");

        assert_eq!(
            compiled.scale_inference_hints_for_mark(0).expect("hints"),
            vec![ScaleInferenceHint::new("y", ScaleTypePreference::Band)]
        );
        let bytes = bincode::serialize(&compiled).expect("serialize");
        let restored: CompiledPlot = bincode::deserialize(&bytes).expect("deserialize");
        assert_eq!(restored.mark_groups.len(), 1);
        assert_eq!(restored.mark_groups[0].id.as_deref(), Some("group"));
        assert_eq!(
            restored.mark_groups[0].scale_inference_hints,
            vec![ScaleInferenceHint::new("y", ScaleTypePreference::Band)]
        );
        assert_eq!(restored.mark_group_index_by_mark, vec![Some(0)]);
    }

    fn repeated_item_cell() -> Plot<Cartesian> {
        Plot::<Cartesian>::new().mark(Symbol::new().x(lit(1.0)).y(repeat::item()).size(64.0))
    }

    fn zerod_branch_cell() -> Plot<ZeroDCoord> {
        Plot::<ZeroDCoord>::new().mark(Symbol::new().fill("#2f7ed8").size(64.0))
    }

    fn repeat_histogram_domain_data(ctx: &SessionContext) -> DataFrame {
        let schema = Arc::new(Schema::new(vec![
            Field::new("a", DataType::Float64, false),
            Field::new("b", DataType::Float64, false),
            Field::new("c", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0])),
                Arc::new(Float64Array::from(vec![0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0])),
                Arc::new(Float64Array::from(vec![2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 3.0])),
            ],
        )
        .expect("repeat histogram domain batch");
        ctx.read_batch(batch).expect("repeat histogram domain df")
    }

    fn child_channel_expr(
        subplot: &crate::concat::CompiledConcatSubplot,
        channel: &str,
        ctx: &SessionContext,
    ) -> String {
        subplot.compiled_subplot().marks[0]
            .data_context()
            .channels()
            .get(channel)
            .expect("child channel")
            .expr(ctx)
            .expect("child expr")
            .to_string()
    }

    fn child_channel_domain_coordination(
        subplot: &crate::concat::CompiledConcatSubplot,
        channel: &str,
    ) -> Option<DomainCoordination> {
        subplot.compiled_subplot().marks[0]
            .data_context()
            .channels()
            .get(channel)
            .expect("child channel")
            .get_domain_coordination()
            .cloned()
    }

    fn child_axis_title(
        subplot: &crate::concat::CompiledConcatSubplot,
        channel: &str,
        ctx: &SessionContext,
    ) -> Option<String> {
        let AxisSpec::Local(axis) = subplot.compiled_subplot().axis_specs.get(channel)?;
        let axis = axis.as_any().downcast_ref::<CartesianAxis>()?;
        let title = axis.title.as_option()?.as_ref()?;
        let scalar = simplify_to_scalar_sync(title.to_default_expr(ctx).ok()?).ok()?;
        match scalar {
            ScalarValue::Utf8(Some(value)) => Some(value),
            other => Some(other.to_string()),
        }
    }

    fn lowered_children<'a>(
        compiled: &'a CompiledPlot,
    ) -> Vec<&'a crate::concat::CompiledConcatSubplot> {
        compiled
            .marks
            .iter()
            .map(|mark| compiled_subplot(mark.as_ref()).expect("lowered concat subplot"))
            .collect()
    }

    async fn compile_with_repeat_context(
        plot: Plot<Cartesian>,
        ctx: &SessionContext,
        repeat_context: RepeatContext,
    ) -> CompiledPlot {
        let tool_context =
            ToolCompileContext::root(TimeContext::default()).with_repeat_context(repeat_context);
        plot.compile_with_tool_context(ctx, Some(&tool_context), true)
            .await
            .expect("plot compiles with repeat context")
    }

    #[tokio::test]
    async fn repeat_columns_lower_to_hconcat_children() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<RepeatColumns>::new()
            .columns(repeat_vars(&["a", "b"]))
            .cell(repeated_column_cell())
            .compile(&ctx)
            .await?;

        assert!(compiled.coord_transform.as_any().is::<HConcat>());
        let children = lowered_children(&compiled);
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].key(), Some("repeat_col:a"));
        assert_eq!(children[1].key(), Some("repeat_col:b"));
        assert_eq!(children[0].label(), Some("Title a"));
        assert_eq!(children[1].label(), Some("Title b"));
        assert_eq!(
            children[0].compiled_state().id.as_deref(),
            Some("repeat_col_a")
        );
        assert_eq!(
            children[1].compiled_state().id.as_deref(),
            Some("repeat_col_b")
        );
        assert_eq!(children[0].data_source(), SubplotDataSource::InheritParent);
        assert_eq!(child_channel_expr(children[0], "x", &ctx), "a");
        assert_eq!(child_channel_expr(children[1], "x", &ctx), "b");
        Ok(())
    }

    #[tokio::test]
    async fn repeat_rows_lower_to_vconcat_children() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<RepeatRows>::new()
            .rows(repeat_vars(&["a", "b"]))
            .cell(repeated_row_cell())
            .compile(&ctx)
            .await?;

        assert!(compiled.coord_transform.as_any().is::<VConcat>());
        let children = lowered_children(&compiled);
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].key(), Some("repeat_row:a"));
        assert_eq!(children[1].key(), Some("repeat_row:b"));
        assert_eq!(child_channel_expr(children[0], "y", &ctx), "a");
        assert_eq!(child_channel_expr(children[1], "y", &ctx), "b");
        Ok(())
    }

    #[tokio::test]
    async fn repeat_grid_lowers_to_grid_concat_with_cell_placement() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let compiled = Plot::<RepeatGrid>::new()
            .rows(repeat_vars(&["r1", "r2"]))
            .columns(repeat_vars(&["c1", "c2", "c3"]))
            .cell(repeated_grid_cell())
            .compile(&ctx)
            .await?;

        assert!(compiled.coord_transform.as_any().is::<GridConcat>());
        let children = lowered_children(&compiled);
        assert_eq!(children.len(), 6);
        let placements = children
            .iter()
            .map(|child| {
                let placement = child.grid_placement().expect("grid placement");
                (placement.row, placement.column)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            placements,
            vec![(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 2)]
        );
        assert_eq!(children[0].key(), Some("repeat_cell:r1:c1"));
        assert_eq!(children[5].key(), Some("repeat_cell:r2:c3"));
        assert_eq!(child_channel_expr(children[0], "x", &ctx), "c1");
        assert_eq!(child_channel_expr(children[0], "y", &ctx), "r1");
        assert_eq!(child_channel_expr(children[5], "x", &ctx), "c3");
        assert_eq!(child_channel_expr(children[5], "y", &ctx), "r2");
        Ok(())
    }

    #[tokio::test]
    async fn repeat_grid_matrix_domains_generate_variable_groups() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let compiled = Plot::<RepeatGrid>::new()
            .rows(repeat_vars(&["a", "b"]))
            .columns(repeat_vars(&["a", "b"]))
            .cell(repeated_grid_cell())
            .matrix_domains()
            .compile(&ctx)
            .await?;

        let children = lowered_children(&compiled);
        let top_left_x = child_channel_domain_coordination(children[0], "x").expect("x domain");
        let top_left_y = child_channel_domain_coordination(children[0], "y").expect("y domain");
        let top_right_x = child_channel_domain_coordination(children[1], "x").expect("x domain");
        assert_eq!(top_left_x.scope, CoordinationScope::Level(u8::MAX));
        assert_eq!(
            top_left_x.group,
            DomainCoordinationGroup::Named("a".to_string())
        );
        assert_eq!(
            top_left_y.group,
            DomainCoordinationGroup::Named("a".to_string())
        );
        assert_eq!(
            top_right_x.group,
            DomainCoordinationGroup::Named("b".to_string())
        );
        Ok(())
    }

    #[tokio::test]
    async fn repeat_grid_matrix_domains_preserve_explicit_scope_and_independent_mode()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let level_compiled = Plot::<RepeatGrid>::new()
            .rows(repeat_vars(&["a"]))
            .columns(repeat_vars(&["b"]))
            .cell(repeated_grid_cell())
            .matrix_domains_with_scope(CoordinationScope::Level(1))
            .compile(&ctx)
            .await?;
        let level_child = lowered_children(&level_compiled);
        let x_coordination =
            child_channel_domain_coordination(level_child[0], "x").expect("x domain");
        assert_eq!(x_coordination.scope, CoordinationScope::Level(1));
        assert_eq!(
            x_coordination.group,
            DomainCoordinationGroup::Named("b".to_string())
        );

        let independent_compiled = Plot::<RepeatGrid>::new()
            .rows(repeat_vars(&["a"]))
            .columns(repeat_vars(&["b"]))
            .cell(repeated_grid_cell())
            .compile(&ctx)
            .await?;
        let independent_child = lowered_children(&independent_compiled);
        assert!(child_channel_domain_coordination(independent_child[0], "x").is_none());
        assert!(child_channel_domain_coordination(independent_child[0], "y").is_none());
        Ok(())
    }

    #[tokio::test]
    async fn repeat_grid_matrix_domains_reject_conflicting_authored_domain_group() {
        let ctx = SessionContext::new();
        let conflicting_cell = Plot::<Cartesian>::new().mark(
            Symbol::new()
                .x_with(repeat::column(), |c| c.with_domain_group("other"))
                .y(repeat::row())
                .size(64.0),
        );
        let err = match Plot::<RepeatGrid>::new()
            .rows(repeat_vars(&["a"]))
            .columns(repeat_vars(&["a"]))
            .cell(conflicting_cell)
            .matrix_domains()
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("conflicting repeat domain group should fail"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("Repeat-generated domain coordination target"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn repeat_grid_conditional_histogram_preserves_count_domain_group()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<RepeatGrid>::new()
            .rows(repeat_vars(&["a", "b"]))
            .columns(repeat_vars(&["a", "b"]))
            .cell(repeated_grid_cell())
            .cell_when(
                repeat::row_index().eq(repeat::column_index()),
                diagonal_histogram_count_shared_cell(),
            )
            .matrix_domains()
            .compile(&ctx)
            .await?;

        let children = lowered_children(&compiled);
        for index in [0, 3] {
            let y = child_channel_domain_coordination(children[index], "y").expect("y domain");
            let y2 = child_channel_domain_coordination(children[index], "y2").expect("y2 domain");
            assert_eq!(y.scope, CoordinationScope::Level(u8::MAX));
            assert_eq!(y2.scope, CoordinationScope::Level(u8::MAX));
            assert_eq!(
                y.group,
                DomainCoordinationGroup::Named("hist_count".to_string())
            );
            assert_eq!(
                y2.group,
                DomainCoordinationGroup::Named("hist_count".to_string())
            );
        }

        Ok(())
    }

    #[tokio::test]
    async fn repeat_grid_conditional_histogram_coordinates_count_domains()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<RepeatGrid>::new()
            .data(repeat_histogram_domain_data(&ctx))
            .plot_size(180.0, 140.0)
            .rows(repeat_vars(&["a", "b", "c"]))
            .columns(repeat_vars(&["a", "b", "c"]))
            .cell(repeated_grid_cell())
            .cell_when(
                repeat::row_index().eq(repeat::column_index()),
                diagonal_histogram_count_shared_cell(),
            )
            .matrix_domains()
            .compile(&ctx)
            .await?;

        let evaluated = compiled.evaluate(&ctx, None).await?;
        let histogram_y_domains = evaluated
            .interaction
            .scopes
            .iter()
            .filter_map(|scope| {
                let child = scope.child_frame_path.last()?;
                let key = child.key.as_deref()?;
                matches!(
                    key,
                    "repeat_cell:a:a" | "repeat_cell:b:b" | "repeat_cell:c:c"
                )
                .then(|| {
                    scope
                        .scales
                        .get("y")
                        .expect("y scale")
                        .numeric_interval_domain()
                        .expect("numeric y domain")
                })
            })
            .collect::<Vec<_>>();

        assert_eq!(histogram_y_domains.len(), 3);
        assert_eq!(histogram_y_domains[0], histogram_y_domains[1]);
        assert_eq!(histogram_y_domains[0], histogram_y_domains[2]);
        assert!(
            histogram_y_domains[0].1 >= 6.0,
            "shared count domain should include the densest histogram bin, got {:?}",
            histogram_y_domains[0]
        );

        Ok(())
    }

    #[tokio::test]
    async fn repeat_wrap_item_domains_generate_item_groups() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<RepeatWrap>::new()
            .items(repeat_vars(&["a", "b"]))
            .columns(2)
            .cell(repeated_item_cell())
            .item_domains_with_scope(CoordinationScope::Level(1))
            .compile(&ctx)
            .await?;

        let children = lowered_children(&compiled);
        let first_y = child_channel_domain_coordination(children[0], "y").expect("y domain");
        let second_y = child_channel_domain_coordination(children[1], "y").expect("y domain");
        assert_eq!(first_y.scope, CoordinationScope::Level(1));
        assert_eq!(
            first_y.group,
            DomainCoordinationGroup::Named("a".to_string())
        );
        assert_eq!(
            second_y.group,
            DomainCoordinationGroup::Named("b".to_string())
        );
        Ok(())
    }

    #[tokio::test]
    async fn repeat_grid_matrix_domains_match_manual_grid_concat() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let repeat_compiled = Plot::<RepeatGrid>::new()
            .rows(repeat_vars(&["a", "b"]))
            .columns(repeat_vars(&["a", "b"]))
            .cell(repeated_grid_cell())
            .matrix_domains()
            .compile(&ctx)
            .await?;

        let manual_cell = |x: &'static str, y: &'static str| {
            Plot::<Cartesian>::new().mark(
                Symbol::new()
                    .x_with(col(x), move |c| c.with_domain_group(x).share_domain())
                    .y_with(col(y), move |c| c.with_domain_group(y).share_domain())
                    .size(64.0),
            )
        };
        let manual_compiled = Plot::<GridConcat>::new()
            .rows(2)
            .columns(2)
            .mark(Subplot::new(manual_cell("a", "a")).grid_cell(0, 0))
            .mark(Subplot::new(manual_cell("b", "a")).grid_cell(0, 1))
            .mark(Subplot::new(manual_cell("a", "b")).grid_cell(1, 0))
            .mark(Subplot::new(manual_cell("b", "b")).grid_cell(1, 1))
            .compile(&ctx)
            .await?;

        let repeat_children = lowered_children(&repeat_compiled);
        let manual_children = lowered_children(&manual_compiled);
        let repeat_targets = repeat_children
            .iter()
            .map(|child| {
                (
                    child_channel_domain_coordination(child, "x"),
                    child_channel_domain_coordination(child, "y"),
                )
            })
            .collect::<Vec<_>>();
        let manual_targets = manual_children
            .iter()
            .map(|child| {
                (
                    child_channel_domain_coordination(child, "x"),
                    child_channel_domain_coordination(child, "y"),
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(repeat_targets, manual_targets);
        Ok(())
    }

    #[tokio::test]
    async fn repeat_grid_matrix_domains_are_visible_to_pan_scroll_zoom()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let repeat_context = RepeatContext::new()
            .with_row(resolved_repeat("a"), 0, 2)
            .with_column(resolved_repeat("b"), 1, 2)
            .with_domain_coordination(RepeatDomainCoordination::by_variable(
                CoordinationScope::Shared,
            ));
        let compiled = compile_with_repeat_context(
            repeated_grid_cell().tool(PanScrollZoom::cartesian()),
            &ctx,
            repeat_context,
        )
        .await;

        assert!(
            compiled
                .param_specs()
                .contains_key("__tool_pan_scroll_zoom__domain__a")
        );
        assert!(
            compiled
                .param_specs()
                .contains_key("__tool_pan_scroll_zoom__domain__b")
        );
        assert!(
            !compiled
                .param_specs()
                .contains_key("__tool_pan_scroll_zoom__x_domain")
        );
        assert!(
            !compiled
                .param_specs()
                .contains_key("__tool_pan_scroll_zoom__y_domain")
        );
        assert_eq!(
            compiled.param_specs()["__tool_pan_scroll_zoom__domain__a"].sharing,
            CoordinationScope::Level(u8::MAX)
        );
        assert_eq!(
            compiled.param_specs()["__tool_pan_scroll_zoom__domain__b"].sharing,
            CoordinationScope::Level(u8::MAX)
        );
        Ok(())
    }

    #[tokio::test]
    async fn repeat_grid_pan_scroll_zoom_expands_across_cells() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<RepeatGrid>::new()
            .rows(repeat_vars(&["a", "b"]))
            .columns(repeat_vars(&["a", "b"]))
            .cell(repeated_grid_cell().tool(PanScrollZoom::cartesian()))
            .matrix_domains()
            .compile(&ctx)
            .await?;

        let tool_param_names = compiled
            .param_specs()
            .keys()
            .filter(|name| name.starts_with("__tool_pan_scroll_zoom__"))
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            tool_param_names,
            vec![
                "__tool_pan_scroll_zoom__enabled".to_string(),
                "__tool_pan_scroll_zoom__domain__a".to_string(),
                "__tool_pan_scroll_zoom__domain__b".to_string(),
            ]
        );
        assert_eq!(compiled.tool_metadata().len(), 1);

        let drag_bindings = compiled
            .event_bindings()
            .iter()
            .filter(|binding| binding.event_type == ChartEventType::CursorMoved)
            .collect::<Vec<_>>();
        assert_eq!(drag_bindings.len(), 4);
        let mut targets = drag_bindings
            .iter()
            .map(|binding| {
                binding
                    .scope_target
                    .as_ref()
                    .and_then(|target| target.resolved_coord_node_path_prefix())
                    .map(|target| target.to_vec())
            })
            .collect::<Vec<_>>();
        targets.sort();
        assert_eq!(
            targets,
            vec![Some(vec![0]), Some(vec![1]), Some(vec![2]), Some(vec![3]),]
        );
        assert!(drag_bindings.iter().any(|binding| {
            binding
                .assignments
                .iter()
                .any(|assignment| assignment.param_name == "__tool_pan_scroll_zoom__domain__a")
                && binding
                    .assignments
                    .iter()
                    .any(|assignment| assignment.param_name == "__tool_pan_scroll_zoom__domain__b")
        }));

        assert_eq!(
            compiled
                .event_bindings()
                .iter()
                .filter(|binding| binding.event_type == ChartEventType::MouseWheel)
                .count(),
            4
        );
        assert_eq!(
            compiled
                .event_bindings()
                .iter()
                .filter(|binding| binding.event_type == ChartEventType::DoubleClick)
                .count(),
            4
        );
        Ok(())
    }

    #[tokio::test]
    async fn repeat_grid_free_pan_scroll_zoom_uses_cell_specific_params()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<RepeatGrid>::new()
            .rows(repeat_vars(&["a", "b"]))
            .columns(repeat_vars(&["a", "b"]))
            .cell(repeated_grid_cell().tool(PanScrollZoom::cartesian()))
            .matrix_domains_with_scope(CoordinationScope::Free)
            .compile(&ctx)
            .await?;

        let tool_param_names = compiled
            .param_specs()
            .keys()
            .filter(|name| name.starts_with("__tool_pan_scroll_zoom__"))
            .cloned()
            .collect::<Vec<_>>();
        assert!(
            tool_param_names
                .iter()
                .any(|name| name == "__tool_pan_scroll_zoom__domain__a__repeat_cell_b_a"),
            "expected a cell-specific a-domain param, got {tool_param_names:?}"
        );
        assert!(
            tool_param_names
                .iter()
                .any(|name| name == "__tool_pan_scroll_zoom__domain__a__repeat_cell_a_b"),
            "expected a distinct cross-orientation a-domain param, got {tool_param_names:?}"
        );
        assert!(
            !tool_param_names
                .iter()
                .any(|name| name == "__tool_pan_scroll_zoom__domain__a"),
            "free repeat should not use a shared a-domain param: {tool_param_names:?}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn repeat_grid_matrix_axes_generate_title_defaults_and_policy()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<RepeatGrid>::new()
            .rows(repeat_vars(&["a", "b"]))
            .columns(repeat_vars(&["a", "b"]))
            .cell(repeated_grid_cell())
            .matrix_domains()
            .matrix_axes()
            .compile(&ctx)
            .await?;

        let grid = compiled
            .coord_transform
            .as_any()
            .downcast_ref::<GridConcat>()
            .expect("lowered to GridConcat");
        assert_eq!(
            grid.axis_guide_visibility_config().labels,
            AxisGuideVisibilityPolicy::OuterForEquivalentDomainGroups
        );
        assert_eq!(
            grid.axis_guide_visibility_config().title,
            AxisGuideVisibilityPolicy::OuterForEquivalentDomainGroups
        );

        let children = lowered_children(&compiled);
        assert_eq!(
            child_axis_title(children[0], "x", &ctx).as_deref(),
            Some("Title a")
        );
        assert_eq!(
            child_axis_title(children[0], "y", &ctx).as_deref(),
            Some("Title a")
        );
        assert_eq!(
            child_axis_title(children[1], "x", &ctx).as_deref(),
            Some("Title b")
        );
        assert_eq!(
            child_axis_title(children[2], "y", &ctx).as_deref(),
            Some("Title b")
        );
        Ok(())
    }

    #[tokio::test]
    async fn repeat_grid_matrix_axes_preserve_explicit_titles_and_skip_non_repeat_axes()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let explicit_cell = Plot::<Cartesian>::new().mark(
            Symbol::new()
                .x_with(repeat::column(), |c| c.axis(|a| a.title("authored x")))
                .y(lit(1.0))
                .size(64.0),
        );
        let compiled = Plot::<RepeatGrid>::new()
            .rows(repeat_vars(&["a"]))
            .columns(repeat_vars(&["b"]))
            .cell(explicit_cell)
            .matrix_domains()
            .matrix_axes()
            .compile(&ctx)
            .await?;

        let children = lowered_children(&compiled);
        assert_eq!(
            child_axis_title(children[0], "x", &ctx).as_deref(),
            Some("authored x")
        );
        assert_eq!(child_axis_title(children[0], "y", &ctx), None);
        Ok(())
    }

    #[tokio::test]
    async fn repeat_grid_cell_when_selects_diagonal_branch() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<RepeatGrid>::new()
            .rows(repeat_vars(&["a", "b"]))
            .columns(repeat_vars(&["a", "b"]))
            .cell(repeated_grid_cell())
            .cell_when(
                repeat::row_index().eq(repeat::column_index()),
                constant_y_grid_cell(99.0),
            )
            .compile(&ctx)
            .await?;

        let children = lowered_children(&compiled);
        assert_eq!(children.len(), 4);
        assert_eq!(children[0].key(), Some("repeat_cell:a:a"));
        assert_eq!(children[1].key(), Some("repeat_cell:a:b"));
        assert_eq!(children[3].key(), Some("repeat_cell:b:b"));
        assert!(
            child_channel_expr(children[0], "y", &ctx).contains("99"),
            "diagonal cell should use branch template"
        );
        assert_eq!(child_channel_expr(children[1], "y", &ctx), "a");
        assert_eq!(child_channel_expr(children[2], "y", &ctx), "b");
        assert!(
            child_channel_expr(children[3], "y", &ctx).contains("99"),
            "diagonal cell should use branch template"
        );
        Ok(())
    }

    #[tokio::test]
    async fn repeat_cell_when_uses_author_order_priority() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<RepeatGrid>::new()
            .rows(repeat_vars(&["a", "b"]))
            .columns(repeat_vars(&["a", "b"]))
            .cell(repeated_grid_cell())
            .cell_when(
                repeat::row_index().eq(repeat::column_index()),
                constant_y_grid_cell(10.0),
            )
            .cell_when(lit(true), constant_y_grid_cell(20.0))
            .compile(&ctx)
            .await?;

        let children = lowered_children(&compiled);
        assert!(
            child_channel_expr(children[0], "y", &ctx).contains("10"),
            "first matching branch should win on diagonal cells"
        );
        assert!(
            child_channel_expr(children[1], "y", &ctx).contains("20"),
            "later broad branch should handle off-diagonal cells"
        );
        assert!(
            child_channel_expr(children[3], "y", &ctx).contains("10"),
            "first matching branch should win on diagonal cells"
        );
        Ok(())
    }

    #[tokio::test]
    async fn repeat_cell_when_allows_heterogeneous_child_plot_specs()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<RepeatGrid>::new()
            .rows(repeat_vars(&["a"]))
            .columns(repeat_vars(&["a"]))
            .cell(repeated_grid_cell())
            .cell_when(
                repeat::row_index().eq(repeat::column_index()),
                zerod_branch_cell(),
            )
            .compile(&ctx)
            .await?;

        let children = lowered_children(&compiled);
        assert_eq!(children.len(), 1);
        assert!(
            children[0]
                .compiled_subplot()
                .coord_transform
                .as_any()
                .is::<ZeroDCoord>(),
            "diagonal branch should compile as the selected ZeroD child plot"
        );
        Ok(())
    }

    #[tokio::test]
    async fn repeat_wrap_lowers_to_wrap_concat_with_item_context() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let compiled = Plot::<RepeatWrap>::new()
            .items(repeat_vars(&["a", "b", "c"]))
            .columns(2)
            .cell(repeated_item_cell())
            .compile(&ctx)
            .await?;

        assert!(compiled.coord_transform.as_any().is::<WrapConcat>());
        let children = lowered_children(&compiled);
        assert_eq!(children.len(), 3);
        assert_eq!(children[0].key(), Some("repeat_item:a"));
        assert_eq!(children[2].key(), Some("repeat_item:c"));
        assert_eq!(children[0].label(), Some("Title a"));
        assert_eq!(children[2].label(), Some("Title c"));
        assert_eq!(child_channel_expr(children[0], "y", &ctx), "a");
        assert_eq!(child_channel_expr(children[2], "y", &ctx), "c");
        Ok(())
    }

    #[tokio::test]
    async fn repeat_generated_cells_preserve_event_datum_translation()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let schema = Arc::new(Schema::new(vec![
            Field::new("a", DataType::Float64, false),
            Field::new("b", DataType::Float64, false),
            Field::new("group_name", DataType::Utf8, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0])),
                Arc::new(Float64Array::from(vec![3.0, 4.0])),
                Arc::new(datafusion::arrow::array::StringArray::from(vec![
                    "Alpha", "Beta",
                ])),
            ],
        )
        .expect("record batch");
        let df = ctx.read_batch(batch).expect("dataframe");

        let compiled = Plot::<RepeatColumns>::new()
            .data(df)
            .columns(vec![
                RepeatVariable::new("a", col("a")),
                RepeatVariable::new("b", col("b")),
            ])
            .cell(repeated_column_cell())
            .event_binding(
                ChartEventBinding::on(ChartEventType::Click)
                    .filter(crate::event::datum("group_name").is_not_null()),
            )
            .compile(&ctx)
            .await?;

        let evaluated = compiled.evaluate(&ctx, None).await?;
        assert_eq!(evaluated.event_datums.rows.len(), 2);
        let subplot_ids = evaluated
            .event_datums
            .rows
            .iter()
            .map(|rows| rows.subplot_id_path.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            subplot_ids,
            vec![
                vec!["repeat_col_a".to_string()],
                vec!["repeat_col_b".to_string()]
            ]
        );
        for rows in &evaluated.event_datums.rows {
            assert_eq!(rows.rows.num_rows(), 2);
            assert!(rows.rows.column_by_name("group_name").is_some());
        }
        Ok(())
    }

    #[tokio::test]
    async fn parallel_axis_title_event_datums_support_scene_query_fields()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let data = ctx.sql("SELECT 50.0 AS speed, 100.0 AS cost").await?;
        let compiled = Plot::with_coord(
            Parallel::new()
                .dimension_with("speed", |d| d.axis(|a| a.title("Speed")))
                .dimension_with("cost", |d| d.axis(|a| a.title("Cost"))),
        )
        .plot_size(120.0, 100.0)
        .data(data)
        .mark(
            ParallelLine::new()
                .dimension("speed", col("speed"))
                .dimension("cost", col("cost")),
        )
        .event_binding(
            ChartEventBinding::on(ChartEventType::Click)
                .filter(parallel_surface_kind().eq(lit(PARALLEL_SURFACE_KIND_DIMENSION_TITLE)))
                .filter(parallel_dimension_id().is_not_null())
                .filter(parallel_title().is_not_null()),
        )
        .compile(&ctx)
        .await?;

        let event_datum_types = compiled.event_datum_types();
        assert_eq!(
            event_datum_types.get(PARALLEL_SURFACE_KIND_FIELD),
            Some(&DataType::Utf8)
        );
        assert_eq!(
            event_datum_types.get(PARALLEL_DIMENSION_ID_FIELD),
            Some(&DataType::Utf8)
        );
        assert_eq!(
            event_datum_types.get(PARALLEL_TITLE_FIELD),
            Some(&DataType::Utf8)
        );

        let evaluated = compiled.evaluate(&ctx, None).await?;
        let axis_title_rows = evaluated
            .event_datums
            .rows
            .iter()
            .find(|rows| {
                rows.rows
                    .column_by_name(PARALLEL_DIMENSION_ID_FIELD)
                    .is_some()
            })
            .expect("parallel axis title event datum rows");
        let queried = evaluated.event_datums.datums_for_mark_instances(
            [MarkInstance {
                name: "parallel_axis_title_hit".to_string(),
                mark_path: axis_title_rows.mark_path.clone(),
                instance_index: Some(0),
            }],
            &[
                SceneQueryDatumField::new("dimension").datum(PARALLEL_DIMENSION_ID_FIELD),
                SceneQueryDatumField::new("title").datum(PARALLEL_TITLE_FIELD),
            ],
            &[],
        )?;

        assert_eq!(queried.num_rows(), 1);
        let dimension =
            ScalarValue::try_from_array(queried.column_by_name("dimension").unwrap(), 0)?;
        let title = ScalarValue::try_from_array(queried.column_by_name("title").unwrap(), 0)?;
        assert_eq!(dimension, ScalarValue::Utf8(Some("speed".to_string())));
        assert_eq!(title, ScalarValue::Utf8(Some("Speed".to_string())));
        Ok(())
    }

    #[tokio::test]
    async fn parallel_symbol_event_datums_support_source_row_and_dimension_scene_query()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let data = ctx
            .sql(
                "SELECT * FROM (VALUES \
                    ('r0', 10.0, 100.0), \
                    ('r1', 20.0, 200.0) \
                ) AS t(row_id, speed, cost)",
            )
            .await?;
        let compiled = Plot::<Parallel>::new()
            .plot_size(120.0, 100.0)
            .data(data)
            .mark(
                ParallelSymbol::new()
                    .dimension("speed", col("speed"))
                    .dimension("cost", col("cost")),
            )
            .event_binding(
                ChartEventBinding::on(ChartEventType::Click)
                    .filter(crate::event::datum("row_id").is_not_null())
                    .filter(parallel_dimension_id().is_not_null())
                    .filter(parallel_surface_kind().eq(lit(PARALLEL_SURFACE_KIND_POINT))),
            )
            .compile(&ctx)
            .await?;

        let evaluated = compiled.evaluate(&ctx, None).await?;
        let speed_point_rows = evaluated
            .event_datums
            .rows
            .iter()
            .find(|rows| {
                let Some(dimension_column) = rows.rows.column_by_name(PARALLEL_DIMENSION_ID_FIELD)
                else {
                    return false;
                };
                matches!(
                    ScalarValue::try_from_array(dimension_column, 0),
                    Ok(ScalarValue::Utf8(Some(value))) if value == "speed"
                )
            })
            .expect("parallel speed point event datum rows");
        let queried = evaluated.event_datums.datums_for_mark_instances(
            [MarkInstance {
                name: "parallel_symbol".to_string(),
                mark_path: speed_point_rows.mark_path.clone(),
                instance_index: Some(1),
            }],
            &[
                SceneQueryDatumField::new("source_row").datum("row_id"),
                SceneQueryDatumField::new("dimension").datum(PARALLEL_DIMENSION_ID_FIELD),
                SceneQueryDatumField::new("surface").datum(PARALLEL_SURFACE_KIND_FIELD),
            ],
            &[],
        )?;

        assert_eq!(queried.num_rows(), 1);
        let source_row =
            ScalarValue::try_from_array(queried.column_by_name("source_row").unwrap(), 0)?;
        let dimension =
            ScalarValue::try_from_array(queried.column_by_name("dimension").unwrap(), 0)?;
        let surface = ScalarValue::try_from_array(queried.column_by_name("surface").unwrap(), 0)?;
        assert_eq!(source_row, ScalarValue::Utf8(Some("r1".to_string())));
        assert_eq!(dimension, ScalarValue::Utf8(Some("speed".to_string())));
        assert_eq!(
            surface,
            ScalarValue::Utf8(Some(PARALLEL_SURFACE_KIND_POINT.to_string()))
        );
        Ok(())
    }

    #[tokio::test]
    async fn parallel_line_zindex_lifts_selected_branch_above_context()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let data = ctx
            .sql(
                "SELECT * FROM (VALUES \
                    ('row0', 10.0, 30.0), \
                    ('row1', 20.0, 40.0) \
                ) AS t(id, speed, cost)",
            )
            .await?;
        let selected = Selection::new("picked").empty_selects_nothing();
        let selected_predicate = selected.predicate();
        let compiled = Arc::new(
            Plot::<Parallel>::new()
                .plot_size(120.0, 100.0)
                .data(data)
                .add_selection(selected)
                .mark(
                    ParallelLine::new()
                        .id("context_lines")
                        .dimension("speed", col("speed"))
                        .dimension("cost", col("cost"))
                        .stroke("#c4cbd5")
                        .zindex(1),
                )
                .mark(
                    ParallelLine::new()
                        .id("selected_lines")
                        .dimension("speed", col("speed"))
                        .dimension("cost", col("cost"))
                        .transform_no_output(Filter::new(selected_predicate), |mark| mark)
                        .stroke("#2563eb")
                        .zindex(20),
                )
                .compile(ctx.as_ref())
                .await?,
        );
        let mut session = compiled.instantiate(ctx);
        session.apply_selection_patch(vec![SelectionAssignment {
            selection_id: "picked".to_string(),
            update: SelectionStateUpdate::ReplaceAllClauses {
                clauses: vec![SelectionClause {
                    id: "row1".to_string(),
                    scope: ResolvedSelectionClauseScope {
                        sharing: CoordinationScope::Shared,
                        owner_path: Vec::new(),
                    },
                    predicate: SelectionPredicateSpec::Equality {
                        dimensions: vec![SelectionEqualityDimensionValue {
                            id: "id".to_string(),
                            field_expr: LogicalExprNode::from_expr(col("id"))?,
                            value: ScalarValue::Utf8(Some("row1".to_string())),
                        }],
                    },
                    facet_context: Vec::new(),
                }],
            },
        }])?;

        let (evaluated, _metrics) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        let mut lines = Vec::new();
        collect_scene_line_marks(&evaluated.scene_graph.marks, &mut lines);
        let context_lines = lines
            .iter()
            .copied()
            .filter(|line| line.name == "context_lines")
            .collect::<Vec<_>>();
        let selected_lines = lines
            .iter()
            .copied()
            .filter(|line| line.name == "selected_lines")
            .collect::<Vec<_>>();

        assert_eq!(context_lines.len(), 2);
        assert_eq!(selected_lines.len(), 1);
        assert!(context_lines.iter().all(|line| line.zindex == Some(1)));
        assert!(selected_lines.iter().all(|line| line.zindex == Some(20)));
        assert!(
            selected_lines[0].zindex > context_lines[0].zindex,
            "selected-line branch should render above context branch"
        );
        Ok(())
    }

    #[tokio::test]
    async fn repeat_grid_inside_hconcat_exports_prefixed_interaction_metadata()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let df = ctx
            .sql("SELECT * FROM (VALUES (1.0, 10.0), (2.0, 20.0)) AS t(a, b)")
            .await?;
        let repeat = Plot::<RepeatGrid>::new()
            .data(df)
            .rows(repeat_vars(&["a", "b"]))
            .columns(repeat_vars(&["a", "b"]))
            .cell(repeated_grid_cell())
            .matrix_domains()
            .matrix_axes();
        let compiled = Plot::<HConcat>::new()
            .canvas_size(620.0, 320.0)
            .mark(Subplot::new(repeat).key("matrix").id("matrix"))
            .mark(
                Subplot::new(Plot::<ZeroDCoord>::new())
                    .key("summary")
                    .id("summary"),
            )
            .compile(&ctx)
            .await?;

        let evaluated = compiled.evaluate(&ctx, None).await?;
        assert_eq!(evaluated.interaction.scopes.len(), 4);
        let scope = evaluated
            .interaction
            .scopes
            .iter()
            .find(|scope| {
                scope
                    .child_frame_path
                    .last()
                    .and_then(|segment| segment.key.as_deref())
                    == Some("repeat_cell:a:b")
            })
            .expect("repeat a/b scope");
        assert_eq!(
            scope.subplot_id_path,
            vec!["matrix".to_string(), "repeat_cell_a_b".to_string()]
        );
        assert_eq!(
            scope
                .child_frame_path
                .iter()
                .filter_map(|segment| segment.key.as_deref())
                .collect::<Vec<_>>(),
            vec!["matrix", "repeat_cell:a:b"]
        );
        assert_eq!(scope.child_frame_path[1].row, Some(0));
        assert_eq!(scope.child_frame_path[1].column, Some(1));
        assert!(scope.facet_path.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn repeat_grid_inside_facet_exports_facet_and_repeat_metadata()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('G1', 1.0, 10.0),
                    ('G1', 2.0, 20.0),
                    ('G2', 3.0, 30.0),
                    ('G2', 4.0, 40.0)
                ) AS t(group_name, a, b)",
            )
            .await?;
        let repeat = Plot::<RepeatGrid>::new()
            .rows(repeat_vars(&["a", "b"]))
            .columns(repeat_vars(&["a", "b"]))
            .cell(repeated_grid_cell())
            .matrix_domains()
            .matrix_axes();
        let compiled = Plot::<FacetColumn>::new()
            .canvas_size(820.0, 320.0)
            .data(df)
            .mark(Subplot::new(repeat).id("matrix").column(col("group_name")))
            .compile(&ctx)
            .await?;

        let evaluated = compiled.evaluate(&ctx, None).await?;
        assert_eq!(evaluated.interaction.scopes.len(), 8);
        let mut groups = evaluated
            .interaction
            .scopes
            .iter()
            .map(|scope| match scope.facet_path.first() {
                Some(ScalarValue::Utf8(Some(value))) => value.clone(),
                other => panic!("expected facet value, got {other:?}"),
            })
            .collect::<Vec<_>>();
        groups.sort();
        groups.dedup();
        assert_eq!(groups, vec!["G1".to_string(), "G2".to_string()]);

        let g1_ab = evaluated
            .interaction
            .scopes
            .iter()
            .find(|scope| {
                matches!(
                    scope.facet_path.first(),
                    Some(ScalarValue::Utf8(Some(value))) if value == "G1"
                ) && scope
                    .child_frame_path
                    .last()
                    .and_then(|segment| segment.key.as_deref())
                    == Some("repeat_cell:a:b")
            })
            .expect("G1 repeat a/b scope");
        assert_eq!(
            g1_ab.subplot_id_path,
            vec!["matrix".to_string(), "repeat_cell_a_b".to_string()]
        );
        assert_eq!(
            g1_ab.sharing_owner_paths.get(&0),
            Some(&g1_ab.facet_path),
            "Free owner path should remain the outer facet cell"
        );
        assert_eq!(
            g1_ab.sharing_owner_paths.get(&1),
            Some(&Vec::new()),
            "Level(1) owner path should project to the root for a one-level facet"
        );
        Ok(())
    }

    #[tokio::test]
    async fn repeat_wrap_inside_facet_wrap_preserves_logical_facet_scope()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('G1', 1.0, 10.0),
                    ('G2', 2.0, 20.0),
                    ('G3', 3.0, 30.0)
                ) AS t(group_name, a, b)",
            )
            .await?;
        let repeat = Plot::<RepeatWrap>::new()
            .items(repeat_vars(&["a", "b"]))
            .columns(2)
            .cell(repeated_item_cell())
            .item_domains_with_scope(CoordinationScope::Free);
        let compiled = Plot::<FacetWrap>::new()
            .canvas_size(760.0, 420.0)
            .data(df)
            .mark(
                Subplot::new(repeat)
                    .id("wrapped_repeat")
                    .wrap_with(col("group_name"), |c| c.columns(2)),
            )
            .compile(&ctx)
            .await?;

        let evaluated = compiled.evaluate(&ctx, None).await?;
        assert_eq!(evaluated.interaction.scopes.len(), 6);
        for scope in &evaluated.interaction.scopes {
            assert_eq!(
                scope.logical_facet_values.len(),
                1,
                "FacetWrap should contribute one logical facet level"
            );
            assert_eq!(
                scope.subplot_id_path.first().map(String::as_str),
                Some("wrapped_repeat")
            );
            assert!(
                scope
                    .child_frame_path
                    .last()
                    .and_then(|segment| segment.key.as_deref())
                    .is_some_and(|key| key.starts_with("repeat_item:")),
                "scope should carry the repeated item child frame: {scope:?}"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn repeat_container_validates_variables_and_cell_template() {
        let ctx = SessionContext::new();
        let missing_cell = match Plot::<RepeatColumns>::new()
            .columns(repeat_vars(&["a"]))
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("missing repeat cell should fail"),
            Err(err) => err,
        };
        assert!(missing_cell.to_string().contains("requires a default"));

        let duplicate = match Plot::<RepeatColumns>::new()
            .columns(vec![
                RepeatVariable::new("a", col("a")),
                RepeatVariable::new("a", col("b")),
            ])
            .cell(repeated_column_cell())
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("duplicate repeat variables should fail"),
            Err(err) => err,
        };
        assert!(
            duplicate
                .to_string()
                .contains("duplicate repeat columns variable id 'a'")
        );

        let empty = match Plot::<RepeatRows>::new()
            .rows(Vec::<RepeatVariable>::new())
            .cell(repeated_row_cell())
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("empty repeat variables should fail"),
            Err(err) => err,
        };
        assert!(
            empty
                .to_string()
                .contains("requires at least one repeat rows variable")
        );

        let data_dependent_predicate = match Plot::<RepeatGrid>::new()
            .rows(repeat_vars(&["a"]))
            .columns(repeat_vars(&["a"]))
            .cell(repeated_grid_cell())
            .cell_when(
                col("datum_value").eq(lit(1_i64)),
                constant_y_grid_cell(99.0),
            )
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("data-dependent repeat branch predicate should fail"),
            Err(err) => err,
        };
        assert!(
            data_dependent_predicate
                .to_string()
                .contains("branch 0 predicate failed"),
            "{data_dependent_predicate}"
        );
    }

    #[tokio::test]
    async fn mark_id_is_accepted_on_regular_mark() {
        let ctx = SessionContext::new();
        Plot::<Cartesian>::new()
            .mark(Symbol::new().id("points").x(lit(1.0)).y(lit(1.0)))
            .compile(&ctx)
            .await
            .expect("mark id compiles");
    }

    #[tokio::test]
    async fn invalid_mark_id_errors() {
        let ctx = SessionContext::new();
        let err = match Plot::<Cartesian>::new()
            .mark(Symbol::new().id("bad.id").x(lit(1.0)).y(lit(1.0)))
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("invalid mark id should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("Invalid mark id"));
    }

    #[tokio::test]
    async fn duplicate_sibling_mark_ids_error() {
        let ctx = SessionContext::new();
        let err = match Plot::<Cartesian>::new()
            .mark(Symbol::new().id("points").x(lit(1.0)).y(lit(1.0)))
            .mark(Symbol::new().id("points").x(lit(2.0)).y(lit(2.0)))
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("duplicate sibling mark ids should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("Duplicate mark id 'points'"));
    }

    #[tokio::test]
    async fn repeat_context_resolves_mark_channel_placeholders_during_compile() {
        let ctx = SessionContext::new();
        let template = || {
            Plot::<Cartesian>::new().mark(Symbol::new().x(repeat::column()).y(lit(1.0)).size(64.0))
        };

        let compiled_a =
            compile_with_repeat_context(template(), &ctx, column_repeat_context("a", 0, 2)).await;
        let compiled_b =
            compile_with_repeat_context(template(), &ctx, column_repeat_context("b", 1, 2)).await;

        let x_a = compiled_a.marks[0]
            .data_context()
            .channels()
            .get("x")
            .expect("x channel")
            .expr(&ctx)
            .expect("x expr");
        let x_b = compiled_b.marks[0]
            .data_context()
            .channels()
            .get("x")
            .expect("x channel")
            .expr(&ctx)
            .expect("x expr");

        assert_eq!(x_a.to_string(), "a");
        assert_eq!(x_b.to_string(), "b");
        assert!(
            collect_repeat_placeholder_kinds(&x_a)
                .expect("collect")
                .is_empty()
        );
        assert!(
            collect_repeat_placeholder_kinds(&x_b)
                .expect("collect")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn coordinate_axis_configs_merge_into_compiled_axes() {
        let ctx = SessionContext::new();
        let compiled = Plot::with_coord(
            Parallel::new()
                .dimension_with("mpg", |d| d.axis(|axis| axis.title("Miles per gallon"))),
        )
        .data(ctx.sql("SELECT 21.0 AS mpg").await.expect("data"))
        .mark(ParallelLine::new().dimension("mpg", col("mpg")))
        .compile(&ctx)
        .await
        .expect("compile parallel plot");

        assert!(
            compiled
                .axis_specs
                .contains_key(&generated_dimension_channel("mpg")),
            "coordinate-level dimension axis should be merged into compiled axis specs"
        );
    }

    #[tokio::test]
    async fn configured_parallel_dimension_without_mark_binding_errors() {
        let ctx = SessionContext::new();
        let err = match Plot::with_coord(Parallel::new().dimension("mpg"))
            .data(ctx.sql("SELECT 21.0 AS mpg").await.expect("data"))
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("configured dimension without mark binding should fail"),
            Err(err) => err,
        };

        assert!(
            err.to_string().contains("no mark binds data"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn coordinate_axis_configs_round_trip_with_compiled_plot() {
        let ctx = SessionContext::new();
        let compiled = Plot::with_coord(
            Parallel::new()
                .dimension_with("mpg", |d| d.axis(|axis| axis.title("Miles per gallon"))),
        )
        .data(ctx.sql("SELECT 21.0 AS mpg").await.expect("data"))
        .mark(ParallelLine::new().dimension("mpg", col("mpg")))
        .compile(&ctx)
        .await
        .expect("compile parallel plot");

        let encoded = bincode::serialize(&compiled).expect("serialize compiled plot");
        let decoded: crate::plot::compiled::CompiledPlot =
            bincode::deserialize(&encoded).expect("deserialize compiled plot");
        assert!(
            decoded
                .axis_specs
                .contains_key(&generated_dimension_channel("mpg")),
            "coordinate-level axis configs should survive compiled plot serialization"
        );
    }

    #[tokio::test]
    async fn repeat_context_resolves_transform_inputs_during_evaluate() {
        let ctx = SessionContext::new();
        let schema = Arc::new(Schema::new(vec![
            Field::new("a", DataType::Float64, false),
            Field::new("b", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0])),
            ],
        )
        .expect("record batch");
        let df = ctx.read_batch(batch).expect("dataframe");

        let template = || {
            Plot::<Cartesian>::new()
                .data(df.clone())
                .mark(Symbol::new().transform_no_output(
                    Calculate::new().expr("repeated", repeat::column()),
                    |mark| mark.x(col("repeated")).y(lit(1.0)).size(64.0),
                ))
        };

        let compiled_a =
            compile_with_repeat_context(template(), &ctx, column_repeat_context("a", 0, 2)).await;
        let evaluated_a = compiled_a
            .evaluate(&ctx, None)
            .await
            .expect("evaluates with repeat-resolved transform input");
        assert!(evaluated_a.scene_graph.width > 0.0);

        let compiled_b =
            compile_with_repeat_context(template(), &ctx, column_repeat_context("b", 1, 2)).await;
        let evaluated_b = compiled_b
            .evaluate(&ctx, None)
            .await
            .expect("evaluates with second repeat-resolved transform input");
        assert!(evaluated_b.scene_graph.width > 0.0);
    }

    #[tokio::test]
    async fn repeat_grid_resolves_event_binding_placeholders() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("a", DataType::Float64, false),
                Field::new("b", DataType::Float64, false),
            ])),
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0])),
                Arc::new(Float64Array::from(vec![3.0, 4.0])),
            ],
        )?;
        let binding = ChartEventBinding::on(ChartEventType::Click)
            .filter(repeat::cell_id().eq(lit("repeat_cell:a:b")))
            .set_store_at_start_scope(
                "brush_boxes",
                StoreUpdate::upsert_rows([StoreRow::new()
                    .field("cell_id", repeat::cell_id())
                    .field("row_id", repeat::row_id())
                    .field("column_id", repeat::column_id())]),
            )
            .set_selection_at_start_scope(
                "brush",
                SelectionUpdate::upsert_clause(
                    SelectionClauseUpdate::interval(repeat::cell_id())
                        .dimension(repeat::column())
                        .endpoints(lit(1.0), lit(2.0))
                        .dimension(repeat::row())
                        .endpoints(lit(3.0), lit(4.0))
                        .build(),
                ),
            )
            .set_selection_at_start_scope(
                "field_pick",
                SelectionUpdate::replace_clause(
                    SelectionClauseUpdate::equality(lit("active"))
                        .dimension_datum_named("column", repeat::column_name())
                        .build(),
                ),
            );
        let compiled = Plot::<RepeatGrid>::new()
            .data(ctx.read_batch(batch)?)
            .rows(repeat_vars(&["a"]))
            .columns(repeat_vars(&["b"]))
            .cell(repeated_grid_cell().event_binding(binding))
            .compile(&ctx)
            .await?;

        let bindings = compiled
            .event_bindings()
            .iter()
            .filter(|binding| binding.event_type == ChartEventType::Click)
            .collect::<Vec<_>>();
        assert_eq!(bindings.len(), 1);
        let binding = bindings[0];
        let filter = binding.filters[0].to_expr(&ctx)?;
        assert_eq!(
            simplify_to_scalar_sync(filter)?,
            ScalarValue::Boolean(Some(true))
        );

        let StoreUpdate::UpsertRows { rows } = &binding.store_assignments[0].update else {
            panic!("expected store upsert");
        };
        let cell_id = rows[0]
            .fields
            .get("cell_id")
            .expect("cell id field")
            .to_expr()?;
        assert_eq!(
            simplify_to_scalar_sync(cell_id)?,
            ScalarValue::Utf8(Some("repeat_cell:a:b".to_string()))
        );
        let row_id = rows[0]
            .fields
            .get("row_id")
            .expect("row id field")
            .to_expr()?;
        let column_id = rows[0]
            .fields
            .get("column_id")
            .expect("column id field")
            .to_expr()?;
        assert_eq!(
            simplify_to_scalar_sync(row_id)?,
            ScalarValue::Utf8(Some("a".to_string()))
        );
        assert_eq!(
            simplify_to_scalar_sync(column_id)?,
            ScalarValue::Utf8(Some("b".to_string()))
        );

        let SelectionUpdate::UpsertClauses { clauses } = &binding.selection_assignments[0].update
        else {
            panic!("expected selection upsert");
        };
        let clause_id = clauses[0].id.to_expr()?;
        assert_eq!(
            simplify_to_scalar_sync(clause_id)?,
            ScalarValue::Utf8(Some("repeat_cell:a:b".to_string()))
        );
        let SelectionPredicateUpdate::Interval { dimensions } = &clauses[0].predicate else {
            panic!("expected interval predicate");
        };
        assert_eq!(dimensions[0].field_expr.to_expr(&ctx)?.to_string(), "b");
        assert_eq!(dimensions[1].field_expr.to_expr(&ctx)?.to_string(), "a");

        let SelectionUpdate::ReplaceAllClauses { clauses } =
            &binding.selection_assignments[1].update
        else {
            panic!("expected selection replacement");
        };
        let SelectionPredicateUpdate::Equality { dimensions } = &clauses[0].predicate else {
            panic!("expected equality predicate");
        };
        assert_eq!(dimensions[0].field_expr.to_expr(&ctx)?.to_string(), "b");
        assert_eq!(
            dimensions[0].value.to_expr()?.to_string(),
            "__event_datum_b"
        );
        Ok(())
    }

    #[tokio::test]
    async fn repeat_placeholder_without_context_errors_during_compile() {
        let ctx = SessionContext::new();
        let err = match Plot::<Cartesian>::new()
            .mark(Symbol::new().x(repeat::column()).y(lit(1.0)))
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("repeat placeholder without context should error"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("repeat::column()"), "{err}");
    }
}
