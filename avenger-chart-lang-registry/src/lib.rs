//! Explicitly composed native authoring schemas paired with Rust lowerers.

use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
};

use async_trait::async_trait;
use avenger_chart::{layout::LayoutSpec, plot::CompiledPlot, prelude::*};
use avenger_chart_core::{
    CompiledDataTransform, CoordinateSystem, DataContext, DataTransformCompileContext,
    DataTransformStage, MarkAdjustmentCompileContext, MarkDataMode, SubplotChildPlotSpec,
};
pub use avenger_chart_lang_types::{
    AdjustmentLanguageDefinition, CoordinateLanguageDefinition, LoweredAdjustment,
    LoweredTransform, MarkLanguageLowerer, NativeLoweringError, NativeOutputValue,
    ObjectLanguageDefinition, ResolvedDeclaration, ResolvedProjectionItem, ResolvedValue,
    TransformLanguageDefinition, TransformPipelineLanguageDefinition, WidgetLanguageDefinition,
};
use avenger_chart_schema::{
    KindSchema, NativeKindKey, NativeKindNamespace, NativeSchemaSnapshot, SchemaVersion, ValueShape,
};
pub use avenger_chart_schema::{
    NativeModuleExport, NativeModuleId, NativeModuleImplementationProfileId, NativeModuleSchema,
    NativeModuleSchemaProfileId,
};
use datafusion::{dataframe::DataFrame, logical_expr::Expr, prelude::SessionContext};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub mod builtins;

#[derive(Clone, Debug)]
pub struct ResolvedChildPlot {
    pub plot: Box<ResolvedPlot>,
    pub placement: ResolvedDeclaration,
}

#[derive(Clone)]
pub struct ResolvedTransformStage {
    pub scope: CoordinationScope,
    pub transform: Box<dyn CompiledDataTransform>,
}

impl fmt::Debug for ResolvedTransformStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResolvedTransformStage")
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub struct ResolvedMarkGroup {
    pub id: Option<String>,
    pub publish_id: bool,
    pub component_kind: Option<String>,
    pub data: Option<DataFrame>,
    pub store_data: Option<avenger_chart_core::StoreData>,
    pub data_mode: MarkDataMode,
    pub transforms: Vec<ResolvedTransformStage>,
    pub view: Option<ResolvedViewScope>,
    pub marks: Vec<ResolvedMark>,
}

#[derive(Clone, Debug)]
pub struct ResolvedViewScope {
    pub spec: avenger_chart_core::CompiledViewSpec,
    pub data: Option<DataFrame>,
    pub store_data: Option<avenger_chart_core::StoreData>,
    pub transforms: Vec<ResolvedTransformStage>,
}

impl ResolvedMarkGroup {
    pub fn new() -> Self {
        Self {
            id: None,
            publish_id: true,
            component_kind: None,
            data: None,
            store_data: None,
            data_mode: MarkDataMode::Inherit,
            transforms: Vec::new(),
            view: None,
            marks: Vec::new(),
        }
    }
}

impl Default for ResolvedMarkGroup {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
struct LanguageBehaviorTool<C: CoordinateSystem> {
    id: String,
    component_kind: String,
    component_id: Option<String>,
    state: Vec<ResolvedBehaviorState>,
    event_bindings: Vec<ChartEventBinding>,
    param_change_bindings: Vec<ChartParamChangeBinding>,
    scale_edits: Vec<ToolScaleEdit>,
    chrome: Vec<PlotMark<C>>,
    nested_tools: Vec<Arc<dyn ChartTool<C>>>,
    exports: Vec<ResolvedBehaviorExport>,
    metadata: Vec<ToolMetadata>,
}

impl<C: CoordinateSystem> ChartTool<C> for LanguageBehaviorTool<C> {
    fn id(&self) -> &str {
        &self.id
    }

    fn expand(
        &self,
        context: ToolExpansionContext<'_>,
    ) -> Result<ToolBehaviorExpansion<C>, avenger_chart_core::AvengerChartError> {
        let mut expansion = ToolBehaviorExpansion::new(context.instance_id.clone())
            .with_instance_ancestry(context.instance_ancestry.clone());
        expansion.component_kind = self.component_kind.clone();
        expansion.component_id = self.component_id.clone().or_else(|| Some(self.id.clone()));
        let mut targets = BTreeMap::new();
        let mut param_ordinal = 0_u64;
        let mut store_ordinal = 0_u64;
        let mut selection_ordinal = 0_u64;
        for state in &self.state {
            match state {
                ResolvedBehaviorState::Param {
                    key,
                    param,
                    sharing,
                } => {
                    let runtime_id = CompiledIdentityAllocator::derive_tool_param(
                        &context.instance_id,
                        param_ordinal,
                    );
                    param_ordinal += 1;
                    targets.insert(key.clone(), ToolExportTarget::Param(runtime_id.clone()));
                    expansion.state.push(ResolvedStateDeclaration::Param {
                        runtime_id,
                        param: param.clone(),
                        sharing: sharing.clone(),
                    });
                }
                ResolvedBehaviorState::Store { key, store } => {
                    let runtime_id = CompiledIdentityAllocator::derive_tool_store(
                        &context.instance_id,
                        store_ordinal,
                    );
                    store_ordinal += 1;
                    targets.insert(key.clone(), ToolExportTarget::Store(runtime_id.clone()));
                    expansion.state.push(ResolvedStateDeclaration::Store {
                        runtime_id,
                        store: store.clone(),
                    });
                }
                ResolvedBehaviorState::Selection { key, selection } => {
                    let runtime_id = CompiledIdentityAllocator::derive_tool_selection(
                        &context.instance_id,
                        selection_ordinal,
                    );
                    selection_ordinal += 1;
                    targets.insert(key.clone(), ToolExportTarget::Selection(runtime_id.clone()));
                    expansion.state.push(ResolvedStateDeclaration::Selection {
                        runtime_id,
                        selection: selection.clone(),
                    });
                }
            }
        }
        for export in &self.exports {
            let key = match &export.target {
                ResolvedBehaviorExportTarget::Param(key)
                | ResolvedBehaviorExportTarget::Store(key)
                | ResolvedBehaviorExportTarget::Selection(key) => key,
            };
            let target = targets.get(key).cloned().ok_or_else(|| {
                avenger_chart_core::AvengerChartError::InternalError(format!(
                    "resolved tool behavior export `{}` has no owned state target",
                    export.alias
                ))
            })?;
            expansion.exports.push(ToolExport {
                alias: export.alias.clone(),
                target,
            });
        }
        expansion.event_bindings = self.event_bindings.clone();
        expansion.param_change_bindings = self.param_change_bindings.clone();
        expansion.scale_edits = self.scale_edits.clone();
        expansion.chrome = self.chrome.clone();
        expansion.nested_tools = self.nested_tools.clone();
        expansion.metadata = self.metadata.clone();
        Ok(expansion)
    }
}

#[derive(Clone, Debug)]
pub enum ResolvedMark {
    Native(ResolvedDeclaration),
    NativeWithChild {
        declaration: ResolvedDeclaration,
        child: Box<ResolvedPlot>,
    },
    Group(Box<ResolvedMarkGroup>),
}

/// One behavior-owned state declaration after language resolution.
#[derive(Clone, Debug)]
pub enum ResolvedBehaviorState {
    Param {
        key: String,
        param: Param,
        sharing: ToolParamSharing,
    },
    Store {
        key: String,
        store: Store,
    },
    Selection {
        key: String,
        selection: Selection,
    },
}

/// Exact public state alias emitted by an ordinary `tool behavior`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolvedBehaviorExportTarget {
    Param(String),
    Store(String),
    Selection(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedBehaviorExport {
    pub alias: String,
    pub target: ResolvedBehaviorExportTarget,
}

/// Coordinate-neutral canonical behavior carried from the language compiler
/// to the coordinate-owned native lowering boundary.
#[derive(Clone, Debug)]
pub struct ResolvedToolBehavior {
    pub id: String,
    pub component_kind: String,
    pub component_id: Option<String>,
    pub state: Vec<ResolvedBehaviorState>,
    pub event_bindings: Vec<ChartEventBinding>,
    pub param_change_bindings: Vec<ChartParamChangeBinding>,
    pub scale_edits: Vec<ToolScaleEdit>,
    pub chrome: Vec<ResolvedMark>,
    pub native_tools: Vec<ResolvedDeclaration>,
    pub nested_behaviors: Vec<ResolvedToolBehavior>,
    pub exports: Vec<ResolvedBehaviorExport>,
    pub metadata: Vec<ToolMetadata>,
}

impl ResolvedToolBehavior {
    pub fn new(id: impl Into<String>, component_kind: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            component_kind: component_kind.into(),
            component_id: None,
            state: Vec::new(),
            event_bindings: Vec::new(),
            param_change_bindings: Vec::new(),
            scale_edits: Vec::new(),
            chrome: Vec::new(),
            native_tools: Vec::new(),
            nested_behaviors: Vec::new(),
            exports: Vec::new(),
            metadata: Vec::new(),
        }
    }
}

impl From<ResolvedDeclaration> for ResolvedMark {
    fn from(value: ResolvedDeclaration) -> Self {
        Self::Native(value)
    }
}

#[derive(Clone, Debug, Default)]
pub struct ResolvedRootFurnishings {
    pub title: Option<Expr>,
    pub subtitle: Option<Expr>,
    pub layout: Option<LayoutSpec>,
    pub theme: Option<avenger_chart_core::Theme>,
    pub time_context: avenger_chart_core::TimeContext,
    pub formatting_context: avenger_chart_core::FormattingContext,
    pub params: Vec<(Param, CoordinationScope)>,
    pub selections: Vec<Selection>,
    pub stores: Vec<Store>,
    pub event_bindings: Vec<ChartEventBinding>,
}

#[derive(Clone, Debug)]
pub struct ResolvedPlot {
    pub coordinate: ResolvedDeclaration,
    pub data: Option<DataFrame>,
    /// True when `data` exists only to plan and lower a lexically inherited
    /// relation. Parent-context containers such as facets remove it before
    /// constructing the runtime child plot so filtered parent data can flow.
    pub data_is_inherited: bool,
    pub marks: Vec<ResolvedMark>,
    pub tools: Vec<ResolvedDeclaration>,
    pub tool_behaviors: Vec<ResolvedToolBehavior>,
    pub widgets: Vec<ResolvedDeclaration>,
    pub children: Vec<ResolvedChildPlot>,
    pub furnishings: ResolvedRootFurnishings,
}

impl ResolvedPlot {
    pub fn new(coordinate_kind: impl Into<String>) -> Self {
        Self {
            coordinate: ResolvedDeclaration::new(coordinate_kind),
            data: None,
            data_is_inherited: false,
            marks: Vec::new(),
            tools: Vec::new(),
            tool_behaviors: Vec::new(),
            widgets: Vec::new(),
            children: Vec::new(),
            furnishings: ResolvedRootFurnishings::default(),
        }
    }
}

pub type NativeTransformLowerer = Arc<
    dyn Fn(
            &ResolvedDeclaration,
            DataTransformCompileContext,
        ) -> Result<LoweredTransform, RegistryError>
        + Send
        + Sync,
>;
pub type NativeTransformPipelineLowerer = Arc<
    dyn Fn(
            &ResolvedDeclaration,
            Vec<DataTransformStage>,
            IndexMap<String, Expr>,
            DataTransformCompileContext,
        ) -> Result<LoweredTransform, RegistryError>
        + Send
        + Sync,
>;
pub type NativeAdjustmentLowerer = Arc<
    dyn Fn(
            &ResolvedDeclaration,
            MarkAdjustmentCompileContext,
        ) -> Result<LoweredAdjustment, RegistryError>
        + Send
        + Sync,
>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeTransformMode {
    Leaf,
    Pipeline,
}
pub type NativeWidgetLowerer =
    Arc<dyn Fn(&ResolvedDeclaration) -> Result<WidgetAttachment, RegistryError> + Send + Sync>;
pub type NativeObjectLowerer = Arc<
    dyn Fn(&ResolvedDeclaration) -> Result<Box<dyn Any + Send + Sync>, RegistryError> + Send + Sync,
>;

type NativeMarkLowerer<C> =
    Arc<dyn Fn(&C, &ResolvedDeclaration) -> Result<Vec<PlotMark<C>>, RegistryError> + Send + Sync>;
type NativeChildMarkLowerer<C> = Arc<
    dyn Fn(
            &ResolvedDeclaration,
            Box<dyn SubplotChildPlotSpec>,
        ) -> Result<Vec<PlotMark<C>>, RegistryError>
        + Send
        + Sync,
>;
type NativeToolLowerer<C> =
    Arc<dyn Fn(&ResolvedDeclaration) -> Result<Arc<dyn ChartTool<C>>, RegistryError> + Send + Sync>;
type CoordinateLowerer<C> =
    Arc<dyn Fn(&ResolvedDeclaration) -> Result<C, RegistryError> + Send + Sync>;
type ChildPlotLowerer<C> = Arc<
    dyn Fn(
            Plot<C>,
            Box<dyn SubplotChildPlotSpec>,
            &ResolvedDeclaration,
            &ResolvedPlot,
        ) -> Result<Plot<C>, RegistryError>
        + Send
        + Sync,
>;
type PlotPropertyLowerer<C> =
    Arc<dyn Fn(Plot<C>, &ResolvedPlot) -> Result<Plot<C>, RegistryError> + Send + Sync>;

struct TypedMarkEntry<C: CoordinateSystem> {
    schema: KindSchema,
    lowerer: NativeMarkLowerer<C>,
}

struct TypedChildMarkEntry<C: CoordinateSystem> {
    schema: KindSchema,
    lowerer: NativeChildMarkLowerer<C>,
}

struct TypedToolEntry<C: CoordinateSystem> {
    schema: KindSchema,
    lowerer: NativeToolLowerer<C>,
}

/// Typed host pack that preserves `Plot<C>` and `PlotMark<C>` until the whole
/// plot crosses an object-safe root/child boundary.
pub struct CoordinatePack<C: CoordinateSystem> {
    kind: String,
    schema: KindSchema,
    coordinate_lowerer: CoordinateLowerer<C>,
    marks: BTreeMap<String, TypedMarkEntry<C>>,
    child_marks: BTreeMap<String, TypedChildMarkEntry<C>>,
    tools: BTreeMap<String, TypedToolEntry<C>>,
    child_plot_lowerer: Option<ChildPlotLowerer<C>>,
    children_use_parent_data_context: bool,
    plot_property_lowerer: Option<PlotPropertyLowerer<C>>,
    duplicate_keys: Vec<NativeKindKey>,
}

impl<C: CoordinateSystem> CoordinatePack<C> {
    pub fn from_language_definition(definition: CoordinateLanguageDefinition<C>) -> Self {
        let CoordinateLanguageDefinition {
            kind,
            schema,
            lowerer,
            marks,
            tools,
        } = definition;
        let mut pack = Self::new(kind, schema, move |declaration| {
            lowerer(declaration).map_err(RegistryError::from)
        });
        for mark in marks {
            pack = match mark.lowerer {
                MarkLanguageLowerer::Declaration(lowerer) => {
                    pack.mark(mark.kind, mark.schema, move |declaration| {
                        lowerer(declaration).map_err(RegistryError::from)
                    })
                }
                MarkLanguageLowerer::Coordinate(lowerer) => pack.mark_with_coordinate(
                    mark.kind,
                    mark.schema,
                    move |coordinate, declaration| {
                        lowerer(coordinate, declaration).map_err(RegistryError::from)
                    },
                ),
            };
        }
        for tool in tools {
            pack = pack.tool(tool.kind, tool.schema, move |declaration| {
                (tool.lowerer)(declaration).map_err(RegistryError::from)
            });
        }
        pack
    }

