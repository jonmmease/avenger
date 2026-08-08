// Lowering returns source-rich diagnostics internally. Boxing every helper's
// error would add pervasive indirection without shrinking the public failure
// type or changing the one-diagnostic-at-a-time lowering control flow.
#![allow(clippy::result_large_err)]

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    ops::ControlFlow,
    sync::Arc,
};

use arrow::{
    datatypes::{DataType, Field, Schema},
    record_batch::{RecordBatch, RecordBatchOptions},
};
use avenger_chart::{
    cartesian::Cartesian,
    layout::LayoutSpec,
    prelude::{
        Auto, ChannelExpr, ChannelValue, ColorbarOverlay, LegendableChannelValue, Linear, Param,
        Scale, ScaleChannelValue, ScaleDomainInference, Selection, Store, WidgetItemRow,
        WidgetItems,
    },
};
use avenger_chart_core::{
    ChartAction, ChartEventBinding, ChartEventStream, ChartEventType,
    CompiledScalarExpressionProgram, ConditionalValue, DataTransformExecutionContext,
    DataTransformStage, DefaultLogicalExprNodeExt, DerivedPrimitiveMarkSpec, DerivedRectMarkSpec,
    DerivedRuleMarkSpec, DerivedSymbolMarkSpec, DerivedTextMarkSpec, ExpressionMarkAdjustmentSpec,
    FormattingContext, ItemChannelAssignment, MarkAdjustmentCompileContext, MarkAdjustmentSpec,
    Param as ChartParam, PatternAnchor, PatternChannelValue, PatternFill, PatternInk, PatternLayer,
    PatternLayerOperation, PhysicalScalarExpressionSpec, PhysicalScalarProgramOptions,
    PrimitiveMarkEffects, SceneGeometryHitPolicy, SceneGeometryQuery, SceneQueryClauseId,
    SceneQueryDatumField, SelectionClauseUpdate, SelectionSceneQuery, SelectionUpdate,
    SerializableScalar, StateMigrationKey, StoreData, StoreFieldPatch, StoreKey, StoreRow,
    StoreUpdate, StripeDash, StripePatternLayer, Theme, TimeContext, TransformMarkAdjustmentSpec,
    ViewRef, WeekStart, event, item_bbox_column_name, item_channel_column_name,
    item_data_column_name,
};
use avenger_chart_lang_registry::{
    NativeOutputValue, NativeRegistry, NativeTransformMode, ResolvedBehaviorExport,
    ResolvedBehaviorExportTarget, ResolvedBehaviorState, ResolvedChildPlot,
    ResolvedDeclaration as NativeDeclaration, ResolvedMark, ResolvedMarkGroup, ResolvedPlot,
    ResolvedProjectionItem as NativeProjectionItem, ResolvedToolBehavior, ResolvedTransformStage,
    ResolvedValue as NativeValue, ResolvedViewScope,
};
use avenger_chart_schema::{NativeKindKey, NativeKindNamespace, ValueShape};
use avenger_lang_core::{
    ChartEntrypointId, DeclarationId, Diagnostic, ImportCapabilities, ParamId, ParamTypeContract,
    ResolvedActionRoute, ResolvedBinding, ResolvedChannelMember, ResolvedContextualAccess,
    ResolvedContextualAccessKind, ResolvedDeclaration, ResolvedEventScope, ResolvedEventSurface,
    ResolvedExpression, ResolvedHelperArgument, ResolvedIntervalBoundary, ResolvedModuleGraph,
    ResolvedOutputHandle, ResolvedOutputShape, ResolvedParam, ResolvedQuery,
    ResolvedRelationTarget, ResolvedSelection, ResolvedSelectionCombine, ResolvedSelectionEmpty,
    ResolvedSqlReference, ResolvedStore, ResolvedTarget, ResolvedValue, ResolvedViewAxis,
    ResolvedViewField, SelectionId, SourceLabel, SourceLoader, SourceSpan, StateSharing, StoreId,
    ast::{BindingTime, SqlExpression, SqlQuery, Visibility, is_state_action_keyword},
    contextual_access_signature,
    module_graph::resolve_relative_origin,
    sql::AvengerSqlDialect,
};
use datafusion::functions_nested::map::map_udf;
use datafusion::{
    common::{
        Column, ScalarValue,
        tree_node::{Transformed, TreeNode},
    },
    dataframe::DataFrame,
    datasource::empty::EmptyTable,
    execution::FunctionRegistry,
    logical_expr::{Expr, ExprSchemable, LogicalPlan, TableScan, cast, col, lit},
    prelude::{SessionContext, make_array, named_struct},
};
use datafusion_proto::protobuf::LogicalExprNode;
use indexmap::IndexMap;
use sqlparser::{
    ast::{Expr as SqlExpr, Value as SqlValue, VisitMut, VisitorMut},
    parser::Parser,
};

use crate::{
    CompiledChartArtifact, DependencyFingerprint, NativeRequirementSet, ParamTypeIndex,
    ParamTypeProvenance,
    sql_profile::{
        exact_numeric_scalar, normalize_sql_expression, normalize_sql_query,
        physical_field_to_arrow, physical_type_to_arrow,
    },
};

pub(crate) struct LoweredChart {
    pub artifact: CompiledChartArtifact,
}

enum LoweredStateAction {
    Cursor(Expr),
    Param {
        param: Param,
        value: Expr,
    },
    Store {
        name: String,
        update: StoreUpdate,
    },
    Selection {
        id: String,
        update: Box<SelectionUpdate>,
    },
}

#[derive(Clone, Debug, Default)]
pub(crate) struct PreparedParams {
    pub values: BTreeMap<ParamId, Param>,
    pub types: ParamTypeIndex,
}

pub(crate) fn prepare_module_params(
    project: &ResolvedModuleGraph,
    registry: &NativeRegistry,
    context: &SessionContext,
    source_loader: &dyn SourceLoader,
    capabilities: &ImportCapabilities,
) -> Result<PreparedParams, Vec<Diagnostic>> {
    let mut lowerer = ModuleLowerer::new(project, registry, context, source_loader, capabilities);
    for id in &project.param_initializer_order {
        if let Err(mut diagnostic) = lowerer.lower_param(id) {
            project
                .expansion_source_map
                .remap_diagnostic(&mut diagnostic);
            return Err(vec![diagnostic]);
        }
    }
    for id in project.params.keys() {
        if !lowerer.params.contains_key(id)
            && let Err(mut diagnostic) = lowerer.lower_param(id)
        {
            project
                .expansion_source_map
                .remap_diagnostic(&mut diagnostic);
            return Err(vec![diagnostic]);
        }
    }
    for requirement in &project.param_type_requirements {
        let ResolvedTarget::Param(id) = &requirement.target else {
            continue;
        };
        let Some(actual) = lowerer
            .params
            .get(id)
            .map(|param| param.default.data_type())
        else {
            continue;
        };
        let expected = physical_type_to_arrow(&requirement.expected);
        if actual != expected {
            let mut diagnostic = Diagnostic::error(
                "AVENGER-PARAM-002",
                "parameter binding has the wrong inferred Arrow type",
                SourceLabel::new(
                    requirement.span,
                    format!(
                        "{} requires `{expected}`, but the bound param infers `{actual}`; cast its initializer explicitly",
                        requirement.role
                    ),
                ),
            );
            project
                .expansion_source_map
                .remap_diagnostic(&mut diagnostic);
            return Err(vec![diagnostic]);
        }
    }
    let mut types = ParamTypeIndex::default();
    for (id, param) in &lowerer.params {
        let provenance = match &project.params[id].type_contract {
            ParamTypeContract::Inferred => ParamTypeProvenance::Inferred,
            ParamTypeContract::SchemaFixed(_) => ParamTypeProvenance::SchemaFixed,
        };
        types.insert(id.clone(), param.default.data_type(), provenance);
    }
    Ok(PreparedParams {
        values: lowerer.params,
        types,
    })
}

pub(crate) struct ChartDatasetAnalysis {
    pub dataset: DeclarationId,
    pub declaration_span: SourceSpan,
    pub stage_span: SourceSpan,
    pub stage_kind: crate::DatasetStageKind,
    pub schema: Arc<Schema>,
    pub columns: Vec<crate::AnalyzedColumn>,
    pub logical_plan_fingerprint: Option<String>,
    pub mark_channels: Vec<crate::AnalyzedMarkChannel>,
}

type TransformStageFuture<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<(DataFrame, DataTransformStage), Diagnostic>> + 'a>,
>;

type ToolBehaviorFuture<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<ResolvedToolBehavior, Diagnostic>> + 'a>,
>;

/// Lower one chart entrypoint in a chart-local session cloned from the
/// module graph's analyzed catalog generation. This is the unit used by the
/// deterministic sequential/parallel scheduler and artifact cache.
pub(crate) async fn lower_module_chart(
    project: &ResolvedModuleGraph,
    entrypoint_id: &ChartEntrypointId,
    registry: &NativeRegistry,
    context: &SessionContext,
    source_loader: &dyn SourceLoader,
    capabilities: &ImportCapabilities,
    prepared_params: &PreparedParams,
) -> Result<LoweredChart, Vec<Diagnostic>> {
    let mut lowerer = ModuleLowerer::new(project, registry, context, source_loader, capabilities);
    lowerer.params.clone_from(&prepared_params.values);
    lowerer.param_types.clone_from(&prepared_params.types);
    let result = match lowerer.lower_entrypoint_state(entrypoint_id) {
        Ok(()) => lowerer.lower_one(entrypoint_id).await,
        Err(diagnostic) => Err(diagnostic),
    };
    match result {
        Ok(chart) => Ok(chart),
        Err(mut diagnostic) => {
            project
                .expansion_source_map
                .remap_diagnostic(&mut diagnostic);
            Err(vec![diagnostic])
        }
    }
}

/// Analyze chart-owned dataflow without lowering marks, tools, layouts, or a
/// native chart. Native transforms are asked only for their logical planning
/// contract and must not create physical plans or execute scans.
pub(crate) async fn analyze_chart_datasets(
    project: &ResolvedModuleGraph,
    registry: &NativeRegistry,
    context: &SessionContext,
    source_loader: &dyn SourceLoader,
    capabilities: &ImportCapabilities,
    prepared_params: &PreparedParams,
) -> Result<Vec<ChartDatasetAnalysis>, Vec<Diagnostic>> {
    let mut analysis = Vec::new();
    let mut diagnostics = Vec::new();
    for (entrypoint_id, entrypoint) in &project.entrypoints {
        let mut lowerer =
            ModuleLowerer::new(project, registry, context, source_loader, capabilities);
        lowerer.params.clone_from(&prepared_params.values);
        lowerer.param_types.clone_from(&prepared_params.types);
        if let Err(mut diagnostic) = lowerer.lower_entrypoint_state(entrypoint_id) {
            project
                .expansion_source_map
                .remap_diagnostic(&mut diagnostic);
            diagnostics.push(diagnostic);
            continue;
        }
        let Some(chart) = find_declaration(project, &entrypoint.declaration) else {
            continue;
        };
        lowerer.active_chart_id = Some(chart.id.clone());
        lowerer.active_chart_path = chart.public_path.clone().or_else(|| chart.name.clone());
        if let Err(mut diagnostic) = lowerer
            .analyze_container_data(chart, None, &mut analysis)
            .await
        {
            project
                .expansion_source_map
                .remap_diagnostic(&mut diagnostic);
            diagnostics.push(diagnostic);
        }
    }
    if diagnostics.is_empty() {
        Ok(analysis)
    } else {
        Err(diagnostics)
    }
}

struct ModuleLowerer<'a> {
    project: &'a ResolvedModuleGraph,
    registry: &'a NativeRegistry,
    context: &'a SessionContext,
    source_loader: &'a dyn SourceLoader,
    capabilities: &'a ImportCapabilities,
    params: BTreeMap<ParamId, Param>,
    param_types: ParamTypeIndex,
    stores: BTreeMap<StoreId, Store>,
    store_table_names: BTreeMap<StoreId, String>,
    selections: BTreeMap<SelectionId, Selection>,
    widget_owned_params: BTreeSet<ParamId>,
    native_owned_stores: BTreeSet<StoreId>,
    native_owned_selections: BTreeSet<SelectionId>,
    transform_outputs: BTreeMap<ResolvedOutputHandle, NativeOutputValue>,
    view_refs: BTreeMap<DeclarationId, ViewRef>,
    legend_overlays: BTreeMap<DeclarationId, ColorbarOverlay>,
    mark_channel_seed_stack: RefCell<BTreeSet<(DeclarationId, String)>>,
    analysis_schemas: Vec<(DeclarationId, SourceSpan, Arc<Schema>)>,
    active_chart_id: Option<DeclarationId>,
    active_chart_path: Option<String>,
}

impl<'a> ModuleLowerer<'a> {
    fn new(
        project: &'a ResolvedModuleGraph,
        registry: &'a NativeRegistry,
        context: &'a SessionContext,
        source_loader: &'a dyn SourceLoader,
        capabilities: &'a ImportCapabilities,
    ) -> Self {
        Self {
            project,
            registry,
            context,
            source_loader,
            capabilities,
            params: BTreeMap::new(),
            param_types: ParamTypeIndex::default(),
            stores: BTreeMap::new(),
            store_table_names: BTreeMap::new(),
            selections: BTreeMap::new(),
            widget_owned_params: widget_owned_params(project),
            native_owned_stores: native_owned_stores(project),
            native_owned_selections: native_owned_selections(project),
            transform_outputs: BTreeMap::new(),
            view_refs: BTreeMap::new(),
            legend_overlays: BTreeMap::new(),
            mark_channel_seed_stack: RefCell::new(BTreeSet::new()),
            analysis_schemas: Vec::new(),
            active_chart_id: None,
            active_chart_path: None,
        }
    }

    async fn lower_one(
        &mut self,
        entrypoint_id: &ChartEntrypointId,
    ) -> Result<LoweredChart, Diagnostic> {
        let entrypoint = self.project.entrypoints.get(entrypoint_id).ok_or_else(|| {
            diagnostic(
                SourceSpan::empty(avenger_lang_core::SourceId::new(0), 0),
                "AVENGER-LOWER-001",
                "resolved chart entrypoint is missing",
                format!("{entrypoint_id:?}"),
            )
        })?;
        let declaration =
            find_declaration(self.project, &entrypoint.declaration).ok_or_else(|| {
                diagnostic(
                    SourceSpan::empty(avenger_lang_core::SourceId::new(0), 0),
                    "AVENGER-LOWER-001",
                    "resolved chart declaration is missing",
                    entrypoint.declaration.to_string(),
                )
            })?;
        self.active_chart_path = declaration
            .public_path
            .clone()
            .or_else(|| declaration.name.clone());
        self.active_chart_id = Some(declaration.id.clone());
        let plot = self.lower_chart(declaration).await?;
        let compiled = self
            .registry
            .compile_root(&plot, self.context)
            .await
            .map_err(|error| lowerer_error(declaration, error.to_string()))?;
        let mut artifact = CompiledChartArtifact::new(
            entrypoint_id.clone(),
            declaration.name.clone(),
            declaration.source,
            Arc::new(compiled),
            NativeRequirementSet::builtin_only(self.registry),
            DependencyFingerprint::new(self.project.source_fingerprint.clone()),
        );
        self.enrich_interface(&mut artifact, declaration);
        self.active_chart_path = None;
        self.active_chart_id = None;
        Ok(LoweredChart { artifact })
    }

    fn lower_entrypoint_state(
        &mut self,
        entrypoint_id: &ChartEntrypointId,
    ) -> Result<(), Diagnostic> {
        let entrypoint = self.project.entrypoints.get(entrypoint_id).ok_or_else(|| {
            diagnostic(
                SourceSpan::empty(avenger_lang_core::SourceId::new(0), 0),
                "AVENGER-LOWER-001",
                "resolved chart entrypoint is missing",
                format!("{entrypoint_id:?}"),
            )
        })?;
        for id in &entrypoint.param_initializer_order {
            if !self.params.contains_key(id) {
                self.lower_param(id)?;
            }
        }
        // Native tool/widget exports have schema-provided defaults and do not
        // participate in the authored param-initializer DAG. They still need the
        // exact same typed parameter representation before their paired
        // lowerers run.
        for id in entrypoint.params.keys() {
            if !self.params.contains_key(id) {
                self.lower_param(id)?;
            }
        }
        for (id, store) in &entrypoint.stores {
            let lowered = self.lower_store(store)?;
            let table_name = format!("__avenger_store_binding_{}", id.as_str());
            let schema = Arc::new(Schema::new(
                lowered
                    .fields
                    .iter()
                    .map(avenger_chart_core::StoreFieldSpec::to_field_ref)
                    .collect::<Vec<_>>(),
            ));
            self.context
                .register_table(&table_name, Arc::new(EmptyTable::new(schema)))
                .map_err(|error| {
                    lowerer_error_at(
                        store.declaration.clone(),
                        self.project,
                        format!("failed to register store query placeholder: {error}"),
                    )
                })?;
            self.store_table_names.insert(id.clone(), table_name);
            self.stores.insert(id.clone(), lowered);
        }
        for (id, selection) in &entrypoint.selections {
            self.selections
                .insert(id.clone(), self.lower_selection(selection));
        }
        Ok(())
    }

    fn analyze_container_data<'b>(
        &'b mut self,
        container: &'b ResolvedDeclaration,
        inherited_data: Option<&'b DataFrame>,
        analysis: &'b mut Vec<ChartDatasetAnalysis>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Diagnostic>> + 'b>> {
        Box::pin(async move {
            if container.keyword == "view" {
                let spec = self.lower_view_spec(container, inherited_data)?;
                self.view_refs.insert(container.id.clone(), spec.view_ref());
            }
            let explicit_data = match container.properties.get("data") {
                Some(value) => {
                    if let Some((planning, _)) = self.store_data_binding(value, container)? {
                        Some(planning)
                    } else {
                        Some(self.lower_data(value, container).await?)
                    }
                }
                None => None,
            };
            let mut current_data = explicit_data.as_ref().or(inherited_data).cloned();
            let has_transforms = container
                .children
                .iter()
                .any(|child| child.keyword == "transform");
            if (explicit_data.is_some() || has_transforms)
                && let Some(data) = current_data.as_ref()
            {
                analysis.push(chart_analysis_record(
                    container,
                    container,
                    crate::DatasetStageKind::DatasetSource,
                    data,
                ));
            }

            for child in container
                .children
                .iter()
                .filter(|child| child.keyword == "transform")
            {
                let input = current_data.as_ref().ok_or_else(|| {
                    lowerer_error(child, "transform has no inherited or explicit data source")
                })?;
                self.analysis_schemas.clear();
                let (data, _) = self.lower_transform_stage(child, input, None).await?;
                let recorded = std::mem::take(&mut self.analysis_schemas);
                if recorded.is_empty() {
                    analysis.push(chart_analysis_record(
                        container,
                        child,
                        crate::DatasetStageKind::Transform {
                            native_kind: child.kind.clone().unwrap_or_else(|| "unknown".to_owned()),
                        },
                        &data,
                    ));
                } else {
                    for (stage_declaration, stage_span, schema) in recorded {
                        let stage =
                            find_declaration(self.project, &stage_declaration).unwrap_or(child);
                        analysis.push(ChartDatasetAnalysis {
                            dataset: container.id.clone(),
                            declaration_span: container.span,
                            stage_span,
                            stage_kind: crate::DatasetStageKind::Transform {
                                native_kind: stage
                                    .kind
                                    .clone()
                                    .unwrap_or_else(|| "unknown".to_owned()),
                            },
                            columns: schema
                                .fields()
                                .iter()
                                .map(|field| crate::AnalyzedColumn {
                                    name: field.name().clone(),
                                    qualifier: None,
                                    data_type: field.data_type().clone(),
                                    nullable: field.is_nullable(),
                                })
                                .collect(),
                            schema,
                            logical_plan_fingerprint: None,
                            mark_channels: Vec::new(),
                        });
                    }
                }
                current_data = Some(data);
            }

            for child in container.children.iter().filter(|child| {
                matches!(
                    child.keyword.as_str(),
                    "view" | "mark" | "cell" | "plot" | "layer"
                )
            }) {
                self.analyze_container_data(child, current_data.as_ref(), analysis)
                    .await?;
            }
            if container.keyword == "mark"
                && !is_resolved_mark_group(container)
                && let Some(data) = current_data.as_ref()
            {
                let mut record = chart_analysis_record(
                    container,
                    container,
                    crate::DatasetStageKind::MarkInput,
                    data,
                );
                record.mark_channels = self.analyze_mark_channels(container, data)?;
                analysis.push(record);
            }
            Ok(())
        })
    }

    fn analyze_mark_channels(
        &self,
        declaration: &ResolvedDeclaration,
        data: &DataFrame,
    ) -> Result<Vec<crate::AnalyzedMarkChannel>, Diagnostic> {
        let Some(kind) = declaration.kind.as_deref() else {
            return Ok(Vec::new());
        };
        let key = NativeKindKey::mark(declaration.coordinate.as_deref().unwrap_or(""), kind);
        let Some(schema) = self.registry.snapshot().entries.get(&key) else {
            return Ok(Vec::new());
        };
        schema
            .channels
            .values()
            .filter_map(|channel| {
                declaration
                    .properties
                    .get(&channel.name)
                    .map(|value| (channel, value))
            })
            .map(|(channel, value)| {
                let expression = match value {
                    ResolvedValue::Object { head, .. } => match head {
                        Some(head) => self.channel_data_expr(head, Some(data), declaration)?,
                        None => lit(1.0_f64),
                    },
                    _ => self.channel_data_expr(value, Some(data), declaration)?,
                };
                let data_type = expression
                    .get_type(data.schema())
                    .map_err(|error| lowerer_error(declaration, error.to_string()))?;
                let nullable = expression
                    .nullable(data.schema())
                    .map_err(|error| lowerer_error(declaration, error.to_string()))?;
                Ok(crate::AnalyzedMarkChannel {
                    name: channel.name.clone(),
                    data_type,
                    nullable,
                })
            })
            .collect()
    }

    fn enrich_interface(&self, artifact: &mut CompiledChartArtifact, chart: &ResolvedDeclaration) {
        for (id, param) in &self.project.params {
            if !belongs_to_chart(&param.owner_ancestry, &chart.id) {
                continue;
            }
            let runtime_name = &self.params[id].name;
            let migration_key = param.migration_key.as_ref().map(|key| {
                avenger_chart_core::StateMigrationKey::from_compiler_identity(key.as_str())
            });
            if let Some(spec) = Arc::make_mut(&mut artifact.compiled)
                .param_specs_mut()
                .get_mut(runtime_name)
            {
                spec.migration_key.clone_from(&migration_key);
            }
            if let Some(spec) = artifact.compiled.param_specs().get(runtime_name)
                && let Some(binding) = artifact.interface.params.get_mut(runtime_name)
            {
                binding.runtime_id = spec.runtime_id.as_opaque_str().to_string();
                binding.migration_key = migration_key;
            }
        }
        for (id, store) in &self.project.stores {
            if !belongs_to_chart(&store.owner_ancestry, &chart.id) {
                continue;
            }
            let runtime_name = &self.stores[id].name;
            let migration_key = store.migration_key.as_ref().map(|key| {
                avenger_chart_core::StateMigrationKey::from_compiler_identity(key.as_str())
            });
            if let Some(spec) = Arc::make_mut(&mut artifact.compiled)
                .store_specs_mut()
                .get_mut(runtime_name)
            {
                spec.migration_key.clone_from(&migration_key);
            }
            if let Some(spec) = artifact.compiled.store_specs().get(runtime_name)
                && let Some(binding) = artifact.interface.stores.get_mut(runtime_name)
            {
                binding.runtime_id = spec.runtime_id.as_opaque_str().to_string();
                binding.migration_key = migration_key;
            }
        }
        for selection in self.project.selections.values() {
            if !belongs_to_chart(&selection.owner_ancestry, &chart.id) {
                continue;
            }
            let migration_key = selection.migration_key.as_ref().map(|key| {
                avenger_chart_core::StateMigrationKey::from_compiler_identity(key.as_str())
            });
            if let Some(spec) = Arc::make_mut(&mut artifact.compiled)
                .selection_specs_mut()
                .get_mut(&selection.source_name)
            {
                spec.migration_key.clone_from(&migration_key);
            }
            if let Some(spec) = artifact
                .compiled
                .selection_specs()
                .get(&selection.source_name)
                && let Some(binding) = artifact
                    .interface
                    .selections
                    .get_mut(&selection.source_name)
            {
                binding.runtime_id = spec.runtime_id.as_opaque_str().to_string();
                binding.migration_key = migration_key;
            }
        }

        let chart_path = chart.public_path.as_deref().or(chart.name.as_deref());
        let compiled_mark_aliases = compiled_mark_aliases(&artifact.compiled);
        for (path, target) in &self.project.public_targets {
            if !public_path_belongs_to_chart(path, chart_path) {
                continue;
            }
            let runtime_id = match target {
                ResolvedTarget::Param(id) => self.params.get(id).and_then(|param| {
                    artifact
                        .compiled
                        .param_specs()
                        .get(&param.name)
                        .map(|spec| spec.runtime_id.as_opaque_str().to_string())
                }),
                ResolvedTarget::Store(id) => self.stores.get(id).and_then(|store| {
                    artifact
                        .compiled
                        .store_specs()
                        .get(&store.name)
                        .map(|spec| spec.runtime_id.as_opaque_str().to_string())
                }),
                ResolvedTarget::Selection(id) => {
                    self.project.selections.get(id).and_then(|value| {
                        artifact
                            .compiled
                            .selection_specs()
                            .get(&value.source_name)
                            .map(|spec| spec.runtime_id.as_opaque_str().to_string())
                    })
                }
                ResolvedTarget::Mark(_) => {
                    compiled_mark_id_for_public_path(&compiled_mark_aliases, path, chart_path)
                }
                ResolvedTarget::Widget(_) => find_declaration_by_target(self.project, target)
                    .and_then(|declaration| widget_source_id(declaration))
                    .and_then(|id| {
                        artifact
                            .compiled
                            .widgets()
                            .iter()
                            .find(|attachment| attachment.widget.id() == id)
                            .map(|attachment| attachment.instance_id.as_opaque_str().to_string())
                    }),
                ResolvedTarget::Part { declaration, .. } => {
                    find_declaration(self.project, declaration).and_then(|owner| {
                        if owner.keyword == "widget" {
                            compiled_widget_part_id(&artifact.compiled, path, chart_path)
                        } else {
                            compiled_mark_id_for_public_path(
                                &compiled_mark_aliases,
                                path,
                                chart_path,
                            )
                        }
                    })
                }
                _ => None,
            };
            if let Some(runtime_id) = runtime_id {
                artifact
                    .interface
                    .public_targets
                    .insert(path.clone(), runtime_id.clone());
                if let ResolvedTarget::Param(id) = target
                    && self.project.params[id]
                        .generated_by
                        .as_ref()
                        .is_some_and(|origin| {
                            find_declaration(self.project, &origin.declaration)
                                .is_some_and(|declaration| declaration.keyword == "widget")
                        })
                {
                    artifact
                        .interface
                        .widget_exports
                        .insert(path.clone(), runtime_id);
                }
            }
        }
    }

    fn active_chart_declaration(&self) -> Result<&ResolvedDeclaration, Diagnostic> {
        let id = self.active_chart_id.as_ref().ok_or_else(|| {
            diagnostic(
                SourceSpan::empty(avenger_lang_core::SourceId::new(0), 0),
                "AVENGER-LOWER-001",
                "tool behavior has no active chart",
                "canonical behavior lowering requires a containing chart",
            )
        })?;
        find_declaration(self.project, id).ok_or_else(|| {
            diagnostic(
                SourceSpan::empty(avenger_lang_core::SourceId::new(0), 0),
                "AVENGER-LOWER-001",
                "active chart declaration is missing",
                id.to_string(),
            )
        })
    }

