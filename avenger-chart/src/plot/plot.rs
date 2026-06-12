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
    CompiledMark, CompiledMarkState, CompiledParamSpec, CompiledSelectionSpec,
    CompiledSubplotChildPlot, CoordinateGuide, CoordinateSystem, CoordinationScope,
    DomainCoordination, DomainCoordinationGroup, IntoExpr, Legend, LegendSurfaceKind, Mark,
    MarkDataMode, MarkState, Param, RepeatContext, RepeatDomainCoordination, RepeatVariable,
    Selection, Store, SubplotChildPlotSpec, Theme, TimeContext, compile_selections,
    validate_structural_id,
};
use avenger_chart_marks::Subplot;
use avenger_chart_scales::{PlotScaleSpec as ScaleSpec, serialization::LogicalPlanNodeExt};

use crate::{
    concat::{GridConcat, HConcat, VConcat, WrapConcat},
    event::{ChartEventBinding, rewrite_legend_event_binding_local_datums},
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

    /// Marks stored until compilation
    marks: Vec<Arc<dyn Mark<C>>>,

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
        self.compile_with_tool_context(session_context, Some(&root_tool_context), true)
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
                return lowered
                    .compile_with_tool_context(session_context, inherited_tool_context, is_root)
                    .await;
            }
            MaybeLoweredRepeatPlot::Original(plot) => {
                return plot
                    .compile_without_repeat_lowering(
                        session_context,
                        inherited_tool_context,
                        is_root,
                    )
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
        let tool_context = ToolCompileContext::from_parent(inherited_tool_context)
            .with_time_context(effective_time_context.clone());
        let coord_transform = self.coord_system.create_transform();

        let mut pre_tool_axis_specs: HashMap<String, AxisSpec> = HashMap::new();
        let mut pre_tool_legends: IndexMap<String, Legend> = self.legends.clone();
        let mut pre_tool_scale_specs: HashMap<String, ScaleSpec> = self.scale_specs.clone();
        let mut pre_tool_scale_to_coord_channel: HashMap<String, String> = HashMap::new();
        let pre_tool_mark_states = resolve_mark_states(&self.marks, tool_context.repeat_context())?;
        for mark_state in &pre_tool_mark_states {
            crate::plot::channel::extract_channel_configs_from_state(
                mark_state,
                session_context,
                &mut pre_tool_axis_specs,
                &mut pre_tool_legends,
                &mut pre_tool_scale_specs,
                &mut pre_tool_scale_to_coord_channel,
            );
        }
        let pre_tool_scale_coordination =
            scale_domain_coordinations_from_states(&pre_tool_mark_states)?;
        let tool_scale_targets = discover_tool_scale_targets(
            coord_transform.as_ref(),
            &pre_tool_scale_to_coord_channel,
            &pre_tool_scale_coordination,
        )?;
        let active_tool_expansions =
            tool_context.expand_local_tools(&self.tools, &tool_scale_targets)?;
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
            marks.extend(active.expansion.marks.iter().cloned());
        }
        validate_sibling_mark_ids(&marks)?;
        let resolved_mark_states = resolve_mark_states(&marks, tool_context.repeat_context())?;

        // 1. Extract and merge channel configs from all marks with proper SessionContext
        for mark_state in &resolved_mark_states {
            crate::plot::channel::extract_channel_configs_from_state(
                mark_state,
                session_context,
                &mut axis_specs,
                &mut legends,
                &mut scale_specs,
                &mut scale_to_coord_channel,
            );
        }

        let scale_coordination = scale_domain_coordinations_from_states(&resolved_mark_states)?;
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
        for (mark_index, (m, mark_state)) in
            marks.iter().zip(resolved_mark_states.iter()).enumerate()
        {
            // Get the DataFrame (or use plot-level data)
            let df_opt = if mark_state.data_mode == MarkDataMode::Unit {
                None
            } else {
                mark_state
                    .data
                    .dataframe()
                    .cloned()
                    .or_else(|| self.data.clone())
            };

            let compiled_state =
                CompiledMarkState::from_mark_state(mark_state, df_opt).with_mark_index(mark_index);
            let compiled_mark = m
                .compile_with_context(compiled_state, session_context, Some(erased_tool_context))
                .await?;
            compiled_marks.push(compiled_mark);
        }

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

        for binding in &event_bindings {
            binding.validate()?;
        }

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
            axis_specs,
            legends,
            legend_colorbar_overlays,
            layout_spec: self.layout_spec,
            title: self.title,
            subtitle: self.subtitle,
            theme: self.theme,
            time_context: effective_time_context,
            scale_to_coord_channel,
            scale_specs,
            data: data_plan_node,
            default_params,
            param_specs,
            store_specs,
            event_bindings,
            event_datum_fields: Vec::new(),
            selection_specs,
            cursor_params,
            tool_metadata,
        };
        compiled.event_datum_fields = compiled.infer_event_datum_fields(session_context)?;

        // 6. Validate scoped raw-domain params are shared at least as broadly as
        // the scales they drive (catches Free/Level pan misconfigurations early).
        compiled.validate_scoped_raw_domain_sharing(session_context)?;
        compiled.validate_transform_output_scale_sharing()?;

        Ok(compiled)
    }

    /// Get a reference to the coordinate system
    pub fn coord_system(&self) -> &C {
        &self.coord_system
    }

    pub fn mark<M: Mark<C> + 'static>(mut self, mark: M) -> Self {
        // Just store the mark - config extraction happens during compile()
        self.marks.push(Arc::new(mark));
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
    Original(Plot<C>),
    Lowered(LoweredRepeatPlot),
}

