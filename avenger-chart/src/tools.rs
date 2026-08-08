//! Chart tool compilation support and built-in tool re-exports.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Arc, Mutex},
};

use avenger_chart_core::{
    Auto, AvengerChartError, ChartEventBinding, ChartParamChangeBinding, CompiledIdentityAllocator,
    CompiledParamSpec, CompiledSelectionSpec, CompiledStoreSpec, CompiledToolBehavior,
    CoordinateSystemCore, CoordinateSystemTransform, CoordinationScope, DefaultLogicalExprNodeExt,
    DomainCoordination, DomainCoordinationGroup, FormattingContext, Param, RepeatContext,
    ResolvedStateDeclaration, Scale, Selection, Store, TimeContext, ToolBehaviorExpansion,
    ToolInstanceId, resolve_repeat_placeholders,
};
use avenger_chart_scales::PlotScaleSpec;
use datafusion::prelude::lit;
use datafusion_proto::protobuf::LogicalExprNode;
use indexmap::IndexMap;

pub use avenger_chart_core::{
    ChartTool, ResolvedToolMark, ToolBehaviorExpansion as ResolvedToolBehaviorExpansion,
    ToolExpansionContext, ToolExport, ToolExportTarget, ToolMetadata, ToolParamSharing,
    ToolScaleEdit, ToolScaleTarget,
};
pub use avenger_chart_tools::{
    BoxSelection, BoxSelectionResolve, BoxZoom, LassoSelection, PanScrollZoom, PointSelection,
    UnitAspectBox,
};

#[derive(Clone)]
pub(crate) struct ToolCompileContext {
    state: Arc<Mutex<ToolCompileState>>,
    coord_node_path: Vec<usize>,
    multiplied_host: bool,
    theme: Option<Arc<avenger_chart_core::Theme>>,
    time_context: TimeContext,
    formatting_context: FormattingContext,
    repeat_context: Option<RepeatContext>,
}