    pub fn new(
        kind: impl Into<String>,
        mut schema: KindSchema,
        lowerer: impl Fn(&ResolvedDeclaration) -> Result<C, RegistryError> + Send + Sync + 'static,
    ) -> Self {
        for (name, property) in avenger_chart_schema::chart_core_properties() {
            schema.properties.entry(name).or_insert(property);
        }
        Self {
            kind: kind.into(),
            schema,
            coordinate_lowerer: Arc::new(lowerer),
            marks: BTreeMap::new(),
            child_marks: BTreeMap::new(),
            tools: BTreeMap::new(),
            child_plot_lowerer: None,
            children_use_parent_data_context: false,
            plot_property_lowerer: None,
            duplicate_keys: Vec::new(),
        }
    }

    pub fn mark(
        mut self,
        kind: impl Into<String>,
        schema: KindSchema,
        lowerer: impl Fn(&ResolvedDeclaration) -> Result<Vec<PlotMark<C>>, RegistryError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        let kind = kind.into();
        if self
            .marks
            .insert(
                kind,
                TypedMarkEntry {
                    schema: schema.clone(),
                    lowerer: Arc::new(move |_coordinate, declaration| lowerer(declaration)),
                },
            )
            .is_some()
        {
            self.duplicate_keys.push(schema.key);
        }
        self
    }

    pub fn mark_with_coordinate(
        mut self,
        kind: impl Into<String>,
        schema: KindSchema,
        lowerer: impl Fn(&C, &ResolvedDeclaration) -> Result<Vec<PlotMark<C>>, RegistryError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        let kind = kind.into();
        if self
            .marks
            .insert(
                kind,
                TypedMarkEntry {
                    schema: schema.clone(),
                    lowerer: Arc::new(lowerer),
                },
            )
            .is_some()
        {
            self.duplicate_keys.push(schema.key);
        }
        self
    }

    pub fn child_mark(
        mut self,
        kind: impl Into<String>,
        schema: KindSchema,
        lowerer: impl Fn(
            &ResolvedDeclaration,
            Box<dyn SubplotChildPlotSpec>,
        ) -> Result<Vec<PlotMark<C>>, RegistryError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        let kind = kind.into();
        if self
            .child_marks
            .insert(
                kind,
                TypedChildMarkEntry {
                    schema: schema.clone(),
                    lowerer: Arc::new(lowerer),
                },
            )
            .is_some()
        {
            self.duplicate_keys.push(schema.key);
        }
        self
    }

    pub fn tool(
        mut self,
        kind: impl Into<String>,
        schema: KindSchema,
        lowerer: impl Fn(&ResolvedDeclaration) -> Result<Arc<dyn ChartTool<C>>, RegistryError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        let kind = kind.into();
        if self
            .tools
            .insert(
                kind,
                TypedToolEntry {
                    schema: schema.clone(),
                    lowerer: Arc::new(lowerer),
                },
            )
            .is_some()
        {
            self.duplicate_keys.push(schema.key);
        }
        self
    }

    pub fn child_plots(
        mut self,
        lowerer: impl Fn(
            Plot<C>,
            Box<dyn SubplotChildPlotSpec>,
            &ResolvedDeclaration,
            &ResolvedPlot,
        ) -> Result<Plot<C>, RegistryError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.child_plot_lowerer = Some(Arc::new(lowerer));
        self
    }

    /// Child plots use the container's compiled data context rather than an
    /// attached copy of lexically inherited plot-level data. Explicit child
    /// data remains attached so the runtime can reject it where unsupported.
    pub fn children_use_parent_data_context(mut self) -> Self {
        self.children_use_parent_data_context = true;
        self
    }

    pub fn plot_properties(
        mut self,
        lowerer: impl Fn(Plot<C>, &ResolvedPlot) -> Result<Plot<C>, RegistryError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.plot_property_lowerer = Some(Arc::new(lowerer));
        self
    }
}

#[async_trait]
trait ErasedCoordinatePack: Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn schemas(&self) -> Vec<KindSchema>;
    fn validate_registration(&self) -> Result<(), RegistryError>;
    fn lower_child(
        &self,
        registry: &NativeRegistry,
        plot: &ResolvedPlot,
    ) -> Result<Box<dyn SubplotChildPlotSpec>, RegistryError>;
    async fn compile_root(
        &self,
        registry: &NativeRegistry,
        plot: &ResolvedPlot,
        session_context: &SessionContext,
    ) -> Result<CompiledPlot, RegistryError>;
}

impl<C: CoordinateSystem> CoordinatePack<C> {
    fn lower_mark(
        &self,
        registry: &NativeRegistry,
        coordinate: &C,
        resolved: &ResolvedMark,
    ) -> Result<Vec<PlotMark<C>>, RegistryError> {
        match resolved {
            ResolvedMark::Native(declaration) => {
                let entry = self.marks.get(&declaration.kind).ok_or_else(|| {
                    RegistryError::UnknownMarkPair {
                        coordinate: self.kind.clone(),
                        mark: declaration.kind.clone(),
                    }
                })?;
                registry.validate(&entry.schema.key, declaration)?;
                (entry.lowerer)(coordinate, declaration).map(|marks| {
                    marks
                        .into_iter()
                        .map(|mark| {
                            mark.with_language_identity(
                                declaration.publish_source_name,
                                declaration.public_aliases.clone(),
                                declaration.component_part_alias.clone(),
                            )
                        })
                        .collect()
                })
            }
            ResolvedMark::NativeWithChild { declaration, child } => {
                let entry = self.child_marks.get(&declaration.kind).ok_or_else(|| {
                    RegistryError::UnknownMarkPair {
                        coordinate: self.kind.clone(),
                        mark: declaration.kind.clone(),
                    }
                })?;
                registry.validate(&entry.schema.key, declaration)?;
                let mut inherited_child;
                let child = if child.data_is_inherited {
                    inherited_child = child.as_ref().clone();
                    inherited_child.data = None;
                    inherited_child.data_is_inherited = false;
                    &inherited_child
                } else {
                    child.as_ref()
                };
                (entry.lowerer)(declaration, registry.lower_child_plot(child)?).map(|marks| {
                    marks
                        .into_iter()
                        .map(|mark| {
                            mark.with_language_identity(
                                declaration.publish_source_name,
                                declaration.public_aliases.clone(),
                                declaration.component_part_alias.clone(),
                            )
                        })
                        .collect()
                })
            }
            ResolvedMark::Group(resolved) => {
                let mut data = match (&resolved.data, &resolved.store_data) {
                    (Some(data), None) => DataContext::new(data.clone()),
                    (None, Some(store)) => DataContext::store_data(store.clone()),
                    (None, None) => DataContext::default(),
                    (Some(_), Some(_)) => {
                        return Err(RegistryError::Lowering {
                            kind: self.kind.clone(),
                            message: "mark group cannot have both dataframe and store data"
                                .to_string(),
                        });
                    }
                };
                for stage in &resolved.transforms {
                    data = data.with_transform_stage(stage.scope, stage.transform.clone());
                }
                let mut group = MarkGroup::<C>::new().with_data_context(data, resolved.data_mode);
                if let Some(id) = &resolved.id {
                    group = group.id(id.clone());
                }
                if let Some(component_kind) = &resolved.component_kind {
                    group = group.component_kind(component_kind.clone());
                }
                if let Some(view) = &resolved.view {
                    let mut view_data = match (&view.data, &view.store_data) {
                        (Some(data), None) => DataContext::new(data.clone()),
                        (None, Some(store)) => DataContext::store_data(store.clone()),
                        (None, None) => DataContext::default(),
                        (Some(_), Some(_)) => {
                            return Err(RegistryError::Lowering {
                                kind: self.kind.clone(),
                                message: "inline view cannot have both dataframe and store data"
                                    .to_string(),
                            });
                        }
                    };
                    for stage in &view.transforms {
                        view_data =
                            view_data.with_transform_stage(stage.scope, stage.transform.clone());
                    }
                    group = group
                        .with_compiled_view_scope(view.spec.clone(), view_data)
                        .map_err(RegistryError::from)?;
                }
                for child in &resolved.marks {
                    for mark in self.lower_mark(registry, coordinate, child)? {
                        group = group.mark(mark);
                    }
                }
                Ok(vec![PlotMark::from_group(group).with_language_identity(
                    resolved.publish_id,
                    Vec::new(),
                    None,
                )])
            }
        }
    }

    fn lower_behavior(
        &self,
        registry: &NativeRegistry,
        coordinate: &C,
        behavior: &ResolvedToolBehavior,
    ) -> Result<Arc<dyn ChartTool<C>>, RegistryError> {
        let chrome = behavior
            .chrome
            .iter()
            .map(|mark| self.lower_mark(registry, coordinate, mark))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect();
        let mut nested_tools = behavior
            .native_tools
            .iter()
            .map(|tool| {
                let entry =
                    self.tools
                        .get(&tool.kind)
                        .ok_or_else(|| RegistryError::UnknownToolPair {
                            coordinate: self.kind.clone(),
                            tool: tool.kind.clone(),
                        })?;
                registry.validate(&entry.schema.key, tool)?;
                (entry.lowerer)(tool)
            })
            .collect::<Result<Vec<_>, _>>()?;
        nested_tools.extend(
            behavior
                .nested_behaviors
                .iter()
                .map(|nested| self.lower_behavior(registry, coordinate, nested))
                .collect::<Result<Vec<_>, _>>()?,
        );
        Ok(Arc::new(LanguageBehaviorTool {
            id: behavior.id.clone(),
            component_kind: behavior.component_kind.clone(),
            component_id: behavior.component_id.clone(),
            state: behavior.state.clone(),
            event_bindings: behavior.event_bindings.clone(),
            param_change_bindings: behavior.param_change_bindings.clone(),
            scale_edits: behavior.scale_edits.clone(),
            chrome,
            nested_tools,
            exports: behavior.exports.clone(),
            metadata: behavior.metadata.clone(),
        }))
    }

    fn lower_typed(
        &self,
        registry: &NativeRegistry,
        resolved: &ResolvedPlot,
    ) -> Result<Plot<C>, RegistryError> {
        registry.validate(
            &NativeKindKey::new(NativeKindNamespace::Coordinate, &self.kind),
            &resolved.coordinate,
        )?;
        let coordinate = (self.coordinate_lowerer)(&resolved.coordinate)?;
        let mut plot = Plot::with_coord(coordinate.clone());
        if let Some(data) = &resolved.data {
            plot = plot.data(data.clone());
        }
        for declaration in &resolved.marks {
            for mark in self.lower_mark(registry, &coordinate, declaration)? {
                plot = plot.mark(mark);
            }
        }
        for declaration in &resolved.tools {
            let entry = self.tools.get(&declaration.kind).ok_or_else(|| {
                RegistryError::UnknownToolPair {
                    coordinate: self.kind.clone(),
                    tool: declaration.kind.clone(),
                }
            })?;
            registry.validate(&entry.schema.key, declaration)?;
            plot = plot.tool_arc((entry.lowerer)(declaration)?);
        }
        for behavior in &resolved.tool_behaviors {
            plot = plot.tool_arc(self.lower_behavior(registry, &coordinate, behavior)?);
        }
        for declaration in &resolved.widgets {
            plot = plot.widget_attachment(registry.lower_widget(declaration)?);
        }
        for child in &resolved.children {
            let lowerer =
                self.child_plot_lowerer
                    .as_ref()
                    .ok_or_else(|| RegistryError::Lowering {
                        kind: self.kind.clone(),
                        message: "coordinate does not accept child plots".to_string(),
                    })?;
            let mut child_plot;
            let child_resolved =
                if self.children_use_parent_data_context && child.plot.data_is_inherited {
                    child_plot = child.plot.as_ref().clone();
                    child_plot.data = None;
                    child_plot.data_is_inherited = false;
                    &child_plot
                } else {
                    child.plot.as_ref()
                };
            plot = lowerer(
                plot,
                registry.lower_child_plot(child_resolved)?,
                &child.placement,
                resolved,
            )?;
        }
        if let Some(lowerer) = &self.plot_property_lowerer {
            plot = lowerer(plot, resolved)?;
        }
        Ok(plot)
    }
}

#[async_trait]
impl<C: CoordinateSystem> ErasedCoordinatePack for CoordinatePack<C> {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn schemas(&self) -> Vec<KindSchema> {
        std::iter::once(self.schema.clone())
            .chain(self.marks.values().map(|entry| entry.schema.clone()))
            .chain(self.child_marks.values().map(|entry| entry.schema.clone()))
            .chain(self.tools.values().map(|entry| entry.schema.clone()))
            .collect()
    }

    fn validate_registration(&self) -> Result<(), RegistryError> {
        if let Some(key) = self.duplicate_keys.first() {
            return Err(RegistryError::DuplicateSchema(key.clone()));
        }
        let expected_coordinate = NativeKindKey::new(NativeKindNamespace::Coordinate, &self.kind);
        if self.schema.key != expected_coordinate {
            return Err(RegistryError::SchemaLowererMismatch(
                self.schema.key.clone(),
            ));
        }
        for (kind, entry) in &self.marks {
            let expected = NativeKindKey::mark(&self.kind, kind);
            if entry.schema.key != expected {
                return Err(RegistryError::SchemaLowererMismatch(
                    entry.schema.key.clone(),
                ));
            }
        }
        for (kind, entry) in &self.child_marks {
            let expected = NativeKindKey::mark(&self.kind, kind);
            if entry.schema.key != expected {
                return Err(RegistryError::SchemaLowererMismatch(
                    entry.schema.key.clone(),
                ));
            }
        }
        for (kind, entry) in &self.tools {
            if entry.schema.key.namespace != NativeKindNamespace::Tool
                || entry.schema.key.kind != *kind
                || !entry.schema.compatible_coordinates.contains(&self.kind)
            {
                return Err(RegistryError::SchemaLowererMismatch(
                    entry.schema.key.clone(),
                ));
            }
        }
        Ok(())
    }

    fn lower_child(
        &self,
        registry: &NativeRegistry,
        plot: &ResolvedPlot,
    ) -> Result<Box<dyn SubplotChildPlotSpec>, RegistryError> {
        Ok(Box::new(self.lower_typed(registry, plot)?))
    }

    async fn compile_root(
        &self,
        registry: &NativeRegistry,
        plot: &ResolvedPlot,
        session_context: &SessionContext,
    ) -> Result<CompiledPlot, RegistryError> {
        let mut chart = Chart::from_plot(self.lower_typed(registry, plot)?);
        if let Some(title) = &plot.furnishings.title {
            chart = chart.title(title.clone());
        }
        if let Some(subtitle) = &plot.furnishings.subtitle {
            chart = chart.subtitle(subtitle.clone());
        }
        if let Some(layout) = &plot.furnishings.layout {
            chart = chart.layout_spec(layout.clone());
        }
        if let Some(theme) = &plot.furnishings.theme {
            chart = chart.theme(theme.clone());
        }
        chart = chart
            .time_context(plot.furnishings.time_context.clone())
            .formatting_context(plot.furnishings.formatting_context.clone());
        for (param, sharing) in &plot.furnishings.params {
            chart = chart.param_with_sharing(param.clone(), *sharing);
        }
        for selection in &plot.furnishings.selections {
            chart = chart.selection(selection.clone());
        }
        for store in &plot.furnishings.stores {
            chart = chart.store(store.clone());
        }
        for binding in &plot.furnishings.event_bindings {
            chart = chart.event_binding(binding.clone());
        }
        Ok(chart.compile(session_context).await?)
    }
}

