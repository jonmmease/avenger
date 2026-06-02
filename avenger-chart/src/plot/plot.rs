//! Plot builder for creating visualizations

use std::{collections::HashMap, sync::Arc};

use datafusion::{common::ScalarValue, dataframe::DataFrame};
use datafusion_proto::protobuf::LogicalPlanNode;
use indexmap::IndexMap;

use avenger_chart_core::{
    AvengerChartError, AxisSpec, ChartTool, CompileContext, CompiledMark, CompiledMarkState,
    CompiledParamSpec, CompiledSelectionSpec, CompiledSubplotChildPlot, CoordinateGuide,
    CoordinateSystem, IntoExpr, Legend, Mark, MarkDataMode, Param, Selection, Sharing, Store,
    SubplotChildPlotSpec, Theme, compile_selections,
};
use avenger_chart_scales::{PlotScaleSpec as ScaleSpec, serialization::LogicalPlanNodeExt};

use crate::{
    event::ChartEventBinding,
    layout::{CanvasConstraint, LayoutSpec, Margins, PlotConstraint, SizeMode},
    serialization::serializable_expr_from_expr,
    tools::ToolCompileContext,
};

use super::{
    compiled::CompiledPlot,
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
        let root_tool_context = ToolCompileContext::root();
        self.compile_with_tool_context(session_context, Some(&root_tool_context), true)
            .await
    }

    pub(crate) async fn compile_with_tool_context(
        self,
        session_context: &datafusion::prelude::SessionContext,
        inherited_tool_context: Option<&ToolCompileContext>,
        is_root: bool,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let tool_context = ToolCompileContext::from_parent(inherited_tool_context);
        let active_tool_expansions = tool_context.expand_local_tools(&self.tools)?;
        tool_context.register_local_stores(&self.stores)?;
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

        // 1. Extract and merge channel configs from all marks with proper SessionContext
        for mark in &marks {
            crate::plot::channel::extract_channel_configs(
                mark.as_ref(),
                session_context,
                &mut axis_specs,
                &mut legends,
                &mut scale_specs,
                &mut scale_to_coord_channel,
            );
        }

        let coord_transform = self.coord_system.create_transform();
        let scale_sharing = scale_domain_share_modes(&marks);
        tool_context.apply_scale_edits(
            &active_tool_expansions,
            coord_transform.as_ref(),
            &scale_to_coord_channel,
            &mut scale_specs,
            &scale_sharing,
        )?;

        // 2. Compile all marks. Aggregate channels are intentionally prepared at
        // runtime so faceted marks aggregate after mark-level data scope has been
        // resolved.
        let mut compiled_marks: Vec<Arc<dyn CompiledMark>> = Vec::new();
        for (mark_index, m) in marks.iter().enumerate() {
            let mark_state = m.state();
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
        let mut event_bindings = self.event_bindings.clone();
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
        let compiled = CompiledPlot {
            coord_transform,
            compiled_guide: Some(compiled_guide),
            marks: compiled_marks,
            axis_specs,
            legends,
            layout_spec: self.layout_spec,
            title: self.title,
            subtitle: self.subtitle,
            theme: self.theme,
            scale_to_coord_channel,
            scale_specs,
            data: data_plan_node,
            default_params,
            param_specs,
            store_specs,
            event_bindings,
            selection_specs,
            cursor_params,
            tool_metadata,
        };

        // 6. Validate scoped raw-domain params are shared at least as broadly as
        // the scales they drive (catches Free/Level pan misconfigurations early).
        compiled.validate_scoped_raw_domain_sharing(session_context)?;

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
    /// The parameter is globally shared (`Sharing::Shared`), matching the
    /// historical single-value behavior. Use [`Plot::add_param_with_sharing`] to
    /// register a parameter with a finer-grained facet sharing scope.
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
    /// `Sharing::Free`/`Level(0)` gives one value per leaf coordinate scope,
    /// `Sharing::Level(N)` shares per logical ancestor `N` levels up, and
    /// `Sharing::Shared` keeps one global value.
    pub fn add_param_with_sharing(mut self, param: Param, sharing: Sharing) -> Self {
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

fn scale_domain_share_modes<C: CoordinateSystem>(
    marks: &[Arc<dyn Mark<C>>],
) -> HashMap<String, Sharing> {
    let mut result = HashMap::new();
    for mark in marks {
        for (channel_name, channel_value) in mark.data_context().channels() {
            let Some(scale_name) = channel_value.get_scale_name(channel_name) else {
                continue;
            };
            let sharing = channel_value
                .get_share_mode()
                .unwrap_or(Sharing::Free)
                .to_normalized();
            result
                .entry(scale_name)
                .and_modify(|existing: &mut Sharing| {
                    if sharing.to_level() > existing.to_level() {
                        *existing = sharing;
                    }
                })
                .or_insert(sharing);
        }
    }
    result
}