impl ToolCompileContext {
    pub(crate) fn root(
        theme: Option<Arc<avenger_chart_core::Theme>>,
        time_context: TimeContext,
        formatting_context: FormattingContext,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(ToolCompileState::default())),
            coord_node_path: Vec::new(),
            multiplied_host: false,
            theme,
            time_context,
            formatting_context,
            repeat_context: None,
        }
    }

    pub(crate) fn from_parent(parent: Option<&ToolCompileContext>) -> Self {
        Self {
            state: parent
                .map(|ctx| ctx.state.clone())
                .unwrap_or_else(|| Arc::new(Mutex::new(ToolCompileState::default()))),
            coord_node_path: parent
                .map(|ctx| ctx.coord_node_path.clone())
                .unwrap_or_default(),
            multiplied_host: parent.is_some_and(|ctx| ctx.multiplied_host),
            theme: parent.and_then(|ctx| ctx.theme.clone()),
            time_context: parent
                .map(|ctx| ctx.time_context.clone())
                .unwrap_or_default(),
            formatting_context: parent
                .map(|ctx| ctx.formatting_context.clone())
                .unwrap_or_default(),
            repeat_context: parent.and_then(|ctx| ctx.repeat_context.clone()),
        }
    }

    pub(crate) fn theme(&self) -> Option<&Arc<avenger_chart_core::Theme>> {
        self.theme.as_ref()
    }

    pub(crate) fn time_context(&self) -> &TimeContext {
        &self.time_context
    }

    pub(crate) fn formatting_context(&self) -> &FormattingContext {
        &self.formatting_context
    }

    pub(crate) fn repeat_context(&self) -> Option<&RepeatContext> {
        self.repeat_context.as_ref()
    }

    pub(crate) fn is_multiplied_host(&self) -> bool {
        self.multiplied_host
    }

    pub(crate) fn compiled_identity_seed(&self) -> String {
        let path = self
            .coord_node_path
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join("/");
        format!("compiled-plot-v1:{path}")
    }

    pub(crate) fn with_multiplied_host(mut self) -> Self {
        self.multiplied_host = true;
        self
    }

    #[allow(dead_code)]
    pub(crate) fn with_repeat_context(mut self, repeat_context: RepeatContext) -> Self {
        self.repeat_context = Some(repeat_context);
        self.multiplied_host = true;
        self
    }

    pub(crate) fn with_coord_node_path_appended(&self, child_index: usize) -> Self {
        let mut coord_node_path = self.coord_node_path.clone();
        coord_node_path.push(child_index);
        Self {
            state: self.state.clone(),
            coord_node_path,
            multiplied_host: self.multiplied_host,
            theme: self.theme.clone(),
            time_context: self.time_context.clone(),
            formatting_context: self.formatting_context.clone(),
            repeat_context: self.repeat_context.clone(),
        }
    }

    pub(crate) fn target_path_with_child(&self, child_index: usize) -> Vec<usize> {
        let mut path = self.coord_node_path.clone();
        path.push(child_index);
        path
    }

    pub(crate) fn register_native_widget(
        &self,
        id: &str,
        identity: usize,
        state_spec: &avenger_chart_core::NativeWidgetStateSpec,
    ) -> Result<(), AvengerChartError> {
        let mut state = self.state.lock().expect("tool compile state lock poisoned");
        state.register_widget_identity(id, identity)?;
        state
            .native_widget_param_specs
            .extend(state_spec.params().iter().cloned());
        Ok(())
    }

    pub(crate) fn expand_local_tools<C: CoordinateSystemCore>(
        &self,
        tools: &[Arc<dyn ChartTool<C>>],
        scale_targets: &[ToolScaleTarget],
        coordinate_metrics: &[avenger_chart_core::CoordinateMetricDescriptor],
        identity_allocator: &mut CompiledIdentityAllocator,
    ) -> Result<Vec<ActiveToolExpansion<C>>, AvengerChartError> {
        let mut active = Vec::new();
        let mut local_ids = HashSet::new();
        for tool in tools {
            self.expand_tool_recursive(
                tool,
                scale_targets,
                coordinate_metrics,
                identity_allocator,
                &[],
                &mut local_ids,
                &mut active,
            )?;
        }
        Ok(active)
    }

    #[allow(clippy::too_many_arguments)]
    fn expand_tool_recursive<C: CoordinateSystemCore>(
        &self,
        tool: &Arc<dyn ChartTool<C>>,
        scale_targets: &[ToolScaleTarget],
        coordinate_metrics: &[avenger_chart_core::CoordinateMetricDescriptor],
        identity_allocator: &mut CompiledIdentityAllocator,
        ancestry: &[ToolInstanceId],
        local_ids: &mut HashSet<String>,
        active: &mut Vec<ActiveToolExpansion<C>>,
    ) -> Result<(), AvengerChartError> {
        let id = tool.id().to_string();
        validate_tool_id(&id)?;
        if !local_ids.insert(id.clone()) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Duplicate chart tool id '{id}'"
            )));
        }
        let identity = Arc::as_ptr(tool) as *const () as usize;
        // Every lowered occurrence receives its own opaque identity. Repeat
        // and facet lowering may reuse the same authored `Arc`, but occurrence
        // identity must still keep private marks and cell-local state apart.
        // `register_expansion` separately recognizes those structural copies
        // by authoring identity and coalesces compatible shared declarations.
        let instance_id = identity_allocator.allocate_tool_instance();
        let mut expansion_context = ToolExpansionContext::new(&id, scale_targets)
            .with_coordinate_metrics(coordinate_metrics)
            .with_resolved_instance(instance_id.clone(), ancestry.to_vec());
        if let Some(repeat_context) = self.repeat_context.as_ref() {
            expansion_context = expansion_context.with_repeat_context(repeat_context);
        }
        let mut expansion = tool.expand(expansion_context)?;
        if expansion.instance_id != instance_id {
            return Err(AvengerChartError::InvalidArgument(format!(
                "tool '{id}' returned behavior for a different tool instance"
            )));
        }
        expansion.instance_ancestry = ancestry.to_vec();
        if expansion.component_kind == "tool" {
            expansion.component_kind = id.clone();
        }
        if expansion.component_id.is_none() {
            expansion.component_id = Some(id.clone());
        }
        self.resolve_repeat_event_bindings(&mut expansion.event_bindings)?;
        self.resolve_repeat_param_change_bindings(&mut expansion.param_change_bindings)?;
        self.localize_event_bindings(&mut expansion.event_bindings);
        self.state
            .lock()
            .expect("tool compile state lock poisoned")
            .register_expansion(&id, identity, &expansion)?;
        let nested = expansion.nested_tools.clone();
        active.push(ActiveToolExpansion {
            id: id.clone(),
            expansion,
        });

        let mut nested_ancestry = ancestry.to_vec();
        nested_ancestry.push(instance_id);
        for nested_tool in nested {
            self.expand_tool_recursive(
                &nested_tool,
                scale_targets,
                coordinate_metrics,
                identity_allocator,
                &nested_ancestry,
                local_ids,
                active,
            )?;
        }
        Ok(())
    }

    pub(crate) fn register_local_event_bindings(
        &self,
        bindings: &[ChartEventBinding],
    ) -> Result<(), AvengerChartError> {
        if bindings.is_empty() {
            return Ok(());
        }
        let mut bindings = bindings.to_vec();
        self.resolve_repeat_event_bindings(&mut bindings)?;
        self.localize_event_bindings(&mut bindings);
        self.state
            .lock()
            .expect("tool compile state lock poisoned")
            .event_bindings
            .extend(bindings);
        Ok(())
    }

    pub(crate) fn register_local_param_change_bindings(
        &self,
        bindings: &[ChartParamChangeBinding],
    ) -> Result<(), AvengerChartError> {
        if bindings.is_empty() {
            return Ok(());
        }
        let mut bindings = bindings.to_vec();
        self.resolve_repeat_param_change_bindings(&mut bindings)?;
        let mut state = self.state.lock().expect("tool compile state lock poisoned");
        for binding in bindings {
            if !state.param_change_bindings.contains(&binding) {
                state.param_change_bindings.push(binding);
            }
        }
        Ok(())
    }

    pub(crate) fn register_widget_expansion(
        &self,
        id: &str,
        identity: usize,
        widget_scene_index: usize,
        expansion: &avenger_chart_core::ToolBehaviorExpansion<avenger_chart_core::PixelFrame>,
        mark_runtime_ids: &[avenger_chart_core::MarkId],
    ) -> Result<(), AvengerChartError> {
        self.register_widget_expansion_with_public_path(
            id,
            id,
            identity,
            expansion,
            (!self.coord_node_path.is_empty()).then(|| {
                expansion
                    .marks
                    .iter()
                    .enumerate()
                    .filter_map(|(mark_index, resolved_mark)| {
                        resolved_mark.mark.state().id.as_deref().map(|part| {
                            (
                                format!("{id}.{part}"),
                                vec![vec![widget_scene_index, mark_index]],
                            )
                        })
                    })
                    .collect()
            }),
            (!self.coord_node_path.is_empty()).then(|| {
                expansion
                    .marks
                    .iter()
                    .enumerate()
                    .filter_map(|(mark_index, resolved_mark)| {
                        resolved_mark.mark.state().id.as_deref().map(|part| {
                            (
                                format!("{id}.{part}"),
                                vec![mark_runtime_ids[mark_index].clone()],
                            )
                        })
                    })
                    .collect()
            }),
        )
    }

    pub(crate) fn register_widget_expansion_with_public_path(
        &self,
        id: &str,
        public_widget_path: &str,
        identity: usize,
        expansion: &avenger_chart_core::ToolBehaviorExpansion<avenger_chart_core::PixelFrame>,
        target_paths: Option<BTreeMap<String, Vec<Vec<usize>>>>,
        target_ids: Option<BTreeMap<String, Vec<avenger_chart_core::MarkId>>>,
    ) -> Result<(), AvengerChartError> {
        let mut expansion = expansion.clone();
        let mut all_targets = HashSet::new();
        let mut interactive_targets = Vec::new();
        for resolved_mark in &expansion.marks {
            let Some(part) = resolved_mark.mark.state().id.as_deref() else {
                continue;
            };
            let target = format!("{id}.{part}");
            all_targets.insert(target.clone());
            if !avenger_chart_core::is_decorative_widget_part(part) {
                interactive_targets.push(target);
            }
        }
        for binding in &mut expansion.event_bindings {
            validate_widget_binding_targets(binding.mark_ids(), &all_targets, id)?;
            if binding.between.is_none() && binding.mark_ids().is_empty() {
                *binding = binding.clone().marks(interactive_targets.clone());
            }
            if let Some(between) = &mut binding.between {
                validate_widget_binding_targets(between.start.mark_ids(), &all_targets, id)?;
                validate_widget_binding_targets(between.end.mark_ids(), &all_targets, id)?;
                if between.start.mark_ids().is_empty() {
                    between.start = between.start.clone().marks(interactive_targets.clone());
                }
                // An untargeted end stream is intentionally surface-global.
                // Pointer gestures must terminate on mouse-up even after the
                // pointer leaves the widget that owns the captured start.
            }
        }
        let public_target = |target: &str| {
            target
                .strip_prefix(&format!("{id}."))
                .map(|part| format!("{public_widget_path}.{part}"))
                .unwrap_or_else(|| target.to_string())
        };
        for binding in &mut expansion.event_bindings {
            *binding = binding.clone().marks(
                binding
                    .mark_ids()
                    .iter()
                    .map(|target| public_target(target)),
            );
            if let Some(between) = &mut binding.between {
                between.start = between.start.clone().marks(
                    between
                        .start
                        .mark_ids()
                        .iter()
                        .map(|target| public_target(target)),
                );
                between.end = between.end.clone().marks(
                    between
                        .end
                        .mark_ids()
                        .iter()
                        .map(|target| public_target(target)),
                );
            }
        }
        self.resolve_repeat_event_bindings(&mut expansion.event_bindings)?;
        self.resolve_repeat_param_change_bindings(&mut expansion.param_change_bindings)?;
        let mut state = self.state.lock().expect("tool compile state lock poisoned");
        if let Some(target_paths) = target_paths {
            for (target, paths) in target_paths {
                let ids = target_ids
                    .as_ref()
                    .and_then(|ids| ids.get(&target))
                    .cloned()
                    .ok_or_else(|| {
                        AvengerChartError::InternalError(format!(
                            "child widget target '{target}' lacks compiled mark identities"
                        ))
                    })?;
                state.register_child_widget_target(target, paths, ids)?;
            }
        }
        state.register_expansion(id, identity, &expansion)
    }

    pub(crate) fn register_local_legend_event_bindings(
        &self,
        bindings: &[ChartEventBinding],
    ) -> Result<(), AvengerChartError> {
        if bindings.is_empty() {
            return Ok(());
        }
        let mut bindings = bindings.to_vec();
        self.resolve_repeat_event_bindings(&mut bindings)?;
        self.state
            .lock()
            .expect("tool compile state lock poisoned")
            .event_bindings
            .extend(bindings);
        Ok(())
    }

    pub(crate) fn register_root_stores(&self, stores: &[Store]) -> Result<(), AvengerChartError> {
        let mut state = self.state.lock().expect("tool compile state lock poisoned");
        for store in stores {
            state.register_store(store.compile()?, false)?;
        }
        Ok(())
    }

    pub(crate) fn downcast(ctx: avenger_chart_core::CompileContext<'_>) -> Option<&Self> {
        ctx.downcast_ref::<Self>()
    }

    pub(crate) fn apply_scale_edits<C: CoordinateSystemCore>(
        &self,
        active_expansions: &[ActiveToolExpansion<C>],
        coord_transform: &dyn CoordinateSystemTransform,
        scale_to_coord_channel: &HashMap<String, String>,
        scale_specs: &mut HashMap<String, PlotScaleSpec>,
        scale_coordinations: &HashMap<String, DomainCoordination>,
    ) -> Result<(), AvengerChartError> {
        let invertible = coord_transform.interaction_invertible_channels();
        if invertible.is_empty() {
            return Ok(());
        }

        for active in active_expansions {
            for edit in &active.expansion.scale_edits {
                match edit {
                    ToolScaleEdit::RawDomain {
                        channel,
                        param_name,
                        override_existing,
                        disable_nice_zero,
                    } => {
                        if !invertible.iter().any(|invertible| invertible == channel) {
                            continue;
                        }
                        let targets = scale_to_coord_channel
                            .iter()
                            .filter_map(|(scale_name, coord_channel)| {
                                (coord_channel == channel).then_some(scale_name.clone())
                            })
                            .collect::<Vec<_>>();
                        for scale_name in targets {
                            let coordination = scale_coordinations
                                .get(&scale_name)
                                .cloned()
                                .unwrap_or_default();
                            self.state
                                .lock()
                                .expect("tool compile state lock poisoned")
                                .record_scale_target(
                                    &active.id,
                                    channel,
                                    &scale_name,
                                    param_name,
                                    &coordination,
                                )?;
                            let param = self
                                .state
                                .lock()
                                .expect("tool compile state lock poisoned")
                                .param(param_name)?
                                .clone();
                            apply_raw_domain_scale_edit(
                                &scale_name,
                                scale_specs,
                                &param,
                                *override_existing,
                                *disable_nice_zero,
                            )?;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    pub(crate) fn finalize_root(&self) -> Result<ToolArtifacts, AvengerChartError> {
        self.state
            .lock()
            .expect("tool compile state lock poisoned")
            .finalize()
    }

    fn localize_event_bindings(&self, bindings: &mut [ChartEventBinding]) {
        for binding in bindings {
            let mut localized = binding.clone().with_plot_surface_target();
            if !self.coord_node_path.is_empty() {
                localized =
                    localized.with_resolved_coord_node_path_target(self.coord_node_path.clone());
            }
            *binding = localized;
        }
    }

    fn resolve_repeat_event_bindings(
        &self,
        bindings: &mut [ChartEventBinding],
    ) -> Result<(), AvengerChartError> {
        let Some(repeat_context) = &self.repeat_context else {
            return Ok(());
        };
        for binding in bindings {
            *binding = binding
                .clone()
                .map_exprs(&mut |expr| resolve_repeat_placeholders(expr, repeat_context))?;
        }
        Ok(())
    }

    fn resolve_repeat_param_change_bindings(
        &self,
        bindings: &mut [ChartParamChangeBinding],
    ) -> Result<(), AvengerChartError> {
        let Some(repeat_context) = &self.repeat_context else {
            return Ok(());
        };
        for binding in bindings {
            *binding = binding
                .clone()
                .map_exprs(&mut |expr| resolve_repeat_placeholders(expr, repeat_context))?;
        }
        Ok(())
    }
}

pub(crate) fn discover_tool_scale_targets(
    coord_transform: &dyn CoordinateSystemTransform,
    scale_to_coord_channel: &HashMap<String, String>,
    scale_coordinations: &HashMap<String, DomainCoordination>,
) -> Result<Vec<ToolScaleTarget>, AvengerChartError> {
    let invertible = coord_transform.interaction_invertible_channels();
    let mut targets = Vec::new();
    for (scale_name, coord_channel) in scale_to_coord_channel {
        if !invertible
            .iter()
            .any(|invertible| invertible == coord_channel)
        {
            continue;
        }
        let coordination = scale_coordinations
            .get(scale_name)
            .cloned()
            .unwrap_or_default();
        targets.push(ToolScaleTarget {
            coord_channel: coord_channel.clone(),
            scale_name: scale_name.clone(),
            domain_coordination: resolve_scale_domain_coordination(scale_name, &coordination)?,
        });
    }
    Ok(targets)
}

#[derive(Clone)]
pub(crate) struct ActiveToolExpansion<C: CoordinateSystemCore> {
    id: String,
    pub(crate) expansion: ToolBehaviorExpansion<C>,
}

#[derive(Default)]
struct ToolCompileState {
    tool_ids: HashMap<String, usize>,
    params: IndexMap<String, GeneratedParamState>,
    stores: IndexMap<String, CompiledStoreSpec>,
    selections: IndexMap<String, CompiledSelectionSpec>,
    event_bindings: Vec<ChartEventBinding>,
    param_change_bindings: Vec<ChartParamChangeBinding>,
    metadata: Vec<ToolMetadata>,
    behaviors: Vec<CompiledToolBehavior>,
    expected_targets: BTreeMap<(String, String, String), usize>,
    child_widget_target_paths: BTreeMap<String, Vec<Vec<usize>>>,
    child_widget_target_ids: BTreeMap<String, Vec<avenger_chart_core::MarkId>>,
    native_widget_param_specs: Vec<CompiledParamSpec>,
}

impl ToolCompileState {
    fn register_widget_identity(
        &mut self,
        id: &str,
        identity: usize,
    ) -> Result<(), AvengerChartError> {
        match self.tool_ids.get(id) {
            Some(existing) if *existing == identity => Ok(()),
            Some(_) => Err(AvengerChartError::InvalidArgument(format!(
                "Duplicate chart tool or widget id '{id}'"
            ))),
            None => {
                self.tool_ids.insert(id.to_string(), identity);
                Ok(())
            }
        }
    }

    fn register_child_widget_target(
        &mut self,
        target: String,
        paths: Vec<Vec<usize>>,
        ids: Vec<avenger_chart_core::MarkId>,
    ) -> Result<(), AvengerChartError> {
        if self.child_widget_target_paths.contains_key(&target) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Duplicate mark target path '{target}'"
            )));
        }
        self.child_widget_target_paths.insert(target.clone(), paths);
        self.child_widget_target_ids.insert(target, ids);
        Ok(())
    }

    fn register_expansion<C: CoordinateSystemCore>(
        &mut self,
        id: &str,
        identity: usize,
        expansion: &ToolBehaviorExpansion<C>,
    ) -> Result<(), AvengerChartError> {
        let first_registration = match self.tool_ids.get(id) {
            Some(existing) if *existing == identity => false,
            Some(_) => {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Duplicate chart tool id '{id}'"
                )));
            }
            None => {
                self.tool_ids.insert(id.to_string(), identity);
                true
            }
        };

        for declaration in &expansion.state {
            match declaration {
                ResolvedStateDeclaration::Param {
                    runtime_id,
                    param,
                    sharing,
                } => self.register_param(runtime_id.clone(), param, sharing)?,
                ResolvedStateDeclaration::Store { runtime_id, store } => {
                    let mut spec = store.compile()?;
                    spec.runtime_id = runtime_id.clone();
                    self.register_store(spec, !first_registration)?;
                }
                ResolvedStateDeclaration::Selection {
                    runtime_id,
                    selection,
                } => self.register_selection(runtime_id.clone(), selection, !first_registration)?,
            }
        }
        for edit in &expansion.scale_edits {
            let ToolScaleEdit::RawDomain {
                channel,
                param_name,
                ..
            } = edit;
            self.expected_targets
                .entry((id.to_string(), channel.clone(), param_name.clone()))
                .or_insert(0);
        }
        self.event_bindings
            .extend(expansion.event_bindings.iter().cloned());
        if first_registration {
            self.param_change_bindings
                .extend(expansion.param_change_bindings.iter().cloned());
        }
        if first_registration {
            self.metadata.extend(expansion.metadata.iter().cloned());
            self.behaviors.push(CompiledToolBehavior {
                source_id: id.to_string(),
                instance_id: expansion.instance_id.clone(),
                instance_ancestry: expansion.instance_ancestry.clone(),
                component_kind: expansion.component_kind.clone(),
                component_id: expansion.component_id.clone(),
                exports: expansion.exports.clone(),
            });
        }
        Ok(())
    }

    fn register_store(
        &mut self,
        mut spec: CompiledStoreSpec,
        allow_existing: bool,
    ) -> Result<(), AvengerChartError> {
        if let Some(existing) = self.stores.get(&spec.name) {
            // Structural copies have distinct occurrence-owned IDs. A shared
            // source declaration retains the first compiled ID when every
            // author-visible property is otherwise identical.
            spec.runtime_id = existing.runtime_id.clone();
            if allow_existing && existing == &spec {
                return Ok(());
            }
            return Err(AvengerChartError::InvalidArgument(format!(
                "Store '{}' was declared more than once",
                spec.name
            )));
        }
        self.stores.insert(spec.name.clone(), spec);
        Ok(())
    }

    fn register_selection(
        &mut self,
        runtime_id: avenger_chart_core::SelectionRef,
        selection: &Selection,
        allow_existing: bool,
    ) -> Result<(), AvengerChartError> {
        let mut spec = selection.compile()?;
        spec.runtime_id = runtime_id;
        if let Some(existing) = self.selections.get(&spec.id) {
            spec.runtime_id = existing.runtime_id.clone();
            if allow_existing && existing == &spec {
                return Ok(());
            }
            return Err(AvengerChartError::InvalidArgument(format!(
                "Selection '{}' was declared more than once",
                spec.id
            )));
        }
        self.selections.insert(spec.id.clone(), spec);
        Ok(())
    }

    fn register_param(
        &mut self,
        runtime_id: avenger_chart_core::ParamRef,
        param: &Param,
        sharing: &ToolParamSharing,
    ) -> Result<(), AvengerChartError> {
        match self.params.get_mut(&param.name) {
            Some(existing) => {
                if existing.param.default != param.default || existing.requested != *sharing {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "Generated tool parameter '{}' was declared more than once with \
                         incompatible defaults or sharing",
                        param.name
                    )));
                }
            }
            None => {
                self.params.insert(
                    param.name.clone(),
                    GeneratedParamState {
                        runtime_id,
                        param: param.clone(),
                        requested: sharing.clone(),
                        resolved_sharing: match sharing {
                            ToolParamSharing::Explicit(sharing) => Some(sharing.to_normalized()),
                            ToolParamSharing::MirrorScale { .. } => None,
                        },
                        resolved_domain_coordination: None,
                    },
                );
            }
        }
        Ok(())
    }

    fn param(&self, name: &str) -> Result<&Param, AvengerChartError> {
        self.params
            .get(name)
            .map(|state| &state.param)
            .ok_or_else(|| {
                AvengerChartError::InvalidArgument(format!(
                    "Tool scale edit references unknown generated parameter '{name}'"
                ))
            })
    }

    fn record_scale_target(
        &mut self,
        tool_id: &str,
        channel: &str,
        scale_name: &str,
        param_name: &str,
        target_coordination: &DomainCoordination,
    ) -> Result<(), AvengerChartError> {
        let target_coordination =
            resolve_scale_domain_coordination(scale_name, target_coordination)?;
        if let Some(count) = self.expected_targets.get_mut(&(
            tool_id.to_string(),
            channel.to_string(),
            param_name.to_string(),
        )) {
            *count += 1;
        }

        let param = self.params.get_mut(param_name).ok_or_else(|| {
            AvengerChartError::InvalidArgument(format!(
                "Tool scale edit references unknown generated parameter '{param_name}'"
            ))
        })?;

        match &param.requested {
            ToolParamSharing::Explicit(explicit) => {
                let explicit = explicit.to_normalized();
                if explicit.to_level() < target_coordination.scope.to_level() {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "tool raw-domain param '{param_name}' is shared at {} but target \
                         channel '{channel}' is shared at {}; the param must be shared at \
                         least as broadly as the scale",
                        describe_sharing(explicit),
                        describe_sharing(target_coordination.scope)
                    )));
                }
                let resolved = target_coordination.with_scope(explicit);
                match &param.resolved_domain_coordination {
                    Some(existing) if existing.group != resolved.group => {
                        return Err(AvengerChartError::InvalidArgument(format!(
                            "tool raw-domain param '{param_name}' targets incompatible domain \
                             groups"
                        )));
                    }
                    Some(_) => {}
                    None => param.resolved_domain_coordination = Some(resolved),
                }
            }
            ToolParamSharing::MirrorScale {
                channel: mirror_channel,
            } => {
                if mirror_channel != channel {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "tool raw-domain param '{param_name}' mirrors channel '{mirror_channel}' \
                         but was used for channel '{channel}'"
                    )));
                }
                match &param.resolved_domain_coordination {
                    Some(existing) if existing != &target_coordination => {
                        return Err(AvengerChartError::InvalidArgument(format!(
                            "tool raw-domain param '{param_name}' targets channel '{channel}' \
                             with conflicting domain coordination targets"
                        )));
                    }
                    Some(_) => {}
                    None => {
                        param.resolved_domain_coordination = Some(target_coordination.clone());
                        param.resolved_sharing = Some(target_coordination.scope);
                    }
                }
            }
        }
        Ok(())
    }

    fn finalize(&self) -> Result<ToolArtifacts, AvengerChartError> {
        for ((tool_id, channel, _param), count) in &self.expected_targets {
            if *count == 0 {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "tool '{tool_id}' did not find an invertible scale target for channel \
                     '{channel}'"
                )));
            }
        }

        let mut param_specs = Vec::new();
        for (name, param) in &self.params {
            let Some(sharing) = param.resolved_sharing else {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "tool parameter '{name}' did not resolve a sharing scope"
                )));
            };
            let mut spec = CompiledParamSpec::new(&param.param, sharing);
            spec.runtime_id = param.runtime_id.clone();
            if let Some(coordination) = &param.resolved_domain_coordination {
                spec = spec.with_domain_coordination(coordination.clone());
            }
            param_specs.push(spec);
        }
        Ok(ToolArtifacts {
            param_specs,
            native_widget_param_specs: self.native_widget_param_specs.clone(),
            store_specs: self.stores.values().cloned().collect(),
            selection_specs: self.selections.values().cloned().collect(),
            event_bindings: self.event_bindings.clone(),
            param_change_bindings: self.param_change_bindings.clone(),
            metadata: self.metadata.clone(),
            behaviors: self.behaviors.clone(),
            child_widget_target_paths: self.child_widget_target_paths.clone(),
            child_widget_target_ids: self.child_widget_target_ids.clone(),
        })
    }
}