struct TransformEntry {
    schema: KindSchema,
    lowerer: TransformEntryLowerer,
}

struct AdjustmentEntry {
    schema: KindSchema,
    lowerer: NativeAdjustmentLowerer,
}

enum TransformEntryLowerer {
    Leaf(NativeTransformLowerer),
    Pipeline(NativeTransformPipelineLowerer),
}

struct WidgetEntry {
    schema: KindSchema,
    lowerer: NativeWidgetLowerer,
}

struct ObjectEntry {
    schema: KindSchema,
    lowerer: NativeObjectLowerer,
}

#[derive(Clone, Debug)]
struct PendingNativeModule {
    schema: NativeModuleSchema,
    implementation_profile: NativeModuleImplementationProfileId,
}

#[derive(Clone, Debug)]
enum RegistrationScope {
    Builtin,
    Native(NativeModuleId),
}

pub struct NativeRegistryBuilder {
    language_major: u32,
    profile_label: String,
    coordinates: BTreeMap<String, Box<dyn ErasedCoordinatePack>>,
    adjustments: BTreeMap<String, AdjustmentEntry>,
    transforms: BTreeMap<String, TransformEntry>,
    widgets: BTreeMap<String, WidgetEntry>,
    objects: BTreeMap<NativeKindKey, ObjectEntry>,
    registration_scope: Option<RegistrationScope>,
    builtin_keys: BTreeSet<NativeKindKey>,
    native_module_keys: BTreeMap<NativeModuleId, BTreeSet<NativeKindKey>>,
    native_modules: BTreeMap<NativeModuleId, PendingNativeModule>,
}

impl NativeRegistryBuilder {
    pub fn new(language_major: u32, profile_label: impl Into<String>) -> Self {
        Self {
            language_major,
            profile_label: profile_label.into(),
            coordinates: BTreeMap::new(),
            adjustments: BTreeMap::new(),
            transforms: BTreeMap::new(),
            widgets: BTreeMap::new(),
            objects: BTreeMap::new(),
            registration_scope: None,
            builtin_keys: BTreeSet::new(),
            native_module_keys: BTreeMap::new(),
            native_modules: BTreeMap::new(),
        }
    }

    pub(crate) fn begin_builtin_registration(&mut self) -> Result<(), RegistryError> {
        if self.registration_scope.is_some() {
            return Err(RegistryError::NestedRegistrationScope);
        }
        self.registration_scope = Some(RegistrationScope::Builtin);
        Ok(())
    }

    pub(crate) fn finish_builtin_registration(&mut self) {
        debug_assert!(matches!(
            self.registration_scope,
            Some(RegistrationScope::Builtin)
        ));
        self.registration_scope = None;
    }

    pub fn native_module(
        &mut self,
        id: NativeModuleId,
        docs: impl Into<String>,
        implementation_profile: NativeModuleImplementationProfileId,
    ) -> Result<NativeModuleBuilder<'_>, RegistryError> {
        if self.registration_scope.is_some() {
            return Err(RegistryError::NestedRegistrationScope);
        }
        if self.native_modules.contains_key(&id) || self.native_module_keys.contains_key(&id) {
            return Err(RegistryError::DuplicateNativeModule(id));
        }
        self.registration_scope = Some(RegistrationScope::Native(id.clone()));
        self.native_module_keys.insert(id.clone(), BTreeSet::new());
        Ok(NativeModuleBuilder {
            registry: self,
            schema: Some(NativeModuleSchema::new(id, docs)),
            implementation_profile: Some(implementation_profile),
            finished: false,
        })
    }

    fn record_registration(
        &mut self,
        keys: impl IntoIterator<Item = NativeKindKey>,
    ) -> Result<(), RegistryError> {
        match self.registration_scope.clone() {
            Some(RegistrationScope::Builtin) => {
                self.builtin_keys.extend(keys);
                Ok(())
            }
            Some(RegistrationScope::Native(id)) => {
                self.native_module_keys
                    .get_mut(&id)
                    .expect("native registration scope has a key set")
                    .extend(keys);
                Ok(())
            }
            None => Err(RegistryError::RegistrationScopeRequired),
        }
    }

    pub fn register_coordinate_pack<C: CoordinateSystem>(
        &mut self,
        pack: CoordinatePack<C>,
    ) -> Result<(), RegistryError> {
        let kind = pack.kind.clone();
        if self.coordinates.contains_key(&kind) {
            return Err(RegistryError::DuplicateCoordinate(kind));
        }
        self.record_registration(
            ErasedCoordinatePack::schemas(&pack)
                .into_iter()
                .map(|schema| schema.key),
        )?;
        self.coordinates.insert(kind, Box::new(pack));
        Ok(())
    }

    /// Add a mark lowerer to an already registered typed coordinate pack.
    ///
    /// This is the downstream extension path when a host registers built-ins
    /// first and then invokes independent extension registration functions.
    pub fn register_mark<C: CoordinateSystem>(
        &mut self,
        coordinate_kind: &str,
        kind: impl Into<String>,
        schema: KindSchema,
        lowerer: impl Fn(&ResolvedDeclaration) -> Result<Vec<PlotMark<C>>, RegistryError>
        + Send
        + Sync
        + 'static,
    ) -> Result<(), RegistryError> {
        let kind = kind.into();
        {
            let pack = self
                .coordinates
                .get(coordinate_kind)
                .ok_or_else(|| RegistryError::UnknownCoordinate(coordinate_kind.to_string()))?;
            let pack = pack
                .as_any()
                .downcast_ref::<CoordinatePack<C>>()
                .ok_or_else(|| RegistryError::CoordinatePackTypeMismatch {
                    coordinate: coordinate_kind.to_string(),
                    expected: std::any::type_name::<C>().to_string(),
                })?;
            if pack.marks.contains_key(&kind) {
                return Err(RegistryError::DuplicateSchema(schema.key));
            }
        }
        self.record_registration(std::iter::once(schema.key.clone()))?;
        let pack = self
            .coordinates
            .get_mut(coordinate_kind)
            .expect("coordinate pack was checked above")
            .as_any_mut()
            .downcast_mut::<CoordinatePack<C>>()
            .expect("coordinate pack type was checked above");
        pack.marks.insert(
            kind,
            TypedMarkEntry {
                schema,
                lowerer: Arc::new(move |_coordinate, declaration| lowerer(declaration)),
            },
        );
        Ok(())
    }

    /// Add a mark lowerer that needs the concrete coordinate authoring value.
    /// Existing declaration-only extensions should continue to use
    /// [`Self::register_mark`].
    pub fn register_mark_with_coordinate<C: CoordinateSystem>(
        &mut self,
        coordinate_kind: &str,
        kind: impl Into<String>,
        schema: KindSchema,
        lowerer: impl Fn(&C, &ResolvedDeclaration) -> Result<Vec<PlotMark<C>>, RegistryError>
        + Send
        + Sync
        + 'static,
    ) -> Result<(), RegistryError> {
        let kind = kind.into();
        {
            let pack = self
                .coordinates
                .get(coordinate_kind)
                .ok_or_else(|| RegistryError::UnknownCoordinate(coordinate_kind.to_string()))?;
            let pack = pack
                .as_any()
                .downcast_ref::<CoordinatePack<C>>()
                .ok_or_else(|| RegistryError::CoordinatePackTypeMismatch {
                    coordinate: coordinate_kind.to_string(),
                    expected: std::any::type_name::<C>().to_string(),
                })?;
            if pack.marks.contains_key(&kind) {
                return Err(RegistryError::DuplicateSchema(schema.key));
            }
        }
        self.record_registration(std::iter::once(schema.key.clone()))?;
        let pack = self
            .coordinates
            .get_mut(coordinate_kind)
            .expect("coordinate pack was checked above")
            .as_any_mut()
            .downcast_mut::<CoordinatePack<C>>()
            .expect("coordinate pack type was checked above");
        pack.marks.insert(
            kind,
            TypedMarkEntry {
                schema,
                lowerer: Arc::new(lowerer),
            },
        );
        Ok(())
    }

    /// Add a tool lowerer to an already registered typed coordinate pack.
    pub fn register_tool<C: CoordinateSystem>(
        &mut self,
        coordinate_kind: &str,
        kind: impl Into<String>,
        schema: KindSchema,
        lowerer: impl Fn(&ResolvedDeclaration) -> Result<Arc<dyn ChartTool<C>>, RegistryError>
        + Send
        + Sync
        + 'static,
    ) -> Result<(), RegistryError> {
        let kind = kind.into();
        {
            let pack = self
                .coordinates
                .get(coordinate_kind)
                .ok_or_else(|| RegistryError::UnknownCoordinate(coordinate_kind.to_string()))?;
            let pack = pack
                .as_any()
                .downcast_ref::<CoordinatePack<C>>()
                .ok_or_else(|| RegistryError::CoordinatePackTypeMismatch {
                    coordinate: coordinate_kind.to_string(),
                    expected: std::any::type_name::<C>().to_string(),
                })?;
            if pack.tools.contains_key(&kind) {
                return Err(RegistryError::DuplicateSchema(schema.key));
            }
        }
        self.record_registration(std::iter::once(schema.key.clone()))?;
        let pack = self
            .coordinates
            .get_mut(coordinate_kind)
            .expect("coordinate pack was checked above")
            .as_any_mut()
            .downcast_mut::<CoordinatePack<C>>()
            .expect("coordinate pack type was checked above");
        pack.tools.insert(
            kind,
            TypedToolEntry {
                schema,
                lowerer: Arc::new(lowerer),
            },
        );
        Ok(())
    }

    pub fn register_transform(
        &mut self,
        schema: KindSchema,
        lowerer: NativeTransformLowerer,
    ) -> Result<(), RegistryError> {
        let kind = schema.key.kind.clone();
        if schema.key.namespace != NativeKindNamespace::Transform {
            return Err(RegistryError::SchemaLowererMismatch(schema.key));
        }
        if self.transforms.contains_key(&kind) {
            return Err(RegistryError::DuplicateKind {
                namespace: NativeKindNamespace::Transform,
                kind,
            });
        }
        self.record_registration(std::iter::once(schema.key.clone()))?;
        self.transforms.insert(
            kind,
            TransformEntry {
                schema,
                lowerer: TransformEntryLowerer::Leaf(lowerer),
            },
        );
        Ok(())
    }

    pub fn register_adjustment(
        &mut self,
        schema: KindSchema,
        lowerer: NativeAdjustmentLowerer,
    ) -> Result<(), RegistryError> {
        let kind = schema.key.kind.clone();
        if schema.key.namespace != NativeKindNamespace::Adjust {
            return Err(RegistryError::SchemaLowererMismatch(schema.key));
        }
        if self.adjustments.contains_key(&kind) {
            return Err(RegistryError::DuplicateKind {
                namespace: NativeKindNamespace::Adjust,
                kind,
            });
        }
        self.record_registration(std::iter::once(schema.key.clone()))?;
        self.adjustments
            .insert(kind, AdjustmentEntry { schema, lowerer });
        Ok(())
    }

    pub fn register_adjustment_definition(
        &mut self,
        definition: AdjustmentLanguageDefinition,
    ) -> Result<(), RegistryError> {
        let lowerer = definition.lowerer;
        self.register_adjustment(
            definition.schema,
            Arc::new(move |declaration, context| {
                lowerer(declaration, context).map_err(RegistryError::from)
            }),
        )
    }

    pub fn register_transform_definition(
        &mut self,
        definition: TransformLanguageDefinition,
    ) -> Result<(), RegistryError> {
        let lowerer = definition.lowerer;
        self.register_transform(
            definition.schema,
            Arc::new(move |declaration, context| {
                lowerer(declaration, context).map_err(RegistryError::from)
            }),
        )
    }

    pub fn register_transform_pipeline_definition(
        &mut self,
        definition: TransformPipelineLanguageDefinition,
    ) -> Result<(), RegistryError> {
        let kind = definition.schema.key.kind.clone();
        if definition.schema.key.namespace != NativeKindNamespace::Transform {
            return Err(RegistryError::SchemaLowererMismatch(definition.schema.key));
        }
        if self.transforms.contains_key(&kind) {
            return Err(RegistryError::DuplicateKind {
                namespace: NativeKindNamespace::Transform,
                kind,
            });
        }
        self.record_registration(std::iter::once(definition.schema.key.clone()))?;
        let lowerer = definition.lowerer;
        self.transforms.insert(
            kind,
            TransformEntry {
                schema: definition.schema,
                lowerer: TransformEntryLowerer::Pipeline(Arc::new(
                    move |declaration, stages, outputs, context| {
                        lowerer(declaration, stages, outputs, context).map_err(RegistryError::from)
                    },
                )),
            },
        );
        Ok(())
    }

    pub fn register_widget(
        &mut self,
        schema: KindSchema,
        lowerer: NativeWidgetLowerer,
    ) -> Result<(), RegistryError> {
        let kind = schema.key.kind.clone();
        if schema.key.namespace != NativeKindNamespace::Widget {
            return Err(RegistryError::SchemaLowererMismatch(schema.key));
        }
        if self.widgets.contains_key(&kind) {
            return Err(RegistryError::DuplicateKind {
                namespace: NativeKindNamespace::Widget,
                kind,
            });
        }
        self.record_registration(std::iter::once(schema.key.clone()))?;
        self.widgets.insert(kind, WidgetEntry { schema, lowerer });
        Ok(())
    }

    pub fn register_widget_definition(
        &mut self,
        definition: WidgetLanguageDefinition,
    ) -> Result<(), RegistryError> {
        let lowerer = definition.lowerer;
        self.register_widget(
            definition.schema,
            Arc::new(move |declaration| lowerer(declaration).map_err(RegistryError::from)),
        )
    }

    pub fn register_object(
        &mut self,
        schema: KindSchema,
        lowerer: NativeObjectLowerer,
    ) -> Result<(), RegistryError> {
        let key = schema.key.clone();
        if !matches!(
            key.namespace,
            NativeKindNamespace::Scale
                | NativeKindNamespace::Axis
                | NativeKindNamespace::Legend
                | NativeKindNamespace::Layout
                | NativeKindNamespace::View
                | NativeKindNamespace::Resource
        ) {
            return Err(RegistryError::SchemaLowererMismatch(key));
        }
        if self.objects.contains_key(&key) {
            return Err(RegistryError::DuplicateKind {
                namespace: key.namespace,
                kind: key.kind,
            });
        }
        self.record_registration(std::iter::once(key.clone()))?;
        self.objects.insert(key, ObjectEntry { schema, lowerer });
        Ok(())
    }

    pub fn register_object_definition(
        &mut self,
        definition: ObjectLanguageDefinition,
    ) -> Result<(), RegistryError> {
        let lowerer = definition.lowerer;
        self.register_object(
            definition.schema,
            Arc::new(move |declaration| lowerer(declaration).map_err(RegistryError::from)),
        )
    }

    pub fn build(self) -> Result<NativeRegistry, RegistryError> {
        if self.registration_scope.is_some() {
            return Err(RegistryError::UnfinishedRegistrationScope);
        }
        let mut entries = BTreeMap::new();
        for pack in self.coordinates.values() {
            pack.validate_registration()?;
            for schema in pack.schemas() {
                let key = schema.key.clone();
                if entries.insert(key.clone(), schema).is_some() {
                    return Err(RegistryError::DuplicateSchema(key));
                }
            }
        }
        for entry in self.adjustments.values() {
            insert_schema(&mut entries, entry.schema.clone())?;
        }
        for entry in self.transforms.values() {
            insert_schema(&mut entries, entry.schema.clone())?;
        }
        for entry in self.widgets.values() {
            insert_schema(&mut entries, entry.schema.clone())?;
        }
        for entry in self.objects.values() {
            insert_schema(&mut entries, entry.schema.clone())?;
        }

        let all_registered = self
            .builtin_keys
            .iter()
            .cloned()
            .chain(
                self.native_module_keys
                    .values()
                    .flat_map(|keys| keys.iter().cloned()),
            )
            .collect::<BTreeSet<_>>();
        let all_entries = entries.keys().cloned().collect::<BTreeSet<_>>();
        if all_registered != all_entries {
            return Err(RegistryError::RegistrationOwnershipMismatch {
                unowned: all_entries.difference(&all_registered).cloned().collect(),
                missing: all_registered.difference(&all_entries).cloned().collect(),
            });
        }

        let mut module_schemas = BTreeMap::new();
        let mut native_modules = BTreeMap::new();
        for (id, registered_keys) in &self.native_module_keys {
            let pending = self
                .native_modules
                .get(id)
                .ok_or_else(|| RegistryError::UnfinishedNativeModule(id.clone()))?;
            let exported_keys = pending
                .schema
                .exports
                .values()
                .map(|export| export.implementation.clone())
                .collect::<BTreeSet<_>>();
            if registered_keys != &exported_keys {
                return Err(RegistryError::NativeModuleOwnershipMismatch {
                    module: id.clone(),
                    unexported: registered_keys
                        .difference(&exported_keys)
                        .cloned()
                        .collect(),
                    unregistered: exported_keys.difference(registered_keys).cloned().collect(),
                });
            }
            pending.schema.validate(&entries)?;
            let schema_profile = pending.schema.schema_profile(&entries)?;
            module_schemas.insert(id.clone(), pending.schema.clone());
            native_modules.insert(
                id.clone(),
                RegisteredNativeModule {
                    schema_profile,
                    implementation_profile: pending.implementation_profile.clone(),
                },
            );
        }
        for id in self.native_modules.keys() {
            if !self.native_module_keys.contains_key(id) {
                return Err(RegistryError::UnfinishedNativeModule(id.clone()));
            }
        }

        let snapshot = NativeSchemaSnapshot {
            version: SchemaVersion {
                major: self.language_major,
                minor: 0,
            },
            profile_label: self.profile_label,
            entries,
            modules: module_schemas,
        };
        snapshot.validate_docs()?;
        let canonical = snapshot.canonical_json()?;
        let module_implementation_profiles = native_modules
            .iter()
            .map(|(id, module)| (id, &module.implementation_profile))
            .collect::<Vec<_>>();
        let profile_id = NativeRegistryProfileId(format!(
            "avenger-native-{}-{:x}",
            self.language_major,
            Sha256::digest(
                serde_json::to_vec(&(canonical, module_implementation_profiles)).map_err(
                    |error| {
                        RegistryError::Schema(avenger_chart_schema::SchemaError::Serialize(error))
                    }
                )?
            )
        ));
        let builtin_schemas = self
            .builtin_keys
            .iter()
            .map(|key| {
                snapshot
                    .entries
                    .get(key)
                    .expect("registered built-in key has a schema")
            })
            .collect::<Vec<_>>();
        let builtin_profile_id = NativeBuiltinProfileId(format!(
            "avenger-native-builtins-{}-{:x}",
            self.language_major,
            Sha256::digest(
                serde_json::to_vec(&(snapshot.version, builtin_schemas)).map_err(|error| {
                    RegistryError::Schema(avenger_chart_schema::SchemaError::Serialize(error))
                })?
            )
        ));
        Ok(NativeRegistry {
            snapshot,
            profile_id,
            builtin_profile_id,
            native_modules,
            coordinates: self
                .coordinates
                .into_iter()
                .map(|(kind, pack)| (kind, Arc::from(pack)))
                .collect(),
            adjustments: self.adjustments,
            transforms: self.transforms,
            widgets: self.widgets,
            objects: self.objects,
        })
    }
}