enum LoweredRepeatPlot {
    Columns(Plot<HConcat>),
    Rows(Plot<VConcat>),
    Grid(Plot<GridConcat>),
    Wrap(Plot<WrapConcat>),
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
        return Ok(MaybeLoweredRepeatPlot::Lowered(LoweredRepeatPlot::Columns(
            lower_repeat_columns_plot(plot, repeat, session_context)?,
        )));
    }
    if let Some(repeat) = (&plot.coord_system as &dyn Any)
        .downcast_ref::<RepeatRows>()
        .cloned()
    {
        return Ok(MaybeLoweredRepeatPlot::Lowered(LoweredRepeatPlot::Rows(
            lower_repeat_rows_plot(plot, repeat, session_context)?,
        )));
    }
    if let Some(repeat) = (&plot.coord_system as &dyn Any)
        .downcast_ref::<RepeatGrid>()
        .cloned()
    {
        return Ok(MaybeLoweredRepeatPlot::Lowered(LoweredRepeatPlot::Grid(
            lower_repeat_grid_plot(plot, repeat, session_context)?,
        )));
    }
    if let Some(repeat) = (&plot.coord_system as &dyn Any)
        .downcast_ref::<RepeatWrap>()
        .cloned()
    {
        return Ok(MaybeLoweredRepeatPlot::Lowered(LoweredRepeatPlot::Wrap(
            lower_repeat_wrap_plot(plot, repeat, session_context)?,
        )));
    }

    Ok(MaybeLoweredRepeatPlot::Original(plot))
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
        marks,
        data: parts.data,
        scale_specs: parts.scale_specs,
        legends: parts.legends,
        layout_spec: parts.layout_spec,
        title: parts.title,
        subtitle: parts.subtitle,
        theme: parts.theme,
        time_context: parts.time_context,
        guide_config: None,
        param_specs: parts.param_specs,
        event_bindings: parts.event_bindings,
        selections: parts.selections,
        stores: parts.stores,
        cursor_params: parts.cursor_params,
        tools: Vec::new(),
    }
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
    Ok(finish_lowered_repeat_plot(parts, HConcat::new(), marks))
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
    Ok(finish_lowered_repeat_plot(parts, VConcat::new(), marks))
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
            .with_axis_guide_visibility_config(repeat.axis_guide_visibility_config()),
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
        WrapConcat::new().with_column_mode(repeat.column_mode_config().clone()),
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
) -> Result<HashMap<String, DomainCoordination>, AvengerChartError> {
    let state_refs = states.iter().collect::<Vec<_>>();
    scale_domain_coordinations_from_state_refs(&state_refs)
}