#[derive(Clone)]
struct GeneratedParamState {
    runtime_id: avenger_chart_core::ParamRef,
    param: Param,
    requested: ToolParamSharing,
    resolved_sharing: Option<CoordinationScope>,
    resolved_domain_coordination: Option<DomainCoordination>,
}

pub(crate) struct ToolArtifacts {
    pub param_specs: Vec<CompiledParamSpec>,
    pub native_widget_param_specs: Vec<CompiledParamSpec>,
    pub store_specs: Vec<CompiledStoreSpec>,
    pub selection_specs: Vec<CompiledSelectionSpec>,
    pub event_bindings: Vec<ChartEventBinding>,
    pub param_change_bindings: Vec<ChartParamChangeBinding>,
    pub metadata: Vec<ToolMetadata>,
    pub behaviors: Vec<CompiledToolBehavior>,
    pub child_widget_target_paths: BTreeMap<String, Vec<Vec<usize>>>,
    pub child_widget_target_ids: BTreeMap<String, Vec<avenger_chart_core::MarkId>>,
}

fn apply_raw_domain_scale_edit(
    scale_name: &str,
    scale_specs: &mut HashMap<String, PlotScaleSpec>,
    param: &Param,
    override_existing: bool,
    disable_nice_zero: bool,
) -> Result<(), AvengerChartError> {
    let raw_domain = LogicalExprNode::from_expr(param.expr()).map_err(|err| {
        AvengerChartError::InternalError(format!("serialize tool raw-domain expression: {err}"))
    })?;
    let spec = scale_specs
        .entry(scale_name.to_string())
        .or_insert_with(|| PlotScaleSpec::Local(Scale::<Auto>::new().into_config()));
    let PlotScaleSpec::Local(config) = spec;
    if let Some(existing) = config
        .domain
        .as_option()
        .and_then(|domain| domain.raw_domain.as_ref())
        && !override_existing
        && existing != &raw_domain
    {
        return Err(AvengerChartError::InvalidArgument(format!(
            "tool cannot install raw_domain for scale '{scale_name}' because it already \
             has a different raw_domain"
        )));
    }

    let mut scale = Scale::<Auto>::from_config(config.clone()).raw_domain(param.expr());
    if disable_nice_zero {
        scale = scale.option("nice", lit(false)).option("zero", lit(false));
    }
    *config = scale.into_config();
    Ok(())
}