    fn owned_by_tool_behavior(&self, ancestry: &[DeclarationId]) -> bool {
        ancestry.iter().any(|id| {
            find_declaration(self.project, id).is_some_and(|declaration| {
                declaration.keyword == "tool" && declaration.kind.as_deref() == Some("behavior")
            })
        })
    }

    fn lower_param(&mut self, id: &ParamId) -> Result<(), Diagnostic> {
        let param = &self.project.params[id];
        let default = self.evaluate_param_initializer(&param.initializer, param)?;
        let runtime_name = self.param_runtime_name(param);
        let mut lowered = Param::new(runtime_name, default);
        if let Some(key) = param.migration_key.as_ref() {
            lowered =
                lowered.migration_key(StateMigrationKey::from_compiler_identity(key.as_str()));
        }
        let provenance = match &param.type_contract {
            ParamTypeContract::Inferred => ParamTypeProvenance::Inferred,
            ParamTypeContract::SchemaFixed(_) => ParamTypeProvenance::SchemaFixed,
        };
        self.param_types
            .insert(id.clone(), lowered.default.data_type(), provenance);
        self.params.insert(id.clone(), lowered);
        Ok(())
    }

    fn param_runtime_name(&self, param: &ResolvedParam) -> String {
        let Some(origin) = &param.generated_by else {
            return param.source_name.clone();
        };
        let Some(declaration) = find_declaration(self.project, &origin.declaration) else {
            return param.source_name.clone();
        };
        if declaration.keyword == "widget"
            && let Some(id) = declaration.name.as_deref()
        {
            let role = match origin.export_role.as_str() {
                "cursor_position" => "cursor",
                role => role,
            };
            return format!("{id}__{role}");
        }
        if declaration.keyword == "tool"
            && let Some(id) = declaration.name.as_deref()
        {
            return format!("__tool_{id}__{}", origin.export_role);
        }
        param.source_name.clone()
    }

    fn lower_selection(&self, selection: &ResolvedSelection) -> Selection {
        let runtime_name = selection
            .generated_by
            .as_ref()
            .and_then(|origin| {
                let declaration = find_declaration(self.project, &origin.declaration)?;
                if declaration.keyword != "widget" {
                    return None;
                }
                let id = declaration.name.as_deref()?;
                Some(format!("{id}__{}", origin.export_role))
            })
            .unwrap_or_else(|| selection.source_name.clone());
        let mut lowered = Selection::new(runtime_name);
        if let Some(key) = selection.migration_key.as_ref() {
            lowered =
                lowered.migration_key(StateMigrationKey::from_compiler_identity(key.as_str()));
        }
        lowered = match selection.empty {
            ResolvedSelectionEmpty::All => lowered.empty_selects_all(),
            ResolvedSelectionEmpty::None => lowered.empty_selects_nothing(),
        };
        lowered.combine(match selection.combine {
            ResolvedSelectionCombine::Union => avenger_chart_core::SelectionCombine::Union,
            ResolvedSelectionCombine::Intersect => avenger_chart_core::SelectionCombine::Intersect,
        })
    }

    fn evaluate_param_initializer(
        &self,
        value: &ResolvedValue,
        param: &ResolvedParam,
    ) -> Result<ScalarValue, Diagnostic> {
        let declaration = find_declaration(self.project, &param.declaration).ok_or_else(|| {
            lowerer_error_at(
                param.declaration.clone(),
                self.project,
                "parameter declaration is unavailable",
            )
        })?;
        let data = self.constant_expression_data(declaration)?;
        let source = self.boundary_source_value(value, Some(&data), declaration, false)?;
        let (expr, target) = match &param.type_contract {
            ParamTypeContract::Inferred => {
                let target = source
                    .get_type(data.schema())
                    .map_err(|error| lowerer_error(declaration, error.to_string()))?;
                if target == DataType::Null {
                    return Err(Diagnostic::error(
                        "AVENGER-PARAM-001",
                        "parameter initializer has no concrete Arrow type",
                        SourceLabel::new(
                            declaration.span,
                            "cast `NULL` or an untyped empty value to the intended SQL/Arrow type",
                        ),
                    ));
                }
                (source, target)
            }
            ParamTypeContract::SchemaFixed(data_type) => {
                let target = physical_type_to_arrow(data_type);
                (cast(source, target.clone()), target)
            }
        };
        let value =
            self.evaluate_constant_boundary(expr, &target, declaration, "parameter initializer")?;
        if value.data_type() != target {
            return Err(lowerer_error(
                declaration,
                format!(
                    "parameter initializer evaluated as `{}`, expected planned type `{target}`",
                    value.data_type()
                ),
            ));
        }
        serde_json::to_vec(&SerializableScalar::new(value.clone())).map_err(|error| {
            Diagnostic::error(
                "AVENGER-PARAM-003",
                "parameter initializer produced an unsupported Arrow scalar",
                SourceLabel::new(
                    declaration.span,
                    format!(
                        "`{target}` cannot round-trip through compiled state serialization: {error}; choose a supported type or cast explicitly"
                    ),
                ),
            )
        })?;
        Ok(value)
    }