/// Explicit registration scope for one host-provided native module.
///
/// Register lowerers through [`Self::registry`] and publish every registered
/// implementation with [`Self::export`] before calling [`Self::finish`].
/// Dropping an unfinished scope leaves the parent builder invalid, preventing
/// accidental implicit built-ins.
pub struct NativeModuleBuilder<'a> {
    registry: &'a mut NativeRegistryBuilder,
    schema: Option<NativeModuleSchema>,
    implementation_profile: Option<NativeModuleImplementationProfileId>,
    finished: bool,
}

impl NativeModuleBuilder<'_> {
    pub fn registry(&mut self) -> &mut NativeRegistryBuilder {
        self.registry
    }

    pub fn export(
        &mut self,
        name: impl Into<String>,
        implementation: NativeKindKey,
        docs: impl Into<String>,
    ) -> Result<&mut Self, RegistryError> {
        let name = name.into();
        let schema = self
            .schema
            .as_mut()
            .expect("unfinished native module has a schema");
        if schema.exports.contains_key(&name) {
            return Err(RegistryError::DuplicateNativeExport {
                module: schema.id.clone(),
                name,
            });
        }
        schema.add_export(name, implementation, docs)?;
        Ok(self)
    }

    pub fn finish(mut self) -> Result<(), RegistryError> {
        let schema = self
            .schema
            .take()
            .expect("unfinished native module has a schema");
        let implementation_profile = self
            .implementation_profile
            .take()
            .expect("unfinished native module has an implementation profile");
        let id = schema.id.clone();
        self.registry.registration_scope = None;
        if self
            .registry
            .native_modules
            .insert(
                id.clone(),
                PendingNativeModule {
                    schema,
                    implementation_profile,
                },
            )
            .is_some()
        {
            return Err(RegistryError::DuplicateNativeModule(id));
        }
        self.finished = true;
        Ok(())
    }
}

impl Drop for NativeModuleBuilder<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.registry.registration_scope = None;
        }
    }
}

fn insert_schema(
    entries: &mut BTreeMap<NativeKindKey, KindSchema>,
    schema: KindSchema,
) -> Result<(), RegistryError> {
    let key = schema.key.clone();
    if entries.insert(key.clone(), schema).is_some() {
        return Err(RegistryError::DuplicateSchema(key));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisteredNativeModule {
    pub schema_profile: NativeModuleSchemaProfileId,
    pub implementation_profile: NativeModuleImplementationProfileId,
}

#[derive(Clone, Copy, Debug)]
pub struct ResolvedNativeExport<'a> {
    pub module: &'a NativeModuleId,
    pub schema_profile: &'a NativeModuleSchemaProfileId,
    pub implementation_profile: &'a NativeModuleImplementationProfileId,
    pub export: &'a NativeModuleExport,
}

pub struct NativeRegistry {
    snapshot: NativeSchemaSnapshot,
    profile_id: NativeRegistryProfileId,
    builtin_profile_id: NativeBuiltinProfileId,
    native_modules: BTreeMap<NativeModuleId, RegisteredNativeModule>,
    coordinates: BTreeMap<String, Arc<dyn ErasedCoordinatePack>>,
    adjustments: BTreeMap<String, AdjustmentEntry>,
    transforms: BTreeMap<String, TransformEntry>,
    widgets: BTreeMap<String, WidgetEntry>,
    objects: BTreeMap<NativeKindKey, ObjectEntry>,
}

impl NativeRegistry {
    pub fn builder() -> NativeRegistryBuilder {
        NativeRegistryBuilder::new(1, "custom")
    }

    pub fn snapshot(&self) -> &NativeSchemaSnapshot {
        &self.snapshot
    }

    pub fn profile_id(&self) -> &NativeRegistryProfileId {
        &self.profile_id
    }

    pub fn builtin_profile_id(&self) -> &NativeBuiltinProfileId {
        &self.builtin_profile_id
    }

    pub fn native_module(
        &self,
        id: &NativeModuleId,
    ) -> Option<(&NativeModuleSchema, &RegisteredNativeModule)> {
        Some((self.snapshot.modules.get(id)?, self.native_modules.get(id)?))
    }

    pub fn native_export(
        &self,
        id: &NativeModuleId,
        name: &str,
    ) -> Result<ResolvedNativeExport<'_>, RegistryError> {
        let (schema, module) = self
            .native_module(id)
            .ok_or_else(|| RegistryError::UnknownNativeModule(id.clone()))?;
        let export =
            schema
                .exports
                .get(name)
                .ok_or_else(|| RegistryError::UnknownNativeExport {
                    module: id.clone(),
                    name: name.to_string(),
                })?;
        Ok(ResolvedNativeExport {
            module: &schema.id,
            schema_profile: &module.schema_profile,
            implementation_profile: &module.implementation_profile,
            export,
        })
    }

    pub fn canonical_schema_json(&self) -> Result<Vec<u8>, RegistryError> {
        Ok(self.snapshot.canonical_json()?)
    }

    pub async fn compile_root(
        &self,
        plot: &ResolvedPlot,
        session_context: &SessionContext,
    ) -> Result<CompiledPlot, RegistryError> {
        let pack = self
            .coordinates
            .get(&plot.coordinate.kind)
            .ok_or_else(|| RegistryError::UnknownCoordinate(plot.coordinate.kind.clone()))?;
        pack.compile_root(self, plot, session_context).await
    }

    pub fn lower_child_plot(
        &self,
        plot: &ResolvedPlot,
    ) -> Result<Box<dyn SubplotChildPlotSpec>, RegistryError> {
        let pack = self
            .coordinates
            .get(&plot.coordinate.kind)
            .ok_or_else(|| RegistryError::UnknownCoordinate(plot.coordinate.kind.clone()))?;
        pack.lower_child(self, plot)
    }

    /// Lower resolved marks through one typed coordinate pack without
    /// constructing a complete plot. Language-owned mark blocks such as
    /// continuous-legend overlays use this boundary after performing their
    /// own dataflow lowering.
    pub fn lower_marks<C: CoordinateSystem>(
        &self,
        coordinate_kind: &str,
        coordinate: &C,
        marks: &[ResolvedMark],
    ) -> Result<Vec<PlotMark<C>>, RegistryError> {
        let pack = self
            .coordinates
            .get(coordinate_kind)
            .ok_or_else(|| RegistryError::UnknownCoordinate(coordinate_kind.to_string()))?;
        let pack = pack
            .as_any()
            .downcast_ref::<CoordinatePack<C>>()
            .ok_or_else(|| RegistryError::CoordinatePackTypeMismatch {
                coordinate: coordinate_kind.to_string(),
                expected: std::any::type_name::<C>().to_string(),
            })?;
        marks
            .iter()
            .map(|mark| pack.lower_mark(self, coordinate, mark))
            .collect::<Result<Vec<_>, _>>()
            .map(|marks| marks.into_iter().flatten().collect())
    }

    pub fn lower_transform(
        &self,
        declaration: &ResolvedDeclaration,
        context: DataTransformCompileContext,
    ) -> Result<LoweredTransform, RegistryError> {
        let entry =
            self.transforms
                .get(&declaration.kind)
                .ok_or_else(|| RegistryError::UnknownKind {
                    namespace: NativeKindNamespace::Transform,
                    kind: declaration.kind.clone(),
                })?;
        self.validate(&entry.schema.key, declaration)?;
        match &entry.lowerer {
            TransformEntryLowerer::Leaf(lowerer) => lowerer(declaration, context),
            TransformEntryLowerer::Pipeline(_) => Err(RegistryError::WrongTransformMode {
                kind: declaration.kind.clone(),
                expected: NativeTransformMode::Leaf,
            }),
        }
    }

    pub fn lower_adjustment(
        &self,
        declaration: &ResolvedDeclaration,
        context: MarkAdjustmentCompileContext,
    ) -> Result<LoweredAdjustment, RegistryError> {
        let entry =
            self.adjustments
                .get(&declaration.kind)
                .ok_or_else(|| RegistryError::UnknownKind {
                    namespace: NativeKindNamespace::Adjust,
                    kind: declaration.kind.clone(),
                })?;
        self.validate(&entry.schema.key, declaration)?;
        (entry.lowerer)(declaration, context)
    }

    pub fn transform_mode(&self, kind: &str) -> Result<NativeTransformMode, RegistryError> {
        let entry = self
            .transforms
            .get(kind)
            .ok_or_else(|| RegistryError::UnknownKind {
                namespace: NativeKindNamespace::Transform,
                kind: kind.to_string(),
            })?;
        Ok(match entry.lowerer {
            TransformEntryLowerer::Leaf(_) => NativeTransformMode::Leaf,
            TransformEntryLowerer::Pipeline(_) => NativeTransformMode::Pipeline,
        })
    }

    pub fn lower_transform_pipeline(
        &self,
        declaration: &ResolvedDeclaration,
        stages: Vec<DataTransformStage>,
        outputs: IndexMap<String, Expr>,
        context: DataTransformCompileContext,
    ) -> Result<LoweredTransform, RegistryError> {
        let entry =
            self.transforms
                .get(&declaration.kind)
                .ok_or_else(|| RegistryError::UnknownKind {
                    namespace: NativeKindNamespace::Transform,
                    kind: declaration.kind.clone(),
                })?;
        self.validate(&entry.schema.key, declaration)?;
        match &entry.lowerer {
            TransformEntryLowerer::Pipeline(lowerer) => {
                lowerer(declaration, stages, outputs, context)
            }
            TransformEntryLowerer::Leaf(_) => Err(RegistryError::WrongTransformMode {
                kind: declaration.kind.clone(),
                expected: NativeTransformMode::Pipeline,
            }),
        }
    }

    pub fn lower_widget(
        &self,
        declaration: &ResolvedDeclaration,
    ) -> Result<WidgetAttachment, RegistryError> {
        let entry =
            self.widgets
                .get(&declaration.kind)
                .ok_or_else(|| RegistryError::UnknownKind {
                    namespace: NativeKindNamespace::Widget,
                    kind: declaration.kind.clone(),
                })?;
        self.validate(&entry.schema.key, declaration)?;
        (entry.lowerer)(declaration)
    }

    pub fn lower_object(
        &self,
        key: &NativeKindKey,
        declaration: &ResolvedDeclaration,
    ) -> Result<Box<dyn Any + Send + Sync>, RegistryError> {
        let entry = self
            .objects
            .get(key)
            .ok_or_else(|| RegistryError::UnknownKind {
                namespace: key.namespace,
                kind: key.kind.clone(),
            })?;
        self.validate(key, declaration)?;
        (entry.lowerer)(declaration)
    }

    pub fn validate(
        &self,
        key: &NativeKindKey,
        declaration: &ResolvedDeclaration,
    ) -> Result<(), RegistryError> {
        let schema = self
            .snapshot
            .entries
            .get(key)
            .ok_or_else(|| RegistryError::UnknownKind {
                namespace: key.namespace,
                kind: key.kind.clone(),
            })?;
        if declaration.kind != schema.key.kind {
            return Err(RegistryError::Lowering {
                kind: declaration.kind.clone(),
                message: format!("expected kind '{}'", schema.key.kind),
            });
        }
        for (name, property) in &schema.properties {
            match declaration.properties.get(name) {
                Some(value) => validate_value_shape(value, &property.shape, name)?,
                None if property.required && property.default.is_none() => {
                    return Err(RegistryError::MissingProperty {
                        kind: declaration.kind.clone(),
                        property: name.clone(),
                    });
                }
                None => {}
            }
        }
        for (name, channel) in &schema.channels {
            match declaration.properties.get(name) {
                Some(value) => validate_value_shape(value, &channel.shape, name)?,
                None if channel.required => {
                    return Err(RegistryError::MissingProperty {
                        kind: declaration.kind.clone(),
                        property: name.clone(),
                    });
                }
                None => {}
            }
        }
        for name in declaration.properties.keys() {
            if let Some(property) = schema.additional_properties.as_ref().filter(|_| {
                !schema.properties.contains_key(name) && !schema.channels.contains_key(name)
            }) {
                validate_value_shape(&declaration.properties[name], &property.shape, name)?;
            } else if !schema.properties.contains_key(name) && !schema.channels.contains_key(name) {
                return Err(RegistryError::UnknownProperty {
                    kind: declaration.kind.clone(),
                    property: name.clone(),
                });
            }
        }
        Ok(())
    }
}