fn validate_tool_id(id: &str) -> Result<(), AvengerChartError> {
    if id.is_empty()
        || id.contains('.')
        || !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Invalid chart tool id '{id}'; ids must be non-empty ASCII identifiers without periods"
        )));
    }
    Ok(())
}

fn validate_widget_binding_targets(
    targets: &[String],
    all_targets: &HashSet<String>,
    widget_id: &str,
) -> Result<(), AvengerChartError> {
    let prefix = format!("{widget_id}.");
    for target in targets {
        if !all_targets.contains(target) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Widget '{widget_id}' event binding targets unknown part '{target}'"
            )));
        }
        let part = target.strip_prefix(&prefix).ok_or_else(|| {
            AvengerChartError::InvalidArgument(format!(
                "Widget '{widget_id}' event target '{target}' must use the '{widget_id}.<part>' form"
            ))
        })?;
        if avenger_chart_core::is_decorative_widget_part(part) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Widget '{widget_id}' event binding cannot target decorative part '{part}'"
            )));
        }
    }
    Ok(())
}

fn describe_sharing(sharing: CoordinationScope) -> String {
    match sharing.to_normalized() {
        CoordinationScope::Free => "Free".to_string(),
        CoordinationScope::Shared => "Shared".to_string(),
        CoordinationScope::Level(level) => format!("Level({level})"),
    }
}