fn scale_domain_coordinations_from_state_refs(
    states: &[&MarkState],
) -> Result<HashMap<String, DomainCoordination>, AvengerChartError> {
    let mut result: HashMap<String, DomainCoordination> = HashMap::new();
    for state in states {
        for (channel_name, channel_value) in state.data.channels() {
            let Some(scale_name) = channel_value.get_scale_name(channel_name) else {
                continue;
            };
            let Some(coordination) = channel_value.get_domain_coordination() else {
                continue;
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

fn validate_sibling_mark_ids<C: CoordinateSystem>(
    marks: &[Arc<dyn Mark<C>>],
) -> Result<(), AvengerChartError> {
    let mut seen = HashSet::new();
    for mark in marks {
        let Some(id) = mark.state().id.as_deref() else {
            continue;
        };
        validate_structural_id("mark", id)?;
        if !seen.insert(id.to_string()) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Duplicate mark id '{id}' among sibling marks"
            )));
        }
    }
    Ok(())
}

fn legend_event_bindings(
    legends: &IndexMap<String, Legend>,
    session_context: &datafusion::prelude::SessionContext,
) -> Result<Vec<ChartEventBinding>, AvengerChartError> {
    let mut bindings = Vec::new();
    for (channel_name, legend) in legends {
        legend.validate_event_surface()?;
        for binding in &legend.event_bindings {
            let binding =
                rewrite_legend_event_binding_local_datums(binding.clone(), session_context)?
                    .with_legend_surface_target(
                        vec![channel_name.clone()],
                        vec![
                            LegendSurfaceKind::DiscreteItem,
                            LegendSurfaceKind::ContinuousColorbar,
                        ],
                    );
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
    use crate::event::{ChartEventBinding, ChartEventType};
    use crate::facet::coord::{FacetColumn, FacetWrap};
    use crate::facet::marks::{FacetColumnSubplotChannels, FacetWrapSubplotChannels};
    use crate::repeat::{RepeatColumns, RepeatGrid, RepeatRows, RepeatWrap};
    use crate::tools::ToolCompileContext;
    use crate::zerod::ZeroDCoord;
    use avenger_chart_cartesian::{
        CartesianAxis, CartesianRectPositionChannels, CartesianSymbolPositionChannels,
    };
    use avenger_chart_core::{
        AxisGuideVisibilityPolicy, DefaultLogicalExprNodeExt, DomainCoordinationGroup,
        RepeatContext, RepeatDomainCoordination, RepeatVariable, ResolvedRepeatVariable,
        ScaleChannelConfig, SelectionClauseUpdate, SelectionPredicateUpdate, SelectionUpdate,
        StoreRow, StoreUpdate, SubplotDataSource, collect_repeat_placeholder_kinds, repeat,
        simplify_to_scalar_sync,
    };
    use avenger_chart_marks::{Rect, Subplot, Symbol};
    use avenger_chart_tools::PanScrollZoom;
    use avenger_chart_transforms::{Bin, Calculate};
    use datafusion::{
        arrow::{
            array::Float64Array,
            datatypes::{DataType, Field, Schema},
            record_batch::RecordBatch,
        },
        functions_aggregate::expr_fn::count,
        prelude::{SessionContext, col, lit},
    };
    use std::sync::Arc;

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

    fn repeated_grid_cell() -> Plot<Cartesian> {
        Plot::<Cartesian>::new().mark(
            Symbol::new()
                .x(repeat::column())
                .y(repeat::row())
                .size(64.0),
        )
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
            );
        let compiled = Plot::<RepeatGrid>::new()
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