    fn lower_store(&self, store: &ResolvedStore) -> Result<Store, Diagnostic> {
        let runtime_name = self.store_runtime_name(store);
        let fields = store
            .fields
            .iter()
            .map(physical_field_to_arrow)
            .collect::<Vec<_>>();
        let schema = Arc::new(Schema::new(fields.clone()));
        let mut lowered = if store.rows.is_empty() {
            Store::new(runtime_name.clone(), schema)
        } else {
            let columns = fields
                .iter()
                .map(|field| {
                    let values = store
                        .rows
                        .iter()
                        .map(|row| {
                            let value = row.get(field.name()).unwrap_or(&ResolvedValue::Null);
                            let declaration = find_declaration(self.project, &store.declaration)
                                .ok_or_else(|| "store declaration is unavailable".to_owned())?;
                            let data = self
                                .constant_expression_data(declaration)
                                .map_err(|diagnostic| diagnostic.message.clone())?;
                            let expr = self
                                .typed_boundary_expr(
                                    value,
                                    field.data_type(),
                                    Some(&data),
                                    declaration,
                                    false,
                                )
                                .map_err(|diagnostic| diagnostic.message.clone())?;
                            self.evaluate_constant_boundary(
                                expr,
                                field.data_type(),
                                declaration,
                                &format!("store field `{}`", field.name()),
                            )
                            .map_err(|diagnostic| diagnostic.message.clone())
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    ScalarValue::iter_to_array(values).map_err(|error| error.to_string())
                })
                .collect::<Result<Vec<_>, String>>()
                .map_err(|error| {
                    lowerer_error_at(store.declaration.clone(), self.project, error)
                })?;
            let batch = RecordBatch::try_new(schema, columns).map_err(|error| {
                lowerer_error_at(store.declaration.clone(), self.project, error.to_string())
            })?;
            Store::from_record_batch(runtime_name, batch)
        };
        lowered = lowered
            .primary_key(store.primary_key.clone())
            .sharing(sharing(store.sharing));
        if let Some(key) = store.migration_key.as_ref() {
            lowered =
                lowered.migration_key(StateMigrationKey::from_compiler_identity(key.as_str()));
        }
        Ok(lowered)
    }

    fn store_runtime_name(&self, store: &ResolvedStore) -> String {
        let Some(origin) = &store.generated_by else {
            return store.source_name.clone();
        };
        let Some(declaration) = find_declaration(self.project, &origin.declaration) else {
            return store.source_name.clone();
        };
        if declaration.keyword == "tool"
            && let Some(id) = declaration.name.as_deref()
        {
            return format!("__tool_{id}__{}", origin.export_role);
        }
        store.source_name.clone()
    }

    fn constant_expression_data(
        &self,
        declaration: &ResolvedDeclaration,
    ) -> Result<DataFrame, Diagnostic> {
        let schema = Arc::new(Schema::empty());
        let batch = RecordBatch::try_new_with_options(
            Arc::clone(&schema),
            Vec::new(),
            &RecordBatchOptions::new().with_row_count(Some(1)),
        )
        .map_err(|error| lowerer_error(declaration, error.to_string()))?;
        self.context
            .read_batch(batch)
            .map_err(|error| lowerer_error(declaration, error.to_string()))
    }

    fn typed_boundary_expr(
        &self,
        value: &ResolvedValue,
        target: &DataType,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
        event: bool,
    ) -> Result<Expr, Diagnostic> {
        let source = match (value, target) {
            (ResolvedValue::Array(values), DataType::List(field))
            | (ResolvedValue::Array(values), DataType::LargeList(field)) => {
                let values = values
                    .iter()
                    .map(|value| {
                        self.typed_boundary_expr(value, field.data_type(), data, declaration, event)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                make_array(values)
            }
            (ResolvedValue::Array(values), DataType::FixedSizeList(field, length)) => {
                if usize::try_from(*length).ok() != Some(values.len()) {
                    return Err(lowerer_error(
                        declaration,
                        format!(
                            "fixed-size list boundary requires {length} values, found {}",
                            values.len()
                        ),
                    ));
                }
                let values = values
                    .iter()
                    .map(|value| {
                        self.typed_boundary_expr(value, field.data_type(), data, declaration, event)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                make_array(values)
            }
            (
                ResolvedValue::Object {
                    properties,
                    children,
                    ..
                },
                DataType::Struct(fields),
            ) => {
                if !children.is_empty() {
                    return Err(lowerer_error(
                        declaration,
                        "typed struct values cannot contain declarations",
                    ));
                }
                let unknown = properties
                    .keys()
                    .filter(|name| fields.find(name).is_none())
                    .cloned()
                    .collect::<Vec<_>>();
                if !unknown.is_empty() {
                    return Err(lowerer_error(
                        declaration,
                        format!("typed struct has unknown field(s): {}", unknown.join(", ")),
                    ));
                }
                let mut arguments = Vec::with_capacity(fields.len() * 2);
                for field in fields {
                    arguments.push(lit(field.name().clone()));
                    match properties.get(field.name()) {
                        Some(value) => arguments.push(self.typed_boundary_expr(
                            value,
                            field.data_type(),
                            data,
                            declaration,
                            event,
                        )?),
                        None if field.is_nullable() => {
                            arguments.push(cast(lit(ScalarValue::Null), field.data_type().clone()))
                        }
                        None => {
                            return Err(lowerer_error(
                                declaration,
                                format!(
                                    "typed struct is missing non-nullable field `{}`",
                                    field.name()
                                ),
                            ));
                        }
                    }
                }
                named_struct(arguments)
            }
            (
                ResolvedValue::Object {
                    properties,
                    children,
                    ..
                },
                DataType::Map(entries, _),
            ) => {
                if !children.is_empty() {
                    return Err(lowerer_error(
                        declaration,
                        "typed map values cannot contain declarations",
                    ));
                }
                let DataType::Struct(entry_fields) = entries.data_type() else {
                    return Err(lowerer_error(declaration, "invalid Arrow map entry type"));
                };
                let key_type = entry_fields[0].data_type();
                let value_type = entry_fields[1].data_type();
                let mut keys = Vec::with_capacity(properties.len());
                let mut values = Vec::with_capacity(properties.len());
                for (name, value) in properties {
                    keys.push(cast(lit(name.clone()), key_type.clone()));
                    values.push(self.typed_boundary_expr(
                        value,
                        value_type,
                        data,
                        declaration,
                        event,
                    )?);
                }
                let key_list =
                    DataType::List(Arc::new(Field::new_list_field(key_type.clone(), false)));
                let value_list =
                    DataType::List(Arc::new(Field::new_list_field(value_type.clone(), true)));
                map_udf().call(vec![
                    cast(make_array(keys), key_list),
                    cast(make_array(values), value_list),
                ])
            }
            (ResolvedValue::Array(_), _) => {
                return Err(lowerer_error(
                    declaration,
                    format!("array value cannot be cast to Arrow type {target}"),
                ));
            }
            (ResolvedValue::Object { .. }, _) => {
                return Err(lowerer_error(
                    declaration,
                    format!("object value cannot be cast to Arrow type {target}"),
                ));
            }
            _ => self.boundary_source_value(value, data, declaration, event)?,
        };
        Ok(cast(source, target.clone()))
    }

    fn evaluate_constant_boundary(
        &self,
        expr: Expr,
        target: &DataType,
        declaration: &ResolvedDeclaration,
        boundary: &str,
    ) -> Result<ScalarValue, Diagnostic> {
        let expr = expr
            .transform_up(|candidate| {
                let Expr::Placeholder(placeholder) = &candidate else {
                    return Ok(Transformed::no(candidate));
                };
                let default = self
                    .params
                    .values()
                    .find(|param| format!("${}", param.name) == placeholder.id)
                    .map(|param| param.default.clone())
                    .ok_or_else(|| {
                        datafusion::error::DataFusionError::Plan(format!(
                            "parameter initializer dependency `{}` was not lowered first",
                            placeholder.id
                        ))
                    })?;
                Ok(Transformed::yes(lit(default)))
            })
            .map(|transformed| transformed.data)
            .map_err(|error: datafusion::error::DataFusionError| {
                lowerer_error(declaration, error.to_string())
            })?;
        let schema = Arc::new(Schema::empty());
        let batch = RecordBatch::try_new_with_options(
            Arc::clone(&schema),
            Vec::new(),
            &RecordBatchOptions::new().with_row_count(Some(1)),
        )
        .map_err(|error| lowerer_error(declaration, error.to_string()))?;
        let program = CompiledScalarExpressionProgram::compile(
            self.context,
            schema,
            vec![
                PhysicalScalarExpressionSpec::new("__avenger_typed_boundary", expr)
                    .with_expected_type(target.clone()),
            ],
            PhysicalScalarProgramOptions::default(),
        )
        .map_err(|error| {
            lowerer_error(
                declaration,
                format!("{boundary} cannot be cast to `{target}`: {error}"),
            )
        })?;
        program.evaluate_value_at(0, &batch).map_err(|error| {
            lowerer_error(
                declaration,
                format!("{boundary} cannot be cast to `{target}`: {error}"),
            )
        })
    }

    async fn lower_chart(
        &mut self,
        chart: &ResolvedDeclaration,
    ) -> Result<ResolvedPlot, Diagnostic> {
        let coordinate = chart
            .kind
            .clone()
            .ok_or_else(|| lowerer_error(chart, "chart coordinate kind was not resolved"))?;
        let mut plot = ResolvedPlot::new(coordinate);
        let data = match chart.properties.get("data") {
            Some(value) => Some(self.lower_data(value, chart).await?),
            None => None,
        };
        plot.coordinate =
            self.native_declaration(chart, data.as_ref(), NativeKindNamespace::Coordinate)?;
        plot.data = data.clone();
        if let Some(data) = &data {
            self.record_schema(chart, data);
        }

        if let Some(value) = chart.properties.get("title") {
            plot.furnishings.title = Some(self.expression_value(value, data.as_ref(), chart)?);
        }
        if let Some(value) = chart.properties.get("subtitle") {
            plot.furnishings.subtitle = Some(self.expression_value(value, data.as_ref(), chart)?);
        }
        if let Some(value) = chart.properties.get("layout") {
            plot.furnishings.layout = Some(self.lower_layout(value, data.as_ref(), chart)?);
        }
        if let Some(value) = chart.properties.get("time") {
            plot.furnishings.time_context = self.lower_time_context(value, chart)?;
        }
        if let Some(value) = chart.properties.get("format") {
            plot.furnishings.formatting_context = self.lower_formatting_context(value, chart)?;
        }
        let mut theme = None;
        for declaration in chart
            .children
            .iter()
            .filter(|child| child.keyword == "theme")
        {
            let loaded_css;
            let css = if let Some(ResolvedValue::String(css)) = declaration.properties.get("css") {
                css.as_str()
            } else if let Some(ResolvedValue::String(path)) = declaration.properties.get("from") {
                let declaring_origin = &self
                    .project
                    .authored_source(declaration.span)
                    .ok_or_else(|| lowerer_error(declaration, "theme source file is unavailable"))?
                    .origin;
                let origin = resolve_relative_origin(
                    declaring_origin,
                    path,
                    &self.capabilities.project_root,
                )
                .map_err(|error| lowerer_error(declaration, error))?;
                loaded_css = self
                    .source_loader
                    .load(&origin, self.capabilities)
                    .await
                    .map_err(|error| lowerer_error(declaration, error.to_string()))?;
                if let Some(ResolvedValue::String(expected)) = declaration.properties.get("sha256")
                {
                    use sha2::{Digest, Sha256};
                    let actual = format!("{:x}", Sha256::digest(loaded_css.text.as_bytes()));
                    let expected = expected.strip_prefix("sha256:").unwrap_or(expected);
                    if !actual.eq_ignore_ascii_case(expected) {
                        return Err(lowerer_error(
                            declaration,
                            format!(
                                "theme CSS sha256 mismatch: expected {expected}, found {actual}"
                            ),
                        ));
                    }
                }
                loaded_css.text.as_ref()
            } else {
                return Err(lowerer_error(
                    declaration,
                    "theme css requires a string or `from` path",
                ));
            };
            match &mut theme {
                None => {
                    theme = Some(Theme::from_css(css).map_err(|error| {
                        lowerer_error(declaration, format!("invalid theme CSS: {error}"))
                    })?);
                }
                Some(theme) => theme.append_css(css).map_err(|error| {
                    lowerer_error(declaration, format!("invalid theme CSS: {error}"))
                })?,
            }
        }
        plot.furnishings.theme = theme;

        for (id, param) in &self.project.params {
            if param.table_owner.is_none()
                && belongs_to_chart(&param.owner_ancestry, &chart.id)
                && !self.widget_owned_params.contains(id)
                && !self.owned_by_tool_behavior(&param.owner_ancestry)
            {
                plot.furnishings
                    .params
                    .push((self.params[id].clone(), sharing(param.sharing)));
            }
        }
        for (id, store) in &self.project.stores {
            if belongs_to_chart(&store.owner_ancestry, &chart.id)
                && !self.native_owned_stores.contains(id)
                && !self.owned_by_tool_behavior(&store.owner_ancestry)
            {
                plot.furnishings.stores.push(self.stores[id].clone());
            }
        }
        for (id, selection) in &self.project.selections {
            if belongs_to_chart(&selection.owner_ancestry, &chart.id)
                && selection.generated_by.is_none()
                && !self.native_owned_selections.contains(id)
                && !self.owned_by_tool_behavior(&selection.owner_ancestry)
            {
                plot.furnishings
                    .selections
                    .push(self.selections[id].clone());
            }
        }
        for binding in chart.children.iter().filter(|child| child.keyword == "on") {
            plot.furnishings
                .event_bindings
                .push(self.lower_event_binding(chart, binding, data.as_ref())?);
        }

        let mut root_group = self
            .lower_container(chart, data.as_ref(), &mut plot)
            .await?;
        if root_group.data.is_none()
            && root_group.transforms.is_empty()
            && root_group.view.is_none()
            && root_group.id.is_none()
            && root_group.component_kind.is_none()
        {
            plot.marks.append(&mut root_group.marks);
        } else if !root_group.marks.is_empty() || !root_group.transforms.is_empty() {
            plot.marks.push(ResolvedMark::Group(Box::new(root_group)));
        }
        Ok(plot)
    }

    fn lower_time_context(
        &self,
        value: &ResolvedValue,
        declaration: &ResolvedDeclaration,
    ) -> Result<TimeContext, Diagnostic> {
        let ResolvedValue::Object { properties, .. } = value else {
            return Err(lowerer_error(declaration, "time must be a property block"));
        };
        let mut context = TimeContext::new();
        if let Some(value) = properties.get("timezone").and_then(resolved_string) {
            context = context.timezone(value);
        }
        if let Some(value) = properties.get("week_start").and_then(resolved_atom) {
            let week_start = match value {
                "sunday" => WeekStart::Sunday,
                "monday" => WeekStart::Monday,
                "tuesday" => WeekStart::Tuesday,
                "wednesday" => WeekStart::Wednesday,
                "thursday" => WeekStart::Thursday,
                "friday" => WeekStart::Friday,
                "saturday" => WeekStart::Saturday,
                other => {
                    return Err(lowerer_error(
                        declaration,
                        format!("unsupported week_start `{other}`"),
                    ));
                }
            };
            context = context.week_start(week_start);
        }
        Ok(context)
    }

    fn lower_formatting_context(
        &self,
        value: &ResolvedValue,
        declaration: &ResolvedDeclaration,
    ) -> Result<FormattingContext, Diagnostic> {
        let ResolvedValue::Object { properties, .. } = value else {
            return Err(lowerer_error(
                declaration,
                "format must be a property block",
            ));
        };
        let mut context = FormattingContext::new();
        if let Some(value) = properties.get("number_locale").and_then(resolved_string) {
            context = context.number_locale(value);
        }
        if let Some(value) = properties.get("datetime_locale").and_then(resolved_string) {
            context = context.datetime_locale(value);
        }
        if let Some(value) = properties
            .get("datetime_timezone")
            .and_then(resolved_string)
        {
            context = context.datetime_timezone(value);
        }
        Ok(context)
    }

    fn lower_event_binding(
        &self,
        chart: &ResolvedDeclaration,
        declaration: &ResolvedDeclaration,
        data: Option<&DataFrame>,
    ) -> Result<ChartEventBinding, Diagnostic> {
        let resolved = declaration
            .event_binding
            .as_ref()
            .ok_or_else(|| lowerer_error(declaration, "event binding metadata was not resolved"))?;
        let mut binding = ChartEventBinding::on(
            chart_event_type(&resolved.event_type)
                .ok_or_else(|| lowerer_error(declaration, "unsupported chart event type"))?,
        );
        if let Some(value) = declaration.properties.get("filter") {
            binding = binding.filter(self.event_expression_value(value, data, declaration)?);
        }
        if let Some(ResolvedValue::Number(value)) = declaration.properties.get("throttle_ms") {
            binding = binding.throttle_ms(value.parse::<u64>().map_err(|_| {
                lowerer_error(declaration, "throttle_ms must be a non-negative integer")
            })?);
        }
        if let Some(ResolvedValue::Boolean(consume)) = declaration.properties.get("consume") {
            binding = binding.consume(*consume);
        }
        if matches!(
            declaration.properties.get("mode"),
            Some(ResolvedValue::Atom(value)) if value == "preview"
        ) {
            binding = binding.preview();
        } else if matches!(
            declaration.properties.get("mode"),
            Some(ResolvedValue::Atom(value)) if value == "exact"
        ) {
            binding = binding.exact();
        }
        if matches!(
            declaration.properties.get("settle_exact"),
            Some(ResolvedValue::Boolean(true))
        ) {
            binding = binding.settle_exact();
        }

        let mark_paths = resolved
            .targets
            .iter()
            .map(|target| self.event_target_path(chart, target, declaration))
            .collect::<Result<Vec<_>, _>>()?;
        if !mark_paths.is_empty() {
            binding = binding.marks(mark_paths);
        }
        if let ResolvedEventScope::Subplots { targets, .. } = &resolved.scope {
            let paths = targets
                .iter()
                .map(|target| self.event_target_path(chart, target, declaration))
                .collect::<Result<Vec<_>, _>>()?;
            binding = binding.within_subplots(paths);
        }
        binding = match &resolved.surface {
            ResolvedEventSurface::All(_) => binding.all_surfaces(),
            ResolvedEventSurface::Plot(_) => binding.with_plot_surface_target(),
            ResolvedEventSurface::Legend { channel, .. } => {
                binding.with_legend_surface_target(vec![channel.clone()], Vec::new())
            }
        };

        if let Some(value) = declaration.properties.get("between") {
            let ResolvedValue::Object { properties, .. } = value else {
                return Err(lowerer_error(declaration, "between must be a stream block"));
            };
            let start = properties
                .get("start")
                .ok_or_else(|| lowerer_error(declaration, "between requires a start stream"))?;
            let end = properties
                .get("end")
                .ok_or_else(|| lowerer_error(declaration, "between requires an end stream"))?;
            binding = binding.between(
                self.lower_event_stream(chart, start, data, declaration)?,
                self.lower_event_stream(chart, end, data, declaration)?,
            );
        }

        for action in &declaration.children {
            if !is_state_action_keyword(&action.keyword) {
                continue;
            }
            let lowered = self.lower_state_action(chart, data, action, true)?;
            binding = match lowered {
                LoweredStateAction::Cursor(value) => binding.set_cursor(value),
                LoweredStateAction::Param { param, value } => {
                    let lvalue = action.state_lvalue.as_ref().ok_or_else(|| {
                        lowerer_error(action, "parameter action target was not resolved")
                    })?;
                    match (lvalue.route, lvalue.replacing_scopes) {
                        (ResolvedActionRoute::Current, false) => binding.set_param(&param, value),
                        (ResolvedActionRoute::Current, true) => {
                            binding.set_param_replacing_scopes(&param, value)
                        }
                        (ResolvedActionRoute::Start, false) => {
                            binding.set_param_at_start_scope(&param, value)
                        }
                        (ResolvedActionRoute::Start, true) => {
                            binding.set_param_at_start_scope_replacing_scopes(&param, value)
                        }
                    }
                }
                LoweredStateAction::Store { name, update } => {
                    let lvalue = action.state_lvalue.as_ref().ok_or_else(|| {
                        lowerer_error(action, "store action target was not resolved")
                    })?;
                    match (lvalue.route, lvalue.replacing_scopes) {
                        (ResolvedActionRoute::Current, false) => binding.set_store(name, update),
                        (ResolvedActionRoute::Current, true) => {
                            binding.set_store_replacing_scopes(name, update)
                        }
                        (ResolvedActionRoute::Start, false) => {
                            binding.set_store_at_start_scope(name, update)
                        }
                        (ResolvedActionRoute::Start, true) => {
                            binding.set_store_at_start_scope_replacing_scopes(name, update)
                        }
                    }
                }
                LoweredStateAction::Selection { id, update } => {
                    let lvalue = action.state_lvalue.as_ref().ok_or_else(|| {
                        lowerer_error(action, "selection action target was not resolved")
                    })?;
                    match lvalue.route {
                        ResolvedActionRoute::Current => binding.set_selection(id, *update),
                        ResolvedActionRoute::Start => {
                            binding.set_selection_at_start_scope(id, *update)
                        }
                    }
                }
            }
        }
        binding
            .validate()
            .map_err(|error| lowerer_error(declaration, error.to_string()))?;
        Ok(binding)
    }

    fn lower_state_action(
        &self,
        chart: &ResolvedDeclaration,
        data: Option<&DataFrame>,
        action: &ResolvedDeclaration,
        event_scalar_values: bool,
    ) -> Result<LoweredStateAction, Diagnostic> {
        match action.kind.as_deref() {
            Some("cursor") => {
                let value = action
                    .properties
                    .get("value")
                    .ok_or_else(|| lowerer_error(action, "cursor action requires a value"))?;
                Ok(LoweredStateAction::Cursor(self.boundary_source_value(
                    value,
                    data,
                    action,
                    event_scalar_values,
                )?))
            }
            Some("param") => {
                let value = action
                    .properties
                    .get("value")
                    .ok_or_else(|| lowerer_error(action, "param action requires a value"))?;
                let lvalue = action.state_lvalue.as_ref().ok_or_else(|| {
                    lowerer_error(action, "parameter action target was not resolved")
                })?;
                let ResolvedTarget::Param(id) = &lvalue.target else {
                    return Err(lowerer_error(
                        action,
                        "definition-owned parameter actions require Phase 7 expansion",
                    ));
                };
                let param = self
                    .params
                    .get(id)
                    .ok_or_else(|| lowerer_error(action, "resolved parameter is unavailable"))?;
                Ok(LoweredStateAction::Param {
                    param: param.clone(),
                    value: self.boundary_source_value(value, data, action, event_scalar_values)?,
                })
            }
            Some("store") => {
                let lvalue = action
                    .state_lvalue
                    .as_ref()
                    .ok_or_else(|| lowerer_error(action, "store action target was not resolved"))?;
                let ResolvedTarget::Store(id) = &lvalue.target else {
                    return Err(lowerer_error(
                        action,
                        "definition-owned store actions require Phase 7 expansion",
                    ));
                };
                let store = self
                    .stores
                    .get(id)
                    .ok_or_else(|| lowerer_error(action, "resolved store is unavailable"))?;
                Ok(LoweredStateAction::Store {
                    name: store.name.clone(),
                    update: self.lower_store_update(data, action)?,
                })
            }
            Some("selection") => {
                let lvalue = action.state_lvalue.as_ref().ok_or_else(|| {
                    lowerer_error(action, "selection action target was not resolved")
                })?;
                let ResolvedTarget::Selection(id) = &lvalue.target else {
                    return Err(lowerer_error(
                        action,
                        "definition-owned selection actions require Phase 7 expansion",
                    ));
                };
                let selection = self
                    .selections
                    .get(id)
                    .ok_or_else(|| lowerer_error(action, "resolved selection is unavailable"))?;
                Ok(LoweredStateAction::Selection {
                    id: selection.id.clone(),
                    update: Box::new(self.lower_selection_update(chart, data, action)?),
                })
            }
            _ => Err(lowerer_error(action, "unsupported state action kind")),
        }
    }

    fn lower_state_action_block(
        &self,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<ChartAction, Diagnostic> {
        let ResolvedValue::Object {
            head: None,
            kind: None,
            properties,
            children,
        } = value
        else {
            return Err(lowerer_error(
                declaration,
                "state action must be an unheaded action block",
            ));
        };
        if !properties.is_empty() {
            return Err(lowerer_error(
                declaration,
                "state action blocks contain only ordered action declarations",
            ));
        }

        let chart_id = self
            .active_chart_id
            .as_ref()
            .ok_or_else(|| lowerer_error(declaration, "state action has no active chart"))?;
        let chart = find_declaration(self.project, chart_id)
            .ok_or_else(|| lowerer_error(declaration, "active chart declaration is unavailable"))?;
        let mut lowered = ChartAction::new();
        for action in children {
            if !is_state_action_keyword(&action.keyword) {
                return Err(lowerer_error(
                    action,
                    "state action blocks contain only action declarations",
                ));
            }
            let lvalue = action
                .state_lvalue
                .as_ref()
                .ok_or_else(|| lowerer_error(action, "state action target was not resolved"))?;
            if lvalue.route != ResolvedActionRoute::Current || lvalue.replacing_scopes {
                return Err(lowerer_error(
                    action,
                    "widget state actions cannot use event-only state routing",
                ));
            }
            lowered = match self.lower_state_action(chart, data, action, false)? {
                LoweredStateAction::Param { param, value } => lowered.set_param(&param, value),
                LoweredStateAction::Store { name, update } => lowered.set_store(name, update),
                LoweredStateAction::Selection { id, update } => lowered.set_selection(id, *update),
                LoweredStateAction::Cursor(_) => {
                    return Err(lowerer_error(
                        action,
                        "widget state actions cannot publish a cursor",
                    ));
                }
            };
        }
        lowered
            .validate()
            .map_err(|error| lowerer_error(declaration, error.to_string()))?;
        Ok(lowered)
    }

    fn lower_store_update(
        &self,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<StoreUpdate, Diagnostic> {
        if declaration.keyword == "clear" {
            return Ok(StoreUpdate::clear());
        }
        let rows = || {
            declaration
                .children
                .iter()
                .filter(|child| child.keyword == "row")
                .map(|row| {
                    row.properties
                        .iter()
                        .try_fold(StoreRow::new(), |row, (name, value)| {
                            Ok(row.field(
                                name,
                                self.boundary_source_value(value, data, declaration, true)?,
                            ))
                        })
                })
                .collect::<Result<Vec<_>, Diagnostic>>()
        };
        Ok(match declaration.keyword.as_str() {
            "replace" => StoreUpdate::replace_rows(rows()?),
            "insert" => StoreUpdate::insert_rows(rows()?),
            "upsert" => StoreUpdate::upsert_rows(rows()?),
            "toggle" => StoreUpdate::toggle_rows(rows()?),
            "patch" => {
                let key = declaration
                    .children
                    .iter()
                    .find(|child| child.keyword == "key")
                    .ok_or_else(|| lowerer_error(declaration, "patch requires a key payload"))?;
                let fields = declaration
                    .children
                    .iter()
                    .find(|child| child.keyword == "fields")
                    .ok_or_else(|| lowerer_error(declaration, "patch requires a fields payload"))?;
                StoreUpdate::update_by_key(
                    self.lower_store_key(key, data, declaration)?,
                    fields.properties.iter().try_fold(
                        StoreFieldPatch::new(),
                        |patch, (name, value)| {
                            Ok(patch.field(
                                name,
                                self.boundary_source_value(value, data, declaration, true)?,
                            ))
                        },
                    )?,
                )
            }
            "delete" => {
                let key = declaration
                    .children
                    .iter()
                    .find(|child| child.keyword == "key")
                    .ok_or_else(|| lowerer_error(declaration, "delete requires a key payload"))?;
                StoreUpdate::delete_by_key(self.lower_store_key(key, data, declaration)?)
            }
            _ => {
                return Err(lowerer_error(
                    declaration,
                    format!("unsupported store update `{}`", declaration.keyword),
                ));
            }
        })
    }

    fn lower_store_key(
        &self,
        key: &ResolvedDeclaration,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<StoreKey, Diagnostic> {
        key.properties
            .iter()
            .try_fold(StoreKey::new(), |key, (name, value)| {
                Ok(key.field(
                    name,
                    self.boundary_source_value(value, data, declaration, true)?,
                ))
            })
    }

    fn lower_selection_update(
        &self,
        chart: &ResolvedDeclaration,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<SelectionUpdate, Diagnostic> {
        let within = declaration.properties.get("within");
        let from_scene = declaration.properties.contains_key("from_scene");
        match declaration.keyword.as_str() {
            "clear" => Ok(if let Some(scope) = within {
                SelectionUpdate::clear_in_scope(self.coordination_scope(scope, declaration)?)
            } else {
                SelectionUpdate::clear()
            }),
            "replace" | "upsert" | "toggle" if from_scene => {
                let mut query = self.lower_selection_scene_query(
                    chart,
                    &declaration.properties,
                    data,
                    declaration,
                )?;
                if let Some(scope) = within {
                    query = query.sharing(self.coordination_scope(scope, declaration)?);
                }
                Ok(match (declaration.keyword.as_str(), within.is_some()) {
                    ("replace", false) => SelectionUpdate::replace_all_from_scene_query(query),
                    ("replace", true) => SelectionUpdate::replace_from_scene_query_in_scope(query),
                    ("upsert", false) => SelectionUpdate::upsert_from_scene_query(query),
                    ("toggle", false) => SelectionUpdate::toggle_from_scene_query(query),
                    _ => {
                        return Err(lowerer_error(
                            declaration,
                            "unsupported scoped scene-query selection operation",
                        ));
                    }
                })
            }
            "replace" | "upsert" | "toggle" => {
                let clauses = declaration
                    .children
                    .iter()
                    .filter(|child| child.keyword == "clause")
                    .map(|clause| self.lower_selection_clause(clause, data, declaration))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(match (declaration.keyword.as_str(), within) {
                    ("replace", None) => SelectionUpdate::replace_all_clauses(clauses),
                    ("replace", Some(scope)) => SelectionUpdate::replace_clauses_in_scope(
                        self.coordination_scope(scope, declaration)?,
                        clauses,
                    ),
                    ("upsert", None) => SelectionUpdate::upsert_clauses(clauses),
                    ("toggle", None) => SelectionUpdate::toggle_clauses(clauses),
                    _ => {
                        return Err(lowerer_error(
                            declaration,
                            "unsupported scoped selection clause operation",
                        ));
                    }
                })
            }
            "delete" => {
                let ResolvedValue::Array(values) =
                    declaration.properties.get("ids").ok_or_else(|| {
                        lowerer_error(declaration, "selection clause deletion requires ids")
                    })?
                else {
                    return Err(lowerer_error(
                        declaration,
                        "selection clause deletion ids must be an array",
                    ));
                };
                let ids = values
                    .iter()
                    .map(|value| self.event_expression_value(value, data, declaration))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(if let Some(scope) = within {
                    SelectionUpdate::delete_clauses_in_scope(
                        self.coordination_scope(scope, declaration)?,
                        ids,
                    )
                } else {
                    SelectionUpdate::delete_clauses(ids)
                })
            }
            _ => Err(lowerer_error(
                declaration,
                format!("unsupported selection update `{}`", declaration.keyword),
            )),
        }
    }

    fn lower_selection_clause(
        &self,
        clause: &ResolvedDeclaration,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<SelectionClauseUpdate, Diagnostic> {
        let id = clause
            .properties
            .get("id")
            .ok_or_else(|| lowerer_error(declaration, "selection clause requires id"))?;
        let id = self.event_expression_value(id, data, declaration)?;
        let predicate = clause
            .children
            .iter()
            .find(|child| matches!(child.keyword.as_str(), "equality" | "interval"))
            .ok_or_else(|| {
                lowerer_error(
                    declaration,
                    "selection clause requires equality or interval predicate",
                )
            })?;

        let mut update = if predicate.keyword == "equality" {
            let mut builder = SelectionClauseUpdate::equality(id);
            for dimension in predicate
                .children
                .iter()
                .filter(|child| child.keyword == "dimension")
            {
                let name = dimension.name.as_deref().ok_or_else(|| {
                    lowerer_error(declaration, "selection dimension requires a name")
                })?;
                let field = dimension.properties.get("field").ok_or_else(|| {
                    lowerer_error(declaration, "equality dimension requires field")
                })?;
                let value = dimension.properties.get("value").ok_or_else(|| {
                    lowerer_error(declaration, "equality dimension requires value")
                })?;
                builder = builder.dimension_named(
                    name,
                    self.selection_field_expr(field, data, declaration)?,
                    self.event_expression_value(value, data, declaration)?,
                );
            }
            builder.build()
        } else {
            let mut builder = SelectionClauseUpdate::interval(id);
            for dimension in predicate
                .children
                .iter()
                .filter(|child| child.keyword == "dimension")
            {
                let name = dimension.name.as_deref().ok_or_else(|| {
                    lowerer_error(declaration, "selection dimension requires a name")
                })?;
                let field = dimension.properties.get("field").ok_or_else(|| {
                    lowerer_error(declaration, "interval dimension requires field")
                })?;
                let from = dimension.properties.get("from").ok_or_else(|| {
                    lowerer_error(declaration, "interval dimension requires from")
                })?;
                let to = dimension
                    .properties
                    .get("to")
                    .ok_or_else(|| lowerer_error(declaration, "interval dimension requires to"))?;
                builder = builder
                    .dimension_named(name, self.selection_field_expr(field, data, declaration)?)
                    .endpoints(
                        self.event_expression_value(from, data, declaration)?,
                        self.event_expression_value(to, data, declaration)?,
                    );
            }
            builder.build()
        };
        if let Some(scope) = clause.properties.get("scope") {
            update = update.facet_scope(self.coordination_scope(scope, declaration)?);
        }
        Ok(update)
    }

    fn selection_field_expr(
        &self,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<Expr, Diagnostic> {
        match value {
            ResolvedValue::String(name)
            | ResolvedValue::Atom(name)
            | ResolvedValue::Column(name) => {
                let data = data.ok_or_else(|| {
                    lowerer_error(
                        declaration,
                        format!("selection field `{name}` has no data schema in scope"),
                    )
                })?;
                data.schema()
                    .field_with_unqualified_name(name)
                    .map_err(|error| lowerer_error(declaration, error.to_string()))?;
                Ok(unqualified_column(name))
            }
            ResolvedValue::Expression(expression) => {
                self.planned_event_expression(expression, data, declaration)
            }
            _ => Err(lowerer_error(
                declaration,
                "selection field must be a column name or SQL expression",
            )),
        }
    }

    fn lower_selection_scene_query(
        &self,
        chart: &ResolvedDeclaration,
        properties: &BTreeMap<String, ResolvedValue>,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<SelectionSceneQuery, Diagnostic> {
        let geometry = properties
            .get("geometry")
            .ok_or_else(|| lowerer_error(declaration, "scene query requires geometry"))?;
        let ResolvedValue::Call { function, args } = geometry else {
            return Err(lowerer_error(
                declaration,
                "scene query geometry must be polygon(...), rect(...), or circle(...)",
            ));
        };
        let mut query = match (function.as_str(), args.as_slice()) {
            ("polygon", [points]) => SceneGeometryQuery::polygon(self.event_expression_value(
                points,
                data,
                declaration,
            )?),
            ("rect", [x0, y0, x1, y1]) => SceneGeometryQuery::rect(
                self.event_expression_value(x0, data, declaration)?,
                self.event_expression_value(y0, data, declaration)?,
                self.event_expression_value(x1, data, declaration)?,
                self.event_expression_value(y1, data, declaration)?,
            ),
            ("circle", [cx, cy, radius]) => SceneGeometryQuery::circle(
                self.event_expression_value(cx, data, declaration)?,
                self.event_expression_value(cy, data, declaration)?,
                self.event_expression_value(radius, data, declaration)?,
            ),
            _ => {
                return Err(lowerer_error(
                    declaration,
                    "scene query geometry has the wrong function or arity",
                ));
            }
        };
        let policy = match properties.get("policy") {
            Some(ResolvedValue::Atom(value)) | Some(ResolvedValue::String(value)) => {
                match value.as_str() {
                    "intersects" | "geometry_intersects" => {
                        SceneGeometryHitPolicy::GeometryIntersects
                    }
                    "envelope_intersects" => SceneGeometryHitPolicy::EnvelopeIntersects,
                    "contained" | "geometry_contained" => SceneGeometryHitPolicy::GeometryContained,
                    "anchor_inside" => SceneGeometryHitPolicy::AnchorInside,
                    "centroid_inside" => SceneGeometryHitPolicy::CentroidInside,
                    _ => {
                        return Err(lowerer_error(
                            declaration,
                            format!("unsupported scene-query hit policy `{value}`"),
                        ));
                    }
                }
            }
            _ => {
                return Err(lowerer_error(
                    declaration,
                    "scene query requires a hit policy",
                ));
            }
        };
        query = query.hit_policy(policy);

        let ResolvedValue::Array(marks) = properties
            .get("marks")
            .ok_or_else(|| lowerer_error(declaration, "scene query requires mark targets"))?
        else {
            return Err(lowerer_error(
                declaration,
                "scene query marks must be an array",
            ));
        };
        let marks = marks
            .iter()
            .map(|value| {
                let ResolvedValue::Reference(reference) = value else {
                    return Err(lowerer_error(
                        declaration,
                        "scene query mark target was not resolved",
                    ));
                };
                self.event_target_path(chart, &reference.target, declaration)
            })
            .collect::<Result<Vec<_>, _>>()?;
        query = query.marks(marks);

        if let Some(ResolvedValue::Array(fields)) = properties.get("fields") {
            for field in fields {
                let ResolvedValue::Object { properties, .. } = field else {
                    return Err(lowerer_error(
                        declaration,
                        "scene-query fields must be objects",
                    ));
                };
                let id = resolved_text(properties.get("id"), declaration, "scene-query field id")?;
                let datum = properties
                    .get("datum")
                    .map(|value| resolved_text(Some(value), declaration, "scene-query datum field"))
                    .transpose()?
                    .unwrap_or_else(|| id.clone());
                let field_expr = properties.get("field").ok_or_else(|| {
                    lowerer_error(declaration, "scene-query field requires field")
                })?;
                query = query.datum_field(
                    SceneQueryDatumField::new(id)
                        .datum(datum)
                        .field_expr(self.selection_field_expr(field_expr, data, declaration)?),
                );
            }
        }
        if let Some(ResolvedValue::Array(values)) = properties.get("unique_by") {
            query = query.unique_by(
                values
                    .iter()
                    .map(|value| {
                        resolved_text(Some(value), declaration, "scene-query unique field")
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            );
        }
        if let Some(ResolvedValue::Number(value)) = properties.get("max_hits") {
            query = query.max_hits(value.parse::<usize>().map_err(|_| {
                lowerer_error(
                    declaration,
                    "scene-query max_hits must be non-negative integer",
                )
            })?);
        }

        let mut query = SelectionSceneQuery::new(query);
        if let Some(scope) = properties.get("sharing") {
            query = query.sharing(self.coordination_scope(scope, declaration)?);
        }
        if let Some(clause_id) = properties.get("clause_id") {
            let clause_id = match clause_id {
                ResolvedValue::Atom(value) if value == "tuple" => SceneQueryClauseId::Tuple,
                ResolvedValue::Call { function, args }
                    if function == "field" && args.len() == 1 =>
                {
                    SceneQueryClauseId::field(resolved_text(
                        args.first(),
                        declaration,
                        "scene-query clause-id field",
                    )?)
                }
                value => SceneQueryClauseId::expr(self.event_expression_value(
                    value,
                    data,
                    declaration,
                )?),
            };
            query = query.clause_id(clause_id);
        }
        Ok(query)
    }

    fn lower_event_stream(
        &self,
        chart: &ResolvedDeclaration,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<ChartEventStream, Diagnostic> {
        let ResolvedValue::Object {
            kind, properties, ..
        } = value
        else {
            return Err(lowerer_error(
                declaration,
                "between stream must be an event block",
            ));
        };
        let kind = kind
            .as_deref()
            .ok_or_else(|| lowerer_error(declaration, "event stream type was not resolved"))?;
        let mut stream = ChartEventStream::on(
            chart_event_type(kind)
                .ok_or_else(|| lowerer_error(declaration, "unsupported event stream type"))?,
        );
        if let Some(value) = properties.get("filter") {
            stream = stream.filter(self.event_expression_value(value, data, declaration)?);
        }
        if let Some(target) = properties.get("target") {
            let targets = match target {
                ResolvedValue::Reference(reference) => vec![reference.target.clone()],
                ResolvedValue::Array(values) => values
                    .iter()
                    .filter_map(|value| match value {
                        ResolvedValue::Reference(reference) => Some(reference.target.clone()),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            let paths = targets
                .iter()
                .map(|target| self.event_target_path(chart, target, declaration))
                .collect::<Result<Vec<_>, _>>()?;
            if !paths.is_empty() {
                stream = stream.marks(paths);
            }
        }
        Ok(stream)
    }

    fn event_target_path(
        &self,
        chart: &ResolvedDeclaration,
        target: &ResolvedTarget,
        declaration: &ResolvedDeclaration,
    ) -> Result<String, Diagnostic> {
        let chart_path = chart.public_path.as_deref().or(chart.name.as_deref());
        if let Some(path) = self
            .project
            .source_modules
            .values()
            .flat_map(|file| file.roots.iter())
            .flat_map(declarations_depth_first)
            .filter_map(|owner| {
                let owner_path = owner.public_path.as_deref()?;
                owner
                    .exports
                    .iter()
                    .find_map(|(alias, candidate)| (candidate == target).then_some(alias))
                    .map(|alias| format!("{owner_path}.{alias}"))
            })
            .filter(|path| public_path_belongs_to_chart(path, chart_path))
            .map(|path| relative_public_path(&path, chart_path).to_string())
            .min()
        {
            return Ok(path);
        }
        if let Some(path) = self
            .project
            .public_targets
            .iter()
            .find_map(|(path, candidate)| {
                (candidate == target && public_path_belongs_to_chart(path, chart_path))
                    .then(|| relative_public_path(path, chart_path).to_string())
            })
        {
            return Ok(path);
        }

        fn find(
            owner: &ResolvedDeclaration,
            target: &ResolvedTarget,
            prefix: &[String],
        ) -> Option<String> {
            let mut path = prefix.to_vec();
            if owner.keyword != "chart"
                && let Some(name) = &owner.name
            {
                path.push(name.clone());
            }
            if owner.runtime_target.as_ref() == Some(target) {
                return Some(path.join("."));
            }
            if let ResolvedTarget::Part { declaration, alias } = target
                && &owner.id == declaration
            {
                path.push(alias.clone());
                return Some(path.join("."));
            }
            owner
                .children
                .iter()
                .find_map(|child| find(child, target, &path))
        }
        find(chart, target, &[])
            .ok_or_else(|| lowerer_error(declaration, "event target has no runtime authoring path"))
    }

    fn lower_tool_behavior<'b>(
        &'b mut self,
        chart: &'b ResolvedDeclaration,
        declaration: &'b ResolvedDeclaration,
        inherited_data: Option<&'b DataFrame>,
    ) -> ToolBehaviorFuture<'b> {
        Box::pin(async move {
            let id = declaration.name.clone().ok_or_else(|| {
                lowerer_error(declaration, "tool behavior requires an instance binder")
            })?;
            let component_kind = declaration
                .component_kind
                .clone()
                .unwrap_or_else(|| id.clone());
            let coordinate = declaration
                .coordinate
                .clone()
                .or_else(|| chart.kind.clone())
                .unwrap_or_else(|| "cartesian".to_string());
            let mut nested = ResolvedPlot::new(coordinate);
            let mut chrome_root = self
                .lower_container(declaration, inherited_data, &mut nested)
                .await?;
            let mut chrome = Vec::new();
            if chrome_root.data.is_none()
                && chrome_root.store_data.is_none()
                && chrome_root.transforms.is_empty()
                && chrome_root.view.is_none()
                && chrome_root.id.is_none()
                && chrome_root.component_kind.is_none()
            {
                chrome.append(&mut chrome_root.marks);
            } else if !chrome_root.marks.is_empty() || !chrome_root.transforms.is_empty() {
                chrome.push(ResolvedMark::Group(Box::new(chrome_root)));
            }

            let mut behavior = ResolvedToolBehavior::new(id.clone(), component_kind);
            behavior.component_id = Some(id);
            behavior.chrome = chrome;
            behavior.native_tools = nested.tools;
            behavior.nested_behaviors = nested.tool_behaviors;

            for (param_id, param) in &self.project.params {
                if param.owner_ancestry.last() == Some(&declaration.id)
                    && param.table_owner.is_none()
                    && param.generated_by.is_none()
                {
                    behavior.state.push(ResolvedBehaviorState::Param {
                        key: param_id.as_str().to_string(),
                        param: self.params[param_id].clone(),
                        sharing: avenger_chart_core::ToolParamSharing::explicit(sharing(
                            param.sharing,
                        )),
                    });
                }
            }
            for (store_id, store) in &self.project.stores {
                if store.owner_ancestry.last() == Some(&declaration.id)
                    && store.generated_by.is_none()
                {
                    behavior.state.push(ResolvedBehaviorState::Store {
                        key: store_id.as_str().to_string(),
                        store: self.stores[store_id].clone(),
                    });
                }
            }
            for (selection_id, selection) in &self.project.selections {
                if selection.owner_ancestry.last() == Some(&declaration.id)
                    && selection.generated_by.is_none()
                    && !self.native_owned_selections.contains(selection_id)
                {
                    behavior.state.push(ResolvedBehaviorState::Selection {
                        key: selection_id.as_str().to_string(),
                        selection: self.selections[selection_id].clone(),
                    });
                }
            }

            for (alias, target) in &declaration.exports {
                let target = match target {
                    ResolvedTarget::Param(id) => {
                        Some(ResolvedBehaviorExportTarget::Param(id.as_str().to_string()))
                    }
                    ResolvedTarget::Store(id) => {
                        Some(ResolvedBehaviorExportTarget::Store(id.as_str().to_string()))
                    }
                    ResolvedTarget::Selection(id) => Some(ResolvedBehaviorExportTarget::Selection(
                        id.as_str().to_string(),
                    )),
                    _ => None,
                };
                if let Some(target) = target {
                    behavior.exports.push(ResolvedBehaviorExport {
                        alias: alias.clone(),
                        target,
                    });
                }
            }

            for child in &declaration.children {
                match child.keyword.as_str() {
                    "on" => behavior.event_bindings.push(self.lower_event_binding(
                        chart,
                        child,
                        inherited_data,
                    )?),
                    "scale_edit" => behavior
                        .scale_edits
                        .push(self.lower_tool_scale_edit(child)?),
                    _ => {}
                }
            }
            Ok(behavior)
        })
    }

    fn lower_tool_scale_edit(
        &self,
        declaration: &ResolvedDeclaration,
    ) -> Result<avenger_chart_core::ToolScaleEdit, Diagnostic> {
        let channel = declaration
            .properties
            .get("channel")
            .and_then(resolved_atom)
            .ok_or_else(|| lowerer_error(declaration, "scale_edit requires `channel:`"))?;
        if let Some(target) = declaration.properties.get("target")
            && !matches!(target, ResolvedValue::Atom(value) if value == "plot")
        {
            return Err(lowerer_error(
                declaration,
                "scale_edit target must be the containing plot",
            ));
        }
        let ResolvedValue::Binding(ResolvedBinding {
            target: ResolvedTarget::Param(param_id),
            time: BindingTime::Current,
            ..
        }) = declaration
            .properties
            .get("raw_domain")
            .ok_or_else(|| lowerer_error(declaration, "scale_edit requires `raw_domain:`"))?
        else {
            return Err(lowerer_error(
                declaration,
                "scale_edit raw_domain must read a current param binding",
            ));
        };
        let mut edit = avenger_chart_core::ToolScaleEdit::raw_domain(
            channel,
            self.params
                .get(param_id)
                .ok_or_else(|| lowerer_error(declaration, "scale_edit param is unavailable"))?
                .name
                .clone(),
        );
        let avenger_chart_core::ToolScaleEdit::RawDomain {
            override_existing,
            disable_nice_zero,
            ..
        } = &mut edit;
        if let Some(ResolvedValue::Boolean(value)) = declaration.properties.get("override_existing")
        {
            *override_existing = *value;
        }
        if let Some(ResolvedValue::Boolean(value)) = declaration.properties.get("disable_nice_zero")
        {
            *disable_nice_zero = *value;
        }
        Ok(edit)
    }

    fn lower_container<'b>(
        &'b mut self,
        container: &'b ResolvedDeclaration,
        inherited_data: Option<&'b DataFrame>,
        plot: &'b mut ResolvedPlot,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ResolvedMarkGroup, Diagnostic>> + 'b>,
    > {
        Box::pin(async move {
            let (explicit_data, explicit_store) = match container.properties.get("data") {
                Some(value) if container.keyword != "chart" => {
                    if let Some((planning, store)) = self.store_data_binding(value, container)? {
                        (Some(planning), Some(store))
                    } else {
                        (Some(self.lower_data(value, container).await?), None)
                    }
                }
                _ => (None, None),
            };
            let mut current_data = explicit_data.as_ref().or(inherited_data).cloned();
            if let Some(data) = &explicit_data {
                self.record_schema(container, data);
            }
            let mut group = ResolvedMarkGroup::new();
            group.data = explicit_store.is_none().then_some(explicit_data).flatten();
            group.store_data = explicit_store;
            if is_resolved_mark_group(container) {
                group.id = container.name.clone();
                group.publish_id = container.public_path.is_some();
                group.component_kind = container.component_kind.clone();
            }

            for child in &container.children {
                match child.keyword.as_str() {
                    "transform" => {
                        let input = current_data.as_ref().ok_or_else(|| {
                            lowerer_error(
                                child,
                                "transform has no inherited or explicit data source",
                            )
                        })?;
                        let (data, stage) = self.lower_transform_stage(child, input, None).await?;
                        current_data = Some(data);
                        group.transforms.push(ResolvedTransformStage {
                            scope: stage.scope,
                            transform: stage.transform,
                        });
                    }
                    "mark" if is_resolved_mark_group(child) => {
                        let child_group = self
                            .lower_container(child, current_data.as_ref(), plot)
                            .await?;
                        group.marks.push(ResolvedMark::Group(Box::new(child_group)));
                    }
                    "view" => {
                        let view_group = self
                            .lower_view_scope(child, current_data.as_ref(), plot)
                            .await?;
                        group.marks.push(ResolvedMark::Group(Box::new(view_group)));
                    }
                    "mark" => {
                        let (mark_data, mark_store) = match child.properties.get("data") {
                            Some(value) => {
                                if let Some((planning, store)) =
                                    self.store_data_binding(value, child)?
                                {
                                    (Some(planning), Some(store))
                                } else {
                                    (Some(self.lower_data(value, child).await?), None)
                                }
                            }
                            None => (None, None),
                        };
                        let planning_data = mark_data.as_ref().or(current_data.as_ref());
                        let view = child
                            .children
                            .iter()
                            .find(|nested| nested.keyword == "view");
                        let mut view_group = if let Some(view) = view {
                            Some(self.lower_view_scope(view, planning_data, plot).await?)
                        } else {
                            None
                        };
                        self.lower_legend_overlays(child, plot).await?;
                        let declaration = self.native_declaration(
                            child,
                            planning_data,
                            NativeKindNamespace::Mark,
                        )?;
                        let key = NativeKindKey::mark(
                            child.coordinate.as_deref().unwrap_or(""),
                            child.kind.as_deref().unwrap_or(""),
                        );
                        let owns_plot_child = self
                            .registry
                            .snapshot()
                            .entries
                            .get(&key)
                            .is_some_and(|schema| {
                                schema.child_rules.iter().any(|rule| rule.role == "plot")
                            });
                        let native = if owns_plot_child {
                            let mut plots = child
                                .children
                                .iter()
                                .filter(|nested| nested.keyword == "plot");
                            let nested = plots.next().ok_or_else(|| {
                                lowerer_error(child, "registered child mark requires one `plot`")
                            })?;
                            if plots.next().is_some() {
                                return Err(lowerer_error(
                                    child,
                                    "registered child mark accepts only one `plot`",
                                ));
                            }
                            ResolvedMark::NativeWithChild {
                                declaration,
                                child: Box::new(
                                    self.lower_nested_plot(nested, planning_data).await?,
                                ),
                            }
                        } else {
                            ResolvedMark::Native(declaration)
                        };
                        if let Some(mut wrapper) = view_group.take() {
                            wrapper.data = mark_store.is_none().then_some(mark_data).flatten();
                            wrapper.store_data = mark_store;
                            wrapper.marks.push(native);
                            group.marks.push(ResolvedMark::Group(Box::new(wrapper)));
                        } else if mark_data.is_some() {
                            let mut wrapper = ResolvedMarkGroup::new();
                            wrapper.data = mark_store.is_none().then_some(mark_data).flatten();
                            wrapper.store_data = mark_store;
                            wrapper.marks.push(native);
                            group.marks.push(ResolvedMark::Group(Box::new(wrapper)));
                        } else {
                            group.marks.push(native);
                        }
                    }
                    "tool" if child.kind.as_deref() == Some("behavior") => {
                        let chart = self.active_chart_declaration()?.clone();
                        plot.tool_behaviors.push(
                            self.lower_tool_behavior(&chart, child, current_data.as_ref())
                                .await?,
                        );
                    }
                    "tool" => plot.tools.push(self.native_declaration(
                        child,
                        current_data.as_ref(),
                        NativeKindNamespace::Tool,
                    )?),
                    "widget" => {
                        let widget_data = match child.properties.get("data") {
                            Some(value) => Some(self.lower_widget_items(value, child).await?),
                            None => None,
                        };
                        let mut declaration = self.native_declaration(
                            child,
                            widget_data
                                .as_ref()
                                .map(|(_, data)| data)
                                .or(current_data.as_ref()),
                            NativeKindNamespace::Widget,
                        )?;
                        if let Some((items, _)) = &widget_data {
                            declaration.properties.insert(
                                "data".to_string(),
                                NativeValue::WidgetItems(items.clone()),
                            );
                        }
                        if let Some(action) = child.properties.get("action") {
                            declaration.state_action = Some(
                                self.lower_state_action_block(
                                    action,
                                    widget_data
                                        .as_ref()
                                        .map(|(_, data)| data)
                                        .or(current_data.as_ref()),
                                    child,
                                )?,
                            );
                        }
                        self.install_native_state_bindings(child, &mut declaration)?;
                        plot.widgets.push(declaration);
                    }
                    "cell" | "plot" => {
                        let child_plot =
                            self.lower_nested_plot(child, current_data.as_ref()).await?;
                        let placement = self.child_placement(child, current_data.as_ref())?;
                        plot.children.push(ResolvedChildPlot {
                            plot: Box::new(child_plot),
                            placement,
                        });
                    }
                    "param" | "store" | "selection" | "on" | "scale_edit" | "theme"
                    | "resource" | "export" => {}
                    other => {
                        let coordinate_key = NativeKindKey::new(
                            NativeKindNamespace::Coordinate,
                            plot.coordinate.kind.clone(),
                        );
                        if self
                            .registry
                            .snapshot()
                            .entries
                            .get(&coordinate_key)
                            .is_some_and(|schema| {
                                schema.child_rules.iter().any(|rule| rule.role == other)
                            })
                        {
                            continue;
                        }
                        return Err(lowerer_error(
                            child,
                            format!(
                                "registered native surface cannot lower `{other}` declarations"
                            ),
                        ));
                    }
                }
            }
            Ok(group)
        })
    }

    fn lower_legend_overlays<'b>(
        &'b mut self,
        declaration: &'b ResolvedDeclaration,
        plot: &'b mut ResolvedPlot,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Diagnostic>> + 'b>> {
        Box::pin(async move {
            for (overlay_id, children) in legend_overlay_blocks(declaration) {
                if self.legend_overlays.contains_key(&overlay_id) {
                    continue;
                }
                validate_legend_overlay_children(declaration, children)?;
                let mut synthetic = declaration.clone();
                synthetic.keyword = "mark".to_string();
                synthetic.kind = Some("group".to_string());
                synthetic.name = None;
                synthetic.visibility = Visibility::Private;
                synthetic.component_kind = None;
                synthetic.properties.clear();
                synthetic.property_channels.clear();
                synthetic.children = children.to_vec();
                synthetic.runtime_target = None;
                synthetic.public_path = None;
                synthetic.parts.clear();
                synthetic.exports.clear();
                synthetic.transform_outputs.clear();
                synthetic.event_binding = None;
                synthetic.state_lvalue = None;

                let mut group = self.lower_container(&synthetic, None, plot).await?;
                // A mark block is an isolated unit-data root. Its authored
                // child groups may replace that root with explicit data or a
                // store, but no chart-row relation crosses this boundary.
                group.data_mode = avenger_chart_core::MarkDataMode::Unit;
                let marks = self
                    .registry
                    .lower_marks(
                        "cartesian",
                        &Cartesian::new(),
                        &[ResolvedMark::Group(Box::new(group))],
                    )
                    .map_err(|error| lowerer_error(declaration, error.to_string()))?;
                let overlay = marks
                    .into_iter()
                    .fold(ColorbarOverlay::new(), |overlay, mark| overlay.mark(mark));
                self.legend_overlays.insert(overlay_id, overlay);
            }
            Ok(())
        })
    }

    fn lower_nested_plot<'b>(
        &'b mut self,
        declaration: &'b ResolvedDeclaration,
        inherited_data: Option<&'b DataFrame>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ResolvedPlot, Diagnostic>> + 'b>>
    {
        Box::pin(async move {
            let coordinate = declaration.kind.clone().ok_or_else(|| {
                lowerer_error(declaration, "child plot coordinate kind was not resolved")
            })?;
            let mut plot = ResolvedPlot::new(coordinate);
            let explicit_data = match declaration.properties.get("data") {
                Some(value) => Some(self.lower_data(value, declaration).await?),
                None => None,
            };
            if let Some(data) = &explicit_data {
                self.record_schema(declaration, data);
            }
            // A nested plot is compiled as an independently typed child plot.
            // Preserve lexical data inheritance by installing the current
            // DataFrame when the child does not author an override; otherwise
            // the child compiler would have no schema or runtime relation even
            // though its DSL scope correctly resolved inherited columns.
            plot.data = explicit_data.clone().or_else(|| inherited_data.cloned());
            plot.data_is_inherited = explicit_data.is_none() && inherited_data.is_some();
            plot.coordinate = self.native_declaration(
                declaration,
                plot.data.as_ref(),
                NativeKindNamespace::Coordinate,
            )?;
            let planning_data = plot.data.clone();
            let root_group = self
                .lower_container(declaration, planning_data.as_ref(), &mut plot)
                .await?;
            if !root_group.marks.is_empty() || !root_group.transforms.is_empty() {
                plot.marks.push(ResolvedMark::Group(Box::new(root_group)));
            }
            Ok(plot)
        })
    }

    fn lower_view_scope<'b>(
        &'b mut self,
        declaration: &'b ResolvedDeclaration,
        inherited_data: Option<&'b DataFrame>,
        plot: &'b mut ResolvedPlot,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ResolvedMarkGroup, Diagnostic>> + 'b>,
    > {
        Box::pin(async move {
            let spec = self.lower_view_spec(declaration, inherited_data)?;
            self.view_refs
                .insert(declaration.id.clone(), spec.view_ref());

            let mut group = self
                .lower_container(declaration, inherited_data, plot)
                .await?;
            let data = group.data.take();
            let store_data = group.store_data.take();
            let transforms = std::mem::take(&mut group.transforms);
            group.view = Some(ResolvedViewScope {
                spec: *spec,
                data,
                store_data,
                transforms,
            });
            Ok(group)
        })
    }

    fn lower_view_spec(
        &self,
        declaration: &ResolvedDeclaration,
        inherited_data: Option<&DataFrame>,
    ) -> Result<Box<avenger_chart_core::CompiledViewSpec>, Diagnostic> {
        let source_name = declaration
            .name
            .clone()
            .unwrap_or_else(|| declaration.id.as_str().to_string());
        let mut native =
            self.native_declaration(declaration, inherited_data, NativeKindNamespace::View)?;
        native.source_name = Some(source_name);
        // Domain strings use the same field shorthand as encoding channels:
        // `x_domain: "x"` means the data column, not a scalar string literal.
        for property in ["x_domain", "y_domain"] {
            if let Some(ResolvedValue::String(field)) = declaration.properties.get(property) {
                native.properties.insert(
                    property.to_string(),
                    NativeValue::Expr(unqualified_column(field)),
                );
            }
        }
        let key = NativeKindKey::new(NativeKindNamespace::View, native.kind.clone());
        self.registry
            .lower_object(&key, &native)
            .map_err(|error| lowerer_error(declaration, error.to_string()))?
            .downcast::<avenger_chart_core::CompiledViewSpec>()
            .map_err(|_| {
                lowerer_error(
                    declaration,
                    "registered view lowerer did not return CompiledViewSpec",
                )
            })
    }

    fn lower_transform_stage<'b>(
        &'b mut self,
        declaration: &'b ResolvedDeclaration,
        input: &'b DataFrame,
        inherited_scope: Option<avenger_chart_core::CoordinationScope>,
    ) -> TransformStageFuture<'b> {
        Box::pin(async move {
            let kind = declaration.kind.as_deref().ok_or_else(|| {
                lowerer_error(declaration, "native transform kind was not resolved")
            })?;
            let scope = self.transform_scope(declaration, inherited_scope)?;
            let native =
                self.native_declaration(declaration, Some(input), NativeKindNamespace::Transform)?;
            let context = avenger_chart_core::DataTransformCompileContext::new(scope);
            let lowered = match self
                .registry
                .transform_mode(kind)
                .map_err(|error| lowerer_error(declaration, error.to_string()))?
            {
                NativeTransformMode::Leaf => self
                    .registry
                    .lower_transform(&native, context)
                    .map_err(|error| lowerer_error(declaration, error.to_string()))?,
                NativeTransformMode::Pipeline => {
                    validate_private_pipeline_input(declaration, input)?;
                    let mut child_data = input.clone();
                    let mut stages = Vec::new();
                    for child in declaration
                        .children
                        .iter()
                        .filter(|child| child.keyword == "transform")
                    {
                        let (next_data, stage) = self
                            .lower_transform_stage(child, &child_data, Some(scope))
                            .await?;
                        child_data = next_data;
                        stages.push(stage);
                    }
                    let mut outputs = IndexMap::new();
                    for output in declaration
                        .children
                        .iter()
                        .filter(|child| child.keyword == "output")
                    {
                        let name = output.name.clone().ok_or_else(|| {
                            lowerer_error(output, "pipeline output has no resolved name")
                        })?;
                        let expr = match output.properties.get("value") {
                            Some(value) => {
                                self.expression_value(value, Some(&child_data), output)?
                            }
                            None => {
                                child_data
                                    .schema()
                                    .field_with_unqualified_name(&name)
                                    .map_err(|error| lowerer_error(output, error.to_string()))?;
                                Expr::Column(Column::new_unqualified(name.clone()))
                            }
                        };
                        outputs.insert(name, expr);
                    }
                    self.registry
                        .lower_transform_pipeline(&native, stages, outputs, context)
                        .map_err(|error| lowerer_error(declaration, error.to_string()))?
                }
            };

            let mut params = self
                .params
                .values()
                .map(|param| (param.name.clone(), param.default.clone()))
                .collect::<IndexMap<_, _>>();
            // View-scoped transforms are applied once during compilation to
            // propagate their output schema. Reserved view placeholders have
            // no live viewport yet, so install type-correct representative
            // values for that planning pass. Runtime materialization replaces
            // them with the evaluated view domain/range/pixel state.
            for view in self.view_refs.values() {
                for axis in [view.x(), view.y()] {
                    for (field, value) in [
                        ("domain_start", ScalarValue::Float64(Some(0.0))),
                        ("domain_end", ScalarValue::Float64(Some(1.0))),
                        ("range_start", ScalarValue::Float64(Some(0.0))),
                        ("range_end", ScalarValue::Float64(Some(1.0))),
                        ("pixels", ScalarValue::UInt32(Some(1))),
                    ] {
                        params.entry(axis.param_name(field)).or_insert(value);
                    }
                }
            }
            let execution = DataTransformExecutionContext {
                session_context: self.context,
                params: &params,
                time_context: TimeContext::default(),
                facet_context: None,
            };
            let result = lowered
                .transform
                .apply(input.clone(), &execution)
                .await
                .map_err(|error| lowerer_error(declaration, error.to_string()))?;
            self.install_transform_outputs(declaration, &lowered.outputs)?;
            self.record_schema(declaration, &result.dataframe);
            Ok((
                result.dataframe,
                DataTransformStage::new(scope, lowered.transform),
            ))
        })
    }

    fn transform_scope(
        &self,
        declaration: &ResolvedDeclaration,
        inherited: Option<avenger_chart_core::CoordinationScope>,
    ) -> Result<avenger_chart_core::CoordinationScope, Diagnostic> {
        let Some(value) = declaration.properties.get("scope") else {
            return Ok(inherited.unwrap_or(avenger_chart_core::CoordinationScope::Free));
        };
        match value {
            ResolvedValue::Atom(value) if value == "shared" => {
                Ok(avenger_chart_core::CoordinationScope::Shared)
            }
            ResolvedValue::Atom(value) if value == "free" => {
                Ok(avenger_chart_core::CoordinationScope::Free)
            }
            ResolvedValue::Call { function, args } if function == "level" => {
                let [ResolvedValue::Number(level)] = args.as_slice() else {
                    return Err(lowerer_error(
                        declaration,
                        "level(...) transform scope requires one integer",
                    ));
                };
                let level = level.parse::<u8>().map_err(|_| {
                    lowerer_error(
                        declaration,
                        "transform scope level must be an integer from 0 through 255",
                    )
                })?;
                Ok(avenger_chart_core::CoordinationScope::Level(level))
            }
            _ => Err(lowerer_error(
                declaration,
                "transform scope must be shared, free, or level(<integer>)",
            )),
        }
    }

    fn install_transform_outputs(
        &mut self,
        declaration: &ResolvedDeclaration,
        outputs: &BTreeMap<String, NativeOutputValue>,
    ) -> Result<(), Diagnostic> {
        for (name, handle) in &declaration.transform_outputs {
            let Some(value) = outputs.get(name) else {
                continue;
            };
            let matches = matches!(
                (&handle.shape, value),
                (
                    ResolvedOutputShape::Expression,
                    NativeOutputValue::Expr(_) | NativeOutputValue::Channel(_)
                ) | (
                    ResolvedOutputShape::RasterDimension,
                    NativeOutputValue::RasterDim(_)
                ) | (ResolvedOutputShape::Opaque, NativeOutputValue::Opaque(_))
            );
            if !matches {
                return Err(lowerer_error(
                    declaration,
                    format!(
                        "registered output `{name}` has native type incompatible with {:?}",
                        handle.shape
                    ),
                ));
            }
            self.transform_outputs.insert(handle.clone(), value.clone());
        }
        Ok(())
    }

    fn child_placement(
        &self,
        declaration: &ResolvedDeclaration,
        data: Option<&DataFrame>,
    ) -> Result<NativeDeclaration, Diagnostic> {
        let mut placement = NativeDeclaration::new("child");
        placement.source_name.clone_from(&declaration.name);
        if let Some(ResolvedValue::Object { properties, .. }) = declaration.properties.get("at") {
            for (name, value) in properties {
                placement
                    .properties
                    .insert(name.clone(), self.native_value(value, data, declaration)?);
            }
        }
        if let Some(label) = declaration.properties.get("label") {
            placement.properties.insert(
                "label".to_string(),
                self.native_value(label, data, declaration)?,
            );
        }
        if let Some(predicate) = declaration.properties.get("when") {
            placement.properties.insert(
                "when".to_string(),
                NativeValue::Expr(self.expression_value(predicate, data, declaration)?),
            );
        }
        Ok(placement)
    }

    fn native_declaration(
        &self,
        declaration: &ResolvedDeclaration,
        data: Option<&DataFrame>,
        namespace: NativeKindNamespace,
    ) -> Result<NativeDeclaration, Diagnostic> {
        let kind = declaration.kind.clone().ok_or_else(|| {
            lowerer_error(declaration, "native declaration kind was not resolved")
        })?;
        let key = match namespace {
            NativeKindNamespace::Mark => {
                NativeKindKey::mark(declaration.coordinate.as_deref().unwrap_or(""), &kind)
            }
            _ => NativeKindKey::new(namespace, &kind),
        };
        let schema = self.registry.snapshot().entries.get(&key).ok_or_else(|| {
            lowerer_error(
                declaration,
                format!("native registry has no schema/lowerer pair for {key:?}"),
            )
        })?;
        let mut native = NativeDeclaration::new(kind.clone());
        native.source_name.clone_from(&declaration.name);
        native.publish_source_name = declaration.public_path.is_some();
        let (aliases, part_alias) = self.mark_interface_metadata(declaration);
        native.public_aliases = aliases;
        native.component_part_alias = part_alias;
        if namespace == NativeKindNamespace::Mark {
            native.mark_effects = self.lower_mark_effects(declaration, data)?;
        }
        native.live_exports = declaration.exports.keys().cloned().collect();
        for (name, value) in &declaration.properties {
            if is_core_property(&declaration.keyword, name) {
                continue;
            }
            let property_shape = schema.properties.get(name).map(|property| &property.shape);
            // Widget data is lowered asynchronously at the container call
            // site and installed after this synchronous schema-directed pass.
            if property_shape == Some(&ValueShape::WidgetData) {
                continue;
            }
            // Ordered action children are lowered to `ChartAction`, not an
            // untyped native object. The owner-facing declaration carries the
            // result separately so authored order cannot be lost in a map.
            if property_shape == Some(&ValueShape::StateActionBlock) {
                continue;
            }
            // Language-owned mark blocks are lowered asynchronously before
            // this registered native declaration is constructed.
            if property_shape == Some(&ValueShape::MarkBlock) {
                continue;
            }
            // Adjustment output routing is validated against the outputs
            // returned by the paired lowerer after native construction.
            // Preserve its expression-shaped map for registry validation
            // without asking DataFusion to resolve synthetic `binder.output`
            // names against the chart-row schema.
            if namespace == NativeKindNamespace::Adjust && name == "apply" {
                let ResolvedValue::Object { properties, .. } = value else {
                    return Err(lowerer_error(
                        declaration,
                        "transform adjustment `apply:` must be a property block",
                    ));
                };
                let mut routed = IndexMap::new();
                for (channel, value) in properties {
                    let output = adjustment_output_name(value, declaration).ok_or_else(|| {
                        lowerer_error(
                            declaration,
                            format!("`apply.{channel}` must reference a bound adjustment output"),
                        )
                    })?;
                    routed.insert(
                        channel.clone(),
                        NativeValue::Expr(unqualified_column(output)),
                    );
                }
                native
                    .properties
                    .insert(name.clone(), NativeValue::Object(routed));
                continue;
            }
            // A mark declaration contains both encoding channels and ordinary
            // native properties (for example Text.syntax and raster
            // dimensions). The registry schema, not the `mark` keyword,
            // decides which lowering path applies.
            let lowered = if let Some(channel) = schema.channels.get(name) {
                if channel.shape == ValueShape::PatternChannel {
                    self.pattern_channel_value(value, data, declaration)?
                } else {
                    self.channel_value(value, data, declaration)?
                }
            } else if schema
                .properties
                .get(name)
                .is_some_and(|property| property.shape == ValueShape::ChannelMap)
            {
                self.channel_map_value(value, data, declaration)?
            } else if schema
                .properties
                .get(name)
                .is_some_and(|property| property.shape == ValueShape::RasterDimensionChannel)
            {
                self.raster_dimension_channel_value(name, value, data, declaration)?
            } else if let Some(ValueShape::ConfiguredExpression(fields)) = property_shape {
                self.configured_expression_value(value, fields, data, declaration)?
            } else if let Some(ValueShape::ConfiguredReference {
                namespaces,
                properties,
            }) = property_shape
            {
                self.configured_reference_value(value, namespaces, properties, data, declaration)?
            } else if schema
                .properties
                .get(name)
                .is_some_and(|property| property.shape == ValueShape::FacetDataScope)
            {
                NativeValue::Integer(self.facet_data_scope_level(value, declaration)?.into())
            } else if property_shape == Some(&ValueShape::CoordinationScope) {
                match self.coordination_scope(value, declaration)? {
                    avenger_chart_core::CoordinationScope::Shared => {
                        NativeValue::String("shared".to_string())
                    }
                    avenger_chart_core::CoordinationScope::Free => {
                        NativeValue::String("free".to_string())
                    }
                    avenger_chart_core::CoordinationScope::Level(level) => {
                        NativeValue::Integer(level.into())
                    }
                }
            } else if matches!(
                property_shape,
                Some(ValueShape::Array(inner)) if inner.as_ref() == &ValueShape::SqlExpression
            ) {
                let ResolvedValue::Array(values) = value else {
                    return Err(lowerer_error(
                        declaration,
                        format!("property `{name}` must be an array of SQL expressions"),
                    ));
                };
                NativeValue::Array(
                    values
                        .iter()
                        .map(|value| {
                            self.expression_value(value, data, declaration)
                                .map(NativeValue::Expr)
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                )
            } else if schema
                .properties
                .get(name)
                .is_some_and(|property| property.shape == ValueShape::SqlExpression)
            {
                NativeValue::Expr(self.expression_value(value, data, declaration)?)
            } else if matches!(property_shape, Some(ValueShape::SqlProjection { .. })) {
                let ResolvedValue::Projection(projection) = value else {
                    return Err(lowerer_error(
                        declaration,
                        format!("property `{name}` must be a SQL projection list"),
                    ));
                };
                NativeValue::Projection(
                    projection
                        .items
                        .iter()
                        .map(|item| {
                            let expression = item.expression.as_ref().ok_or_else(|| {
                                lowerer_error(
                                    declaration,
                                    "wildcards are not allowed in projection lists",
                                )
                            })?;
                            Ok(NativeProjectionItem {
                                expr: self.expression_value(
                                    &ResolvedValue::Expression(expression.clone()),
                                    data,
                                    declaration,
                                )?,
                                alias: item.aliases.first().cloned(),
                                direct_column: item.direct_column,
                            })
                        })
                        .collect::<Result<Vec<_>, Diagnostic>>()?,
                )
            } else if property_shape == Some(&ValueShape::SelectionBinding) {
                let ResolvedValue::Reference(reference) = value else {
                    return Err(lowerer_error(
                        declaration,
                        format!("property `{name}` must reference a selection"),
                    ));
                };
                let ResolvedTarget::Selection(id) = &reference.target else {
                    return Err(lowerer_error(
                        declaration,
                        format!("property `{name}` must reference a selection"),
                    ));
                };
                NativeValue::Selection(self.selections.get(id).cloned().ok_or_else(|| {
                    lowerer_error(declaration, "resolved selection is unavailable")
                })?)
            } else if property_shape == Some(&ValueShape::ScalarBinding) {
                let ResolvedValue::Binding(binding) = value else {
                    return Err(lowerer_error(
                        declaration,
                        format!("property `{name}` must bind a parameter"),
                    ));
                };
                let ResolvedTarget::Param(id) = &binding.target else {
                    return Err(lowerer_error(
                        declaration,
                        format!("property `{name}` must bind a parameter"),
                    ));
                };
                NativeValue::Param(self.params.get(id).cloned().ok_or_else(|| {
                    lowerer_error(declaration, "resolved parameter is unavailable")
                })?)
            } else {
                self.native_value(value, data, declaration)?
            };
            native.properties.insert(name.clone(), lowered);
        }
        for child in &declaration.children {
            // Coordinate-owned structural children (for example parallel
            // dimensions) are consumed directly by the coordinate lowerer.
            // Transform pipeline children have a dedicated ordered lowering
            // path below: forwarding them here would resolve output handles
            // before the preceding child stages have installed those outputs.
            if namespace == NativeKindNamespace::Coordinate
                && !matches!(child.keyword.as_str(), "cell" | "plot")
                && schema
                    .child_rules
                    .iter()
                    .any(|rule| rule.role == child.keyword)
            {
                native.children.push(self.native_owner_child(child, data)?);
            }
        }
        Ok(native)
    }

    fn lower_mark_effects(
        &self,
        declaration: &ResolvedDeclaration,
        data: Option<&DataFrame>,
    ) -> Result<PrimitiveMarkEffects, Diagnostic> {
        let mut effects = PrimitiveMarkEffects::default();
        for child in &declaration.children {
            match child.keyword.as_str() {
                "adjust" if child.kind.is_none() => {
                    let mut assignments = Vec::new();
                    for (channel, value) in &child.properties {
                        assignments.push(
                            ItemChannelAssignment::new(
                                channel,
                                self.event_expression_value(value, data, child)?,
                            )
                            .map_err(|error| lowerer_error(child, error.to_string()))?,
                        );
                    }
                    if !assignments.is_empty() {
                        effects.push_adjustment(MarkAdjustmentSpec::Expr(
                            ExpressionMarkAdjustmentSpec::new(assignments),
                        ));
                    }
                }
                "adjust" => {
                    effects.push_adjustment(MarkAdjustmentSpec::Transform(
                        self.lower_transform_mark_adjustment(child, effects.adjustments.len())?,
                    ));
                }
                "derive" => effects.push_derived(self.lower_derived_mark(child, data)?),
                _ => {}
            }
        }
        Ok(effects)
    }

    fn lower_transform_mark_adjustment(
        &self,
        declaration: &ResolvedDeclaration,
        stage_index: usize,
    ) -> Result<TransformMarkAdjustmentSpec, Diagnostic> {
        let kind = declaration.kind.as_deref().ok_or_else(|| {
            lowerer_error(
                declaration,
                "transform adjustment requires a registered kind",
            )
        })?;
        let native = self.native_declaration(declaration, None, NativeKindNamespace::Adjust)?;
        let context = MarkAdjustmentCompileContext::new(stage_index);
        let lowered = self
            .registry
            .lower_adjustment(&native, context)
            .map_err(|error| lowerer_error(declaration, error.to_string()))?;
        let compiled = lowered.transform;
        let outputs = lowered.outputs;
        let apply = declaration
            .properties
            .get("apply")
            .ok_or_else(|| lowerer_error(declaration, "transform adjustment requires `apply:`"))?;
        let ResolvedValue::Object { properties, .. } = apply else {
            return Err(lowerer_error(
                declaration,
                "transform adjustment `apply:` must be a property block",
            ));
        };
        let mut assignments = Vec::new();
        for (channel, value) in properties {
            let output = adjustment_output_name(value, declaration).ok_or_else(|| {
                lowerer_error(
                    declaration,
                    format!("`apply.{channel}` must reference a bound adjustment output"),
                )
            })?;
            let expr = outputs.get(output.as_str()).ok_or_else(|| {
                lowerer_error(
                    declaration,
                    format!("adjustment `{kind}` has no `{output}` output"),
                )
            })?;
            assignments.push(
                ItemChannelAssignment::new(channel, expr.clone())
                    .map_err(|error| lowerer_error(declaration, error.to_string()))?,
            );
        }
        Ok(TransformMarkAdjustmentSpec::new(compiled, assignments))
    }

    fn lower_derived_mark(
        &self,
        declaration: &ResolvedDeclaration,
        data: Option<&DataFrame>,
    ) -> Result<DerivedPrimitiveMarkSpec, Diagnostic> {
        let kind = declaration
            .kind
            .as_deref()
            .ok_or_else(|| lowerer_error(declaration, "derived mark requires a primitive kind"))?;
        let zindex = declaration
            .properties
            .get("zindex")
            .map(|value| resolved_i32(value, declaration, "zindex"))
            .transpose()?;
        let mut assignments = Vec::new();
        for (channel, value) in &declaration.properties {
            if channel == "zindex" {
                continue;
            }
            assignments.push(
                ItemChannelAssignment::new(
                    channel,
                    self.event_expression_value(value, data, declaration)?,
                )
                .map_err(|error| lowerer_error(declaration, error.to_string()))?,
            );
        }
        Ok(match kind {
            "symbol" => {
                DerivedPrimitiveMarkSpec::Symbol(DerivedSymbolMarkSpec::new(assignments, zindex))
            }
            "rule" => DerivedPrimitiveMarkSpec::Rule(DerivedRuleMarkSpec::new(assignments, zindex)),
            "rect" => DerivedPrimitiveMarkSpec::Rect(DerivedRectMarkSpec::new(assignments, zindex)),
            "text" => DerivedPrimitiveMarkSpec::Text(DerivedTextMarkSpec::new(
                assignments,
                PrimitiveMarkEffects::default(),
                zindex,
            )),
            _ => {
                return Err(lowerer_error(
                    declaration,
                    format!("unsupported derived primitive mark `{kind}`"),
                ));
            }
        })
    }

    fn mark_interface_metadata(
        &self,
        declaration: &ResolvedDeclaration,
    ) -> (Vec<String>, Option<String>) {
        if declaration.keyword != "mark" {
            return (Vec::new(), None);
        }
        let Some(target) = declaration.runtime_target.as_ref() else {
            return (Vec::new(), None);
        };
        let mut aliases = Vec::new();
        let mut component_aliases = Vec::new();
        for owner in self
            .project
            .source_modules
            .values()
            .flat_map(|file| file.roots.iter())
            .flat_map(declarations_depth_first)
        {
            let Some(owner_path) = owner.public_path.as_deref() else {
                continue;
            };
            for (alias, exported) in &owner.exports {
                if exported != target {
                    continue;
                }
                let full = format!("{owner_path}.{alias}");
                let relative = self
                    .active_chart_path
                    .as_deref()
                    .and_then(|chart| full.strip_prefix(chart))
                    .and_then(|path| path.strip_prefix('.'))
                    .unwrap_or(&full)
                    .to_string();
                aliases.push(relative);
                if owner.component_kind.is_some() {
                    component_aliases.push(alias.clone());
                }
            }
        }
        aliases.sort();
        aliases.dedup();
        component_aliases.sort();
        component_aliases.dedup();
        (aliases, component_aliases.into_iter().next())
    }

    fn configured_expression_value(
        &self,
        value: &ResolvedValue,
        fields: &std::collections::BTreeMap<String, avenger_chart_schema::PropertySchema>,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<NativeValue, Diagnostic> {
        let ResolvedValue::Object {
            head: Some(head),
            properties,
            ..
        } = value
        else {
            return Err(lowerer_error(
                declaration,
                "configured expression requires a value head and property block",
            ));
        };
        let properties = properties
            .iter()
            .map(|(name, value)| {
                let field = fields.get(name).ok_or_else(|| {
                    lowerer_error(
                        declaration,
                        format!("unsupported configured-expression property `{name}`"),
                    )
                })?;
                let value = match &field.shape {
                    ValueShape::SqlExpression => {
                        NativeValue::Expr(self.expression_value(value, data, declaration)?)
                    }
                    ValueShape::CoordinationScope => {
                        match self.coordination_scope(value, declaration)? {
                            avenger_chart_core::CoordinationScope::Shared => {
                                NativeValue::String("shared".to_string())
                            }
                            avenger_chart_core::CoordinationScope::Free => {
                                NativeValue::String("free".to_string())
                            }
                            avenger_chart_core::CoordinationScope::Level(level) => {
                                NativeValue::Integer(level.into())
                            }
                        }
                    }
                    _ => self.native_value(value, data, declaration)?,
                };
                Ok((name.clone(), value))
            })
            .collect::<Result<_, Diagnostic>>()?;
        Ok(NativeValue::Configured {
            head: Box::new(self.channel_value(head, data, declaration)?),
            properties,
        })
    }

    fn facet_data_scope_level(
        &self,
        value: &ResolvedValue,
        declaration: &ResolvedDeclaration,
    ) -> Result<u8, Diagnostic> {
        match value {
            ResolvedValue::Atom(value) if value == "filtered" => Ok(0),
            ResolvedValue::Atom(value) if value == "broadcast" => Ok(u8::MAX),
            ResolvedValue::Call { function, args } if function == "level" => {
                let [ResolvedValue::Number(level)] = args.as_slice() else {
                    return Err(lowerer_error(
                        declaration,
                        "facet data scope level(...) requires one integer",
                    ));
                };
                level.parse::<u8>().map_err(|_| {
                    lowerer_error(
                        declaration,
                        "facet data scope level must be an integer from 0 through 255",
                    )
                })
            }
            _ => Err(lowerer_error(
                declaration,
                "facet_data_scope must be filtered, broadcast, or level(<integer>)",
            )),
        }
    }

    fn coordination_scope(
        &self,
        value: &ResolvedValue,
        declaration: &ResolvedDeclaration,
    ) -> Result<avenger_chart_core::CoordinationScope, Diagnostic> {
        match value {
            ResolvedValue::Atom(value) if value == "shared" => {
                Ok(avenger_chart_core::CoordinationScope::Shared)
            }
            ResolvedValue::Atom(value) if value == "free" => {
                Ok(avenger_chart_core::CoordinationScope::Free)
            }
            ResolvedValue::Call { function, args } if function == "level" => {
                let [ResolvedValue::Number(level)] = args.as_slice() else {
                    return Err(lowerer_error(
                        declaration,
                        "coordination scope level(...) requires one integer",
                    ));
                };
                level
                    .parse::<u8>()
                    .map(avenger_chart_core::CoordinationScope::Level)
                    .map_err(|_| {
                        lowerer_error(
                            declaration,
                            "coordination scope level must be an integer from 0 through 255",
                        )
                    })
            }
            _ => Err(lowerer_error(
                declaration,
                "coordination scope must be shared, free, or level(<integer>)",
            )),
        }
    }

    fn install_native_state_bindings(
        &self,
        source: &ResolvedDeclaration,
        native: &mut NativeDeclaration,
    ) -> Result<(), Diagnostic> {
        let kind = source
            .kind
            .as_deref()
            .ok_or_else(|| lowerer_error(source, "native widget kind was not resolved"))?;
        let key = NativeKindKey::new(NativeKindNamespace::Widget, kind);
        let schema = &self.registry.snapshot().entries[&key];
        for export in schema.exports.values() {
            let Some(property) = &export.binding_property else {
                continue;
            };
            if native.properties.contains_key(property) {
                continue;
            }
            let Some(target) = source.exports.get(&export.alias) else {
                continue;
            };
            let value = match target {
                ResolvedTarget::Param(id)
                    if self.project.params[id].generated_by.is_some()
                        && self.params[id].default.is_null() =>
                {
                    None
                }
                ResolvedTarget::Param(id) => self.params.get(id).cloned().map(NativeValue::Param),
                ResolvedTarget::Selection(id) => {
                    self.selections.get(id).cloned().map(NativeValue::Selection)
                }
                _ => None,
            };
            if let Some(value) = value {
                native.properties.insert(property.clone(), value);
            }
        }
        Ok(())
    }

    fn native_owner_child(
        &self,
        declaration: &ResolvedDeclaration,
        data: Option<&DataFrame>,
    ) -> Result<NativeDeclaration, Diagnostic> {
        let mut native = NativeDeclaration::new(declaration.keyword.clone());
        native.variant.clone_from(&declaration.kind);
        native.source_name.clone_from(&declaration.name);
        for (name, value) in &declaration.properties {
            native
                .properties
                .insert(name.clone(), self.native_value(value, data, declaration)?);
        }
        native.children = declaration
            .children
            .iter()
            .map(|child| self.native_owner_child(child, data))
            .collect::<Result<_, _>>()?;
        Ok(native)
    }

    fn channel_map_value(
        &self,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<NativeValue, Diagnostic> {
        let ResolvedValue::Object { properties, .. } = value else {
            return Err(lowerer_error(
                declaration,
                "configured channel map did not resolve to an object",
            ));
        };
        Ok(NativeValue::Object(
            properties
                .iter()
                .map(|(name, value)| {
                    Ok((name.clone(), self.channel_value(value, data, declaration)?))
                })
                .collect::<Result<_, Diagnostic>>()?,
        ))
    }

    fn raster_dimension_channel_value(
        &self,
        property: &str,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<NativeValue, Diagnostic> {
        let (dimension_value, configs) = match value {
            ResolvedValue::Object {
                head: Some(head),
                properties,
                ..
            } => (head.as_ref(), Some(properties)),
            _ => (value, None),
        };
        let NativeValue::Output(NativeOutputValue::RasterDim(dimension)) =
            self.native_value(dimension_value, data, declaration)?
        else {
            return Err(lowerer_error(
                declaration,
                "configured raster channel requires a raster dimension handle",
            ));
        };
        // Categorical overlay dimensions use an explicitly typed null seed so
        // scale inference chooses ordinal without adding a placeholder domain
        // value. Position dimensions use the same numeric seed as the native
        // RasterPositionConfig builder.
        let mut channel = if property == "fill_by" {
            ChannelValue::from(lit(ScalarValue::Utf8(None)))
        } else {
            ChannelValue::from(lit(0.0_f64)).with_scale_name(property)
        };
        if let Some(configs) = configs {
            channel = self.apply_channel_configs(channel, configs, data, declaration)?;
        }
        Ok(NativeValue::RasterDimensionChannel {
            dimension,
            channel: Box::new(channel),
        })
    }

    fn pattern_channel_value(
        &self,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<NativeValue, Diagnostic> {
        if let ResolvedValue::Pattern(value) = value {
            return Ok(NativeValue::Pattern(PatternChannelValue::value(Some(
                self.pattern_fill(value, declaration)?,
            ))));
        }
        let NativeValue::Channel(channel) = self.channel_value(value, data, declaration)? else {
            unreachable!("channel lowering always returns a channel")
        };
        Ok(NativeValue::Pattern((*channel).into()))
    }

    fn pattern_fill(
        &self,
        value: &ResolvedValue,
        declaration: &ResolvedDeclaration,
    ) -> Result<PatternFill, Diagnostic> {
        let ResolvedValue::Object {
            properties,
            children,
            ..
        } = value
        else {
            return Err(lowerer_error(
                declaration,
                "pattern literal requires a property block",
            ));
        };
        let anchor = match properties.get("anchor").and_then(resolved_atom) {
            None | Some("plot") => PatternAnchor::Plot,
            Some("mark") => PatternAnchor::Mark,
            Some("chart") => PatternAnchor::Chart,
            Some(value) => {
                return Err(lowerer_error(
                    declaration,
                    format!("unsupported pattern anchor `{value}`"),
                ));
            }
        };
        let ink = match properties.get("ink") {
            None => PatternInk::default(),
            Some(ResolvedValue::Object {
                kind: Some(kind),
                properties,
                ..
            }) if kind == "auto_contrast" => PatternInk::AutoContrast {
                opacity: resolved_f32(properties.get("opacity"), 0.18, declaration, "ink.opacity")?,
            },
            Some(ResolvedValue::Object {
                kind: Some(kind),
                properties,
                ..
            }) if kind == "solid" => {
                let color = properties
                    .get("color")
                    .and_then(resolved_string)
                    .ok_or_else(|| {
                        lowerer_error(declaration, "solid pattern ink requires `color`")
                    })?;
                PatternInk::Solid {
                    color: avenger_color::parse_color_string_strict(color).map_err(|error| {
                        lowerer_error(declaration, format!("invalid pattern ink color: {error}"))
                    })?,
                    opacity: resolved_f32(
                        properties.get("opacity"),
                        0.18,
                        declaration,
                        "ink.opacity",
                    )?,
                }
            }
            Some(_) => {
                return Err(lowerer_error(
                    declaration,
                    "pattern ink must be `auto_contrast { ... }` or `solid { ... }`",
                ));
            }
        };
        let mut layers = Vec::new();
        for layer in children {
            if layer.keyword != "layer" {
                return Err(lowerer_error(
                    declaration,
                    format!("unsupported pattern child `{}`", layer.keyword),
                ));
            }
            match layer.kind.as_deref() {
                Some("stripe") => {
                    let angle = required_resolved_f32(layer, "angle", declaration)?;
                    let spacing =
                        required_resolved_f32_alias(layer, "spacing_px", "spacing", declaration)?;
                    let stroke_width = optional_resolved_f32_alias(
                        layer,
                        "stroke_width_px",
                        "stroke_width",
                        1.0,
                        declaration,
                    )?;
                    let mut stripe = StripePatternLayer::new(angle, spacing, stroke_width);
                    stripe.phase =
                        optional_resolved_f32_alias(layer, "phase_px", "phase", 0.0, declaration)?;
                    stripe.operation =
                        pattern_operation(layer.properties.get("operation"), declaration)?;
                    if let Some(ResolvedValue::Object { properties, .. }) =
                        layer.properties.get("dash")
                    {
                        stripe.dash = Some(StripeDash {
                            length: resolved_f32(
                                properties
                                    .get("length_px")
                                    .or_else(|| properties.get("length")),
                                f32::NAN,
                                declaration,
                                "dash.length_px",
                            )?,
                            gap: resolved_f32(
                                properties.get("gap_px").or_else(|| properties.get("gap")),
                                f32::NAN,
                                declaration,
                                "dash.gap_px",
                            )?,
                            phase: resolved_f32(
                                properties
                                    .get("phase_px")
                                    .or_else(|| properties.get("phase")),
                                0.0,
                                declaration,
                                "dash.phase_px",
                            )?,
                        });
                    }
                    layers.push(PatternLayer::Stripe(stripe));
                }
                Some(kind) => {
                    return Err(lowerer_error(
                        declaration,
                        format!("unsupported pattern layer `{kind}`"),
                    ));
                }
                None => {
                    return Err(lowerer_error(declaration, "pattern layer requires a kind"));
                }
            }
        }
        let pattern = PatternFill {
            anchor,
            ink,
            layers,
        };
        pattern
            .validate()
            .map_err(|error| lowerer_error(declaration, format!("{error:?}")))?;
        Ok(pattern)
    }

    fn native_value(
        &self,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<NativeValue, Diagnostic> {
        if let ResolvedValue::Expression(expression) = value
            && let Some(output) = self.direct_output(expression)
        {
            return Ok(NativeValue::Output(output.clone()));
        }
        Ok(match value {
            ResolvedValue::Boolean(value) => NativeValue::Boolean(*value),
            ResolvedValue::Number(value) => value
                .parse::<i64>()
                .map(NativeValue::Integer)
                .or_else(|_| value.parse::<f64>().map(NativeValue::Number))
                .map_err(|_| {
                    lowerer_error(declaration, format!("invalid numeric literal `{value}`"))
                })?,
            ResolvedValue::String(value) | ResolvedValue::Atom(value) => {
                NativeValue::String(value.clone())
            }
            ResolvedValue::Null => NativeValue::Scalar(ScalarValue::Null),
            ResolvedValue::ChannelValue(_) => {
                return self.channel_value(value, data, declaration);
            }
            ResolvedValue::Column(_)
            | ResolvedValue::Expression(_)
            | ResolvedValue::Channel { .. }
            | ResolvedValue::Binding(_) => {
                NativeValue::Expr(self.expression_value(value, data, declaration)?)
            }
            ResolvedValue::Projection(projection) => NativeValue::Projection(
                projection
                    .items
                    .iter()
                    .map(|item| {
                        let expression = item.expression.as_ref().ok_or_else(|| {
                            lowerer_error(
                                declaration,
                                "wildcards are not allowed in projection lists",
                            )
                        })?;
                        Ok(NativeProjectionItem {
                            expr: self.expression_value(
                                &ResolvedValue::Expression(expression.clone()),
                                data,
                                declaration,
                            )?,
                            alias: item.aliases.first().cloned(),
                            direct_column: item.direct_column,
                        })
                    })
                    .collect::<Result<Vec<_>, Diagnostic>>()?,
            ),
            ResolvedValue::Query(query) => NativeValue::Query(self.query_sql(query, declaration)?),
            ResolvedValue::Array(values) => NativeValue::Array(
                values
                    .iter()
                    .map(|value| self.native_value(value, data, declaration))
                    .collect::<Result<_, _>>()?,
            ),
            ResolvedValue::Object { properties, .. } => NativeValue::Object(
                properties
                    .iter()
                    .map(|(name, value)| {
                        Ok((name.clone(), self.native_value(value, data, declaration)?))
                    })
                    .collect::<Result<_, Diagnostic>>()?,
            ),
            ResolvedValue::Call { function, args } => NativeValue::Call {
                function: function.clone(),
                args: args
                    .iter()
                    .map(|value| self.native_value(value, data, declaration))
                    .collect::<Result<_, _>>()?,
            },
            ResolvedValue::Dimension(dimension) => NativeValue::Output(
                self.transform_outputs
                    .get(&dimension.target)
                    .cloned()
                    .ok_or_else(|| {
                        lowerer_error(
                            declaration,
                            format!(
                                "raster dimension `{}` is unavailable at this dataflow position",
                                dimension.authored_path.join(".")
                            ),
                        )
                    })?,
            ),
            ResolvedValue::Reference(reference)
                if matches!(reference.target, ResolvedTarget::Output(_)) =>
            {
                let ResolvedTarget::Output(handle) = &reference.target else {
                    unreachable!()
                };
                NativeValue::Output(self.transform_outputs.get(handle).cloned().ok_or_else(
                    || {
                        lowerer_error(
                            declaration,
                            format!(
                                "transform output `{}` is unavailable at this dataflow position",
                                reference.authored_path.join(".")
                            ),
                        )
                    },
                )?)
            }
            ResolvedValue::Reference(_)
            | ResolvedValue::Relation(_)
            | ResolvedValue::Pattern(_)
            | ResolvedValue::Environment(_)
            | ResolvedValue::None
            | ResolvedValue::DefinitionArgument(_)
            | ResolvedValue::Invalid => {
                return Err(lowerer_error(
                    declaration,
                    format!("value `{value:?}` is not lowerable in this native slot"),
                ));
            }
        })
    }

    fn configured_reference_value(
        &self,
        value: &ResolvedValue,
        namespaces: &BTreeSet<NativeKindNamespace>,
        fields: &BTreeMap<String, avenger_chart_schema::PropertySchema>,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<NativeValue, Diagnostic> {
        let ResolvedValue::Object {
            head: Some(head),
            properties,
            ..
        } = value
        else {
            return Err(lowerer_error(
                declaration,
                "configured reference requires a reference head",
            ));
        };
        let ResolvedValue::Reference(reference) = head.as_ref() else {
            return Err(lowerer_error(
                declaration,
                "configured reference head was not resolved",
            ));
        };
        if namespaces.len() != 1 || !namespaces.contains(&NativeKindNamespace::Resource) {
            return Err(lowerer_error(
                declaration,
                "configured native references currently require one resource namespace",
            ));
        }
        let ResolvedTarget::Declaration(id) = &reference.target else {
            return Err(lowerer_error(
                declaration,
                "configured resource reference has the wrong target kind",
            ));
        };
        let resource = find_declaration(self.project, id).ok_or_else(|| {
            lowerer_error(
                declaration,
                "configured resource declaration is unavailable",
            )
        })?;
        if resource.keyword != "resource" {
            return Err(lowerer_error(
                declaration,
                "configured reference does not name a resource declaration",
            ));
        }
        let native = self.native_declaration(resource, None, NativeKindNamespace::Resource)?;
        let key = NativeKindKey::new(NativeKindNamespace::Resource, native.kind.clone());
        let lowered = self
            .registry
            .lower_object(&key, &native)
            .map_err(|error| lowerer_error(resource, error.to_string()))?;
        let opaque: Arc<dyn std::any::Any + Send + Sync> = lowered.into();
        let properties = properties
            .iter()
            .map(|(name, value)| {
                if !fields.contains_key(name) {
                    return Err(lowerer_error(
                        declaration,
                        format!("unknown configured resource property `{name}`"),
                    ));
                }
                Ok((name.clone(), self.native_value(value, data, declaration)?))
            })
            .collect::<Result<IndexMap<_, _>, _>>()?;
        Ok(NativeValue::Configured {
            head: Box::new(NativeValue::Output(NativeOutputValue::Opaque(opaque))),
            properties,
        })
    }

    fn channel_value(
        &self,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<NativeValue, Diagnostic> {
        let data_expr = match value {
            ResolvedValue::ChannelValue(channel) => {
                self.channel_data_expr(&channel.effective_fallback().expression, data, declaration)?
            }
            ResolvedValue::Object {
                head, properties, ..
            } => match properties
                .get("otherwise")
                .and_then(conditional_branch_expression)
                .or(head.as_deref())
            {
                Some(head) => self.channel_data_expr(head, data, declaration)?,
                None => lit(1.0_f64),
            },
            _ => self.channel_data_expr(value, data, declaration)?,
        };
        let mut channel = match value {
            ResolvedValue::ChannelValue(resolved) => {
                self.lower_resolved_channel_value(resolved, data, declaration)?
            }
            ResolvedValue::Object {
                head,
                properties,
                children,
                ..
            } => {
                // Configuration-only channels (for example a raster's
                // opacity-by-total scale) intentionally omit a data head. A
                // scaled numeric seed asks the native owner/runtime to supply
                // the real values while retaining ordinary scale metadata.
                let mut channel = match head {
                    Some(head) => self.raw_channel_value(head, data, declaration)?,
                    None => ChannelValue::from(lit(1.0_f64)),
                };
                if !children.is_empty() || properties.contains_key("otherwise") {
                    channel = self.apply_channel_conditions(
                        channel,
                        children,
                        properties.get("otherwise"),
                        data,
                        declaration,
                    )?;
                }
                self.apply_channel_configs(channel, properties, data, declaration)?
            }
            _ => self.raw_channel_value(value, data, declaration)?,
        };
        if channel.get_scale_config().is_none()
            && matches!(
                &channel,
                ChannelValue::Scaled { .. } | ChannelValue::Conditional { .. }
            )
            && data.is_some_and(|data| {
                data_expr.get_type(data.schema()).is_ok_and(|data_type| {
                    matches!(
                        data_type,
                        DataType::Decimal128(_, _) | DataType::Decimal256(_, _)
                    )
                })
            })
        {
            // Exact fractional SQL literals infer as Arrow decimals. The
            // general Rust API intentionally retains its existing scale-type
            // policy, so the language compiler supplies the numeric intent at
            // this boundary without changing excluded/injected scale behavior
            // in compound coordinate systems.
            channel = channel.scale_with::<Linear>(|scale| scale);
        }
        Ok(NativeValue::Channel(Box::new(ChannelExpr::new(
            data_expr, channel,
        ))))
    }

    fn lower_resolved_channel_value(
        &self,
        resolved: &avenger_lang_core::ResolvedChannelValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<ChannelValue, Diagnostic> {
        let fallback = resolved.effective_fallback();
        let mut channel = self.raw_resolved_channel_branch(fallback, data, declaration)?;
        if !resolved.conditions.is_empty() {
            let conditions = resolved
                .conditions
                .iter()
                .map(|condition| {
                    let predicate =
                        self.expression_value(&condition.predicate, data, declaration)?;
                    let predicate = LogicalExprNode::from_default_expr(predicate)
                        .map_err(|error| lowerer_error(declaration, error.to_string()))?;
                    let branch =
                        self.resolved_conditional_branch(&condition.branch, data, declaration)?;
                    Ok((predicate, branch))
                })
                .collect::<Result<Vec<_>, Diagnostic>>()?;
            let otherwise = self.resolved_conditional_branch(fallback, data, declaration)?;
            channel = match channel {
                ChannelValue::Scaled {
                    scale_config,
                    nested_band_config,
                    legend_config,
                    axis_config,
                    domain_coordination,
                    transform_scope,
                    scale_domain_inference,
                    ..
                } => ChannelValue::Conditional {
                    conditions,
                    otherwise,
                    scale_config,
                    nested_band_config,
                    legend_config,
                    axis_config,
                    domain_coordination,
                    transform_scope,
                    scale_domain_inference,
                },
                ChannelValue::Value { .. } => ChannelValue::Conditional {
                    conditions,
                    otherwise,
                    scale_config: None,
                    nested_band_config: None,
                    legend_config: None,
                    axis_config: None,
                    domain_coordination: None,
                    transform_scope: None,
                    scale_domain_inference: ScaleDomainInference::Infer,
                },
                ChannelValue::Conditional { .. } => {
                    return Err(lowerer_error(
                        declaration,
                        "authored channel conditions cannot wrap a conditional transform output",
                    ));
                }
            };
        }
        self.apply_channel_configs(channel, &resolved.configuration, data, declaration)
    }

    fn raw_resolved_channel_branch(
        &self,
        branch: &avenger_lang_core::ResolvedChannelBranch,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<ChannelValue, Diagnostic> {
        match branch.mode {
            avenger_lang_core::ast::ChannelMode::Encoded => {
                self.raw_channel_value(&branch.expression, data, declaration)
            }
            avenger_lang_core::ast::ChannelMode::Direct => Ok(ChannelValue::from(
                self.channel_data_expr(&branch.expression, data, declaration)?,
            )
            .no_scale()),
        }
    }

    fn resolved_conditional_branch(
        &self,
        branch: &avenger_lang_core::ResolvedChannelBranch,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<ConditionalValue, Diagnostic> {
        let expression = self.channel_data_expr(&branch.expression, data, declaration)?;
        let expression = LogicalExprNode::from_default_expr(expression)
            .map_err(|error| lowerer_error(declaration, error.to_string()))?;
        Ok(match branch.mode {
            avenger_lang_core::ast::ChannelMode::Encoded => {
                ConditionalValue::Scaled { expr: expression }
            }
            avenger_lang_core::ast::ChannelMode::Direct => {
                ConditionalValue::Value { expr: expression }
            }
        })
    }

    fn channel_data_expr(
        &self,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<Expr, Diagnostic> {
        if let ResolvedValue::Expression(expression) = value
            && let Some(output) = self.direct_output(expression)
        {
            return match output {
                NativeOutputValue::Expr(expr) => Ok(expr.clone()),
                NativeOutputValue::Channel(channel) => Ok(channel.data_expr().clone()),
                NativeOutputValue::RasterDim(_) | NativeOutputValue::Opaque(_) => Err(
                    lowerer_error(declaration, "this transform output is not a channel value"),
                ),
            };
        }
        match value {
            ResolvedValue::String(value) => Ok(lit(value.clone())),
            ResolvedValue::Number(value) if value.parse::<i64>().is_ok() => {
                Ok(lit(value.parse::<i64>().expect("checked integer spelling")))
            }
            ResolvedValue::Number(value) => value.parse::<f64>().map(lit).map_err(|_| {
                lowerer_error(declaration, format!("invalid numeric literal `{value}`"))
            }),
            ResolvedValue::Boolean(value) => Ok(lit(*value)),
            ResolvedValue::Null => Ok(lit(ScalarValue::Null)),
            ResolvedValue::ChannelValue(channel) => {
                self.channel_data_expr(&channel.effective_fallback().expression, data, declaration)
            }
            ResolvedValue::Channel {
                expression: inner, ..
            } => self.channel_data_expr(inner, data, declaration),
            _ => self.expression_value(value, data, declaration),
        }
    }

    fn apply_channel_configs(
        &self,
        mut channel: ChannelValue,
        properties: &std::collections::BTreeMap<String, ResolvedValue>,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<ChannelValue, Diagnostic> {
        for (name, config) in properties {
            channel = match name.as_str() {
                "otherwise" => channel,
                "scale" => self.apply_scale(channel, config, data, declaration)?,
                "axis" => self.apply_axis(channel, config, data, declaration)?,
                "legend" => self.apply_legend(channel, config, data, declaration)?,
                "band" => channel.band(self.expression_value(config, data, declaration)?),
                "domain_contribution" => {
                    let inference = match resolved_atom(config) {
                        Some("infer") => ScaleDomainInference::Infer,
                        Some("exclude") => ScaleDomainInference::Exclude,
                        Some(value) => {
                            return Err(lowerer_error(
                                declaration,
                                format!(
                                    "`domain_contribution` must be `infer` or `exclude`, found `{value}`"
                                ),
                            ));
                        }
                        None => {
                            return Err(lowerer_error(
                                declaration,
                                "`domain_contribution` must be `infer` or `exclude`",
                            ));
                        }
                    };
                    channel.with_scale_domain_inference(inference)
                }
                other => {
                    return Err(lowerer_error(
                        declaration,
                        format!("unsupported channel configuration `{other}`"),
                    ));
                }
            };
        }
        Ok(channel)
    }

    fn raw_channel_value(
        &self,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<ChannelValue, Diagnostic> {
        if let ResolvedValue::Expression(expression) = value
            && let Some(output) = self.direct_output(expression)
        {
            return match output {
                NativeOutputValue::Expr(expr) => Ok(ChannelValue::from(expr.clone())),
                NativeOutputValue::Channel(channel) => Ok(channel.channel_value().clone()),
                NativeOutputValue::RasterDim(_) | NativeOutputValue::Opaque(_) => Err(
                    lowerer_error(declaration, "this transform output is not a channel value"),
                ),
            };
        }
        Ok(match value {
            ResolvedValue::String(value) => scaled_literal_channel(lit(value.clone())),
            ResolvedValue::Number(value) if value.parse::<i64>().is_ok() => {
                scaled_literal_channel(lit(value.parse::<i64>().unwrap()))
            }
            ResolvedValue::Number(value) => {
                scaled_literal_channel(lit(value.parse::<f64>().map_err(|_| {
                    lowerer_error(declaration, format!("invalid numeric literal `{value}`"))
                })?))
            }
            ResolvedValue::Boolean(value) => scaled_literal_channel(lit(*value)),
            ResolvedValue::Null => scaled_literal_channel(lit(ScalarValue::Null)),
            ResolvedValue::Channel { mode, expression } => match mode {
                avenger_lang_core::ast::ChannelMode::Encoded => {
                    self.raw_channel_value(expression, data, declaration)?
                }
                avenger_lang_core::ast::ChannelMode::Direct => {
                    ChannelValue::from(self.channel_data_expr(expression, data, declaration)?)
                        .no_scale()
                }
            },
            _ => ChannelValue::from(self.expression_value(value, data, declaration)?),
        })
    }

    fn apply_channel_conditions(
        &self,
        base: ChannelValue,
        children: &[ResolvedDeclaration],
        otherwise: Option<&ResolvedValue>,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<ChannelValue, Diagnostic> {
        let mut conditions = Vec::new();
        for child in children {
            if child.keyword.as_str() != "when" {
                continue;
            }
            let predicate = child.properties.get("predicate").ok_or_else(|| {
                lowerer_error(child, "conditional channel branch is missing `predicate:`")
            })?;
            let predicate = self.expression_value(predicate, data, child)?;
            let value = self.lower_conditional_branch(
                &ResolvedValue::Object {
                    head: None,
                    kind: None,
                    properties: child.properties.clone(),
                    children: Vec::new(),
                },
                data,
                child,
            )?;
            conditions.push((
                LogicalExprNode::from_default_expr(predicate)
                    .map_err(|error| lowerer_error(child, error.to_string()))?,
                value,
            ));
        }

        let otherwise = match otherwise {
            Some(value) => self.lower_conditional_branch(value, data, declaration)?,
            None => channel_fallback(&base).ok_or_else(|| {
                lowerer_error(
                    declaration,
                    "a conditional channel cannot use an already-conditional value as its fallback",
                )
            })?,
        };

        Ok(match base {
            ChannelValue::Scaled {
                scale_config,
                nested_band_config,
                legend_config,
                axis_config,
                domain_coordination,
                transform_scope,
                scale_domain_inference,
                ..
            } => ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                nested_band_config,
                legend_config,
                axis_config,
                domain_coordination,
                transform_scope,
                scale_domain_inference,
            },
            ChannelValue::Value { .. } => ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config: None,
                nested_band_config: None,
                legend_config: None,
                axis_config: None,
                domain_coordination: None,
                transform_scope: None,
                scale_domain_inference: ScaleDomainInference::Infer,
            },
            ChannelValue::Conditional { .. } => {
                return Err(lowerer_error(
                    declaration,
                    "authored channel conditions cannot wrap a conditional transform output",
                ));
            }
        })
    }

    fn lower_conditional_branch(
        &self,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<ConditionalValue, Diagnostic> {
        let ResolvedValue::Object { properties, .. } = value else {
            return Err(lowerer_error(
                declaration,
                "conditional channel branch must be a property block",
            ));
        };
        let (mode, value) = match (properties.get("encoded"), properties.get("direct")) {
            (Some(value), None) => (avenger_lang_core::ast::ChannelMode::Encoded, value),
            (None, Some(value)) => (avenger_lang_core::ast::ChannelMode::Direct, value),
            _ => {
                return Err(lowerer_error(
                    declaration,
                    "conditional channel branch requires exactly one of `encoded:` or `direct:`",
                ));
            }
        };
        let expression = self.channel_data_expr(value, data, declaration)?;
        let expression = LogicalExprNode::from_default_expr(expression)
            .map_err(|error| lowerer_error(declaration, error.to_string()))?;
        Ok(match mode {
            avenger_lang_core::ast::ChannelMode::Encoded => {
                ConditionalValue::Scaled { expr: expression }
            }
            avenger_lang_core::ast::ChannelMode::Direct => {
                ConditionalValue::Value { expr: expression }
            }
        })
    }

    fn apply_scale(
        &self,
        channel: ChannelValue,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<ChannelValue, Diagnostic> {
        if matches!(value, ResolvedValue::None) {
            return Ok(channel.no_scale());
        }
        let native = self.object_declaration(value, data, declaration, "linear")?;
        let key = NativeKindKey::new(NativeKindNamespace::Scale, native.kind.clone());
        let lowered = self
            .registry
            .lower_object(&key, &native)
            .map_err(|error| lowerer_error(declaration, error.to_string()))?;
        let scale = lowered.downcast::<Scale<Auto>>().map_err(|_| {
            lowerer_error(
                declaration,
                "registered scale lowerer did not return Scale<Auto>",
            )
        })?;
        Ok(channel.scale(move |_| (*scale).clone()))
    }

    fn apply_axis(
        &self,
        channel: ChannelValue,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<ChannelValue, Diagnostic> {
        if matches!(value, ResolvedValue::None) {
            return Ok(channel);
        }
        let native = self.object_declaration(value, data, declaration, "cartesian")?;
        let key = NativeKindKey::new(NativeKindNamespace::Axis, native.kind.clone());
        let lowered = self
            .registry
            .lower_object(&key, &native)
            .map_err(|error| lowerer_error(declaration, error.to_string()))?;
        let axis = lowered
            .downcast::<Box<dyn avenger_chart_core::Axis>>()
            .map_err(|_| {
                lowerer_error(
                    declaration,
                    "registered axis lowerer did not return Box<dyn Axis>",
                )
            })?;
        Ok(channel.with_boxed_axis_config(*axis))
    }

    fn apply_legend(
        &self,
        channel: ChannelValue,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<ChannelValue, Diagnostic> {
        if matches!(value, ResolvedValue::None) {
            return Ok(channel.no_legend());
        }
        let overlay_id = legend_overlay_id(value);
        let mut native = self.object_declaration(value, data, declaration, "standard")?;
        native.properties.shift_remove("overlay");
        let key = NativeKindKey::new(NativeKindNamespace::Legend, native.kind.clone());
        let lowered = self
            .registry
            .lower_object(&key, &native)
            .map_err(|error| lowerer_error(declaration, error.to_string()))?;
        let mut legend = *lowered
            .downcast::<avenger_chart_core::Legend>()
            .map_err(|_| {
                lowerer_error(
                    declaration,
                    "registered legend lowerer did not return Legend",
                )
            })?;
        if let Some(overlay_id) = overlay_id {
            let overlay = self.legend_overlays.get(overlay_id).ok_or_else(|| {
                lowerer_error(
                    declaration,
                    "resolved legend overlay was not prepared before mark lowering",
                )
            })?;
            legend = legend.colorbar_overlay_any(Arc::new(overlay.clone()));
        }
        Ok(channel.legend(legend))
    }

    fn object_declaration(
        &self,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
        default_kind: &str,
    ) -> Result<NativeDeclaration, Diagnostic> {
        let (kind, properties) = match value {
            ResolvedValue::Atom(kind) => (kind.clone(), None),
            ResolvedValue::Object {
                kind, properties, ..
            } => (
                kind.clone().unwrap_or_else(|| default_kind.to_string()),
                Some(properties),
            ),
            _ => {
                return Err(lowerer_error(
                    declaration,
                    "configuration must be a kind atom or property block",
                ));
            }
        };
        let mut native = NativeDeclaration::new(kind);
        if let Some(properties) = properties {
            for (name, value) in properties {
                native
                    .properties
                    .insert(name.clone(), self.native_value(value, data, declaration)?);
            }
        }
        Ok(native)
    }

    fn expression_value(
        &self,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<Expr, Diagnostic> {
        if let ResolvedValue::Expression(expression) = value
            && let Some(output) = self.direct_output(expression)
        {
            return match output {
                NativeOutputValue::Expr(expr) => Ok(expr.clone()),
                NativeOutputValue::Channel(channel) => Ok(channel.data_expr().clone()),
                NativeOutputValue::RasterDim(_) | NativeOutputValue::Opaque(_) => Err(
                    lowerer_error(declaration, "this transform output is not a SQL expression"),
                ),
            };
        }
        match value {
            ResolvedValue::Atom(value) if value.eq_ignore_ascii_case("null") => {
                Ok(lit(ScalarValue::Null))
            }
            ResolvedValue::String(value) | ResolvedValue::Atom(value) => Ok(lit(value.clone())),
            ResolvedValue::Number(value) if value.parse::<i64>().is_ok() => {
                Ok(lit(value.parse::<i64>().expect("checked integer spelling")))
            }
            ResolvedValue::Number(value) => value.parse::<f64>().map(lit).map_err(|_| {
                lowerer_error(declaration, format!("invalid numeric literal `{value}`"))
            }),
            ResolvedValue::Boolean(value) => Ok(lit(*value)),
            ResolvedValue::Null => Ok(lit(ScalarValue::Null)),
            ResolvedValue::Column(name) => {
                let data = data.ok_or_else(|| {
                    lowerer_error(
                        declaration,
                        format!("column `{name}` has no data schema in scope"),
                    )
                })?;
                data.schema()
                    .field_with_unqualified_name(name)
                    .map_err(|error| lowerer_error(declaration, error.to_string()))?;
                Ok(unqualified_column(name))
            }
            ResolvedValue::Expression(expression) => {
                self.planned_expression(expression, data, declaration)
            }
            ResolvedValue::Binding(binding) => self.binding_expr(binding, declaration),
            ResolvedValue::Call { function, args } => {
                let function = self
                    .context
                    .udf(function)
                    .map_err(|error| lowerer_error(declaration, error.to_string()))?;
                let args = args
                    .iter()
                    .map(|arg| self.expression_value(arg, data, declaration))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(function.call(args))
            }
            ResolvedValue::Channel {
                expression: inner, ..
            } => self.expression_value(inner, data, declaration),
            _ => Err(lowerer_error(
                declaration,
                format!("value `{value:?}` is not a SQL expression"),
            )),
        }
    }

    fn event_expression_value(
        &self,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<Expr, Diagnostic> {
        match value {
            ResolvedValue::Expression(expression) => {
                self.planned_event_expression(expression, data, declaration)
            }
            ResolvedValue::Binding(binding) => self.event_binding_expr(binding, declaration),
            ResolvedValue::Call { function, args } => {
                if matches!(function.as_str(), "span" | "span_ordered") {
                    let [left, right] = args.as_slice() else {
                        return Err(lowerer_error(
                            declaration,
                            format!("event helper `{function}` requires two arguments"),
                        ));
                    };
                    let left = self.event_expression_value(left, data, declaration)?;
                    let right = self.event_expression_value(right, data, declaration)?;
                    return Ok(if function == "span" {
                        event::interval(left, right)
                    } else {
                        event::interval_ordered(left, right)
                    });
                }
                self.expression_value(value, data, declaration)
            }
            _ => self.expression_value(value, data, declaration),
        }
    }

    /// Lower the source side of a declared physical boundary while retaining
    /// exact structural numeric spellings. Untyped native/channel values keep
    /// their established integer-or-f64 representation.
    fn boundary_source_value(
        &self,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
        event: bool,
    ) -> Result<Expr, Diagnostic> {
        match value {
            ResolvedValue::Number(value) => exact_numeric_scalar(value)
                .map(lit)
                .map_err(|error| lowerer_error(declaration, error)),
            ResolvedValue::Expression(expression) if event => {
                self.planned_event_expression(expression, data, declaration)
            }
            ResolvedValue::Expression(expression) => {
                self.planned_expression(expression, data, declaration)
            }
            ResolvedValue::Binding(binding) if event => {
                self.event_binding_expr(binding, declaration)
            }
            ResolvedValue::Call { function, args }
                if event && matches!(function.as_str(), "span" | "span_ordered") =>
            {
                let [left, right] = args.as_slice() else {
                    return Err(lowerer_error(
                        declaration,
                        format!("event helper `{function}` requires two arguments"),
                    ));
                };
                let left = self.boundary_source_value(left, data, declaration, true)?;
                let right = self.boundary_source_value(right, data, declaration, true)?;
                Ok(if function == "span" {
                    event::interval(left, right)
                } else {
                    event::interval_ordered(left, right)
                })
            }
            ResolvedValue::Call { function, args } => {
                let function = self
                    .context
                    .udf(function)
                    .map_err(|error| lowerer_error(declaration, error.to_string()))?;
                let args = args
                    .iter()
                    .map(|arg| self.boundary_source_value(arg, data, declaration, event))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(function.call(args))
            }
            ResolvedValue::Channel {
                expression: inner, ..
            } => self.boundary_source_value(inner, data, declaration, event),
            _ if event => self.event_expression_value(value, data, declaration),
            _ => self.expression_value(value, data, declaration),
        }
    }

    fn event_binding_expr(
        &self,
        binding: &ResolvedBinding,
        declaration: &ResolvedDeclaration,
    ) -> Result<Expr, Diagnostic> {
        let ResolvedTarget::Param(id) = &binding.target else {
            return Err(lowerer_error(
                declaration,
                "event scalar expressions require parameter bindings",
            ));
        };
        let param = self
            .params
            .get(id)
            .ok_or_else(|| lowerer_error(declaration, "resolved parameter is unavailable"))?;
        Ok(match binding.time {
            BindingTime::Current => param.expr(),
            BindingTime::Start => event::start_param(param),
            BindingTime::Previous => event::previous_param(param),
        })
    }

    fn planned_event_expression(
        &self,
        expression: &ResolvedExpression,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<Expr, Diagnostic> {
        let mut parse_data = match data {
            Some(data) => data.clone(),
            None => self
                .context
                .read_batch(RecordBatch::new_empty(Arc::new(Schema::new(vec![
                    Field::new("__avenger_event_parse", DataType::Boolean, true),
                ]))))
                .map_err(|error| lowerer_error(declaration, error.to_string()))?,
        };
        let mut sql = expression.sql.clone();
        let mut replacements = BTreeMap::new();
        let mut store_scan_targets = BTreeMap::new();
        let mut source_replacements = Vec::<(String, String)>::new();

        for (index, binding) in expression.bindings.iter().enumerate() {
            let authored = binding_spelling(binding);
            if let ResolvedTarget::Store(id) = &binding.target {
                if binding.time != BindingTime::Current {
                    return Err(lowerer_error(
                        declaration,
                        "store relation bindings do not support temporal qualifiers",
                    ));
                }
                let table_name = self.store_table_names.get(id).ok_or_else(|| {
                    lowerer_error(
                        declaration,
                        "resolved store query placeholder is unavailable",
                    )
                })?;
                let store = self
                    .stores
                    .get(id)
                    .ok_or_else(|| lowerer_error(declaration, "resolved store is unavailable"))?;
                let replacement = format!("\"{}\"", table_name.replace('"', "\"\""));
                sql = sql.replace(&authored, &replacement);
                source_replacements.push((authored, replacement));
                store_scan_targets.insert(table_name.clone(), store.name.clone());
                continue;
            }

            let synthetic = format!("__avenger_event_binding_{index:08}");
            let replacement = format!("\"{synthetic}\"");
            sql = sql.replace(&authored, &replacement);
            let param = match &binding.target {
                ResolvedTarget::Param(id) => self.params.get(id).ok_or_else(|| {
                    lowerer_error(declaration, "resolved parameter is unavailable")
                })?,
                _ => {
                    return Err(lowerer_error(
                        declaration,
                        "event scalar bindings must reference params or stores",
                    ));
                }
            };
            parse_data = parse_data
                .with_column(&synthetic, lit(param.default.clone()))
                .map_err(|error| lowerer_error(declaration, error.to_string()))?;
            replacements.insert(
                synthetic,
                match binding.time {
                    BindingTime::Current => param.expr(),
                    BindingTime::Start => event::start_param(param),
                    BindingTime::Previous => event::previous_param(param),
                },
            );
            source_replacements.push((authored, replacement));
        }

        let contextual_replacements = expression
            .contextual_accesses
            .iter()
            .enumerate()
            .map(|(index, access)| {
                (
                    contextual_access_spelling(access),
                    format!("__avenger_contextual_access_{index:08}"),
                )
            })
            .collect::<BTreeMap<_, _>>();
        if !contextual_replacements.is_empty() {
            let mut parsed = SqlExpression::parse(&sql)
                .map_err(|error| lowerer_error(declaration, error.to_string()))?;
            parsed.rewrite_expression_nodes(|candidate| {
                contextual_replacements.get(&candidate.to_string()).cloned()
            });
            sql = parsed.canonical_sql();
            for access in &expression.contextual_accesses {
                let authored = contextual_access_spelling(access);
                let synthetic = contextual_replacements.get(&authored).ok_or_else(|| {
                    lowerer_error(declaration, "resolved contextual access is unavailable")
                })?;
                let (runtime_expr, seed) =
                    self.contextual_access_expr(access, data, declaration)?;
                parse_data = parse_data
                    .with_column(synthetic, lit(seed.clone()))
                    .map_err(|error| lowerer_error(declaration, error.to_string()))?;
                replacements.insert(synthetic.clone(), runtime_expr);
                source_replacements
                    .push((authored, format!("\"{}\"", synthetic.replace('"', "\"\""))));
            }
        }

        // sqlparser reports helper calls in pre-order. Lower them in reverse so
        // nested calls become synthetic columns before their parents are
        // planned. Parent SQL arguments can then refer to those columns.
        for (index, helper) in expression.helpers.iter().enumerate().rev() {
            let authored = helper_spelling(helper, declaration)?;
            let current = rewrite_source_fragment(&authored, &source_replacements);
            let (expr, seed) = self.event_helper_expr_with_context(
                helper,
                data,
                &parse_data,
                &source_replacements,
                declaration,
            )?;
            let synthetic = format!("__avenger_event_helper_{index:08}");
            let replacement = format!("\"{synthetic}\"");
            let rewritten = sql.replacen(&current, &replacement, 1);
            if rewritten == sql {
                return Err(lowerer_error(
                    declaration,
                    format!("could not rewrite resolved event helper `{authored}`"),
                ));
            }
            sql = rewritten;
            parse_data = parse_data
                .with_column(&synthetic, lit(seed))
                .map_err(|error| lowerer_error(declaration, error.to_string()))?;
            replacements.insert(synthetic, expr);
            source_replacements.push((authored, replacement));
        }
        let sql =
            normalize_sql_expression(&sql).map_err(|error| lowerer_error(declaration, error))?;
        let parsed = if store_scan_targets.is_empty() {
            self.context
                .parse_sql_expr(&sql, parse_data.schema())
                .map_err(|error| lowerer_error(declaration, error.to_string()))?
        } else {
            self.plan_event_expression_with_relations(&sql, &parse_data, declaration)?
        };
        let parsed = parsed
            .transform_up(|candidate| {
                if let Expr::Column(column) = &candidate
                    && let Some(replacement) = replacements.get(&column.name)
                {
                    return Ok(Transformed::yes(replacement.clone()));
                }
                Ok(Transformed::no(candidate))
            })
            .map(|result| result.data)
            .map_err(|error: datafusion::error::DataFusionError| {
                lowerer_error(declaration, error.to_string())
            })?;
        rewrite_store_subquery_targets(parsed, &store_scan_targets)
            .map_err(|error| lowerer_error(declaration, error.to_string()))
    }

    /// DataFusion's expression-only SQL planner intentionally has no relation
    /// providers, so it cannot plan a scalar subquery over a store. Plan the
    /// same expression as a one-column `SELECT` over a schema-only outer table,
    /// then recover the projection expression. No rows are read at compile
    /// time; the store scan is retargeted to its runtime relation immediately
    /// afterward.
    fn plan_event_expression_with_relations(
        &self,
        sql: &str,
        parse_data: &DataFrame,
        declaration: &ResolvedDeclaration,
    ) -> Result<Expr, Diagnostic> {
        use sha2::{Digest, Sha256};

        let fingerprint = Sha256::digest(format!("{sql}\n{:?}", parse_data.schema()).as_bytes());
        let outer_name = format!("__avenger_event_outer_{fingerprint:x}");
        self.context
            .register_table(
                &outer_name,
                Arc::new(EmptyTable::new(Arc::clone(parse_data.schema().inner()))),
            )
            .map_err(|error| lowerer_error(declaration, error.to_string()))?;

        let query = format!("SELECT {sql} FROM \"{}\"", outer_name.replace('"', "\"\""));
        let state = self.context.state();
        let plan = futures::executor::block_on(state.create_logical_plan(&query))
            .map_err(|error| lowerer_error(declaration, error.to_string()))?;
        let LogicalPlan::Projection(projection) = plan else {
            return Err(lowerer_error(
                declaration,
                "event expression query did not lower to a projection",
            ));
        };
        let [expression] = projection.expr.as_slice() else {
            return Err(lowerer_error(
                declaration,
                "event expression query did not produce exactly one expression",
            ));
        };
        expression
            .clone()
            .unalias()
            .transform_up(|candidate| {
                let Expr::Column(column) = candidate else {
                    return Ok(Transformed::no(candidate));
                };
                if column
                    .relation
                    .as_ref()
                    .is_some_and(|relation| relation.table() == outer_name)
                {
                    return Ok(Transformed::yes(Expr::Column(Column::new_unqualified(
                        column.name,
                    ))));
                }
                Ok(Transformed::no(Expr::Column(column)))
            })
            .map(|transformed| transformed.data)
            .map_err(|error: datafusion::error::DataFusionError| {
                lowerer_error(declaration, error.to_string())
            })
    }

    fn contextual_access_expr(
        &self,
        access: &ResolvedContextualAccess,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<(Expr, ScalarValue), Diagnostic> {
        let utf8 = || ScalarValue::Utf8(None);
        let fixed = || fixed_contextual_planning_seed(&access.kind, declaration);
        let channel_name = |channel: &ResolvedChannelMember| channel.authored_name();
        let result = match &access.kind {
            ResolvedContextualAccessKind::DatumField { field } => {
                let seed = data
                    .and_then(|data| data.schema().field_with_unqualified_name(field).ok())
                    .and_then(|field| ScalarValue::try_new_null(field.data_type()).ok())
                    .unwrap_or_else(utf8);
                (event::datum(field), seed)
            }
            ResolvedContextualAccessKind::MarkChannel { channel } => {
                let name = channel_name(channel);
                (
                    col(format!(":{name}")),
                    self.mark_channel_planning_seed(&name, data, declaration)?,
                )
            }
            ResolvedContextualAccessKind::EventCoord { channel } => {
                (event::event_coord(&channel_name(channel)), fixed()?)
            }
            ResolvedContextualAccessKind::EventStartCoord { channel } => {
                (event::start_coord(&channel_name(channel)), fixed()?)
            }
            ResolvedContextualAccessKind::EventDomainBoundary { channel, boundary } => {
                let domain = event::event_domain(&channel_name(channel));
                let expression = match boundary {
                    ResolvedIntervalBoundary::Start => event::interval_start(domain),
                    ResolvedIntervalBoundary::End => event::interval_end(domain),
                };
                (expression, fixed()?)
            }
            ResolvedContextualAccessKind::EventPath => (event::event_path(), fixed()?),
            ResolvedContextualAccessKind::EventFacet { one_based_index } => (
                event::event_facet_value((*one_based_index - 1) as usize),
                fixed()?,
            ),
            ResolvedContextualAccessKind::EventLegendValue => (event::legend_value(), fixed()?),
            ResolvedContextualAccessKind::ItemChannel {
                channel,
                physical_type,
            } => {
                let seed = ScalarValue::try_new_null(&physical_type_to_arrow(physical_type))
                    .map_err(|error| lowerer_error(declaration, error.to_string()))?;
                (col(item_channel_column_name(&channel_name(channel))), seed)
            }
            ResolvedContextualAccessKind::ItemDataField { field } => {
                let seed = data
                    .and_then(|data| data.schema().field_with_unqualified_name(field).ok())
                    .and_then(|field| ScalarValue::try_new_null(field.data_type()).ok())
                    .unwrap_or_else(utf8);
                (col(item_data_column_name(field)), seed)
            }
            ResolvedContextualAccessKind::ItemBbox { edge } => {
                (col(item_bbox_column_name(edge.as_str())), fixed()?)
            }
            ResolvedContextualAccessKind::ViewField {
                target: ResolvedTarget::Declaration(view_id),
                axis,
                field,
                ..
            } => {
                let view = self.view_refs.get(view_id).ok_or_else(|| {
                    lowerer_error(declaration, "resolved inline view is unavailable")
                })?;
                let axis = match axis {
                    ResolvedViewAxis::X => view.x(),
                    ResolvedViewAxis::Y => view.y(),
                };
                match field {
                    ResolvedViewField::DomainStart => (axis.domain_start(), fixed()?),
                    ResolvedViewField::DomainEnd => (axis.domain_end(), fixed()?),
                    ResolvedViewField::Pixels => (axis.pixels(), fixed()?),
                }
            }
            ResolvedContextualAccessKind::ViewField { .. } => {
                return Err(lowerer_error(
                    declaration,
                    "resolved inline-view access has an invalid target",
                ));
            }
        };
        Ok(result)
    }

    fn mark_channel_planning_seed(
        &self,
        channel: &str,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<ScalarValue, Diagnostic> {
        let Some(data) = data else {
            return Ok(ScalarValue::Float64(None));
        };
        let key = (declaration.id.clone(), channel.to_owned());
        if !self
            .mark_channel_seed_stack
            .borrow_mut()
            .insert(key.clone())
        {
            // Runtime channel resolution owns the user-facing cycle diagnostic.
            // A temporary numeric seed lets lowering reach that validation
            // without recursively planning the same dependency forever.
            return Ok(ScalarValue::Float64(None));
        }

        let result = (|| {
            let value = declaration.properties.get(channel).ok_or_else(|| {
                lowerer_error(
                    declaration,
                    format!("referenced channel `{channel}` is unavailable for type planning"),
                )
            })?;
            let expression = self.channel_data_expr(value, Some(data), declaration)?;
            let data_type = expression
                .get_type(data.schema())
                .map_err(|error| lowerer_error(declaration, error.to_string()))?;
            ScalarValue::try_new_null(&data_type)
                .map_err(|error| lowerer_error(declaration, error.to_string()))
        })();
        self.mark_channel_seed_stack.borrow_mut().remove(&key);
        result
    }

    fn event_helper_expr_with_context(
        &self,
        helper: &avenger_lang_core::ResolvedHelper,
        _data: Option<&DataFrame>,
        parse_data: &DataFrame,
        source_replacements: &[(String, String)],
        declaration: &ResolvedDeclaration,
    ) -> Result<(Expr, ScalarValue), Diagnostic> {
        let boolean = || ScalarValue::Boolean(None);
        let list = || ScalarValue::new_null_list(DataType::Float64, true, 1);
        let result = match (helper.name.as_str(), helper.arguments.as_slice()) {
            (
                "selection_contains",
                [
                    ResolvedHelperArgument::Target {
                        target: ResolvedTarget::Selection(selection_id),
                        ..
                    },
                    ResolvedHelperArgument::DatumField(field),
                ],
            ) => {
                let selection = self.selections.get(selection_id).ok_or_else(|| {
                    lowerer_error(declaration, "resolved selection is unavailable")
                })?;
                (
                    selection
                        .contains_equality_value(unqualified_column(field), event::datum(field)),
                    boolean(),
                )
            }
            ("span", [left, right]) => (
                event::interval(
                    helper_argument_expr(left, parse_data, source_replacements, declaration)?,
                    helper_argument_expr(right, parse_data, source_replacements, declaration)?,
                ),
                list(),
            ),
            ("span_ordered", [left, right]) => (
                event::interval_ordered(
                    helper_argument_expr(left, parse_data, source_replacements, declaration)?,
                    helper_argument_expr(right, parse_data, source_replacements, declaration)?,
                ),
                list(),
            ),
            _ => {
                return Err(lowerer_error(
                    declaration,
                    format!("event helper `{}` is not lowered yet", helper.name),
                ));
            }
        };
        Ok(result)
    }

    fn binding_expr(
        &self,
        binding: &ResolvedBinding,
        declaration: &ResolvedDeclaration,
    ) -> Result<Expr, Diagnostic> {
        if binding.time != BindingTime::Current {
            return Err(lowerer_error(
                declaration,
                "temporal parameter reads are only lowered with event bindings",
            ));
        }
        match &binding.target {
            ResolvedTarget::Param(id) => self
                .params
                .get(id)
                .map(ChartParam::expr)
                .ok_or_else(|| lowerer_error(declaration, "resolved parameter is unavailable")),
            ResolvedTarget::Selection(id) => self
                .selections
                .get(id)
                .map(Selection::predicate)
                .ok_or_else(|| lowerer_error(declaration, "resolved selection is unavailable")),
            ResolvedTarget::DefinitionSelection { .. } => Err(lowerer_error(
                declaration,
                "definition-owned selection predicates require expansion before lowering",
            )),
            _ => Err(lowerer_error(
                declaration,
                "table bindings are not scalar expressions",
            )),
        }
    }

    fn planned_expression(
        &self,
        expression: &ResolvedExpression,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<Expr, Diagnostic> {
        if !expression.helpers.is_empty() {
            return Err(lowerer_error(
                declaration,
                "this reserved SQL helper is not valid in a native data expression",
            ));
        }
        if expression.contextual_accesses.iter().any(|access| {
            !matches!(
                &access.kind,
                ResolvedContextualAccessKind::MarkChannel { .. }
                    | ResolvedContextualAccessKind::ViewField { .. }
            )
        }) {
            return Err(lowerer_error(
                declaration,
                "this contextual access is not valid in a native data expression",
            ));
        }
        let data = data.ok_or_else(|| {
            lowerer_error(declaration, "SQL expression has no data schema in scope")
        })?;
        let mut sql = expression.sql.clone();
        let mut parse_data = data.clone();
        let mut replacements = BTreeMap::new();
        let contextual_replacements = expression
            .contextual_accesses
            .iter()
            .enumerate()
            .map(|(index, access)| {
                (
                    contextual_access_spelling(access),
                    format!("__avenger_contextual_access_{index:08}"),
                )
            })
            .collect::<BTreeMap<_, _>>();
        if !contextual_replacements.is_empty() {
            let mut parsed = SqlExpression::parse(&sql)
                .map_err(|error| lowerer_error(declaration, error.to_string()))?;
            parsed.rewrite_expression_nodes(|candidate| {
                contextual_replacements.get(&candidate.to_string()).cloned()
            });
            sql = parsed.canonical_sql();
        }
        for access in &expression.contextual_accesses {
            let authored = contextual_access_spelling(access);
            let synthetic = contextual_replacements.get(&authored).ok_or_else(|| {
                lowerer_error(declaration, "resolved contextual access is unavailable")
            })?;
            let (expr, seed) = self.contextual_access_expr(access, Some(data), declaration)?;
            parse_data = parse_data
                .with_column(synthetic, lit(seed))
                .map_err(|error| lowerer_error(declaration, error.to_string()))?;
            replacements.insert(synthetic.clone(), expr);
        }
        for (index, reference) in expression.references.iter().enumerate() {
            let (expr, planning_seed) = match &reference.target {
                ResolvedTarget::Output(handle) => {
                    let output = self.transform_outputs.get(handle).ok_or_else(|| {
                        lowerer_error(
                            declaration,
                            format!(
                                "transform output `{}` is unavailable at this dataflow position",
                                reference.authored_path.join(".")
                            ),
                        )
                    })?;
                    let expr = match output {
                        NativeOutputValue::Expr(expr) => expr.clone(),
                        NativeOutputValue::Channel(channel) => channel.data_expr().clone(),
                        NativeOutputValue::RasterDim(_) | NativeOutputValue::Opaque(_) => {
                            return Err(lowerer_error(
                                declaration,
                                format!(
                                    "transform output `{}` cannot be used in a SQL expression",
                                    reference.authored_path.join(".")
                                ),
                            ));
                        }
                    };
                    (expr.clone(), expr)
                }
                ResolvedTarget::Reserved { namespace, path } if namespace == "repeat" => {
                    repeat_reference_expr(path).ok_or_else(|| {
                        lowerer_error(
                            declaration,
                            format!(
                                "unsupported repeat reference `{}`",
                                reference.authored_path.join(".")
                            ),
                        )
                    })?
                }
                _ => continue,
            };
            let synthetic = format!("__avenger_output_{index:08}");
            sql = rewrite_reference_sql(sql, reference, &synthetic);
            parse_data = parse_data
                .with_column(&synthetic, planning_seed)
                .map_err(|error| lowerer_error(declaration, error.to_string()))?;
            replacements.insert(synthetic, expr);
        }
        for (index, binding) in expression.bindings.iter().enumerate() {
            let synthetic = format!("__avenger_binding_{index:08}");
            let authored = binding_spelling(binding);
            sql = sql.replace(&authored, &format!("\"{synthetic}\""));
            let expr = self.binding_expr(binding, declaration)?;
            parse_data = parse_data
                .with_column(&synthetic, expr.clone())
                .map_err(|error| lowerer_error(declaration, error.to_string()))?;
            replacements.insert(synthetic, expr);
        }
        let sql =
            normalize_sql_expression(&sql).map_err(|error| lowerer_error(declaration, error))?;
        let parsed = parse_data
            .parse_sql_expr(&sql)
            .map_err(|error| lowerer_error(declaration, error.to_string()))?;
        parsed
            .transform_up(|candidate| {
                if let Expr::Column(column) = &candidate
                    && let Some(replacement) = replacements.get(&column.name)
                {
                    return Ok(Transformed::yes(replacement.clone()));
                }
                Ok(Transformed::no(candidate))
            })
            .map(|result| result.data)
            .map_err(|error: datafusion::error::DataFusionError| {
                lowerer_error(declaration, error.to_string())
            })
    }

    fn query_sql(
        &self,
        query: &ResolvedQuery,
        declaration: &ResolvedDeclaration,
    ) -> Result<String, Diagnostic> {
        if !query.helpers.is_empty() {
            return Err(lowerer_error(
                declaration,
                "reserved helper calls are not valid in SQL transform queries",
            ));
        }
        let mut sql = query.sql.clone();
        for reference in &query.references {
            let ResolvedTarget::Output(handle) = &reference.target else {
                return Err(lowerer_error(
                    declaration,
                    format!(
                        "reference `{}` is not a relation-column output",
                        reference.authored_path.join(".")
                    ),
                ));
            };
            let output = self.transform_outputs.get(handle).ok_or_else(|| {
                lowerer_error(
                    declaration,
                    format!(
                        "transform output `{}` is unavailable at this dataflow position",
                        reference.authored_path.join(".")
                    ),
                )
            })?;
            if !matches!(
                output,
                NativeOutputValue::Expr(_) | NativeOutputValue::Channel(_)
            ) {
                return Err(lowerer_error(
                    declaration,
                    format!(
                        "transform output `{}` is not a SQL column",
                        reference.authored_path.join(".")
                    ),
                ));
            }
            sql = rewrite_reference_sql(sql, reference, &handle.name);
        }
        for binding in &query.bindings {
            if binding.time != BindingTime::Current {
                return Err(lowerer_error(
                    declaration,
                    "temporal parameter reads are only valid in event expressions",
                ));
            }
            let ResolvedTarget::Param(id) = &binding.target else {
                return Err(lowerer_error(
                    declaration,
                    "SQL query bindings must reference scalar parameters",
                ));
            };
            let param = self.params.get(id).ok_or_else(|| {
                lowerer_error(declaration, "resolved SQL parameter is unavailable")
            })?;
            sql = sql.replace(&binding_spelling(binding), &format!("${}", param.name));
        }
        crate::catalog::ensure_query_relations_registered(self.context, query)
            .map_err(|error| lowerer_error(declaration, error))?;
        let sql = crate::catalog::expand_chart_sql(self.project, &self.param_types, query, &sql)
            .map_err(|error| lowerer_error(declaration, error))?;
        normalize_sql_query(&sql).map_err(|error| lowerer_error(declaration, error))
    }

    fn direct_output(&self, expression: &ResolvedExpression) -> Option<&NativeOutputValue> {
        if !expression.bindings.is_empty()
            || !expression.helpers.is_empty()
            || expression.references.len() != 1
        {
            return None;
        }
        let reference = expression.references.first()?;
        if !is_direct_reference_sql(&expression.sql, &reference.authored_path) {
            return None;
        }
        let ResolvedTarget::Output(handle) = &reference.target else {
            return None;
        };
        self.transform_outputs.get(handle)
    }

    async fn lower_widget_items(
        &self,
        value: &ResolvedValue,
        declaration: &ResolvedDeclaration,
    ) -> Result<(WidgetItems, DataFrame), Diagnostic> {
        let data = self.lower_data(value, declaration).await?;
        if let ResolvedValue::Object { properties, .. } = value
            && let Some(ResolvedValue::Array(rows)) = properties.get("values")
        {
            let mut items = Vec::with_capacity(rows.len());
            for row in rows {
                let ResolvedValue::Object { properties, .. } = row else {
                    return Err(lowerer_error(
                        declaration,
                        "widget inline data rows must be anonymous objects",
                    ));
                };
                let values = properties
                    .iter()
                    .map(|(name, value)| {
                        Ok((
                            name.clone(),
                            widget_item_scalar(value).map_err(|message| {
                                lowerer_error(
                                    declaration,
                                    format!("widget data field `{name}` {message}"),
                                )
                            })?,
                        ))
                    })
                    .collect::<Result<Vec<_>, Diagnostic>>()?;
                items.push(WidgetItemRow::new(values));
            }
            return Ok((WidgetItems::Static(items), data));
        }

        let order_key = declaration
            .properties
            .get("order_by")
            .map(|value| match value {
                ResolvedValue::Array(values) => values
                    .iter()
                    .map(|value| self.expression_value(value, Some(&data), declaration))
                    .collect::<Result<Vec<_>, _>>(),
                value => self
                    .expression_value(value, Some(&data), declaration)
                    .map(|value| vec![value]),
            })
            .transpose()?
            .unwrap_or_default();
        if order_key.is_empty() {
            return Err(lowerer_error(
                declaration,
                "non-inline widget data requires a nonempty total `order_by` expression list",
            ));
        }
        Ok((
            WidgetItems::DataFrame {
                data: data.clone(),
                order_key,
            },
            data,
        ))
    }

    fn store_data_binding(
        &self,
        value: &ResolvedValue,
        declaration: &ResolvedDeclaration,
    ) -> Result<Option<(DataFrame, StoreData)>, Diagnostic> {
        let ResolvedValue::Binding(ResolvedBinding {
            target: ResolvedTarget::Store(id),
            time: BindingTime::Current,
            ..
        }) = value
        else {
            return Ok(None);
        };
        let store = self
            .stores
            .get(id)
            .ok_or_else(|| lowerer_error(declaration, "resolved store is unavailable"))?;
        let schema = Arc::new(Schema::new(
            store
                .fields
                .iter()
                .map(avenger_chart_core::StoreFieldSpec::to_field_ref)
                .collect::<Vec<_>>(),
        ));
        let planning = self
            .context
            .read_batch(RecordBatch::new_empty(schema))
            .map_err(|error| lowerer_error(declaration, error.to_string()))?;
        Ok(Some((planning, StoreData::new(store.name.clone()))))
    }

    async fn lower_data(
        &self,
        value: &ResolvedValue,
        declaration: &ResolvedDeclaration,
    ) -> Result<DataFrame, Diagnostic> {
        match value {
            ResolvedValue::Binding(ResolvedBinding {
                target: ResolvedTarget::Store(_),
                ..
            }) => Err(lowerer_error(
                declaration,
                "store-backed mark data requires the event-runtime store relation seam",
            )),
            ResolvedValue::Object { properties, .. } => {
                let sources = ["values", "table", "sql"]
                    .into_iter()
                    .filter(|name| properties.contains_key(*name))
                    .collect::<Vec<_>>();
                if sources.len() != 1 {
                    return Err(lowerer_error(
                        declaration,
                        "data block must contain exactly one of `values`, `table`, or `sql`",
                    ));
                }
                match sources[0] {
                    "values" => self.inline_values(&properties["values"], declaration).await,
                    "table" => {
                        let ResolvedValue::Relation(relation) = &properties["table"] else {
                            return Err(lowerer_error(
                                declaration,
                                "data table must be a relation path",
                            ));
                        };
                        let authored_path = relation.authored_path.clone();
                        let table = authored_path.join(".");
                        let ResolvedRelationTarget::Relation(relation_id) = &relation.target else {
                            return Err(lowerer_error(
                                declaration,
                                "`input` cannot be used as a chart data source",
                            ));
                        };
                        let arguments = properties
                            .iter()
                            .filter(|(name, _)| *name != "table")
                            .map(|(name, value)| {
                                self.table_binding_sql(value, declaration)
                                    .map(|value| format!("{name} => {value}"))
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        if arguments.is_empty() {
                            crate::catalog::ensure_relation_registered(
                                self.context,
                                relation_id,
                                &authored_path,
                            )
                            .map_err(|error| lowerer_error(declaration, error))?;
                            self.context
                                .table(crate::catalog::internal_relation_name(relation_id))
                                .await
                                .map_err(|error| lowerer_error(declaration, error.to_string()))
                        } else {
                            let sql = format!("SELECT * FROM {table}({})", arguments.join(", "));
                            let mut invocation = ResolvedQuery {
                                sql: sql.clone(),
                                bindings: Vec::new(),
                                helpers: Vec::new(),
                                references: Vec::new(),
                                relations: vec![relation.clone()],
                            };
                            invocation.relations[0].authored_path = authored_path;
                            let sql = crate::catalog::expand_chart_sql(
                                self.project,
                                &self.param_types,
                                &invocation,
                                &sql,
                            )
                            .map_err(|error| lowerer_error(declaration, error))?;
                            self.context
                                .sql(&sql)
                                .await
                                .map_err(|error| lowerer_error(declaration, error.to_string()))
                        }
                    }
                    "sql" => {
                        let ResolvedValue::Query(query) = &properties["sql"] else {
                            return Err(lowerer_error(
                                declaration,
                                "data sql source must be a query",
                            ));
                        };
                        let sql = self.query_sql(query, declaration)?;
                        self.context
                            .sql(&sql)
                            .await
                            .map_err(|error| lowerer_error(declaration, error.to_string()))
                    }
                    _ => unreachable!(),
                }
            }
            _ => Err(lowerer_error(
                declaration,
                "data must be a source block or table-valued store binding",
            )),
        }
    }

    async fn inline_values(
        &self,
        value: &ResolvedValue,
        declaration: &ResolvedDeclaration,
    ) -> Result<DataFrame, Diagnostic> {
        let ResolvedValue::Array(rows) = value else {
            return Err(lowerer_error(declaration, "inline values must be an array"));
        };
        if rows.is_empty() {
            return Err(lowerer_error(declaration, "inline values cannot be empty"));
        }
        let ResolvedValue::Object {
            properties: first, ..
        } = &rows[0]
        else {
            return Err(lowerer_error(
                declaration,
                "inline rows must be anonymous objects",
            ));
        };
        if first.is_empty() {
            return Err(lowerer_error(declaration, "inline rows cannot be empty"));
        }
        let columns = first.keys().cloned().collect::<Vec<_>>();
        let mut sql_rows = Vec::new();
        for row in rows {
            let ResolvedValue::Object { properties, .. } = row else {
                return Err(lowerer_error(
                    declaration,
                    "inline rows must be anonymous objects",
                ));
            };
            if properties.keys().ne(columns.iter()) {
                return Err(lowerer_error(
                    declaration,
                    "every inline row must contain the same fields",
                ));
            }
            sql_rows.push(format!(
                "({})",
                columns
                    .iter()
                    .map(|name| inline_literal_sql(&properties[name]))
                    .collect::<Result<Vec<_>, _>>()?
                    .join(", ")
            ));
        }
        let aliases = columns
            .iter()
            .map(|name| format!("\"{}\"", name.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT * FROM (VALUES {}) AS __avenger_inline({aliases})",
            sql_rows.join(", ")
        );
        self.context
            .sql(&sql)
            .await
            .map_err(|error| lowerer_error(declaration, error.to_string()))
    }

    fn table_binding_sql(
        &self,
        value: &ResolvedValue,
        declaration: &ResolvedDeclaration,
    ) -> Result<String, Diagnostic> {
        match value {
            ResolvedValue::Binding(ResolvedBinding {
                target: ResolvedTarget::Param(id),
                time: BindingTime::Current,
                ..
            }) => self
                .params
                .get(id)
                .map(|param| format!("${}", param.name))
                .ok_or_else(|| lowerer_error(declaration, "table binding param is unavailable")),
            ResolvedValue::Expression(expression) => {
                self.row_free_expression_sql(expression, declaration)
            }
            ResolvedValue::Call { function, args } => Ok(format!(
                "{function}({})",
                args.iter()
                    .map(|value| self.table_binding_sql(value, declaration))
                    .collect::<Result<Vec<_>, _>>()?
                    .join(", ")
            )),
            ResolvedValue::String(_)
            | ResolvedValue::Number(_)
            | ResolvedValue::Boolean(_)
            | ResolvedValue::Null => inline_literal_sql(value),
            ResolvedValue::Array(values) => Ok(format!(
                "[{}]",
                values
                    .iter()
                    .map(|value| self.table_binding_sql(value, declaration))
                    .collect::<Result<Vec<_>, _>>()?
                    .join(", ")
            )),
            ResolvedValue::Object { properties, .. } => {
                let mut arguments = Vec::with_capacity(properties.len() * 2);
                for (name, value) in properties {
                    arguments.push(format!("'{}'", name.replace('\'', "''")));
                    arguments.push(self.table_binding_sql(value, declaration)?);
                }
                Ok(format!("named_struct({})", arguments.join(", ")))
            }
            _ => Err(lowerer_error(
                declaration,
                "table parameter binding must be a row-free SQL expression using current scalar params",
            )),
        }
    }

    fn row_free_expression_sql(
        &self,
        expression: &ResolvedExpression,
        declaration: &ResolvedDeclaration,
    ) -> Result<String, Diagnostic> {
        let mut replacements = BTreeMap::new();
        for binding in &expression.bindings {
            let ResolvedTarget::Param(id) = &binding.target else {
                return Err(lowerer_error(
                    declaration,
                    "table arguments may read only scalar params",
                ));
            };
            if binding.time != BindingTime::Current {
                return Err(lowerer_error(
                    declaration,
                    "table arguments may read only current scalar params",
                ));
            }
            let param = self
                .params
                .get(id)
                .ok_or_else(|| lowerer_error(declaration, "table binding param is unavailable"))?;
            replacements.insert(binding_spelling(binding), format!("${}", param.name));
        }
        struct Substituter<'a> {
            replacements: &'a BTreeMap<String, String>,
        }
        impl VisitorMut for Substituter<'_> {
            type Break = ();

            fn pre_visit_expr(&mut self, candidate: &mut SqlExpr) -> ControlFlow<Self::Break> {
                let SqlExpr::Value(value) = candidate else {
                    return ControlFlow::Continue(());
                };
                let SqlValue::Placeholder(placeholder) = &mut value.value else {
                    return ControlFlow::Continue(());
                };
                if let Some(replacement) = self.replacements.get(placeholder) {
                    *placeholder = replacement.clone();
                }
                ControlFlow::Continue(())
            }
        }
        let mut parsed = Parser::new(&AvengerSqlDialect::new())
            .try_with_sql(&expression.sql)
            .map_err(|error| lowerer_error(declaration, error.to_string()))?
            .parse_expr()
            .map_err(|error| lowerer_error(declaration, error.to_string()))?;
        let _ = parsed.visit(&mut Substituter {
            replacements: &replacements,
        });
        Ok(parsed.to_string())
    }

    fn lower_layout(
        &self,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<LayoutSpec, Diagnostic> {
        let native = self.object_declaration(value, data, declaration, "chart")?;
        let key = NativeKindKey::new(NativeKindNamespace::Layout, native.kind.clone());
        let lowered = self
            .registry
            .lower_object(&key, &native)
            .map_err(|error| lowerer_error(declaration, error.to_string()))?;
        lowered
            .downcast::<LayoutSpec>()
            .map(|value| *value)
            .map_err(|_| {
                lowerer_error(
                    declaration,
                    "registered layout lowerer did not return LayoutSpec",
                )
            })
    }

    fn record_schema(&mut self, declaration: &ResolvedDeclaration, data: &DataFrame) {
        self.analysis_schemas.push((
            declaration.id.clone(),
            declaration.span,
            Arc::new(data.schema().as_arrow().clone()),
        ));
    }
}

fn repeat_reference_expr(path: &[String]) -> Option<(Expr, Expr)> {
    use avenger_chart_core::repeat;

    let [name] = path else {
        return None;
    };
    let (resolved, seed) = match name.as_str() {
        "row" => (repeat::row().into_data_expr(), lit(0.0_f64)),
        "column" => (repeat::column().into_data_expr(), lit(0.0_f64)),
        "item" => (repeat::item().into_data_expr(), lit(0.0_f64)),
        "row_name" => (lit(repeat::row_name()), lit("")),
        "column_name" => (lit(repeat::column_name()), lit("")),
        "item_name" => (lit(repeat::item_name()), lit("")),
        "row_index" => (repeat::row_index(), lit(0_i64)),
        "column_index" => (repeat::column_index(), lit(0_i64)),
        "item_index" => (repeat::item_index(), lit(0_i64)),
        "cell_id" => (repeat::cell_id(), lit("")),
        "row_id" => (repeat::row_id(), lit("")),
        "column_id" => (repeat::column_id(), lit("")),
        "item_id" => (repeat::item_id(), lit("")),
        "row_title" => (repeat::row_title(), lit("")),
        "column_title" => (repeat::column_title(), lit("")),
        "item_title" => (repeat::item_title(), lit("")),
        _ => return None,
    };
    Some((resolved, seed))
}

fn chart_analysis_record(
    dataset: &ResolvedDeclaration,
    stage: &ResolvedDeclaration,
    stage_kind: crate::DatasetStageKind,
    data: &DataFrame,
) -> ChartDatasetAnalysis {
    ChartDatasetAnalysis {
        dataset: dataset.id.clone(),
        declaration_span: dataset.span,
        stage_span: stage.span,
        stage_kind,
        schema: Arc::new(data.schema().as_arrow().clone()),
        columns: data
            .schema()
            .iter()
            .map(|(qualifier, field)| crate::AnalyzedColumn {
                name: field.name().clone(),
                qualifier: qualifier.map(ToString::to_string),
                data_type: field.data_type().clone(),
                nullable: field.is_nullable(),
            })
            .collect(),
        logical_plan_fingerprint: Some(logical_plan_fingerprint(data)),
        mark_channels: Vec::new(),
    }
}

fn logical_plan_fingerprint(data: &DataFrame) -> String {
    use sha2::{Digest, Sha256};
    let plan = data.logical_plan().display_indent_schema().to_string();
    format!("sha256:{:x}", Sha256::digest(plan.as_bytes()))
}

fn widget_owned_params(project: &ResolvedModuleGraph) -> BTreeSet<ParamId> {
    project
        .source_modules
        .values()
        .flat_map(|file| &file.roots)
        .flat_map(declarations_depth_first)
        .filter(|declaration| {
            declaration.keyword == "widget"
                || (declaration.keyword == "tool"
                    && declaration.kind.as_deref() != Some("behavior"))
        })
        .flat_map(|declaration| declaration.exports.values())
        .filter_map(|target| match target {
            ResolvedTarget::Param(id) => Some(id.clone()),
            _ => None,
        })
        .collect()
}

fn native_owned_selections(project: &ResolvedModuleGraph) -> BTreeSet<SelectionId> {
    project
        .source_modules
        .values()
        .flat_map(|file| &file.roots)
        .flat_map(declarations_depth_first)
        .filter(|declaration| {
            declaration.keyword == "widget"
                || (declaration.keyword == "tool"
                    && declaration.kind.as_deref() != Some("behavior"))
        })
        .flat_map(|declaration| declaration.exports.values())
        .filter_map(|target| match target {
            ResolvedTarget::Selection(id) => Some(id.clone()),
            _ => None,
        })
        .collect()
}

fn native_owned_stores(project: &ResolvedModuleGraph) -> BTreeSet<StoreId> {
    project
        .source_modules
        .values()
        .flat_map(|file| &file.roots)
        .flat_map(declarations_depth_first)
        .filter(|declaration| {
            declaration.keyword == "widget"
                || (declaration.keyword == "tool"
                    && declaration.kind.as_deref() != Some("behavior"))
        })
        .flat_map(|declaration| declaration.exports.values())
        .filter_map(|target| match target {
            ResolvedTarget::Store(id) => Some(id.clone()),
            _ => None,
        })
        .collect()
}

fn declarations_depth_first(root: &ResolvedDeclaration) -> Vec<&ResolvedDeclaration> {
    let mut result = vec![root];
    for child in &root.children {
        result.extend(declarations_depth_first(child));
    }
    result
}

fn find_declaration<'a>(
    project: &'a ResolvedModuleGraph,
    id: &DeclarationId,
) -> Option<&'a ResolvedDeclaration> {
    project
        .source_modules
        .values()
        .flat_map(|file| &file.roots)
        .flat_map(declarations_depth_first)
        .find(|declaration| &declaration.id == id)
}

fn legend_overlay_blocks(
    declaration: &ResolvedDeclaration,
) -> Vec<(DeclarationId, &[ResolvedDeclaration])> {
    let mut overlays = Vec::new();
    for value in declaration.properties.values() {
        collect_legend_overlay_blocks(value, &mut overlays);
    }
    overlays
}

fn collect_legend_overlay_blocks<'a>(
    value: &'a ResolvedValue,
    overlays: &mut Vec<(DeclarationId, &'a [ResolvedDeclaration])>,
) {
    match value {
        ResolvedValue::Object { properties, .. } => {
            if let Some(legend) = properties.get("legend")
                && let Some((id, children)) = legend_overlay_block(legend)
            {
                overlays.push((id.clone(), children));
            }
            for nested in properties.values() {
                collect_legend_overlay_blocks(nested, overlays);
            }
        }
        ResolvedValue::Array(values) => {
            for nested in values {
                collect_legend_overlay_blocks(nested, overlays);
            }
        }
        ResolvedValue::Channel {
            expression: value, ..
        }
        | ResolvedValue::Pattern(value) => {
            collect_legend_overlay_blocks(value, overlays);
        }
        ResolvedValue::ChannelValue(channel) => {
            if let Some(legend) = channel.configuration.get("legend")
                && let Some((id, children)) = legend_overlay_block(legend)
            {
                overlays.push((id.clone(), children));
            }
            collect_legend_overlay_blocks(&channel.head.expression, overlays);
            if let Some(otherwise) = &channel.otherwise {
                collect_legend_overlay_blocks(&otherwise.expression, overlays);
            }
            for condition in &channel.conditions {
                collect_legend_overlay_blocks(&condition.predicate, overlays);
                collect_legend_overlay_blocks(&condition.branch.expression, overlays);
            }
            for value in channel.configuration.values() {
                collect_legend_overlay_blocks(value, overlays);
            }
        }
        ResolvedValue::Call { args, .. } => {
            for nested in args {
                collect_legend_overlay_blocks(nested, overlays);
            }
        }
        _ => {}
    }
}

fn legend_overlay_block(
    legend: &ResolvedValue,
) -> Option<(&DeclarationId, &[ResolvedDeclaration])> {
    let ResolvedValue::Object { properties, .. } = legend else {
        return None;
    };
    let ResolvedValue::Object { children, .. } = properties.get("overlay")? else {
        return None;
    };
    children
        .first()
        .map(|child| (&child.id, children.as_slice()))
}

fn legend_overlay_id(legend: &ResolvedValue) -> Option<&DeclarationId> {
    legend_overlay_block(legend).map(|(id, _)| id)
}

fn validate_legend_overlay_children(
    owner: &ResolvedDeclaration,
    children: &[ResolvedDeclaration],
) -> Result<(), Diagnostic> {
    if children.is_empty() {
        return Err(lowerer_error(
            owner,
            "legend overlay must contain at least one mark",
        ));
    }
    for child in children {
        if child.keyword != "mark" {
            return Err(lowerer_error(
                child,
                "legend overlay blocks may contain only mark declarations",
            ));
        }
        validate_legend_overlay_mark(child)?;
    }
    Ok(())
}

fn validate_legend_overlay_mark(declaration: &ResolvedDeclaration) -> Result<(), Diagnostic> {
    for child in &declaration.children {
        match child.keyword.as_str() {
            "mark" | "transform" | "view" => validate_legend_overlay_mark(child)?,
            "tool" | "widget" | "on" | "cell" | "plot" | "param" | "store" | "selection"
            | "scale_edit" | "theme" | "resource" | "export" => {
                return Err(lowerer_error(
                    child,
                    format!(
                        "legend overlay marks cannot contain `{}` declarations",
                        child.keyword
                    ),
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

fn find_declaration_by_target<'a>(
    project: &'a ResolvedModuleGraph,
    target: &ResolvedTarget,
) -> Option<&'a ResolvedDeclaration> {
    project
        .source_modules
        .values()
        .flat_map(|file| &file.roots)
        .flat_map(declarations_depth_first)
        .find(|declaration| declaration.runtime_target.as_ref() == Some(target))
}

fn widget_source_id(declaration: &ResolvedDeclaration) -> Option<&str> {
    declaration.name.as_deref()
}

fn public_path_belongs_to_chart(path: &str, chart_path: Option<&str>) -> bool {
    chart_path.is_none_or(|chart| path == chart || path.starts_with(&format!("{chart}.")))
}

fn relative_public_path<'a>(path: &'a str, chart_path: Option<&str>) -> &'a str {
    chart_path
        .and_then(|chart| path.strip_prefix(chart))
        .and_then(|path| path.strip_prefix('.'))
        .unwrap_or(path)
}

fn compiled_mark_aliases(compiled: &avenger_chart::plot::CompiledPlot) -> BTreeMap<String, String> {
    compiled
        .marks()
        .iter()
        .flat_map(|mark| {
            let runtime_id = mark.state().identity.runtime_id.as_opaque_str().to_string();
            mark.state()
                .identity
                .public_aliases
                .iter()
                .cloned()
                .map(move |alias| (alias, runtime_id.clone()))
        })
        .collect()
}

fn compiled_mark_id_for_public_path(
    aliases: &BTreeMap<String, String>,
    path: &str,
    chart_path: Option<&str>,
) -> Option<String> {
    aliases.get(relative_public_path(path, chart_path)).cloned()
}

fn compiled_widget_part_id(
    compiled: &avenger_chart::plot::CompiledPlot,
    path: &str,
    chart_path: Option<&str>,
) -> Option<String> {
    let relative = relative_public_path(path, chart_path);
    compiled.widgets().iter().find_map(|attachment| {
        let avenger_chart_core::CompiledWidget::Composed(widget) = &attachment.widget else {
            return None;
        };
        widget
            .relative_target_ids
            .get(relative)
            .and_then(|ids| ids.first())
            .map(|id| id.as_opaque_str().to_string())
    })
}

fn belongs_to_chart(ancestry: &[DeclarationId], chart: &DeclarationId) -> bool {
    ancestry.iter().any(|ancestor| ancestor == chart)
}

fn resolved_atom(value: &ResolvedValue) -> Option<&str> {
    match value {
        ResolvedValue::Atom(value) | ResolvedValue::String(value) => Some(value),
        _ => None,
    }
}

fn resolved_string(value: &ResolvedValue) -> Option<&str> {
    match value {
        ResolvedValue::String(value) => Some(value),
        _ => None,
    }
}

fn resolved_text(
    value: Option<&ResolvedValue>,
    declaration: &ResolvedDeclaration,
    property: &str,
) -> Result<String, Diagnostic> {
    value
        .and_then(resolved_atom)
        .map(str::to_owned)
        .ok_or_else(|| {
            lowerer_error(
                declaration,
                format!("{property} must be a string or identifier"),
            )
        })
}

fn resolved_f32(
    value: Option<&ResolvedValue>,
    default: f32,
    declaration: &ResolvedDeclaration,
    property: &str,
) -> Result<f32, Diagnostic> {
    match value {
        Some(ResolvedValue::Number(value)) => value.parse::<f32>().map_err(|_| {
            lowerer_error(
                declaration,
                format!("property `{property}` must be a finite number"),
            )
        }),
        None if !default.is_nan() => Ok(default),
        None => Err(lowerer_error(
            declaration,
            format!("pattern property `{property}` is required"),
        )),
        Some(_) => Err(lowerer_error(
            declaration,
            format!("property `{property}` must be a number"),
        )),
    }
}

fn resolved_i32(
    value: &ResolvedValue,
    declaration: &ResolvedDeclaration,
    property: &str,
) -> Result<i32, Diagnostic> {
    let ResolvedValue::Number(value) = value else {
        return Err(lowerer_error(
            declaration,
            format!("property `{property}` must be a 32-bit integer"),
        ));
    };
    value.parse::<i32>().map_err(|_| {
        lowerer_error(
            declaration,
            format!("property `{property}` must be a 32-bit integer"),
        )
    })
}

fn adjustment_output_name(
    value: &ResolvedValue,
    declaration: &ResolvedDeclaration,
) -> Option<String> {
    let binder = declaration.name.as_deref()?;
    let sql = match value {
        ResolvedValue::Expression(expression) => expression.sql.trim(),
        ResolvedValue::Column(column) | ResolvedValue::Atom(column) => column.as_str(),
        _ => return None,
    };
    let (owner, output) = sql.rsplit_once('.')?;
    (owner.trim_matches('"') == binder).then(|| output.trim_matches('"').to_string())
}

fn required_resolved_f32(
    declaration_value: &ResolvedDeclaration,
    property: &str,
    owner: &ResolvedDeclaration,
) -> Result<f32, Diagnostic> {
    resolved_f32(
        declaration_value.properties.get(property),
        f32::NAN,
        owner,
        property,
    )
}

fn required_resolved_f32_alias(
    declaration_value: &ResolvedDeclaration,
    preferred: &str,
    fallback: &str,
    owner: &ResolvedDeclaration,
) -> Result<f32, Diagnostic> {
    resolved_f32(
        declaration_value
            .properties
            .get(preferred)
            .or_else(|| declaration_value.properties.get(fallback)),
        f32::NAN,
        owner,
        preferred,
    )
}

fn optional_resolved_f32_alias(
    declaration_value: &ResolvedDeclaration,
    preferred: &str,
    fallback: &str,
    default: f32,
    owner: &ResolvedDeclaration,
) -> Result<f32, Diagnostic> {
    resolved_f32(
        declaration_value
            .properties
            .get(preferred)
            .or_else(|| declaration_value.properties.get(fallback)),
        default,
        owner,
        preferred,
    )
}

fn pattern_operation(
    value: Option<&ResolvedValue>,
    declaration: &ResolvedDeclaration,
) -> Result<PatternLayerOperation, Diagnostic> {
    match value.and_then(resolved_atom) {
        None | Some("add") => Ok(PatternLayerOperation::Add),
        Some("subtract") => Ok(PatternLayerOperation::Subtract),
        Some("xor") => Ok(PatternLayerOperation::Xor),
        Some(value) => Err(lowerer_error(
            declaration,
            format!("unsupported pattern operation `{value}`"),
        )),
    }
}

fn is_core_property(keyword: &str, name: &str) -> bool {
    matches!(
        (keyword, name),
        (
            "chart" | "plot",
            "data" | "title" | "subtitle" | "layout" | "theme" | "time" | "format" | "guide"
        ) | ("cell", "at" | "data" | "label" | "when")
            | ("mark", "data")
            | ("transform", "scope")
            | ("tool", "id")
    )
}

fn is_resolved_mark_group(declaration: &ResolvedDeclaration) -> bool {
    declaration.keyword == "mark" && declaration.kind.as_deref() == Some("group")
}

fn chart_event_type(value: &str) -> Option<ChartEventType> {
    Some(match value {
        "mouse_down" => ChartEventType::MouseDown,
        "mouse_up" => ChartEventType::MouseUp,
        "click" => ChartEventType::Click,
        "double_click" => ChartEventType::DoubleClick,
        "mouse_wheel" => ChartEventType::MouseWheel,
        "key_press" => ChartEventType::KeyPress,
        "key_release" => ChartEventType::KeyRelease,
        "cursor_moved" => ChartEventType::CursorMoved,
        "mark_mouse_enter" => ChartEventType::MarkMouseEnter,
        "mark_mouse_leave" => ChartEventType::MarkMouseLeave,
        "window_resize" => ChartEventType::WindowResize,
        "window_resize_settled" => ChartEventType::WindowResizeSettled,
        "canvas_resize" => ChartEventType::CanvasResize,
        "canvas_resize_settled" => ChartEventType::CanvasResizeSettled,
        "window_moved" => ChartEventType::WindowMoved,
        "window_focused" => ChartEventType::WindowFocused,
        "window_close_requested" => ChartEventType::WindowCloseRequested,
        _ => return None,
    })
}

fn helper_spelling(
    helper: &avenger_lang_core::ResolvedHelper,
    declaration: &ResolvedDeclaration,
) -> Result<String, Diagnostic> {
    let args = helper
        .arguments
        .iter()
        .map(|argument| match argument {
            ResolvedHelperArgument::Name(value) | ResolvedHelperArgument::Number(value) => {
                Ok(value.clone())
            }
            ResolvedHelperArgument::String(value) => Ok(format!("'{}'", value.replace('\'', "''"))),
            ResolvedHelperArgument::DatumField(value) => Ok(datum_field_spelling(value)),
            ResolvedHelperArgument::Target { authored_path, .. } => Ok(authored_path.join(".")),
            ResolvedHelperArgument::Sql(value) => Ok(value.clone()),
            ResolvedHelperArgument::DefinitionChannel {
                target: _,
                family_suffix,
            } => Err(lowerer_error(
                declaration,
                format!(
                    "helper `{}` cannot reconstruct definition channel suffix `{family_suffix}`",
                    helper.name,
                ),
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(format!("{}({})", helper.name, args.join(", ")))
}

fn contextual_access_spelling(access: &ResolvedContextualAccess) -> String {
    let channel = |channel: &ResolvedChannelMember| channel.authored_name();
    match &access.kind {
        ResolvedContextualAccessKind::DatumField { field } => datum_field_spelling(field),
        ResolvedContextualAccessKind::MarkChannel { channel: member } => {
            format!("channel.{}", channel(member))
        }
        ResolvedContextualAccessKind::EventCoord { channel: member } => {
            format!("event.coord.{}", channel(member))
        }
        ResolvedContextualAccessKind::EventStartCoord { channel: member } => {
            format!("event.start.coord.{}", channel(member))
        }
        ResolvedContextualAccessKind::EventDomainBoundary {
            channel: member,
            boundary,
        } => format!(
            "event.domain.{}.{}",
            channel(member),
            match boundary {
                ResolvedIntervalBoundary::Start => "start",
                ResolvedIntervalBoundary::End => "end",
            }
        ),
        ResolvedContextualAccessKind::EventPath => "event.path".to_owned(),
        ResolvedContextualAccessKind::EventFacet { one_based_index } => {
            format!("event.facet[{one_based_index}]")
        }
        ResolvedContextualAccessKind::EventLegendValue => "event.legend.value".to_owned(),
        ResolvedContextualAccessKind::ItemChannel {
            channel: member, ..
        } => {
            format!("item.channel.{}", channel(member))
        }
        ResolvedContextualAccessKind::ItemDataField { field } => {
            format!("item.data.\"{}\"", field.replace('"', "\"\""))
        }
        ResolvedContextualAccessKind::ItemBbox { edge } => {
            format!("item.bbox.{}", edge.as_str())
        }
        ResolvedContextualAccessKind::ViewField {
            authored_view,
            axis,
            field,
            ..
        } => {
            let axis = match axis {
                ResolvedViewAxis::X => "x",
                ResolvedViewAxis::Y => "y",
            };
            let field = match field {
                ResolvedViewField::DomainStart => "domain.start",
                ResolvedViewField::DomainEnd => "domain.end",
                ResolvedViewField::Pixels => "pixels",
            };
            format!("{}.{axis}.{field}", authored_view.join("."))
        }
    }
}

fn rewrite_source_fragment(source: &str, replacements: &[(String, String)]) -> String {
    replacements
        .iter()
        .fold(source.to_owned(), |source, (authored, replacement)| {
            source.replace(authored, replacement)
        })
}

fn datum_field_spelling(field: &str) -> String {
    format!("datum.\"{}\"", field.replace('"', "\"\""))
}

fn unqualified_column(name: impl Into<String>) -> Expr {
    // DataFusion's `col(&str)` helper parses a SQL-style qualified name and
    // normalizes unquoted identifier case. The DSL has already established
    // that this is a decoded double-quoted column identifier, so construct the
    // column directly to preserve its exact Arrow field spelling.
    Expr::Column(Column::new_unqualified(name))
}

fn rewrite_store_subquery_targets(
    expr: Expr,
    targets: &BTreeMap<String, String>,
) -> Result<Expr, datafusion::error::DataFusionError> {
    if targets.is_empty() {
        return Ok(expr);
    }
    expr.transform(|candidate| {
        let Expr::ScalarSubquery(mut subquery) = candidate else {
            return Ok(Transformed::no(candidate));
        };
        let plan = subquery
            .subquery
            .as_ref()
            .clone()
            .transform_up_with_subqueries(|candidate| {
                let LogicalPlan::TableScan(scan) = candidate else {
                    return Ok(Transformed::no(candidate));
                };
                let Some(target) = targets.get(scan.table_name.table()) else {
                    return Ok(Transformed::no(LogicalPlan::TableScan(scan)));
                };
                Ok(Transformed::yes(LogicalPlan::TableScan(
                    TableScan::try_new(
                        target.as_str(),
                        scan.source,
                        scan.projection,
                        scan.filters,
                        scan.fetch,
                    )?,
                )))
            })?
            .data;
        subquery.subquery = Arc::new(plan);
        Ok(Transformed::yes(Expr::ScalarSubquery(subquery)))
    })
    .map(|transformed| transformed.data)
}

fn helper_argument_expr(
    argument: &ResolvedHelperArgument,
    parse_data: &DataFrame,
    source_replacements: &[(String, String)],
    declaration: &ResolvedDeclaration,
) -> Result<Expr, Diagnostic> {
    let source = match argument {
        ResolvedHelperArgument::Name(value) | ResolvedHelperArgument::Number(value) => {
            value.clone()
        }
        ResolvedHelperArgument::String(value) => format!("'{}'", value.replace('\'', "''")),
        ResolvedHelperArgument::DatumField(value) => datum_field_spelling(value),
        ResolvedHelperArgument::Sql(value) => value.clone(),
        ResolvedHelperArgument::Target { .. }
        | ResolvedHelperArgument::DefinitionChannel { .. } => {
            return Err(lowerer_error(
                declaration,
                "a resolved target cannot be used as a scalar helper argument",
            ));
        }
    };
    let source = rewrite_source_fragment(&source, source_replacements);
    let source =
        normalize_sql_expression(&source).map_err(|error| lowerer_error(declaration, error))?;
    parse_data
        .parse_sql_expr(&source)
        .map_err(|error| lowerer_error(declaration, error.to_string()))
}

fn sharing(value: StateSharing) -> avenger_chart_core::CoordinationScope {
    match value {
        StateSharing::Shared => avenger_chart_core::CoordinationScope::Shared,
        StateSharing::Free => avenger_chart_core::CoordinationScope::Free,
        StateSharing::Level(level) => {
            avenger_chart_core::CoordinationScope::Level(level.min(u8::MAX as u32) as u8)
        }
    }
}

fn inline_literal_sql(value: &ResolvedValue) -> Result<String, Diagnostic> {
    match value {
        ResolvedValue::String(value) => Ok(format!("'{}'", value.replace('\'', "''"))),
        ResolvedValue::Number(value) => Ok(value.clone()),
        ResolvedValue::Boolean(value) => Ok(value.to_string().to_uppercase()),
        ResolvedValue::Null => Ok("NULL".to_string()),
        ResolvedValue::Array(values) => Ok(format!(
            "[{}]",
            values
                .iter()
                .map(inline_literal_sql)
                .collect::<Result<Vec<_>, _>>()?
                .join(", ")
        )),
        ResolvedValue::Object { properties, .. } => {
            let mut arguments = Vec::with_capacity(properties.len() * 2);
            for (name, value) in properties {
                arguments.push(format!("'{}'", name.replace('\'', "''")));
                arguments.push(inline_literal_sql(value)?);
            }
            Ok(format!("named_struct({})", arguments.join(", ")))
        }
        ResolvedValue::Expression(expression)
            if expression.bindings.is_empty()
                && expression.helpers.is_empty()
                && expression.references.is_empty() =>
        {
            Ok(expression.sql.clone())
        }
        _ => Err(diagnostic(
            SourceSpan::empty(avenger_lang_core::SourceId::new(0), 0),
            "AVENGER-LOWER-006",
            "inline data contains a non-literal value",
            format!("found {value:?}"),
        )),
    }
}

fn widget_item_scalar(value: &ResolvedValue) -> Result<ScalarValue, &'static str> {
    match value {
        ResolvedValue::String(value) => Ok(ScalarValue::Utf8(Some(value.clone()))),
        ResolvedValue::Number(value) if value.parse::<i64>().is_ok() => {
            Ok(ScalarValue::Int64(Some(value.parse::<i64>().unwrap())))
        }
        ResolvedValue::Number(value) => value
            .parse::<f64>()
            .map(|value| ScalarValue::Float64(Some(value)))
            .map_err(|_| "must be a scalar literal"),
        ResolvedValue::Boolean(value) => Ok(ScalarValue::Boolean(Some(*value))),
        ResolvedValue::Null => Ok(ScalarValue::Null),
        _ => Err("must be a scalar literal"),
    }
}

fn fixed_contextual_planning_seed(
    kind: &ResolvedContextualAccessKind,
    declaration: &ResolvedDeclaration,
) -> Result<ScalarValue, Diagnostic> {
    let signature = contextual_access_signature(kind.signature_pattern()).ok_or_else(|| {
        lowerer_error(
            declaration,
            format!(
                "contextual access `{}` has no registered signature",
                kind.signature_pattern()
            ),
        )
    })?;
    let physical_type = signature.fixed_arrow_type.ok_or_else(|| {
        lowerer_error(
            declaration,
            format!(
                "contextual access `{}` requires a schema-derived planning type",
                signature.pattern
            ),
        )
    })?;
    let physical_type = physical_type.physical_type();
    ScalarValue::try_new_null(&physical_type_to_arrow(&physical_type))
        .map_err(|error| lowerer_error(declaration, error.to_string()))
}

fn binding_spelling(binding: &ResolvedBinding) -> String {
    let suffix = match binding.time {
        BindingTime::Current => "",
        BindingTime::Start => "@start",
        BindingTime::Previous => "@previous",
    };
    format!("${}{suffix}", binding.authored_path.join("."))
}

fn rewrite_reference_sql(
    mut sql: String,
    reference: &ResolvedSqlReference,
    replacement: &str,
) -> String {
    let quoted_replacement = format!("\"{}\"", replacement.replace('"', "\"\""));
    let bare = reference.authored_path.join(".");
    let quoted = reference
        .authored_path
        .iter()
        .map(|segment| format!("\"{}\"", segment.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(".");
    sql = sql.replace(&quoted, &quoted_replacement);
    sql.replace(&bare, &quoted_replacement)
}

fn is_direct_reference_sql(sql: &str, path: &[String]) -> bool {
    let candidate = sql.trim().trim_end_matches(';').trim();
    let bare = path.join(".");
    let quoted = path
        .iter()
        .map(|segment| format!("\"{}\"", segment.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(".");
    candidate == bare || candidate == quoted
}

fn conditional_branch_expression(value: &ResolvedValue) -> Option<&ResolvedValue> {
    let ResolvedValue::Object { properties, .. } = value else {
        return None;
    };
    match (properties.get("encoded"), properties.get("direct")) {
        (Some(value), None) | (None, Some(value)) => Some(value),
        _ => None,
    }
}

fn channel_fallback(value: &ChannelValue) -> Option<ConditionalValue> {
    match value {
        ChannelValue::Scaled { expr, .. } => Some(ConditionalValue::Scaled { expr: expr.clone() }),
        ChannelValue::Value { expr } => Some(ConditionalValue::Value { expr: expr.clone() }),
        ChannelValue::Conditional { .. } => None,
    }
}

/// Encoded DSL channel literals use the ordinary registered channel policy.
///
/// Do not use `ChannelValue`'s primitive `From` implementations here: those
/// intentionally provide identity-value ergonomics to Rust chart authors.
/// The DSL compiler applies `.no_scale()` only for explicit `direct` mode.
fn scaled_literal_channel(expr: Expr) -> ChannelValue {
    ChannelValue::from(expr)
}

fn validate_private_pipeline_input(
    declaration: &ResolvedDeclaration,
    input: &DataFrame,
) -> Result<(), Diagnostic> {
    let mut private_columns = BTreeSet::new();
    collect_private_physical_columns(declaration, &mut private_columns);
    if private_columns.is_empty() {
        return Ok(());
    }

    if let Some(name) = input.schema().iter().find_map(|(_, field)| {
        let name = field.name();
        (name.starts_with("__av_") || name.starts_with("__private_")).then(|| name.clone())
    }) {
        return Err(diagnostic(
            declaration.span,
            "AVENGER-LOWER-003",
            "defined transform input uses a reserved private-column prefix",
            format!(
                "input column `{name}` must be renamed or projected before this definition instance"
            ),
        ));
    }
    Ok(())
}

fn collect_private_physical_columns(
    declaration: &ResolvedDeclaration,
    output: &mut BTreeSet<String>,
) {
    for value in declaration.properties.values() {
        collect_private_physical_columns_from_value(value, output);
    }
    for child in &declaration.children {
        collect_private_physical_columns(child, output);
    }
}

fn collect_private_physical_columns_from_value(
    value: &ResolvedValue,
    output: &mut BTreeSet<String>,
) {
    match value {
        ResolvedValue::Expression(expression) => {
            if let Ok(expression) = SqlExpression::parse(&expression.sql) {
                output.extend(
                    expression
                        .column_identifier_values()
                        .into_iter()
                        .filter(|name| name.starts_with("__av_col_")),
                );
            }
        }
        ResolvedValue::Query(query) => {
            if let Ok(query) = SqlQuery::parse(&query.sql) {
                output.extend(
                    query
                        .column_identifier_values()
                        .into_iter()
                        .filter(|name| name.starts_with("__av_col_")),
                );
            }
        }
        ResolvedValue::Array(values) | ResolvedValue::Call { args: values, .. } => {
            for value in values {
                collect_private_physical_columns_from_value(value, output);
            }
        }
        ResolvedValue::Object {
            head,
            properties,
            children,
            ..
        } => {
            if let Some(head) = head {
                collect_private_physical_columns_from_value(head, output);
            }
            for value in properties.values() {
                collect_private_physical_columns_from_value(value, output);
            }
            for child in children {
                collect_private_physical_columns(child, output);
            }
        }
        ResolvedValue::Channel {
            expression: value, ..
        }
        | ResolvedValue::Pattern(value) => {
            collect_private_physical_columns_from_value(value, output);
        }
        ResolvedValue::ChannelValue(channel) => {
            collect_private_physical_columns_from_value(&channel.head.expression, output);
            if let Some(otherwise) = &channel.otherwise {
                collect_private_physical_columns_from_value(&otherwise.expression, output);
            }
            for condition in &channel.conditions {
                collect_private_physical_columns_from_value(&condition.predicate, output);
                collect_private_physical_columns_from_value(&condition.branch.expression, output);
            }
            for value in channel.configuration.values() {
                collect_private_physical_columns_from_value(value, output);
            }
        }
        _ => {}
    }
}

fn lowerer_error(declaration: &ResolvedDeclaration, message: impl Into<String>) -> Diagnostic {
    diagnostic(
        declaration.span,
        "AVENGER-LOWER-002",
        "native chart lowering failed",
        message,
    )
}

fn lowerer_error_at(
    declaration: DeclarationId,
    project: &ResolvedModuleGraph,
    message: impl Into<String>,
) -> Diagnostic {
    let message = message.into();
    find_declaration(project, &declaration).map_or_else(
        || {
            diagnostic(
                SourceSpan::empty(avenger_lang_core::SourceId::new(0), 0),
                "AVENGER-LOWER-002",
                "native chart lowering failed",
                message.clone(),
            )
        },
        |declaration| lowerer_error(declaration, message.clone()),
    )
}

fn diagnostic(span: SourceSpan, code: &str, message: &str, label: impl Into<String>) -> Diagnostic {
    Diagnostic::error(code, message, SourceLabel::new(span, label))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dsl_encoded_and_direct_modes_adapt_to_existing_rust_channel_variants() {
        let encoded = scaled_literal_channel(lit(80_i64));
        assert!(matches!(encoded, ChannelValue::Scaled { .. }));

        let direct = scaled_literal_channel(lit(80_i64)).no_scale();
        assert!(matches!(direct, ChannelValue::Value { .. }));
    }
}