fn resolve_scale_domain_coordination(
    scale_name: &str,
    coordination: &DomainCoordination,
) -> Result<DomainCoordination, AvengerChartError> {
    let group = match &coordination.group {
        DomainCoordinationGroup::ScaleName => scale_name.to_string(),
        DomainCoordinationGroup::Named(group) => group.clone(),
    };
    Ok(DomainCoordination::new(
        coordination.scope,
        DomainCoordinationGroup::Named(group),
    ))
}

#[cfg(test)]
mod tests {
    use avenger_chart_core::{DefaultLogicalExprNodeExt, DomainCoordinationGroup};
    use avenger_chart_scales::Linear;
    use avenger_scenegraph::marks::mark::SceneMark;
    use datafusion::arrow::datatypes::DataType;
    use datafusion::prelude::{SessionContext, col, lit};

    use super::*;
    use crate::event as ev;
    use crate::event::ChartEventType;
    use crate::prelude::*;

    async fn data(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
        ctx.sql(
            "SELECT * FROM (VALUES
                ('A', 1.0, 2.0),
                ('A', 2.0, 3.0),
                ('B', 3.0, 4.0),
                ('B', 4.0, 5.0)
            ) AS t(group_name, x, y)",
        )
        .await
        .expect("data")
    }

    fn raw_domain_debug(compiled: &crate::plot::CompiledPlot, scale: &str) -> String {
        let PlotScaleSpec::Local(config) = compiled
            .scale_specs()
            .get(scale)
            .unwrap_or_else(|| panic!("missing scale {scale}"));
        let raw = config
            .domain
            .as_option()
            .and_then(|domain| domain.raw_domain.as_ref())
            .unwrap_or_else(|| panic!("missing raw_domain for {scale}"));
        format!(
            "{:?}",
            raw.to_expr(&SessionContext::new()).expect("raw expr")
        )
    }

    fn total_symbol_len(mark: &SceneMark) -> usize {
        match mark {
            SceneMark::Group(group) => group.marks.iter().map(total_symbol_len).sum(),
            SceneMark::Symbol(symbol) => symbol.len as usize,
            _ => 0,
        }
    }

    #[tokio::test]
    async fn pan_scroll_zoom_plain_cartesian_generates_params_bindings_and_raw_domains() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .data(df)
            .mark(Symbol::new().x(col("x")).y(col("y")))
            .tool(PanScrollZoom::cartesian())
            .compile(&ctx)
            .await
            .expect("compile");

        assert!(
            compiled
                .param_specs()
                .contains_key("__tool_pan_scroll_zoom__enabled")
        );
        assert!(
            compiled
                .param_specs()
                .contains_key("__tool_pan_scroll_zoom__x_domain")
        );
        assert!(
            compiled
                .param_specs()
                .contains_key("__tool_pan_scroll_zoom__y_domain")
        );
        assert_eq!(compiled.event_bindings().len(), 3);
        assert!(
            compiled
                .event_bindings()
                .iter()
                .any(|binding| binding.event_type == ChartEventType::DoubleClick)
        );
        assert_eq!(compiled.tool_metadata().len(), 1);
        assert!(raw_domain_debug(&compiled, "x").contains("__tool_pan_scroll_zoom__x_domain"));
        assert!(raw_domain_debug(&compiled, "y").contains("__tool_pan_scroll_zoom__y_domain"));
    }

    #[tokio::test]
    async fn leaf_tool_inside_facet_column_mirrors_shared_scale() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let leaf = crate::plot::Plot::<Cartesian>::new()
            .mark(
                Symbol::new()
                    .x_with(col("x"), |c| c.share_domain())
                    .y_with(col("y"), |c| c.free_domain()),
            )
            .tool(PanScrollZoom::cartesian());
        let compiled = crate::plot::Chart::<FacetColumn>::new()
            .data(df)
            .mark(Subplot::new(leaf).column(col("group_name")))
            .compile(&ctx)
            .await
            .expect("compile");

        assert_eq!(
            compiled.param_specs()["__tool_pan_scroll_zoom__x_domain"].sharing,
            CoordinationScope::Level(u8::MAX)
        );
        assert_eq!(
            compiled.param_specs()["__tool_pan_scroll_zoom__y_domain"].sharing,
            CoordinationScope::Level(0)
        );
    }

    #[tokio::test]
    async fn tool_raw_domain_param_mirrors_named_domain_group() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .data(df)
            .mark(Symbol::new().x_with(col("x"), |c| {
                c.with_domain_group("measurement").share_domain()
            }))
            .tool(PanScrollZoom::cartesian().x_only())
            .compile(&ctx)
            .await
            .expect("compile");

        let spec = &compiled.param_specs()["__tool_pan_scroll_zoom__domain__measurement"];
        assert_eq!(spec.sharing, CoordinationScope::Level(u8::MAX));
        let coordination = spec
            .domain_coordination
            .as_ref()
            .expect("domain coordination");
        assert_eq!(coordination.scope, CoordinationScope::Level(u8::MAX));
        assert_eq!(
            coordination.group,
            DomainCoordinationGroup::Named("measurement".to_string())
        );
    }

    #[tokio::test]
    async fn pan_scroll_zoom_generates_one_raw_domain_param_for_same_named_group() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .data(df)
            .mark(
                Symbol::new()
                    .x_with(col("x"), |c| {
                        c.with_domain_group("measurement").share_domain()
                    })
                    .y_with(col("y"), |c| {
                        c.with_domain_group("measurement").share_domain()
                    }),
            )
            .tool(PanScrollZoom::cartesian())
            .compile(&ctx)
            .await
            .expect("compile");

        assert!(
            compiled
                .param_specs()
                .contains_key("__tool_pan_scroll_zoom__domain__measurement")
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
        assert!(raw_domain_debug(&compiled, "x").contains("domain__measurement"));
        assert!(raw_domain_debug(&compiled, "y").contains("domain__measurement"));

        let drag = compiled
            .event_bindings()
            .iter()
            .find(|binding| binding.event_type == ChartEventType::CursorMoved)
            .expect("drag binding");
        assert_eq!(
            drag.action
                .param_steps()
                .filter(|assignment| assignment.param_name
                    == "__tool_pan_scroll_zoom__domain__measurement")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn raw_domain_param_rejects_incompatible_named_groups() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let domain = Param::raw_domain("shared_domain");
        let x_raw = domain.expr();
        let y_raw = domain.expr();
        let err = match crate::plot::Chart::<Cartesian>::new()
            .data(df)
            .param_with_sharing(domain, CoordinationScope::Shared)
            .mark(
                Symbol::new()
                    .x_with(col("x"), move |c| {
                        let x_raw = x_raw.clone();
                        c.with_domain_group("x_measure")
                            .share_domain()
                            .scale_with::<Linear>(move |s| s.raw_domain(x_raw.clone()))
                    })
                    .y_with(col("y"), move |c| {
                        let y_raw = y_raw.clone();
                        c.with_domain_group("y_measure")
                            .share_domain()
                            .scale_with::<Linear>(move |s| s.raw_domain(y_raw.clone()))
                    }),
            )
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("incompatible raw-domain groups should fail"),
            Err(err) => err,
        };

        assert!(err.to_string().contains("incompatible domain groups"));
    }

    #[tokio::test]
    async fn raw_domain_param_allows_same_named_group_across_channels() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let domain = Param::raw_domain("measurement_domain");
        let x_raw = domain.expr();
        let y_raw = domain.expr();
        crate::plot::Chart::<Cartesian>::new()
            .data(df)
            .param_with_sharing(domain, CoordinationScope::Shared)
            .mark(
                Symbol::new()
                    .x_with(col("x"), move |c| {
                        let x_raw = x_raw.clone();
                        c.with_domain_group("measurement")
                            .share_domain()
                            .scale_with::<Linear>(move |s| s.raw_domain(x_raw.clone()))
                    })
                    .y_with(col("y"), move |c| {
                        let y_raw = y_raw.clone();
                        c.with_domain_group("measurement")
                            .share_domain()
                            .scale_with::<Linear>(move |s| s.raw_domain(y_raw.clone()))
                    }),
            )
            .compile(&ctx)
            .await
            .expect("same named group can share one raw-domain param");
    }

    #[tokio::test]
    async fn leaf_tool_inside_facet_wrap_compiles() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let leaf = crate::plot::Plot::<Cartesian>::new()
            .mark(
                Symbol::new()
                    .x_with(col("x"), |c| c.share_domain())
                    .y_with(col("y"), |c| c.share_domain()),
            )
            .tool(PanScrollZoom::cartesian());
        let compiled = crate::plot::Chart::<FacetWrap>::new()
            .data(df)
            .mark(Subplot::new(leaf).wrap(col("group_name")))
            .compile(&ctx)
            .await
            .expect("compile");

        assert_eq!(
            compiled.param_specs()["__tool_pan_scroll_zoom__x_domain"].sharing,
            CoordinationScope::Level(u8::MAX)
        );
        assert!(compiled.event_bindings().len() >= 2);
    }

    #[tokio::test]
    async fn leaf_tool_inside_nested_row_column_uses_level_sharing() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let leaf = crate::plot::Plot::<Cartesian>::new()
            .mark(
                Symbol::new()
                    .x_with(col("x"), |c| {
                        c.with_domain_scope(CoordinationScope::Level(1))
                    })
                    .y_with(col("y"), |c| {
                        c.with_domain_scope(CoordinationScope::Level(1))
                    }),
            )
            .tool(PanScrollZoom::cartesian());
        let column = crate::plot::Plot::<FacetColumn>::new()
            .mark(Subplot::new(leaf).column(col("group_name")));
        let compiled = crate::plot::Chart::<FacetRow>::new()
            .data(df)
            .mark(Subplot::new(column).row(col("group_name")))
            .compile(&ctx)
            .await
            .expect("compile");

        assert_eq!(
            compiled.param_specs()["__tool_pan_scroll_zoom__x_domain"].sharing,
            CoordinationScope::Level(1)
        );
        assert_eq!(
            compiled.param_specs()["__tool_pan_scroll_zoom__y_domain"].sharing,
            CoordinationScope::Level(1)
        );
    }

    #[derive(Clone)]
    struct CustomXTool;

    impl ChartTool<Cartesian> for CustomXTool {
        fn id(&self) -> &str {
            "custom_x"
        }

        fn expand(
            &self,
            ctx: ToolExpansionContext<'_>,
        ) -> Result<ToolBehaviorExpansion<Cartesian>, AvengerChartError> {
            let param = Param::raw_domain("__tool_custom_x__x_domain");
            Ok(ToolBehaviorExpansion::new(ctx.instance_id.clone())
                .param(param.clone(), ToolParamSharing::mirror_scale("x"))
                .scale_edit(ToolScaleEdit::raw_domain("x", param.name.clone()))
                .event_binding(
                    ChartEventBinding::on(ChartEventType::CanvasResize)
                        .set_param(&param, ev::interval(lit(0.0), lit(1.0)))
                        .preview(),
                ))
        }
    }

    #[tokio::test]
    async fn public_custom_tool_can_expand_to_param_binding_and_scale_edit() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .data(df)
            .mark(Symbol::new().x(col("x")).y(col("y")))
            .tool(CustomXTool)
            .compile(&ctx)
            .await
            .expect("compile");

        assert!(
            compiled
                .param_specs()
                .contains_key("__tool_custom_x__x_domain")
        );
        assert_eq!(compiled.event_bindings().len(), 1);
        assert!(raw_domain_debug(&compiled, "x").contains("__tool_custom_x__x_domain"));
    }

    #[derive(Clone)]
    struct CustomSelectionTool;

    impl ChartTool<Cartesian> for CustomSelectionTool {
        fn id(&self) -> &str {
            "custom_selection"
        }

        fn expand(
            &self,
            ctx: ToolExpansionContext<'_>,
        ) -> Result<ToolBehaviorExpansion<Cartesian>, AvengerChartError> {
            let store = Store::empty("__tool_custom_selection__boxes")
                .field("id", DataType::Utf8, false)
                .field("x_min", DataType::Float64, false)
                .field("x_max", DataType::Float64, false)
                .field("y_min", DataType::Float64, false)
                .field("y_max", DataType::Float64, false)
                .primary_key(["id"]);
            let selection =
                Selection::new("__tool_custom_selection__brush").empty_selects_nothing();
            Ok(ToolBehaviorExpansion::new(ctx.instance_id.clone())
                .store(store)
                .selection(selection))
        }
    }

    #[tokio::test]
    async fn leaf_tool_inside_facet_can_expand_to_store_and_selection() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let leaf = crate::plot::Plot::<Cartesian>::new()
            .mark(Symbol::new().x(col("x")).y(col("y")))
            .tool(CustomSelectionTool);
        let compiled = crate::plot::Chart::<FacetColumn>::new()
            .data(df)
            .mark(Subplot::new(leaf).column(col("group_name")))
            .compile(&ctx)
            .await
            .expect("compile");

        assert!(
            compiled
                .store_specs()
                .contains_key("__tool_custom_selection__boxes")
        );
        assert!(
            compiled
                .selection_specs()
                .contains_key("__tool_custom_selection__brush")
        );
    }

    #[derive(Clone)]
    struct IdentityTool {
        id: String,
        nested: Option<Arc<dyn ChartTool<Cartesian>>>,
    }

    impl IdentityTool {
        fn leaf(id: &str) -> Self {
            Self {
                id: id.to_string(),
                nested: None,
            }
        }

        fn parent(id: &str, nested: Arc<dyn ChartTool<Cartesian>>) -> Self {
            Self {
                id: id.to_string(),
                nested: Some(nested),
            }
        }
    }

    impl ChartTool<Cartesian> for IdentityTool {
        fn id(&self) -> &str {
            &self.id
        }

        fn expand(
            &self,
            ctx: ToolExpansionContext<'_>,
        ) -> Result<ToolBehaviorExpansion<Cartesian>, AvengerChartError> {
            let param = Param::new(format!("{}__enabled", self.id), true);
            let mut behavior = ToolBehaviorExpansion::new(ctx.instance_id.clone())
                .component("identity-tool", self.id.clone())
                .param(param, ToolParamSharing::Explicit(CoordinationScope::Shared));
            if let Some(nested) = &self.nested {
                behavior = behavior.nested_tool(nested.clone());
            }
            Ok(behavior)
        }
    }

    #[tokio::test]
    async fn tool_instances_own_distinct_state_and_exports_survive_serialization() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .data(df)
            .mark(Symbol::new().x(col("x")).y(col("y")))
            .tool(IdentityTool::leaf("left"))
            .tool(IdentityTool::leaf("right"))
            .compile(&ctx)
            .await
            .expect("compile identity tools");

        assert_eq!(compiled.tool_behaviors().len(), 2);
        assert_ne!(
            compiled.tool_behaviors()[0].instance_id,
            compiled.tool_behaviors()[1].instance_id
        );
        for behavior in compiled.tool_behaviors() {
            let export = behavior.exports.first().expect("state export");
            let ToolExportTarget::Param(exported_id) = &export.target else {
                panic!("expected param export")
            };
            assert_eq!(
                compiled.param_specs().resolve_source_name(&export.alias),
                Some(exported_id)
            );
        }

        let restored: crate::plot::CompiledPlot =
            bincode::deserialize(&bincode::serialize(&compiled).unwrap()).unwrap();
        assert_eq!(restored.tool_behaviors(), compiled.tool_behaviors());
    }

    #[tokio::test]
    async fn nested_tool_identity_retains_deterministic_ancestry() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .data(df)
            .mark(Symbol::new().x(col("x")).y(col("y")))
            .tool(IdentityTool::parent(
                "parent",
                Arc::new(IdentityTool::leaf("child")),
            ))
            .compile(&ctx)
            .await
            .expect("compile nested tool");
        let parent = &compiled.tool_behaviors()[0];
        let child = &compiled.tool_behaviors()[1];
        assert!(parent.instance_ancestry.is_empty());
        assert_eq!(child.instance_ancestry, vec![parent.instance_id.clone()]);
        assert_ne!(child.instance_id, parent.instance_id);
    }

    #[tokio::test]
    async fn point_selection_tool_contributes_selection_bindings_and_metadata() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let picked = PointSelection::new("picked").field("group_name");
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .data(df)
            .mark(
                Symbol::new()
                    .x(col("x"))
                    .y(col("y"))
                    .fill_with(lit("#b8beca"), |c| {
                        c.no_scale()
                            .when_value(picked.predicate(), lit("#2563eb"))
                            .no_legend()
                    }),
            )
            .tool(picked)
            .compile(&ctx)
            .await
            .expect("compile");

        assert!(
            compiled
                .param_specs()
                .contains_key("__tool_picked__enabled")
        );
        assert!(
            !compiled.param_specs().contains_key("__tool_picked__cursor"),
            "cursor effects must not mint a conventionally named parameter"
        );
        assert!(compiled.event_bindings().iter().any(|binding| {
            binding
                .action
                .steps
                .iter()
                .any(|step| matches!(step, avenger_chart_core::ChartActionStep::SetCursor(_)))
        }));
        assert!(compiled.selection_specs().contains_key("picked"));
        assert_eq!(compiled.event_bindings().len(), 5);
        assert_eq!(compiled.tool_metadata().len(), 1);
        assert_eq!(compiled.tool_metadata()[0].id, "picked");
        assert_eq!(
            compiled.event_datum_types().get("group_name"),
            Some(&DataType::Utf8),
            "point selection should request clicked datum values"
        );
    }

    #[tokio::test]
    async fn box_selection_tool_compiles_inside_repeat_grid() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let brush = BoxSelection::cartesian("brush")
            .dimensions(repeat::column(), repeat::row())
            .resolve(BoxSelectionResolve::Union)
            .repeat_cell_chrome();
        let cell = crate::plot::Plot::<Cartesian>::new()
            .mark(Symbol::new().x(repeat::column()).y(repeat::row()))
            .tool(brush);
        let compiled = crate::plot::Chart::<RepeatGrid>::new()
            .data(df)
            .configure_coord(|c| {
                c.rows([RepeatVariable::new("x", col("x"))])
                    .columns([RepeatVariable::new("y", col("y"))])
                    .cell(cell)
            })
            .compile(&ctx)
            .await
            .expect("compile");

        assert!(compiled.param_specs().contains_key("__tool_brush__enabled"));
        assert!(compiled.store_specs().contains_key("__tool_brush__store"));
        assert!(compiled.selection_specs().contains_key("brush"));
        assert_eq!(compiled.event_bindings().len(), 3);
        assert_eq!(compiled.tool_metadata().len(), 1);
        assert_eq!(compiled.tool_metadata()[0].id, "brush");
    }

    #[tokio::test]
    async fn box_zoom_cartesian_injects_unit_rect_mark_and_raw_domains() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .data(df)
            .mark(Symbol::new().x(col("x")).y(col("y")))
            .tool(BoxZoom::cartesian())
            .compile(&ctx)
            .await
            .expect("compile");

        assert!(
            compiled
                .param_specs()
                .contains_key("__tool_box_zoom__active")
        );
        assert!(
            compiled
                .param_specs()
                .contains_key("__tool_box_zoom__x_domain")
        );
        assert_eq!(compiled.event_bindings().len(), 5);
        let overlay = compiled
            .marks()
            .iter()
            .find(|mark| mark.mark_type() == "rect")
            .expect("box-zoom overlay mark");
        let component = overlay
            .state()
            .identity
            .component
            .as_ref()
            .expect("tool component provenance");
        assert_eq!(component.component_kind, "box_zoom");
        assert_eq!(component.part_alias, "selection");
        assert!(raw_domain_debug(&compiled, "x").contains("__tool_box_zoom__x_domain"));
        assert!(raw_domain_debug(&compiled, "y").contains("__tool_box_zoom__y_domain"));
    }

    #[tokio::test]
    async fn box_zoom_unit_aspect_requires_coordinate_constraint() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let err = match crate::plot::Chart::<Cartesian>::new()
            .data(df)
            .mark(Symbol::new().x(col("x")).y(col("y")))
            .tool(BoxZoom::cartesian().unit_aspect())
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("unit-aspect box zoom should require a coordinate constraint"),
            Err(err) => err,
        };

        assert!(err.to_string().contains("no active coordinate metric"));
    }

    #[tokio::test]
    async fn box_zoom_unit_aspect_receives_coordinate_constraint() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let compiled = crate::plot::Chart::with_coord(Cartesian::new().unit_aspect(1.0))
            .data(df)
            .mark(Symbol::new().x(col("x")).y(col("y")))
            .tool(BoxZoom::cartesian().unit_aspect())
            .compile(&ctx)
            .await
            .expect("compile");

        let drag = compiled
            .event_bindings()
            .iter()
            .find(|binding| binding.event_type == ChartEventType::CursorMoved)
            .expect("drag binding");
        assert_eq!(
            drag.filters.len(),
            9,
            "unit-aspect box zoom should request start-domain and start-plot-size guards"
        );
    }

    #[tokio::test]
    async fn unit_data_mark_renders_once_without_inherited_rows() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .data(df)
            .mark(
                Symbol::new()
                    .unit_data()
                    .x_with(lit(10.0), |c| c.no_scale())
                    .y_with(lit(10.0), |c| c.no_scale()),
            )
            .compile(&ctx)
            .await
            .expect("compile");

        let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate");
        let symbol_count: usize = evaluated
            .scene_graph
            .marks
            .iter()
            .map(total_symbol_len)
            .sum();
        assert_eq!(symbol_count, 1);
    }

    #[tokio::test]
    async fn visible_false_suppresses_mark_rendering() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .data(df)
            .mark(
                Symbol::new()
                    .unit_data()
                    .x_with(lit(10.0), |c| c.no_scale())
                    .y_with(lit(10.0), |c| c.no_scale())
                    .visible(lit(false)),
            )
            .compile(&ctx)
            .await
            .expect("compile");

        let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate");
        let symbol_count: usize = evaluated
            .scene_graph
            .marks
            .iter()
            .map(total_symbol_len)
            .sum();
        assert_eq!(symbol_count, 0);
    }

    #[tokio::test]
    async fn duplicate_tool_ids_error() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let result = crate::plot::Chart::<Cartesian>::new()
            .data(df)
            .mark(Symbol::new().x(col("x")).y(col("y")))
            .tool(PanScrollZoom::cartesian())
            .tool(PanScrollZoom::cartesian())
            .compile(&ctx)
            .await;
        let Err(err) = result else {
            panic!("duplicate ids should fail");
        };

        assert!(err.to_string().contains("Duplicate chart tool id"));
    }

    #[tokio::test]
    async fn invalid_tool_id_errors() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let result = crate::plot::Chart::<Cartesian>::new()
            .data(df)
            .mark(Symbol::new().x(col("x")).y(col("y")))
            .tool(PanScrollZoom::cartesian().id("bad.id"))
            .compile(&ctx)
            .await;
        let Err(err) = result else {
            panic!("invalid tool id should fail");
        };

        assert!(err.to_string().contains("Invalid chart tool id"));
    }

    #[tokio::test]
    async fn pan_scroll_zoom_errors_without_matching_target() {
        let ctx = SessionContext::new();
        let result = crate::plot::Chart::<Cartesian>::new()
            .tool(PanScrollZoom::cartesian())
            .compile(&ctx)
            .await;
        let Err(err) = result else {
            panic!("no x/y scales should fail");
        };

        assert!(
            err.to_string()
                .contains("did not find an invertible scale target")
        );
    }

    #[tokio::test]
    async fn explicit_tool_param_sharing_must_cover_target_scale_sharing() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let result = crate::plot::Chart::<Cartesian>::new()
            .data(df)
            .mark(
                Symbol::new()
                    .x_with(col("x"), |c| c.share_domain())
                    .y(col("y")),
            )
            .tool(
                PanScrollZoom::cartesian()
                    .x_only()
                    .x_sharing(CoordinationScope::Free),
            )
            .compile(&ctx)
            .await;
        let Err(err) = result else {
            panic!("too-narrow explicit sharing should fail");
        };

        assert!(
            err.to_string()
                .contains("must be shared at least as broadly")
        );
    }

    #[tokio::test]
    async fn existing_different_raw_domain_conflicts() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let result = crate::plot::Chart::<Cartesian>::new()
            .data(df)
            .mark(
                Symbol::new()
                    .x_with(col("x"), |c| {
                        c.scale_with::<Linear>(|s| s.raw_domain(ev::interval(lit(0.0), lit(1.0))))
                    })
                    .y(col("y")),
            )
            .tool(PanScrollZoom::cartesian().x_only())
            .compile(&ctx)
            .await;
        let Err(err) = result else {
            panic!("existing raw_domain should conflict");
        };

        assert!(
            err.to_string()
                .contains("already has a different raw_domain")
        );
    }
}