fn validate_value_shape(
    value: &ResolvedValue,
    shape: &ValueShape,
    property: &str,
) -> Result<(), RegistryError> {
    let matches = match (value, shape) {
        (_, ValueShape::Any) => true,
        (ResolvedValue::Boolean(_), ValueShape::Boolean) => true,
        (ResolvedValue::Integer(_), ValueShape::Integer) => true,
        (ResolvedValue::Integer(_) | ResolvedValue::Number(_), ValueShape::Number) => true,
        (
            ResolvedValue::String(_),
            ValueShape::String | ValueShape::Identifier | ValueShape::ScalarBinding,
        ) => true,
        (ResolvedValue::Param(_), ValueShape::ScalarBinding) => true,
        (ResolvedValue::Selection(_), ValueShape::SelectionBinding) => true,
        (ResolvedValue::WidgetItems(_), ValueShape::WidgetData) => true,
        (
            ResolvedValue::Output(NativeOutputValue::Opaque(_)),
            ValueShape::TypedReference { .. },
        ) => true,
        (ResolvedValue::String(value), ValueShape::Atom { values }) => {
            values.iter().any(|candidate| candidate.value == *value)
        }
        (
            ResolvedValue::Boolean(_)
            | ResolvedValue::Integer(_)
            | ResolvedValue::Number(_)
            | ResolvedValue::String(_)
            | ResolvedValue::Scalar(_)
            | ResolvedValue::Expr(_)
            | ResolvedValue::Channel(_),
            ValueShape::SqlExpression,
        ) => true,
        (
            ResolvedValue::Output(NativeOutputValue::Expr(_) | NativeOutputValue::Channel(_)),
            ValueShape::SqlExpression,
        ) => true,
        (ResolvedValue::Projection(items), ValueShape::SqlProjection { policy, .. }) => {
            !items.is_empty()
                && items.iter().all(|item| match policy {
                    avenger_chart_schema::ProjectionPolicy::Named => item.alias.is_some(),
                    avenger_chart_schema::ProjectionPolicy::Select => {
                        item.alias.is_some() || item.direct_column
                    }
                })
        }
        (ResolvedValue::Query(_) | ResolvedValue::String(_), ValueShape::SqlQuery) => true,
        (ResolvedValue::Channel(_), ValueShape::ChannelConfig) => true,
        (
            ResolvedValue::Configured { head, properties },
            ValueShape::ConfiguredExpression(fields),
        ) => {
            validate_value_shape(head, &ValueShape::SqlExpression, property).is_ok()
                && properties.iter().all(|(name, value)| {
                    fields.get(name).is_some_and(|field| {
                        validate_value_shape(value, &field.shape, name).is_ok()
                    })
                })
                && fields.iter().all(|(name, field)| {
                    !field.required || field.default.is_some() || properties.contains_key(name)
                })
        }
        (
            ResolvedValue::Configured { head, properties },
            ValueShape::ConfiguredReference {
                namespaces,
                properties: fields,
            },
        ) => {
            validate_value_shape(
                head,
                &ValueShape::TypedReference {
                    namespaces: namespaces.clone(),
                },
                property,
            )
            .is_ok()
                && properties.iter().all(|(name, value)| {
                    fields.get(name).is_some_and(|field| {
                        validate_value_shape(value, &field.shape, name).is_ok()
                    })
                })
                && fields.iter().all(|(name, field)| {
                    !field.required || field.default.is_some() || properties.contains_key(name)
                })
        }
        (ResolvedValue::Pattern(_), ValueShape::PatternChannel) => true,
        (ResolvedValue::String(_) | ResolvedValue::Integer(_), ValueShape::CoordinationScope) => {
            true
        }
        (ResolvedValue::String(_) | ResolvedValue::Integer(_), ValueShape::FacetDataScope) => true,
        (ResolvedValue::Output(NativeOutputValue::RasterDim(_)), ValueShape::RasterDimension) => {
            true
        }
        (ResolvedValue::RasterDimensionChannel { .. }, ValueShape::RasterDimensionChannel) => true,
        (ResolvedValue::DataFrame(_), ValueShape::TableBinding) => true,
        (value, ValueShape::Union(shapes)) => shapes
            .iter()
            .any(|shape| validate_value_shape(value, shape, property).is_ok()),
        (ResolvedValue::Array(values), ValueShape::OneOrMany(inner)) => values
            .iter()
            .all(|value| validate_value_shape(value, inner, property).is_ok()),
        (value, ValueShape::OneOrMany(inner)) => {
            validate_value_shape(value, inner, property).is_ok()
        }
        (ResolvedValue::Array(values), ValueShape::Array(inner)) => values
            .iter()
            .all(|value| validate_value_shape(value, inner, property).is_ok()),
        (ResolvedValue::Object(values), ValueShape::Map(inner)) => values
            .values()
            .all(|value| validate_value_shape(value, inner, property).is_ok()),
        (ResolvedValue::Object(values), ValueShape::ChannelMap) => values
            .values()
            .all(|value| validate_value_shape(value, &ValueShape::SqlExpression, property).is_ok()),
        (ResolvedValue::Object(values), ValueShape::Object(fields)) => {
            values.iter().all(|(name, value)| {
                fields
                    .get(name)
                    .is_some_and(|field| validate_value_shape(value, &field.shape, name).is_ok())
            }) && fields.iter().all(|(name, field)| {
                !field.required || field.default.is_some() || values.contains_key(name)
            })
        }
        (
            ResolvedValue::Scalar(_),
            ValueShape::Boolean | ValueShape::Integer | ValueShape::Number | ValueShape::String,
        ) => true,
        _ => false,
    };
    if matches {
        Ok(())
    } else {
        Err(RegistryError::InvalidPropertyType {
            property: property.to_string(),
            expected: format!("{shape:?}"),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NativeRegistryProfileId(String);

impl NativeRegistryProfileId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NativeBuiltinProfileId(String);

impl NativeBuiltinProfileId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error(transparent)]
    Chart(#[from] AvengerChartError),
    #[error(transparent)]
    Schema(#[from] avenger_chart_schema::SchemaError),
    #[error(transparent)]
    NativeLowering(#[from] NativeLoweringError),
    #[error("duplicate coordinate '{0}'")]
    DuplicateCoordinate(String),
    #[error("duplicate {namespace:?} kind '{kind}'")]
    DuplicateKind {
        namespace: NativeKindNamespace,
        kind: String,
    },
    #[error("duplicate native schema entry {0:?}")]
    DuplicateSchema(NativeKindKey),
    #[error("native lowerers must be registered inside a built-in or native-module scope")]
    RegistrationScopeRequired,
    #[error("cannot nest native registration scopes")]
    NestedRegistrationScope,
    #[error("native registration scope was not finished")]
    UnfinishedRegistrationScope,
    #[error("native module '{0}' was not finished")]
    UnfinishedNativeModule(NativeModuleId),
    #[error("duplicate native module '{0}'")]
    DuplicateNativeModule(NativeModuleId),
    #[error("native module '{module}' has duplicate export '{name}'")]
    DuplicateNativeExport {
        module: NativeModuleId,
        name: String,
    },
    #[error("unknown native module '{0}'")]
    UnknownNativeModule(NativeModuleId),
    #[error("native module '{module}' has no export '{name}'")]
    UnknownNativeExport {
        module: NativeModuleId,
        name: String,
    },
    #[error(
        "native registration ownership does not match registry entries (unowned: {unowned:?}, missing: {missing:?})"
    )]
    RegistrationOwnershipMismatch {
        unowned: Vec<NativeKindKey>,
        missing: Vec<NativeKindKey>,
    },
    #[error(
        "native module '{module}' registration/export mismatch (unexported: {unexported:?}, unregistered: {unregistered:?})"
    )]
    NativeModuleOwnershipMismatch {
        module: NativeModuleId,
        unexported: Vec<NativeKindKey>,
        unregistered: Vec<NativeKindKey>,
    },
    #[error("schema/lowerer namespace mismatch for {0:?}")]
    SchemaLowererMismatch(NativeKindKey),
    #[error("unknown coordinate '{0}'")]
    UnknownCoordinate(String),
    #[error("coordinate pack '{coordinate}' does not use expected Rust type '{expected}'")]
    CoordinatePackTypeMismatch {
        coordinate: String,
        expected: String,
    },
    #[error("unknown mark '{mark}' for coordinate '{coordinate}'")]
    UnknownMarkPair { coordinate: String, mark: String },
    #[error("unknown tool '{tool}' for coordinate '{coordinate}'")]
    UnknownToolPair { coordinate: String, tool: String },
    #[error("unknown {namespace:?} kind '{kind}'")]
    UnknownKind {
        namespace: NativeKindNamespace,
        kind: String,
    },
    #[error("kind '{kind}' is missing required property '{property}'")]
    MissingProperty { kind: String, property: String },
    #[error("kind '{kind}' has unknown property '{property}'")]
    UnknownProperty { kind: String, property: String },
    #[error("property '{property}' expected {expected}")]
    InvalidPropertyType { property: String, expected: String },
    #[error("failed to lower '{kind}': {message}")]
    Lowering { kind: String, message: String },
    #[error("transform '{kind}' does not use the expected {expected:?} lowering mode")]
    WrongTransformMode {
        kind: String,
        expected: NativeTransformMode,
    },
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeSet,
        fs,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };

    use avenger_chart::prelude::{
        Auto, Cartesian, CompiledWidget, CoordinationScope, FacetColumn,
        FacetColumnSubplotChannels, IntoPlotMark, PanScrollZoom, Scale, Selection, Subplot, Symbol,
        ToolExportTarget, WidgetItemRow, WidgetItems,
    };
    use avenger_chart_core::{
        ChannelExpr, DataTransformCompileContext, DataTransformStage, ParamRef, RasterDim,
    };
    use avenger_chart_external_test::{
        external_compound_mark::ExternalMeanPoint,
        external_coord_system::{Cube, Isometric},
        external_mark::HexBin,
    };
    use avenger_chart_schema::ChannelSchema;
    use datafusion::{
        common::ScalarValue,
        functions_aggregate::expr_fn::sum,
        logical_expr::{col, lit},
    };
    use indexmap::IndexMap;
    use serde_json::json;

    use super::*;

    fn test_builtin_builder() -> NativeRegistryBuilder {
        let mut builder = NativeRegistry::builder();
        builder.begin_builtin_registration().unwrap();
        builder
    }

    fn build_test_builtins(
        mut builder: NativeRegistryBuilder,
    ) -> Result<NativeRegistry, RegistryError> {
        builder.finish_builtin_registration();
        builder.build()
    }

    fn schema_smoke_value(shape: &ValueShape, property: &str) -> ResolvedValue {
        match shape {
            ValueShape::Boolean => ResolvedValue::Boolean(true),
            ValueShape::Integer => ResolvedValue::Integer(1),
            ValueShape::Number => ResolvedValue::Number(1.0),
            ValueShape::String | ValueShape::Identifier => ResolvedValue::String("x".to_string()),
            ValueShape::Atom { values } => {
                ResolvedValue::String(values.first().unwrap().value.clone())
            }
            ValueShape::SqlExpression => ResolvedValue::Expr(match property {
                "predicate" => lit(true),
                "top_n" => lit(1_i64),
                "fill_value" => lit(0.0_f64),
                "y" => col("y"),
                _ => col("x"),
            }),
            ValueShape::SqlProjection { .. } => {
                ResolvedValue::Projection(vec![ResolvedProjectionItem {
                    expr: sum(col("x")),
                    alias: Some("output".to_string()),
                    direct_column: false,
                }])
            }
            ValueShape::SqlQuery => ResolvedValue::Query("SELECT * FROM input".to_string()),
            ValueShape::ChannelConfig => {
                ResolvedValue::Channel(Box::new(ChannelExpr::scaled(col("x"))))
            }
            ValueShape::ConfiguredExpression(fields) => ResolvedValue::Configured {
                head: Box::new(ResolvedValue::Expr(col("x"))),
                properties: fields
                    .iter()
                    .filter(|(_, field)| field.required)
                    .map(|(name, field)| (name.clone(), schema_smoke_value(&field.shape, name)))
                    .collect(),
            },
            ValueShape::ConfiguredReference { .. } | ValueShape::TypedReference { .. } => {
                ResolvedValue::Output(NativeOutputValue::opaque(()))
            }
            ValueShape::PatternChannel => ResolvedValue::Expr(col("x")),
            ValueShape::CoordinationScope => ResolvedValue::String("shared".to_string()),
            ValueShape::FacetDataScope => ResolvedValue::String("filtered".to_string()),
            ValueShape::RasterDimension => {
                ResolvedValue::Output(NativeOutputValue::RasterDim(RasterDim::new("x")))
            }
            ValueShape::RasterDimensionChannel => ResolvedValue::RasterDimensionChannel {
                dimension: RasterDim::new("x"),
                channel: Box::new(col("x").into()),
            },
            ValueShape::ScalarBinding => ResolvedValue::Param(Param::new("smoke", 1_i64)),
            ValueShape::TableBinding => ResolvedValue::String("input".to_string()),
            ValueShape::SelectionBinding => ResolvedValue::Selection(Selection::new("smoke")),
            ValueShape::WidgetData => ResolvedValue::WidgetItems(WidgetItems::Static(vec![])),
            ValueShape::StateActionBlock => ResolvedValue::Object(IndexMap::new()),
            ValueShape::MarkBlock => ResolvedValue::Object(IndexMap::new()),
            ValueShape::Union(shapes) => schema_smoke_value(shapes.first().unwrap(), property),
            ValueShape::OneOrMany(inner) => schema_smoke_value(inner, property),
            ValueShape::Array(inner) => {
                ResolvedValue::Array(vec![schema_smoke_value(inner, property)])
            }
            ValueShape::Map(inner) => ResolvedValue::Object(IndexMap::from([(
                "x".to_string(),
                schema_smoke_value(inner, "x"),
            )])),
            ValueShape::ChannelMap => ResolvedValue::Object(IndexMap::from([
                (
                    "x".to_string(),
                    ResolvedValue::Channel(Box::new(ChannelExpr::scaled(col("x")))),
                ),
                (
                    "y".to_string(),
                    ResolvedValue::Channel(Box::new(ChannelExpr::scaled(col("y")))),
                ),
            ])),
            ValueShape::Object(fields) => ResolvedValue::Object(
                fields
                    .iter()
                    .filter(|(_, field)| field.required)
                    .map(|(name, field)| (name.clone(), schema_smoke_value(&field.shape, name)))
                    .collect(),
            ),
            ValueShape::Any => ResolvedValue::String("smoke".to_string()),
        }
    }

    fn schema_smoke_declaration(schema: &KindSchema) -> ResolvedDeclaration {
        let mut declaration =
            ResolvedDeclaration::new(schema.key.kind.clone()).source_name("smoke");
        for (name, property) in schema.properties.iter().filter(|(_, value)| value.required) {
            declaration
                .properties
                .insert(name.clone(), schema_smoke_value(&property.shape, name));
        }
        for (name, channel) in schema.channels.iter().filter(|(_, value)| value.required) {
            declaration
                .properties
                .insert(name.clone(), schema_smoke_value(&channel.shape, name));
        }
        declaration
    }

    fn symbol() -> ResolvedDeclaration {
        ResolvedDeclaration::new("symbol")
            .property("x", ResolvedValue::Expr(col("x")))
            .property("y", ResolvedValue::Expr(col("y")))
    }

    fn radio_widget() -> ResolvedDeclaration {
        ResolvedDeclaration::new("radio_button_list")
            .source_name("region")
            .property(
                "data",
                ResolvedValue::WidgetItems(WidgetItems::Static(vec![
                    WidgetItemRow::new([
                        (
                            "value".to_string(),
                            ScalarValue::Utf8(Some("north".to_string())),
                        ),
                        (
                            "label".to_string(),
                            ScalarValue::Utf8(Some("North".to_string())),
                        ),
                    ]),
                    WidgetItemRow::new([
                        (
                            "value".to_string(),
                            ScalarValue::Utf8(Some("south".to_string())),
                        ),
                        (
                            "label".to_string(),
                            ScalarValue::Utf8(Some("South".to_string())),
                        ),
                    ]),
                ])),
            )
            .property("position", ResolvedValue::String("left".to_string()))
    }

    #[test]
    fn stock_v1_schema_is_deterministic_and_profile_is_schema_derived() {
        let left = builtins::stock_registry().unwrap();
        let right = builtins::stock_registry().unwrap();
        assert_eq!(
            left.canonical_schema_json().unwrap(),
            right.canonical_schema_json().unwrap()
        );
        assert_eq!(left.profile_id(), right.profile_id());

        let mut builder = NativeRegistryBuilder::new(1, builtins::STOCK_V1_PROFILE_LABEL);
        builtins::register_stock_builtins(&mut builder).unwrap();
        let module_id = NativeModuleId::new("native:com.acme.symbols@1").unwrap();
        let external_key = NativeKindKey::mark("cartesian", "external_symbol");
        let mut module = builder
            .native_module(
                module_id.clone(),
                "Acme symbol extensions.",
                NativeModuleImplementationProfileId::new("acme-symbols-rust-v1").unwrap(),
            )
            .unwrap();
        module
            .registry()
            .register_mark::<Cartesian>(
                "cartesian",
                "external_symbol",
                KindSchema::new(external_key.clone(), "External symbol fixture."),
                |_| Ok(Symbol::<Cartesian>::new().x(0.0).y(0.0).into_plot_marks()),
            )
            .unwrap();
        module
            .export(
                "symbol",
                external_key,
                "A Cartesian symbol supplied by Acme.",
            )
            .unwrap();
        module.finish().unwrap();
        let extended = builder.build().unwrap();
        assert_ne!(left.profile_id(), extended.profile_id());
        assert_eq!(left.builtin_profile_id(), extended.builtin_profile_id());
        assert_eq!(
            extended
                .native_export(&module_id, "symbol")
                .unwrap()
                .export
                .category,
            NativeKindNamespace::Mark
        );
    }

    fn registry_with_profile(
        module_id: &NativeModuleId,
        implementation_profile: &str,
    ) -> NativeRegistry {
        let mut builder = NativeRegistryBuilder::new(1, builtins::STOCK_V1_PROFILE_LABEL);
        builtins::register_stock_builtins(&mut builder).unwrap();
        let implementation = NativeKindKey::mark("cartesian", "profile_fixture_symbol");
        let mut module = builder
            .native_module(
                module_id.clone(),
                "Profile isolation fixture.",
                NativeModuleImplementationProfileId::new(implementation_profile).unwrap(),
            )
            .unwrap();
        module
            .registry()
            .register_mark::<Cartesian>(
                "cartesian",
                "profile_fixture_symbol",
                KindSchema::new(implementation.clone(), "Profile fixture symbol."),
                |_| Ok(Symbol::<Cartesian>::new().x(0.0).y(0.0).into_plot_marks()),
            )
            .unwrap();
        module
            .export("symbol", implementation, "Profile fixture export.")
            .unwrap();
        module.finish().unwrap();
        builder.build().unwrap()
    }

    #[test]
    fn native_module_profiles_and_public_names_are_independent() {
        let first_id = NativeModuleId::new("native:com.acme.first@1").unwrap();
        let second_id = NativeModuleId::new("native:com.acme.second@1").unwrap();
        let mut builder = NativeRegistryBuilder::new(1, builtins::STOCK_V1_PROFILE_LABEL);
        builtins::register_stock_builtins(&mut builder).unwrap();

        for (id, kind, profile) in [
            (&first_id, "first_symbol", "first-rust-v1"),
            (&second_id, "second_symbol", "second-rust-v1"),
        ] {
            let implementation = NativeKindKey::mark("cartesian", kind);
            let mut module = builder
                .native_module(
                    id.clone(),
                    format!("{kind} module."),
                    NativeModuleImplementationProfileId::new(profile).unwrap(),
                )
                .unwrap();
            module
                .registry()
                .register_mark::<Cartesian>(
                    "cartesian",
                    kind,
                    KindSchema::new(implementation.clone(), format!("{kind} implementation.")),
                    |_| Ok(Symbol::<Cartesian>::new().x(0.0).y(0.0).into_plot_marks()),
                )
                .unwrap();
            module
                .export("symbol", implementation, format!("{kind} public export."))
                .unwrap();
            module.finish().unwrap();
        }

        let registry = builder.build().unwrap();
        let first = registry.native_export(&first_id, "symbol").unwrap();
        let second = registry.native_export(&second_id, "symbol").unwrap();
        assert_ne!(
            first.export.implementation, second.export.implementation,
            "the module ID must disambiguate the same public export spelling"
        );

        let profile_id = NativeModuleId::new("native:com.acme.profile@1").unwrap();
        let old = registry_with_profile(&profile_id, "profile-rust-v1");
        let changed = registry_with_profile(&profile_id, "profile-rust-v2");
        assert_eq!(old.builtin_profile_id(), changed.builtin_profile_id());
        assert_eq!(
            old.native_module(&profile_id).unwrap().1.schema_profile,
            changed.native_module(&profile_id).unwrap().1.schema_profile
        );
        assert_ne!(
            old.native_module(&profile_id)
                .unwrap()
                .1
                .implementation_profile,
            changed
                .native_module(&profile_id)
                .unwrap()
                .1
                .implementation_profile
        );
        assert_ne!(old.profile_id(), changed.profile_id());
    }

    #[test]
    fn native_module_registration_boundaries_are_enforced() {
        let key = NativeKindKey::new(NativeKindNamespace::Transform, "unscoped");
        let mut unscoped = NativeRegistry::builder();
        assert!(matches!(
            unscoped.register_transform(
                KindSchema::new(key, "Unscoped transform."),
                Arc::new(|_, _| unreachable!()),
            ),
            Err(RegistryError::RegistrationScopeRequired)
        ));

        let module_id = NativeModuleId::new("native:com.acme.boundary@1").unwrap();
        let mut builder = NativeRegistry::builder();
        let mut module = builder
            .native_module(
                module_id.clone(),
                "Registration boundary fixture.",
                NativeModuleImplementationProfileId::new("boundary-rust-v1").unwrap(),
            )
            .unwrap();
        let first = NativeKindKey::new(NativeKindNamespace::Transform, "boundary_first");
        let second = NativeKindKey::new(NativeKindNamespace::Transform, "boundary_second");
        for key in [&first, &second] {
            module
                .registry()
                .register_transform(
                    KindSchema::new(key.clone(), "Boundary transform."),
                    Arc::new(|_, _| unreachable!()),
                )
                .unwrap();
        }
        module
            .export("transform", first, "First public transform.")
            .unwrap();
        assert!(matches!(
            module.export("transform", second, "Duplicate public transform."),
            Err(RegistryError::DuplicateNativeExport { .. })
        ));
        drop(module);
        assert!(matches!(
            builder.build(),
            Err(RegistryError::UnfinishedNativeModule(_))
        ));

        let missing_id = NativeModuleId::new("native:com.acme.missing-export@1").unwrap();
        let mut builder = NativeRegistry::builder();
        let mut module = builder
            .native_module(
                missing_id.clone(),
                "Missing export fixture.",
                NativeModuleImplementationProfileId::new("missing-export-rust-v1").unwrap(),
            )
            .unwrap();
        let hidden = NativeKindKey::new(NativeKindNamespace::Transform, "hidden_transform");
        module
            .registry()
            .register_transform(
                KindSchema::new(hidden, "Unexported transform."),
                Arc::new(|_, _| unreachable!()),
            )
            .unwrap();
        module.finish().unwrap();
        assert!(matches!(
            builder.build(),
            Err(RegistryError::NativeModuleOwnershipMismatch { .. })
        ));

        let duplicate_id = NativeModuleId::new("native:com.acme.duplicate@1").unwrap();
        let mut builder = NativeRegistry::builder();
        builder
            .native_module(
                duplicate_id.clone(),
                "First registration.",
                NativeModuleImplementationProfileId::new("duplicate-rust-v1").unwrap(),
            )
            .unwrap()
            .finish()
            .unwrap();
        assert!(matches!(
            builder.native_module(
                duplicate_id.clone(),
                "Second registration.",
                NativeModuleImplementationProfileId::new("duplicate-rust-v2").unwrap(),
            ),
            Err(RegistryError::DuplicateNativeModule(id)) if id == duplicate_id
        ));
    }

    #[test]
    fn native_module_lookup_distinguishes_missing_modules_and_exports() {
        let registry = builtins::stock_registry().unwrap();
        let missing = NativeModuleId::new("native:com.acme.not-installed@1").unwrap();
        assert!(matches!(
            registry.native_export(&missing, "symbol"),
            Err(RegistryError::UnknownNativeModule(id)) if id == missing
        ));

        let module_id = NativeModuleId::new("native:com.acme.lookup@1").unwrap();
        let registry = registry_with_profile(&module_id, "lookup-rust-v1");
        assert!(matches!(
            registry.native_export(&module_id, "missing"),
            Err(RegistryError::UnknownNativeExport { module, name })
                if module == module_id && name == "missing"
        ));
    }

    #[test]
    fn stock_v1_every_schema_entry_has_a_paired_lowerer() {
        let registry = builtins::stock_registry().unwrap();
        let paired = registry
            .coordinates
            .values()
            .flat_map(|pack| pack.schemas())
            .map(|schema| schema.key)
            .chain(
                registry
                    .adjustments
                    .values()
                    .map(|entry| entry.schema.key.clone()),
            )
            .chain(
                registry
                    .transforms
                    .values()
                    .map(|entry| entry.schema.key.clone()),
            )
            .chain(
                registry
                    .widgets
                    .values()
                    .map(|entry| entry.schema.key.clone()),
            )
            .chain(
                registry
                    .objects
                    .values()
                    .map(|entry| entry.schema.key.clone()),
            )
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            paired,
            registry
                .snapshot()
                .entries
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
        );
    }

    #[test]
    fn native_surface_all_stock_scales_have_owner_lowerers() {
        let registry = builtins::bootstrap_registry().unwrap();
        let kinds = [
            "linear",
            "log",
            "pow",
            "sqrt",
            "symlog",
            "time",
            "band",
            "nested_band",
            "point",
            "ordinal",
            "threshold",
            "quantile",
            "quantize",
        ];
        for kind in kinds {
            let key = NativeKindKey::new(NativeKindNamespace::Scale, kind);
            let declaration = ResolvedDeclaration::new(kind);
            let lowered = registry
                .lower_object(&key, &declaration)
                .unwrap_or_else(|error| panic!("failed to lower {kind}: {error}"));
            assert!(lowered.downcast::<Scale<Auto>>().is_ok(), "{kind}");
        }

        let key = NativeKindKey::new(NativeKindNamespace::Scale, "linear");
        let declaration = ResolvedDeclaration::new("linear")
            .property(
                "domain",
                ResolvedValue::Array(vec![
                    ResolvedValue::Number(0.0),
                    ResolvedValue::Number(10.0),
                ]),
            )
            .property("nice", ResolvedValue::Boolean(true))
            .property("clamp", ResolvedValue::Boolean(true));
        registry.lower_object(&key, &declaration).unwrap();
    }

    #[test]
    fn native_surface_axis_and_legend_owners_cover_public_configuration() {
        let registry = builtins::bootstrap_registry().unwrap();
        for (kind, property) in [("cartesian", "label_angle"), ("polar", "start_angle")] {
            let key = NativeKindKey::new(NativeKindNamespace::Axis, kind);
            let declaration =
                ResolvedDeclaration::new(kind).property(property, ResolvedValue::Expr(lit(30.0)));
            let lowered = registry.lower_object(&key, &declaration).unwrap();
            assert!(
                lowered
                    .downcast::<Box<dyn avenger_chart_core::Axis>>()
                    .is_ok()
            );
        }

        let key = NativeKindKey::new(NativeKindNamespace::Legend, "standard");
        let declaration = ResolvedDeclaration::new("standard")
            .property("title", ResolvedValue::Expr(lit("Legend")))
            .property("title_syntax", ResolvedValue::String("typst".to_string()))
            .property("background_padding", ResolvedValue::Expr(lit(8.0)));
        let lowered = registry.lower_object(&key, &declaration).unwrap();
        assert!(lowered.downcast::<avenger_chart_core::Legend>().is_ok());
    }

    #[test]
    fn native_surface_geo_tile_resource_has_an_owner_lowerer() {
        let registry = builtins::bootstrap_registry().unwrap();
        let key = NativeKindKey::new(NativeKindNamespace::Resource, "tiles");
        let declaration = ResolvedDeclaration::new("tiles")
            .source_name("osm")
            .property("kind", ResolvedValue::String("xyz".to_string()))
            .property(
                "url",
                ResolvedValue::String("https://tile.example/{z}/{x}/{y}.png".to_string()),
            )
            .property("min_zoom", ResolvedValue::Integer(0))
            .property("max_zoom", ResolvedValue::Integer(19));
        let layer = registry
            .lower_object(&key, &declaration)
            .unwrap()
            .downcast::<avenger_chart_geo::RasterTileLayer>()
            .unwrap();
        assert_eq!(layer.layer_id(), "osm");
        assert_eq!(layer.min_zoom_value(), 0);
        assert_eq!(layer.max_zoom_value(), 19);
    }

    #[tokio::test]
    async fn native_surface_stock_tools_expand_with_stable_public_exports() {
        fn selection_tool(kind: &str, name: &str) -> ResolvedDeclaration {
            ResolvedDeclaration::new(kind)
                .source_name(name)
                .property(
                    "selection",
                    ResolvedValue::Selection(Selection::new(format!("{name}_selection"))),
                )
                .property(
                    "fields",
                    ResolvedValue::Array(vec![ResolvedValue::String("x".to_string())]),
                )
        }

        let registry = builtins::bootstrap_registry().unwrap();
        let context = SessionContext::new();
        let mut plot = ResolvedPlot::new("cartesian");
        plot.data = Some(context.sql("SELECT 1.0 AS x, 2.0 AS y").await.unwrap());
        plot.marks.push(symbol().into());
        plot.tools.push(selection_tool("point_selection", "points"));
        plot.tools.push(selection_tool("lasso_selection", "lasso"));
        plot.tools.push(
            ResolvedDeclaration::new("box_selection")
                .source_name("brush")
                .property(
                    "selection",
                    ResolvedValue::Selection(Selection::new("brush_selection")),
                ),
        );
        let compiled = registry.compile_root(&plot, &context).await.unwrap();
        for kind in [
            "pan_scroll_zoom",
            "point_selection",
            "lasso_selection",
            "box_selection",
            "box_zoom",
        ] {
            assert!(
                registry
                    .snapshot()
                    .entries
                    .contains_key(&NativeKindKey::new(NativeKindNamespace::Tool, kind)),
                "{kind}"
            );
        }
        let aliases = compiled
            .tool_behaviors()
            .iter()
            .flat_map(|behavior| behavior.exports.iter())
            .map(|export| export.alias.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert!(aliases.contains("selection"));
        assert!(aliases.contains("store"));

        for (kind, name) in [("pan_scroll_zoom", "navigation"), ("box_zoom", "zoom_box")] {
            let mut plot = ResolvedPlot::new("cartesian");
            plot.data = Some(context.sql("SELECT 1.0 AS x, 2.0 AS y").await.unwrap());
            plot.marks.push(symbol().into());
            plot.tools
                .push(ResolvedDeclaration::new(kind).source_name(name));
            let compiled = registry.compile_root(&plot, &context).await.unwrap();
            assert!(
                compiled
                    .tool_behaviors()
                    .iter()
                    .flat_map(|behavior| &behavior.exports)
                    .any(|export| export.alias == "x_domain")
            );
        }

        let mut geo = ResolvedPlot::new("geo");
        geo.tools
            .push(ResolvedDeclaration::new("geo_pan_zoom").source_name("map_nav"));
        let compiled = registry.compile_root(&geo, &context).await.unwrap();
        assert!(
            compiled
                .tool_behaviors()
                .iter()
                .flat_map(|behavior| &behavior.exports)
                .any(|export| export.alias == "center_x")
        );
    }

    #[test]
    fn checked_full_v1_schema_and_documentation_do_not_drift() {
        let registry = builtins::stock_registry().unwrap();
        if std::env::var_os("AVENGER_LANG_UPDATE_BASELINES").is_some() {
            let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            fs::write(
                root.join("snapshots/full-v1-authoring-schema.json"),
                serde_json::to_string_pretty(registry.snapshot()).unwrap() + "\n",
            )
            .unwrap();
            fs::write(
                root.join("docs/full-v1-native-kinds.md"),
                registry.snapshot().markdown_reference(),
            )
            .unwrap();
            return;
        }
        let checked: NativeSchemaSnapshot =
            serde_json::from_str(include_str!("../snapshots/full-v1-authoring-schema.json"))
                .unwrap();
        assert_eq!(registry.snapshot(), &checked);
        assert_eq!(
            registry.snapshot().markdown_reference(),
            include_str!("../docs/full-v1-native-kinds.md")
        );
    }

    #[test]
    fn checked_bootstrap_schema_does_not_drift() {
        let registry = builtins::bootstrap_registry().unwrap();
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let path = root.join("snapshots/bootstrap-schema.json");
        if std::env::var_os("AVENGER_LANG_UPDATE_BASELINES").is_some() {
            fs::write(
                path,
                serde_json::to_string_pretty(registry.snapshot()).unwrap() + "\n",
            )
            .unwrap();
            fs::write(
                root.join("docs/bootstrap-native-kinds.md"),
                registry.snapshot().markdown_reference(),
            )
            .unwrap();
            return;
        }
        let checked: NativeSchemaSnapshot =
            serde_json::from_str(include_str!("../snapshots/bootstrap-schema.json")).unwrap();
        assert_eq!(registry.snapshot(), &checked);
        assert_eq!(
            registry.snapshot().markdown_reference(),
            include_str!("../docs/bootstrap-native-kinds.md")
        );
    }

    #[tokio::test]
    async fn bootstrap_vertical_slice_compiles_evaluates_and_serializes() {
        let registry = builtins::bootstrap_registry().unwrap();
        let context = SessionContext::new();
        let mut plot = ResolvedPlot::new("cartesian");
        plot.data = Some(context.sql("SELECT 10.0 AS x, 20.0 AS y").await.unwrap());
        plot.marks.push(symbol().into());
        plot.tools.push(ResolvedDeclaration::new("pan_scroll_zoom"));
        plot.widgets.push(radio_widget());
        let compiled = registry.compile_root(&plot, &context).await.unwrap();

        assert_eq!(compiled.marks().len(), 1);
        assert_eq!(compiled.widgets().len(), 1);
        assert_eq!(compiled.tool_behaviors().len(), 2);
        let pan = compiled
            .tool_behaviors()
            .iter()
            .find(|behavior| behavior.component_kind == "pan_scroll_zoom")
            .unwrap();
        assert_eq!(pan.component_kind, "pan_scroll_zoom");
        assert!(pan.exports.iter().any(|export| export.alias == "x_domain"));
        assert!(pan.exports.iter().any(|export| export.alias == "y_domain"));

        let CompiledWidget::Composed(widget) = &compiled.widgets()[0].widget else {
            panic!("bootstrap widget must use the composed widget path")
        };
        assert_eq!(widget.kind, "radio-button-list");
        assert!(widget.behavior_exports.iter().any(|export| {
            export.alias == "value" && matches!(&export.target, ToolExportTarget::Param(_))
        }));

        let bytes = bincode::serialize(&compiled).unwrap();
        let decoded: CompiledPlot = bincode::deserialize(&bytes).unwrap();
        assert_eq!(decoded.widgets().len(), 1);
        decoded.evaluate(&context, None).await.unwrap();
    }

    #[test]
    fn schema_validation_precedes_lowering() {
        let called = Arc::new(AtomicBool::new(false));
        let called_by_lowerer = called.clone();
        let mark_schema = KindSchema::new(
            NativeKindKey::mark("cartesian", "strict"),
            "Strict validation fixture.",
        )
        .channel(ChannelSchema {
            name: "x".to_string(),
            required: true,
            shape: ValueShape::SqlExpression,
            item_type: None,
            docs: "Required x expression.".to_string(),
        });
        let pack = CoordinatePack::new(
            "cartesian",
            KindSchema::new(
                NativeKindKey::new(NativeKindNamespace::Coordinate, "cartesian"),
                "Cartesian validation fixture.",
            ),
            |_| Ok(Cartesian::new()),
        )
        .mark("strict", mark_schema, move |_| {
            called_by_lowerer.store(true, Ordering::SeqCst);
            Ok(Symbol::<Cartesian>::new().x(0.0).y(0.0).into_plot_marks())
        });
        let mut builder = test_builtin_builder();
        builder.register_coordinate_pack(pack).unwrap();
        let registry = build_test_builtins(builder).unwrap();
        let mut plot = ResolvedPlot::new("cartesian");
        plot.marks.push(
            ResolvedDeclaration::new("strict")
                .property("x", ResolvedValue::Object(IndexMap::new()))
                .into(),
        );
        assert!(matches!(
            registry.lower_child_plot(&plot),
            Err(RegistryError::InvalidPropertyType { .. })
        ));
        assert!(!called.load(Ordering::SeqCst));
    }

    #[test]
    fn duplicate_and_mismatched_registrations_are_rejected() {
        let coordinate_schema = || {
            KindSchema::new(
                NativeKindKey::new(NativeKindNamespace::Coordinate, "cartesian"),
                "Cartesian fixture.",
            )
        };
        let pack =
            || CoordinatePack::new("cartesian", coordinate_schema(), |_| Ok(Cartesian::new()));
        let mut builder = test_builtin_builder();
        builder.register_coordinate_pack(pack()).unwrap();
        assert!(matches!(
            builder.register_coordinate_pack(pack()),
            Err(RegistryError::DuplicateCoordinate(_))
        ));
        assert!(matches!(
            builder.register_mark::<FacetColumn>(
                "cartesian",
                "wrong_rust_type",
                KindSchema::new(
                    NativeKindKey::mark("cartesian", "wrong_rust_type"),
                    "Wrong typed-pack fixture.",
                ),
                |_| Ok(Vec::new()),
            ),
            Err(RegistryError::CoordinatePackTypeMismatch { .. })
        ));
        assert!(matches!(
            builder.register_mark::<Cartesian>(
                "missing",
                "dot",
                KindSchema::new(
                    NativeKindKey::mark("missing", "dot"),
                    "Missing coordinate fixture.",
                ),
                |_| Ok(Vec::new()),
            ),
            Err(RegistryError::UnknownCoordinate(kind)) if kind == "missing"
        ));

        let duplicate_mark = pack()
            .mark(
                "same",
                KindSchema::new(NativeKindKey::mark("cartesian", "same"), "First mark."),
                |_| Ok(Symbol::<Cartesian>::new().into_plot_marks()),
            )
            .mark(
                "same",
                KindSchema::new(NativeKindKey::mark("cartesian", "same"), "Second mark."),
                |_| Ok(Symbol::<Cartesian>::new().into_plot_marks()),
            );
        let mut builder = test_builtin_builder();
        builder.register_coordinate_pack(duplicate_mark).unwrap();
        assert!(matches!(
            build_test_builtins(builder),
            Err(RegistryError::DuplicateSchema(_))
        ));

        let tool_schema = || {
            let mut schema = KindSchema::new(
                NativeKindKey::new(NativeKindNamespace::Tool, "same_tool"),
                "Duplicate tool fixture.",
            );
            schema
                .compatible_coordinates
                .insert("cartesian".to_string());
            schema
        };
        let duplicate_tool = pack()
            .tool("same_tool", tool_schema(), |_| {
                Ok(Arc::new(PanScrollZoom::cartesian()))
            })
            .tool("same_tool", tool_schema(), |_| {
                Ok(Arc::new(PanScrollZoom::cartesian()))
            });
        let mut builder = test_builtin_builder();
        builder.register_coordinate_pack(duplicate_tool).unwrap();
        assert!(matches!(
            build_test_builtins(builder),
            Err(RegistryError::DuplicateSchema(_))
        ));

        let transform_schema = KindSchema::new(
            NativeKindKey::new(NativeKindNamespace::Transform, "same_transform"),
            "Duplicate transform fixture.",
        );
        let transform_lowerer: NativeTransformLowerer = Arc::new(|_, _| unreachable!());
        let mut builder = test_builtin_builder();
        builder
            .register_transform(transform_schema.clone(), transform_lowerer.clone())
            .unwrap();
        assert!(matches!(
            builder.register_transform(transform_schema, transform_lowerer),
            Err(RegistryError::DuplicateKind {
                namespace: NativeKindNamespace::Transform,
                ..
            })
        ));

        let widget_schema = KindSchema::new(
            NativeKindKey::new(NativeKindNamespace::Widget, "same_widget"),
            "Duplicate widget fixture.",
        );
        let widget_lowerer: NativeWidgetLowerer = Arc::new(|_| unreachable!());
        let mut builder = test_builtin_builder();
        builder
            .register_widget(widget_schema.clone(), widget_lowerer.clone())
            .unwrap();
        assert!(matches!(
            builder.register_widget(widget_schema, widget_lowerer),
            Err(RegistryError::DuplicateKind {
                namespace: NativeKindNamespace::Widget,
                ..
            })
        ));

        let mismatch = CoordinatePack::new(
            "cartesian",
            KindSchema::new(
                NativeKindKey::new(NativeKindNamespace::Coordinate, "polar"),
                "Mismatched coordinate fixture.",
            ),
            |_| Ok(Cartesian::new()),
        );
        let mut builder = test_builtin_builder();
        builder.register_coordinate_pack(mismatch).unwrap();
        assert!(matches!(
            build_test_builtins(builder),
            Err(RegistryError::SchemaLowererMismatch(_))
        ));
    }

    #[tokio::test]
    async fn bootstrap_symbol_channel_inventory_tracks_compiled_symbol() {
        let registry = builtins::bootstrap_registry().unwrap();
        let schema = registry
            .snapshot()
            .entries
            .get(&NativeKindKey::mark("cartesian", "symbol"))
            .unwrap();
        let context = SessionContext::new();
        let compiled = registry
            .compile_root(
                &ResolvedPlot {
                    coordinate: ResolvedDeclaration::new("cartesian"),
                    data: None,
                    data_is_inherited: false,
                    marks: vec![symbol().into()],
                    tools: Vec::new(),
                    tool_behaviors: Vec::new(),
                    widgets: Vec::new(),
                    children: Vec::new(),
                    furnishings: ResolvedRootFurnishings::default(),
                },
                &context,
            )
            .await
            .unwrap();
        let rust_channels: std::collections::BTreeSet<_> = compiled.marks()[0]
            .supported_channels()
            .into_iter()
            .map(|channel| channel.name.to_string())
            .collect();
        assert_eq!(
            schema
                .channels
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>(),
            rust_channels
        );
    }

    #[test]
    fn transforms_lower_with_output_handles() {
        let registry = builtins::bootstrap_registry().unwrap();
        let context = DataTransformCompileContext::new(CoordinationScope::Shared);
        registry
            .lower_transform(
                &ResolvedDeclaration::new("filter")
                    .property("predicate", ResolvedValue::Expr(col("x").gt(lit(0)))),
                context,
            )
            .unwrap();
        registry
            .lower_transform(
                &ResolvedDeclaration::new("sql").property(
                    "query",
                    ResolvedValue::Query("SELECT * FROM input".to_string()),
                ),
                context,
            )
            .unwrap();
        let aggregate = registry
            .lower_transform(
                &ResolvedDeclaration::new("aggregate").property(
                    "expressions",
                    ResolvedValue::Projection(vec![ResolvedProjectionItem {
                        expr: sum(col("x")),
                        alias: Some("total".to_string()),
                        direct_column: false,
                    }]),
                ),
                context,
            )
            .unwrap();
        assert_eq!(aggregate.outputs.keys().collect::<Vec<_>>(), vec!["total"]);
    }

    #[test]
    fn native_surface_schema_generated_smoke_lowers_every_transform_kind() {
        let registry = builtins::stock_registry().unwrap();
        let context = DataTransformCompileContext::new(CoordinationScope::Shared);
        let schemas = registry
            .snapshot()
            .entries
            .values()
            .filter(|schema| schema.key.namespace == NativeKindNamespace::Transform)
            .cloned()
            .collect::<Vec<_>>();
        let levels_schema = schemas
            .iter()
            .find(|schema| schema.key.kind == "time_levels")
            .unwrap();
        let levels = registry
            .lower_transform(&schema_smoke_declaration(levels_schema), context)
            .unwrap()
            .outputs
            .remove("levels")
            .unwrap();

        let mut lowered = BTreeSet::new();
        for schema in schemas {
            let kind = schema.key.kind.clone();
            let mut declaration = schema_smoke_declaration(&schema);
            if kind == "impute" {
                declaration.properties.insert(
                    "method".to_string(),
                    ResolvedValue::String("mean".to_string()),
                );
            }
            if matches!(
                kind.as_str(),
                "aggregate" | "join_aggregate" | "scalar_aggregate"
            ) {
                declaration.properties.insert(
                    "expressions".to_string(),
                    ResolvedValue::Projection(vec![ResolvedProjectionItem {
                        expr: sum(col("x")),
                        alias: Some("total".to_string()),
                        direct_column: false,
                    }]),
                );
            }
            let result = if kind == "pipeline" {
                let filter = registry
                    .lower_transform(
                        &ResolvedDeclaration::new("filter")
                            .property("predicate", ResolvedValue::Expr(lit(true))),
                        context,
                    )
                    .unwrap();
                registry.lower_transform_pipeline(
                    &declaration,
                    vec![DataTransformStage::new(context.scope, filter.transform)],
                    IndexMap::from([("x".to_string(), col("x"))]),
                    context,
                )
            } else {
                if kind == "time_fill" {
                    declaration
                        .properties
                        .insert("levels".to_string(), ResolvedValue::Output(levels.clone()));
                }
                registry.lower_transform(&declaration, context)
            };
            result.unwrap_or_else(|error| panic!("schema-generated {kind} smoke failed: {error}"));
            lowered.insert(kind);
        }

        let registered = registry
            .snapshot()
            .entries
            .keys()
            .filter(|key| key.namespace == NativeKindNamespace::Transform)
            .map(|key| key.kind.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(lowered, registered);
    }

    #[test]
    fn native_surface_schema_generated_smoke_lowers_every_adjustment_kind() {
        let registry = builtins::stock_registry().unwrap();
        let schemas = registry
            .snapshot()
            .entries
            .values()
            .filter(|schema| schema.key.namespace == NativeKindNamespace::Adjust)
            .collect::<Vec<_>>();

        assert_eq!(schemas.len(), 3);
        for (stage, schema) in schemas.into_iter().enumerate() {
            let lowered = registry
                .lower_adjustment(
                    &schema_smoke_declaration(schema),
                    MarkAdjustmentCompileContext::new(stage),
                )
                .unwrap_or_else(|error| {
                    panic!(
                        "schema-generated {} adjustment smoke failed: {error}",
                        schema.key.kind
                    )
                });
            assert_eq!(
                lowered.outputs.keys().cloned().collect::<BTreeSet<_>>(),
                schema.outputs.keys().cloned().collect::<BTreeSet<_>>()
            );
        }
    }

    #[tokio::test]
    async fn native_surface_schema_generated_smoke_compiles_every_mark_coordinate_pair() {
        let registry = builtins::stock_registry().unwrap();
        let context = SessionContext::new();
        let data = context
            .sql(
                "SELECT 1.0 AS x, 2.0 AS y, 1.0 AS value, \
                 'root' AS region, 'leaf' AS category, 'label' AS label",
            )
            .await
            .unwrap();
        let schemas = registry
            .snapshot()
            .entries
            .values()
            .filter(|schema| schema.key.namespace == NativeKindNamespace::Mark)
            .cloned()
            .collect::<Vec<_>>();
        let mut lowered = BTreeSet::new();

        for schema in schemas {
            let coordinate = schema.key.coordinate.clone().unwrap();
            let coordinate_schema = registry
                .snapshot()
                .entries
                .get(&NativeKindKey::new(
                    NativeKindNamespace::Coordinate,
                    coordinate.clone(),
                ))
                .unwrap();
            let mut plot = ResolvedPlot::new(coordinate.clone());
            plot.coordinate = schema_smoke_declaration(coordinate_schema);
            plot.data = Some(data.clone());
            if coordinate == "parallel" {
                plot.coordinate
                    .children
                    .push(ResolvedDeclaration::new("dimension").source_name("x"));
                plot.coordinate
                    .children
                    .push(ResolvedDeclaration::new("dimension").source_name("y"));
            } else if coordinate == "treemap" {
                plot.coordinate.properties.insert(
                    "path".to_string(),
                    ResolvedValue::Array(vec![
                        ResolvedValue::Expr(col("region")),
                        ResolvedValue::Expr(col("category")),
                    ]),
                );
                plot.coordinate
                    .properties
                    .insert("value".to_string(), ResolvedValue::Expr(col("value")));
            }

            let mut declaration = schema_smoke_declaration(&schema);
            if schema.key.kind == "uniform_raster_2d" {
                declaration.properties.insert(
                    "x".to_string(),
                    ResolvedValue::RasterDimensionChannel {
                        dimension: RasterDim::new("x"),
                        channel: Box::new(col("x").into()),
                    },
                );
                declaration.properties.insert(
                    "y".to_string(),
                    ResolvedValue::RasterDimensionChannel {
                        dimension: RasterDim::new("y"),
                        channel: Box::new(col("y").into()),
                    },
                );
            }
            let mark = if schema.child_rules.iter().any(|rule| rule.role == "plot") {
                ResolvedMark::NativeWithChild {
                    declaration,
                    child: Box::new(ResolvedPlot::new("zerod")),
                }
            } else {
                ResolvedMark::Native(declaration)
            };
            plot.marks.push(mark);
            let key = (coordinate, schema.key.kind.clone());
            registry
                .compile_root(&plot, &context)
                .await
                .unwrap_or_else(|error| {
                    panic!("schema-generated {}/{} smoke failed: {error}", key.0, key.1)
                });
            lowered.insert(key);
        }

        let registered = registry
            .snapshot()
            .entries
            .keys()
            .filter(|key| key.namespace == NativeKindNamespace::Mark)
            .map(|key| (key.coordinate.clone().unwrap(), key.kind.clone()))
            .collect::<BTreeSet<_>>();
        assert_eq!(lowered, registered);
    }

    #[test]
    fn bootstrap_widget_schema_records_authoring_runtime_mapping() {
        let registry = builtins::bootstrap_registry().unwrap();
        let schema = registry
            .snapshot()
            .entries
            .get(&NativeKindKey::new(
                NativeKindNamespace::Widget,
                "radio_button_list",
            ))
            .unwrap();
        assert_eq!(schema.runtime_kind.as_deref(), Some("radio-button-list"));
        assert_eq!(
            schema.parts["focus_ring"].runtime_alias.as_deref(),
            Some("focus-ring")
        );
        assert_eq!(schema.exports["value"].value_kind, "param<item_scalar>");
    }

    #[test]
    fn checked_full_v1_widget_schema_mapping_does_not_drift() {
        let registry = builtins::stock_registry().unwrap();
        let widgets = registry
            .snapshot()
            .entries
            .iter()
            .filter(|(key, _)| key.namespace == NativeKindNamespace::Widget)
            .map(|(key, schema)| {
                json!({
                    "authoring_kind": key.kind,
                    "runtime_kind": schema.runtime_kind,
                    "properties": schema.properties,
                    "parts": schema.parts,
                    "exports": schema.exports,
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(widgets.len(), 6);
        let mapping = json!({
            "schema_version": registry.snapshot().version,
            "profile_label": registry.snapshot().profile_label,
            "widgets": widgets,
        });
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let path = root.join("snapshots/full-v1-widget-schema-mapping.json");
        if std::env::var_os("AVENGER_LANG_UPDATE_BASELINES").is_some() {
            fs::write(path, serde_json::to_string_pretty(&mapping).unwrap() + "\n").unwrap();
            return;
        }
        let checked: serde_json::Value = serde_json::from_str(include_str!(
            "../snapshots/full-v1-widget-schema-mapping.json"
        ))
        .unwrap();
        assert_eq!(mapping, checked);
    }

    #[tokio::test]
    async fn generated_and_supplied_widget_value_share_the_same_resolved_slot() {
        fn exported_value(plot: &CompiledPlot) -> ParamRef {
            let CompiledWidget::Composed(widget) = &plot.widgets()[0].widget else {
                panic!("expected composed widget")
            };
            widget
                .behavior_exports
                .iter()
                .find_map(|export| match (&*export.alias, &export.target) {
                    ("value", ToolExportTarget::Param(id)) => Some(id.clone()),
                    _ => None,
                })
                .expect("radio value export")
        }

        let registry = builtins::bootstrap_registry().unwrap();
        let context = SessionContext::new();
        let mut generated = ResolvedPlot::new("cartesian");
        generated.widgets.push(radio_widget());
        let generated = registry.compile_root(&generated, &context).await.unwrap();

        let mut supplied_declaration = radio_widget();
        supplied_declaration.properties.insert(
            "value_param".to_string(),
            ResolvedValue::Param(Param::new("external_value", "north")),
        );
        let mut supplied = ResolvedPlot::new("cartesian");
        supplied.widgets.push(supplied_declaration);
        let supplied = registry.compile_root(&supplied, &context).await.unwrap();

        assert_eq!(exported_value(&generated), exported_value(&supplied));
    }

    #[tokio::test]
    async fn downstream_extensions_cross_coordinate_and_typetag_boundaries() {
        fn register_extension(builder: &mut NativeRegistryBuilder) -> Result<(), RegistryError> {
            builder.register_mark::<Cartesian>(
                "cartesian",
                "external_hexbin",
                KindSchema::new(
                    NativeKindKey::mark("cartesian", "external_hexbin"),
                    "A Cartesian mark implemented by a downstream crate.",
                ),
                |_| Ok(HexBin::<Cartesian>::new().x(1.0).y(1.0).into_plot_marks()),
            )?;
            builder.register_mark::<Cartesian>(
                "cartesian",
                "external_mean_point",
                KindSchema::new(
                    NativeKindKey::mark("cartesian", "external_mean_point"),
                    "A downstream aggregate-backed compound mark.",
                ),
                |_| Ok(ExternalMeanPoint::new(lit("all"), lit(2.0)).into_plot_marks()),
            )
        }

        let external_coordinate = CoordinatePack::new(
            "external_isometric",
            KindSchema::new(
                NativeKindKey::new(NativeKindNamespace::Coordinate, "external_isometric"),
                "A downstream isometric coordinate pack fixture.",
            ),
            |_| Ok(Isometric::new()),
        )
        .mark(
            "external_cube",
            KindSchema::new(
                NativeKindKey::mark("external_isometric", "external_cube"),
                "A cube implemented by a downstream crate.",
            ),
            |_| {
                Ok(Cube::<Isometric>::new()
                    .iso_x(1.0)
                    .iso_y(2.0)
                    .iso_z(3.0)
                    .into_plot_marks())
            },
        );

        let external_container = CoordinatePack::new(
            "external_facet_column",
            KindSchema::new(
                NativeKindKey::new(NativeKindNamespace::Coordinate, "external_facet_column"),
                "A downstream mixed-coordinate child container.",
            ),
            |_| Ok(FacetColumn),
        )
        .child_plots(|plot, child, placement, _parent| {
            let column = match placement.get("column")? {
                ResolvedValue::Expr(expr) => expr.clone(),
                _ => {
                    return Err(RegistryError::InvalidPropertyType {
                        property: "column".to_string(),
                        expected: "SQL expression".to_string(),
                    });
                }
            };
            Ok(plot.mark(Subplot::<FacetColumn>::new(child).column(column)))
        });

        let mut builder = NativeRegistryBuilder::new(1, "downstream-fixture");
        builtins::register_bootstrap_builtins(&mut builder).unwrap();
        let module_id = NativeModuleId::new("native:com.acme.downstream-fixture@1").unwrap();
        let mut module = builder
            .native_module(
                module_id.clone(),
                "Downstream extension fixture.",
                NativeModuleImplementationProfileId::new("downstream-fixture-rust-v1").unwrap(),
            )
            .unwrap();
        register_extension(module.registry()).unwrap();
        module
            .registry()
            .register_coordinate_pack(external_coordinate)
            .unwrap();
        module
            .registry()
            .register_coordinate_pack(external_container)
            .unwrap();
        for (name, key, docs) in [
            (
                "hexbin",
                NativeKindKey::mark("cartesian", "external_hexbin"),
                "Downstream Cartesian hexbin mark.",
            ),
            (
                "mean_point",
                NativeKindKey::mark("cartesian", "external_mean_point"),
                "Downstream Cartesian aggregate mark.",
            ),
            (
                "isometric",
                NativeKindKey::new(NativeKindNamespace::Coordinate, "external_isometric"),
                "Downstream isometric coordinate.",
            ),
            (
                "cube",
                NativeKindKey::mark("external_isometric", "external_cube"),
                "Downstream isometric cube mark.",
            ),
            (
                "facet_column",
                NativeKindKey::new(NativeKindNamespace::Coordinate, "external_facet_column"),
                "Downstream facet-column coordinate.",
            ),
        ] {
            module.export(name, key, docs).unwrap();
        }
        module.finish().unwrap();
        let registry = builder.build().unwrap();
        assert!(registry.native_export(&module_id, "hexbin").is_ok());
        let context = SessionContext::new();

        let mut augmented = ResolvedPlot::new("cartesian");
        augmented
            .marks
            .push(ResolvedDeclaration::new("external_hexbin").into());
        augmented
            .marks
            .push(ResolvedDeclaration::new("external_mean_point").into());
        let compiled = registry.compile_root(&augmented, &context).await.unwrap();
        assert_eq!(compiled.marks().len(), 2);
        assert_eq!(compiled.marks()[0].mark_type(), "hexbin");
        let decoded: CompiledPlot =
            bincode::deserialize(&bincode::serialize(&compiled).unwrap()).unwrap();
        assert_eq!(decoded.marks()[0].mark_type(), "hexbin");

        let mut external = ResolvedPlot::new("external_isometric");
        external
            .marks
            .push(ResolvedDeclaration::new("external_cube").into());
        let compiled = registry.compile_root(&external, &context).await.unwrap();
        assert_eq!(compiled.marks().len(), 1);
        assert_eq!(compiled.marks()[0].mark_type(), "cube");
        let decoded: CompiledPlot =
            bincode::deserialize(&bincode::serialize(&compiled).unwrap()).unwrap();
        assert_eq!(decoded.marks()[0].mark_type(), "cube");

        let mut built_in_child = ResolvedPlot::new("cartesian");
        built_in_child
            .marks
            .push(ResolvedDeclaration::new("external_hexbin").into());
        let mut parent = ResolvedPlot::new("external_facet_column");
        parent.children.push(ResolvedChildPlot {
            plot: Box::new(built_in_child),
            placement: ResolvedDeclaration::new("child")
                .property("column", ResolvedValue::Expr(lit("built_in"))),
        });
        parent.children.push(ResolvedChildPlot {
            plot: Box::new(external),
            placement: ResolvedDeclaration::new("child")
                .property("column", ResolvedValue::Expr(lit("custom"))),
        });
        let compiled = registry.compile_root(&parent, &context).await.unwrap();
        let bytes = bincode::serialize(&compiled).unwrap();
        let decoded: CompiledPlot = bincode::deserialize(&bytes).unwrap();
        assert_eq!(decoded.marks().len(), 2);
    }
}
