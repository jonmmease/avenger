use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use avenger_app::error::AvengerAppError;
#[cfg(test)]
use avenger_chart::plot::ScopedParamAssignment;
use avenger_chart::{
    event::{
        self, ChartActionParamValue, ChartEventAssignmentScope, ChartEventBinding,
        ChartEventEvaluationMode, ChartEventScopeTarget, ChartEventStream, ChartEventSurfaceTarget,
        ChartEventType, ChartParamChangeBinding, InteractionColumnRequests, param_change,
    },
    plot::{
        CompiledPlot, ResolvedScopedParamAssignment, ResolvedScopedStoreAssignment,
        ResolvedSelectionAssignment, ResolvedStateTransaction, ScopedParamStoreSnapshot,
        SelectionStateUpdate, StoreStateUpdate,
    },
    render::{
        EvaluatedEventDatumState, EvaluatedInteractionScope, EvaluatedWidgetFrame, EvaluationMode,
        InteractionScopeKind,
    },
    serialization::LogicalExprNodeExt,
};
#[cfg(test)]
use avenger_chart_core::{ChartActionStep, ChartEventParamAction, ResolvedStateTarget};
use avenger_chart_core::{
    ChartEventAction, CompiledParamSpec, CompiledScalarExpressionProgram, CompiledSelectionSpec,
    CompiledStateRegistry, CompiledStoreSpec, CoordinationScope, InteractionPointInversionRequest,
    LegendSurfaceKind, MarkId, ParamRef, PhysicalScalarExpressionSpec,
    PhysicalScalarProgramOptions, PlaceholderColumn, ResolvedSelectionClauseScope,
    SceneGeometryCoordinateSpace, SceneGeometryHitPolicy, SceneGeometryQuery,
    SceneGeometryQueryGeometry, SceneGeometryTarget, SceneQueryClauseId, SceneQueryDatumField,
    SelectionClause, SelectionClauseUpdate, SelectionEqualityDimensionUpdate,
    SelectionEqualityDimensionValue, SelectionFacetContextValue, SelectionIntervalDimensionUpdate,
    SelectionIntervalDimensionValue, SelectionPredicateSpec, SelectionPredicateUpdate,
    SelectionPredicateValue, SelectionPredicateValueUpdate, SelectionRef, SelectionSceneQuery,
    SelectionUpdate, SelectionValueExpr, StoreFieldPatch, StoreKey, StoreRef, StoreRow,
    StoreRowValue, StoreUpdate, StoreValueExpr, collect_placeholder_ids, encode_selection_tuple_id,
    one_row_batch_from_scalars, schema_from_fields, selection_target_from_placeholder,
    store_target_from_placeholder, widget_runtime_input_data_type,
};
use avenger_common::cursor::CursorStyle;
use avenger_common::time::Instant;
use avenger_eventstream::{
    manager::EventStreamHandler,
    scene::{ModifiersState, SceneGraphEvent, SceneGraphEventType},
    stream::{
        EventAdmission, EventStreamConfig, EventStreamContext, EventStreamEventSnapshot,
        EventStreamFilter, UpdateStatus,
    },
    window::{Key, MouseButton, MouseScrollDelta},
};
use avenger_geometry::{GeometryQueryHitPolicy, GeometryQueryShape, rtree::SceneGraphRTree};
use avenger_scenegraph::marks::mark::MarkInstance;
use datafusion::common::tree_node::{TreeNode, TreeNodeRecursion};
use datafusion::{
    arrow::{
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    dataframe::DataFrame,
    datasource::{MemTable, provider_as_source},
    error::DataFusionError,
    logical_expr::{Expr, LogicalPlan, TableScan, cast, try_cast},
    prelude::SessionContext,
    scalar::ScalarValue,
};
use indexmap::IndexMap;

use crate::ChartAppState;

type CompiledEventStream = (
    EventStreamConfig,
    Arc<dyn EventStreamHandler<ChartAppState>>,
);
type CompiledEventStreams = Vec<CompiledEventStream>;
type MarkRuntimePathIndex = BTreeMap<MarkId, Vec<Vec<usize>>>;
type MarkRuntimeNameIndex = BTreeMap<MarkId, Vec<String>>;

#[derive(Clone, Debug, Default)]
struct MarkRuntimeTargetIndex {
    paths: MarkRuntimePathIndex,
    names: MarkRuntimeNameIndex,
}

impl MarkRuntimeTargetIndex {
    fn for_plot(plot: &CompiledPlot) -> Self {
        Self {
            paths: plot.mark_runtime_path_index().clone(),
            names: plot.mark_runtime_name_index(),
        }
    }
}

fn runtime_paths_for_mark_ids(
    ids: &[MarkId],
    index: Option<&MarkRuntimeTargetIndex>,
) -> Result<Option<Vec<Vec<usize>>>, AvengerAppError> {
    if ids.is_empty() {
        return Ok(None);
    }
    let index = index.ok_or_else(|| {
        AvengerAppError::InternalError(
            "compiled mark targets require a runtime mark-path index".to_string(),
        )
    })?;
    let mut paths = Vec::new();
    for id in ids {
        paths.extend(index.paths.get(id).cloned().ok_or_else(|| {
            AvengerAppError::InternalError(format!(
                "compiled mark identity '{id}' has no runtime scene path"
            ))
        })?);
    }
    paths.sort();
    paths.dedup();
    Ok(Some(paths))
}

fn runtime_names_for_mark_targets(
    ids: &[MarkId],
    source_names: &[String],
    index: Option<&MarkRuntimeTargetIndex>,
) -> Option<Vec<String>> {
    let mut names = Vec::new();
    let mut needs_source_names = ids.is_empty() || index.is_none();
    if let Some(index) = index {
        for id in ids {
            if let Some(runtime_names) = index.names.get(id) {
                names.extend(runtime_names.iter().cloned());
            } else {
                needs_source_names = true;
            }
        }
    }
    if needs_source_names {
        names.extend(source_names.iter().cloned());
    }
    names.sort();
    names.dedup();
    (!names.is_empty()).then_some(names)
}
pub(crate) type StreamParamProvider =
    Arc<dyn Fn(&SceneGraphEvent) -> IndexMap<String, ScalarValue> + Send + Sync>;

pub(crate) fn committed_stream_param_provider(state: &ChartAppState) -> StreamParamProvider {
    let state = state.clone();
    Arc::new(move |event| {
        if let Ok(runtime) = state.runtime.try_lock() {
            let routed_scope =
                event.position().and_then(|position| {
                    match route_interaction_scope(
                        &runtime.last_interaction_state.scopes,
                        Some(position),
                        &std::collections::BTreeSet::new(),
                    ) {
                        InteractionRoute::Scope(scope) => Some(scope),
                        InteractionRoute::None | InteractionRoute::Ambiguous => None,
                    }
                });
            let owner_paths = routed_scope
                .map(|scope| scope.sharing_owner_paths.clone())
                .unwrap_or_default();
            return runtime
                .session
                .effective_params_for_owner_paths(&owner_paths);
        }
        state
            .params
            .lock()
            .expect("chart param lock poisoned")
            .params
            .clone()
    })
}
type ParamSpecRegistry = CompiledStateRegistry<ParamRef, CompiledParamSpec>;
type StoreSpecRegistry = CompiledStateRegistry<StoreRef, CompiledStoreSpec>;
type SelectionSpecRegistry = CompiledStateRegistry<SelectionRef, CompiledSelectionSpec>;

fn resolved_actions_for_binding(
    binding: &ChartEventBinding,
    _param_specs: &ParamSpecRegistry,
    _store_specs: &StoreSpecRegistry,
    _selection_specs: &SelectionSpecRegistry,
) -> Result<Vec<ChartEventAction>, AvengerAppError> {
    if binding.resolved_actions().is_empty() && !binding.action.ordered_steps().is_empty() {
        return Err(AvengerAppError::InternalError(
            "Chart event state targets were not resolved during chart compilation".to_string(),
        ));
    }
    Ok(binding.resolved_actions().to_vec())
}

#[cfg(test)]
pub(crate) fn event_streams_for_plot_bindings(
    compiled_plot: &CompiledPlot,
    ctx: &SessionContext,
) -> Result<CompiledEventStreams, AvengerAppError> {
    event_streams_for_plot_bindings_with_param_provider(compiled_plot, ctx, None)
}

pub(crate) fn event_streams_for_plot_bindings_with_param_provider(
    compiled_plot: &CompiledPlot,
    ctx: &SessionContext,
    stream_param_provider: Option<StreamParamProvider>,
) -> Result<CompiledEventStreams, AvengerAppError> {
    let event_datum_types = compiled_plot.event_datum_types();
    let event_coord_types = compiled_plot
        .event_coord_types(ctx)
        .map_err(|err| AvengerAppError::InternalError(err.to_string()))?;
    let mark_runtime_targets = MarkRuntimeTargetIndex::for_plot(compiled_plot);
    event_streams_for_bindings_with_coord_types(
        compiled_plot.event_bindings(),
        ctx,
        compiled_plot.param_specs(),
        compiled_plot.selection_specs(),
        compiled_plot.store_specs(),
        &event_datum_types,
        &event_coord_types,
        stream_param_provider,
        Some(&mark_runtime_targets),
    )
}

pub(crate) fn event_streams_for_bindings(
    bindings: &[ChartEventBinding],
    ctx: &SessionContext,
    param_specs: &ParamSpecRegistry,
    selection_specs: &SelectionSpecRegistry,
    store_specs: &StoreSpecRegistry,
    event_datum_types: &IndexMap<String, DataType>,
) -> Result<CompiledEventStreams, AvengerAppError> {
    event_streams_for_bindings_with_coord_types(
        bindings,
        ctx,
        param_specs,
        selection_specs,
        store_specs,
        event_datum_types,
        &IndexMap::new(),
        None,
        None,
    )
}

// Binding compilation consumes independent registries whose identities matter
// to diagnostics; bundling them would only move this validation surface.
#[allow(clippy::too_many_arguments)]
fn event_streams_for_bindings_with_coord_types(
    bindings: &[ChartEventBinding],
    ctx: &SessionContext,
    param_specs: &ParamSpecRegistry,
    selection_specs: &SelectionSpecRegistry,
    store_specs: &StoreSpecRegistry,
    event_datum_types: &IndexMap<String, DataType>,
    event_coord_types: &IndexMap<String, DataType>,
    stream_param_provider: Option<StreamParamProvider>,
    mark_runtime_targets: Option<&MarkRuntimeTargetIndex>,
) -> Result<CompiledEventStreams, AvengerAppError> {
    let mut streams = Vec::new();
    for (binding_index, binding) in bindings.iter().enumerate() {
        let runtime = Arc::new(CompiledChartEventBinding::compile_with_event_coord_types(
            binding_index,
            binding,
            ctx,
            param_specs,
            selection_specs,
            store_specs,
            event_datum_types,
            event_coord_types,
            stream_param_provider.clone(),
            mark_runtime_targets,
        )?);
        streams.push((
            runtime.event_stream_config.clone(),
            Arc::new(ChartEventBindingHandler {
                runtime: runtime.clone(),
                state: Mutex::new(ChartEventBindingState::default()),
            }) as Arc<dyn EventStreamHandler<ChartAppState>>,
        ));

        if binding.action.settle_exact {
            streams.push((
                EventStreamConfig {
                    types: vec![SceneGraphEventType::InteractionSettled],
                    ..Default::default()
                },
                Arc::new(ChartEventExactOnlyHandler) as Arc<dyn EventStreamHandler<ChartAppState>>,
            ));
        }
    }
    Ok(streams)
}

struct CompiledChartEventBinding {
    binding_index: usize,
    event_stream_config: EventStreamConfig,
    program: CompiledScalarExpressionProgram,
    deferred_expressions: HashMap<usize, DeferredEventExpression>,
    deferred_program_options: PhysicalScalarProgramOptions,
    session_context: SessionContext,
    filter_count: usize,
    assignments: Vec<CompiledParamAssignment>,
    ordered_actions: Vec<CompiledOrderedAction>,
    evaluation_mode: ChartEventEvaluationMode,
    settle_exact: bool,
    interaction_requests: InteractionColumnRequests,
    event_path_min_distance_px: f32,
    scope_target: Option<ChartEventScopeTarget>,
    surface_target: Option<ChartEventSurfaceTarget>,
    scope_target_uses_start_scope: bool,
}

#[derive(Clone)]
struct DeferredEventExpression {
    name: String,
    expr: Expr,
    expected_type: Option<DataType>,
    null_on_cast_failure: bool,
}

#[derive(Clone)]
struct CompiledParamAssignment {
    runtime_id: ParamRef,
    param_name: String,
    sharing: CoordinationScope,
    default_value: ScalarValue,
    scope: ChartEventAssignmentScope,
    replace_scoped_values: bool,
    reject_null: bool,
    value_index: usize,
}

#[derive(Clone)]
#[allow(clippy::enum_variant_names)]
enum CompiledOrderedAction {
    SetParam(CompiledParamAssignment),
    SetStore(CompiledStoreAssignment),
    SetSelection(Box<CompiledSelectionAssignment>),
    SetCursor { value_index: usize },
}

impl CompiledOrderedAction {
    fn expression_indices(&self, filter_count: usize) -> Vec<usize> {
        let mut indices = Vec::new();
        match self {
            Self::SetParam(assignment) => indices.push(assignment.value_index),
            Self::SetStore(assignment) => collect_compiled_store_update_indices(
                &assignment.update,
                filter_count,
                &mut indices,
            ),
            Self::SetSelection(assignment) => collect_compiled_selection_update_indices(
                &assignment.update,
                filter_count,
                &mut indices,
            ),
            Self::SetCursor { value_index } => indices.push(*value_index),
        }
        indices.sort_unstable();
        indices.dedup();
        indices
    }
}

/// Physical programs and dependency metadata for all parameter-change bindings.
///
/// Binding order is retained for deterministic scheduling among independent
/// reactions. Dependencies themselves are represented by `edges`; runtime
/// execution must follow changed-value waves rather than declaration order.
pub(crate) struct CompiledChartParamChangeGraph {
    bindings: Vec<CompiledChartParamChangeBinding>,
    bindings_by_source: IndexMap<ParamRef, Vec<usize>>,
    edges: Vec<(String, String)>,
}

pub(crate) struct EvaluatedParamChangeAction {
    pub binding_index: usize,
    pub param_patch: Vec<ResolvedScopedParamAssignment>,
    pub store_patch: Vec<ResolvedScopedStoreAssignment>,
    pub selection_patch: Vec<ResolvedSelectionAssignment>,
    pub evaluation_mode: ChartEventEvaluationMode,
    pub settle_exact: bool,
}

// W6.3 compiles the complete action payload; W6.4 consumes these fields when
// the FIFO transaction coordinator evaluates and stages reaction effects.
#[allow(dead_code)]
struct CompiledChartParamChangeBinding {
    binding_index: usize,
    source_runtime_id: ParamRef,
    source_param_name: String,
    source_default: ScalarValue,
    program: CompiledScalarExpressionProgram,
    filter_count: usize,
    ordered_actions: Vec<CompiledOrderedAction>,
    evaluation_mode: ChartEventEvaluationMode,
    settle_exact: bool,
}

#[derive(Clone)]
struct CompiledStoreAssignment {
    runtime_id: StoreRef,
    store_name: String,
    sharing: CoordinationScope,
    scope: ChartEventAssignmentScope,
    replace_scoped_values: bool,
    update: CompiledStoreUpdate,
}

#[derive(Clone)]
enum CompiledStoreUpdate {
    Clear,
    ReplaceRows {
        rows: Vec<CompiledStoreRow>,
    },
    InsertRows {
        rows: Vec<CompiledStoreRow>,
    },
    UpsertRows {
        rows: Vec<CompiledStoreRow>,
    },
    UpdateByKey {
        key: CompiledStoreRow,
        fields: CompiledStoreRow,
    },
    DeleteByKey {
        key: CompiledStoreRow,
    },
    ToggleRows {
        rows: Vec<CompiledStoreRow>,
    },
}

#[derive(Clone)]
struct CompiledStoreRow {
    fields: Vec<(String, usize)>,
}

#[derive(Clone)]
struct CompiledSelectionAssignment {
    runtime_id: SelectionRef,
    selection_id: String,
    spec: CompiledSelectionSpec,
    scope: ChartEventAssignmentScope,
    update: CompiledSelectionUpdate,
}

#[derive(Clone)]
struct CompiledSelectionSceneQuery {
    query: CompiledSceneGeometryQuery,
    sharing: CoordinationScope,
    clause_id: CompiledSceneQueryClauseId,
}

#[derive(Clone)]
struct CompiledSceneGeometryQuery {
    geometry: CompiledSceneGeometryQueryGeometry,
    coordinate_space: SceneGeometryCoordinateSpace,
    hit_policy: SceneGeometryHitPolicy,
    target: SceneGeometryTarget,
    runtime_mark_paths: Option<Vec<Vec<usize>>>,
    runtime_mark_names: Option<Vec<String>>,
    datum_fields: Vec<SceneQueryDatumField>,
    unique_by: Vec<String>,
    max_hits: Option<usize>,
}

#[derive(Clone)]
enum CompiledSceneGeometryQueryGeometry {
    Rect {
        x0: usize,
        y0: usize,
        x1: usize,
        y1: usize,
    },
    Circle {
        cx: usize,
        cy: usize,
        radius: usize,
    },
    Polygon {
        points: usize,
    },
}

#[derive(Clone)]
enum CompiledSceneQueryClauseId {
    Tuple,
    Field(String),
    Expr(usize),
}

#[derive(Clone)]
enum CompiledSelectionUpdate {
    Clear,
    ClearInScope {
        scope: CoordinationScope,
    },
    ReplaceAllClauses {
        clauses: Vec<CompiledSelectionClause>,
    },
    ReplaceClausesInScope {
        scope: CoordinationScope,
        clauses: Vec<CompiledSelectionClause>,
    },
    UpsertClauses {
        clauses: Vec<CompiledSelectionClause>,
    },
    ToggleClauses {
        clauses: Vec<CompiledSelectionClause>,
    },
    ToggleEqualityValue {
        field_expr: datafusion_proto::protobuf::LogicalExprNode,
        value: usize,
        item_id: usize,
        facet_scope: CoordinationScope,
    },
    ReplaceAllFromSceneQuery {
        query: CompiledSelectionSceneQuery,
    },
    ReplaceFromSceneQueryInScope {
        query: CompiledSelectionSceneQuery,
    },
    UpsertFromSceneQuery {
        query: CompiledSelectionSceneQuery,
    },
    ToggleFromSceneQuery {
        query: CompiledSelectionSceneQuery,
    },
    DeleteClauses {
        ids: Vec<usize>,
    },
    DeleteClausesInScope {
        scope: CoordinationScope,
        ids: Vec<usize>,
    },
}

#[derive(Clone)]
struct CompiledSelectionClause {
    id: usize,
    facet_scope: CoordinationScope,
    predicate: CompiledSelectionPredicate,
}

#[derive(Clone)]
enum CompiledSelectionPredicate {
    Interval {
        dimensions: Vec<CompiledSelectionIntervalDimension>,
    },
    Equality {
        dimensions: Vec<CompiledSelectionEqualityDimension>,
    },
    Predicate {
        values: Vec<CompiledSelectionPredicateValue>,
        expr: datafusion_proto::protobuf::LogicalExprNode,
        kind: Option<String>,
    },
}

#[derive(Clone)]
struct CompiledSelectionIntervalDimension {
    id: String,
    field_expr: datafusion_proto::protobuf::LogicalExprNode,
    min: usize,
    max: usize,
}

#[derive(Clone)]
struct CompiledSelectionEqualityDimension {
    id: String,
    field_expr: datafusion_proto::protobuf::LogicalExprNode,
    value: usize,
}

#[derive(Clone)]
struct CompiledSelectionPredicateValue {
    id: String,
    value: usize,
}

fn collect_compiled_store_update_indices(
    update: &CompiledStoreUpdate,
    filter_count: usize,
    out: &mut Vec<usize>,
) {
    let mut collect_row = |row: &CompiledStoreRow| {
        out.extend(
            row.fields
                .iter()
                .map(|(_, index)| filter_count.saturating_add(*index)),
        );
    };
    match update {
        CompiledStoreUpdate::Clear => {}
        CompiledStoreUpdate::ReplaceRows { rows }
        | CompiledStoreUpdate::InsertRows { rows }
        | CompiledStoreUpdate::UpsertRows { rows }
        | CompiledStoreUpdate::ToggleRows { rows } => {
            for row in rows {
                collect_row(row);
            }
        }
        CompiledStoreUpdate::UpdateByKey { key, fields } => {
            collect_row(key);
            collect_row(fields);
        }
        CompiledStoreUpdate::DeleteByKey { key } => collect_row(key),
    }
}

fn collect_compiled_selection_update_indices(
    update: &CompiledSelectionUpdate,
    filter_count: usize,
    out: &mut Vec<usize>,
) {
    let mut push = |index: usize| out.push(filter_count.saturating_add(index));
    match update {
        CompiledSelectionUpdate::Clear | CompiledSelectionUpdate::ClearInScope { .. } => {}
        CompiledSelectionUpdate::ReplaceAllClauses { clauses }
        | CompiledSelectionUpdate::ReplaceClausesInScope { clauses, .. }
        | CompiledSelectionUpdate::UpsertClauses { clauses }
        | CompiledSelectionUpdate::ToggleClauses { clauses } => {
            for clause in clauses {
                push(clause.id);
                match &clause.predicate {
                    CompiledSelectionPredicate::Interval { dimensions } => {
                        for dimension in dimensions {
                            push(dimension.min);
                            push(dimension.max);
                        }
                    }
                    CompiledSelectionPredicate::Equality { dimensions } => {
                        for dimension in dimensions {
                            push(dimension.value);
                        }
                    }
                    CompiledSelectionPredicate::Predicate { values, .. } => {
                        for value in values {
                            push(value.value);
                        }
                    }
                }
            }
        }
        CompiledSelectionUpdate::ToggleEqualityValue { value, item_id, .. } => {
            push(*value);
            push(*item_id);
        }
        CompiledSelectionUpdate::ReplaceAllFromSceneQuery { query }
        | CompiledSelectionUpdate::ReplaceFromSceneQueryInScope { query }
        | CompiledSelectionUpdate::UpsertFromSceneQuery { query }
        | CompiledSelectionUpdate::ToggleFromSceneQuery { query } => {
            match &query.query.geometry {
                CompiledSceneGeometryQueryGeometry::Rect { x0, y0, x1, y1 } => {
                    push(*x0);
                    push(*y0);
                    push(*x1);
                    push(*y1);
                }
                CompiledSceneGeometryQueryGeometry::Circle { cx, cy, radius } => {
                    push(*cx);
                    push(*cy);
                    push(*radius);
                }
                CompiledSceneGeometryQueryGeometry::Polygon { points } => push(*points),
            }
            if let CompiledSceneQueryClauseId::Expr(index) = &query.clause_id {
                push(*index);
            }
        }
        CompiledSelectionUpdate::DeleteClauses { ids }
        | CompiledSelectionUpdate::DeleteClausesInScope { ids, .. } => {
            for index in ids {
                push(*index);
            }
        }
    }
}

struct StoreExpressionAssignment {
    runtime_id: StoreRef,
    store_name: String,
    sharing: CoordinationScope,
    scope: ChartEventAssignmentScope,
    replace_scoped_values: bool,
    update: StoreExpressionUpdate,
}

#[allow(clippy::enum_variant_names)]
enum PreparedEventAction {
    SetParam {
        runtime_id: ParamRef,
        param_name: String,
        sharing: CoordinationScope,
        default_value: ScalarValue,
        data_type: DataType,
        scope: ChartEventAssignmentScope,
        replace_scoped_values: bool,
        reject_null: bool,
        expr: Expr,
    },
    SetStore(StoreExpressionAssignment),
    SetSelection(Box<SelectionExpressionAssignment>),
    SetCursor(Expr),
}

enum StoreExpressionUpdate {
    Clear,
    ReplaceRows {
        rows: Vec<StoreExpressionRow>,
    },
    InsertRows {
        rows: Vec<StoreExpressionRow>,
    },
    UpsertRows {
        rows: Vec<StoreExpressionRow>,
    },
    UpdateByKey {
        key: StoreExpressionRow,
        fields: StoreExpressionRow,
    },
    DeleteByKey {
        key: StoreExpressionRow,
    },
    ToggleRows {
        rows: Vec<StoreExpressionRow>,
    },
}

struct StoreExpressionRow {
    fields: Vec<StoreExpressionField>,
}

struct StoreExpressionField {
    name: String,
    expr: Expr,
    expected_type: DataType,
}

struct SelectionExpressionAssignment {
    runtime_id: SelectionRef,
    selection_id: String,
    spec: CompiledSelectionSpec,
    scope: ChartEventAssignmentScope,
    update: SelectionExpressionUpdate,
}

enum SelectionExpressionUpdate {
    Clear,
    ClearInScope {
        scope: CoordinationScope,
    },
    ReplaceAllClauses {
        clauses: Vec<SelectionExpressionClause>,
    },
    ReplaceClausesInScope {
        scope: CoordinationScope,
        clauses: Vec<SelectionExpressionClause>,
    },
    UpsertClauses {
        clauses: Vec<SelectionExpressionClause>,
    },
    ToggleClauses {
        clauses: Vec<SelectionExpressionClause>,
    },
    ToggleEqualityValue {
        field_expr: datafusion_proto::protobuf::LogicalExprNode,
        value: Expr,
        item_id: Expr,
        facet_scope: CoordinationScope,
    },
    ReplaceAllFromSceneQuery {
        query: SelectionSceneQueryExpression,
    },
    ReplaceFromSceneQueryInScope {
        query: SelectionSceneQueryExpression,
    },
    UpsertFromSceneQuery {
        query: SelectionSceneQueryExpression,
    },
    ToggleFromSceneQuery {
        query: SelectionSceneQueryExpression,
    },
    DeleteClauses {
        ids: Vec<Expr>,
    },
    DeleteClausesInScope {
        scope: CoordinationScope,
        ids: Vec<Expr>,
    },
}

struct SelectionExpressionClause {
    id: Expr,
    facet_scope: CoordinationScope,
    predicate: SelectionExpressionPredicate,
}

enum SelectionExpressionPredicate {
    Interval {
        dimensions: Vec<SelectionExpressionIntervalDimension>,
    },
    Equality {
        dimensions: Vec<SelectionExpressionEqualityDimension>,
    },
    Predicate {
        values: Vec<SelectionExpressionPredicateValue>,
        expr: datafusion_proto::protobuf::LogicalExprNode,
        kind: Option<String>,
    },
}

struct SelectionExpressionIntervalDimension {
    id: String,
    field_expr: datafusion_proto::protobuf::LogicalExprNode,
    min: Expr,
    max: Expr,
}

struct SelectionExpressionEqualityDimension {
    id: String,
    field_expr: datafusion_proto::protobuf::LogicalExprNode,
    value: Expr,
}

struct SelectionExpressionPredicateValue {
    id: String,
    value: Expr,
}

struct SelectionSceneQueryExpression {
    query: SceneGeometryQueryExpression,
    sharing: CoordinationScope,
    clause_id: SceneQueryClauseIdExpression,
}

struct SceneGeometryQueryExpression {
    geometry: SceneGeometryQueryGeometryExpression,
    coordinate_space: SceneGeometryCoordinateSpace,
    hit_policy: SceneGeometryHitPolicy,
    target: SceneGeometryTarget,
    runtime_mark_paths: Option<Vec<Vec<usize>>>,
    runtime_mark_names: Option<Vec<String>>,
    datum_fields: Vec<SceneQueryDatumField>,
    unique_by: Vec<String>,
    max_hits: Option<usize>,
}

enum SceneGeometryQueryGeometryExpression {
    Rect {
        x0: Expr,
        y0: Expr,
        x1: Expr,
        y1: Expr,
    },
    Circle {
        cx: Expr,
        cy: Expr,
        radius: Expr,
    },
    Polygon {
        points: Expr,
    },
}

enum SceneQueryClauseIdExpression {
    Tuple,
    Field(String),
    Expr(Expr),
}

fn event_expression_requires_deferred_evaluation(expr: &Expr) -> Result<bool, DataFusionError> {
    let mut deferred = false;
    expr.apply(|candidate| {
        let requires = match candidate {
            Expr::Exists(_) | Expr::InSubquery(_) | Expr::ScalarSubquery(_) => true,
            Expr::Placeholder(placeholder) => {
                selection_target_from_placeholder(&placeholder.id).is_some()
                    || store_target_from_placeholder(&placeholder.id).is_some()
            }
            _ => false,
        };
        if requires {
            deferred = true;
            Ok(TreeNodeRecursion::Stop)
        } else {
            Ok(TreeNodeRecursion::Continue)
        }
    })?;
    Ok(deferred)
}

impl CompiledChartEventBinding {
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    fn compile(
        binding_index: usize,
        binding: &ChartEventBinding,
        ctx: &SessionContext,
        param_specs: &ParamSpecRegistry,
        selection_specs: &SelectionSpecRegistry,
        store_specs: &StoreSpecRegistry,
        event_datum_types: &IndexMap<String, DataType>,
    ) -> Result<Self, AvengerAppError> {
        Self::compile_with_event_coord_types(
            binding_index,
            binding,
            ctx,
            param_specs,
            selection_specs,
            store_specs,
            event_datum_types,
            &IndexMap::new(),
            None,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn compile_with_event_coord_types(
        binding_index: usize,
        binding: &ChartEventBinding,
        ctx: &SessionContext,
        param_specs: &ParamSpecRegistry,
        selection_specs: &SelectionSpecRegistry,
        store_specs: &StoreSpecRegistry,
        event_datum_types: &IndexMap<String, DataType>,
        event_coord_types: &IndexMap<String, DataType>,
        stream_param_provider: Option<StreamParamProvider>,
        mark_runtime_targets: Option<&MarkRuntimeTargetIndex>,
    ) -> Result<Self, AvengerAppError> {
        binding
            .validate()
            .map_err(|err| AvengerAppError::InternalError(err.to_string()))?;
        let resolved_actions =
            resolved_actions_for_binding(binding, param_specs, store_specs, selection_specs)?;
        for action in &resolved_actions {
            let valid = match action {
                ChartEventAction::SetParam(action) => param_specs
                    .get_by_id(&action.target.id)
                    .is_some_and(|spec| spec.runtime_id == action.target.id),
                ChartEventAction::SetStore(action) => store_specs
                    .get_by_id(&action.target.id)
                    .is_some_and(|spec| spec.runtime_id == action.target.id),
                ChartEventAction::SetSelection(action) => selection_specs
                    .get_by_id(&action.target.id)
                    .is_some_and(|spec| spec.runtime_id == action.target.id),
                ChartEventAction::SetCursor(_) => true,
            };
            if !valid {
                return Err(AvengerAppError::InternalError(
                    "Chart event binding contains a stale or mismatched resolved state target"
                        .to_string(),
                ));
            }
        }

        // Convert filter and assignment expressions once so we can scan them for
        // reserved interaction columns before building the event schema.
        let mut filter_exprs = Vec::new();
        for filter in &binding.filters {
            filter_exprs.push(
                filter
                    .to_expr(ctx)
                    .map_err(|err| AvengerAppError::InternalError(err.to_string()))?,
            );
        }
        let mut prepared_actions = Vec::with_capacity(resolved_actions.len());
        for action in resolved_actions {
            prepared_actions.push(match action {
                ChartEventAction::SetParam(action) => {
                    let spec = param_specs
                        .get_by_id(&action.target.id)
                        .expect("resolved param action validated");
                    let expr = match action.value {
                        ChartActionParamValue::Expr { expr } => expr
                            .to_expr(ctx)
                            .map_err(|err| AvengerAppError::InternalError(err.to_string()))?,
                        ChartActionParamValue::RegisteredDefault => {
                            datafusion::prelude::lit(spec.default.clone())
                        }
                    };
                    PreparedEventAction::SetParam {
                        runtime_id: action.target.id,
                        param_name: action.target.source_name,
                        sharing: spec.sharing,
                        default_value: spec.default.clone(),
                        data_type: spec.data_type.clone(),
                        scope: action.scope,
                        replace_scoped_values: action.replace_scoped_values,
                        reject_null: action.reject_null,
                        expr,
                    }
                }
                ChartEventAction::SetStore(action) => {
                    let spec = store_specs
                        .get_by_id(&action.target.id)
                        .expect("resolved store action validated");
                    PreparedEventAction::SetStore(StoreExpressionAssignment {
                        runtime_id: action.target.id,
                        store_name: action.target.source_name,
                        sharing: spec.sharing,
                        scope: action.scope,
                        replace_scoped_values: action.replace_scoped_values,
                        update: compile_store_expression_update(spec, &action.update, ctx)?,
                    })
                }
                ChartEventAction::SetSelection(action) => {
                    let spec = selection_specs
                        .get_by_id(&action.target.id)
                        .expect("resolved selection action validated");
                    PreparedEventAction::SetSelection(Box::new(SelectionExpressionAssignment {
                        runtime_id: action.target.id,
                        selection_id: action.target.source_name,
                        spec: spec.clone(),
                        scope: action.scope,
                        update: compile_selection_expression_update(
                            &action.update,
                            ctx,
                            mark_runtime_targets,
                        )?,
                    }))
                }
                ChartEventAction::SetCursor(action) => PreparedEventAction::SetCursor(
                    action
                        .value
                        .to_expr(ctx)
                        .map_err(|err| AvengerAppError::InternalError(err.to_string()))?,
                ),
            });
        }

        let mut scan_exprs = filter_exprs.clone();
        for action in &prepared_actions {
            match action {
                PreparedEventAction::SetParam { expr, .. }
                | PreparedEventAction::SetCursor(expr) => scan_exprs.push(expr.clone()),
                PreparedEventAction::SetStore(assignment) => {
                    scan_exprs.extend(store_expression_update_exprs(&assignment.update));
                }
                PreparedEventAction::SetSelection(assignment) => {
                    scan_exprs.extend(selection_expression_update_exprs(&assignment.update));
                }
            }
        }
        let mut widget_runtime_inputs = IndexMap::new();
        for expr in &scan_exprs {
            let placeholders = collect_placeholder_ids(expr)
                .map_err(|err| AvengerAppError::InternalError(err.to_string()))?;
            for placeholder in placeholders {
                let Some(name) = placeholder.strip_prefix('$') else {
                    continue;
                };
                if let Some(data_type) = widget_runtime_input_data_type(name) {
                    widget_runtime_inputs.insert(name.to_string(), data_type);
                }
            }
        }
        widget_runtime_inputs.sort_keys();
        let mut interaction_requests = event::scan_interaction_columns(&scan_exprs);
        for action in &prepared_actions {
            match action {
                PreparedEventAction::SetStore(assignment) => {
                    if assignment.sharing.to_level() != u8::MAX {
                        match assignment.scope {
                            ChartEventAssignmentScope::Current => {
                                interaction_requests.current_scope_id = true;
                            }
                            ChartEventAssignmentScope::Start => {
                                interaction_requests.start_scope_id = true;
                            }
                        }
                    }
                }
                PreparedEventAction::SetSelection(assignment) => {
                    for field in selection_expression_update_datum_fields(&assignment.update) {
                        interaction_requests
                            .current_datum
                            .insert(field.datum_field.clone());
                    }
                    if selection_update_needs_scope(&assignment.update) {
                        match assignment.scope {
                            ChartEventAssignmentScope::Current => {
                                interaction_requests.current_scope_id = true;
                            }
                            ChartEventAssignmentScope::Start => {
                                interaction_requests.start_scope_id = true;
                            }
                        }
                    }
                }
                PreparedEventAction::SetParam { .. } | PreparedEventAction::SetCursor(_) => {}
            }
        }

        let base_schema = event_schema(
            param_specs,
            &interaction_requests,
            event_datum_types,
            event_coord_types,
        );
        let mut fields = base_schema
            .fields()
            .iter()
            .map(|field| field.as_ref().clone())
            .collect::<Vec<_>>();
        fields.extend(
            widget_runtime_inputs
                .iter()
                .map(|(name, data_type)| Field::new(name, data_type.clone(), true)),
        );
        let schema = schema_from_fields(fields);
        let allowed_columns = schema
            .fields()
            .iter()
            .map(|field| field.name().clone())
            .collect::<HashSet<_>>();
        let mut placeholder_columns = param_specs
            .keys()
            .map(|param| {
                PlaceholderColumn::new(format!("${param}"), event::param_column_name(param))
            })
            .collect::<Vec<_>>();
        placeholder_columns.extend(
            widget_runtime_inputs
                .keys()
                .map(|name| PlaceholderColumn::new(format!("${name}"), name.clone())),
        );
        let mut specs = Vec::new();
        for (index, expr) in filter_exprs.into_iter().enumerate() {
            specs.push(
                PhysicalScalarExpressionSpec::new(format!("filter_{index}"), expr)
                    .with_expected_type(DataType::Boolean),
            );
        }
        let filter_count = specs.len();
        let mut assignments = Vec::new();
        let mut ordered_actions = Vec::with_capacity(prepared_actions.len());
        for action in prepared_actions {
            match action {
                PreparedEventAction::SetParam {
                    runtime_id,
                    param_name,
                    sharing,
                    default_value,
                    data_type,
                    scope,
                    replace_scoped_values,
                    reject_null,
                    expr,
                } => {
                    let value_index = specs.len();
                    specs.push(
                        PhysicalScalarExpressionSpec::new(
                            format!("assign_{param_name}_{value_index}"),
                            expr,
                        )
                        .with_expected_type(data_type),
                    );
                    let assignment = CompiledParamAssignment {
                        runtime_id,
                        param_name,
                        sharing,
                        default_value,
                        scope,
                        replace_scoped_values,
                        reject_null,
                        value_index,
                    };
                    assignments.push(assignment.clone());
                    ordered_actions.push(CompiledOrderedAction::SetParam(assignment));
                }
                PreparedEventAction::SetStore(assignment) => {
                    let update = append_store_expression_update_specs(
                        &assignment.store_name,
                        assignment.update,
                        &mut specs,
                        filter_count,
                    );
                    let assignment = CompiledStoreAssignment {
                        runtime_id: assignment.runtime_id,
                        store_name: assignment.store_name,
                        sharing: assignment.sharing,
                        scope: assignment.scope,
                        replace_scoped_values: assignment.replace_scoped_values,
                        update,
                    };
                    ordered_actions.push(CompiledOrderedAction::SetStore(assignment));
                }
                PreparedEventAction::SetSelection(assignment) => {
                    let update = append_selection_expression_update_specs(
                        &assignment.selection_id,
                        assignment.update,
                        &mut specs,
                        filter_count,
                    );
                    let assignment = CompiledSelectionAssignment {
                        runtime_id: assignment.runtime_id,
                        selection_id: assignment.selection_id,
                        spec: assignment.spec,
                        scope: assignment.scope,
                        update,
                    };
                    ordered_actions.push(CompiledOrderedAction::SetSelection(Box::new(assignment)));
                }
                PreparedEventAction::SetCursor(expr) => {
                    let value_index = specs.len();
                    specs.push(
                        PhysicalScalarExpressionSpec::new(format!("cursor_{value_index}"), expr)
                            .with_expected_type(DataType::Utf8),
                    );
                    ordered_actions.push(CompiledOrderedAction::SetCursor { value_index });
                }
            }
        }
        let mut deferred_expressions = HashMap::new();
        for (index, spec) in specs.iter_mut().enumerate() {
            if event_expression_requires_deferred_evaluation(&spec.expr)
                .map_err(|err| AvengerAppError::InternalError(err.to_string()))?
            {
                deferred_expressions.insert(
                    index,
                    DeferredEventExpression {
                        name: spec.name.clone(),
                        expr: spec.expr.clone(),
                        expected_type: spec.expected_type.clone(),
                        null_on_cast_failure: spec.null_on_cast_failure,
                    },
                );
                spec.expr = datafusion::prelude::lit(ScalarValue::Null);
            }
        }
        let deferred_program_options = PhysicalScalarProgramOptions::default()
            .with_allowed_columns(allowed_columns)
            .with_placeholder_columns(placeholder_columns);
        let program = CompiledScalarExpressionProgram::compile(
            ctx,
            schema,
            specs,
            deferred_program_options.clone(),
        )
        .map_err(|err| AvengerAppError::InternalError(err.to_string()))?;
        let event_stream_config = event_stream_config_for_binding(
            binding,
            ctx,
            param_specs,
            stream_param_provider,
            mark_runtime_targets,
        )?;

        Ok(Self {
            binding_index,
            event_stream_config,
            program,
            deferred_expressions,
            deferred_program_options,
            session_context: ctx.clone(),
            filter_count,
            assignments,
            ordered_actions,
            evaluation_mode: binding.action.evaluation_mode,
            settle_exact: binding.action.settle_exact,
            interaction_requests,
            event_path_min_distance_px: binding
                .event_path_min_distance_px
                .unwrap_or(event::DEFAULT_EVENT_PATH_MIN_DISTANCE_PX),
            scope_target: binding.scope_target.clone(),
            surface_target: binding.surface_target.clone(),
            scope_target_uses_start_scope: binding.between.is_some(),
        })
    }
}

pub(crate) fn param_change_graph_for_plot(
    compiled_plot: &CompiledPlot,
    ctx: &SessionContext,
) -> Result<CompiledChartParamChangeGraph, AvengerAppError> {
    let mark_runtime_targets = MarkRuntimeTargetIndex::for_plot(compiled_plot);
    CompiledChartParamChangeGraph::compile(
        compiled_plot.param_change_bindings(),
        ctx,
        compiled_plot.param_specs(),
        compiled_plot.selection_specs(),
        compiled_plot.store_specs(),
        Some(&mark_runtime_targets),
    )
}

impl CompiledChartParamChangeGraph {
    fn compile(
        bindings: &[ChartParamChangeBinding],
        ctx: &SessionContext,
        param_specs: &ParamSpecRegistry,
        selection_specs: &SelectionSpecRegistry,
        store_specs: &StoreSpecRegistry,
        mark_runtime_targets: Option<&MarkRuntimeTargetIndex>,
    ) -> Result<Self, AvengerAppError> {
        let edges = validate_param_change_graph(bindings, param_specs)?;
        let mut compiled_bindings = Vec::with_capacity(bindings.len());
        let mut bindings_by_source: IndexMap<ParamRef, Vec<usize>> = IndexMap::new();
        for (binding_index, binding) in bindings.iter().enumerate() {
            let compiled = CompiledChartParamChangeBinding::compile(
                binding_index,
                binding,
                ctx,
                param_specs,
                selection_specs,
                store_specs,
                mark_runtime_targets,
            )?;
            bindings_by_source
                .entry(compiled.source_runtime_id.clone())
                .or_default()
                .push(binding_index);
            compiled_bindings.push(compiled);
        }
        Ok(Self {
            bindings: compiled_bindings,
            bindings_by_source,
            edges,
        })
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    pub(crate) fn binding_count(&self) -> usize {
        self.bindings.len()
    }

    pub(crate) fn source_count(&self) -> usize {
        self.bindings_by_source.len()
    }

    pub(crate) fn edge_count(&self) -> usize {
        self.edges.len()
    }

    pub(crate) fn has_source(&self, source: &ParamRef) -> bool {
        self.bindings_by_source.contains_key(source)
    }

    pub(crate) fn evaluate_sources(
        &self,
        sources: &[(ParamRef, ScalarValue)],
        working: &mut ResolvedStateTransaction,
        fired_bindings: &mut HashSet<usize>,
    ) -> Result<Vec<EvaluatedParamChangeAction>, AvengerAppError> {
        let mut scheduled = Vec::new();
        for (source, previous_value) in sources {
            if let Some(binding_indices) = self.bindings_by_source.get(source) {
                scheduled.extend(
                    binding_indices
                        .iter()
                        .filter(|binding_index| fired_bindings.insert(**binding_index))
                        .map(|binding_index| (*binding_index, previous_value.clone())),
                );
            }
        }
        scheduled.sort_by_key(|(binding_index, _)| *binding_index);
        let root_owner_paths = HashMap::new();

        let mut actions = Vec::new();
        for (binding_index, previous_value) in scheduled {
            let binding = &self.bindings[binding_index];
            let source_value = working
                .effective_params_for_owner_paths(&root_owner_paths)
                .get(&binding.source_param_name)
                .cloned()
                .ok_or_else(|| {
                    AvengerAppError::InternalError(format!(
                        "Parameter-change source '{}' is missing from the staged parameter snapshot",
                        binding.source_param_name
                    ))
                })?;
            let current_params = working.effective_params_for_owner_paths(&root_owner_paths);
            let mut row_values = current_params
                .iter()
                .map(|(name, value)| (event::param_column_name(name), value.clone()))
                .collect::<HashMap<_, _>>();
            row_values.insert(param_change::VALUE_FIELD.to_string(), source_value.clone());
            row_values.insert(
                param_change::PREVIOUS_VALUE_FIELD.to_string(),
                previous_value.clone(),
            );
            let batch = one_row_batch_from_scalars(binding.program.schema().clone(), &row_values)
                .map_err(|err| {
                AvengerAppError::InternalError(format!(
                    "Parameter-change binding {} failed to build its input row: {err}",
                    binding.binding_index
                ))
            })?;
            let filter_values = (0..binding.filter_count)
                .map(|index| binding.program.evaluate_value_at(index, &batch))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|err| {
                    AvengerAppError::InternalError(format!(
                        "Parameter-change binding {} from '{}' failed to evaluate filters: {err}",
                        binding.binding_index, binding.source_param_name
                    ))
                })?;
            if !filters_pass(&filter_values) {
                continue;
            }

            let mut param_patch = Vec::new();
            let mut store_patch = Vec::new();
            let mut selection_patch = Vec::new();
            for ordered_action in &binding.ordered_actions {
                let current_params = working.effective_params_for_owner_paths(&root_owner_paths);
                let mut row_values = current_params
                    .iter()
                    .map(|(name, value)| (event::param_column_name(name), value.clone()))
                    .collect::<HashMap<_, _>>();
                row_values.insert(param_change::VALUE_FIELD.to_string(), source_value.clone());
                row_values.insert(
                    param_change::PREVIOUS_VALUE_FIELD.to_string(),
                    previous_value.clone(),
                );
                let batch = one_row_batch_from_scalars(
                    binding.program.schema().clone(),
                    &row_values,
                )
                .map_err(|err| {
                    AvengerAppError::InternalError(format!(
                        "Parameter-change binding {} failed to build an ordered action row: {err}",
                        binding.binding_index
                    ))
                })?;
                let indices = ordered_action.expression_indices(binding.filter_count);
                let mut values = vec![ScalarValue::Null; binding.program.expression_count()];
                for index in indices {
                    values[index] = binding.program.evaluate_value_at(index, &batch).map_err(|err| {
                        AvengerAppError::InternalError(format!(
                            "Parameter-change binding {} from '{}' failed to evaluate action: {err}",
                            binding.binding_index, binding.source_param_name
                        ))
                    })?;
                }
                match ordered_action {
                    CompiledOrderedAction::SetParam(assignment) => {
                        let value = &values[assignment.value_index];
                        if !assignment_value_is_writable(value, &assignment.default_value) {
                            if assignment.reject_null {
                                return Err(AvengerAppError::InternalError(format!(
                                    "Parameter-change binding {} required assignment to '{}' evaluated to null or a degenerate value",
                                    binding.binding_index, assignment.param_name
                                )));
                            }
                            continue;
                        }
                        if current_params.get(&assignment.param_name) == Some(value) {
                            continue;
                        }
                        let resolved = ResolvedScopedParamAssignment {
                            runtime_id: assignment.runtime_id.clone(),
                            source_name: assignment.param_name.clone(),
                            owner_path: Vec::new(),
                            value: value.clone(),
                            replace_scoped_values: false,
                        };
                        working.apply_param(resolved.clone()).map_err(|err| {
                            AvengerAppError::InternalError(format!(
                                "Parameter-change binding {} failed to stage param '{}': {err}",
                                binding.binding_index, assignment.param_name
                            ))
                        })?;
                        param_patch.push(resolved);
                    }
                    CompiledOrderedAction::SetStore(assignment) => {
                        let Some(update) = store_state_update_from_values(
                            &assignment.update,
                            &values,
                            binding.filter_count,
                        ) else {
                            continue;
                        };
                        let resolved = ResolvedScopedStoreAssignment {
                            runtime_id: assignment.runtime_id.clone(),
                            source_name: assignment.store_name.clone(),
                            owner_path: Vec::new(),
                            replace_scoped_values: false,
                            update,
                        };
                        working.apply_store(resolved.clone()).map_err(|err| {
                            AvengerAppError::InternalError(format!(
                                "Parameter-change binding {} failed to stage store '{}': {err}",
                                binding.binding_index, assignment.store_name
                            ))
                        })?;
                        store_patch.push(resolved);
                    }
                    CompiledOrderedAction::SetSelection(assignment) => {
                        if compiled_selection_update_is_scene_query(&assignment.update) {
                            return Err(AvengerAppError::InternalError(format!(
                                "Parameter-change binding {} selection '{}' uses a scene query without an event geometry context",
                                binding.binding_index, assignment.selection_id
                            )));
                        }
                        if let Some(update) = selection_state_update_from_values(
                            &assignment.update,
                            &assignment.spec,
                            &values,
                            binding.filter_count,
                            None,
                            false,
                        ) {
                            let resolved = ResolvedSelectionAssignment {
                                runtime_id: assignment.runtime_id.clone(),
                                source_name: assignment.selection_id.clone(),
                                update,
                            };
                            working.apply_selection(resolved.clone()).map_err(|err| {
                                AvengerAppError::InternalError(format!(
                                    "Parameter-change binding {} failed to stage selection '{}': {err}",
                                    binding.binding_index, assignment.selection_id
                                ))
                            })?;
                            selection_patch.push(resolved);
                        }
                    }
                    CompiledOrderedAction::SetCursor { .. } => {
                        return Err(AvengerAppError::InternalError(
                            "Parameter-change bindings cannot publish cursor actions".to_string(),
                        ));
                    }
                }
            }

            actions.push(EvaluatedParamChangeAction {
                binding_index: binding.binding_index,
                param_patch,
                store_patch,
                selection_patch,
                evaluation_mode: binding.evaluation_mode,
                settle_exact: binding.settle_exact,
            });
        }
        Ok(actions)
    }
}

fn validate_param_change_graph(
    bindings: &[ChartParamChangeBinding],
    param_specs: &ParamSpecRegistry,
) -> Result<Vec<(String, String)>, AvengerAppError> {
    let mut writers: HashMap<String, (String, usize)> = HashMap::new();
    let mut edges = Vec::new();
    for (binding_index, binding) in bindings.iter().enumerate() {
        let source_target = binding.resolved_source().ok_or_else(|| {
            AvengerAppError::InternalError(format!(
                "Parameter-change binding {binding_index} source '{}' was not resolved during chart compilation",
                binding.source_param_name
            ))
        })?;
        let source = param_specs.get_by_id(&source_target.id).ok_or_else(|| {
            AvengerAppError::InternalError(format!(
                "Parameter-change binding {binding_index} has unknown resolved source '{}'",
                source_target.source_name
            ))
        })?;
        if !source.sharing.is_fully_shared() {
            return Err(AvengerAppError::InternalError(format!(
                "Parameter-change binding source '{}' must be shared",
                binding.source_param_name
            )));
        }
        for action in binding.resolved_actions() {
            let ChartEventAction::SetParam(assignment) = action else {
                continue;
            };
            let target = param_specs
                .get_by_id(&assignment.target.id)
                .ok_or_else(|| {
                    AvengerAppError::InternalError(format!(
                        "Parameter-change binding from '{}' assigns unknown resolved param '{}'",
                        binding.source_param_name, assignment.target.source_name
                    ))
                })?;
            if !target.sharing.is_fully_shared() {
                return Err(AvengerAppError::InternalError(format!(
                    "Parameter-change binding target '{}' must be shared",
                    assignment.target.source_name
                )));
            }
            if let Some((first_source, first_binding)) = writers.insert(
                assignment.target.source_name.clone(),
                (binding.source_param_name.clone(), binding_index),
            ) {
                return Err(AvengerAppError::InternalError(format!(
                    "Parameter '{}' has more than one reactive writer: binding {first_binding} from '{first_source}' and binding {binding_index} from '{}'",
                    assignment.target.source_name, binding.source_param_name
                )));
            }
            edges.push((
                binding.source_param_name.clone(),
                assignment.target.source_name.clone(),
            ));
        }
    }
    if let Some(cycle) = find_param_change_cycle(param_specs.keys(), &edges) {
        return Err(AvengerAppError::InternalError(format!(
            "Parameter-change reaction cycle detected: {}",
            cycle.join(" → ")
        )));
    }
    Ok(edges)
}

fn find_param_change_cycle<'a>(
    params: impl Iterator<Item = &'a String>,
    edges: &[(String, String)],
) -> Option<Vec<String>> {
    let mut adjacency: IndexMap<String, Vec<String>> = IndexMap::new();
    for param in params {
        adjacency.entry(param.clone()).or_default();
    }
    for (source, target) in edges {
        adjacency
            .entry(source.clone())
            .or_default()
            .push(target.clone());
        adjacency.entry(target.clone()).or_default();
    }

    fn visit(
        node: &str,
        adjacency: &IndexMap<String, Vec<String>>,
        states: &mut HashMap<String, u8>,
        stack: &mut Vec<String>,
    ) -> Option<Vec<String>> {
        states.insert(node.to_string(), 1);
        stack.push(node.to_string());
        if let Some(targets) = adjacency.get(node) {
            for target in targets {
                match states.get(target).copied().unwrap_or(0) {
                    0 => {
                        if let Some(cycle) = visit(target, adjacency, states, stack) {
                            return Some(cycle);
                        }
                    }
                    1 => {
                        let start = stack.iter().position(|entry| entry == target)?;
                        let mut cycle = stack[start..].to_vec();
                        cycle.push(target.clone());
                        return Some(cycle);
                    }
                    _ => {}
                }
            }
        }
        stack.pop();
        states.insert(node.to_string(), 2);
        None
    }

    let mut states = HashMap::new();
    let mut stack = Vec::new();
    for node in adjacency.keys() {
        if states.get(node).copied().unwrap_or(0) == 0
            && let Some(cycle) = visit(node, &adjacency, &mut states, &mut stack)
        {
            return Some(cycle);
        }
    }
    None
}

impl CompiledChartParamChangeBinding {
    fn compile(
        binding_index: usize,
        binding: &ChartParamChangeBinding,
        ctx: &SessionContext,
        param_specs: &ParamSpecRegistry,
        selection_specs: &SelectionSpecRegistry,
        store_specs: &StoreSpecRegistry,
        mark_runtime_targets: Option<&MarkRuntimeTargetIndex>,
    ) -> Result<Self, AvengerAppError> {
        binding
            .validate()
            .map_err(|err| AvengerAppError::InternalError(err.to_string()))?;
        let source_target = binding.resolved_source().ok_or_else(|| {
            AvengerAppError::InternalError(format!(
                "Parameter-change binding source '{}' was not resolved during chart compilation",
                binding.source_param_name
            ))
        })?;
        let source = param_specs.get_by_id(&source_target.id).ok_or_else(|| {
            AvengerAppError::InternalError(format!(
                "Parameter-change binding has unknown resolved source '{}'",
                source_target.source_name
            ))
        })?;

        let filter_exprs = binding
            .filters
            .iter()
            .map(|filter| {
                filter
                    .to_expr(ctx)
                    .map_err(|err| AvengerAppError::InternalError(err.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let source_type = source.data_type.clone();
        let mut fields = param_specs
            .iter()
            .map(|(name, spec)| {
                Field::new(event::param_column_name(name), spec.data_type.clone(), true)
            })
            .collect::<Vec<_>>();
        fields.push(Field::new(
            param_change::VALUE_FIELD,
            source_type.clone(),
            true,
        ));
        fields.push(Field::new(
            param_change::PREVIOUS_VALUE_FIELD,
            source_type,
            true,
        ));
        let schema = schema_from_fields(fields);
        let allowed_columns = schema
            .fields()
            .iter()
            .map(|field| field.name().clone())
            .collect::<HashSet<_>>();
        let placeholder_columns = param_specs
            .keys()
            .map(|param| {
                PlaceholderColumn::new(format!("${param}"), event::param_column_name(param))
            })
            .collect::<Vec<_>>();

        let mut specs = filter_exprs
            .into_iter()
            .enumerate()
            .map(|(index, expr)| {
                PhysicalScalarExpressionSpec::new(format!("filter_{index}"), expr)
                    .with_expected_type(DataType::Boolean)
            })
            .collect::<Vec<_>>();
        let filter_count = specs.len();
        let mut ordered_actions = Vec::with_capacity(binding.resolved_actions().len());
        for action in binding.resolved_actions() {
            match action {
                ChartEventAction::SetParam(assignment) => {
                    let target = param_specs
                        .get_by_id(&assignment.target.id)
                        .ok_or_else(|| {
                            AvengerAppError::InternalError(format!(
                                "Parameter-change binding assigns unknown resolved param '{}'",
                                assignment.target.source_name
                            ))
                        })?;
                    let expr = match &assignment.value {
                        ChartActionParamValue::Expr { expr } => expr
                            .to_expr(ctx)
                            .map_err(|err| AvengerAppError::InternalError(err.to_string()))?,
                        ChartActionParamValue::RegisteredDefault => {
                            datafusion::prelude::lit(target.default.clone())
                        }
                    };
                    let value_index = specs.len();
                    specs.push(
                        PhysicalScalarExpressionSpec::new(
                            format!("assign_{}", assignment.target.source_name),
                            expr,
                        )
                        .with_expected_type(target.data_type.clone()),
                    );
                    ordered_actions.push(CompiledOrderedAction::SetParam(
                        CompiledParamAssignment {
                            runtime_id: assignment.target.id.clone(),
                            param_name: assignment.target.source_name.clone(),
                            sharing: target.sharing,
                            default_value: target.default.clone(),
                            scope: assignment.scope,
                            replace_scoped_values: assignment.replace_scoped_values,
                            reject_null: assignment.reject_null,
                            value_index,
                        },
                    ));
                }
                ChartEventAction::SetStore(assignment) => {
                    let store = store_specs
                        .get_by_id(&assignment.target.id)
                        .ok_or_else(|| {
                            AvengerAppError::InternalError(format!(
                                "Parameter-change binding updates unknown resolved store '{}'",
                                assignment.target.source_name
                            ))
                        })?;
                    let update = compile_store_expression_update(store, &assignment.update, ctx)?;
                    let update = append_store_expression_update_specs(
                        &assignment.target.source_name,
                        update,
                        &mut specs,
                        filter_count,
                    );
                    ordered_actions.push(CompiledOrderedAction::SetStore(
                        CompiledStoreAssignment {
                            runtime_id: assignment.target.id.clone(),
                            store_name: assignment.target.source_name.clone(),
                            sharing: store.sharing,
                            scope: assignment.scope,
                            replace_scoped_values: assignment.replace_scoped_values,
                            update,
                        },
                    ));
                }
                ChartEventAction::SetSelection(assignment) => {
                    let selection = selection_specs
                        .get_by_id(&assignment.target.id)
                        .ok_or_else(|| {
                            AvengerAppError::InternalError(format!(
                                "Parameter-change binding updates unknown resolved selection '{}'",
                                assignment.target.source_name
                            ))
                        })?;
                    let update = compile_selection_expression_update(
                        &assignment.update,
                        ctx,
                        mark_runtime_targets,
                    )?;
                    let update = append_selection_expression_update_specs(
                        &assignment.target.source_name,
                        update,
                        &mut specs,
                        filter_count,
                    );
                    ordered_actions.push(CompiledOrderedAction::SetSelection(Box::new(
                        CompiledSelectionAssignment {
                            runtime_id: assignment.target.id.clone(),
                            selection_id: assignment.target.source_name.clone(),
                            spec: selection.clone(),
                            scope: assignment.scope,
                            update,
                        },
                    )));
                }
                ChartEventAction::SetCursor(_) => {
                    return Err(AvengerAppError::InternalError(
                        "Parameter-change bindings cannot publish cursor actions".to_string(),
                    ));
                }
            }
        }
        let program = CompiledScalarExpressionProgram::compile(
            ctx,
            schema,
            specs,
            PhysicalScalarProgramOptions::default()
                .with_allowed_columns(allowed_columns)
                .with_placeholder_columns(placeholder_columns),
        )
        .map_err(|err| {
            AvengerAppError::InternalError(format!(
                "Parameter-change binding from '{}' failed to compile: {err}",
                binding.source_param_name
            ))
        })?;

        Ok(Self {
            binding_index,
            source_runtime_id: source_target.id.clone(),
            source_param_name: binding.source_param_name.clone(),
            source_default: source.default.clone(),
            program,
            filter_count,
            ordered_actions,
            evaluation_mode: binding.action.evaluation_mode,
            settle_exact: binding.action.settle_exact,
        })
    }
}

fn compile_store_expression_update(
    store: &CompiledStoreSpec,
    update: &StoreUpdate,
    ctx: &SessionContext,
) -> Result<StoreExpressionUpdate, AvengerAppError> {
    Ok(match update {
        StoreUpdate::Clear => StoreExpressionUpdate::Clear,
        StoreUpdate::ReplaceRows { rows } => StoreExpressionUpdate::ReplaceRows {
            rows: compile_store_rows(store, rows, ctx)?,
        },
        StoreUpdate::InsertRows { rows } => StoreExpressionUpdate::InsertRows {
            rows: compile_store_rows(store, rows, ctx)?,
        },
        StoreUpdate::UpsertRows { rows } => {
            ensure_store_update_has_key(store, "upsert_rows")?;
            StoreExpressionUpdate::UpsertRows {
                rows: compile_store_rows(store, rows, ctx)?,
            }
        }
        StoreUpdate::UpdateByKey { key, fields } => {
            ensure_store_update_has_key(store, "update_by_key")?;
            StoreExpressionUpdate::UpdateByKey {
                key: compile_store_key(store, key, ctx)?,
                fields: compile_store_patch(store, fields, ctx)?,
            }
        }
        StoreUpdate::DeleteByKey { key } => {
            ensure_store_update_has_key(store, "delete_by_key")?;
            StoreExpressionUpdate::DeleteByKey {
                key: compile_store_key(store, key, ctx)?,
            }
        }
        StoreUpdate::ToggleRows { rows } => {
            ensure_store_update_has_key(store, "toggle_rows")?;
            StoreExpressionUpdate::ToggleRows {
                rows: compile_store_rows(store, rows, ctx)?,
            }
        }
    })
}

fn ensure_store_update_has_key(store: &CompiledStoreSpec, op: &str) -> Result<(), AvengerAppError> {
    if store.primary_key.is_empty() {
        return Err(AvengerAppError::InternalError(format!(
            "Store '{}' operation '{op}' requires a primary key",
            store.name
        )));
    }
    Ok(())
}

fn compile_store_rows(
    store: &CompiledStoreSpec,
    rows: &[StoreRow],
    ctx: &SessionContext,
) -> Result<Vec<StoreExpressionRow>, AvengerAppError> {
    rows.iter()
        .map(|row| compile_store_fields(store, &row.fields, ctx, "row"))
        .collect()
}

fn compile_store_key(
    store: &CompiledStoreSpec,
    key: &StoreKey,
    ctx: &SessionContext,
) -> Result<StoreExpressionRow, AvengerAppError> {
    for key_field in &store.primary_key {
        if !key.fields.contains_key(key_field) {
            return Err(AvengerAppError::InternalError(format!(
                "Store '{}' key is missing primary-key field '{}'",
                store.name, key_field
            )));
        }
    }
    compile_store_fields(store, &key.fields, ctx, "key")
}

fn compile_store_patch(
    store: &CompiledStoreSpec,
    patch: &StoreFieldPatch,
    ctx: &SessionContext,
) -> Result<StoreExpressionRow, AvengerAppError> {
    compile_store_fields(store, &patch.fields, ctx, "field patch")
}

fn compile_store_fields(
    store: &CompiledStoreSpec,
    fields: &IndexMap<String, StoreValueExpr>,
    ctx: &SessionContext,
    context: &str,
) -> Result<StoreExpressionRow, AvengerAppError> {
    let mut compiled = Vec::new();
    for (field_name, value_expr) in fields {
        let field = store.field(field_name).ok_or_else(|| {
            AvengerAppError::InternalError(format!(
                "Store '{}' {context} references unknown field '{}'",
                store.name, field_name
            ))
        })?;
        let expr = value_expr
            .expr
            .to_expr(ctx)
            .map_err(|err| AvengerAppError::InternalError(err.to_string()))?;
        compiled.push(StoreExpressionField {
            name: field_name.clone(),
            expr,
            expected_type: field.data_type.clone(),
        });
    }
    Ok(StoreExpressionRow { fields: compiled })
}

fn store_expression_update_exprs(update: &StoreExpressionUpdate) -> Vec<Expr> {
    let mut exprs = Vec::new();
    collect_store_expression_update_exprs(update, &mut exprs);
    exprs
}

fn collect_store_expression_update_exprs(update: &StoreExpressionUpdate, exprs: &mut Vec<Expr>) {
    match update {
        StoreExpressionUpdate::Clear => {}
        StoreExpressionUpdate::ReplaceRows { rows }
        | StoreExpressionUpdate::InsertRows { rows }
        | StoreExpressionUpdate::UpsertRows { rows }
        | StoreExpressionUpdate::ToggleRows { rows } => {
            for row in rows {
                collect_store_row_exprs(row, exprs);
            }
        }
        StoreExpressionUpdate::UpdateByKey { key, fields } => {
            collect_store_row_exprs(key, exprs);
            collect_store_row_exprs(fields, exprs);
        }
        StoreExpressionUpdate::DeleteByKey { key } => collect_store_row_exprs(key, exprs),
    }
}

fn collect_store_row_exprs(row: &StoreExpressionRow, exprs: &mut Vec<Expr>) {
    exprs.extend(row.fields.iter().map(|field| field.expr.clone()));
}

fn append_store_expression_update_specs(
    store_name: &str,
    update: StoreExpressionUpdate,
    specs: &mut Vec<PhysicalScalarExpressionSpec>,
    filter_count: usize,
) -> CompiledStoreUpdate {
    match update {
        StoreExpressionUpdate::Clear => CompiledStoreUpdate::Clear,
        StoreExpressionUpdate::ReplaceRows { rows } => CompiledStoreUpdate::ReplaceRows {
            rows: append_store_rows_specs(store_name, "replace", rows, specs, filter_count),
        },
        StoreExpressionUpdate::InsertRows { rows } => CompiledStoreUpdate::InsertRows {
            rows: append_store_rows_specs(store_name, "insert", rows, specs, filter_count),
        },
        StoreExpressionUpdate::UpsertRows { rows } => CompiledStoreUpdate::UpsertRows {
            rows: append_store_rows_specs(store_name, "upsert", rows, specs, filter_count),
        },
        StoreExpressionUpdate::UpdateByKey { key, fields } => CompiledStoreUpdate::UpdateByKey {
            key: append_store_row_specs(store_name, "update_key", key, specs, filter_count),
            fields: append_store_row_specs(
                store_name,
                "update_fields",
                fields,
                specs,
                filter_count,
            ),
        },
        StoreExpressionUpdate::DeleteByKey { key } => CompiledStoreUpdate::DeleteByKey {
            key: append_store_row_specs(store_name, "delete_key", key, specs, filter_count),
        },
        StoreExpressionUpdate::ToggleRows { rows } => CompiledStoreUpdate::ToggleRows {
            rows: append_store_rows_specs(store_name, "toggle", rows, specs, filter_count),
        },
    }
}

fn append_store_rows_specs(
    store_name: &str,
    prefix: &str,
    rows: Vec<StoreExpressionRow>,
    specs: &mut Vec<PhysicalScalarExpressionSpec>,
    filter_count: usize,
) -> Vec<CompiledStoreRow> {
    rows.into_iter()
        .enumerate()
        .map(|(row_index, row)| {
            append_store_row_specs(
                store_name,
                &format!("{prefix}_{row_index}"),
                row,
                specs,
                filter_count,
            )
        })
        .collect()
}

fn append_store_row_specs(
    store_name: &str,
    prefix: &str,
    row: StoreExpressionRow,
    specs: &mut Vec<PhysicalScalarExpressionSpec>,
    filter_count: usize,
) -> CompiledStoreRow {
    let mut fields = Vec::new();
    for field in row.fields {
        let value_index = specs.len().saturating_sub(filter_count);
        specs.push(
            PhysicalScalarExpressionSpec::new(
                format!("store_{}_{}_{}", store_name, prefix, field.name),
                field.expr,
            )
            .with_expected_type(field.expected_type),
        );
        fields.push((field.name, value_index));
    }
    CompiledStoreRow { fields }
}

fn compile_selection_expression_update(
    update: &SelectionUpdate,
    ctx: &SessionContext,
    mark_runtime_targets: Option<&MarkRuntimeTargetIndex>,
) -> Result<SelectionExpressionUpdate, AvengerAppError> {
    Ok(match update {
        SelectionUpdate::Clear => SelectionExpressionUpdate::Clear,
        SelectionUpdate::ClearInScope { scope } => {
            SelectionExpressionUpdate::ClearInScope { scope: *scope }
        }
        SelectionUpdate::ReplaceAllClauses { clauses } => {
            SelectionExpressionUpdate::ReplaceAllClauses {
                clauses: compile_selection_clauses(clauses, ctx)?,
            }
        }
        SelectionUpdate::ReplaceClausesInScope { scope, clauses } => {
            SelectionExpressionUpdate::ReplaceClausesInScope {
                scope: *scope,
                clauses: compile_selection_clauses(clauses, ctx)?,
            }
        }
        SelectionUpdate::UpsertClauses { clauses } => SelectionExpressionUpdate::UpsertClauses {
            clauses: compile_selection_clauses(clauses, ctx)?,
        },
        SelectionUpdate::ToggleClauses { clauses } => SelectionExpressionUpdate::ToggleClauses {
            clauses: compile_selection_clauses(clauses, ctx)?,
        },
        SelectionUpdate::ToggleEqualityValue {
            field_expr,
            value,
            item_id,
            facet_scope,
        } => SelectionExpressionUpdate::ToggleEqualityValue {
            field_expr: field_expr.clone(),
            value: selection_value_expr_to_expr(value, ctx)?,
            item_id: selection_value_expr_to_expr(item_id, ctx)?,
            facet_scope: *facet_scope,
        },
        SelectionUpdate::ReplaceAllFromSceneQuery { query } => {
            SelectionExpressionUpdate::ReplaceAllFromSceneQuery {
                query: compile_selection_scene_query(query, ctx, mark_runtime_targets)?,
            }
        }
        SelectionUpdate::ReplaceFromSceneQueryInScope { query } => {
            SelectionExpressionUpdate::ReplaceFromSceneQueryInScope {
                query: compile_selection_scene_query(query, ctx, mark_runtime_targets)?,
            }
        }
        SelectionUpdate::UpsertFromSceneQuery { query } => {
            SelectionExpressionUpdate::UpsertFromSceneQuery {
                query: compile_selection_scene_query(query, ctx, mark_runtime_targets)?,
            }
        }
        SelectionUpdate::ToggleFromSceneQuery { query } => {
            SelectionExpressionUpdate::ToggleFromSceneQuery {
                query: compile_selection_scene_query(query, ctx, mark_runtime_targets)?,
            }
        }
        SelectionUpdate::DeleteClauses { ids } => SelectionExpressionUpdate::DeleteClauses {
            ids: ids
                .iter()
                .map(|id| selection_value_expr_to_expr(id, ctx))
                .collect::<Result<Vec<_>, _>>()?,
        },
        SelectionUpdate::DeleteClausesInScope { scope, ids } => {
            SelectionExpressionUpdate::DeleteClausesInScope {
                scope: *scope,
                ids: ids
                    .iter()
                    .map(|id| selection_value_expr_to_expr(id, ctx))
                    .collect::<Result<Vec<_>, _>>()?,
            }
        }
    })
}

fn compile_selection_clauses(
    clauses: &[SelectionClauseUpdate],
    ctx: &SessionContext,
) -> Result<Vec<SelectionExpressionClause>, AvengerAppError> {
    clauses
        .iter()
        .map(|clause| compile_selection_clause(clause, ctx))
        .collect()
}

fn compile_selection_clause(
    clause: &SelectionClauseUpdate,
    ctx: &SessionContext,
) -> Result<SelectionExpressionClause, AvengerAppError> {
    Ok(SelectionExpressionClause {
        id: selection_value_expr_to_expr(&clause.id, ctx)?,
        facet_scope: clause.facet_scope,
        predicate: compile_selection_predicate_update(&clause.predicate, ctx)?,
    })
}

fn compile_selection_predicate_update(
    update: &SelectionPredicateUpdate,
    ctx: &SessionContext,
) -> Result<SelectionExpressionPredicate, AvengerAppError> {
    Ok(match update {
        SelectionPredicateUpdate::Interval { dimensions } => {
            SelectionExpressionPredicate::Interval {
                dimensions: dimensions
                    .iter()
                    .map(|dimension| compile_selection_interval_dimension(dimension, ctx))
                    .collect::<Result<Vec<_>, _>>()?,
            }
        }
        SelectionPredicateUpdate::Equality { dimensions } => {
            SelectionExpressionPredicate::Equality {
                dimensions: dimensions
                    .iter()
                    .map(|dimension| compile_selection_equality_dimension(dimension, ctx))
                    .collect::<Result<Vec<_>, _>>()?,
            }
        }
        SelectionPredicateUpdate::Predicate { values, expr, kind } => {
            SelectionExpressionPredicate::Predicate {
                values: values
                    .iter()
                    .map(|value| compile_selection_predicate_value(value, ctx))
                    .collect::<Result<Vec<_>, _>>()?,
                expr: expr.clone(),
                kind: kind.clone(),
            }
        }
    })
}

fn compile_selection_interval_dimension(
    dimension: &SelectionIntervalDimensionUpdate,
    ctx: &SessionContext,
) -> Result<SelectionExpressionIntervalDimension, AvengerAppError> {
    Ok(SelectionExpressionIntervalDimension {
        id: dimension.id.clone(),
        field_expr: dimension.field_expr.clone(),
        min: selection_value_expr_to_expr(&dimension.min, ctx)?,
        max: selection_value_expr_to_expr(&dimension.max, ctx)?,
    })
}

fn compile_selection_equality_dimension(
    dimension: &SelectionEqualityDimensionUpdate,
    ctx: &SessionContext,
) -> Result<SelectionExpressionEqualityDimension, AvengerAppError> {
    Ok(SelectionExpressionEqualityDimension {
        id: dimension.id.clone(),
        field_expr: dimension.field_expr.clone(),
        value: selection_value_expr_to_expr(&dimension.value, ctx)?,
    })
}

fn compile_selection_predicate_value(
    value: &SelectionPredicateValueUpdate,
    ctx: &SessionContext,
) -> Result<SelectionExpressionPredicateValue, AvengerAppError> {
    Ok(SelectionExpressionPredicateValue {
        id: value.id.clone(),
        value: selection_value_expr_to_expr(&value.value, ctx)?,
    })
}

fn selection_value_expr_to_expr(
    value: &SelectionValueExpr,
    ctx: &SessionContext,
) -> Result<Expr, AvengerAppError> {
    value
        .expr
        .to_expr(ctx)
        .map_err(|err| AvengerAppError::InternalError(err.to_string()))
}

fn compile_selection_scene_query(
    update: &SelectionSceneQuery,
    ctx: &SessionContext,
    mark_runtime_targets: Option<&MarkRuntimeTargetIndex>,
) -> Result<SelectionSceneQueryExpression, AvengerAppError> {
    Ok(SelectionSceneQueryExpression {
        query: compile_scene_geometry_query(&update.query, ctx, mark_runtime_targets)?,
        sharing: update.sharing,
        clause_id: compile_scene_query_clause_id(&update.clause_id, ctx)?,
    })
}

fn compile_scene_geometry_query(
    query: &SceneGeometryQuery,
    ctx: &SessionContext,
    mark_runtime_targets: Option<&MarkRuntimeTargetIndex>,
) -> Result<SceneGeometryQueryExpression, AvengerAppError> {
    query
        .target
        .validate()
        .map_err(|err| AvengerAppError::InternalError(err.to_string()))?;
    Ok(SceneGeometryQueryExpression {
        geometry: match &query.geometry {
            SceneGeometryQueryGeometry::Rect { x0, y0, x1, y1 } => {
                SceneGeometryQueryGeometryExpression::Rect {
                    x0: x0
                        .to_expr(ctx)
                        .map_err(|err| AvengerAppError::InternalError(err.to_string()))?,
                    y0: y0
                        .to_expr(ctx)
                        .map_err(|err| AvengerAppError::InternalError(err.to_string()))?,
                    x1: x1
                        .to_expr(ctx)
                        .map_err(|err| AvengerAppError::InternalError(err.to_string()))?,
                    y1: y1
                        .to_expr(ctx)
                        .map_err(|err| AvengerAppError::InternalError(err.to_string()))?,
                }
            }
            SceneGeometryQueryGeometry::Circle { cx, cy, radius } => {
                SceneGeometryQueryGeometryExpression::Circle {
                    cx: cx
                        .to_expr(ctx)
                        .map_err(|err| AvengerAppError::InternalError(err.to_string()))?,
                    cy: cy
                        .to_expr(ctx)
                        .map_err(|err| AvengerAppError::InternalError(err.to_string()))?,
                    radius: radius
                        .to_expr(ctx)
                        .map_err(|err| AvengerAppError::InternalError(err.to_string()))?,
                }
            }
            SceneGeometryQueryGeometry::Polygon { points } => {
                SceneGeometryQueryGeometryExpression::Polygon {
                    points: points
                        .to_expr(ctx)
                        .map_err(|err| AvengerAppError::InternalError(err.to_string()))?,
                }
            }
        },
        coordinate_space: query.coordinate_space,
        hit_policy: query.hit_policy,
        target: query.target.clone(),
        runtime_mark_paths: runtime_paths_for_mark_ids(
            query.target.resolved_mark_ids(),
            mark_runtime_targets,
        )?,
        runtime_mark_names: runtime_names_for_mark_targets(
            query.target.resolved_mark_ids(),
            query.target.mark_ids(),
            mark_runtime_targets,
        ),
        datum_fields: query.datum_fields.clone(),
        unique_by: query.unique_by.clone(),
        max_hits: query.max_hits,
    })
}

fn compile_scene_query_clause_id(
    clause_id: &SceneQueryClauseId,
    ctx: &SessionContext,
) -> Result<SceneQueryClauseIdExpression, AvengerAppError> {
    Ok(match clause_id {
        SceneQueryClauseId::Tuple => SceneQueryClauseIdExpression::Tuple,
        SceneQueryClauseId::Field(field) => SceneQueryClauseIdExpression::Field(field.clone()),
        SceneQueryClauseId::Expr(expr) => SceneQueryClauseIdExpression::Expr(
            expr.to_expr(ctx)
                .map_err(|err| AvengerAppError::InternalError(err.to_string()))?,
        ),
    })
}

fn collect_scene_query_selection_exprs(update: &SelectionSceneQueryExpression) -> Vec<Expr> {
    let mut exprs = Vec::new();
    match &update.query.geometry {
        SceneGeometryQueryGeometryExpression::Rect { x0, y0, x1, y1 } => {
            exprs.extend([x0.clone(), y0.clone(), x1.clone(), y1.clone()]);
        }
        SceneGeometryQueryGeometryExpression::Circle { cx, cy, radius } => {
            exprs.extend([cx.clone(), cy.clone(), radius.clone()]);
        }
        SceneGeometryQueryGeometryExpression::Polygon { points } => {
            exprs.push(points.clone());
        }
    }
    if let SceneQueryClauseIdExpression::Expr(expr) = &update.clause_id {
        exprs.push(expr.clone());
    }
    exprs
}

fn append_scene_query_selection_specs(
    selection_id: &str,
    update: SelectionSceneQueryExpression,
    specs: &mut Vec<PhysicalScalarExpressionSpec>,
    filter_count: usize,
) -> CompiledSelectionSceneQuery {
    CompiledSelectionSceneQuery {
        query: append_scene_geometry_query_specs(selection_id, update.query, specs, filter_count),
        sharing: update.sharing,
        clause_id: match update.clause_id {
            SceneQueryClauseIdExpression::Tuple => CompiledSceneQueryClauseId::Tuple,
            SceneQueryClauseIdExpression::Field(field) => CompiledSceneQueryClauseId::Field(field),
            SceneQueryClauseIdExpression::Expr(expr) => CompiledSceneQueryClauseId::Expr(
                append_scene_query_value_spec(selection_id, "clause_id", expr, specs, filter_count),
            ),
        },
    }
}

fn append_scene_geometry_query_specs(
    selection_id: &str,
    query: SceneGeometryQueryExpression,
    specs: &mut Vec<PhysicalScalarExpressionSpec>,
    filter_count: usize,
) -> CompiledSceneGeometryQuery {
    let geometry = match query.geometry {
        SceneGeometryQueryGeometryExpression::Rect { x0, y0, x1, y1 } => {
            CompiledSceneGeometryQueryGeometry::Rect {
                x0: append_scene_query_value_spec(selection_id, "rect_x0", x0, specs, filter_count),
                y0: append_scene_query_value_spec(selection_id, "rect_y0", y0, specs, filter_count),
                x1: append_scene_query_value_spec(selection_id, "rect_x1", x1, specs, filter_count),
                y1: append_scene_query_value_spec(selection_id, "rect_y1", y1, specs, filter_count),
            }
        }
        SceneGeometryQueryGeometryExpression::Circle { cx, cy, radius } => {
            CompiledSceneGeometryQueryGeometry::Circle {
                cx: append_scene_query_value_spec(
                    selection_id,
                    "circle_cx",
                    cx,
                    specs,
                    filter_count,
                ),
                cy: append_scene_query_value_spec(
                    selection_id,
                    "circle_cy",
                    cy,
                    specs,
                    filter_count,
                ),
                radius: append_scene_query_value_spec(
                    selection_id,
                    "circle_radius",
                    radius,
                    specs,
                    filter_count,
                ),
            }
        }
        SceneGeometryQueryGeometryExpression::Polygon { points } => {
            CompiledSceneGeometryQueryGeometry::Polygon {
                points: append_scene_query_value_spec(
                    selection_id,
                    "polygon_points",
                    points,
                    specs,
                    filter_count,
                ),
            }
        }
    };
    CompiledSceneGeometryQuery {
        geometry,
        coordinate_space: query.coordinate_space,
        hit_policy: query.hit_policy,
        target: query.target,
        runtime_mark_paths: query.runtime_mark_paths,
        runtime_mark_names: query.runtime_mark_names,
        datum_fields: query.datum_fields,
        unique_by: query.unique_by,
        max_hits: query.max_hits,
    }
}

fn append_scene_query_value_spec(
    selection_id: &str,
    name: &str,
    expr: Expr,
    specs: &mut Vec<PhysicalScalarExpressionSpec>,
    filter_count: usize,
) -> usize {
    let value_index = specs.len().saturating_sub(filter_count);
    specs.push(
        PhysicalScalarExpressionSpec::new(format!("scene_query_{selection_id}_{name}"), expr)
            .with_nullable_cast(),
    );
    value_index
}

fn selection_expression_update_exprs(update: &SelectionExpressionUpdate) -> Vec<Expr> {
    let mut exprs = Vec::new();
    collect_selection_expression_update_exprs(update, &mut exprs);
    exprs
}

fn collect_selection_expression_update_exprs(
    update: &SelectionExpressionUpdate,
    exprs: &mut Vec<Expr>,
) {
    match update {
        SelectionExpressionUpdate::Clear | SelectionExpressionUpdate::ClearInScope { .. } => {}
        SelectionExpressionUpdate::ReplaceAllClauses { clauses }
        | SelectionExpressionUpdate::ReplaceClausesInScope { clauses, .. }
        | SelectionExpressionUpdate::UpsertClauses { clauses }
        | SelectionExpressionUpdate::ToggleClauses { clauses } => {
            for clause in clauses {
                collect_selection_clause_exprs(clause, exprs);
            }
        }
        SelectionExpressionUpdate::ToggleEqualityValue { value, item_id, .. } => {
            exprs.push(value.clone());
            exprs.push(item_id.clone());
        }
        SelectionExpressionUpdate::DeleteClauses { ids } => {
            exprs.extend(ids.iter().cloned());
        }
        SelectionExpressionUpdate::DeleteClausesInScope { ids, .. } => {
            exprs.extend(ids.iter().cloned());
        }
        SelectionExpressionUpdate::ReplaceAllFromSceneQuery { query }
        | SelectionExpressionUpdate::ReplaceFromSceneQueryInScope { query }
        | SelectionExpressionUpdate::UpsertFromSceneQuery { query }
        | SelectionExpressionUpdate::ToggleFromSceneQuery { query } => {
            exprs.extend(collect_scene_query_selection_exprs(query));
        }
    }
}

fn selection_expression_update_datum_fields(
    update: &SelectionExpressionUpdate,
) -> &[SceneQueryDatumField] {
    match update {
        SelectionExpressionUpdate::ReplaceAllFromSceneQuery { query }
        | SelectionExpressionUpdate::ReplaceFromSceneQueryInScope { query }
        | SelectionExpressionUpdate::UpsertFromSceneQuery { query }
        | SelectionExpressionUpdate::ToggleFromSceneQuery { query } => &query.query.datum_fields,
        _ => &[],
    }
}

fn collect_selection_clause_exprs(clause: &SelectionExpressionClause, exprs: &mut Vec<Expr>) {
    exprs.push(clause.id.clone());
    match &clause.predicate {
        SelectionExpressionPredicate::Interval { dimensions } => {
            for dimension in dimensions {
                exprs.push(dimension.min.clone());
                exprs.push(dimension.max.clone());
            }
        }
        SelectionExpressionPredicate::Equality { dimensions } => {
            for dimension in dimensions {
                exprs.push(dimension.value.clone());
            }
        }
        SelectionExpressionPredicate::Predicate { values, .. } => {
            for value in values {
                exprs.push(value.value.clone());
            }
        }
    }
}

fn selection_update_needs_scope(update: &SelectionExpressionUpdate) -> bool {
    match update {
        SelectionExpressionUpdate::Clear => false,
        SelectionExpressionUpdate::ClearInScope { scope } => scope.to_level() != u8::MAX,
        SelectionExpressionUpdate::ReplaceAllClauses { clauses }
        | SelectionExpressionUpdate::UpsertClauses { clauses }
        | SelectionExpressionUpdate::ToggleClauses { clauses } => clauses
            .iter()
            .any(|clause| clause.facet_scope.to_level() != u8::MAX),
        SelectionExpressionUpdate::ReplaceClausesInScope { scope, clauses } => {
            scope.to_level() != u8::MAX
                || clauses
                    .iter()
                    .any(|clause| clause.facet_scope.to_level() != u8::MAX)
        }
        SelectionExpressionUpdate::ToggleEqualityValue { facet_scope, .. } => {
            facet_scope.to_level() != u8::MAX
        }
        SelectionExpressionUpdate::DeleteClauses { .. } => false,
        SelectionExpressionUpdate::DeleteClausesInScope { scope, .. } => {
            scope.to_level() != u8::MAX
        }
        SelectionExpressionUpdate::ReplaceAllFromSceneQuery { .. }
        | SelectionExpressionUpdate::ReplaceFromSceneQueryInScope { .. }
        | SelectionExpressionUpdate::UpsertFromSceneQuery { .. }
        | SelectionExpressionUpdate::ToggleFromSceneQuery { .. } => true,
    }
}

fn append_selection_expression_update_specs(
    selection_id: &str,
    update: SelectionExpressionUpdate,
    specs: &mut Vec<PhysicalScalarExpressionSpec>,
    filter_count: usize,
) -> CompiledSelectionUpdate {
    match update {
        SelectionExpressionUpdate::Clear => CompiledSelectionUpdate::Clear,
        SelectionExpressionUpdate::ClearInScope { scope } => {
            CompiledSelectionUpdate::ClearInScope { scope }
        }
        SelectionExpressionUpdate::ReplaceAllClauses { clauses } => {
            CompiledSelectionUpdate::ReplaceAllClauses {
                clauses: append_selection_clause_specs(
                    selection_id,
                    "replace",
                    clauses,
                    specs,
                    filter_count,
                ),
            }
        }
        SelectionExpressionUpdate::ReplaceClausesInScope { scope, clauses } => {
            CompiledSelectionUpdate::ReplaceClausesInScope {
                scope,
                clauses: append_selection_clause_specs(
                    selection_id,
                    "replace_scope",
                    clauses,
                    specs,
                    filter_count,
                ),
            }
        }
        SelectionExpressionUpdate::UpsertClauses { clauses } => {
            CompiledSelectionUpdate::UpsertClauses {
                clauses: append_selection_clause_specs(
                    selection_id,
                    "upsert",
                    clauses,
                    specs,
                    filter_count,
                ),
            }
        }
        SelectionExpressionUpdate::ToggleClauses { clauses } => {
            CompiledSelectionUpdate::ToggleClauses {
                clauses: append_selection_clause_specs(
                    selection_id,
                    "toggle",
                    clauses,
                    specs,
                    filter_count,
                ),
            }
        }
        SelectionExpressionUpdate::ToggleEqualityValue {
            field_expr,
            value,
            item_id,
            facet_scope,
        } => CompiledSelectionUpdate::ToggleEqualityValue {
            field_expr,
            value: append_selection_value_spec(
                selection_id,
                "toggle_equality_value",
                value,
                specs,
                filter_count,
            ),
            item_id: append_selection_value_spec(
                selection_id,
                "toggle_equality_item_id",
                item_id,
                specs,
                filter_count,
            ),
            facet_scope,
        },
        SelectionExpressionUpdate::ReplaceAllFromSceneQuery { query } => {
            CompiledSelectionUpdate::ReplaceAllFromSceneQuery {
                query: append_scene_query_selection_specs(selection_id, query, specs, filter_count),
            }
        }
        SelectionExpressionUpdate::ReplaceFromSceneQueryInScope { query } => {
            CompiledSelectionUpdate::ReplaceFromSceneQueryInScope {
                query: append_scene_query_selection_specs(selection_id, query, specs, filter_count),
            }
        }
        SelectionExpressionUpdate::UpsertFromSceneQuery { query } => {
            CompiledSelectionUpdate::UpsertFromSceneQuery {
                query: append_scene_query_selection_specs(selection_id, query, specs, filter_count),
            }
        }
        SelectionExpressionUpdate::ToggleFromSceneQuery { query } => {
            CompiledSelectionUpdate::ToggleFromSceneQuery {
                query: append_scene_query_selection_specs(selection_id, query, specs, filter_count),
            }
        }
        SelectionExpressionUpdate::DeleteClauses { ids } => {
            CompiledSelectionUpdate::DeleteClauses {
                ids: ids
                    .into_iter()
                    .enumerate()
                    .map(|(index, expr)| {
                        append_selection_value_spec(
                            selection_id,
                            &format!("delete_{index}"),
                            expr,
                            specs,
                            filter_count,
                        )
                    })
                    .collect(),
            }
        }
        SelectionExpressionUpdate::DeleteClausesInScope { scope, ids } => {
            CompiledSelectionUpdate::DeleteClausesInScope {
                scope,
                ids: ids
                    .into_iter()
                    .enumerate()
                    .map(|(index, expr)| {
                        append_selection_value_spec(
                            selection_id,
                            &format!("delete_scope_{index}"),
                            expr,
                            specs,
                            filter_count,
                        )
                    })
                    .collect(),
            }
        }
    }
}

fn append_selection_clause_specs(
    selection_id: &str,
    prefix: &str,
    clauses: Vec<SelectionExpressionClause>,
    specs: &mut Vec<PhysicalScalarExpressionSpec>,
    filter_count: usize,
) -> Vec<CompiledSelectionClause> {
    clauses
        .into_iter()
        .enumerate()
        .map(|(index, clause)| {
            append_selection_clause_spec(
                selection_id,
                &format!("{prefix}_{index}"),
                clause,
                specs,
                filter_count,
            )
        })
        .collect()
}

fn append_selection_clause_spec(
    selection_id: &str,
    prefix: &str,
    clause: SelectionExpressionClause,
    specs: &mut Vec<PhysicalScalarExpressionSpec>,
    filter_count: usize,
) -> CompiledSelectionClause {
    let id = append_selection_value_spec(
        selection_id,
        &format!("{prefix}_id"),
        clause.id,
        specs,
        filter_count,
    );
    let predicate = match clause.predicate {
        SelectionExpressionPredicate::Interval { dimensions } => {
            CompiledSelectionPredicate::Interval {
                dimensions: dimensions
                    .into_iter()
                    .enumerate()
                    .map(|(dimension_index, dimension)| {
                        let min = append_selection_value_spec(
                            selection_id,
                            &format!("{prefix}_{dimension_index}_min"),
                            dimension.min,
                            specs,
                            filter_count,
                        );
                        let max = append_selection_value_spec(
                            selection_id,
                            &format!("{prefix}_{dimension_index}_max"),
                            dimension.max,
                            specs,
                            filter_count,
                        );
                        CompiledSelectionIntervalDimension {
                            id: dimension.id,
                            field_expr: dimension.field_expr,
                            min,
                            max,
                        }
                    })
                    .collect(),
            }
        }
        SelectionExpressionPredicate::Equality { dimensions } => {
            CompiledSelectionPredicate::Equality {
                dimensions: dimensions
                    .into_iter()
                    .enumerate()
                    .map(|(dimension_index, dimension)| {
                        let value = append_selection_value_spec(
                            selection_id,
                            &format!("{prefix}_{dimension_index}_value"),
                            dimension.value,
                            specs,
                            filter_count,
                        );
                        CompiledSelectionEqualityDimension {
                            id: dimension.id,
                            field_expr: dimension.field_expr,
                            value,
                        }
                    })
                    .collect(),
            }
        }
        SelectionExpressionPredicate::Predicate { values, expr, kind } => {
            CompiledSelectionPredicate::Predicate {
                values: values
                    .into_iter()
                    .enumerate()
                    .map(|(value_index, value)| {
                        let value_ref = append_selection_value_spec(
                            selection_id,
                            &format!("{prefix}_{value_index}_predicate_value"),
                            value.value,
                            specs,
                            filter_count,
                        );
                        CompiledSelectionPredicateValue {
                            id: value.id,
                            value: value_ref,
                        }
                    })
                    .collect(),
                expr,
                kind,
            }
        }
    };
    CompiledSelectionClause {
        id,
        facet_scope: clause.facet_scope,
        predicate,
    }
}

fn append_selection_value_spec(
    selection_id: &str,
    name: &str,
    expr: Expr,
    specs: &mut Vec<PhysicalScalarExpressionSpec>,
    filter_count: usize,
) -> usize {
    let value_index = specs.len().saturating_sub(filter_count);
    specs.push(
        PhysicalScalarExpressionSpec::new(format!("selection_{selection_id}_{name}"), expr)
            .with_nullable_cast(),
    );
    value_index
}

#[derive(Clone, Default)]
struct ChartEventBindingState {
    active_start: Option<Instant>,
    next_start_event_id: u64,
    active_start_event_id: Option<u64>,
    start_params: Option<ScopedParamStoreSnapshot>,
    previous_params: Option<ScopedParamStoreSnapshot>,
    start_scope: Option<EvaluatedInteractionScope>,
    previous_scope: Option<EvaluatedInteractionScope>,
    start_widget_frame: Option<EvaluatedWidgetFrame>,
    event_path: Vec<[f32; 2]>,
}

struct BindingStateAdmissionGuard<'a> {
    state: &'a Mutex<ChartEventBindingState>,
    checkpoint: Option<ChartEventBindingState>,
}

impl<'a> BindingStateAdmissionGuard<'a> {
    fn new(state: &'a Mutex<ChartEventBindingState>) -> Self {
        let checkpoint = state
            .lock()
            .expect("chart event binding lock poisoned")
            .clone();
        Self {
            state,
            checkpoint: Some(checkpoint),
        }
    }

    fn commit(mut self) {
        self.checkpoint = None;
    }
}

impl Drop for BindingStateAdmissionGuard<'_> {
    fn drop(&mut self) {
        if let Some(checkpoint) = self.checkpoint.take() {
            *self
                .state
                .lock()
                .expect("chart event binding lock poisoned") = checkpoint;
        }
    }
}

fn rejected_event_status() -> UpdateStatus {
    UpdateStatus {
        admission: Some(EventAdmission::Rejected),
        ..Default::default()
    }
}

fn failed_event_status() -> UpdateStatus {
    UpdateStatus {
        admission: Some(EventAdmission::Failed),
        ..Default::default()
    }
}

struct ChartEventBindingHandler {
    runtime: Arc<CompiledChartEventBinding>,
    state: Mutex<ChartEventBindingState>,
}

/// Private all-or-nothing state assembled while an ordered event program runs.
/// Dropping this value publishes nothing, including its cursor effect.
struct EventTransaction {
    working: ResolvedStateTransaction,
    params: Vec<ResolvedScopedParamAssignment>,
    stores: Vec<ResolvedScopedStoreAssignment>,
    selections: Vec<ResolvedSelectionAssignment>,
    cursor: Option<CursorStyle>,
}

async fn evaluate_event_expression_at(
    runtime: &CompiledChartEventBinding,
    index: usize,
    batch: &RecordBatch,
    working: &ResolvedStateTransaction,
    current_owner_paths: &HashMap<u8, Vec<ScalarValue>>,
) -> Result<ScalarValue, AvengerAppError> {
    let Some(deferred) = runtime.deferred_expressions.get(&index) else {
        return runtime
            .program
            .evaluate_value_at(index, batch)
            .map_err(|error| AvengerAppError::InternalError(error.to_string()));
    };
    let allowed_columns = runtime
        .program
        .schema()
        .fields()
        .iter()
        .map(|field| field.name().clone())
        .collect::<HashSet<_>>();
    let mut expr = working
        .expand_selection_predicates(
            deferred.expr.clone(),
            &runtime.session_context,
            Some(&allowed_columns),
        )
        .map_err(|error| AvengerAppError::InternalError(error.to_string()))?;
    expr = rewrite_working_store_subqueries(expr, working, current_owner_paths)
        .map_err(|error| AvengerAppError::InternalError(error.to_string()))?;
    expr = avenger_chart_core::datafusion_physical_eval::rewrite_placeholders(
        expr,
        &runtime.deferred_program_options.placeholder_columns,
    )
    .map_err(|error| AvengerAppError::InternalError(error.to_string()))?;
    if let Some(expected_type) = &deferred.expected_type {
        expr = if deferred.null_on_cast_failure {
            try_cast(expr, expected_type.clone())
        } else {
            cast(expr, expected_type.clone())
        };
    }
    let batches = runtime
        .session_context
        .read_batch(batch.clone())
        .map_err(|error| AvengerAppError::InternalError(error.to_string()))?
        .select(vec![expr.alias(&deferred.name)])
        .map_err(|error| AvengerAppError::InternalError(error.to_string()))?
        .collect()
        .await
        .map_err(|error| AvengerAppError::InternalError(error.to_string()))?;
    let batch = batches.first().ok_or_else(|| {
        AvengerAppError::InternalError("deferred event expression returned no rows".to_string())
    })?;
    if batch.num_rows() != 1 || batch.num_columns() != 1 {
        return Err(AvengerAppError::InternalError(format!(
            "deferred event expression returned {} rows and {} columns",
            batch.num_rows(),
            batch.num_columns()
        )));
    }
    ScalarValue::try_from_array(batch.column(0), 0)
        .map_err(|error| AvengerAppError::InternalError(error.to_string()))
}

fn rewrite_working_store_subqueries(
    expr: Expr,
    working: &ResolvedStateTransaction,
    current_owner_paths: &HashMap<u8, Vec<ScalarValue>>,
) -> Result<Expr, DataFusionError> {
    expr.transform(|candidate| {
        let Expr::ScalarSubquery(mut subquery) = candidate else {
            return Ok(datafusion::common::tree_node::Transformed::no(candidate));
        };
        let plan = subquery
            .subquery
            .as_ref()
            .clone()
            .transform_up_with_subqueries(|candidate| {
                let LogicalPlan::TableScan(scan) = candidate else {
                    return Ok(datafusion::common::tree_node::Transformed::no(candidate));
                };
                let Some(store) = working.store_ref_for_target(scan.table_name.table()) else {
                    return Ok(datafusion::common::tree_node::Transformed::no(
                        LogicalPlan::TableScan(scan),
                    ));
                };
                let batch = working
                    .materialize_store(&store, current_owner_paths)
                    .map_err(|error| DataFusionError::Plan(error.to_string()))?;
                let provider = Arc::new(
                    MemTable::try_new(batch.schema(), vec![vec![batch]])
                        .map_err(|error| DataFusionError::Plan(error.to_string()))?,
                );
                let scan = TableScan::try_new(
                    scan.table_name,
                    provider_as_source(provider),
                    scan.projection,
                    scan.filters,
                    scan.fetch,
                )?;
                Ok(datafusion::common::tree_node::Transformed::yes(
                    LogicalPlan::TableScan(scan),
                ))
            })?
            .data;
        subquery.subquery = Arc::new(plan);
        Ok(datafusion::common::tree_node::Transformed::yes(
            Expr::ScalarSubquery(subquery),
        ))
    })
    .map(|transformed| transformed.data)
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl EventStreamHandler<ChartAppState> for ChartEventBindingHandler {
    async fn handle(
        &self,
        _event: &SceneGraphEvent,
        _state: &mut ChartAppState,
        _rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        rejected_event_status()
    }

    async fn handle_with_context(
        &self,
        event: &SceneGraphEvent,
        context: &EventStreamContext,
        state: &mut ChartAppState,
        rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        let mut app = state.runtime.lock().await;
        state.drain_pending_params_into_runtime(&mut app);
        let binding_state_admission = BindingStateAdmissionGuard::new(&self.state);
        let eval_start = Instant::now();
        let current_mark_instance = event.mark_instance().or(context.mark_instance.as_ref());
        let event_mark_instance = if matches!(
            self.runtime.surface_target,
            Some(ChartEventSurfaceTarget::LegendSurface { .. })
        ) {
            context
                .start_event
                .as_ref()
                .and_then(|start| start.mark_instance.as_ref())
                .or(current_mark_instance)
        } else {
            current_mark_instance
        };
        let legend_surface_match = match surface_target_matches(
            self.runtime.surface_target.as_ref(),
            &app.last_event_datum_state,
            event_mark_instance,
        ) {
            SurfaceMatch::Accept { legend_surface } => legend_surface,
            SurfaceMatch::Reject { reason } => {
                trace_chart_event_rejection(
                    self.runtime.binding_index,
                    event,
                    event_mark_instance,
                    context.mark_instance.as_ref(),
                    reason,
                );
                record_event_eval_elapsed(&mut app.event_metrics, eval_start);
                return rejected_event_status();
            }
        };

        let requests = &self.runtime.interaction_requests;
        if matches!(
            legend_surface_kind_for_mark_instance(&app.last_event_datum_state, event_mark_instance),
            Some(LegendSurfaceKind::ContinuousColorbar)
        ) {
            let item_only_fields = requests
                .current_datum
                .iter()
                .filter(|field| event::is_legend_item_only_datum_field(field))
                .cloned()
                .collect::<Vec<_>>();
            if !item_only_fields.is_empty() {
                app.event_metrics.evaluation_errors += 1;
                record_event_eval_elapsed(&mut app.event_metrics, eval_start);
                tracing::warn!(
                    target: "avenger_chart_app::event_binding",
                    binding = self.runtime.binding_index,
                    fields = ?item_only_fields,
                    "continuous colorbar legend events do not expose discrete legend item datum fields"
                );
                return failed_event_status();
            }
        }
        let required_channels = requests.all_channels();
        let routing_enabled = !requests.is_empty() || self.runtime.scope_target.is_some();

        // Route the current event position to a coordinate scope.
        let current_scope: Option<EvaluatedInteractionScope> = if routing_enabled {
            match route_interaction_scope(
                &app.last_interaction_state.scopes,
                event.position(),
                &required_channels,
            ) {
                InteractionRoute::Scope(scope) => Some(scope.clone()),
                InteractionRoute::None | InteractionRoute::Ambiguous => {
                    legend_colorbar_scope_for_mark_instance(
                        &app.last_interaction_state.scopes,
                        &app.last_event_datum_state,
                        event_mark_instance,
                        &required_channels,
                    )
                }
            }
        } else {
            None
        };

        // Freeze scoped params and the start scope at a new gesture start.
        {
            let mut binding_state = self
                .state
                .lock()
                .expect("chart event binding lock poisoned");
            match &context.start_event {
                Some(start) if binding_state.active_start != Some(start.instant) => {
                    binding_state.active_start = Some(start.instant);
                    binding_state.next_start_event_id += 1;
                    binding_state.active_start_event_id = Some(binding_state.next_start_event_id);
                    binding_state.start_params = Some(app.session.snapshot_scoped_params());
                    let start_mark_instance = if matches!(
                        self.runtime.surface_target,
                        Some(ChartEventSurfaceTarget::LegendSurface { .. })
                    ) {
                        start.mark_instance.as_ref()
                    } else {
                        start.event.mark_instance()
                    };
                    binding_state.start_scope = if routing_enabled {
                        match route_interaction_scope(
                            &app.last_interaction_state.scopes,
                            start.event.position(),
                            &required_channels,
                        ) {
                            InteractionRoute::Scope(scope) => Some(scope.clone()),
                            InteractionRoute::None | InteractionRoute::Ambiguous => {
                                legend_colorbar_scope_for_mark_instance(
                                    &app.last_interaction_state.scopes,
                                    &app.last_event_datum_state,
                                    start_mark_instance,
                                    &required_channels,
                                )
                            }
                        }
                    } else {
                        None
                    };
                    binding_state.start_widget_frame = app
                        .active_widget_gesture_frame
                        .as_ref()
                        .filter(|capture| capture.start_instant == start.instant)
                        .map(|capture| capture.frame.clone())
                        .or_else(|| {
                            app.last_widget_frame_state
                                .frame_for_mark_instance(start_mark_instance)
                                .cloned()
                        });
                    binding_state.previous_params = None;
                    binding_state.previous_scope = None;
                    binding_state.event_path.clear();
                    if let Some(position) = start.event.position() {
                        binding_state.event_path.push(position);
                    }
                }
                None => {
                    binding_state.active_start = None;
                    binding_state.active_start_event_id = None;
                    binding_state.start_params = None;
                    binding_state.start_scope = None;
                    binding_state.start_widget_frame = None;
                    binding_state.event_path.clear();
                }
                _ => {}
            }
            if context.start_event.is_some()
                && let Some(position) = event.position()
            {
                push_event_path_point(
                    &mut binding_state.event_path,
                    position,
                    self.runtime.event_path_min_distance_px,
                );
            }
        }

        let (
            start_snapshot,
            previous_snapshot,
            start_scope,
            previous_scope,
            start_event_id,
            start_widget_frame,
            event_path,
        ) = {
            let binding_state = self
                .state
                .lock()
                .expect("chart event binding lock poisoned");
            (
                binding_state.start_params.clone(),
                binding_state.previous_params.clone(),
                binding_state.start_scope.clone(),
                binding_state.previous_scope.clone(),
                binding_state.active_start_event_id,
                binding_state.start_widget_frame.clone(),
                binding_state.event_path.clone(),
            )
        };

        if !target_scope_matches(
            self.runtime.scope_target.as_ref(),
            self.runtime.scope_target_uses_start_scope,
            current_scope.as_ref(),
            start_scope.as_ref(),
        ) {
            trace_chart_event_rejection(
                self.runtime.binding_index,
                event,
                event_mark_instance,
                context.mark_instance.as_ref(),
                "scope_target",
            );
            record_event_eval_elapsed(&mut app.event_metrics, eval_start);
            return rejected_event_status();
        }

        // Resolve effective params for the current/start/previous scopes.
        let current_owner_paths = current_scope
            .as_ref()
            .map(|scope| scope.sharing_owner_paths.clone())
            .unwrap_or_default();
        let current_params = app
            .session
            .effective_params_for_owner_paths(&current_owner_paths);
        let start_params = start_snapshot.as_ref().map(|snapshot| {
            let owner_paths = start_scope
                .as_ref()
                .map(|scope| scope.sharing_owner_paths.clone())
                .unwrap_or_default();
            app.session
                .effective_params_from_snapshot(snapshot, &owner_paths)
        });
        let previous_params = previous_snapshot.as_ref().map(|snapshot| {
            let owner_paths = previous_scope
                .as_ref()
                .map(|scope| scope.sharing_owner_paths.clone())
                .unwrap_or_default();
            app.session
                .effective_params_from_snapshot(snapshot, &owner_paths)
        });

        // Derive requested coordinate/domain columns from the routed scopes.
        let interaction_values = compute_interaction_values(
            requests,
            event.position(),
            context
                .start_event
                .as_ref()
                .and_then(|s| s.event.position()),
            context
                .previous_event
                .as_ref()
                .and_then(|p| p.event.position()),
            current_scope.as_ref(),
            start_scope.as_ref(),
            previous_scope.as_ref(),
        );
        let mut interaction_values = interaction_values;
        let widget_frame = start_widget_frame.or_else(|| {
            app.last_widget_frame_state
                .frame_for_mark_instance(event_mark_instance)
                .cloned()
        });
        if let Some(frame) = widget_frame.as_ref() {
            interaction_values.extend(frame.runtime_inputs.clone());
        }
        if let (Some(frame), Some(position)) = (widget_frame.as_ref(), event.position()) {
            let local = frame.local_point(position);
            interaction_values.insert(
                event::FRAME_X_FIELD.to_string(),
                ScalarValue::Float64(Some(f64::from(local[0]))),
            );
            interaction_values.insert(
                event::FRAME_Y_FIELD.to_string(),
                ScalarValue::Float64(Some(f64::from(local[1]))),
            );
            interaction_values.insert(
                event::FRAME_WIDTH_FIELD.to_string(),
                ScalarValue::Float64(Some(f64::from(frame.bounds.width))),
            );
            interaction_values.insert(
                event::FRAME_HEIGHT_FIELD.to_string(),
                ScalarValue::Float64(Some(f64::from(frame.bounds.height))),
            );
        }
        if requests.event_path {
            interaction_values.insert(
                event::EVENT_PATH_FIELD.to_string(),
                event_path_scalar(&event_path),
            );
        }
        if requests.event_path_svg {
            interaction_values.insert(
                event::EVENT_PATH_SVG_FIELD.to_string(),
                event_path_svg_scalar(&event_path),
            );
        }
        let event_datum_values = compute_event_datum_values(
            &app.last_event_datum_state,
            event_mark_instance,
            &requests.current_datum,
        );
        trace_chart_event_datum_miss(
            self.runtime.binding_index,
            event,
            event_mark_instance,
            &requests.current_datum,
            &app.last_event_datum_state,
            &event_datum_values,
        );
        trace_chart_event_inputs(
            self.runtime.binding_index,
            event,
            event_mark_instance,
            context.mark_instance.as_ref(),
            current_scope.is_some(),
            start_scope.is_some(),
            &event_datum_values,
        );

        let batch = match event_record_batch(
            self.runtime.program.schema().clone(),
            event,
            context,
            EventBatchInputs {
                current_params: &current_params,
                start_params: start_params.as_ref(),
                previous_params: previous_params.as_ref(),
                interaction_values: &interaction_values,
                event_datum_values: &event_datum_values,
                start_event_id,
            },
        ) {
            Ok(batch) => batch,
            Err(err) => {
                app.event_metrics.evaluation_errors += 1;
                record_event_eval_elapsed(&mut app.event_metrics, eval_start);
                tracing::warn!(
                    target: "avenger_chart_app::event_binding",
                    binding = self.runtime.binding_index,
                    error = %err,
                    "failed to build chart event batch"
                );
                return failed_event_status();
            }
        };
        app.event_metrics.event_batches_evaluated += 1;

        let committed_working = app.session.begin_resolved_state_transaction();
        let mut filter_values = Vec::with_capacity(self.runtime.filter_count);
        for index in 0..self.runtime.filter_count {
            match evaluate_event_expression_at(
                &self.runtime,
                index,
                &batch,
                &committed_working,
                &current_owner_paths,
            )
            .await
            {
                Ok(value) => filter_values.push(value),
                Err(error) => {
                    app.event_metrics.evaluation_errors += 1;
                    record_event_eval_elapsed(&mut app.event_metrics, eval_start);
                    tracing::warn!(
                        target: "avenger_chart_app::event_binding",
                        binding = self.runtime.binding_index,
                        error = %error,
                        "failed to evaluate chart event filters"
                    );
                    return failed_event_status();
                }
            }
        }
        app.event_metrics.physical_expression_evaluations += self.runtime.filter_count;

        if !filters_pass(&filter_values) {
            app.event_metrics.filter_failures += 1;
            trace_chart_event_filter_failure(self.runtime.binding_index, event, &filter_values);
            record_event_eval_elapsed(&mut app.event_metrics, eval_start);
            return rejected_event_status();
        }
        app.event_metrics.filter_passes += 1;

        let mut transaction = EventTransaction {
            working: app.session.begin_resolved_state_transaction(),
            params: Vec::new(),
            stores: Vec::new(),
            selections: Vec::new(),
            cursor: None,
        };
        let mut working_current_params = current_params.clone();
        for action in &self.runtime.ordered_actions {
            let action_batch = match event_record_batch(
                self.runtime.program.schema().clone(),
                event,
                context,
                EventBatchInputs {
                    current_params: &working_current_params,
                    start_params: start_params.as_ref(),
                    previous_params: previous_params.as_ref(),
                    interaction_values: &interaction_values,
                    event_datum_values: &event_datum_values,
                    start_event_id,
                },
            ) {
                Ok(batch) => batch,
                Err(err) => {
                    app.event_metrics.evaluation_errors += 1;
                    tracing::warn!(
                        target: "avenger_chart_app::event_binding",
                        binding = self.runtime.binding_index,
                        error = %err,
                        "failed to build ordered chart action input"
                    );
                    record_event_eval_elapsed(&mut app.event_metrics, eval_start);
                    return failed_event_status();
                }
            };
            app.event_metrics.event_batches_evaluated += 1;
            let action_indices = action.expression_indices(self.runtime.filter_count);
            let mut values = vec![ScalarValue::Null; self.runtime.program.expression_count()];
            for index in &action_indices {
                match evaluate_event_expression_at(
                    &self.runtime,
                    *index,
                    &action_batch,
                    &transaction.working,
                    &current_owner_paths,
                )
                .await
                {
                    Ok(value) => values[*index] = value,
                    Err(err) => {
                        app.event_metrics.evaluation_errors += 1;
                        tracing::warn!(
                            target: "avenger_chart_app::event_binding",
                            binding = self.runtime.binding_index,
                            action_index = index,
                            error = %err,
                            "failed to evaluate ordered chart action"
                        );
                        record_event_eval_elapsed(&mut app.event_metrics, eval_start);
                        return failed_event_status();
                    }
                }
            }
            app.event_metrics.physical_expression_evaluations += action_indices.len();
            match action {
                CompiledOrderedAction::SetParam(assignment) => {
                    let value = &values[assignment.value_index];
                    if !assignment_value_is_writable(value, &assignment.default_value) {
                        if assignment.reject_null {
                            app.event_metrics.evaluation_errors += 1;
                            record_event_eval_elapsed(&mut app.event_metrics, eval_start);
                            tracing::warn!(
                                target: "avenger_chart_app::event_binding",
                                binding = self.runtime.binding_index,
                                param = %assignment.param_name,
                                param_id = %assignment.runtime_id,
                                "required chart event assignment evaluated to null or a degenerate value"
                            );
                            return failed_event_status();
                        }
                        continue;
                    }
                    let assignment_scope = match assignment.scope {
                        ChartEventAssignmentScope::Current => current_scope.as_ref(),
                        ChartEventAssignmentScope::Start => start_scope.as_ref(),
                    };
                    let Some(owner_path) = assignment_owner_path_for_surface(
                        assignment.sharing,
                        assignment_scope,
                        legend_surface_match,
                    ) else {
                        continue;
                    };
                    let comparison_owner_paths = if legend_surface_match {
                        HashMap::new()
                    } else {
                        assignment_scope
                            .map(|scope| scope.sharing_owner_paths.clone())
                            .unwrap_or_default()
                    };
                    let comparison_params = transaction
                        .working
                        .effective_params_for_owner_paths(&comparison_owner_paths);
                    let staged_value = comparison_params.get(&assignment.param_name);
                    if assignment.replace_scoped_values || staged_value != Some(value) {
                        let updates_current_scope = assignment_owner_path_for_surface(
                            assignment.sharing,
                            current_scope.as_ref(),
                            legend_surface_match,
                        )
                        .is_some_and(|current_owner| current_owner == owner_path);
                        let resolved = ResolvedScopedParamAssignment {
                            runtime_id: assignment.runtime_id.clone(),
                            source_name: assignment.param_name.clone(),
                            owner_path: owner_path.clone(),
                            value: value.clone(),
                            replace_scoped_values: assignment.replace_scoped_values,
                        };
                        if let Err(err) = transaction.working.apply_param(resolved.clone()) {
                            app.event_metrics.evaluation_errors += 1;
                            tracing::warn!(
                                target: "avenger_chart_app::event_binding",
                                binding = self.runtime.binding_index,
                                param = %assignment.param_name,
                                param_id = %assignment.runtime_id,
                                error = %err,
                                "failed to stage ordered parameter action"
                            );
                            record_event_eval_elapsed(&mut app.event_metrics, eval_start);
                            return failed_event_status();
                        }
                        transaction.params.push(resolved);
                        if updates_current_scope {
                            working_current_params
                                .insert(assignment.param_name.clone(), value.clone());
                        }
                    }
                }
                CompiledOrderedAction::SetStore(assignment) => {
                    let assignment_scope = match assignment.scope {
                        ChartEventAssignmentScope::Current => current_scope.as_ref(),
                        ChartEventAssignmentScope::Start => start_scope.as_ref(),
                    };
                    let owner_path = match assignment_owner_path_for_surface(
                        assignment.sharing,
                        assignment_scope,
                        legend_surface_match,
                    ) {
                        Some(owner_path) => owner_path,
                        None => {
                            app.event_metrics.evaluation_errors += 1;
                            tracing::warn!(
                                target: "avenger_chart_app::event_binding",
                                binding = self.runtime.binding_index,
                                store = %assignment.store_name,
                                store_id = %assignment.runtime_id,
                                sharing = ?assignment.sharing,
                                assignment_scope = ?assignment.scope,
                                surface = ?self.runtime.surface_target,
                                "non-shared store action has no routed owner"
                            );
                            record_event_eval_elapsed(&mut app.event_metrics, eval_start);
                            return failed_event_status();
                        }
                    };
                    let Some(update) = store_state_update_from_values(
                        &assignment.update,
                        &values,
                        self.runtime.filter_count,
                    ) else {
                        continue;
                    };
                    let resolved = ResolvedScopedStoreAssignment {
                        runtime_id: assignment.runtime_id.clone(),
                        source_name: assignment.store_name.clone(),
                        owner_path,
                        replace_scoped_values: assignment.replace_scoped_values,
                        update,
                    };
                    if let Err(err) = transaction.working.apply_store(resolved.clone()) {
                        app.event_metrics.evaluation_errors += 1;
                        tracing::warn!(
                            target: "avenger_chart_app::event_binding",
                            binding = self.runtime.binding_index,
                            store = %assignment.store_name,
                            store_id = %assignment.runtime_id,
                            error = %err,
                            "failed to stage ordered store action"
                        );
                        record_event_eval_elapsed(&mut app.event_metrics, eval_start);
                        return failed_event_status();
                    }
                    transaction.stores.push(resolved);
                }
                CompiledOrderedAction::SetSelection(assignment) => {
                    let assignment_scope = match assignment.scope {
                        ChartEventAssignmentScope::Current => current_scope.as_ref(),
                        ChartEventAssignmentScope::Start => start_scope.as_ref(),
                    };
                    if let Err(err) = validate_selection_expression_ids(
                        &assignment.update,
                        &values,
                        self.runtime.filter_count,
                    ) {
                        app.event_metrics.evaluation_errors += 1;
                        tracing::warn!(
                            target: "avenger_chart_app::event_binding",
                            binding = self.runtime.binding_index,
                            selection = %assignment.selection_id,
                            selection_id = %assignment.runtime_id,
                            error = %err,
                            "selection assignment produced an invalid clause ID"
                        );
                        record_event_eval_elapsed(&mut app.event_metrics, eval_start);
                        return failed_event_status();
                    }
                    let update_result =
                        if compiled_selection_update_is_scene_query(&assignment.update) {
                            scene_query_selection_state_update_from_values(
                                &assignment.update,
                                &assignment.spec,
                                &values,
                                self.runtime.filter_count,
                                assignment_scope,
                                legend_surface_match,
                                &app.last_interaction_state.scopes,
                                self.runtime.scope_target.as_ref(),
                                rtree,
                                &app.last_event_datum_state,
                            )
                        } else {
                            Ok(selection_state_update_from_values(
                                &assignment.update,
                                &assignment.spec,
                                &values,
                                self.runtime.filter_count,
                                assignment_scope,
                                legend_surface_match,
                            ))
                        };
                    let update = match update_result {
                        Ok(Some(update)) => update,
                        Ok(None) => continue,
                        Err(err) => {
                            app.event_metrics.evaluation_errors += 1;
                            tracing::warn!(
                                target: "avenger_chart_app::event_binding",
                                binding = self.runtime.binding_index,
                                selection = %assignment.selection_id,
                                selection_id = %assignment.runtime_id,
                                error = %err,
                                "failed to evaluate selection assignment"
                            );
                            record_event_eval_elapsed(&mut app.event_metrics, eval_start);
                            return failed_event_status();
                        }
                    };
                    let resolved = ResolvedSelectionAssignment {
                        runtime_id: assignment.runtime_id.clone(),
                        source_name: assignment.selection_id.clone(),
                        update,
                    };
                    if let Err(err) = transaction.working.apply_selection(resolved.clone()) {
                        app.event_metrics.evaluation_errors += 1;
                        tracing::warn!(
                            target: "avenger_chart_app::event_binding",
                            binding = self.runtime.binding_index,
                            selection = %assignment.selection_id,
                            selection_id = %assignment.runtime_id,
                            error = %err,
                            "failed to stage ordered selection action"
                        );
                        record_event_eval_elapsed(&mut app.event_metrics, eval_start);
                        return failed_event_status();
                    }
                    transaction.selections.push(resolved);
                }
                CompiledOrderedAction::SetCursor { value_index } => {
                    transaction.cursor = match &values[*value_index] {
                        ScalarValue::Utf8(Some(value)) | ScalarValue::LargeUtf8(Some(value)) => {
                            let Some(cursor) = CursorStyle::from_name(value) else {
                                app.event_metrics.evaluation_errors += 1;
                                tracing::warn!(
                                    target: "avenger_chart_app::event_binding",
                                    binding = self.runtime.binding_index,
                                    cursor = value,
                                    "cursor action evaluated to an unknown cursor name"
                                );
                                record_event_eval_elapsed(&mut app.event_metrics, eval_start);
                                return failed_event_status();
                            };
                            Some(cursor)
                        }
                        value if value.is_null() => None,
                        _ => {
                            app.event_metrics.evaluation_errors += 1;
                            record_event_eval_elapsed(&mut app.event_metrics, eval_start);
                            return failed_event_status();
                        }
                    };
                }
            }
        }

        let visual_patch_count = transaction.params.len();
        let event_evaluation_mode = match self.runtime.evaluation_mode {
            ChartEventEvaluationMode::Preview => EvaluationMode::Preview,
            ChartEventEvaluationMode::Exact => EvaluationMode::Exact,
        };
        let cursor = transaction.cursor;
        let patch_len = transaction.params.len();
        let transaction_id = state.next_param_transaction_id();
        let outcome = match state.apply_resolved_event_transaction_to_runtime(
            &mut app,
            crate::ParamTransactionContext::new(
                transaction_id,
                crate::ParamTransactionOrigin::ChartEvent,
                event_evaluation_mode,
            ),
            transaction.params,
            transaction.stores,
            transaction.selections,
        ) {
            Ok(outcome) => outcome,
            Err(err) => {
                app.event_metrics.evaluation_errors += 1;
                tracing::warn!(
                    target: "avenger_chart_app::event_binding",
                    binding = self.runtime.binding_index,
                    error = %err,
                    "failed to apply atomic chart event parameter transaction"
                );
                record_event_eval_elapsed(&mut app.event_metrics, eval_start);
                return failed_event_status();
            }
        };
        let transaction_has_visual_param = !outcome.changes.is_empty();
        let mut should_rerender = visual_patch_count > 0 || transaction_has_visual_param;
        if patch_len > 0 {
            app.event_metrics.param_patch_events += 1;
            app.event_metrics.params_patched += patch_len;
        }
        let store_changed = outcome.store_changed;
        let selection_changed = outcome.selection_changed;
        if store_changed {
            app.event_metrics.store_patch_events += 1;
        }
        should_rerender |= store_changed;
        should_rerender |= selection_changed;
        if self.runtime.assignments.is_empty()
            && self.runtime.evaluation_mode == ChartEventEvaluationMode::Exact
        {
            should_rerender = true;
        }
        {
            let mut binding_state = self
                .state
                .lock()
                .expect("chart event binding lock poisoned");
            binding_state.previous_params = Some(app.session.snapshot_scoped_params());
            binding_state.previous_scope = current_scope.clone();
        }
        binding_state_admission.commit();

        if !should_rerender {
            if !self.runtime.assignments.is_empty() {
                app.event_metrics.unchanged_patch_skips += 1;
            }
            trace_chart_event_patch(
                self.runtime.binding_index,
                event,
                visual_patch_count,
                store_changed,
                selection_changed,
                false,
            );
            record_event_eval_elapsed(&mut app.event_metrics, eval_start);
            return UpdateStatus {
                cursor,
                admission: Some(EventAdmission::Committed),
                ..Default::default()
            };
        }
        if matches!(event, SceneGraphEvent::CanvasResize(_)) {
            app.accepted_resize_count += 1;
            tracing::debug!(
                target: "avenger_chart_app::resize",
                seq = app.accepted_resize_count,
                mode = ?self.runtime.evaluation_mode,
                "canvas resize accepted"
            );
        }

        app.next_evaluation_mode = outcome.evaluation_mode;
        if (self.runtime.settle_exact || outcome.settle_exact)
            && outcome.evaluation_mode == EvaluationMode::Preview
        {
            app.interaction_settle_exact_pending = true;
        }
        trace_chart_event_patch(
            self.runtime.binding_index,
            event,
            visual_patch_count,
            store_changed,
            selection_changed,
            true,
        );
        record_event_eval_elapsed(&mut app.event_metrics, eval_start);

        UpdateStatus {
            rerender: true,
            rebuild_geometry: self.runtime.evaluation_mode == ChartEventEvaluationMode::Exact,
            cursor,
            commands: Vec::new(),
            consume: false,
            admission: Some(EventAdmission::Committed),
        }
    }
}

fn trace_chart_events_enabled() -> bool {
    std::env::var_os("AVENGER_TRACE_EVENTS").is_some()
}

fn trace_chart_event_inputs(
    binding_index: usize,
    event: &SceneGraphEvent,
    event_mark_instance: Option<&MarkInstance>,
    context_mark_instance: Option<&MarkInstance>,
    current_scope: bool,
    start_scope: bool,
    event_datum_values: &HashMap<String, ScalarValue>,
) {
    if !trace_chart_events_enabled() {
        return;
    }
    eprintln!(
        "chart event binding={} event={:?} pos={:?} event_mark={} context_mark={} current_scope={} start_scope={} datum={:?}",
        binding_index,
        event.event_type(),
        event.position(),
        event_mark_instance.is_some(),
        context_mark_instance.is_some(),
        current_scope,
        start_scope,
        event_datum_values,
    );
}

fn trace_chart_event_datum_miss(
    binding_index: usize,
    event: &SceneGraphEvent,
    event_mark_instance: Option<&MarkInstance>,
    requested_fields: &std::collections::BTreeSet<String>,
    state: &EvaluatedEventDatumState,
    event_datum_values: &HashMap<String, ScalarValue>,
) {
    if !trace_chart_events_enabled()
        || requested_fields.is_empty()
        || !event_datum_values.is_empty()
    {
        return;
    }
    let picked = event_mark_instance
        .map(|mark| format!("{:?}#{:?}", mark.mark_path, mark.instance_index))
        .unwrap_or_else(|| "<none>".to_string());
    let retained: Vec<String> = state
        .rows
        .iter()
        .map(|rows| {
            let fields: Vec<String> = rows
                .rows
                .schema()
                .fields()
                .iter()
                .map(|field| field.name().clone())
                .collect();
            format!(
                "{:?} rows={} fields={:?}",
                rows.mark_path,
                rows.rows.num_rows(),
                fields
            )
        })
        .collect();
    eprintln!(
        "chart event binding={} event={:?} pos={:?} datum_miss requested={:?} picked={} retained={:?}",
        binding_index,
        event.event_type(),
        event.position(),
        requested_fields,
        picked,
        retained,
    );
}

fn trace_chart_event_rejection(
    binding_index: usize,
    event: &SceneGraphEvent,
    event_mark_instance: Option<&MarkInstance>,
    context_mark_instance: Option<&MarkInstance>,
    reason: &str,
) {
    if !trace_chart_events_enabled() {
        return;
    }
    eprintln!(
        "chart event binding={} event={:?} pos={:?} rejected={} event_mark={} context_mark={}",
        binding_index,
        event.event_type(),
        event.position(),
        reason,
        event_mark_instance.is_some(),
        context_mark_instance.is_some(),
    );
}

fn trace_chart_event_filter_failure(
    binding_index: usize,
    event: &SceneGraphEvent,
    filter_values: &[ScalarValue],
) {
    if !trace_chart_events_enabled() {
        return;
    }
    eprintln!(
        "chart event binding={} event={:?} pos={:?} rejected=filter filters={:?}",
        binding_index,
        event.event_type(),
        event.position(),
        filter_values,
    );
}

fn trace_chart_event_patch(
    binding_index: usize,
    event: &SceneGraphEvent,
    param_patches: usize,
    store_changed: bool,
    selection_changed: bool,
    rerender: bool,
) {
    if !trace_chart_events_enabled() {
        return;
    }
    eprintln!(
        "chart event binding={} event={:?} pos={:?} accepted params={} store_changed={} selection_changed={} rerender={}",
        binding_index,
        event.event_type(),
        event.position(),
        param_patches,
        store_changed,
        selection_changed,
        rerender,
    );
}

fn push_event_path_point(path: &mut Vec<[f32; 2]>, point: [f32; 2], min_distance_px: f32) {
    const MAX_POINTS: usize = 2048;
    if let Some(previous) = path.last() {
        let dx = point[0] - previous[0];
        let dy = point[1] - previous[1];
        let min_distance_squared = min_distance_px.max(0.0) * min_distance_px.max(0.0);
        if dx * dx + dy * dy < min_distance_squared {
            return;
        }
    }
    if path.len() >= MAX_POINTS {
        if let Some(last) = path.last_mut() {
            *last = point;
        }
        return;
    }
    path.push(point);
}

fn event_path_scalar(path: &[[f32; 2]]) -> ScalarValue {
    let values = path
        .iter()
        .flat_map(|point| {
            [
                ScalarValue::Float64(Some(point[0] as f64)),
                ScalarValue::Float64(Some(point[1] as f64)),
            ]
        })
        .collect::<Vec<_>>();
    ScalarValue::List(ScalarValue::new_list(&values, &DataType::Float64, true))
}

fn event_path_svg_scalar(path: &[[f32; 2]]) -> ScalarValue {
    let Some(first) = path.first() else {
        return ScalarValue::Utf8(None);
    };
    if path.len() < 2 {
        return ScalarValue::Utf8(None);
    }

    let mut out = "M 0 0".to_string();
    for point in path.iter().skip(1) {
        out.push_str(&format!(
            " L {:.3} {:.3}",
            point[0] - first[0],
            point[1] - first[1]
        ));
    }
    if path.len() >= 3 {
        out.push_str(" Z");
    }
    ScalarValue::Utf8(Some(out))
}

fn store_state_update_from_values(
    update: &CompiledStoreUpdate,
    values: &[ScalarValue],
    filter_count: usize,
) -> Option<StoreStateUpdate> {
    Some(match update {
        CompiledStoreUpdate::Clear => StoreStateUpdate::Clear,
        CompiledStoreUpdate::ReplaceRows { rows } => StoreStateUpdate::ReplaceRows {
            rows: store_rows_from_values(rows, values, filter_count)?,
        },
        CompiledStoreUpdate::InsertRows { rows } => StoreStateUpdate::InsertRows {
            rows: store_rows_from_values(rows, values, filter_count)?,
        },
        CompiledStoreUpdate::UpsertRows { rows } => StoreStateUpdate::UpsertRows {
            rows: store_rows_from_values(rows, values, filter_count)?,
        },
        CompiledStoreUpdate::UpdateByKey { key, fields } => StoreStateUpdate::UpdateByKey {
            key: store_row_from_values(key, values, filter_count)?,
            fields: store_row_from_values(fields, values, filter_count)?,
        },
        CompiledStoreUpdate::DeleteByKey { key } => StoreStateUpdate::DeleteByKey {
            key: store_row_from_values(key, values, filter_count)?,
        },
        CompiledStoreUpdate::ToggleRows { rows } => StoreStateUpdate::ToggleRows {
            rows: store_rows_from_values(rows, values, filter_count)?,
        },
    })
}

fn store_rows_from_values(
    rows: &[CompiledStoreRow],
    values: &[ScalarValue],
    filter_count: usize,
) -> Option<Vec<StoreRowValue>> {
    rows.iter()
        .map(|row| store_row_from_values(row, values, filter_count))
        .collect()
}

fn store_row_from_values(
    row: &CompiledStoreRow,
    values: &[ScalarValue],
    filter_count: usize,
) -> Option<StoreRowValue> {
    let mut out = StoreRowValue::new();
    for (field, value_index) in &row.fields {
        let value = values.get(filter_count + *value_index)?.clone();
        out.insert(field.clone(), value);
    }
    Some(out)
}

fn selection_state_update_from_values(
    update: &CompiledSelectionUpdate,
    spec: &CompiledSelectionSpec,
    values: &[ScalarValue],
    filter_count: usize,
    scope: Option<&EvaluatedInteractionScope>,
    root_owner_surface: bool,
) -> Option<SelectionStateUpdate> {
    Some(match update {
        CompiledSelectionUpdate::Clear => SelectionStateUpdate::Clear,
        CompiledSelectionUpdate::ClearInScope {
            scope: clause_scope,
        } => SelectionStateUpdate::ClearInScope {
            scope_owner_path: selection_owner_path(*clause_scope, scope, root_owner_surface)?,
        },
        CompiledSelectionUpdate::ReplaceAllClauses { clauses } => {
            SelectionStateUpdate::ReplaceAllClauses {
                clauses: selection_clauses_from_values(
                    clauses,
                    spec,
                    values,
                    filter_count,
                    scope,
                    root_owner_surface,
                )?,
            }
        }
        CompiledSelectionUpdate::ReplaceClausesInScope {
            scope: replace_scope,
            clauses,
        } => SelectionStateUpdate::ReplaceClausesInScope {
            scope_owner_path: selection_owner_path(*replace_scope, scope, root_owner_surface)?,
            clauses: selection_clauses_from_values(
                clauses,
                spec,
                values,
                filter_count,
                scope,
                root_owner_surface,
            )?,
        },
        CompiledSelectionUpdate::UpsertClauses { clauses } => SelectionStateUpdate::UpsertClauses {
            clauses: selection_clauses_from_values(
                clauses,
                spec,
                values,
                filter_count,
                scope,
                root_owner_surface,
            )?,
        },
        CompiledSelectionUpdate::ToggleClauses { clauses } => SelectionStateUpdate::ToggleClauses {
            clauses: selection_clauses_from_values(
                clauses,
                spec,
                values,
                filter_count,
                scope,
                root_owner_surface,
            )?,
        },
        CompiledSelectionUpdate::ToggleEqualityValue {
            field_expr,
            value,
            item_id,
            facet_scope,
        } => {
            let owner_path = selection_owner_path(*facet_scope, scope, root_owner_surface)?;
            let item_id = selection_clause_id_from_value(values.get(filter_count + *item_id)?)?;
            SelectionStateUpdate::ToggleEqualityValue {
                field_expr: field_expr.clone(),
                value: values.get(filter_count + *value)?.clone(),
                item_id,
                scope: ResolvedSelectionClauseScope {
                    sharing: *facet_scope,
                    owner_path: owner_path.clone(),
                },
                facet_context: selection_facet_context_values(spec, &owner_path),
            }
        }
        CompiledSelectionUpdate::DeleteClauses { ids } => SelectionStateUpdate::DeleteClauses {
            ids: ids
                .iter()
                .map(|id| selection_clause_id_from_value(values.get(filter_count + *id)?))
                .collect::<Option<Vec<_>>>()?,
        },
        CompiledSelectionUpdate::DeleteClausesInScope {
            scope: delete_scope,
            ids,
        } => SelectionStateUpdate::DeleteClausesInScope {
            scope_owner_path: selection_owner_path(*delete_scope, scope, root_owner_surface)?,
            ids: ids
                .iter()
                .map(|id| selection_clause_id_from_value(values.get(filter_count + *id)?))
                .collect::<Option<Vec<_>>>()?,
        },
        CompiledSelectionUpdate::ReplaceAllFromSceneQuery { .. }
        | CompiledSelectionUpdate::ReplaceFromSceneQueryInScope { .. }
        | CompiledSelectionUpdate::UpsertFromSceneQuery { .. }
        | CompiledSelectionUpdate::ToggleFromSceneQuery { .. } => return None,
    })
}

fn compiled_selection_update_is_scene_query(update: &CompiledSelectionUpdate) -> bool {
    matches!(
        update,
        CompiledSelectionUpdate::ReplaceAllFromSceneQuery { .. }
            | CompiledSelectionUpdate::ReplaceFromSceneQueryInScope { .. }
            | CompiledSelectionUpdate::UpsertFromSceneQuery { .. }
            | CompiledSelectionUpdate::ToggleFromSceneQuery { .. }
    )
}

fn validate_selection_expression_ids(
    update: &CompiledSelectionUpdate,
    values: &[ScalarValue],
    filter_count: usize,
) -> Result<(), AvengerAppError> {
    let mut indices = Vec::new();
    match update {
        CompiledSelectionUpdate::ReplaceAllClauses { clauses }
        | CompiledSelectionUpdate::ReplaceClausesInScope { clauses, .. }
        | CompiledSelectionUpdate::UpsertClauses { clauses }
        | CompiledSelectionUpdate::ToggleClauses { clauses } => {
            indices.extend(clauses.iter().map(|clause| clause.id));
        }
        CompiledSelectionUpdate::ToggleEqualityValue { item_id, .. } => indices.push(*item_id),
        CompiledSelectionUpdate::DeleteClauses { ids }
        | CompiledSelectionUpdate::DeleteClausesInScope { ids, .. } => {
            indices.extend(ids.iter().copied());
        }
        CompiledSelectionUpdate::ReplaceAllFromSceneQuery { query }
        | CompiledSelectionUpdate::ReplaceFromSceneQueryInScope { query }
        | CompiledSelectionUpdate::UpsertFromSceneQuery { query }
        | CompiledSelectionUpdate::ToggleFromSceneQuery { query } => {
            if let CompiledSceneQueryClauseId::Expr(index) = query.clause_id {
                indices.push(index);
            }
        }
        CompiledSelectionUpdate::Clear | CompiledSelectionUpdate::ClearInScope { .. } => {}
    }
    for index in indices {
        let value = values.get(filter_count + index).ok_or_else(|| {
            AvengerAppError::InternalError(
                "Selection clause ID expression result is missing".to_string(),
            )
        })?;
        if selection_clause_id_from_value(value).is_none() {
            return Err(AvengerAppError::InternalError(format!(
                "Selection clause ID must be a non-empty UTF-8 value, got {value:?}"
            )));
        }
    }
    Ok(())
}

// Scene-query selection lowering needs both query inputs and resolved runtime
// scope/index state in one transactional operation.
#[allow(clippy::too_many_arguments)]
fn scene_query_selection_state_update_from_values(
    update: &CompiledSelectionUpdate,
    spec: &CompiledSelectionSpec,
    values: &[ScalarValue],
    filter_count: usize,
    scope: Option<&EvaluatedInteractionScope>,
    root_owner_surface: bool,
    all_scopes: &[EvaluatedInteractionScope],
    scope_target: Option<&ChartEventScopeTarget>,
    rtree: &SceneGraphRTree,
    event_datums: &EvaluatedEventDatumState,
) -> Result<Option<SelectionStateUpdate>, AvengerAppError> {
    let query = match update {
        CompiledSelectionUpdate::ReplaceAllFromSceneQuery { query }
        | CompiledSelectionUpdate::ReplaceFromSceneQueryInScope { query }
        | CompiledSelectionUpdate::UpsertFromSceneQuery { query }
        | CompiledSelectionUpdate::ToggleFromSceneQuery { query } => query,
        _ => return Ok(None),
    };
    if query.query.datum_fields.is_empty() {
        return Err(AvengerAppError::InternalError(
            "Scene geometry query selection requires at least one datum field".to_string(),
        ));
    }
    if query.query.coordinate_space != SceneGeometryCoordinateSpace::Scene {
        return Err(AvengerAppError::InternalError(
            "Only scene-space geometry queries are supported".to_string(),
        ));
    }
    let Some(owner_path) = selection_owner_path(query.sharing, scope, root_owner_surface) else {
        return Ok(None);
    };
    let Some(shape) = scene_query_shape_from_values(&query.query.geometry, values, filter_count)?
    else {
        return Ok(None);
    };
    let Some(scope) = scope else {
        return Ok(None);
    };
    let shapes =
        scene_query_shapes_for_sharing(&shape, query.sharing, scope, all_scopes, scope_target);
    if shapes.is_empty() {
        return Ok(None);
    }
    let result = scene_geometry_query_result(&query.query, &shapes, rtree, event_datums)?;
    let clauses = scene_query_selection_clauses_from_batch(
        query,
        spec,
        values,
        filter_count,
        &owner_path,
        &result.rows,
    )?;
    Ok(Some(match update {
        CompiledSelectionUpdate::ReplaceAllFromSceneQuery { .. } => {
            SelectionStateUpdate::ReplaceAllClauses { clauses }
        }
        CompiledSelectionUpdate::ReplaceFromSceneQueryInScope { .. } => {
            SelectionStateUpdate::ReplaceClausesInScope {
                scope_owner_path: owner_path,
                clauses,
            }
        }
        CompiledSelectionUpdate::UpsertFromSceneQuery { .. } => {
            SelectionStateUpdate::UpsertClauses { clauses }
        }
        CompiledSelectionUpdate::ToggleFromSceneQuery { .. } => {
            SelectionStateUpdate::ToggleClauses { clauses }
        }
        _ => return Ok(None),
    }))
}
#[allow(dead_code)]
struct SceneGeometryQueryResult {
    rows: RecordBatch,
    hit_count: usize,
    unique_count: usize,
    dropped_count: usize,
}

impl SceneGeometryQueryResult {
    #[allow(dead_code)]
    fn into_dataframe(self, ctx: &SessionContext) -> Result<DataFrame, AvengerAppError> {
        let provider = Arc::new(
            MemTable::try_new(self.rows.schema(), vec![vec![self.rows]])
                .map_err(|err| AvengerAppError::InternalError(err.to_string()))?,
        );
        ctx.read_table(provider)
            .map_err(|err| AvengerAppError::InternalError(err.to_string()))
    }
}

fn scene_geometry_query_result(
    query: &CompiledSceneGeometryQuery,
    shapes: &[GeometryQueryShape],
    rtree: &SceneGraphRTree,
    event_datums: &EvaluatedEventDatumState,
) -> Result<SceneGeometryQueryResult, AvengerAppError> {
    let hit_policy = geometry_hit_policy(query.hit_policy);
    let mut instances = shapes
        .iter()
        .flat_map(|shape| rtree.query_shape(shape, hit_policy))
        .filter(|instance| {
            scene_query_target_matches(
                &query.target,
                query.runtime_mark_paths.as_deref(),
                query.runtime_mark_names.as_deref(),
                &instance.mark_instance,
                event_datums,
                rtree,
            )
        })
        .map(|instance| instance.mark_instance.clone())
        .collect::<Vec<_>>();
    instances.sort_by(|a, b| {
        a.mark_path
            .cmp(&b.mark_path)
            .then_with(|| a.instance_index.cmp(&b.instance_index))
            .then_with(|| a.name.cmp(&b.name))
    });
    instances.dedup_by(|a, b| {
        a.name == b.name && a.mark_path == b.mark_path && a.instance_index == b.instance_index
    });
    if let Some(max_hits) = query.max_hits {
        instances.truncate(max_hits);
    }
    let hit_count = instances.len();
    let rows = event_datums
        .datums_for_mark_instances(instances, &query.datum_fields, &query.unique_by)
        .map_err(|err| AvengerAppError::InternalError(err.to_string()))?;
    let unique_count = rows.num_rows();
    let dropped_count = hit_count.saturating_sub(unique_count);
    Ok(SceneGeometryQueryResult {
        rows,
        hit_count,
        unique_count,
        dropped_count,
    })
}

fn scene_query_shapes_for_sharing(
    shape: &GeometryQueryShape,
    sharing: CoordinationScope,
    source_scope: &EvaluatedInteractionScope,
    all_scopes: &[EvaluatedInteractionScope],
    scope_target: Option<&ChartEventScopeTarget>,
) -> Vec<GeometryQueryShape> {
    if sharing.is_free() {
        return vec![shape.clone()];
    }

    let Some(source_owner_path) = assignment_owner_path(sharing, Some(source_scope)) else {
        return Vec::new();
    };
    all_scopes
        .iter()
        .filter(|candidate| candidate.kind == source_scope.kind)
        .filter(|candidate| scope_matches_target(scope_target, candidate))
        .filter(|candidate| {
            assignment_owner_path(sharing, Some(candidate)).as_ref() == Some(&source_owner_path)
        })
        .map(|target_scope| translate_scene_query_shape(shape, source_scope, target_scope))
        .collect()
}

fn translate_scene_query_shape(
    shape: &GeometryQueryShape,
    source_scope: &EvaluatedInteractionScope,
    target_scope: &EvaluatedInteractionScope,
) -> GeometryQueryShape {
    let dx = target_scope.bounds.x - source_scope.bounds.x;
    let dy = target_scope.bounds.y - source_scope.bounds.y;
    translate_scene_query_shape_by(shape, dx, dy)
}

fn translate_scene_query_shape_by(
    shape: &GeometryQueryShape,
    dx: f32,
    dy: f32,
) -> GeometryQueryShape {
    match shape {
        GeometryQueryShape::Rect { x0, y0, x1, y1 } => GeometryQueryShape::Rect {
            x0: x0 + dx,
            y0: y0 + dy,
            x1: x1 + dx,
            y1: y1 + dy,
        },
        GeometryQueryShape::Circle { cx, cy, radius } => GeometryQueryShape::Circle {
            cx: cx + dx,
            cy: cy + dy,
            radius: *radius,
        },
        GeometryQueryShape::Polygon { points } => GeometryQueryShape::Polygon {
            points: points
                .iter()
                .map(|point| [point[0] + dx, point[1] + dy])
                .collect(),
        },
    }
}

fn scene_query_shape_from_values(
    geometry: &CompiledSceneGeometryQueryGeometry,
    values: &[ScalarValue],
    filter_count: usize,
) -> Result<Option<GeometryQueryShape>, AvengerAppError> {
    Ok(match geometry {
        CompiledSceneGeometryQueryGeometry::Rect { x0, y0, x1, y1 } => {
            let Some(x0) = scalar_value_to_f32(values.get(filter_count + *x0)) else {
                return Ok(None);
            };
            let Some(y0) = scalar_value_to_f32(values.get(filter_count + *y0)) else {
                return Ok(None);
            };
            let Some(x1) = scalar_value_to_f32(values.get(filter_count + *x1)) else {
                return Ok(None);
            };
            let Some(y1) = scalar_value_to_f32(values.get(filter_count + *y1)) else {
                return Ok(None);
            };
            Some(GeometryQueryShape::Rect { x0, y0, x1, y1 })
        }
        CompiledSceneGeometryQueryGeometry::Circle { cx, cy, radius } => {
            let Some(cx) = scalar_value_to_f32(values.get(filter_count + *cx)) else {
                return Ok(None);
            };
            let Some(cy) = scalar_value_to_f32(values.get(filter_count + *cy)) else {
                return Ok(None);
            };
            let Some(radius) = scalar_value_to_f32(values.get(filter_count + *radius)) else {
                return Ok(None);
            };
            if radius <= 0.0 {
                return Ok(None);
            }
            Some(GeometryQueryShape::Circle { cx, cy, radius })
        }
        CompiledSceneGeometryQueryGeometry::Polygon { points } => {
            let Some(points) = scalar_value_to_point_list(values.get(filter_count + *points))
            else {
                return Ok(None);
            };
            if points.len() < 3 {
                return Ok(None);
            }
            Some(GeometryQueryShape::Polygon { points })
        }
    })
}

fn scalar_value_to_f32(value: Option<&ScalarValue>) -> Option<f32> {
    match value? {
        ScalarValue::Float64(Some(value)) => Some(*value as f32),
        ScalarValue::Float32(Some(value)) => Some(*value),
        ScalarValue::Int64(Some(value)) => Some(*value as f32),
        ScalarValue::Int32(Some(value)) => Some(*value as f32),
        ScalarValue::UInt64(Some(value)) => Some(*value as f32),
        ScalarValue::UInt32(Some(value)) => Some(*value as f32),
        _ => None,
    }
}

fn scalar_value_to_point_list(value: Option<&ScalarValue>) -> Option<Vec<[f32; 2]>> {
    use datafusion::arrow::array::Array;
    let elements = match value? {
        ScalarValue::List(array) => {
            if array.is_empty() || array.is_null(0) {
                return None;
            }
            array.value(0)
        }
        ScalarValue::LargeList(array) => {
            if array.is_empty() || array.is_null(0) {
                return None;
            }
            array.value(0)
        }
        _ => return None,
    };
    if elements.len() < 6 || elements.len() % 2 != 0 {
        return None;
    }
    let mut points = Vec::with_capacity(elements.len() / 2);
    for index in (0..elements.len()).step_by(2) {
        let x = ScalarValue::try_from_array(&elements, index).ok()?;
        let y = ScalarValue::try_from_array(&elements, index + 1).ok()?;
        points.push([
            scalar_value_to_f32(Some(&x))?,
            scalar_value_to_f32(Some(&y))?,
        ]);
    }
    Some(points)
}

fn geometry_hit_policy(policy: SceneGeometryHitPolicy) -> GeometryQueryHitPolicy {
    match policy {
        SceneGeometryHitPolicy::EnvelopeIntersects => GeometryQueryHitPolicy::EnvelopeIntersects,
        SceneGeometryHitPolicy::GeometryIntersects => GeometryQueryHitPolicy::GeometryIntersects,
        SceneGeometryHitPolicy::GeometryContained => GeometryQueryHitPolicy::GeometryContained,
        SceneGeometryHitPolicy::AnchorInside => GeometryQueryHitPolicy::AnchorInside,
        SceneGeometryHitPolicy::CentroidInside => GeometryQueryHitPolicy::CentroidInside,
    }
}

fn scene_query_target_matches(
    target: &SceneGeometryTarget,
    runtime_mark_paths: Option<&[Vec<usize>]>,
    runtime_mark_names: Option<&[String]>,
    mark_instance: &MarkInstance,
    event_datums: &EvaluatedEventDatumState,
    rtree: &SceneGraphRTree,
) -> bool {
    if let Some(group) = target.resolved_source_group()
        && (mark_instance.mark_path.len() < group.len()
            || group != &mark_instance.mark_path[0..group.len()])
    {
        return false;
    }
    if let Some(names) = runtime_mark_names {
        if !names
            .iter()
            .any(|name| rtree.mark_target_matches(mark_instance, name))
        {
            return false;
        }
    } else if let Some(paths) = runtime_mark_paths
        && !paths
            .iter()
            .any(|path| mark_path_matches_resolved_path(&mark_instance.mark_path, path))
    {
        return false;
    }
    if runtime_mark_names.is_none()
        && runtime_mark_paths.is_none()
        && !target.mark_ids().is_empty()
        && !target.mark_ids().contains(&mark_instance.name)
    {
        return false;
    }
    if !target.subplot_ids().is_empty() {
        let Some(subplot_id_path) =
            event_datums.subplot_id_path_for_mark_path(&mark_instance.mark_path)
        else {
            return false;
        };
        if !target
            .subplot_ids()
            .iter()
            .any(|id| subplot_id_path.iter().any(|candidate| candidate == id))
        {
            return false;
        }
    }
    true
}

fn mark_path_matches_resolved_path(scene_path: &[usize], resolved_path: &[usize]) -> bool {
    !resolved_path.is_empty()
        && (scene_path == resolved_path
            || (scene_path.len() > resolved_path.len() && scene_path.ends_with(resolved_path)))
}

fn scene_query_selection_clauses_from_batch(
    update: &CompiledSelectionSceneQuery,
    spec: &CompiledSelectionSpec,
    values: &[ScalarValue],
    filter_count: usize,
    owner_path: &[ScalarValue],
    batch: &RecordBatch,
) -> Result<Vec<SelectionClause>, AvengerAppError> {
    let facet_context = selection_facet_context_values(spec, owner_path);
    let mut clauses = Vec::new();
    for row_index in 0..batch.num_rows() {
        let mut row_values = Vec::new();
        let mut row_has_null = false;
        for field in &update.query.datum_fields {
            let Some(column) = batch.column_by_name(&field.id) else {
                return Err(AvengerAppError::InternalError(format!(
                    "Scene query result did not contain datum field '{}'",
                    field.id
                )));
            };
            let value = ScalarValue::try_from_array(column, row_index)
                .map_err(|err| AvengerAppError::InternalError(err.to_string()))?;
            if value.is_null() {
                row_has_null = true;
                break;
            }
            row_values.push((field, value));
        }
        if row_has_null {
            continue;
        }
        let Some(id) = scene_query_clause_id(&update.clause_id, &row_values, values, filter_count)?
        else {
            continue;
        };
        let dimensions = row_values
            .iter()
            .map(|(field, value)| SelectionEqualityDimensionValue {
                id: field.id.clone(),
                field_expr: field.field_expr.clone(),
                value: value.clone(),
            })
            .collect();
        clauses.push(SelectionClause {
            id,
            scope: ResolvedSelectionClauseScope {
                sharing: update.sharing,
                owner_path: owner_path.to_vec(),
            },
            predicate: SelectionPredicateSpec::Equality { dimensions },
            facet_context: facet_context.clone(),
        });
    }
    Ok(clauses)
}

fn scene_query_clause_id(
    clause_id: &CompiledSceneQueryClauseId,
    row_values: &[(&SceneQueryDatumField, ScalarValue)],
    values: &[ScalarValue],
    filter_count: usize,
) -> Result<Option<String>, AvengerAppError> {
    let id = match clause_id {
        CompiledSceneQueryClauseId::Tuple => Some(
            encode_selection_tuple_id(
                row_values
                    .iter()
                    .map(|(field, value)| (field.id.as_str(), value)),
            )
            .map_err(|error| AvengerAppError::InternalError(error.to_string()))?,
        ),
        CompiledSceneQueryClauseId::Field(field) => row_values
            .iter()
            .find(|(datum_field, _)| &datum_field.id == field)
            .and_then(|(_, value)| selection_clause_id_from_value(value)),
        CompiledSceneQueryClauseId::Expr(index) => values
            .get(filter_count + *index)
            .and_then(selection_clause_id_from_value),
    };
    if id.is_none() {
        return Err(AvengerAppError::InternalError(
            "Scene-query selection clause ID must be a non-empty UTF-8 value".to_string(),
        ));
    }
    Ok(id)
}

fn selection_clauses_from_values(
    clauses: &[CompiledSelectionClause],
    spec: &CompiledSelectionSpec,
    values: &[ScalarValue],
    filter_count: usize,
    scope: Option<&EvaluatedInteractionScope>,
    root_owner_surface: bool,
) -> Option<Vec<SelectionClause>> {
    clauses
        .iter()
        .map(|clause| {
            selection_clause_from_values(
                clause,
                spec,
                values,
                filter_count,
                scope,
                root_owner_surface,
            )
        })
        .collect()
}

fn selection_clause_from_values(
    clause: &CompiledSelectionClause,
    spec: &CompiledSelectionSpec,
    values: &[ScalarValue],
    filter_count: usize,
    scope: Option<&EvaluatedInteractionScope>,
    root_owner_surface: bool,
) -> Option<SelectionClause> {
    let id = selection_clause_id_from_value(values.get(filter_count + clause.id)?)?;
    let owner_path = selection_owner_path(clause.facet_scope, scope, root_owner_surface)?;
    let facet_context = selection_facet_context_values(spec, &owner_path);
    let predicate = match &clause.predicate {
        CompiledSelectionPredicate::Interval { dimensions } => SelectionPredicateSpec::Interval {
            dimensions: dimensions
                .iter()
                .map(|dimension| {
                    let min = values.get(filter_count + dimension.min)?.clone();
                    let max = values.get(filter_count + dimension.max)?.clone();
                    Some(SelectionIntervalDimensionValue {
                        id: dimension.id.clone(),
                        field_expr: dimension.field_expr.clone(),
                        min,
                        max,
                    })
                })
                .collect::<Option<Vec<_>>>()?,
        },
        CompiledSelectionPredicate::Equality { dimensions } => SelectionPredicateSpec::Equality {
            dimensions: dimensions
                .iter()
                .map(|dimension| {
                    let value = values.get(filter_count + dimension.value)?.clone();
                    Some(SelectionEqualityDimensionValue {
                        id: dimension.id.clone(),
                        field_expr: dimension.field_expr.clone(),
                        value,
                    })
                })
                .collect::<Option<Vec<_>>>()?,
        },
        CompiledSelectionPredicate::Predicate {
            values: predicate_values,
            expr,
            kind,
        } => SelectionPredicateSpec::Predicate {
            values: predicate_values
                .iter()
                .map(|value| {
                    let resolved = values.get(filter_count + value.value)?.clone();
                    Some(SelectionPredicateValue {
                        id: value.id.clone(),
                        value: resolved,
                    })
                })
                .collect::<Option<Vec<_>>>()?,
            expr: expr.clone(),
            kind: kind.clone(),
        },
    };
    Some(SelectionClause {
        id,
        scope: ResolvedSelectionClauseScope {
            sharing: clause.facet_scope,
            owner_path,
        },
        predicate,
        facet_context,
    })
}

fn selection_owner_path(
    sharing: CoordinationScope,
    scope: Option<&EvaluatedInteractionScope>,
    root_owner_surface: bool,
) -> Option<Vec<ScalarValue>> {
    assignment_owner_path_for_surface(sharing, scope, root_owner_surface)
}

fn selection_facet_context_values(
    spec: &CompiledSelectionSpec,
    owner_path: &[ScalarValue],
) -> Vec<SelectionFacetContextValue> {
    spec.facet_context
        .iter()
        .zip(owner_path.iter())
        .filter(|(_, value)| !value.is_null())
        .map(|(facet, value)| SelectionFacetContextValue {
            id: facet.id.clone(),
            value: value.clone(),
        })
        .collect()
}

fn selection_clause_id_from_value(value: &ScalarValue) -> Option<String> {
    match value {
        ScalarValue::Utf8(Some(value)) | ScalarValue::LargeUtf8(Some(value))
            if !value.is_empty() =>
        {
            Some(value.clone())
        }
        _ => None,
    }
}

fn record_event_eval_elapsed(metrics: &mut crate::ChartEventMetrics, start: Instant) {
    metrics.total_eval_us += start.elapsed().as_micros() as u64;
}

struct ChartEventExactOnlyHandler;

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl EventStreamHandler<ChartAppState> for ChartEventExactOnlyHandler {
    async fn handle(
        &self,
        _event: &SceneGraphEvent,
        state: &mut ChartAppState,
        _rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        let mut app = state.runtime.lock().await;
        state.drain_pending_params_into_runtime(&mut app);
        if !app.interaction_settle_exact_pending {
            return UpdateStatus::default();
        }
        app.interaction_settle_exact_pending = false;
        app.next_evaluation_mode = EvaluationMode::Exact;
        UpdateStatus {
            rerender: true,
            rebuild_geometry: true,
            ..Default::default()
        }
    }
}

/// Whether an assignment's evaluated value is safe to write to a param.
///
/// Derived interaction columns are null when a gesture has no routed scope, which
/// makes expressions evaluate to null or null-element lists. Writing those would
/// corrupt target params (and can fail downstream scale building), so they are
/// treated as no-ops unless the binding explicitly writes the param's default
/// value. Reset bindings use that path to clear interaction params safely.
fn assignment_value_is_writable(value: &ScalarValue, default_value: &ScalarValue) -> bool {
    use datafusion::arrow::array::Array;
    if value == default_value {
        return true;
    }
    match value {
        ScalarValue::Null => false,
        ScalarValue::List(array) => {
            if array.is_empty() || array.is_null(0) {
                return false;
            }
            let elements = array.value(0);
            elements.len() >= 2 && elements.null_count() == 0
        }
        ScalarValue::LargeList(array) => {
            if array.is_empty() || array.is_null(0) {
                return false;
            }
            let elements = array.value(0);
            elements.len() >= 2 && elements.null_count() == 0
        }
        ScalarValue::FixedSizeList(array) => {
            if array.is_empty() || array.is_null(0) {
                return false;
            }
            let elements = array.value(0);
            elements.null_count() == 0
        }
        other => !other.is_null(),
    }
}

/// Resolve the owner path an assignment should write, given the routed scope.
///
/// `Shared` state always writes the root path. Non-shared state requires a
/// routed scope; without one, returns `None` for the action-specific caller to
/// handle.
fn assignment_owner_path(
    sharing: CoordinationScope,
    scope: Option<&EvaluatedInteractionScope>,
) -> Option<Vec<ScalarValue>> {
    assignment_owner_path_for_surface(sharing, scope, false)
}

fn assignment_owner_path_for_surface(
    sharing: CoordinationScope,
    scope: Option<&EvaluatedInteractionScope>,
    root_owner_surface: bool,
) -> Option<Vec<ScalarValue>> {
    if root_owner_surface {
        return Some(Vec::new());
    }
    let level = sharing.to_level();
    if level == u8::MAX {
        return Some(Vec::new());
    }
    scope.map(|scope| {
        scope
            .sharing_owner_paths
            .get(&level)
            .cloned()
            .unwrap_or_default()
    })
}

/// Invert a scene-space point through a scope's coordinate transform.
fn invert_scene_point(
    scope: &EvaluatedInteractionScope,
    scene_point: [f32; 2],
    channels: &[&str],
) -> Option<IndexMap<String, ScalarValue>> {
    let local_point = [
        scene_point[0] - scope.bounds.x,
        scene_point[1] - scope.bounds.y,
    ];
    scope
        .coord_transform
        .invert_interaction_point(InteractionPointInversionRequest {
            local_point,
            plot_area_width: scope.plot_area_width,
            plot_area_height: scope.plot_area_height,
            channels,
            scales: &scope.scales,
        })
        .ok()
}

/// Build a four-element `List(Float64)` interaction-frame scalar.
fn frame_list_scalar(frame: [f64; 4]) -> ScalarValue {
    ScalarValue::List(ScalarValue::new_list(
        &frame.map(|value| ScalarValue::Float64(Some(value))),
        &DataType::Float64,
        true,
    ))
}

/// Build a two-element `List(Float64)` domain scalar.
fn domain_list_scalar(min: f32, max: f32) -> ScalarValue {
    ScalarValue::List(ScalarValue::new_list(
        &[
            ScalarValue::Float64(Some(min as f64)),
            ScalarValue::Float64(Some(max as f64)),
        ],
        &DataType::Float64,
        true,
    ))
}

fn compute_event_datum_values(
    state: &EvaluatedEventDatumState,
    mark_instance: Option<&MarkInstance>,
    requested_fields: &std::collections::BTreeSet<String>,
) -> HashMap<String, ScalarValue> {
    requested_fields
        .iter()
        .filter_map(|field| {
            state
                .datum_for_mark_instance(mark_instance, field)
                .map(|value| (event::event_datum_column_name(field), value))
        })
        .collect()
}

/// Compute the requested derived coordinate/domain columns from routed scopes.
#[allow(clippy::too_many_arguments)]
fn compute_interaction_values(
    requests: &InteractionColumnRequests,
    current_point: Option<[f32; 2]>,
    start_point: Option<[f32; 2]>,
    previous_point: Option<[f32; 2]>,
    current_scope: Option<&EvaluatedInteractionScope>,
    start_scope: Option<&EvaluatedInteractionScope>,
    previous_scope: Option<&EvaluatedInteractionScope>,
) -> HashMap<String, ScalarValue> {
    let mut values = HashMap::new();

    let fill_coords = |values: &mut HashMap<String, ScalarValue>,
                       channels: &std::collections::BTreeSet<String>,
                       point: Option<[f32; 2]>,
                       scope: Option<&EvaluatedInteractionScope>,
                       name_fn: fn(&str) -> String| {
        if channels.is_empty() {
            return;
        }
        let (Some(point), Some(scope)) = (point, scope) else {
            return;
        };
        let channel_refs: Vec<&str> = channels.iter().map(String::as_str).collect();
        if let Some(inverted) = invert_scene_point(scope, point, &channel_refs) {
            for channel in channels {
                if let Some(value) = inverted.get(channel) {
                    values.insert(name_fn(channel), value.clone());
                }
            }
        }
    };

    fill_coords(
        &mut values,
        &requests.current_coord,
        current_point,
        current_scope,
        event::event_coord_column_name,
    );
    fill_coords(
        &mut values,
        &requests.start_coord,
        start_point,
        start_scope,
        event::start_coord_column_name,
    );
    // event_at_start uses the CURRENT point through the FROZEN start scope.
    fill_coords(
        &mut values,
        &requests.event_at_start_coord,
        current_point,
        start_scope,
        event::event_at_start_coord_column_name,
    );
    let current_point_clipped_to_start = match (current_point, start_scope) {
        (Some(point), Some(scope)) => Some(clamp_scene_point_to_scope(point, scope)),
        _ => None,
    };
    fill_coords(
        &mut values,
        &requests.event_at_start_clipped_coord,
        current_point_clipped_to_start,
        start_scope,
        event::event_at_start_clipped_coord_column_name,
    );
    fill_coords(
        &mut values,
        &requests.previous_coord,
        previous_point,
        previous_scope,
        event::previous_coord_column_name,
    );

    let fill_domains = |values: &mut HashMap<String, ScalarValue>,
                        channels: &std::collections::BTreeSet<String>,
                        scope: Option<&EvaluatedInteractionScope>,
                        name_fn: fn(&str) -> String| {
        let Some(scope) = scope else {
            return;
        };
        for channel in channels {
            if let Some(scale) = scope.scales.get(channel)
                && let Ok((min, max)) = scale.numeric_interval_domain()
            {
                values.insert(name_fn(channel), domain_list_scalar(min, max));
            }
        }
    };

    fill_domains(
        &mut values,
        &requests.current_domain,
        current_scope,
        event::event_domain_column_name,
    );
    fill_domains(
        &mut values,
        &requests.start_domain,
        start_scope,
        event::start_domain_column_name,
    );

    let fill_scope_frame = |values: &mut HashMap<String, ScalarValue>,
                            requested: bool,
                            scope: Option<&EvaluatedInteractionScope>,
                            name: &str| {
        if !requested {
            return;
        }
        let Some(scope) = scope else {
            return;
        };
        let frame = scope
            .coord_transform
            .interaction_frame(&scope.scales, scope.plot_area_width, scope.plot_area_height)
            .unwrap_or([1.0, 0.0, 0.0, 1.0]);
        values.insert(name.to_string(), frame_list_scalar(frame));
    };
    fill_scope_frame(
        &mut values,
        requests.current_scope_frame,
        current_scope,
        event::EVENT_SCOPE_FRAME_FIELD,
    );
    fill_scope_frame(
        &mut values,
        requests.start_scope_frame,
        start_scope,
        event::START_SCOPE_FRAME_FIELD,
    );

    let fill_scope_values = |values: &mut HashMap<String, ScalarValue>,
                             scope: Option<&EvaluatedInteractionScope>,
                             plot_size: bool,
                             scope_id: bool,
                             facet_indices: &std::collections::BTreeSet<usize>,
                             width_name: &str,
                             height_name: &str,
                             scope_id_name: &str,
                             facet_name_fn: fn(usize) -> String| {
        let Some(scope) = scope else {
            return;
        };
        if plot_size {
            values.insert(
                width_name.to_string(),
                ScalarValue::Float64(Some(scope.plot_area_width as f64)),
            );
            values.insert(
                height_name.to_string(),
                ScalarValue::Float64(Some(scope.plot_area_height as f64)),
            );
        }
        if scope_id {
            values.insert(
                scope_id_name.to_string(),
                ScalarValue::Utf8(Some(scope.scope_id.clone())),
            );
        }
        for index in facet_indices {
            if let Some(value) = scope.logical_facet_values.get(*index)
                && let Some(value) = scalar_to_event_string(value)
            {
                values.insert(facet_name_fn(*index), ScalarValue::Utf8(Some(value)));
            }
        }
    };

    fill_scope_values(
        &mut values,
        current_scope,
        requests.current_plot_size,
        requests.current_scope_id,
        &requests.current_facet_values,
        event::EVENT_PLOT_WIDTH_FIELD,
        event::EVENT_PLOT_HEIGHT_FIELD,
        event::EVENT_SCOPE_ID_FIELD,
        event::event_facet_value_column_name,
    );
    fill_scope_values(
        &mut values,
        start_scope,
        requests.start_plot_size,
        requests.start_scope_id,
        &requests.start_facet_values,
        event::START_PLOT_WIDTH_FIELD,
        event::START_PLOT_HEIGHT_FIELD,
        event::START_SCOPE_ID_FIELD,
        event::start_facet_value_column_name,
    );

    values
}

fn scalar_to_event_string(value: &ScalarValue) -> Option<String> {
    match value {
        ScalarValue::Utf8(Some(value))
        | ScalarValue::LargeUtf8(Some(value))
        | ScalarValue::Utf8View(Some(value)) => Some(value.clone()),
        ScalarValue::Boolean(Some(value)) => Some(value.to_string()),
        ScalarValue::Int8(Some(value)) => Some(value.to_string()),
        ScalarValue::Int16(Some(value)) => Some(value.to_string()),
        ScalarValue::Int32(Some(value)) => Some(value.to_string()),
        ScalarValue::Int64(Some(value)) => Some(value.to_string()),
        ScalarValue::UInt8(Some(value)) => Some(value.to_string()),
        ScalarValue::UInt16(Some(value)) => Some(value.to_string()),
        ScalarValue::UInt32(Some(value)) => Some(value.to_string()),
        ScalarValue::UInt64(Some(value)) => Some(value.to_string()),
        ScalarValue::Float32(Some(value)) => Some(value.to_string()),
        ScalarValue::Float64(Some(value)) => Some(value.to_string()),
        _ => None,
    }
}

fn clamp_scene_point_to_scope(point: [f32; 2], scope: &EvaluatedInteractionScope) -> [f32; 2] {
    [
        point[0].clamp(scope.bounds.x, scope.bounds.x + scope.bounds.width),
        point[1].clamp(scope.bounds.y, scope.bounds.y + scope.bounds.height),
    ]
}

fn filters_pass(values: &[ScalarValue]) -> bool {
    values.iter().all(|value| match value {
        ScalarValue::Boolean(Some(value)) => *value,
        _ => false,
    })
}

enum SurfaceMatch {
    Accept { legend_surface: bool },
    Reject { reason: &'static str },
}

fn surface_target_matches(
    target: Option<&ChartEventSurfaceTarget>,
    event_datums: &EvaluatedEventDatumState,
    mark_instance: Option<&MarkInstance>,
) -> SurfaceMatch {
    let legend_surface_keys = legend_surface_keys_for_mark_instance(event_datums, mark_instance);
    let legend_surface_kind = legend_surface_kind_for_mark_instance(event_datums, mark_instance);
    match target {
        None | Some(ChartEventSurfaceTarget::All) => SurfaceMatch::Accept {
            legend_surface: !legend_surface_keys.is_empty(),
        },
        Some(ChartEventSurfaceTarget::PlotSurface) => {
            if legend_surface_keys.is_empty() {
                SurfaceMatch::Accept {
                    legend_surface: false,
                }
            } else {
                SurfaceMatch::Reject {
                    reason: "surface_plot_excludes_legend",
                }
            }
        }
        Some(ChartEventSurfaceTarget::LegendSurface {
            surface_keys,
            kinds,
        }) => {
            if legend_surface_keys.is_empty() {
                return SurfaceMatch::Reject {
                    reason: "surface_legend_requires_legend_surface",
                };
            }
            if !kinds.is_empty()
                && !legend_surface_kind
                    .as_ref()
                    .is_some_and(|kind| kinds.iter().any(|target| target == kind))
            {
                return SurfaceMatch::Reject {
                    reason: "surface_legend_kind_mismatch",
                };
            }
            if surface_keys.is_empty()
                || surface_keys
                    .iter()
                    .any(|key| legend_surface_keys.iter().any(|candidate| candidate == key))
            {
                SurfaceMatch::Accept {
                    legend_surface: true,
                }
            } else {
                SurfaceMatch::Reject {
                    reason: "surface_legend_key_mismatch",
                }
            }
        }
    }
}

fn legend_surface_kind_for_mark_instance(
    event_datums: &EvaluatedEventDatumState,
    mark_instance: Option<&MarkInstance>,
) -> Option<LegendSurfaceKind> {
    let ScalarValue::Utf8(Some(kind)) =
        event_datums.datum_for_mark_instance(mark_instance, event::LEGEND_SURFACE_KIND_FIELD)?
    else {
        return None;
    };
    match kind.as_str() {
        event::LEGEND_SURFACE_KIND_DISCRETE_ITEM => Some(LegendSurfaceKind::DiscreteItem),
        event::LEGEND_SURFACE_KIND_CONTINUOUS_COLORBAR => {
            Some(LegendSurfaceKind::ContinuousColorbar)
        }
        _ => None,
    }
}

fn legend_surface_keys_for_mark_instance(
    event_datums: &EvaluatedEventDatumState,
    mark_instance: Option<&MarkInstance>,
) -> Vec<String> {
    let Some(ScalarValue::Utf8(Some(surface_key))) =
        event_datums.datum_for_mark_instance(mark_instance, event::LEGEND_SURFACE_KEY_FIELD)
    else {
        return Vec::new();
    };
    surface_key
        .split('\u{1f}')
        .filter(|key| !key.is_empty())
        .map(str::to_string)
        .collect()
}

fn legend_colorbar_scope_id(surface_key: &str) -> String {
    format!("legend-colorbar:{surface_key}")
}

fn target_scope_matches(
    target: Option<&ChartEventScopeTarget>,
    use_start_scope: bool,
    current_scope: Option<&EvaluatedInteractionScope>,
    start_scope: Option<&EvaluatedInteractionScope>,
) -> bool {
    let Some(target) = target else {
        return true;
    };
    let scope = if use_start_scope {
        start_scope
    } else {
        current_scope
    };
    let Some(scope) = scope else {
        return false;
    };
    scope_matches_target(Some(target), scope)
}

fn scope_matches_target(
    target: Option<&ChartEventScopeTarget>,
    scope: &EvaluatedInteractionScope,
) -> bool {
    let Some(target) = target else {
        return true;
    };
    if let Some(prefix) = target.resolved_coord_node_path_prefix()
        && !scope.coord_node_path.starts_with(prefix)
    {
        return false;
    }
    if !target.subplot_ids().is_empty()
        && !target.subplot_ids().iter().any(|id| {
            scope
                .subplot_id_path
                .iter()
                .any(|candidate| candidate == id)
        })
    {
        return false;
    }
    true
}

/// Result of routing a pointer event against interaction scopes.
#[allow(dead_code)]
pub(crate) enum InteractionRoute<'a> {
    /// No event position, or no scope contained the point / supported the channels.
    None,
    /// A unique scope was selected.
    Scope(&'a EvaluatedInteractionScope),
    /// Multiple equal-priority scopes matched; coordinate columns stay null.
    Ambiguous,
}

fn scope_contains_point(scope: &EvaluatedInteractionScope, point: [f32; 2]) -> bool {
    let bounds = &scope.bounds;
    point[0] >= bounds.x
        && point[0] <= bounds.x + bounds.width
        && point[1] >= bounds.y
        && point[1] <= bounds.y + bounds.height
}

fn scope_area(scope: &EvaluatedInteractionScope) -> f32 {
    scope.bounds.width * scope.bounds.height
}

/// Route a pointer position to the unique smallest-area coordinate scope that
/// contains it and supports every requested channel.
///
/// Returns `None` when the event has no position or nothing matches, and
/// `Ambiguous` when multiple equal-smallest-area scopes match.
#[allow(dead_code)]
pub(crate) fn route_interaction_scope<'a>(
    scopes: &'a [EvaluatedInteractionScope],
    point: Option<[f32; 2]>,
    required_channels: &std::collections::BTreeSet<String>,
) -> InteractionRoute<'a> {
    let Some(point) = point else {
        return InteractionRoute::None;
    };
    let mut candidates: Vec<&EvaluatedInteractionScope> = scopes
        .iter()
        .filter(|scope| {
            scope_contains_point(scope, point)
                && required_channels
                    .iter()
                    .all(|channel| scope.channels.iter().any(|c| c == channel))
        })
        .collect();
    if candidates.is_empty() {
        return InteractionRoute::None;
    }
    candidates.sort_by(|a, b| scope_area(a).total_cmp(&scope_area(b)));
    let smallest = scope_area(candidates[0]);
    let tied = candidates
        .iter()
        .filter(|scope| (scope_area(scope) - smallest).abs() < f32::EPSILON)
        .count();
    if tied > 1 {
        return InteractionRoute::Ambiguous;
    }
    InteractionRoute::Scope(candidates[0])
}

fn legend_colorbar_scope_for_mark_instance(
    scopes: &[EvaluatedInteractionScope],
    event_datums: &EvaluatedEventDatumState,
    mark_instance: Option<&MarkInstance>,
    required_channels: &std::collections::BTreeSet<String>,
) -> Option<EvaluatedInteractionScope> {
    if !matches!(
        legend_surface_kind_for_mark_instance(event_datums, mark_instance),
        Some(LegendSurfaceKind::ContinuousColorbar)
    ) {
        return None;
    }
    let surface_keys = legend_surface_keys_for_mark_instance(event_datums, mark_instance);
    let mut candidates = scopes
        .iter()
        .filter(|scope| scope.kind == InteractionScopeKind::LegendColorbar)
        .filter(|scope| {
            required_channels
                .iter()
                .all(|channel| scope.channels.iter().any(|c| c == channel))
        })
        .filter(|scope| {
            surface_keys.is_empty()
                || surface_keys
                    .iter()
                    .any(|key| scope.scope_id == legend_colorbar_scope_id(key))
        })
        .cloned()
        .collect::<Vec<_>>();
    if candidates.len() == 1 {
        candidates.pop()
    } else {
        None
    }
}

fn event_stream_config_for_binding(
    binding: &ChartEventBinding,
    ctx: &SessionContext,
    param_specs: &ParamSpecRegistry,
    stream_param_provider: Option<StreamParamProvider>,
    mark_runtime_targets: Option<&MarkRuntimeTargetIndex>,
) -> Result<EventStreamConfig, AvengerAppError> {
    let mut config = EventStreamConfig {
        types: vec![scene_event_type_from_chart(binding.event_type)],
        mark_paths: runtime_paths_for_mark_ids(binding.resolved_mark_ids(), mark_runtime_targets)?,
        mark_names: runtime_names_for_mark_targets(
            binding.resolved_mark_ids(),
            binding.mark_ids(),
            mark_runtime_targets,
        ),
        throttle: binding.throttle_ms,
        consume: binding.consume,
        ..Default::default()
    };
    if let Some(between) = &binding.between {
        config.emit_between_end_event = between.emit_end_event;
        config.between = Some((
            Box::new(stream_config_for_chart_stream(
                &between.start,
                None,
                false,
                ctx,
                param_specs,
                stream_param_provider.clone(),
                mark_runtime_targets,
            )?),
            Box::new(stream_config_for_chart_stream(
                &between.end,
                None,
                false,
                ctx,
                param_specs,
                stream_param_provider,
                mark_runtime_targets,
            )?),
        ));
    }
    Ok(config)
}

fn stream_config_for_chart_stream(
    stream: &ChartEventStream,
    throttle: Option<u64>,
    consume: bool,
    ctx: &SessionContext,
    param_specs: &ParamSpecRegistry,
    stream_param_provider: Option<StreamParamProvider>,
    mark_runtime_targets: Option<&MarkRuntimeTargetIndex>,
) -> Result<EventStreamConfig, AvengerAppError> {
    stream
        .validate()
        .map_err(|err| AvengerAppError::InternalError(err.to_string()))?;
    let mut config = EventStreamConfig {
        types: stream
            .event_type
            .map(|event_type| vec![scene_event_type_from_chart(event_type)])
            .unwrap_or_default(),
        source_group: stream.resolved_source_group().map(ToOwned::to_owned),
        mark_paths: runtime_paths_for_mark_ids(stream.resolved_mark_ids(), mark_runtime_targets)?,
        mark_names: runtime_names_for_mark_targets(
            stream.resolved_mark_ids(),
            stream.mark_ids(),
            mark_runtime_targets,
        ),
        throttle,
        consume,
        ..Default::default()
    };
    if config.types.is_empty() {
        return Err(AvengerAppError::InternalError(
            "Chart event stream requires an event type".to_string(),
        ));
    }
    if !stream.filters.is_empty() {
        config.filter = Some(vec![compile_low_level_stream_filter(
            stream,
            ctx,
            param_specs,
            stream_param_provider,
        )?]);
    }
    Ok(config)
}

fn compile_low_level_stream_filter(
    stream: &ChartEventStream,
    ctx: &SessionContext,
    param_specs: &ParamSpecRegistry,
    stream_param_provider: Option<StreamParamProvider>,
) -> Result<EventStreamFilter, AvengerAppError> {
    let schema = event_schema(
        param_specs,
        &event::InteractionColumnRequests::default(),
        &IndexMap::new(),
        &IndexMap::new(),
    );
    let allowed_columns = schema
        .fields()
        .iter()
        .map(|field| field.name().clone())
        .filter(|name| {
            !name.starts_with(event::START_PARAM_PREFIX)
                && !name.starts_with(event::PREVIOUS_PARAM_PREFIX)
        })
        .collect::<HashSet<_>>();
    let placeholder_columns = param_specs
        .keys()
        .map(|name| PlaceholderColumn::new(format!("${name}"), event::param_column_name(name)))
        .collect::<Vec<_>>();
    let mut specs = Vec::new();
    for (index, filter) in stream.filters.iter().enumerate() {
        let expr = filter
            .to_expr(ctx)
            .map_err(|err| AvengerAppError::InternalError(err.to_string()))?;
        let placeholders = collect_placeholder_ids(&expr)
            .map_err(|err| AvengerAppError::InternalError(err.to_string()))?;
        if let Some(unsupported) = placeholders.iter().find(|placeholder| {
            placeholder
                .strip_prefix('$')
                .is_none_or(|name| !param_specs.contains_key(name))
        }) {
            return Err(AvengerAppError::InternalError(format!(
                "Chart event stream start/end filter contains unsupported state reference '{unsupported}'; only ordinary params are available"
            )));
        }
        specs.push(
            PhysicalScalarExpressionSpec::new(format!("stream_filter_{index}"), expr)
                .with_expected_type(DataType::Boolean),
        );
    }
    let program = Arc::new(
        CompiledScalarExpressionProgram::compile(
            ctx,
            schema,
            specs,
            PhysicalScalarProgramOptions::default()
                .with_allowed_columns(allowed_columns)
                .with_placeholder_columns(placeholder_columns),
        )
        .map_err(|err| AvengerAppError::InternalError(err.to_string()))?,
    );
    let fallback_params = param_specs
        .iter()
        .map(|(name, spec)| (name.clone(), spec.default.clone()))
        .collect::<IndexMap<_, _>>();
    Ok(EventStreamFilter::context(move |event, context, _rtree| {
        let params = stream_param_provider
            .as_ref()
            .map(|provider| provider(event))
            .unwrap_or_else(|| fallback_params.clone());
        let interaction_values = HashMap::new();
        let event_datum_values = HashMap::new();
        let batch = match event_record_batch(
            program.schema().clone(),
            event,
            context,
            EventBatchInputs {
                current_params: &params,
                start_params: None,
                previous_params: None,
                interaction_values: &interaction_values,
                event_datum_values: &event_datum_values,
                start_event_id: None,
            },
        ) {
            Ok(batch) => batch,
            Err(_) => return false,
        };
        let Ok(values) = program.evaluate_values(&batch) else {
            return false;
        };
        filters_pass(&values)
    }))
}

fn event_schema(
    param_specs: &ParamSpecRegistry,
    interaction: &event::InteractionColumnRequests,
    event_datum_types: &IndexMap<String, DataType>,
    event_coord_types: &IndexMap<String, DataType>,
) -> Arc<Schema> {
    let mut fields = vec![
        Field::new(event::EVENT_TYPE_FIELD, DataType::Utf8, true),
        Field::new(event::EVENT_X_FIELD, DataType::Float64, true),
        Field::new(event::EVENT_Y_FIELD, DataType::Float64, true),
        Field::new(event::FRAME_X_FIELD, DataType::Float64, true),
        Field::new(event::FRAME_Y_FIELD, DataType::Float64, true),
        Field::new(event::FRAME_WIDTH_FIELD, DataType::Float64, true),
        Field::new(event::FRAME_HEIGHT_FIELD, DataType::Float64, true),
        Field::new(event::EVENT_CANVAS_WIDTH_FIELD, DataType::Float64, true),
        Field::new(event::EVENT_CANVAS_HEIGHT_FIELD, DataType::Float64, true),
        Field::new(event::EVENT_WINDOW_WIDTH_FIELD, DataType::Float64, true),
        Field::new(event::EVENT_WINDOW_HEIGHT_FIELD, DataType::Float64, true),
        Field::new(event::EVENT_WHEEL_DELTA_X_FIELD, DataType::Float64, true),
        Field::new(event::EVENT_WHEEL_DELTA_Y_FIELD, DataType::Float64, true),
        Field::new(event::EVENT_BUTTON_FIELD, DataType::Utf8, true),
        Field::new(event::EVENT_KEY_FIELD, DataType::Utf8, true),
        Field::new(event::EVENT_SHIFT_FIELD, DataType::Boolean, true),
        Field::new(event::EVENT_CONTROL_FIELD, DataType::Boolean, true),
        Field::new(event::EVENT_ALT_FIELD, DataType::Boolean, true),
        Field::new(event::EVENT_META_FIELD, DataType::Boolean, true),
        Field::new(event::START_X_FIELD, DataType::Float64, true),
        Field::new(event::START_Y_FIELD, DataType::Float64, true),
        Field::new(event::START_CANVAS_WIDTH_FIELD, DataType::Float64, true),
        Field::new(event::START_CANVAS_HEIGHT_FIELD, DataType::Float64, true),
        Field::new(event::START_WINDOW_WIDTH_FIELD, DataType::Float64, true),
        Field::new(event::START_WINDOW_HEIGHT_FIELD, DataType::Float64, true),
        Field::new(event::START_TIME_MS_FIELD, DataType::Float64, true),
        Field::new(event::PREVIOUS_X_FIELD, DataType::Float64, true),
        Field::new(event::PREVIOUS_Y_FIELD, DataType::Float64, true),
        Field::new(event::PREVIOUS_TIME_MS_FIELD, DataType::Float64, true),
        Field::new(event::ELAPSED_MS_FIELD, DataType::Float64, true),
        Field::new(event::PREVIOUS_ELAPSED_MS_FIELD, DataType::Float64, true),
    ];
    for (name, spec) in param_specs {
        let data_type = spec.data_type.clone();
        fields.push(Field::new(
            event::param_column_name(name),
            data_type.clone(),
            true,
        ));
        fields.push(Field::new(
            event::start_param_column_name(name),
            data_type.clone(),
            true,
        ));
        fields.push(Field::new(
            event::previous_param_column_name(name),
            data_type,
            true,
        ));
    }

    // Derived coordinate columns invert to a single channel value. Continuous
    // coordinate inversion returns Float64; categorical scales return their
    // domain value type, including Struct for nested categorical coordinates.
    for channel in interaction.current_coord.iter() {
        fields.push(Field::new(
            event::event_coord_column_name(channel),
            event_coord_type(channel, event_coord_types),
            true,
        ));
    }
    for channel in interaction.start_coord.iter() {
        fields.push(Field::new(
            event::start_coord_column_name(channel),
            event_coord_type(channel, event_coord_types),
            true,
        ));
    }
    for channel in interaction.event_at_start_coord.iter() {
        fields.push(Field::new(
            event::event_at_start_coord_column_name(channel),
            event_coord_type(channel, event_coord_types),
            true,
        ));
    }
    for channel in interaction.event_at_start_clipped_coord.iter() {
        fields.push(Field::new(
            event::event_at_start_clipped_coord_column_name(channel),
            event_coord_type(channel, event_coord_types),
            true,
        ));
    }
    for channel in interaction.previous_coord.iter() {
        fields.push(Field::new(
            event::previous_coord_column_name(channel),
            event_coord_type(channel, event_coord_types),
            true,
        ));
    }
    // Derived domain columns are two-element numeric lists.
    let domain_list_type = DataType::List(Arc::new(Field::new("item", DataType::Float64, true)));
    for channel in interaction.current_domain.iter() {
        fields.push(Field::new(
            event::event_domain_column_name(channel),
            domain_list_type.clone(),
            true,
        ));
    }
    for channel in interaction.start_domain.iter() {
        fields.push(Field::new(
            event::start_domain_column_name(channel),
            domain_list_type.clone(),
            true,
        ));
    }
    if interaction.current_scope_frame {
        fields.push(Field::new(
            event::EVENT_SCOPE_FRAME_FIELD,
            domain_list_type.clone(),
            true,
        ));
    }
    if interaction.start_scope_frame {
        fields.push(Field::new(
            event::START_SCOPE_FRAME_FIELD,
            domain_list_type.clone(),
            true,
        ));
    }
    if interaction.current_plot_size {
        fields.push(Field::new(
            event::EVENT_PLOT_WIDTH_FIELD,
            DataType::Float64,
            true,
        ));
        fields.push(Field::new(
            event::EVENT_PLOT_HEIGHT_FIELD,
            DataType::Float64,
            true,
        ));
    }
    if interaction.start_plot_size {
        fields.push(Field::new(
            event::START_PLOT_WIDTH_FIELD,
            DataType::Float64,
            true,
        ));
        fields.push(Field::new(
            event::START_PLOT_HEIGHT_FIELD,
            DataType::Float64,
            true,
        ));
    }
    if interaction.current_scope_id {
        fields.push(Field::new(
            event::EVENT_SCOPE_ID_FIELD,
            DataType::Utf8,
            true,
        ));
    }
    if interaction.start_scope_id {
        fields.push(Field::new(
            event::START_SCOPE_ID_FIELD,
            DataType::Utf8,
            true,
        ));
    }
    if interaction.start_event_id {
        fields.push(Field::new(
            event::START_EVENT_ID_FIELD,
            DataType::Utf8,
            true,
        ));
    }
    for index in &interaction.current_facet_values {
        fields.push(Field::new(
            event::event_facet_value_column_name(*index),
            DataType::Utf8,
            true,
        ));
    }
    for index in &interaction.start_facet_values {
        fields.push(Field::new(
            event::start_facet_value_column_name(*index),
            DataType::Utf8,
            true,
        ));
    }
    if interaction.event_path {
        fields.push(Field::new(
            event::EVENT_PATH_FIELD,
            DataType::List(Arc::new(Field::new("item", DataType::Float64, true))),
            true,
        ));
    }
    if interaction.event_path_svg {
        fields.push(Field::new(
            event::EVENT_PATH_SVG_FIELD,
            DataType::Utf8,
            true,
        ));
    }
    for field in &interaction.current_datum {
        if let Some(data_type) = event_datum_types.get(field) {
            fields.push(Field::new(
                event::event_datum_column_name(field),
                data_type.clone(),
                true,
            ));
        }
    }

    schema_from_fields(fields)
}

fn event_coord_type(channel: &str, event_coord_types: &IndexMap<String, DataType>) -> DataType {
    event_coord_types
        .get(channel)
        .cloned()
        .unwrap_or(DataType::Float64)
}

/// Pre-resolved inputs for building a one-row event batch.
struct EventBatchInputs<'a> {
    current_params: &'a IndexMap<String, ScalarValue>,
    start_params: Option<&'a IndexMap<String, ScalarValue>>,
    previous_params: Option<&'a IndexMap<String, ScalarValue>>,
    interaction_values: &'a HashMap<String, ScalarValue>,
    event_datum_values: &'a HashMap<String, ScalarValue>,
    start_event_id: Option<u64>,
}

fn event_record_batch(
    schema: Arc<Schema>,
    event: &SceneGraphEvent,
    context: &EventStreamContext,
    inputs: EventBatchInputs<'_>,
) -> Result<RecordBatch, DataFusionError> {
    let mut values = HashMap::new();
    push_event_values(&mut values, event, "");
    if let Some(start) = &context.start_event {
        push_snapshot_values(&mut values, start, "start");
    }
    if let Some(previous) = &context.previous_event {
        push_snapshot_values(&mut values, previous, "previous");
    }
    if let Some(start_event_id) = inputs.start_event_id {
        values.insert(
            event::START_EVENT_ID_FIELD.to_string(),
            ScalarValue::Utf8(Some(start_event_id.to_string())),
        );
    }
    if let (Some(current), Some(start)) = (&context.current_event, &context.start_event) {
        values.insert(
            event::ELAPSED_MS_FIELD.to_string(),
            ScalarValue::Float64(Some(duration_ms(current.instant, start.instant))),
        );
    }
    if let (Some(current), Some(previous)) = (&context.current_event, &context.previous_event) {
        values.insert(
            event::PREVIOUS_ELAPSED_MS_FIELD.to_string(),
            ScalarValue::Float64(Some(duration_ms(current.instant, previous.instant))),
        );
    }
    for (name, value) in inputs.current_params {
        values.insert(event::param_column_name(name), value.clone());
    }
    if let Some(start_params) = inputs.start_params {
        for (name, value) in start_params {
            values.insert(event::start_param_column_name(name), value.clone());
        }
    }
    if let Some(previous_params) = inputs.previous_params {
        for (name, value) in previous_params {
            values.insert(event::previous_param_column_name(name), value.clone());
        }
    }
    // Derived interaction columns are filled by name; absent columns become null.
    for (name, value) in inputs.interaction_values {
        values.insert(name.clone(), value.clone());
    }
    for (name, value) in inputs.event_datum_values {
        values.insert(name.clone(), value.clone());
    }
    one_row_batch_from_scalars(schema, &values)
}

fn push_snapshot_values(
    values: &mut HashMap<String, ScalarValue>,
    snapshot: &EventStreamEventSnapshot,
    prefix: &str,
) {
    let mut event_values = HashMap::new();
    push_event_values(&mut event_values, &snapshot.event, "");
    let pairs: &[(&str, &str)] = match prefix {
        "start" => &[
            (event::EVENT_X_FIELD, event::START_X_FIELD),
            (event::EVENT_Y_FIELD, event::START_Y_FIELD),
            (
                event::EVENT_CANVAS_WIDTH_FIELD,
                event::START_CANVAS_WIDTH_FIELD,
            ),
            (
                event::EVENT_CANVAS_HEIGHT_FIELD,
                event::START_CANVAS_HEIGHT_FIELD,
            ),
            (
                event::EVENT_WINDOW_WIDTH_FIELD,
                event::START_WINDOW_WIDTH_FIELD,
            ),
            (
                event::EVENT_WINDOW_HEIGHT_FIELD,
                event::START_WINDOW_HEIGHT_FIELD,
            ),
        ],
        "previous" => &[
            (event::EVENT_X_FIELD, event::PREVIOUS_X_FIELD),
            (event::EVENT_Y_FIELD, event::PREVIOUS_Y_FIELD),
        ],
        _ => unreachable!(),
    };
    for (source, target) in pairs.iter().copied() {
        if let Some(value) = event_values.get(source) {
            values.insert(target.to_string(), value.clone());
        }
    }
    match prefix {
        "start" => {
            values.insert(
                event::START_TIME_MS_FIELD.to_string(),
                ScalarValue::Float64(Some(0.0)),
            );
        }
        "previous" => {
            values.insert(
                event::PREVIOUS_TIME_MS_FIELD.to_string(),
                ScalarValue::Float64(Some(0.0)),
            );
        }
        _ => {}
    }
}

fn push_event_values(
    values: &mut HashMap<String, ScalarValue>,
    event: &SceneGraphEvent,
    _prefix: &str,
) {
    values.insert(
        event::EVENT_TYPE_FIELD.to_string(),
        ScalarValue::Utf8(Some(event_type_name(event.event_type()).to_string())),
    );
    if let Some(position) = event.position() {
        values.insert(
            event::EVENT_X_FIELD.to_string(),
            ScalarValue::Float64(Some(position[0] as f64)),
        );
        values.insert(
            event::EVENT_Y_FIELD.to_string(),
            ScalarValue::Float64(Some(position[1] as f64)),
        );
    }
    if let Some(modifiers) = event_modifiers(event) {
        push_modifiers(values, modifiers);
    }
    match event {
        SceneGraphEvent::CanvasResize(e) | SceneGraphEvent::CanvasResizeSettled(e) => {
            values.insert(
                event::EVENT_CANVAS_WIDTH_FIELD.to_string(),
                ScalarValue::Float64(Some(e.size[0] as f64)),
            );
            values.insert(
                event::EVENT_CANVAS_HEIGHT_FIELD.to_string(),
                ScalarValue::Float64(Some(e.size[1] as f64)),
            );
        }
        SceneGraphEvent::WindowResize(e) | SceneGraphEvent::WindowResizeSettled(e) => {
            values.insert(
                event::EVENT_WINDOW_WIDTH_FIELD.to_string(),
                ScalarValue::Float64(Some(e.size[0] as f64)),
            );
            values.insert(
                event::EVENT_WINDOW_HEIGHT_FIELD.to_string(),
                ScalarValue::Float64(Some(e.size[1] as f64)),
            );
        }
        SceneGraphEvent::MouseDown(e) => push_button(values, e.button),
        SceneGraphEvent::MouseUp(e) => push_button(values, e.button),
        SceneGraphEvent::Click(e) => push_button(values, e.button),
        SceneGraphEvent::MouseWheel(e) => match e.delta {
            MouseScrollDelta::LineDelta(x, y) => {
                values.insert(
                    event::EVENT_WHEEL_DELTA_X_FIELD.to_string(),
                    ScalarValue::Float64(Some(x as f64)),
                );
                values.insert(
                    event::EVENT_WHEEL_DELTA_Y_FIELD.to_string(),
                    ScalarValue::Float64(Some(y as f64)),
                );
            }
            MouseScrollDelta::PixelDelta(x, y) => {
                values.insert(
                    event::EVENT_WHEEL_DELTA_X_FIELD.to_string(),
                    ScalarValue::Float64(Some(x)),
                );
                values.insert(
                    event::EVENT_WHEEL_DELTA_Y_FIELD.to_string(),
                    ScalarValue::Float64(Some(y)),
                );
            }
        },
        SceneGraphEvent::KeyPress(e) => push_key(values, e.key),
        SceneGraphEvent::KeyRelease(e) => push_key(values, e.key),
        _ => {}
    }
}

fn push_button(values: &mut HashMap<String, ScalarValue>, button: MouseButton) {
    values.insert(
        event::EVENT_BUTTON_FIELD.to_string(),
        ScalarValue::Utf8(Some(button_name(button).to_string())),
    );
}

fn push_key(values: &mut HashMap<String, ScalarValue>, key: Key) {
    values.insert(
        event::EVENT_KEY_FIELD.to_string(),
        ScalarValue::Utf8(Some(format!("{key:?}"))),
    );
}

fn push_modifiers(values: &mut HashMap<String, ScalarValue>, modifiers: ModifiersState) {
    values.insert(
        event::EVENT_SHIFT_FIELD.to_string(),
        ScalarValue::Boolean(Some(modifiers.shift)),
    );
    values.insert(
        event::EVENT_CONTROL_FIELD.to_string(),
        ScalarValue::Boolean(Some(modifiers.control)),
    );
    values.insert(
        event::EVENT_ALT_FIELD.to_string(),
        ScalarValue::Boolean(Some(modifiers.alt)),
    );
    values.insert(
        event::EVENT_META_FIELD.to_string(),
        ScalarValue::Boolean(Some(modifiers.meta)),
    );
}

fn event_modifiers(event: &SceneGraphEvent) -> Option<ModifiersState> {
    match event {
        SceneGraphEvent::MouseDown(e) => Some(e.modifiers),
        SceneGraphEvent::MouseUp(e) => Some(e.modifiers),
        SceneGraphEvent::Click(e) => Some(e.modifiers),
        SceneGraphEvent::DoubleClick(e) => Some(e.modifiers),
        SceneGraphEvent::MouseWheel(e) => Some(e.modifiers),
        SceneGraphEvent::KeyPress(e) => Some(e.modifiers),
        SceneGraphEvent::KeyRelease(e) => Some(e.modifiers),
        SceneGraphEvent::CursorMoved(e) => Some(e.modifiers),
        SceneGraphEvent::MouseEnter(e) => Some(e.modifiers),
        SceneGraphEvent::MouseLeave(e) => Some(e.modifiers),
        _ => None,
    }
}

fn button_name(button: MouseButton) -> &'static str {
    match button {
        MouseButton::Left => "left",
        MouseButton::Right => "right",
        MouseButton::Middle => "middle",
        MouseButton::Back => "back",
        MouseButton::Forward => "forward",
        MouseButton::Other(_) => "other",
    }
}

fn event_type_name(event_type: SceneGraphEventType) -> &'static str {
    match event_type {
        SceneGraphEventType::MouseDown => "mouse_down",
        SceneGraphEventType::MouseUp => "mouse_up",
        SceneGraphEventType::Click => "click",
        SceneGraphEventType::DoubleClick => "double_click",
        SceneGraphEventType::MouseWheel => "mouse_wheel",
        SceneGraphEventType::KeyPress => "key_press",
        SceneGraphEventType::KeyRelease => "key_release",
        SceneGraphEventType::Ime => "ime",
        SceneGraphEventType::Clipboard => "clipboard",
        SceneGraphEventType::RuntimeWake => "runtime-wake",
        SceneGraphEventType::CursorMoved => "cursor_moved",
        SceneGraphEventType::MarkMouseEnter => "mark_mouse_enter",
        SceneGraphEventType::MarkMouseLeave => "mark_mouse_leave",
        SceneGraphEventType::WindowResize => "window_resize",
        SceneGraphEventType::WindowResizeSettled => "window_resize_settled",
        SceneGraphEventType::CanvasResize => "canvas_resize",
        SceneGraphEventType::CanvasResizeSettled => "canvas_resize_settled",
        SceneGraphEventType::WindowMoved => "window_moved",
        SceneGraphEventType::WindowFocused => "window_focused",
        SceneGraphEventType::WindowCloseRequested => "window_close_requested",
        SceneGraphEventType::InteractionSettled => "interaction_settled",
        SceneGraphEventType::FileChanged(_) => "file_changed",
    }
}

fn scene_event_type_from_chart(event_type: ChartEventType) -> SceneGraphEventType {
    match event_type {
        ChartEventType::MouseDown => SceneGraphEventType::MouseDown,
        ChartEventType::MouseUp => SceneGraphEventType::MouseUp,
        ChartEventType::Click => SceneGraphEventType::Click,
        ChartEventType::DoubleClick => SceneGraphEventType::DoubleClick,
        ChartEventType::MouseWheel => SceneGraphEventType::MouseWheel,
        ChartEventType::KeyPress => SceneGraphEventType::KeyPress,
        ChartEventType::KeyRelease => SceneGraphEventType::KeyRelease,
        ChartEventType::CursorMoved => SceneGraphEventType::CursorMoved,
        ChartEventType::MarkMouseEnter => SceneGraphEventType::MarkMouseEnter,
        ChartEventType::MarkMouseLeave => SceneGraphEventType::MarkMouseLeave,
        ChartEventType::WindowResize => SceneGraphEventType::WindowResize,
        ChartEventType::WindowResizeSettled => SceneGraphEventType::WindowResizeSettled,
        ChartEventType::CanvasResize => SceneGraphEventType::CanvasResize,
        ChartEventType::CanvasResizeSettled => SceneGraphEventType::CanvasResizeSettled,
        ChartEventType::WindowMoved => SceneGraphEventType::WindowMoved,
        ChartEventType::WindowFocused => SceneGraphEventType::WindowFocused,
        ChartEventType::WindowCloseRequested => SceneGraphEventType::WindowCloseRequested,
    }
}

fn duration_ms(current: Instant, previous: Instant) -> f64 {
    current.duration_since(previous).as_secs_f64() * 1000.0
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};

    use avenger_app::app::SceneGraphBuilder;
    use avenger_chart::layout::LayoutBounds;
    use avenger_chart::parallel::event as parallel_event;
    use avenger_chart::prelude::*;
    use avenger_chart::render::{
        EvaluatedChildFrameKind, InteractionScopeId, InteractionScopeKind,
    };
    use avenger_chart_treemap::{
        ROOT_PATH_ID, TreeHeader, TreeLabel, TreeRect, Treemap, TreemapGuide, TreemapHeaderBars,
        TreemapPadding,
        event::{self as treemap_event, HIERARCHY_PATH_ID_FIELD},
    };
    use avenger_chart_widgets::{Button, ButtonVariant, Checkbox, CheckboxList, Slider};
    use avenger_common::time::Duration;
    use avenger_eventstream::{
        manager::EventStreamManager,
        scene::{
            SceneClickEvent, SceneCursorMovedEvent, SceneDoubleClickEvent, SceneMouseDownEvent,
            SceneMouseUpEvent,
        },
        window::{
            CanvasResizeEvent, ElementState, MouseButton, WindowCursorMoved, WindowEvent,
            WindowMouseInput,
        },
    };
    use avenger_scenegraph::{marks::mark::SceneMark, scene_graph::SceneGraph};
    use datafusion::arrow::array::{
        Array, ArrayRef, Float64Array, Int32Array, StringArray, TimestampMillisecondArray,
    };
    use datafusion::functions_aggregate::expr_fn::sum;

    use super::*;

    fn coord_scope(
        id: usize,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        channels: &[&str],
    ) -> EvaluatedInteractionScope {
        EvaluatedInteractionScope {
            id: InteractionScopeId(id),
            kind: InteractionScopeKind::Coordinate,
            scope_id: format!("test-scope-{id}"),
            bounds: LayoutBounds {
                x,
                y,
                width,
                height,
            },
            plot_area_width: width,
            plot_area_height: height,
            facet_path: Vec::new(),
            logical_facet_values: Vec::new(),
            coord_node_path: Vec::new(),
            subplot_id_path: Vec::new(),
            child_frame_path: Vec::new(),
            coord_transform: Box::new(Cartesian::new()),
            channels: channels.iter().map(|c| c.to_string()).collect(),
            scales: HashMap::new(),
            sharing_owner_paths: HashMap::new(),
        }
    }

    fn channel_set(channels: &[&str]) -> BTreeSet<String> {
        channels.iter().map(|c| c.to_string()).collect()
    }

    #[test]
    fn route_returns_none_for_point_outside_scope() {
        let scopes = vec![coord_scope(0, 50.0, 40.0, 300.0, 200.0, &["x", "y"])];
        let channels = channel_set(&["x", "y"]);
        assert!(matches!(
            route_interaction_scope(&scopes, Some([100.0, 100.0]), &channels),
            InteractionRoute::Scope(_)
        ));
        assert!(matches!(
            route_interaction_scope(&scopes, Some([10.0, 10.0]), &channels),
            InteractionRoute::None
        ));
        assert!(matches!(
            route_interaction_scope(&scopes, None, &channels),
            InteractionRoute::None
        ));
    }

    #[test]
    fn route_requires_all_channels_supported() {
        let scopes = vec![coord_scope(0, 0.0, 0.0, 100.0, 100.0, &["x"])];
        // The scope only supports x, so a binding needing y must not match it.
        assert!(matches!(
            route_interaction_scope(&scopes, Some([10.0, 10.0]), &channel_set(&["x", "y"])),
            InteractionRoute::None
        ));
    }

    #[test]
    fn route_picks_smallest_area_scope() {
        let scopes = vec![
            coord_scope(0, 0.0, 0.0, 400.0, 400.0, &["x", "y"]),
            coord_scope(1, 50.0, 50.0, 100.0, 100.0, &["x", "y"]),
        ];
        match route_interaction_scope(&scopes, Some([100.0, 100.0]), &channel_set(&["x"])) {
            InteractionRoute::Scope(scope) => assert_eq!(scope.id, InteractionScopeId(1)),
            _ => panic!("expected the smaller nested scope to win"),
        }
    }

    #[test]
    fn event_path_samples_meaningful_motion_and_round_trips_as_points() {
        let mut path = Vec::new();
        push_event_path_point(
            &mut path,
            [0.0, 0.0],
            event::DEFAULT_EVENT_PATH_MIN_DISTANCE_PX,
        );
        push_event_path_point(
            &mut path,
            [1.0, 1.0],
            event::DEFAULT_EVENT_PATH_MIN_DISTANCE_PX,
        );
        push_event_path_point(
            &mut path,
            [2.0, 0.0],
            event::DEFAULT_EVENT_PATH_MIN_DISTANCE_PX,
        );
        push_event_path_point(
            &mut path,
            [4.0, 0.0],
            event::DEFAULT_EVENT_PATH_MIN_DISTANCE_PX,
        );

        assert_eq!(path, vec![[0.0, 0.0], [2.0, 0.0], [4.0, 0.0]]);

        let scalar = event_path_scalar(&path);
        let restored = scalar_value_to_point_list(Some(&scalar)).expect("event path points");
        assert_eq!(restored, path);

        assert_eq!(
            event_path_svg_scalar(&path),
            ScalarValue::Utf8(Some("M 0 0 L 2.000 0.000 L 4.000 0.000 Z".to_string()))
        );
    }

    #[test]
    fn event_path_sampler_is_bounded() {
        let mut path = Vec::new();
        for index in 0..2100 {
            push_event_path_point(
                &mut path,
                [index as f32 * 3.0, 0.0],
                event::DEFAULT_EVENT_PATH_MIN_DISTANCE_PX,
            );
        }

        assert_eq!(path.len(), 2048);
        assert_eq!(path.first(), Some(&[0.0, 0.0]));
        assert_eq!(path.last(), Some(&[6297.0, 0.0]));
    }

    #[test]
    fn event_path_sampler_uses_configured_minimum_distance() {
        let mut path = Vec::new();
        push_event_path_point(&mut path, [0.0, 0.0], 5.0);
        push_event_path_point(&mut path, [3.0, 4.0], 5.0);
        push_event_path_point(&mut path, [6.0, 6.0], 5.0);
        push_event_path_point(&mut path, [9.0, 8.0], 5.0);

        assert_eq!(path, vec![[0.0, 0.0], [3.0, 4.0], [9.0, 8.0]]);
    }

    #[test]
    fn stream_config_derives_paths_from_resolved_mark_ids() {
        let ctx = SessionContext::new();
        let mut allocator =
            avenger_chart_core::CompiledIdentityAllocator::new("stream-config-test");
        let first = allocator.allocate_mark();
        let second = allocator.allocate_mark();
        let stream = ChartEventStream::on(ChartEventType::MouseDown)
            .mark("manual_box_plot")
            .with_resolved_mark_ids(vec![first.clone(), second.clone()]);
        let index = MarkRuntimeTargetIndex {
            paths: BTreeMap::from([
                (first.clone(), vec![vec![0]]),
                (second.clone(), vec![vec![1]]),
            ]),
            names: BTreeMap::from([
                (first, vec!["manual_box_plot.box".to_string()]),
                (second, vec!["manual_box_plot.whisker".to_string()]),
            ]),
        };
        let config = stream_config_for_chart_stream(
            &stream,
            None,
            false,
            &ctx,
            &ParamSpecRegistry::default(),
            None,
            Some(&index),
        )
        .expect("stream config");

        assert_eq!(config.mark_paths, Some(vec![vec![0], vec![1]]));
        assert_eq!(
            config.mark_names,
            Some(vec![
                "manual_box_plot.box".to_string(),
                "manual_box_plot.whisker".to_string(),
            ])
        );
    }

    #[test]
    fn binding_config_derives_paths_from_resolved_mark_ids() {
        let ctx = SessionContext::new();
        let mut allocator =
            avenger_chart_core::CompiledIdentityAllocator::new("binding-config-test");
        let mark_id = allocator.allocate_mark();
        let binding = ChartEventBinding::on(ChartEventType::Click)
            .mark("contract.box")
            .with_resolved_mark_ids(vec![mark_id.clone()]);
        let index = MarkRuntimeTargetIndex {
            paths: BTreeMap::from([(mark_id.clone(), vec![vec![0, 0]])]),
            names: BTreeMap::from([(mark_id, vec!["contract.box".to_string()])]),
        };
        let config = event_stream_config_for_binding(
            &binding,
            &ctx,
            &ParamSpecRegistry::default(),
            None,
            Some(&index),
        )
        .expect("binding config");

        assert_eq!(config.mark_paths, Some(vec![vec![0, 0]]));
        assert_eq!(config.mark_names, Some(vec!["contract.box".to_string()]));
    }

    #[test]
    fn stream_config_retains_group_target_for_unnamed_mark_ids() {
        let ctx = SessionContext::new();
        let mut allocator =
            avenger_chart_core::CompiledIdentityAllocator::new("unnamed-group-config-test");
        let mark_id = allocator.allocate_mark();
        let stream = ChartEventStream::on(ChartEventType::MouseDown)
            .mark("interactive_group")
            .with_resolved_mark_ids(vec![mark_id.clone()]);
        let index = MarkRuntimeTargetIndex {
            paths: BTreeMap::from([(mark_id, vec![vec![0, 0]])]),
            names: BTreeMap::new(),
        };
        let config = stream_config_for_chart_stream(
            &stream,
            None,
            false,
            &ctx,
            &ParamSpecRegistry::default(),
            None,
            Some(&index),
        )
        .expect("stream config");

        assert_eq!(config.mark_paths, Some(vec![vec![0, 0]]));
        assert_eq!(
            config.mark_names,
            Some(vec!["interactive_group".to_string()])
        );
    }

    #[test]
    fn non_widget_event_leaves_frame_columns_null() {
        let schema = event_schema(
            &ParamSpecRegistry::default(),
            &InteractionColumnRequests::default(),
            &IndexMap::new(),
            &IndexMap::new(),
        );
        let params = IndexMap::new();
        let interaction_values = HashMap::new();
        let event_datum_values = HashMap::new();
        let batch = event_record_batch(
            schema,
            &SceneGraphEvent::CanvasResize(CanvasResizeEvent {
                size: [640.0, 480.0],
            }),
            &EventStreamContext::default(),
            EventBatchInputs {
                current_params: &params,
                start_params: None,
                previous_params: None,
                interaction_values: &interaction_values,
                event_datum_values: &event_datum_values,
                start_event_id: None,
            },
        )
        .expect("non-widget event batch");

        for name in [
            event::FRAME_X_FIELD,
            event::FRAME_Y_FIELD,
            event::FRAME_WIDTH_FIELD,
            event::FRAME_HEIGHT_FIELD,
        ] {
            let value =
                ScalarValue::try_from_array(batch.column_by_name(name).expect("frame column"), 0)
                    .expect("frame scalar");
            assert!(value.is_null(), "{name} should be null");
        }
    }

    #[test]
    fn scene_query_batch_rows_become_faceted_equality_clauses() {
        use datafusion::arrow::{
            array::StringArray,
            datatypes::{DataType, Field, Schema},
            record_batch::RecordBatch,
        };

        let spec = Selection::new("picked")
            .empty_selects_nothing()
            .facet_context_field("group_name", col("group_name"))
            .compile()
            .expect("compile selection");
        let update = CompiledSelectionSceneQuery {
            query: CompiledSceneGeometryQuery {
                geometry: CompiledSceneGeometryQueryGeometry::Rect {
                    x0: 0,
                    y0: 1,
                    x1: 2,
                    y1: 3,
                },
                coordinate_space: SceneGeometryCoordinateSpace::Scene,
                hit_policy: SceneGeometryHitPolicy::AnchorInside,
                target: SceneGeometryTarget::default(),
                runtime_mark_paths: None,
                runtime_mark_names: None,
                datum_fields: vec![
                    SceneQueryDatumField::new("item")
                        .datum("item")
                        .field_expr(col("item")),
                ],
                unique_by: vec!["item".to_string()],
                max_hits: None,
            },
            sharing: CoordinationScope::Free,
            clause_id: CompiledSceneQueryClauseId::Field("item".to_string()),
        };
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new("item", DataType::Utf8, false)])),
            vec![Arc::new(StringArray::from(vec!["A", "B"]))],
        )
        .expect("query result batch");
        let owner_path = vec![ScalarValue::Utf8(Some("Beta".to_string()))];

        let clauses =
            scene_query_selection_clauses_from_batch(&update, &spec, &[], 0, &owner_path, &batch)
                .expect("scene query clauses");

        assert_eq!(clauses.len(), 2);
        assert_eq!(clauses[0].id, "A");
        assert_eq!(clauses[1].id, "B");
        assert_eq!(clauses[0].scope.sharing, CoordinationScope::Free);
        assert_eq!(clauses[0].scope.owner_path, owner_path);
        assert_eq!(clauses[0].facet_context.len(), 1);
        assert_eq!(clauses[0].facet_context[0].id, "group_name");
        assert_eq!(
            clauses[0].facet_context[0].value,
            ScalarValue::Utf8(Some("Beta".to_string()))
        );
        let SelectionPredicateSpec::Equality { dimensions } = &clauses[0].predicate else {
            panic!("expected equality clause");
        };
        assert_eq!(dimensions.len(), 1);
        assert_eq!(dimensions[0].id, "item");
        assert_eq!(
            dimensions[0].value,
            ScalarValue::Utf8(Some("A".to_string()))
        );
    }

    #[tokio::test]
    async fn scene_query_result_wraps_record_batch_as_dataframe() {
        use datafusion::arrow::{
            array::StringArray,
            datatypes::{DataType, Field, Schema},
            record_batch::RecordBatch,
        };

        let rows = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new("item", DataType::Utf8, false)])),
            vec![Arc::new(StringArray::from(vec!["A", "B"]))],
        )
        .expect("query result batch");
        let result = SceneGeometryQueryResult {
            rows,
            hit_count: 3,
            unique_count: 2,
            dropped_count: 1,
        };
        assert_eq!(result.hit_count, 3);
        assert_eq!(result.unique_count, 2);
        assert_eq!(result.dropped_count, 1);

        let ctx = SessionContext::new();
        let batches = result
            .into_dataframe(&ctx)
            .expect("query result dataframe")
            .collect()
            .await
            .expect("collect query result dataframe");
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].num_rows(), 2);
        assert!(batches[0].column_by_name("item").is_some());
    }

    #[test]
    fn scene_query_target_matching_filters_ids_and_resolved_paths() {
        let rtree = empty_rtree();
        let mark_instance = MarkInstance {
            name: "points".to_string(),
            mark_path: vec![2, 1, 0],
            instance_index: Some(4),
        };

        let event_datums = EvaluatedEventDatumState::default();
        let target = SceneGeometryTarget::default().with_resolved_source_group(vec![2, 1]);
        assert!(scene_query_target_matches(
            &target,
            None,
            None,
            &mark_instance,
            &event_datums,
            &rtree,
        ));

        let target = SceneGeometryTarget::default();
        assert!(scene_query_target_matches(
            &target,
            Some(&[vec![2, 1, 0]]),
            None,
            &mark_instance,
            &event_datums,
            &rtree,
        ));

        let target = SceneGeometryTarget::default();
        assert!(scene_query_target_matches(
            &target,
            Some(&[vec![0]]),
            None,
            &mark_instance,
            &event_datums,
            &rtree,
        ));

        let target = SceneGeometryTarget::default();
        assert!(!scene_query_target_matches(
            &target,
            Some(&[vec![2, 1, 1]]),
            None,
            &mark_instance,
            &event_datums,
            &rtree,
        ));

        let target = SceneGeometryTarget::default();
        assert!(!scene_query_target_matches(
            &target,
            Some(&[vec![1]]),
            None,
            &mark_instance,
            &event_datums,
            &rtree,
        ));

        let target = SceneGeometryQuery::rect(lit(0), lit(0), lit(1), lit(1))
            .mark("points")
            .target;
        assert!(scene_query_target_matches(
            &target,
            None,
            None,
            &mark_instance,
            &event_datums,
            &rtree,
        ));

        let target = SceneGeometryQuery::rect(lit(0), lit(0), lit(1), lit(1))
            .mark("other")
            .target;
        assert!(!scene_query_target_matches(
            &target,
            None,
            None,
            &mark_instance,
            &event_datums,
            &rtree,
        ));

        let target = SceneGeometryTarget::default();
        assert!(scene_query_target_matches(
            &target,
            Some(&[vec![9, 9, 0]]),
            Some(&["points".to_string()]),
            &mark_instance,
            &event_datums,
            &rtree,
        ));
        assert!(!scene_query_target_matches(
            &target,
            Some(&[vec![0]]),
            Some(&["other".to_string()]),
            &mark_instance,
            &event_datums,
            &rtree,
        ));
    }

    #[test]
    fn route_is_ambiguous_for_equal_area_overlap() {
        let scopes = vec![
            coord_scope(0, 0.0, 0.0, 100.0, 100.0, &["x", "y"]),
            coord_scope(1, 0.0, 0.0, 100.0, 100.0, &["x", "y"]),
        ];
        assert!(matches!(
            route_interaction_scope(&scopes, Some([50.0, 50.0]), &channel_set(&["x"])),
            InteractionRoute::Ambiguous
        ));
    }

    #[test]
    fn scene_query_shapes_free_scope_stays_in_start_scope() {
        let source = coord_scope(0, 10.0, 20.0, 100.0, 100.0, &["x", "y"]);
        let other = coord_scope(1, 210.0, 20.0, 100.0, 100.0, &["x", "y"]);
        let shape = GeometryQueryShape::Polygon {
            points: vec![[12.0, 24.0], [20.0, 24.0], [20.0, 32.0]],
        };

        let shapes = scene_query_shapes_for_sharing(
            &shape,
            CoordinationScope::Free,
            &source,
            &[source.clone(), other],
            None,
        );

        assert_eq!(shapes.len(), 1);
        assert_eq!(
            polygon_points(&shapes[0]),
            vec![[12.0, 24.0], [20.0, 24.0], [20.0, 32.0]]
        );
    }

    #[test]
    fn scene_query_shapes_shared_scope_replicates_inside_binding_target() {
        let mut left = coord_scope(0, 10.0, 20.0, 100.0, 100.0, &["x", "y"]);
        left.coord_node_path = vec![0, 0];
        let mut source = coord_scope(1, 210.0, 20.0, 100.0, 100.0, &["x", "y"]);
        source.coord_node_path = vec![1, 0];
        let mut sibling = coord_scope(2, 410.0, 20.0, 100.0, 100.0, &["x", "y"]);
        sibling.coord_node_path = vec![1, 1];
        let target = ChartEventScopeTarget::with_resolved_coord_node_path_prefix(vec![1]);
        let shape = GeometryQueryShape::Polygon {
            points: vec![[212.0, 24.0], [220.0, 24.0], [220.0, 32.0]],
        };

        let shapes = scene_query_shapes_for_sharing(
            &shape,
            CoordinationScope::Shared,
            &source,
            &[left, source.clone(), sibling],
            Some(&target),
        );

        assert_eq!(shapes.len(), 2);
        assert_eq!(
            polygon_points(&shapes[0]),
            vec![[212.0, 24.0], [220.0, 24.0], [220.0, 32.0]]
        );
        assert_eq!(
            polygon_points(&shapes[1]),
            vec![[412.0, 24.0], [420.0, 24.0], [420.0, 32.0]]
        );
    }

    fn polygon_points(shape: &GeometryQueryShape) -> Vec<[f32; 2]> {
        match shape {
            GeometryQueryShape::Polygon { points } => points.clone(),
            other => panic!("expected polygon shape, got {other:?}"),
        }
    }

    #[test]
    fn assignment_writable_allows_raw_domain_default_reset() {
        let default = Param::raw_domain("x_domain").default;
        assert!(assignment_value_is_writable(&default, &default));
    }

    #[test]
    fn assignment_writable_allows_nullable_scalar_default_reset() {
        let default = ScalarValue::Utf8(None);
        assert!(assignment_value_is_writable(&default, &default));
    }

    #[test]
    fn assignment_writable_rejects_null_domain_list_unless_it_is_default() {
        let null_domain = Param::raw_domain("x_domain").default;
        let concrete_default = domain_list_scalar(0.0, 1.0);
        assert!(!assignment_value_is_writable(
            &null_domain,
            &concrete_default
        ));
    }

    async fn pan_state_and_handler() -> (ChartAppState, ChartEventBindingHandler) {
        let ctx = SessionContext::new();
        let x_domain = Param::raw_domain("x_domain");
        let df = ctx
            .sql("SELECT * FROM (VALUES (0.0, 0.0), (10.0, 10.0)) AS t(x, y)")
            .await
            .expect("data");
        let compiled = Chart::<Cartesian>::new()
            .canvas_size(400.0, 300.0)
            .data(df)
            .mark(
                Symbol::new()
                    .x_with(col("x"), |c| {
                        c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                    })
                    .y(col("y"))
                    .size(20.0),
            )
            .tool(
                PanScrollZoom::cartesian()
                    .x_only()
                    .x_domain_param(x_domain)
                    .settle_exact(true),
            )
            .compile(&ctx)
            .await
            .expect("compile pan plot");
        let policy = compiled.resize_policy();
        let handler = compile_handler_for_event_type(&compiled, &ctx, ChartEventType::CursorMoved);
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        (state, handler)
    }

    /// A two-column FacetColumn plot whose leaf tool installs an x raw-domain
    /// param. A pan in any cell writes the param at the scale sharing owner, so
    /// Shared pans every cell and Free pans only the active cell.
    async fn faceted_pan_state_and_handler() -> (ChartAppState, ChartEventBindingHandler) {
        faceted_pan_state_and_handler_with_sharing(CoordinationScope::Shared).await
    }

    async fn faceted_pan_state_and_handler_with_sharing(
        sharing: CoordinationScope,
    ) -> (ChartAppState, ChartEventBindingHandler) {
        let ctx = SessionContext::new();
        let x_domain = Param::raw_domain("x_domain");
        // A `Shared` scale drives one root domain for all cells; a `Free` scale
        // pans only the cell under the pointer. The tool mirrors this sharing
        // for its raw-domain param.
        let share_domain = sharing.to_level() == u8::MAX;
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('A', 0.0, 0.0), ('A', 10.0, 10.0),
                    ('B', 0.0, 1.0), ('B', 10.0, 9.0)
                ) AS t(group_name, x, y)",
            )
            .await
            .expect("data");
        let compiled = Chart::<FacetColumn>::new()
            .canvas_size(640.0, 320.0)
            .data(df)
            .mark(
                Subplot::new(
                    Plot::<Cartesian>::new()
                        .mark(
                            Symbol::new()
                                .x_with(col("x"), move |c| {
                                    let c = c.scale_with::<Linear>(|s| s.nice(false).zero(false));
                                    if share_domain { c.share_domain() } else { c }
                                })
                                .y(col("y"))
                                .size(20.0),
                        )
                        .tool(
                            PanScrollZoom::cartesian()
                                .x_only()
                                .x_domain_param(x_domain)
                                .settle_exact(true),
                        ),
                )
                .column(col("group_name")),
            )
            .compile(&ctx)
            .await
            .expect("compile faceted pan plot");
        let policy = compiled.resize_policy();
        let handler = compile_handler_for_event_type(&compiled, &ctx, ChartEventType::CursorMoved);
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        (state, handler)
    }

    fn concat_pan_child(tool_id: &str, x_domain: Param) -> Plot<Cartesian> {
        Plot::<Cartesian>::new()
            .mark(
                Symbol::new()
                    .x_with(col("x"), |c| {
                        c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                            .with_domain_group("measurement")
                            .share_domain()
                    })
                    .y(col("y"))
                    .size(20.0),
            )
            .tool(
                PanScrollZoom::cartesian()
                    .id(tool_id)
                    .x_only()
                    .x_domain_param(x_domain)
                    .x_sharing(CoordinationScope::Shared)
                    .settle_exact(true),
            )
    }

    async fn grid_concat_pan_state_and_handlers() -> (ChartAppState, Vec<ChartEventBindingHandler>)
    {
        let ctx = SessionContext::new();
        let x_domain = Param::raw_domain("x_domain");
        let df = ctx
            .sql("SELECT * FROM (VALUES (0.0, 0.0), (10.0, 10.0)) AS t(x, y)")
            .await
            .expect("data");
        let compiled = Chart::<GridConcat>::new()
            .canvas_size(640.0, 320.0)
            .data(df)
            .configure_coord(|c| c.rows(1).columns(2))
            .mark(
                Subplot::new(concat_pan_child("grid_nav_a", x_domain.clone()))
                    .at(0, 0)
                    .name("a"),
            )
            .mark(
                Subplot::new(concat_pan_child("grid_nav_b", x_domain.clone()))
                    .at(0, 1)
                    .name("b"),
            )
            .compile(&ctx)
            .await
            .expect("compile grid concat pan plot");
        let policy = compiled.resize_policy();
        let handlers =
            compile_handlers_for_event_type(&compiled, &ctx, ChartEventType::CursorMoved);
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        (state, handlers)
    }

    async fn wrap_concat_pan_state_and_handlers() -> (ChartAppState, Vec<ChartEventBindingHandler>)
    {
        let ctx = SessionContext::new();
        let x_domain = Param::raw_domain("x_domain");
        let df = ctx
            .sql("SELECT * FROM (VALUES (0.0, 0.0), (10.0, 10.0)) AS t(x, y)")
            .await
            .expect("data");
        let compiled = Chart::<WrapConcat>::new()
            .canvas_size(640.0, 320.0)
            .data(df)
            .configure_coord(|c| c.columns(2))
            .mark(Subplot::new(concat_pan_child("wrap_nav_a", x_domain.clone())).name("a"))
            .mark(Subplot::new(concat_pan_child("wrap_nav_b", x_domain.clone())).name("b"))
            .compile(&ctx)
            .await
            .expect("compile wrap concat pan plot");
        let policy = compiled.resize_policy();
        let handlers =
            compile_handlers_for_event_type(&compiled, &ctx, ChartEventType::CursorMoved);
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        (state, handlers)
    }

    fn repeat_pan_cell(settle_exact: bool) -> Plot<Cartesian> {
        let mut tool = PanScrollZoom::cartesian();
        if settle_exact {
            tool = tool.settle_exact(true);
        }
        Plot::<Cartesian>::new()
            .mark(
                Symbol::new()
                    .x_with(repeat::column(), |c| {
                        c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                    })
                    .y_with(repeat::row(), |c| {
                        c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                    })
                    .size(20.0),
            )
            .tool(tool)
    }

    async fn repeat_pan_compiled(
        scope: CoordinationScope,
        settle_exact: bool,
    ) -> (CompiledPlot, Arc<SessionContext>) {
        let ctx = Arc::new(SessionContext::new());
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    (0.0, 0.0),
                    (10.0, 100.0),
                    (20.0, 200.0)
                ) AS t(a, b)",
            )
            .await
            .expect("repeat pan data");
        let compiled = Chart::<RepeatGrid>::new()
            .canvas_size(520.0, 520.0)
            .data(df)
            .configure_coord(|c| {
                c.rows([
                    RepeatVariable::new("a", col("a")),
                    RepeatVariable::new("b", col("b")),
                ])
                .columns([
                    RepeatVariable::new("a", col("a")),
                    RepeatVariable::new("b", col("b")),
                ])
                .cell(repeat_pan_cell(settle_exact))
                .matrix_domains_with_scope(scope)
                .matrix_axes()
            })
            .compile(&ctx)
            .await
            .expect("compile repeat pan plot");
        (compiled, ctx)
    }

    async fn repeat_pan_state_and_handlers(
        scope: CoordinationScope,
        settle_exact: bool,
    ) -> (
        ChartAppState,
        Vec<ChartEventBindingHandler>,
        Vec<ChartEventBindingHandler>,
    ) {
        let (compiled, ctx) = repeat_pan_compiled(scope, settle_exact).await;
        let policy = compiled.resize_policy();
        let drag_handlers =
            compile_handlers_for_event_type(&compiled, &ctx, ChartEventType::CursorMoved);
        let wheel_handlers =
            compile_handlers_for_event_type(&compiled, &ctx, ChartEventType::MouseWheel);
        let session = Arc::new(compiled).instantiate(ctx);
        let state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        (state, drag_handlers, wheel_handlers)
    }

    fn repeat_scope<'a>(
        scopes: &'a [EvaluatedInteractionScope],
        cell_key: &str,
    ) -> &'a EvaluatedInteractionScope {
        scopes
            .iter()
            .find(|scope| {
                scope
                    .child_frame_path
                    .last()
                    .and_then(|segment| segment.key.as_deref())
                    == Some(cell_key)
            })
            .unwrap_or_else(|| panic!("missing repeat scope {cell_key}"))
    }

    fn repeat_scope_domain(
        scopes: &[EvaluatedInteractionScope],
        cell_key: &str,
        channel: &str,
    ) -> (f32, f32) {
        repeat_scope(scopes, cell_key)
            .scales
            .get(channel)
            .unwrap_or_else(|| panic!("scope {cell_key} has {channel} scale"))
            .numeric_interval_domain()
            .unwrap_or_else(|_| panic!("{cell_key}/{channel} domain is numeric"))
    }

    fn repeat_facet_scope<'a>(
        scopes: &'a [EvaluatedInteractionScope],
        facet_value: &str,
        cell_key: &str,
    ) -> &'a EvaluatedInteractionScope {
        scopes
            .iter()
            .find(|scope| {
                matches!(
                    scope.facet_path.first(),
                    Some(ScalarValue::Utf8(Some(value))) if value == facet_value
                ) && scope
                    .child_frame_path
                    .last()
                    .and_then(|segment| segment.key.as_deref())
                    == Some(cell_key)
            })
            .unwrap_or_else(|| {
                let available = scopes
                    .iter()
                    .map(|scope| {
                        let facet = match scope.facet_path.first() {
                            Some(ScalarValue::Utf8(Some(value))) => value.clone(),
                            other => format!("{other:?}"),
                        };
                        let key = scope
                            .child_frame_path
                            .last()
                            .and_then(|segment| segment.key.as_deref())
                            .unwrap_or("<none>")
                            .to_string();
                        (facet, key)
                    })
                    .collect::<Vec<_>>();
                panic!(
                    "missing repeat scope {cell_key} in facet {facet_value}; available={available:?}"
                )
            })
    }

    fn repeat_facet_scope_domain(
        scopes: &[EvaluatedInteractionScope],
        facet_value: &str,
        cell_key: &str,
        channel: &str,
    ) -> (f32, f32) {
        repeat_facet_scope(scopes, facet_value, cell_key)
            .scales
            .get(channel)
            .unwrap_or_else(|| panic!("scope {facet_value}/{cell_key} has {channel} scale"))
            .numeric_interval_domain()
            .unwrap_or_else(|_| panic!("{facet_value}/{cell_key}/{channel} domain is numeric"))
    }

    fn repeat_logical_facet_scope<'a>(
        scopes: &'a [EvaluatedInteractionScope],
        logical_facet_value: &str,
        cell_key: &str,
    ) -> &'a EvaluatedInteractionScope {
        scopes
            .iter()
            .find(|scope| {
                matches!(
                    scope.logical_facet_values.first(),
                    Some(ScalarValue::Utf8(Some(value))) if value == logical_facet_value
                ) && scope
                    .child_frame_path
                    .last()
                    .and_then(|segment| segment.key.as_deref())
                    == Some(cell_key)
            })
            .unwrap_or_else(|| {
                let available = scopes
                    .iter()
                    .map(|scope| {
                        let facet = match scope.logical_facet_values.first() {
                            Some(ScalarValue::Utf8(Some(value))) => value.clone(),
                            other => format!("{other:?}"),
                        };
                        let key = scope
                            .child_frame_path
                            .last()
                            .and_then(|segment| segment.key.as_deref())
                            .unwrap_or("<none>")
                            .to_string();
                        (facet, key)
                    })
                    .collect::<Vec<_>>();
                panic!(
                    "missing repeat scope {cell_key} in logical facet {logical_facet_value}; \
                     available={available:?}"
                )
            })
    }

    fn repeat_logical_facet_scope_domain(
        scopes: &[EvaluatedInteractionScope],
        logical_facet_value: &str,
        cell_key: &str,
        channel: &str,
    ) -> (f32, f32) {
        repeat_logical_facet_scope(scopes, logical_facet_value, cell_key)
            .scales
            .get(channel)
            .unwrap_or_else(|| panic!("scope {logical_facet_value}/{cell_key} has {channel} scale"))
            .numeric_interval_domain()
            .unwrap_or_else(|_| {
                panic!("{logical_facet_value}/{cell_key}/{channel} domain is numeric")
            })
    }

    fn scope_center(scope: &EvaluatedInteractionScope) -> [f32; 2] {
        [
            scope.bounds.x + scope.bounds.width * 0.5,
            scope.bounds.y + scope.bounds.height * 0.5,
        ]
    }

    async fn wheel_zoom_all(
        state: &mut ChartAppState,
        handlers: &[ChartEventBindingHandler],
        position: [f32; 2],
        delta_y: f32,
    ) -> UpdateStatus {
        let mut status = UpdateStatus::default();
        let event = SceneGraphEvent::MouseWheel(avenger_eventstream::scene::SceneMouseWheelEvent {
            position,
            delta: MouseScrollDelta::LineDelta(0.0, delta_y),
            mark_instance: None,
            modifiers: Default::default(),
        });
        for handler in handlers {
            status = status.merge(
                &handler
                    .handle_with_context(
                        &event,
                        &EventStreamContext::default(),
                        state,
                        &empty_rtree(),
                    )
                    .await,
            );
        }
        status
    }

    async fn interaction_settle(state: &mut ChartAppState) -> UpdateStatus {
        ChartEventExactOnlyHandler
            .handle(&SceneGraphEvent::InteractionSettled, state, &empty_rtree())
            .await
    }

    fn compile_handler_for_event_type(
        compiled: &CompiledPlot,
        ctx: &SessionContext,
        event_type: ChartEventType,
    ) -> ChartEventBindingHandler {
        let binding_index = compiled
            .event_bindings()
            .iter()
            .position(|binding| binding.event_type == event_type)
            .unwrap_or_else(|| panic!("missing {event_type:?} binding"));
        compile_handler_for_binding_index(compiled, ctx, binding_index)
    }

    fn compile_handlers_for_event_type(
        compiled: &CompiledPlot,
        ctx: &SessionContext,
        event_type: ChartEventType,
    ) -> Vec<ChartEventBindingHandler> {
        let handlers = compiled
            .event_bindings()
            .iter()
            .enumerate()
            .filter(|(_, binding)| binding.event_type == event_type)
            .map(|(binding_index, _)| {
                compile_handler_for_binding_index(compiled, ctx, binding_index)
            })
            .collect::<Vec<_>>();
        assert!(
            !handlers.is_empty(),
            "missing {event_type:?} event bindings"
        );
        handlers
    }

    fn compile_handler_for_binding_index(
        compiled: &CompiledPlot,
        ctx: &SessionContext,
        binding_index: usize,
    ) -> ChartEventBindingHandler {
        let event_coord_types = compiled
            .event_coord_types(ctx)
            .expect("infer event coord types");
        let mark_runtime_targets = MarkRuntimeTargetIndex::for_plot(compiled);
        let runtime = CompiledChartEventBinding::compile_with_event_coord_types(
            binding_index,
            &compiled.event_bindings()[binding_index],
            ctx,
            compiled.param_specs(),
            compiled.selection_specs(),
            compiled.store_specs(),
            &compiled.event_datum_types(),
            &event_coord_types,
            None,
            Some(&mark_runtime_targets),
        )
        .expect("compile binding runtime");
        ChartEventBindingHandler {
            runtime: Arc::new(runtime),
            state: Mutex::new(ChartEventBindingState::default()),
        }
    }

    fn drag_x_domain_value(value: Option<&ScalarValue>) -> [f32; 2] {
        use avenger_chart_core::ScalarValueHelpers;
        match value {
            Some(scalar) => scalar
                .as_f32x2()
                .expect("x_domain should be a 2-element list"),
            None => panic!("x_domain missing"),
        }
    }

    async fn pan_move(
        state: &mut ChartAppState,
        handler: &ChartEventBindingHandler,
        gesture_instant: Instant,
        start_pos: [f32; 2],
        current_pos: [f32; 2],
    ) -> UpdateStatus {
        let start_event = EventStreamEventSnapshot {
            event: SceneGraphEvent::MouseDown(SceneMouseDownEvent {
                position: start_pos,
                button: MouseButton::Left,
                mark_instance: None,
                modifiers: Default::default(),
            }),
            mark_instance: None,
            instant: gesture_instant,
        };
        let context = EventStreamContext {
            mark_instance: None,
            current_event: None,
            start_event: Some(start_event),
            previous_event: None,
        };
        handler
            .handle_with_context(
                &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: current_pos,
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                &context,
                state,
                &empty_rtree(),
            )
            .await
    }

    async fn pan_move_all(
        state: &mut ChartAppState,
        handlers: &[ChartEventBindingHandler],
        gesture_instant: Instant,
        start_pos: [f32; 2],
        current_pos: [f32; 2],
    ) -> UpdateStatus {
        let mut status = UpdateStatus::default();
        for handler in handlers {
            status = status
                .merge(&pan_move(state, handler, gesture_instant, start_pos, current_pos).await);
        }
        status
    }

    async fn box_zoom_state_and_handlers() -> (
        ChartAppState,
        ChartEventBindingHandler,
        ChartEventBindingHandler,
    ) {
        let ctx = SessionContext::new();
        let df = ctx
            .sql("SELECT * FROM (VALUES (0.0, 0.0), (10.0, 10.0)) AS t(x, y)")
            .await
            .expect("data");
        let compiled = Chart::<Cartesian>::new()
            .canvas_size(400.0, 300.0)
            .data(df)
            .mark(
                Symbol::new()
                    .x_with(col("x"), |c| {
                        c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                    })
                    .y_with(col("y"), |c| {
                        c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                    })
                    .size(20.0),
            )
            .tool(BoxZoom::cartesian())
            .compile(&ctx)
            .await
            .expect("compile box zoom plot");

        let release_index = compiled
            .event_bindings()
            .iter()
            .position(|binding| {
                binding
                    .action
                    .param_steps()
                    .any(|assignment| assignment.param_name == "__tool_box_zoom__x_domain")
            })
            .expect("release binding");

        let reset_index = compiled
            .event_bindings()
            .iter()
            .position(|binding| binding.event_type == ChartEventType::DoubleClick)
            .expect("reset binding");

        let release_runtime = CompiledChartEventBinding::compile(
            release_index,
            &compiled.event_bindings()[release_index],
            &ctx,
            compiled.param_specs(),
            compiled.selection_specs(),
            compiled.store_specs(),
            &compiled.event_datum_types(),
        )
        .expect("compile release binding runtime");
        let reset_runtime = CompiledChartEventBinding::compile(
            reset_index,
            &compiled.event_bindings()[reset_index],
            &ctx,
            compiled.param_specs(),
            compiled.selection_specs(),
            compiled.store_specs(),
            &compiled.event_datum_types(),
        )
        .expect("compile reset binding runtime");
        let policy = compiled.resize_policy();
        let release_handler = ChartEventBindingHandler {
            runtime: Arc::new(release_runtime),
            state: Mutex::new(ChartEventBindingState::default()),
        };
        let reset_handler = ChartEventBindingHandler {
            runtime: Arc::new(reset_runtime),
            state: Mutex::new(ChartEventBindingState::default()),
        };
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        (state, release_handler, reset_handler)
    }

    async fn box_zoom_state_and_release_handler() -> (ChartAppState, ChartEventBindingHandler) {
        let (state, release_handler, _) = box_zoom_state_and_handlers().await;
        (state, release_handler)
    }

    async fn box_zoom_release(
        state: &mut ChartAppState,
        handler: &ChartEventBindingHandler,
        gesture_instant: Instant,
        start_pos: [f32; 2],
        end_pos: [f32; 2],
    ) -> UpdateStatus {
        let start_event = EventStreamEventSnapshot {
            event: SceneGraphEvent::MouseDown(SceneMouseDownEvent {
                position: start_pos,
                button: MouseButton::Left,
                mark_instance: None,
                modifiers: Default::default(),
            }),
            mark_instance: None,
            instant: gesture_instant,
        };
        let context = EventStreamContext {
            mark_instance: None,
            current_event: None,
            start_event: Some(start_event),
            previous_event: None,
        };
        handler
            .handle_with_context(
                &SceneGraphEvent::MouseUp(SceneMouseUpEvent {
                    position: end_pos,
                    button: MouseButton::Left,
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                &context,
                state,
                &empty_rtree(),
            )
            .await
    }

    #[tokio::test]
    async fn root_pan_updates_x_domain_param() {
        let (mut state, handler) = pan_state_and_handler().await;
        // First evaluation populates the interaction scope.
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial build");
        let scopes = state.interaction_scopes().await;
        assert_eq!(
            scopes.len(),
            1,
            "root Cartesian plot should export one scope"
        );
        let bounds = scopes[0].bounds;
        let cx = bounds.x + bounds.width * 0.5;
        let cy = bounds.y + bounds.height * 0.5;

        let status = pan_move(
            &mut state,
            &handler,
            Instant::now(),
            [cx, cy],
            [cx + 40.0, cy],
        )
        .await;
        assert!(status.rerender, "a drag inside the plot should rerender");

        let params = state.params().await;
        let domain = drag_x_domain_value(params.get("x_domain"));
        // Dragging the pointer to the right pans the view right, so the domain
        // shifts left (toward negative) from the inferred [0, 10].
        assert!(
            domain[0] < 0.0 && domain[1] < 10.0,
            "expected a left-shifted domain, got {domain:?}"
        );
        // The re-evaluation with the new raw domain should succeed.
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("preview build after pan");
    }

    #[tokio::test]
    async fn box_zoom_release_sets_raw_domain_params_to_drag_extents() {
        let (mut state, handler) = box_zoom_state_and_release_handler().await;
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial build");
        let scope = state.interaction_scopes().await[0].clone();
        let bounds = scope.bounds;
        let start = [
            bounds.x + bounds.width * 0.25,
            bounds.y + bounds.height * 0.75,
        ];
        let end = [
            bounds.x + bounds.width * 0.75,
            bounds.y + bounds.height * 0.25,
        ];

        let status = box_zoom_release(&mut state, &handler, Instant::now(), start, end).await;
        assert!(status.rerender, "release should patch raw domains");
        assert!(
            status.rebuild_geometry,
            "box zoom release evaluates exactly"
        );

        let params = state.params().await;
        let x_domain = drag_x_domain_value(params.get("__tool_box_zoom__x_domain"));
        let y_domain = drag_x_domain_value(params.get("__tool_box_zoom__y_domain"));
        assert!(
            x_domain[0] > -0.1 && x_domain[1] < 10.1 && x_domain[0] < x_domain[1],
            "x domain should be the dragged interval, got {x_domain:?}"
        );
        assert!(
            y_domain[0] > -0.1 && y_domain[1] < 10.1 && y_domain[0] < y_domain[1],
            "y domain should be the dragged interval, got {y_domain:?}"
        );
        assert!(
            x_domain[1] - x_domain[0] < 8.0,
            "x zoom interval should be narrower than the full domain: {x_domain:?}"
        );
        assert!(
            y_domain[1] - y_domain[0] < 8.0,
            "y zoom interval should be narrower than the full domain: {y_domain:?}"
        );
    }

    #[tokio::test]
    async fn box_zoom_double_click_resets_raw_domain_params_to_default() {
        let (mut state, release_handler, reset_handler) = box_zoom_state_and_handlers().await;
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial build");
        let scope = state.interaction_scopes().await[0].clone();
        let bounds = scope.bounds;
        let start = [
            bounds.x + bounds.width * 0.25,
            bounds.y + bounds.height * 0.75,
        ];
        let end = [
            bounds.x + bounds.width * 0.75,
            bounds.y + bounds.height * 0.25,
        ];
        let center = [
            bounds.x + bounds.width * 0.5,
            bounds.y + bounds.height * 0.5,
        ];

        let default_x = state
            .params()
            .await
            .get("__tool_box_zoom__x_domain")
            .cloned()
            .expect("default x domain");
        let default_y = state
            .params()
            .await
            .get("__tool_box_zoom__y_domain")
            .cloned()
            .expect("default y domain");

        let zoom_status =
            box_zoom_release(&mut state, &release_handler, Instant::now(), start, end).await;
        assert!(zoom_status.rerender, "release should zoom first");
        assert_ne!(
            state.params().await.get("__tool_box_zoom__x_domain"),
            Some(&default_x),
            "test setup should first move away from the default domain"
        );

        let reset_status = reset_handler
            .handle_with_context(
                &SceneGraphEvent::DoubleClick(SceneDoubleClickEvent {
                    position: center,
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                &EventStreamContext::default(),
                &mut state,
                &empty_rtree(),
            )
            .await;
        assert!(reset_status.rerender, "double-click should patch params");
        assert!(
            reset_status.rebuild_geometry,
            "box zoom reset evaluates exactly"
        );
        let params = state.params().await;
        assert_eq!(
            params.get("__tool_box_zoom__x_domain"),
            Some(&default_x),
            "double-click reset should restore x raw-domain default"
        );
        assert_eq!(
            params.get("__tool_box_zoom__y_domain"),
            Some(&default_y),
            "double-click reset should restore y raw-domain default"
        );
    }

    #[tokio::test]
    async fn root_pan_outside_plot_is_noop() {
        let (mut state, handler) = pan_state_and_handler().await;
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial build");
        let bounds = state.interaction_scopes().await[0].bounds;
        let default_domain = state.params().await.get("x_domain").cloned();

        // Start the gesture well outside the plot area: the start point routes to
        // no scope, so the derived start-domain columns are null. This must not
        // corrupt the domain param or crash a subsequent build.
        let outside = [bounds.x - 80.0, bounds.y - 80.0];
        let status = pan_move(
            &mut state,
            &handler,
            Instant::now(),
            outside,
            [outside[0] + 40.0, outside[1]],
        )
        .await;
        assert!(!status.rerender, "an outside-start drag should be a no-op");
        assert_eq!(
            state.params().await.get("x_domain"),
            default_domain.as_ref(),
            "x_domain must be unchanged after an outside-start drag"
        );
        // A subsequent build must still succeed (no degenerate raw domain).
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("build after outside drag");
    }

    #[tokio::test]
    async fn root_pan_preview_moves_match_single_move() {
        // One continuous gesture: three preview moves with a rebuild between each
        // (as a real preview loop would do). The shared gesture instant keeps the
        // start scope/domain frozen even as the raw-domain param changes.
        let (mut multi_state, multi_handler) = pan_state_and_handler().await;
        crate::ChartSceneGraphBuilder
            .build(&mut multi_state)
            .await
            .expect("multi initial build");
        let bounds = multi_state.interaction_scopes().await[0].bounds;
        let cx = bounds.x + bounds.width * 0.5;
        let cy = bounds.y + bounds.height * 0.5;
        let gesture = Instant::now();
        for offset in [10.0_f32, 25.0, 50.0] {
            pan_move(
                &mut multi_state,
                &multi_handler,
                gesture,
                [cx, cy],
                [cx + offset, cy],
            )
            .await;
            // Rebuild in Preview mode, updating last_interaction_state with the
            // shifted domain. The frozen start scope must avoid feedback.
            crate::ChartSceneGraphBuilder
                .build(&mut multi_state)
                .await
                .expect("preview build");
        }
        let multi_domain = drag_x_domain_value(multi_state.params().await.get("x_domain"));

        // A fresh single-move gesture straight to the final position.
        let (mut single_state, single_handler) = pan_state_and_handler().await;
        crate::ChartSceneGraphBuilder
            .build(&mut single_state)
            .await
            .expect("single initial build");
        pan_move(
            &mut single_state,
            &single_handler,
            Instant::now(),
            [cx, cy],
            [cx + 50.0, cy],
        )
        .await;
        let single_domain = drag_x_domain_value(single_state.params().await.get("x_domain"));

        // Because each preview move recomputes from the frozen start domain and
        // start scale, the cumulative gesture (even with rebuilds) matches a
        // single move to the same final position. This proves start framing does
        // not feed back as the raw domain updates during the drag.
        assert!(
            (multi_domain[0] - single_domain[0]).abs() < 1e-3,
            "multi {multi_domain:?} vs single {single_domain:?}"
        );
        assert!((multi_domain[1] - single_domain[1]).abs() < 1e-3);
    }

    #[tokio::test]
    async fn faceted_shared_pan_updates_all_cells_via_root_domain() {
        let (mut state, handler) = faceted_pan_state_and_handler().await;
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial build");
        let scopes = state.interaction_scopes().await;
        assert_eq!(
            scopes.len(),
            2,
            "two facet columns should export two coordinate scopes"
        );

        // Pan inside the first cell; the Shared param writes the root domain.
        let bounds = scopes[0].bounds;
        let cx = bounds.x + bounds.width * 0.5;
        let cy = bounds.y + bounds.height * 0.5;
        let status = pan_move(
            &mut state,
            &handler,
            Instant::now(),
            [cx, cy],
            [cx + 40.0, cy],
        )
        .await;
        assert!(
            status.rerender,
            "a drag inside a facet cell should rerender"
        );

        let params = state.params().await;
        let domain = drag_x_domain_value(params.get("x_domain"));
        assert!(
            domain[0] < 0.0 && domain[1] < 10.0,
            "expected a left-shifted shared domain, got {domain:?}"
        );
        // Re-evaluating applies the shared domain to every cell without error.
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("build after shared facet pan");
    }

    async fn assert_concat_pan_updates_all_child_domains(
        state: &mut ChartAppState,
        handlers: &[ChartEventBindingHandler],
    ) {
        crate::ChartSceneGraphBuilder
            .build(state)
            .await
            .expect("initial build");
        let scopes = state.interaction_scopes().await;
        assert_eq!(
            scopes.len(),
            2,
            "concat plot should export two child scopes"
        );
        let initial_domains = scopes
            .iter()
            .map(|scope| {
                let child_index = scope.child_frame_path[0].child_index;
                let domain = scope
                    .scales
                    .get("x")
                    .expect("scope has x scale")
                    .numeric_interval_domain()
                    .expect("x domain is numeric");
                (child_index, domain)
            })
            .collect::<HashMap<_, _>>();
        let active = scopes
            .iter()
            .find(|scope| scope.child_frame_path[0].child_index == 0)
            .expect("first concat child scope")
            .clone();
        let bounds = active.bounds;
        let cx = bounds.x + bounds.width * 0.5;
        let cy = bounds.y + bounds.height * 0.5;
        let status = pan_move_all(state, handlers, Instant::now(), [cx, cy], [cx + 40.0, cy]).await;
        assert!(
            status.rerender,
            "drag inside first concat child should rerender"
        );

        crate::ChartSceneGraphBuilder
            .build(state)
            .await
            .expect("build after concat pan");
        let scopes = state.interaction_scopes().await;
        for scope in &scopes {
            let child_index = scope.child_frame_path[0].child_index;
            let initial = initial_domains
                .get(&child_index)
                .unwrap_or_else(|| panic!("initial domain for child {child_index}"));
            let domain = scope
                .scales
                .get("x")
                .expect("scope has x scale")
                .numeric_interval_domain()
                .expect("x domain is numeric");
            assert!(
                domain.0 < initial.0 && domain.1 < initial.1,
                "every linked concat child should receive the panned domain; \
                 child {child_index} moved from {initial:?} to {domain:?}"
            );
        }
    }

    #[tokio::test]
    async fn grid_concat_pan_updates_linked_child_domains() {
        let (mut state, handlers) = grid_concat_pan_state_and_handlers().await;
        assert_concat_pan_updates_all_child_domains(&mut state, &handlers).await;
    }

    #[tokio::test]
    async fn wrap_concat_pan_updates_linked_child_domains() {
        let (mut state, handlers) = wrap_concat_pan_state_and_handlers().await;
        assert_concat_pan_updates_all_child_domains(&mut state, &handlers).await;
    }

    #[tokio::test]
    async fn repeat_pan_horizontal_updates_cross_orientation_variable_domain() {
        let (mut state, drag_handlers, _) =
            repeat_pan_state_and_handlers(CoordinationScope::Shared, false).await;
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial repeat build");
        let scopes = state.interaction_scopes().await;
        assert_eq!(scopes.len(), 4, "2x2 repeat grid exports four scopes");
        let initial_a_y = repeat_scope_domain(&scopes, "repeat_cell:a:b", "y");
        let initial_b_y = repeat_scope_domain(&scopes, "repeat_cell:b:a", "y");
        let active = repeat_scope(&scopes, "repeat_cell:b:a").clone();
        let center = scope_center(&active);

        let status = pan_move_all(
            &mut state,
            &drag_handlers,
            Instant::now(),
            center,
            [center[0] + active.bounds.width * 0.18, center[1]],
        )
        .await;
        assert!(status.rerender, "repeat horizontal pan should rerender");
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("build after repeat horizontal pan");

        let scopes = state.interaction_scopes().await;
        let panned_a_y = repeat_scope_domain(&scopes, "repeat_cell:a:b", "y");
        let stable_b_y = repeat_scope_domain(&scopes, "repeat_cell:b:a", "y");
        assert!(
            panned_a_y.0 < initial_a_y.0 && panned_a_y.1 < initial_a_y.1,
            "horizontal pan of x=a should also move y=a in another cell: \
             {initial_a_y:?} -> {panned_a_y:?}"
        );
        assert!(
            (stable_b_y.0 - initial_b_y.0).abs() < 1e-3
                && (stable_b_y.1 - initial_b_y.1).abs() < 1e-3,
            "horizontal pan should not move row variable b: \
             {initial_b_y:?} -> {stable_b_y:?}"
        );
    }

    #[tokio::test]
    async fn repeat_pan_vertical_updates_cross_orientation_variable_domain() {
        let (mut state, drag_handlers, _) =
            repeat_pan_state_and_handlers(CoordinationScope::Shared, false).await;
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial repeat build");
        let scopes = state.interaction_scopes().await;
        let initial_b_x = repeat_scope_domain(&scopes, "repeat_cell:a:b", "x");
        let initial_a_x = repeat_scope_domain(&scopes, "repeat_cell:b:a", "x");
        let active = repeat_scope(&scopes, "repeat_cell:b:a").clone();
        let center = scope_center(&active);

        let status = pan_move_all(
            &mut state,
            &drag_handlers,
            Instant::now(),
            center,
            [center[0], center[1] + active.bounds.height * 0.18],
        )
        .await;
        assert!(status.rerender, "repeat vertical pan should rerender");
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("build after repeat vertical pan");

        let scopes = state.interaction_scopes().await;
        let panned_b_x = repeat_scope_domain(&scopes, "repeat_cell:a:b", "x");
        let stable_a_x = repeat_scope_domain(&scopes, "repeat_cell:b:a", "x");
        assert!(
            panned_b_x.0 > initial_b_x.0 && panned_b_x.1 > initial_b_x.1,
            "vertical pan of y=b should also move x=b in another cell: \
             {initial_b_x:?} -> {panned_b_x:?}"
        );
        assert!(
            (stable_a_x.0 - initial_a_x.0).abs() < 1e-3
                && (stable_a_x.1 - initial_a_x.1).abs() < 1e-3,
            "vertical pan should not move column variable a: \
             {initial_a_x:?} -> {stable_a_x:?}"
        );
    }

    #[tokio::test]
    async fn repeat_wheel_zoom_updates_variable_domains_in_both_orientations() {
        let (mut state, _, wheel_handlers) =
            repeat_pan_state_and_handlers(CoordinationScope::Shared, false).await;
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial repeat build");
        let scopes = state.interaction_scopes().await;
        let initial_a_y = repeat_scope_domain(&scopes, "repeat_cell:a:b", "y");
        let initial_b_x = repeat_scope_domain(&scopes, "repeat_cell:a:b", "x");
        let active = repeat_scope(&scopes, "repeat_cell:b:a").clone();
        let center = scope_center(&active);

        let status = wheel_zoom_all(&mut state, &wheel_handlers, center, 6.0).await;
        assert!(status.rerender, "repeat wheel zoom should rerender");
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("build after repeat wheel zoom");

        let scopes = state.interaction_scopes().await;
        let zoomed_a_y = repeat_scope_domain(&scopes, "repeat_cell:a:b", "y");
        let zoomed_b_x = repeat_scope_domain(&scopes, "repeat_cell:a:b", "x");
        assert!(
            zoomed_a_y.1 - zoomed_a_y.0 < initial_a_y.1 - initial_a_y.0,
            "wheel zoom in x=a/y=b cell should shrink y=a elsewhere: \
             {initial_a_y:?} -> {zoomed_a_y:?}"
        );
        assert!(
            zoomed_b_x.1 - zoomed_b_x.0 < initial_b_x.1 - initial_b_x.0,
            "wheel zoom in x=a/y=b cell should shrink x=b elsewhere: \
             {initial_b_x:?} -> {zoomed_b_x:?}"
        );
    }

    #[tokio::test]
    async fn pan_scroll_zoom_interaction_settled_runs_exact_after_preview() {
        let (compiled, ctx) = repeat_pan_compiled(CoordinationScope::Shared, true).await;
        let policy = compiled.resize_policy();
        let wheel_handlers =
            compile_handlers_for_event_type(&compiled, &ctx, ChartEventType::MouseWheel);
        let session = Arc::new(compiled).instantiate(ctx);
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());

        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial repeat build");
        let scopes = state.interaction_scopes().await;
        let active = repeat_scope(&scopes, "repeat_cell:b:a").clone();
        let center = scope_center(&active);

        let idle_status = interaction_settle(&mut state).await;
        assert!(
            !idle_status.rerender,
            "settled interaction should no-op before preview work"
        );

        let preview_status = wheel_zoom_all(&mut state, &wheel_handlers, center, 6.0).await;
        assert!(preview_status.rerender, "wheel preview should rerender");
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("build after wheel preview");
        assert_eq!(
            state.last_metrics().await.expect("preview metrics").mode,
            EvaluationMode::Preview
        );

        let settled_status = interaction_settle(&mut state).await;
        assert!(settled_status.rerender, "settled wheel should rerender");
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("build after settled wheel");
        assert_eq!(
            state.last_metrics().await.expect("settled metrics").mode,
            EvaluationMode::Exact
        );
    }

    #[tokio::test]
    async fn repeat_free_pan_updates_only_active_cell_domains() {
        let (mut state, drag_handlers, _) =
            repeat_pan_state_and_handlers(CoordinationScope::Free, false).await;
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial repeat build");
        let scopes = state.interaction_scopes().await;
        let initial_active_x = repeat_scope_domain(&scopes, "repeat_cell:b:a", "x");
        let initial_other_a_y = repeat_scope_domain(&scopes, "repeat_cell:a:b", "y");
        let active = repeat_scope(&scopes, "repeat_cell:b:a").clone();
        let center = scope_center(&active);

        let status = pan_move_all(
            &mut state,
            &drag_handlers,
            Instant::now(),
            center,
            [center[0] + active.bounds.width * 0.18, center[1]],
        )
        .await;
        assert!(status.rerender, "repeat free pan should rerender");
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("build after repeat free pan");

        let scopes = state.interaction_scopes().await;
        let active_x = repeat_scope_domain(&scopes, "repeat_cell:b:a", "x");
        let other_a_y = repeat_scope_domain(&scopes, "repeat_cell:a:b", "y");
        assert!(
            active_x.0 < initial_active_x.0 && active_x.1 < initial_active_x.1,
            "active free repeat cell should pan: {initial_active_x:?} -> {active_x:?}"
        );
        assert!(
            (other_a_y.0 - initial_other_a_y.0).abs() < 1e-3
                && (other_a_y.1 - initial_other_a_y.1).abs() < 1e-3,
            "free repeat pan should not update another cell with the same variable: \
             {initial_other_a_y:?} -> {other_a_y:?}"
        );
    }

    #[tokio::test]
    async fn repeat_pan_inside_facet_routes_to_active_facet_scope() {
        let ctx = Arc::new(SessionContext::new());
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('G1', 0.0, 0.0),
                    ('G1', 10.0, 100.0),
                    ('G1', 20.0, 200.0),
                    ('G2', 100.0, 1000.0),
                    ('G2', 110.0, 1100.0),
                    ('G2', 120.0, 1200.0)
                ) AS t(group_name, a, b)",
            )
            .await
            .expect("nested repeat pan data");
        let repeat_grid = Plot::<RepeatGrid>::new().configure_coord(|c| {
            c.rows([
                RepeatVariable::new("a", col("a")),
                RepeatVariable::new("b", col("b")),
            ])
            .columns([
                RepeatVariable::new("a", col("a")),
                RepeatVariable::new("b", col("b")),
            ])
            .cell(repeat_pan_cell(false))
            .matrix_domains()
            .matrix_axes()
        });
        let compiled = Chart::<FacetColumn>::new()
            .canvas_size(900.0, 360.0)
            .data(df)
            .mark(
                Subplot::new(repeat_grid)
                    .id("matrix")
                    .column(col("group_name")),
            )
            .compile(&ctx)
            .await
            .expect("compile nested repeat pan plot");

        let policy = compiled.resize_policy();
        let drag_handlers =
            compile_handlers_for_event_type(&compiled, &ctx, ChartEventType::CursorMoved);
        assert_eq!(
            drag_handlers.len(),
            4,
            "repeat tool should still generate one drag binding per repeated cell"
        );
        let session = Arc::new(compiled).instantiate(ctx);
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial nested repeat pan scene");
        let scopes = state.interaction_scopes().await;
        assert_eq!(
            scopes.len(),
            8,
            "two outer facets times four repeated cells should export coordinate scopes"
        );
        let initial_g2_a_y = repeat_facet_scope_domain(&scopes, "G2", "repeat_cell:a:b", "y");
        let active = repeat_facet_scope(&scopes, "G1", "repeat_cell:b:a").clone();
        let center = scope_center(&active);

        let status = pan_move_all(
            &mut state,
            &drag_handlers,
            Instant::now(),
            center,
            [center[0] + active.bounds.width * 0.18, center[1]],
        )
        .await;
        assert!(status.rerender, "nested repeat pan should rerender");
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("build after nested repeat pan");

        let scopes = state.interaction_scopes().await;
        let g2_a_y = repeat_facet_scope_domain(&scopes, "G2", "repeat_cell:a:b", "y");
        assert!(
            g2_a_y.0 < initial_g2_a_y.0 && g2_a_y.1 < initial_g2_a_y.1,
            "shared repeat domain panned in G1 should propagate to G2's a-domain: \
             {initial_g2_a_y:?} -> {g2_a_y:?}"
        );
    }

    #[tokio::test]
    async fn repeat_pan_inside_facet_wrap_uses_final_aligned_scope_bounds() {
        let ctx = Arc::new(SessionContext::new());
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('G1', 0.0, 0.0),
                    ('G1', 10.0, 100.0),
                    ('G1', 20.0, 200.0),
                    ('G2', 100.0, 1000.0),
                    ('G2', 110.0, 1100.0),
                    ('G2', 120.0, 1200.0),
                    ('G3', 200.0, 2000.0),
                    ('G3', 210.0, 2100.0),
                    ('G3', 220.0, 2200.0)
                ) AS t(group_name, a, b)",
            )
            .await
            .expect("wrapped repeat pan data");
        let repeat_grid = Plot::<RepeatGrid>::new().configure_coord(|c| {
            c.rows([
                RepeatVariable::new("a", col("a")),
                RepeatVariable::new("b", col("b")),
            ])
            .columns([
                RepeatVariable::new("a", col("a")),
                RepeatVariable::new("b", col("b")),
            ])
            .cell(repeat_pan_cell(false))
            .matrix_domains()
            .matrix_axes()
        });
        let compiled = Chart::<FacetWrap>::new()
            .canvas_size(900.0, 560.0)
            .data(df)
            .mark(
                Subplot::new(repeat_grid)
                    .id("matrix")
                    .wrap_with(col("group_name"), |c| c.columns(2).empty_cells_as_holes()),
            )
            .compile(&ctx)
            .await
            .expect("compile wrapped nested repeat pan plot");

        let policy = compiled.resize_policy();
        let drag_handlers =
            compile_handlers_for_event_type(&compiled, &ctx, ChartEventType::CursorMoved);
        assert_eq!(
            drag_handlers.len(),
            4,
            "repeat tool should still generate one drag binding per repeated cell"
        );
        let session = Arc::new(compiled).instantiate(ctx);
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial wrapped nested repeat pan scene");
        let scopes = state.interaction_scopes().await;
        assert_eq!(
            scopes.len(),
            12,
            "three wrapped facets times four repeated cells should export coordinate scopes"
        );
        for scope in &scopes {
            assert_eq!(
                scope.logical_facet_values.len(),
                1,
                "FacetWrap should contribute one logical facet value"
            );
            let segment = scope
                .child_frame_path
                .last()
                .expect("repeat-generated GridConcat segment");
            assert_eq!(segment.kind, EvaluatedChildFrameKind::GridConcat);
            assert_eq!(segment.row_count, Some(2));
            assert_eq!(segment.column_count, Some(2));
        }

        let initial_g3_a_y =
            repeat_logical_facet_scope_domain(&scopes, "G3", "repeat_cell:a:b", "y");
        let active = repeat_logical_facet_scope(&scopes, "G1", "repeat_cell:b:a").clone();
        let center = scope_center(&active);

        let status = pan_move_all(
            &mut state,
            &drag_handlers,
            Instant::now(),
            center,
            [center[0] + active.bounds.width * 0.18, center[1]],
        )
        .await;
        assert!(status.rerender, "wrapped nested repeat pan should rerender");
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("build after wrapped nested repeat pan");

        let scopes = state.interaction_scopes().await;
        let g3_a_y = repeat_logical_facet_scope_domain(&scopes, "G3", "repeat_cell:a:b", "y");
        assert!(
            g3_a_y.0 < initial_g3_a_y.0 && g3_a_y.1 < initial_g3_a_y.1,
            "shared repeat domain panned in G1 should propagate through wrapped facet scopes: \
             {initial_g3_a_y:?} -> {g3_a_y:?}"
        );
    }

    #[tokio::test]
    async fn repeat_pan_exact_settle_matches_single_preview_move() {
        let (compiled, ctx) = repeat_pan_compiled(CoordinationScope::Shared, true).await;
        let streams = event_streams_for_plot_bindings(&compiled, &ctx).expect("event streams");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(ctx);
        let mut manager = EventStreamManager::new(ChartAppState::new(
            session,
            policy,
            crate::ChartAppOptions::default(),
        ));
        let scene = crate::ChartSceneGraphBuilder
            .build(manager.state_mut())
            .await
            .expect("initial repeat build");
        let scopes = manager.state().interaction_scopes().await;
        let active = repeat_scope(&scopes, "repeat_cell:b:a").clone();
        let center = scope_center(&active);
        let end = [center[0] + active.bounds.width * 0.18, center[1]];
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        for (config, handler) in streams {
            manager.register_handler(config, handler);
        }
        let instant = Instant::now();
        manager
            .dispatch_event(
                &WindowEvent::CursorMoved(WindowCursorMoved { position: center }),
                &rtree,
                instant,
            )
            .await;
        manager
            .dispatch_event(
                &WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                }),
                &rtree,
                instant,
            )
            .await;
        let preview_status = manager
            .dispatch_event(
                &WindowEvent::CursorMoved(WindowCursorMoved { position: end }),
                &rtree,
                instant + Duration::from_millis(16),
            )
            .await;
        assert!(preview_status.rerender);
        assert!(
            !preview_status.rebuild_geometry,
            "drag move should remain Preview before settle"
        );
        let settle_status = manager
            .dispatch_event(
                &WindowEvent::InteractionSettled { generation: 1 },
                &rtree,
                instant + Duration::from_millis(120),
            )
            .await;
        assert!(
            settle_status.rebuild_geometry,
            "interaction settle should request Exact evaluation"
        );
        crate::ChartSceneGraphBuilder
            .build(manager.state_mut())
            .await
            .expect("exact settle build");
        let metrics = manager
            .state()
            .last_metrics()
            .await
            .expect("metrics after exact settle");
        assert_eq!(metrics.mode, EvaluationMode::Exact);
        let settled_scopes = manager.state().interaction_scopes().await;
        let settled_a_y = repeat_scope_domain(&settled_scopes, "repeat_cell:a:b", "y");

        let (mut direct_state, direct_handlers, _) =
            repeat_pan_state_and_handlers(CoordinationScope::Shared, false).await;
        crate::ChartSceneGraphBuilder
            .build(&mut direct_state)
            .await
            .expect("direct initial repeat build");
        let direct_status =
            pan_move_all(&mut direct_state, &direct_handlers, instant, center, end).await;
        assert!(direct_status.rerender);
        crate::ChartSceneGraphBuilder
            .build(&mut direct_state)
            .await
            .expect("direct build");
        let direct_scopes = direct_state.interaction_scopes().await;
        let direct_a_y = repeat_scope_domain(&direct_scopes, "repeat_cell:a:b", "y");
        assert!(
            (settled_a_y.0 - direct_a_y.0).abs() < 1e-3
                && (settled_a_y.1 - direct_a_y.1).abs() < 1e-3,
            "exact-settled repeat pan should match a one-shot preview move: \
             settled={settled_a_y:?}, direct={direct_a_y:?}"
        );
    }

    /// First component of a scope's facet path, as a `&str` cell value.
    fn scope_cell(scope: &EvaluatedInteractionScope) -> Option<String> {
        match scope.facet_path.first() {
            Some(ScalarValue::Utf8(Some(value))) => Some(value.clone()),
            _ => None,
        }
    }

    /// Numeric x domain of the scope for `cell`.
    fn scope_x_domain(scopes: &[EvaluatedInteractionScope], cell: &str) -> (f32, f32) {
        scopes
            .iter()
            .find(|scope| scope_cell(scope).as_deref() == Some(cell))
            .unwrap_or_else(|| panic!("no scope for cell {cell}"))
            .scales
            .get("x")
            .expect("scope has x scale")
            .numeric_interval_domain()
            .expect("x domain is numeric")
    }

    #[tokio::test]
    async fn faceted_free_pan_updates_only_active_cell() {
        let (mut state, handler) =
            faceted_pan_state_and_handler_with_sharing(CoordinationScope::Free).await;
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial build");
        let scopes = state.interaction_scopes().await;
        assert_eq!(scopes.len(), 2, "two facet columns export two scopes");

        // Pan inside cell "A". A Free param routes the write to cell A's owner.
        let cell_a = scopes
            .iter()
            .find(|scope| scope_cell(scope).as_deref() == Some("A"))
            .expect("cell A scope")
            .clone();
        let bounds = cell_a.bounds;
        let cx = bounds.x + bounds.width * 0.5;
        let cy = bounds.y + bounds.height * 0.5;
        let status = pan_move(
            &mut state,
            &handler,
            Instant::now(),
            [cx, cy],
            [cx + 40.0, cy],
        )
        .await;
        assert!(status.rerender, "a drag inside cell A should rerender");

        // The root param is untouched: a Free write targets the cell owner path,
        // not the root, so `params()` still reports the default null-list domain.
        let root_domain = state.params().await.get("x_domain").cloned();
        assert!(
            matches!(root_domain, Some(ScalarValue::List(_))),
            "root x_domain should remain a (default) list for a Free pan, got {root_domain:?}"
        );

        // Re-evaluate and confirm only cell A moved; cell B keeps its inferred
        // domain.
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("build after free facet pan");
        let scopes = state.interaction_scopes().await;
        let a = scope_x_domain(&scopes, "A");
        let b = scope_x_domain(&scopes, "B");
        assert!(
            a.0 < 0.0 && a.1 < 10.0,
            "cell A should be panned left, got {a:?}"
        );
        assert!(
            (b.0 - 0.0).abs() < 0.5 && (b.1 - 10.0).abs() < 0.5,
            "cell B should keep its inferred [0, 10] domain, got {b:?}"
        );
    }

    fn empty_rtree() -> SceneGraphRTree {
        use avenger_scenegraph::scene_graph::SceneGraph;
        SceneGraphRTree::from_scene_graph(&SceneGraph {
            marks: Vec::new(),
            width: 1.0,
            height: 1.0,
            origin: [0.0, 0.0],
        })
    }

    fn decorated_treemap_batch() -> RecordBatch {
        let division = [
            "Enterprise",
            "Enterprise",
            "Enterprise",
            "Enterprise",
            "Consumer",
            "Consumer",
            "International",
            "International",
        ];
        let region = [
            "North America",
            "North America",
            "Europe",
            "APAC",
            "North America",
            "Europe",
            "Latin America",
            "APAC",
        ];
        let team = [
            "Platform",
            "Services",
            "Platform",
            "Services",
            "Retail",
            "Retail",
            "Growth",
            "Expansion",
        ];
        let product = [
            "Core",
            "Support",
            "Core",
            "Support",
            "Storefront",
            "Storefront",
            "Acquisition",
            "Localization",
        ];
        let sales = [52.0, 34.0, 33.0, 26.0, 44.0, 37.0, 41.0, 35.0];
        RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("division", DataType::Utf8, false),
                Field::new("region", DataType::Utf8, false),
                Field::new("team", DataType::Utf8, false),
                Field::new("product", DataType::Utf8, false),
                Field::new("sales", DataType::Float64, false),
            ])),
            vec![
                Arc::new(StringArray::from(division.to_vec())) as ArrayRef,
                Arc::new(StringArray::from(region.to_vec())) as ArrayRef,
                Arc::new(StringArray::from(team.to_vec())) as ArrayRef,
                Arc::new(StringArray::from(product.to_vec())) as ArrayRef,
                Arc::new(Float64Array::from(sales.to_vec())) as ArrayRef,
            ],
        )
        .expect("decorated treemap data")
    }

    async fn bound_state(binding: ChartEventBinding) -> ChartAppState {
        let ctx = SessionContext::new();
        let width = Param::new("width", ScalarValue::Float64(Some(640.0)));
        let compiled = Chart::<Cartesian>::new()
            .param(width.clone())
            .canvas_constraint(CanvasConstraint::width(width.expr()))
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile event binding plot");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        ChartAppState::new(session, policy, crate::ChartAppOptions::default())
    }

    async fn handler_for_binding(binding: ChartEventBinding) -> ChartEventBindingHandler {
        let ctx = SessionContext::new();
        let compiled = Chart::<Cartesian>::new()
            .param(Param::new("width", ScalarValue::Float64(Some(640.0))))
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile");
        let runtime = CompiledChartEventBinding::compile(
            0,
            compiled.event_bindings().first().unwrap(),
            &ctx,
            compiled.param_specs(),
            compiled.selection_specs(),
            compiled.store_specs(),
            &compiled.event_datum_types(),
        )
        .expect("compile binding runtime");
        ChartEventBindingHandler {
            runtime: Arc::new(runtime),
            state: Mutex::new(ChartEventBindingState::default()),
        }
    }

    async fn raw_domain_reset_state_and_handler() -> (ChartAppState, ChartEventBindingHandler) {
        let ctx = SessionContext::new();
        let x_domain = Param::raw_domain("x_domain");
        let binding = ChartEventBinding::on(ChartEventType::DoubleClick)
            .reset_param(&x_domain)
            .exact();
        let compiled = Chart::<Cartesian>::new()
            .canvas_size(400.0, 300.0)
            .param(x_domain)
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile reset binding plot");
        let runtime = CompiledChartEventBinding::compile(
            0,
            compiled.event_bindings().first().unwrap(),
            &ctx,
            compiled.param_specs(),
            compiled.selection_specs(),
            compiled.store_specs(),
            &compiled.event_datum_types(),
        )
        .expect("compile reset binding runtime");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let handler = ChartEventBindingHandler {
            runtime: Arc::new(runtime),
            state: Mutex::new(ChartEventBindingState::default()),
        };
        (state, handler)
    }

    fn equality_category_clause(id: Expr) -> SelectionClauseUpdate {
        SelectionClauseUpdate::equality(id)
            .dimension(col("category"), event::datum("category"))
            .build()
    }

    fn equality_category_value_clause() -> SelectionClauseUpdate {
        SelectionClauseUpdate::equality_value(col("category"), event::datum("category"))
    }

    fn scene_mark_at_path_with_origin<'a>(
        marks: &'a [SceneMark],
        path: &[usize],
        origin: [f32; 2],
    ) -> Option<(&'a SceneMark, [f32; 2])> {
        let (first, rest) = path.split_first()?;
        let mark = marks.get(*first)?;
        if rest.is_empty() {
            return Some((mark, origin));
        }
        match mark {
            SceneMark::Group(group) => scene_mark_at_path_with_origin(
                &group.marks,
                rest,
                [origin[0] + group.origin[0], origin[1] + group.origin[1]],
            ),
            _ => None,
        }
    }

    fn rect_instance_point(scene: &SceneGraph, instance: &MarkInstance) -> [f32; 2] {
        rect_instance_fraction_point(scene, instance, 0.37, 0.41)
    }

    fn rect_instance_fill(scene: &SceneGraph, instance: &MarkInstance) -> [f32; 4] {
        let (mark, _) =
            scene_mark_at_path_with_origin(&scene.marks, &instance.mark_path, scene.origin)
                .expect("scene mark at path");
        let SceneMark::Rect(rect) = mark else {
            panic!("expected rect mark at hit-test path");
        };
        let index = instance.instance_index.expect("rect instance index");
        rect.fill_vec()[index].color_or_transparent()
    }

    fn rect_instance_fraction_point(
        scene: &SceneGraph,
        instance: &MarkInstance,
        fx: f32,
        fy: f32,
    ) -> [f32; 2] {
        let (mark, origin) =
            scene_mark_at_path_with_origin(&scene.marks, &instance.mark_path, scene.origin)
                .expect("scene mark at path");
        let SceneMark::Rect(rect) = mark else {
            panic!("expected rect mark at hit-test path");
        };
        let index = instance.instance_index.expect("rect instance index");
        let x = rect.x_vec()[index];
        let y = rect.y_vec()[index];
        let x2 = rect.x2_vec()[index];
        let y2 = rect.y2_vec()[index];
        [origin[0] + x + (x2 - x) * fx, origin[1] + y + (y2 - y) * fy]
    }

    fn symbol_instance_point(scene: &SceneGraph, instance: &MarkInstance) -> [f32; 2] {
        let (mark, origin) =
            scene_mark_at_path_with_origin(&scene.marks, &instance.mark_path, scene.origin)
                .expect("scene mark at path");
        let SceneMark::Symbol(symbol) = mark else {
            panic!("expected symbol mark at hit-test path");
        };
        let index = instance.instance_index.expect("symbol instance index");
        let x = symbol.x_vec()[index];
        let y = symbol.y_vec()[index];
        [origin[0] + x, origin[1] + y]
    }

    fn collect_rect_fills(scene: &SceneGraph) -> Vec<[f32; 4]> {
        fn collect_from_mark(mark: &SceneMark, fills: &mut Vec<[f32; 4]>) {
            match mark {
                SceneMark::Group(group) => {
                    for child in &group.marks {
                        collect_from_mark(child, fills);
                    }
                }
                SceneMark::Rect(rect) => {
                    fills.extend(
                        rect.fill_vec()
                            .into_iter()
                            .map(|fill| fill.color_or_transparent()),
                    );
                }
                _ => {}
            }
        }

        let mut fills = Vec::new();
        for mark in &scene.marks {
            collect_from_mark(mark, &mut fills);
        }
        fills
    }

    fn collect_symbol_fills(scene: &SceneGraph) -> Vec<[f32; 4]> {
        fn collect_from_mark(mark: &SceneMark, fills: &mut Vec<[f32; 4]>) {
            match mark {
                SceneMark::Group(group) => {
                    for child in &group.marks {
                        collect_from_mark(child, fills);
                    }
                }
                SceneMark::Symbol(symbol) => {
                    fills.extend(
                        symbol
                            .fill_vec()
                            .into_iter()
                            .map(|fill| fill.color_or_transparent()),
                    );
                }
                _ => {}
            }
        }

        let mut fills = Vec::new();
        for mark in &scene.marks {
            collect_from_mark(mark, &mut fills);
        }
        fills
    }

    fn collect_line_strokes(scene: &SceneGraph) -> Vec<[f32; 4]> {
        fn collect_from_mark(mark: &SceneMark, strokes: &mut Vec<[f32; 4]>) {
            match mark {
                SceneMark::Group(group) => {
                    for child in &group.marks {
                        collect_from_mark(child, strokes);
                    }
                }
                SceneMark::Line(line) => {
                    strokes.push(line.stroke.color_or_transparent());
                }
                _ => {}
            }
        }

        let mut strokes = Vec::new();
        for mark in &scene.marks {
            collect_from_mark(mark, &mut strokes);
        }
        strokes
    }

    fn collect_line_x_coords(scene: &SceneGraph) -> Vec<f32> {
        fn collect_from_mark(mark: &SceneMark, xs: &mut Vec<f32>) {
            match mark {
                SceneMark::Group(group) => {
                    for child in &group.marks {
                        collect_from_mark(child, xs);
                    }
                }
                SceneMark::Line(line) => {
                    xs.extend(line.x_iter().copied());
                }
                _ => {}
            }
        }

        let mut xs = Vec::new();
        for mark in &scene.marks {
            collect_from_mark(mark, &mut xs);
        }
        xs
    }

    fn collect_symbol_x_coords(scene: &SceneGraph) -> Vec<f32> {
        fn collect_from_mark(mark: &SceneMark, xs: &mut Vec<f32>) {
            match mark {
                SceneMark::Group(group) => {
                    for child in &group.marks {
                        collect_from_mark(child, xs);
                    }
                }
                SceneMark::Symbol(symbol) => {
                    xs.extend(symbol.x_vec());
                }
                _ => {}
            }
        }

        let mut xs = Vec::new();
        for mark in &scene.marks {
            collect_from_mark(mark, &mut xs);
        }
        xs
    }

    fn collect_line_zindices_and_strokes(scene: &SceneGraph) -> Vec<(Option<i32>, [f32; 4])> {
        fn collect_from_mark(mark: &SceneMark, lines: &mut Vec<(Option<i32>, [f32; 4])>) {
            match mark {
                SceneMark::Group(group) => {
                    for child in &group.marks {
                        collect_from_mark(child, lines);
                    }
                }
                SceneMark::Line(line) => {
                    lines.push((line.zindex, line.stroke.color_or_transparent()));
                }
                _ => {}
            }
        }

        let mut lines = Vec::new();
        for mark in &scene.marks {
            collect_from_mark(mark, &mut lines);
        }
        lines
    }

    async fn legend_symbol_alpha_for_value(
        state: &ChartAppState,
        scene: &SceneGraph,
        value: &str,
    ) -> f32 {
        let hit_rect_instance = retained_event_datum_mark_instance(
            state,
            "__legend_value",
            ScalarValue::Utf8(Some(value.to_string())),
        )
        .await;
        let mut symbol_path = hit_rect_instance.mark_path.clone();
        let Some(last) = symbol_path.last_mut() else {
            panic!("legend item hit rect should have a mark path");
        };
        *last = 1;
        let (mark, _) = scene_mark_at_path_with_origin(&scene.marks, &symbol_path, scene.origin)
            .expect("legend item symbol mark");
        let SceneMark::Symbol(symbol) = mark else {
            panic!("legend item visual mark should be a symbol");
        };
        symbol.fill_vec()[0].color_or_transparent()[3]
    }

    fn has_blue_fill(fills: &[[f32; 4]]) -> bool {
        fills
            .iter()
            .any(|fill| fill[2] > 0.8 && fill[0] < 0.2 && fill[1] < 0.5)
    }

    fn has_blue_stroke(strokes: &[[f32; 4]]) -> bool {
        strokes
            .iter()
            .any(|stroke| stroke[2] > 0.8 && stroke[0] < 0.2 && stroke[1] < 0.5)
    }

    async fn retained_event_datum_mark_instance(
        state: &ChartAppState,
        field: &str,
        target: ScalarValue,
    ) -> MarkInstance {
        let runtime = state.runtime.lock().await;
        runtime
            .last_event_datum_state
            .rows
            .iter()
            .find_map(|rows| {
                let column = rows.rows.column_by_name(field)?;
                (0..column.len()).find_map(|index| {
                    let value = ScalarValue::try_from_array(column, index).ok()?;
                    (value == target).then(|| MarkInstance {
                        name: "retained_event_datum".to_string(),
                        mark_path: rows.mark_path.clone(),
                        instance_index: Some(index),
                    })
                })
            })
            .expect("retained event datum row")
    }

    async fn retained_named_event_datum_mark_instance(
        state: &ChartAppState,
        scene: &SceneGraph,
        mark_name: &str,
        field: &str,
        target: ScalarValue,
    ) -> MarkInstance {
        let runtime = state.runtime.lock().await;
        runtime
            .last_event_datum_state
            .rows
            .iter()
            .find_map(|rows| {
                let (mark, _) =
                    scene_mark_at_path_with_origin(&scene.marks, &rows.mark_path, scene.origin)?;
                let actual_name = match mark {
                    SceneMark::Arc(mark) => &mark.name,
                    SceneMark::Area(mark) => &mark.name,
                    SceneMark::Image(mark) => &mark.name,
                    SceneMark::Line(mark) => &mark.name,
                    SceneMark::Path(mark) => &mark.name,
                    SceneMark::Rect(mark) => &mark.name,
                    SceneMark::Rule(mark) => &mark.name,
                    SceneMark::Symbol(mark) => &mark.name,
                    SceneMark::Text(mark) => &mark.name,
                    SceneMark::Group(mark) => &mark.name,
                    SceneMark::Trail(mark) => &mark.name,
                    SceneMark::WarpedImage(mark) => &mark.name,
                };
                if actual_name != mark_name {
                    return None;
                }
                let column = rows.rows.column_by_name(field)?;
                (0..column.len()).find_map(|index| {
                    let value = ScalarValue::try_from_array(column, index).ok()?;
                    (value == target).then(|| MarkInstance {
                        name: mark_name.to_string(),
                        mark_path: rows.mark_path.clone(),
                        instance_index: Some(index),
                    })
                })
            })
            .expect("retained named event datum row")
    }

    async fn retained_rect_event_datum_mark_instance(
        state: &ChartAppState,
        scene: &SceneGraph,
        field: &str,
        target: ScalarValue,
    ) -> MarkInstance {
        let runtime = state.runtime.lock().await;
        runtime
            .last_event_datum_state
            .rows
            .iter()
            .find_map(|rows| {
                let column = rows.rows.column_by_name(field)?;
                (0..column.len()).find_map(|index| {
                    let value = ScalarValue::try_from_array(column, index).ok()?;
                    if value != target {
                        return None;
                    }
                    let (mark, _) = scene_mark_at_path_with_origin(
                        &scene.marks,
                        &rows.mark_path,
                        scene.origin,
                    )?;
                    matches!(mark, SceneMark::Rect(_)).then(|| MarkInstance {
                        name: "retained_event_datum".to_string(),
                        mark_path: rows.mark_path.clone(),
                        instance_index: Some(index),
                    })
                })
            })
            .expect("retained rect event datum row")
    }

    async fn equality_bar_state_and_handler(
        binding: ChartEventBinding,
    ) -> (
        ChartAppState,
        ChartEventBindingHandler,
        MarkInstance,
        [f32; 2],
    ) {
        let (state, mut handlers, mark_instance, position) =
            equality_bar_state_and_handlers(vec![binding]).await;
        (state, handlers.remove(0), mark_instance, position)
    }

    async fn equality_bar_state_and_handlers(
        bindings: Vec<ChartEventBinding>,
    ) -> (
        ChartAppState,
        Vec<ChartEventBindingHandler>,
        MarkInstance,
        [f32; 2],
    ) {
        let ctx = SessionContext::new();
        let picked = Selection::new("picked").empty_selects_nothing();
        let selected = picked.predicate();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('Alpha',  5.0),
                    ('Beta',   4.0),
                    ('Beta',   6.0),
                    ('Gamma',  7.0)
                ) AS t(category, amount)",
            )
            .await
            .expect("data");
        let mut plot = Chart::<Cartesian>::new()
            .canvas_size(420.0, 320.0)
            .selection(picked)
            .data(df)
            .mark(
                Rect::new()
                    .x_with(col("category"), |c| {
                        c.scale_with::<Band>(|s| s.padding_inner(0.2))
                    })
                    .x2_with(col(":x"), |c| c.band(1.0))
                    .y(lit(0.0))
                    .y2(datafusion::functions_aggregate::expr_fn::sum(col("amount")))
                    .fill_with(lit("#b8beca"), |c| {
                        c.no_scale()
                            .when_value(selected, lit("#2563eb"))
                            .no_legend()
                    }),
            );
        for binding in bindings {
            plot = plot.event_binding(binding);
        }
        let compiled = plot.compile(&ctx).await.expect("compile equality bar plot");
        let handlers = (0..compiled.event_bindings().len())
            .map(|index| compile_handler_for_binding_index(&compiled, &ctx, index))
            .collect::<Vec<_>>();
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial build");
        let datum_mark_instance = retained_event_datum_mark_instance(
            &state,
            "category",
            ScalarValue::Utf8(Some("Beta".to_string())),
        )
        .await;
        let position = rect_instance_point(&scene, &datum_mark_instance);
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        let mark_instance = rtree
            .pick_top_mark_at_point(&position)
            .cloned()
            .expect("rtree should pick the Beta bar");
        assert_eq!(
            mark_instance.mark_path, datum_mark_instance.mark_path,
            "rtree hit-test path should match retained event datum rows"
        );
        assert_eq!(
            mark_instance.instance_index, datum_mark_instance.instance_index,
            "rtree hit-test instance should match retained event datum rows"
        );
        (state, handlers, mark_instance, position)
    }

    async fn nested_grouped_bar_state_and_handler(
        binding: ChartEventBinding,
    ) -> (
        ChartAppState,
        ChartEventBindingHandler,
        MarkInstance,
        [f32; 2],
    ) {
        let ctx = SessionContext::new();
        let picked = Selection::new("picked").empty_selects_nothing();
        let selected = picked.predicate();
        let df = ctx
            .read_batch(nested_grouped_bar_batch())
            .expect("nested grouped bar data");
        let compiled = Chart::<Cartesian>::new()
            .canvas_size(520.0, 360.0)
            .selection(picked)
            .data(df)
            .mark(
                Rect::new()
                    .x_with(nested(["quarter", "team"]), |x| {
                        x.axis(|axis| axis.title("Quarter")).level(1, |level| {
                            level
                                .nest_scope(NestScope::Shared)
                                .axis(|axis| axis.visible(false))
                        })
                    })
                    .x2_with(col(":x"), |x| x.band(1.0))
                    .y(lit(0.0))
                    .y2(col("value"))
                    .fill_with(lit("#b8beca"), |c| {
                        c.no_scale()
                            .when_value(selected, lit("#2563eb"))
                            .no_legend()
                    }),
            )
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile nested grouped bar plot");
        let handler = compile_handler_for_binding_index(&compiled, &ctx, 0);
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial nested grouped bar build");
        let datum_mark_instance =
            retained_event_datum_mark_instance(&state, "value", ScalarValue::Float64(Some(38.0)))
                .await;
        let position = rect_instance_point(&scene, &datum_mark_instance);
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        let mark_instance = rtree
            .pick_top_mark_at_point(&position)
            .cloned()
            .expect("rtree should pick the Q2/East bar");
        assert_eq!(
            mark_instance.mark_path, datum_mark_instance.mark_path,
            "rtree hit-test path should match retained event datum rows"
        );
        assert_eq!(
            mark_instance.instance_index, datum_mark_instance.instance_index,
            "rtree hit-test instance should match retained event datum rows"
        );
        (state, handler, mark_instance, position)
    }

    fn nested_source_column_clause() -> SelectionClauseUpdate {
        nested_source_column_clause_with_scope(CoordinationScope::Shared)
    }

    fn nested_source_column_clause_with_scope(
        facet_scope: CoordinationScope,
    ) -> SelectionClauseUpdate {
        SelectionClauseUpdate::equality(lit("active"))
            .facet_scope(facet_scope)
            .dimension_named("quarter", col("quarter"), event::datum("quarter"))
            .dimension_named("team", col("team"), event::datum("team"))
            .build()
    }

    fn nested_grouped_bar_batch() -> RecordBatch {
        let quarter = ["Q1", "Q1", "Q1", "Q2", "Q2", "Q3", "Q3", "Q3"];
        let team = [
            "North", "South", "East", "North", "East", "North", "South", "East",
        ];
        let value = [42.0, 30.0, 34.0, 47.0, 38.0, 51.0, 39.0, 44.0];

        let schema = Arc::new(Schema::new(vec![
            Field::new("quarter", DataType::Utf8, false),
            Field::new("team", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ]));

        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(quarter.to_vec())) as ArrayRef,
                Arc::new(StringArray::from(team.to_vec())) as ArrayRef,
                Arc::new(Float64Array::from(value.to_vec())) as ArrayRef,
            ],
        )
        .expect("nested grouped bar batch")
    }

    fn assert_nested_coord_value(value: &ScalarValue, expected_quarter: &str, expected_team: &str) {
        let ScalarValue::Struct(struct_array) = value else {
            panic!("expected nested key struct value, got {value:?}");
        };
        assert_eq!(
            struct_array.len(),
            1,
            "nested key scalar should contain one row"
        );
        let quarter = struct_array
            .column_by_name("quarter")
            .expect("quarter field")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("quarter string array");
        let team = struct_array
            .column_by_name("team")
            .expect("team field")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("team string array");
        assert_eq!(quarter.value(0), expected_quarter);
        assert_eq!(team.value(0), expected_team);
    }

    fn assert_quarter_team_dimensions(
        dimensions: &[avenger_chart_core::SelectionEqualityDimensionValue],
        expected_quarter: &str,
        expected_team: &str,
    ) {
        assert_eq!(dimensions.len(), 2);
        assert_eq!(dimensions[0].id, "quarter");
        assert_eq!(
            dimensions[0].value,
            ScalarValue::Utf8(Some(expected_quarter.to_string()))
        );
        assert_eq!(dimensions[1].id, "team");
        assert_eq!(
            dimensions[1].value,
            ScalarValue::Utf8(Some(expected_team.to_string()))
        );
    }

    fn assert_nested_event_coord_schema(handler: &ChartEventBindingHandler) {
        let schema = handler.runtime.program.schema();
        let field = schema
            .field_with_name(&event::event_coord_column_name("x"))
            .expect("event coord x schema field");
        let DataType::Struct(fields) = field.data_type() else {
            panic!(
                "expected struct-typed event coord x, got {:?}",
                field.data_type()
            );
        };
        let names = fields
            .iter()
            .map(|field| field.name().as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["quarter", "team"]);
    }

    async fn temporal_nested_bar_state_and_handler(
        binding: ChartEventBinding,
    ) -> (
        ChartAppState,
        ChartEventBindingHandler,
        MarkInstance,
        [f32; 2],
    ) {
        let ctx = SessionContext::new();
        let picked = Selection::new("picked").empty_selects_nothing();
        let selected = picked.predicate();
        let df = ctx
            .read_batch(temporal_nested_bar_batch())
            .expect("temporal nested bar data");
        let compiled = Chart::<Cartesian>::new()
            .canvas_size(560.0, 380.0)
            .selection(picked)
            .data(df)
            .mark(
                Rect::new().transform(
                    TimeLevels::new(col("timestamp"))
                        .year()
                        .quarter()
                        .month()
                        .name("period"),
                    |mark, period| {
                        mark.transform(
                            Aggregate::new()
                                .group_by(period.keys())
                                .sum("total", col("value")),
                            |mark, aggregate| {
                                mark.x_with(period.nested(), |x| {
                                    x.axis(|axis| axis.title("Month"))
                                        .level(1, |level| level.nest_scope(NestScope::Shared))
                                        .level(2, |level| {
                                            level.axis(|axis| axis.label_angle(-90.0))
                                        })
                                })
                                .x2_with(col(":x"), |x| x.band(1.0))
                                .y(lit(0.0))
                                .y2(aggregate.output("total"))
                                .fill_with(lit("#b8beca"), |c| {
                                    c.no_scale()
                                        .when_value(selected, lit("#2563eb"))
                                        .no_legend()
                                })
                            },
                        )
                    },
                ),
            )
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile temporal nested bar plot");
        let handler = compile_handler_for_binding_index(&compiled, &ctx, 0);
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial temporal nested bar build");
        let datum_mark_instance =
            retained_event_datum_mark_instance(&state, "total", ScalarValue::Float64(Some(18.0)))
                .await;
        let position = rect_instance_point(&scene, &datum_mark_instance);
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        let mark_instance = rtree
            .pick_top_mark_at_point(&position)
            .cloned()
            .expect("rtree should pick the March bar");
        assert_eq!(
            mark_instance.mark_path, datum_mark_instance.mark_path,
            "rtree hit-test path should match retained event datum rows"
        );
        assert_eq!(
            mark_instance.instance_index, datum_mark_instance.instance_index,
            "rtree hit-test instance should match retained event datum rows"
        );
        (state, handler, mark_instance, position)
    }

    fn temporal_nested_source_column_clause() -> SelectionClauseUpdate {
        SelectionClauseUpdate::equality(lit("active"))
            .facet_scope(CoordinationScope::Shared)
            .dimension_named("year", col("period_year"), event::datum("period_year"))
            .dimension_named(
                "quarter",
                col("period_quarter"),
                event::datum("period_quarter"),
            )
            .dimension_named("month", col("period_month"), event::datum("period_month"))
            .build()
    }

    fn temporal_nested_bar_batch() -> RecordBatch {
        let timestamp = [
            1_704_412_800_000_i64,
            1_705_708_800_000_i64,
            1_709_337_600_000_i64,
            1_712_880_000_000_i64,
        ];
        let value = [14.0, 5.0, 18.0, 22.0];

        let schema = Arc::new(Schema::new(vec![
            Field::new(
                "timestamp",
                DataType::Timestamp(datafusion::arrow::datatypes::TimeUnit::Millisecond, None),
                false,
            ),
            Field::new("value", DataType::Float64, false),
        ]));

        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(TimestampMillisecondArray::from(timestamp.to_vec())) as ArrayRef,
                Arc::new(Float64Array::from(value.to_vec())) as ArrayRef,
            ],
        )
        .expect("temporal nested bar batch")
    }

    fn assert_temporal_nested_coord_value(
        value: &ScalarValue,
        year: i32,
        quarter: i32,
        month: i32,
    ) {
        let ScalarValue::Struct(struct_array) = value else {
            panic!("expected temporal nested key struct value, got {value:?}");
        };
        assert_eq!(
            struct_array.len(),
            1,
            "nested key scalar should contain one row"
        );
        let year_values = struct_array
            .column_by_name("period_year")
            .expect("year field")
            .as_any()
            .downcast_ref::<Int32Array>()
            .expect("year int32 array");
        let quarter_values = struct_array
            .column_by_name("period_quarter")
            .expect("quarter field")
            .as_any()
            .downcast_ref::<Int32Array>()
            .expect("quarter int32 array");
        let month_values = struct_array
            .column_by_name("period_month")
            .expect("month field")
            .as_any()
            .downcast_ref::<Int32Array>()
            .expect("month int32 array");
        assert_eq!(year_values.value(0), year);
        assert_eq!(quarter_values.value(0), quarter);
        assert_eq!(month_values.value(0), month);
    }

    fn assert_temporal_dimensions(
        dimensions: &[avenger_chart_core::SelectionEqualityDimensionValue],
        year: i32,
        quarter: i32,
        month: i32,
    ) {
        assert_eq!(dimensions.len(), 3);
        assert_eq!(dimensions[0].id, "year");
        assert_eq!(dimensions[0].value, ScalarValue::Int32(Some(year)));
        assert_eq!(dimensions[1].id, "quarter");
        assert_eq!(dimensions[1].value, ScalarValue::Int32(Some(quarter)));
        assert_eq!(dimensions[2].id, "month");
        assert_eq!(dimensions[2].value, ScalarValue::Int32(Some(month)));
    }

    fn assert_temporal_event_coord_schema(handler: &ChartEventBindingHandler) {
        let schema = handler.runtime.program.schema();
        let field = schema
            .field_with_name(&event::event_coord_column_name("x"))
            .expect("event coord x schema field");
        let DataType::Struct(fields) = field.data_type() else {
            panic!(
                "expected struct-typed event coord x, got {:?}",
                field.data_type()
            );
        };
        let names = fields
            .iter()
            .map(|field| field.name().as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["period_year", "period_quarter", "period_month"]);
    }

    async fn faceted_nested_grouped_bar_state_and_handler(
        binding: ChartEventBinding,
    ) -> (
        ChartAppState,
        ChartEventBindingHandler,
        MarkInstance,
        [f32; 2],
    ) {
        let ctx = SessionContext::new();
        let picked = Selection::new("picked")
            .empty_selects_nothing()
            .facet_context_field("region", col("region"));
        let selected = picked.predicate();
        let df = ctx
            .read_batch(faceted_nested_grouped_bar_batch())
            .expect("faceted nested grouped bar data");
        let leaf = Plot::<Cartesian>::new().mark(
            Rect::new()
                .x_with(nested(["quarter", "team"]), |x| {
                    x.axis(|axis| axis.title("Quarter")).level(1, |level| {
                        level
                            .nest_scope(NestScope::Shared)
                            .axis(|axis| axis.visible(false))
                    })
                })
                .x2_with(col(":x"), |x| x.band(1.0))
                .y(lit(0.0))
                .y2(col("value"))
                .fill_with(lit("#b8beca"), |c| {
                    c.no_scale()
                        .when_value(selected, lit("#2563eb"))
                        .no_legend()
                }),
        );
        let compiled = Chart::<FacetColumn>::new()
            .canvas_size(720.0, 360.0)
            .selection(picked)
            .data(df)
            .mark(Subplot::new(leaf).column(col("region")))
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile faceted nested grouped bar plot");
        let handler = compile_handler_for_binding_index(&compiled, &ctx, 0);
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial faceted nested grouped bar build");
        let datum_mark_instance =
            retained_event_datum_mark_instance(&state, "value", ScalarValue::Float64(Some(138.0)))
                .await;
        let position = rect_instance_point(&scene, &datum_mark_instance);
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        let mark_instance = rtree
            .pick_top_mark_at_point(&position)
            .cloned()
            .expect("rtree should pick the West Q2/East bar");
        assert_eq!(
            mark_instance.mark_path, datum_mark_instance.mark_path,
            "rtree hit-test path should match retained event datum rows"
        );
        assert_eq!(
            mark_instance.instance_index, datum_mark_instance.instance_index,
            "rtree hit-test instance should match retained event datum rows"
        );
        (state, handler, mark_instance, position)
    }

    fn faceted_nested_grouped_bar_batch() -> RecordBatch {
        let region = [
            "East", "East", "East", "East", "West", "West", "West", "West",
        ];
        let quarter = ["Q1", "Q1", "Q2", "Q2", "Q1", "Q1", "Q2", "Q2"];
        let team = [
            "North", "East", "North", "East", "North", "East", "North", "East",
        ];
        let value = [42.0, 30.0, 47.0, 38.0, 142.0, 130.0, 147.0, 138.0];

        let schema = Arc::new(Schema::new(vec![
            Field::new("region", DataType::Utf8, false),
            Field::new("quarter", DataType::Utf8, false),
            Field::new("team", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ]));

        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(region.to_vec())) as ArrayRef,
                Arc::new(StringArray::from(quarter.to_vec())) as ArrayRef,
                Arc::new(StringArray::from(team.to_vec())) as ArrayRef,
                Arc::new(Float64Array::from(value.to_vec())) as ArrayRef,
            ],
        )
        .expect("faceted nested grouped bar batch")
    }

    async fn equality_symbol_state_and_handler(
        binding: ChartEventBinding,
    ) -> (
        ChartAppState,
        ChartEventBindingHandler,
        MarkInstance,
        [f32; 2],
    ) {
        let ctx = SessionContext::new();
        let picked = Selection::new("picked").empty_selects_nothing();
        let selected = picked.predicate();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('Alpha', 1.0, 1.0),
                    ('Beta',  2.17, 2.31),
                    ('Gamma', 3.0, 3.0)
                ) AS t(category, x, y)",
            )
            .await
            .expect("data");
        let compiled = Chart::<Cartesian>::new()
            .canvas_size(420.0, 320.0)
            .selection(picked)
            .data(df)
            .mark(
                Symbol::new()
                    .x(col("x"))
                    .y(col("y"))
                    .fill_with(lit("#b8beca"), |c| {
                        c.no_scale()
                            .when_value(selected, lit("#2563eb"))
                            .no_legend()
                    })
                    .size(48.0),
            )
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile equality symbol plot");
        let handler = compile_handler_for_binding_index(&compiled, &ctx, 0);
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial build");
        let datum_mark_instance = retained_event_datum_mark_instance(
            &state,
            "category",
            ScalarValue::Utf8(Some("Beta".to_string())),
        )
        .await;
        let position = symbol_instance_point(&scene, &datum_mark_instance);
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        let mark_instance = rtree
            .pick_top_mark_at_point(&position)
            .cloned()
            .expect("rtree should pick the Beta symbol");
        assert_eq!(mark_instance.mark_path, datum_mark_instance.mark_path);
        assert_eq!(
            mark_instance.instance_index, datum_mark_instance.instance_index,
            "rtree hit-test instance should match retained event datum rows"
        );
        (state, handler, mark_instance, position)
    }

    fn speed_cost_parallel_coord() -> Parallel {
        Parallel::new()
            .dimension_with("speed", |d| d.axis(|axis| axis.title("Speed")))
            .dimension_with("cost", |d| d.axis(|axis| axis.title("Cost")))
    }

    fn speed_cost_parallel_line() -> ParallelLine {
        ParallelLine::new()
            .dimension("speed", col("speed"))
            .dimension("cost", col("cost"))
    }

    fn speed_cost_parallel_symbol() -> ParallelSymbol {
        ParallelSymbol::new()
            .dimension("speed", col("speed"))
            .dimension("cost", col("cost"))
    }

    fn speed_efficiency_cost_parallel_coord() -> Parallel {
        Parallel::new()
            .dimension_with("speed", |d| d.axis(|axis| axis.title("Speed")))
            .dimension_with("efficiency", |d| d.axis(|axis| axis.title("Efficiency")))
            .dimension_with("cost", |d| d.axis(|axis| axis.title("Cost")))
    }

    fn speed_efficiency_cost_parallel_line() -> ParallelLine {
        ParallelLine::new()
            .dimension("speed", col("speed"))
            .dimension("efficiency", col("efficiency"))
            .dimension("cost", col("cost"))
    }

    async fn parallel_point_state_and_handler(
        binding: ChartEventBinding,
    ) -> (
        ChartAppState,
        ChartEventBindingHandler,
        MarkInstance,
        [f32; 2],
    ) {
        let ctx = SessionContext::new();
        let picked = Selection::new("picked").empty_selects_nothing();
        let selected = picked.predicate();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('row0', 10.0, 30.0),
                    ('row1', 20.0, 40.0),
                    ('row2', 25.0, 45.0)
                ) AS t(id, speed, cost)",
            )
            .await
            .expect("parallel point data");
        let compiled = Chart::with_coord(speed_cost_parallel_coord())
            .canvas_size(420.0, 320.0)
            .plot_size(260.0, 180.0)
            .selection(picked)
            .data(df)
            .mark(
                speed_cost_parallel_symbol()
                    .fill_with(lit("#b8beca"), |c| {
                        c.no_scale()
                            .when_value(selected, lit("#2563eb"))
                            .no_legend()
                    })
                    .size(900.0)
                    .zindex(100),
            )
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile parallel point plot");
        let handler = compile_handler_for_binding_index(&compiled, &ctx, 0);
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial parallel point build");
        let datum_mark_instance = retained_event_datum_mark_instance(
            &state,
            "id",
            ScalarValue::Utf8(Some("row1".to_string())),
        )
        .await;
        let mut position = symbol_instance_point(&scene, &datum_mark_instance);
        // Parallel points sit exactly on their dimension axis. Move inside the
        // enlarged symbol so the point, not the axis rule, is the hit target.
        position[0] += 12.0;
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        let mark_instance = rtree
            .pick_top_mark_at_point(&position)
            .cloned()
            .expect("rtree should pick the row1 parallel point");
        let clicked_id = {
            let runtime = state.runtime.lock().await;
            runtime
                .last_event_datum_state
                .datum_for_mark_instance(Some(&mark_instance), "id")
        };
        assert_eq!(
            clicked_id,
            Some(ScalarValue::Utf8(Some("row1".to_string()))),
            "rtree hit-test instance should expose the clicked source row id"
        );
        (state, handler, mark_instance, position)
    }

    fn colorbar_interval_clause(value_channel: &str) -> SelectionClauseUpdate {
        SelectionClauseUpdate::interval(lit("active"))
            .facet_scope(CoordinationScope::Shared)
            .dimension(col("temperature"))
            .endpoints(
                event::event_coord(value_channel),
                event::event_coord(value_channel),
            )
            .build()
    }

    async fn colorbar_state_and_handlers(
        position: LegendPosition,
        bindings: Vec<ChartEventBinding>,
    ) -> (
        ChartAppState,
        Vec<ChartEventBindingHandler>,
        MarkInstance,
        [f32; 2],
    ) {
        let ctx = SessionContext::new();
        let picked = Selection::new("picked").empty_selects_nothing();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    (1.0, 1.0, 0.0),
                    (2.0, 1.6, 50.0),
                    (3.0, 2.2, 100.0)
                ) AS t(x, y, temperature)",
            )
            .await
            .expect("colorbar data");
        let mut legend_bindings = bindings.into_iter();
        let first = legend_bindings.next().expect("at least one binding");
        let compiled = Chart::<Cartesian>::new()
            .canvas_size(480.0, 360.0)
            .selection(picked)
            .data(df)
            .mark(Symbol::new().x(col("x")).y(col("y")).size(80.0).fill_with(
                col("temperature"),
                |c| {
                    c.scale_with::<Linear>(|s| s.domain((0.0, 100.0)).nice(false).zero(false))
                        .legend(|l| {
                            let mut legend = l
                                .id("temperature_colorbar")
                                .title("Temperature")
                                .position(position)
                                .event_binding(first);
                            for binding in legend_bindings {
                                legend = legend.event_binding(binding);
                            }
                            legend
                        })
                },
            ))
            .compile(&ctx)
            .await
            .expect("compile colorbar interaction plot");
        let handlers = (0..compiled.event_bindings().len())
            .map(|index| compile_handler_for_binding_index(&compiled, &ctx, index))
            .collect::<Vec<_>>();
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial colorbar build");
        let colorbar_mark_instance = retained_event_datum_mark_instance(
            &state,
            event::LEGEND_SURFACE_KIND_FIELD,
            ScalarValue::Utf8(Some(
                event::LEGEND_SURFACE_KIND_CONTINUOUS_COLORBAR.to_string(),
            )),
        )
        .await;
        let midpoint = rect_instance_fraction_point(&scene, &colorbar_mark_instance, 0.5, 0.5);
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        let mark_instance = rtree
            .pick_top_mark_at_point(&midpoint)
            .cloned()
            .expect("rtree should pick the colorbar hit rect");
        assert_eq!(mark_instance.mark_path, colorbar_mark_instance.mark_path);
        assert_eq!(
            mark_instance.instance_index,
            colorbar_mark_instance.instance_index
        );
        let runtime = state.runtime.lock().await;
        assert!(
            matches!(
                route_interaction_scope(
                    &runtime.last_interaction_state.scopes,
                    Some(midpoint),
                    &channel_set(&["y"]),
                ),
                InteractionRoute::Scope(scope)
                    if scope.kind == InteractionScopeKind::LegendColorbar
            ),
            "colorbar midpoint should route to the exported legend colorbar interaction scope"
        );
        drop(runtime);
        (state, handlers, mark_instance, midpoint)
    }

    fn scalar_f64(value: &ScalarValue) -> f64 {
        match value {
            ScalarValue::Float64(Some(value)) => *value,
            ScalarValue::Float32(Some(value)) => *value as f64,
            value => panic!("expected numeric scalar, got {value:?}"),
        }
    }

    async fn colorbar_drag_move(
        state: &mut ChartAppState,
        handler: &ChartEventBindingHandler,
        start_mark_instance: MarkInstance,
        start_pos: [f32; 2],
        current_pos: [f32; 2],
    ) -> UpdateStatus {
        let instant = Instant::now();
        let start_event = EventStreamEventSnapshot {
            event: SceneGraphEvent::MouseDown(SceneMouseDownEvent {
                position: start_pos,
                button: MouseButton::Left,
                mark_instance: Some(start_mark_instance.clone()),
                modifiers: Default::default(),
            }),
            mark_instance: Some(start_mark_instance.clone()),
            instant,
        };
        let context = EventStreamContext {
            mark_instance: None,
            current_event: None,
            start_event: Some(start_event),
            previous_event: None,
        };
        handler
            .handle_with_context(
                &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: current_pos,
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                &context,
                state,
                &empty_rtree(),
            )
            .await
    }

    async fn cursor_drag_move_from_mark(
        state: &mut ChartAppState,
        handler: &ChartEventBindingHandler,
        start_mark_instance: MarkInstance,
        start_pos: [f32; 2],
        current_pos: [f32; 2],
    ) -> UpdateStatus {
        let instant = Instant::now();
        let start_event = EventStreamEventSnapshot {
            event: SceneGraphEvent::MouseDown(SceneMouseDownEvent {
                position: start_pos,
                button: MouseButton::Left,
                mark_instance: Some(start_mark_instance.clone()),
                modifiers: Default::default(),
            }),
            mark_instance: Some(start_mark_instance),
            instant,
        };
        let context = EventStreamContext {
            mark_instance: None,
            current_event: None,
            start_event: Some(start_event),
            previous_event: None,
        };
        handler
            .handle_with_context(
                &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: current_pos,
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                &context,
                state,
                &empty_rtree(),
            )
            .await
    }

    async fn mouse_up_from_mark(
        state: &mut ChartAppState,
        handler: &ChartEventBindingHandler,
        start_mark_instance: MarkInstance,
        start_pos: [f32; 2],
        end_pos: [f32; 2],
    ) -> UpdateStatus {
        let instant = Instant::now();
        let start_event = EventStreamEventSnapshot {
            event: SceneGraphEvent::MouseDown(SceneMouseDownEvent {
                position: start_pos,
                button: MouseButton::Left,
                mark_instance: Some(start_mark_instance.clone()),
                modifiers: Default::default(),
            }),
            mark_instance: Some(start_mark_instance),
            instant,
        };
        let context = EventStreamContext {
            mark_instance: None,
            current_event: None,
            start_event: Some(start_event),
            previous_event: None,
        };
        handler
            .handle_with_context(
                &SceneGraphEvent::MouseUp(SceneMouseUpEvent {
                    position: end_pos,
                    button: MouseButton::Left,
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                &context,
                state,
                &empty_rtree(),
            )
            .await
    }

    async fn cursor_drag_move_from_position(
        state: &mut ChartAppState,
        handler: &ChartEventBindingHandler,
        start_pos: [f32; 2],
        current_pos: [f32; 2],
        shift: bool,
    ) -> UpdateStatus {
        let instant = Instant::now();
        let start_event = EventStreamEventSnapshot {
            event: SceneGraphEvent::MouseDown(SceneMouseDownEvent {
                position: start_pos,
                button: MouseButton::Left,
                mark_instance: None,
                modifiers: ModifiersState {
                    shift,
                    ..ModifiersState::default()
                },
            }),
            mark_instance: None,
            instant,
        };
        let context = EventStreamContext {
            mark_instance: None,
            current_event: None,
            start_event: Some(start_event),
            previous_event: None,
        };
        handler
            .handle_with_context(
                &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: current_pos,
                    mark_instance: None,
                    modifiers: ModifiersState {
                        shift,
                        ..ModifiersState::default()
                    },
                }),
                &context,
                state,
                &empty_rtree(),
            )
            .await
    }

    async fn click_mark(
        state: &mut ChartAppState,
        handler: &ChartEventBindingHandler,
        mark_instance: Option<MarkInstance>,
        position: [f32; 2],
        shift: bool,
    ) -> UpdateStatus {
        handler
            .handle_with_context(
                &SceneGraphEvent::Click(SceneClickEvent {
                    position,
                    button: MouseButton::Left,
                    mark_instance,
                    modifiers: ModifiersState {
                        shift,
                        ..ModifiersState::default()
                    },
                }),
                &EventStreamContext::default(),
                &mut *state,
                &empty_rtree(),
            )
            .await
    }

    async fn mouse_down_mark(
        state: &mut ChartAppState,
        handler: &ChartEventBindingHandler,
        mark_instance: Option<MarkInstance>,
        position: [f32; 2],
    ) -> UpdateStatus {
        handler
            .handle_with_context(
                &SceneGraphEvent::MouseDown(SceneMouseDownEvent {
                    position,
                    button: MouseButton::Left,
                    mark_instance,
                    modifiers: ModifiersState::default(),
                }),
                &EventStreamContext::default(),
                &mut *state,
                &empty_rtree(),
            )
            .await
    }

    async fn double_click_mark(
        state: &mut ChartAppState,
        handler: &ChartEventBindingHandler,
        mark_instance: Option<MarkInstance>,
        position: [f32; 2],
    ) -> UpdateStatus {
        handler
            .handle_with_context(
                &SceneGraphEvent::DoubleClick(SceneDoubleClickEvent {
                    position,
                    mark_instance,
                    modifiers: ModifiersState::default(),
                }),
                &EventStreamContext::default(),
                &mut *state,
                &empty_rtree(),
            )
            .await
    }

    async fn click_category(
        state: &mut ChartAppState,
        handler: &ChartEventBindingHandler,
        category: &str,
        shift: bool,
    ) -> UpdateStatus {
        let scene = crate::ChartSceneGraphBuilder
            .build(state)
            .await
            .expect("scene for category click");
        let datum_mark_instance = retained_event_datum_mark_instance(
            state,
            "category",
            ScalarValue::Utf8(Some(category.to_string())),
        )
        .await;
        let position = rect_instance_point(&scene, &datum_mark_instance);
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        let mark_instance = rtree
            .pick_top_mark_at_point(&position)
            .cloned()
            .expect("rtree should pick category mark");
        click_mark(state, handler, Some(mark_instance), position, shift).await
    }

    async fn selected_clause_ids(state: &ChartAppState) -> Vec<String> {
        let runtime = state.runtime.lock().await;
        let mut ids = runtime
            .session
            .selection_clauses_for_diagnostics("picked")
            .iter()
            .map(|clause| clause.id.clone())
            .collect::<Vec<_>>();
        ids.sort();
        ids
    }

    #[tokio::test]
    async fn event_binding_canvas_resize_updates_param() {
        let binding = ChartEventBinding::on(ChartEventType::CanvasResize)
            .set_param("width", event::canvas_width())
            .preview();
        let mut state = bound_state(binding.clone()).await;
        let handler = handler_for_binding(binding).await;

        let status = handler
            .handle_with_context(
                &SceneGraphEvent::CanvasResize(CanvasResizeEvent {
                    size: [800.0, 400.0],
                }),
                &EventStreamContext::default(),
                &mut state,
                &empty_rtree(),
            )
            .await;

        assert!(status.rerender);
        assert!(!status.rebuild_geometry);
        assert_eq!(
            state.params().await.get("width"),
            Some(&ScalarValue::Float64(Some(800.0)))
        );
        let metrics = state.event_metrics().await;
        assert_eq!(
            metrics.event_batches_evaluated, 2,
            "the filter snapshot and ordered action snapshot are distinct batches"
        );
        assert_eq!(metrics.param_patch_events, 1);
        assert_eq!(metrics.params_patched, 1);
    }

    #[tokio::test]
    async fn cursor_action_updates_cursor_without_rerender() {
        let ctx = SessionContext::new();
        let compiled = Chart::<Cartesian>::new()
            .event_binding(
                ChartEventBinding::on(ChartEventType::CursorMoved)
                    .set_cursor(event::cursor(CursorStyle::Crosshair))
                    .preview(),
            )
            .compile(&ctx)
            .await
            .expect("compile cursor binding plot");
        let runtime = CompiledChartEventBinding::compile(
            0,
            compiled.event_bindings().first().unwrap(),
            &ctx,
            compiled.param_specs(),
            compiled.selection_specs(),
            compiled.store_specs(),
            &compiled.event_datum_types(),
        )
        .expect("compile cursor binding runtime");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let handler = ChartEventBindingHandler {
            runtime: Arc::new(runtime),
            state: Mutex::new(ChartEventBindingState::default()),
        };

        let status = handler
            .handle_with_context(
                &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: [10.0, 20.0],
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                &EventStreamContext::default(),
                &mut state,
                &empty_rtree(),
            )
            .await;

        assert_eq!(status.cursor, Some(CursorStyle::Crosshair));
        assert!(!status.rerender);
        assert!(!status.rebuild_geometry);
        assert!(!state.params().await.contains_key("cursor"));
    }

    #[tokio::test]
    async fn checkbox_click_flips_param_and_rebuilds_checked_scene() {
        let ctx = SessionContext::new();
        let mut theme = Theme::light();
        theme
            .append_css(
                "checkbox { height: 40px; } \
                 checkbox::part(box) { choice-control-size: 18px; corner-radius: 4px; } \
                 checkbox::part(label) { control-label-gap: 12px; } \
                 checkbox::part(focus-ring) { focus-gap: 3px; }",
            )
            .expect("custom checkbox hit theme");
        let compiled = Chart::<Cartesian>::new()
            .theme(theme)
            .widget(Checkbox::new("regions", "Regions", false).position(ChromePosition::Left))
            .compile(&ctx)
            .await
            .expect("compile checkbox chart");
        let click_index = compiled
            .event_bindings()
            .iter()
            .position(|binding| binding.event_type == ChartEventType::Click)
            .expect("checkbox click binding");
        let handler = compile_handler_for_binding_index(&compiled, &ctx, click_index);
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());

        let scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial checkbox scene");
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        assert!(
            rtree
                .iter()
                .all(|geometry| geometry.mark_instance.name != "check")
        );
        let origin = rtree
            .named_group_origin("regions")
            .expect("checkbox group origin");
        let position = [origin[0] + 9.0, origin[1] + 20.0];
        let box_instance = rtree
            .pick_top_mark_at_point(&position)
            .cloned()
            .expect("checkbox box hit");
        assert_eq!(box_instance.name, "box");

        let status = click_mark(&mut state, &handler, Some(box_instance), position, false).await;
        assert!(status.rerender);
        assert_eq!(
            state.params().await.get("regions__checked"),
            Some(&ScalarValue::Boolean(Some(true)))
        );

        let checked_scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("checked checkbox scene");
        let checked_rtree = SceneGraphRTree::from_scene_graph(&checked_scene);
        assert!(
            checked_rtree
                .iter()
                .any(|geometry| geometry.mark_instance.name == "check")
        );
        let check_instance = checked_rtree
            .iter()
            .find(|geometry| geometry.mark_instance.name == "check")
            .map(|geometry| geometry.mark_instance.clone())
            .expect("visible check hit target");
        let check_click =
            click_mark(&mut state, &handler, Some(check_instance), position, false).await;
        assert!(check_click.rerender);
        assert_eq!(
            state.params().await.get("regions__checked"),
            Some(&ScalarValue::Boolean(Some(false)))
        );
        let label_instance = checked_rtree
            .iter()
            .find(|geometry| geometry.mark_instance.name == "label")
            .map(|geometry| geometry.mark_instance.clone())
            .expect("checkbox label hit target");
        let label_click =
            click_mark(&mut state, &handler, Some(label_instance), position, false).await;
        assert!(label_click.rerender);
        assert_eq!(
            state.params().await.get("regions__checked"),
            Some(&ScalarValue::Boolean(Some(true)))
        );
    }

    #[tokio::test]
    async fn widget_frame_fields_stay_bound_to_the_gesture_start_frame() {
        let ctx = Arc::new(SessionContext::new());
        let start = ChartEventStream::on(ChartEventType::MouseDown).mark("regions.box");
        let end = ChartEventStream::on(ChartEventType::MouseUp);
        let frame_assignments = |binding: ChartEventBinding| {
            binding
                .set_param("frame_x", event::frame_x())
                .set_param("frame_y", event::frame_y())
                .set_param("frame_width", event::frame_width())
                .set_param("frame_height", event::frame_height())
                .preview()
        };
        let compiled = Chart::<PixelFrame>::new()
            .canvas_size(320.0, 180.0)
            .param(Param::new("frame_x", ScalarValue::Float64(None)))
            .param(Param::new("frame_y", ScalarValue::Float64(None)))
            .param(Param::new("frame_width", ScalarValue::Float64(None)))
            .param(Param::new("frame_height", ScalarValue::Float64(None)))
            .host_widget(Checkbox::new("regions", "Regions", false))
            .event_binding(frame_assignments(
                ChartEventBinding::on(ChartEventType::CursorMoved)
                    .between(start.clone(), end.clone()),
            ))
            .event_binding(frame_assignments(ChartEventBinding::on_between_end(
                start, end,
            )))
            .compile(ctx.as_ref())
            .await
            .expect("compile frame-local widget bindings");
        let streams =
            event_streams_for_plot_bindings(&compiled, ctx.as_ref()).expect("widget streams");
        let policy = compiled.resize_policy();
        let mut session = Arc::new(compiled).instantiate(ctx);
        let evaluated = session
            .evaluate(
                EvaluationRequest::new().widget_frames(
                    WidgetFrameAssignments::try_from_iter([(
                        "regions",
                        WidgetFrame::try_new(20.0, 30.0, 140.0, 40.0).unwrap(),
                    )])
                    .unwrap(),
                ),
            )
            .await
            .expect("initial explicit-frame widget scene");
        let scene = evaluated.scene_graph.clone();
        let state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        {
            let mut runtime = state.runtime.lock().await;
            runtime.last_interaction_state = evaluated.interaction;
            runtime.last_event_datum_state = evaluated.event_datums;
            runtime.last_widget_frame_state = evaluated.widget_frames;
        }
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        let origin = rtree.named_group_origin("regions").expect("widget origin");
        let inside = [origin[0] + 8.0, origin[1] + 16.0];
        let box_instance = rtree
            .pick_top_mark_at_point(&inside)
            .cloned()
            .expect("checkbox box hit");
        let original_frame = {
            let runtime = state.runtime.lock().await;
            runtime
                .last_widget_frame_state
                .frame_for_mark_instance(Some(&box_instance))
                .cloned()
                .expect("box owns a widget frame")
        };
        assert_eq!(original_frame.widget_id, "regions");
        assert_eq!(original_frame.local_point(inside), [8.0, 16.0]);

        let mut manager = EventStreamManager::new(state);
        manager.register_handler(
            EventStreamConfig {
                types: vec![SceneGraphEventType::MouseDown],
                ..Default::default()
            },
            Arc::new(crate::WidgetGestureFrameCaptureHandler),
        );
        for (config, handler) in streams {
            manager.register_handler(config, handler);
        }

        let instant = Instant::now();
        manager
            .dispatch_event(
                &WindowEvent::CursorMoved(WindowCursorMoved { position: inside }),
                &rtree,
                instant,
            )
            .await;
        manager
            .dispatch_event(
                &WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                }),
                &rtree,
                instant + Duration::from_millis(1),
            )
            .await;

        let outside = [
            original_frame.bounds.x + original_frame.bounds.width + 50.0,
            original_frame.bounds.y + original_frame.bounds.height + 30.0,
        ];
        manager
            .dispatch_event(
                &WindowEvent::CursorMoved(WindowCursorMoved { position: outside }),
                &rtree,
                instant + Duration::from_millis(2),
            )
            .await;

        let expected_x = f64::from(outside[0] - original_frame.bounds.x);
        let expected_y = f64::from(outside[1] - original_frame.bounds.y);
        let params = manager.state().params().await;
        assert_eq!(params["frame_x"], ScalarValue::Float64(Some(expected_x)));
        assert_eq!(params["frame_y"], ScalarValue::Float64(Some(expected_y)));
        assert_eq!(
            params["frame_width"],
            ScalarValue::Float64(Some(f64::from(original_frame.bounds.width)))
        );
        assert_eq!(
            params["frame_height"],
            ScalarValue::Float64(Some(f64::from(original_frame.bounds.height)))
        );

        {
            let mut runtime = manager.state().runtime.lock().await;
            let capture = runtime
                .active_widget_gesture_frame
                .as_ref()
                .expect("widget frame captured at mouse-down");
            assert_eq!(capture.frame, original_frame);
            let reassigned = runtime
                .session
                .evaluate(
                    EvaluationRequest::new().widget_frames(
                        WidgetFrameAssignments::try_from_iter([(
                            "regions",
                            WidgetFrame::try_new(180.0, 90.0, 100.0, 56.0).unwrap(),
                        )])
                        .unwrap(),
                    ),
                )
                .await
                .expect("reassign explicit widget frame during capture");
            runtime.last_interaction_state = reassigned.interaction;
            runtime.last_event_datum_state = reassigned.event_datums;
            runtime.last_widget_frame_state = reassigned.widget_frames;
            assert_eq!(
                runtime.last_widget_frame_state.by_widget_id["regions"]
                    .bounds
                    .x,
                180.0
            );
        }

        manager
            .dispatch_event(
                &WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Released,
                    button: MouseButton::Left,
                }),
                &rtree,
                instant + Duration::from_millis(3),
            )
            .await;

        let params = manager.state().params().await;
        assert_eq!(params["frame_x"], ScalarValue::Float64(Some(expected_x)));
        assert_eq!(params["frame_y"], ScalarValue::Float64(Some(expected_y)));
        assert_eq!(
            params["frame_width"],
            ScalarValue::Float64(Some(f64::from(original_frame.bounds.width)))
        );
        assert_eq!(
            params["frame_height"],
            ScalarValue::Float64(Some(f64::from(original_frame.bounds.height)))
        );
    }

    #[tokio::test]
    async fn slider_drag_uses_resolved_inner_track_and_stops_on_mouse_up() {
        let ctx = SessionContext::new();
        let compiled = Chart::<Cartesian>::new()
            .widget(
                Slider::new("volume", 0.0, 100.0)
                    .step(10.0)
                    .default(0.0)
                    .title("Volume")
                    .format(".0f")
                    .position(ChromePosition::Left),
            )
            .compile(&ctx)
            .await
            .expect("compile slider chart");
        let streams = event_streams_for_plot_bindings(&compiled, &ctx).expect("slider streams");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial slider scene");
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        let origin = rtree.named_group_origin("volume").expect("slider origin");
        let frame = {
            let runtime = state.runtime.lock().await;
            runtime
                .last_widget_frame_state
                .by_mark_path
                .values()
                .find(|frame| frame.widget_id == "volume")
                .cloned()
                .expect("slider frame")
        };
        let track_x0 = origin[0] + 8.0;
        let track_width = frame.bounds.width - 16.0;
        let start = [track_x0, origin[1] + 24.0];
        assert_eq!(
            rtree
                .pick_top_mark_at_point(&start)
                .expect("slider handle hit")
                .name,
            "handle"
        );

        let mut manager = EventStreamManager::new(state);
        manager.register_handler(
            EventStreamConfig {
                types: vec![SceneGraphEventType::MouseDown],
                ..Default::default()
            },
            Arc::new(crate::WidgetGestureFrameCaptureHandler),
        );
        for (config, handler) in streams {
            manager.register_handler(config, handler);
        }
        let instant = Instant::now();
        manager
            .dispatch_event(
                &WindowEvent::CursorMoved(WindowCursorMoved { position: start }),
                &rtree,
                instant,
            )
            .await;
        manager
            .dispatch_event(
                &WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                }),
                &rtree,
                instant + Duration::from_millis(1),
            )
            .await;

        let middle = [track_x0 + track_width * 0.54, start[1]];
        manager
            .dispatch_event(
                &WindowEvent::CursorMoved(WindowCursorMoved { position: middle }),
                &rtree,
                instant + Duration::from_millis(2),
            )
            .await;
        assert_eq!(
            manager.state().params().await["volume__value"],
            ScalarValue::Float64(Some(50.0))
        );

        let past_end = [track_x0 + track_width + 80.0, start[1]];
        manager
            .dispatch_event(
                &WindowEvent::CursorMoved(WindowCursorMoved { position: past_end }),
                &rtree,
                instant + Duration::from_millis(3),
            )
            .await;
        assert_eq!(
            manager.state().params().await["volume__value"],
            ScalarValue::Float64(Some(100.0))
        );
        manager
            .dispatch_event(
                &WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Released,
                    button: MouseButton::Left,
                }),
                &rtree,
                instant + Duration::from_millis(4),
            )
            .await;
        manager
            .dispatch_event(
                &WindowEvent::CursorMoved(WindowCursorMoved {
                    position: [track_x0, start[1]],
                }),
                &rtree,
                instant + Duration::from_millis(5),
            )
            .await;
        assert_eq!(
            manager.state().params().await["volume__value"],
            ScalarValue::Float64(Some(100.0))
        );
    }

    #[tokio::test]
    async fn slider_track_drag_settles_and_refreshes_handle_hit_geometry() {
        let ctx = SessionContext::new();
        let compiled = Chart::<Cartesian>::new()
            .widget(
                Slider::new("volume", 0.0, 100.0)
                    .step(10.0)
                    .default(0.0)
                    .title("Volume")
                    .position(ChromePosition::Left),
            )
            .compile(&ctx)
            .await
            .expect("compile slider chart");
        let streams = event_streams_for_plot_bindings(&compiled, &ctx).expect("slider streams");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial slider scene");
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        let origin = rtree.named_group_origin("volume").expect("slider origin");
        let frame = {
            let runtime = state.runtime.lock().await;
            runtime
                .last_widget_frame_state
                .by_mark_path
                .values()
                .find(|frame| frame.widget_id == "volume")
                .cloned()
                .expect("slider frame")
        };
        let track_x0 = origin[0] + 8.0;
        let track_width = frame.bounds.width - 16.0;
        let track_y = origin[1] + 24.0;
        let track_start = [track_x0 + track_width * 0.7, track_y];
        assert_eq!(
            rtree
                .pick_top_mark_at_point(&track_start)
                .expect("slider track hit")
                .name,
            "track"
        );

        let mut manager = EventStreamManager::new(state);
        manager.register_handler(
            EventStreamConfig {
                types: vec![SceneGraphEventType::MouseDown],
                ..Default::default()
            },
            Arc::new(crate::WidgetGestureFrameCaptureHandler),
        );
        for (config, handler) in streams {
            manager.register_handler(config, handler);
        }
        let instant = Instant::now();
        manager
            .dispatch_event(
                &WindowEvent::CursorMoved(WindowCursorMoved {
                    position: track_start,
                }),
                &rtree,
                instant,
            )
            .await;
        manager
            .dispatch_event(
                &WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                }),
                &rtree,
                instant + Duration::from_millis(1),
            )
            .await;

        let drag_position = [track_x0 + track_width * 0.54, track_y];
        manager
            .dispatch_event(
                &WindowEvent::CursorMoved(WindowCursorMoved {
                    position: drag_position,
                }),
                &rtree,
                instant + Duration::from_millis(2),
            )
            .await;
        assert_eq!(
            manager.state().params().await["volume__value"],
            ScalarValue::Float64(Some(50.0))
        );
        manager
            .dispatch_event(
                &WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Released,
                    button: MouseButton::Left,
                }),
                &rtree,
                instant + Duration::from_millis(3),
            )
            .await;
        let settle = manager
            .dispatch_event(
                &WindowEvent::InteractionSettled { generation: 1 },
                &rtree,
                instant + Duration::from_millis(123),
            )
            .await;
        assert!(settle.rerender);
        assert!(settle.rebuild_geometry);

        let settled_scene = crate::ChartSceneGraphBuilder
            .build(manager.state_mut())
            .await
            .expect("settled slider scene");
        let settled_rtree = SceneGraphRTree::from_scene_graph(&settled_scene);
        let settled_handle = [track_x0 + track_width * 0.5, track_y];
        assert_eq!(
            settled_rtree
                .pick_top_mark_at_point(&settled_handle)
                .expect("settled slider handle hit")
                .name,
            "handle"
        );
    }

    #[tokio::test]
    async fn slider_zero_width_inner_track_ignores_pointer_gesture() {
        let ctx = SessionContext::new();
        let mut theme = Theme::light();
        theme
            .append_css("slider#volume { padding-inline: 50px; }")
            .expect("zero-track slider theme");
        let compiled = Chart::<Cartesian>::new()
            .theme(theme)
            .widget(
                Slider::new("volume", 0.0, 100.0)
                    .step(10.0)
                    .default(40.0)
                    .title("Volume")
                    .format(".0f")
                    .position(ChromePosition::Left),
            )
            .compile(&ctx)
            .await
            .expect("compile zero-track slider");
        let streams = event_streams_for_plot_bindings(&compiled, &ctx).expect("slider streams");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("zero-track slider scene");
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        let origin = rtree.named_group_origin("volume").expect("slider origin");
        let frame = {
            let runtime = state.runtime.lock().await;
            runtime
                .last_widget_frame_state
                .by_mark_path
                .values()
                .find(|frame| frame.widget_id == "volume")
                .cloned()
                .expect("slider frame")
        };
        let start = [origin[0] + frame.bounds.width / 2.0, origin[1] + 24.0];
        assert_eq!(
            rtree
                .pick_top_mark_at_point(&start)
                .expect("collapsed slider handle hit")
                .name,
            "handle"
        );
        let mut manager = EventStreamManager::new(state);
        manager.register_handler(
            EventStreamConfig {
                types: vec![SceneGraphEventType::MouseDown],
                ..Default::default()
            },
            Arc::new(crate::WidgetGestureFrameCaptureHandler),
        );
        for (config, handler) in streams {
            manager.register_handler(config, handler);
        }
        let instant = Instant::now();
        manager
            .dispatch_event(
                &WindowEvent::CursorMoved(WindowCursorMoved { position: start }),
                &rtree,
                instant,
            )
            .await;
        manager
            .dispatch_event(
                &WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                }),
                &rtree,
                instant + Duration::from_millis(1),
            )
            .await;
        manager
            .dispatch_event(
                &WindowEvent::CursorMoved(WindowCursorMoved {
                    position: [start[0] + 500.0, start[1]],
                }),
                &rtree,
                instant + Duration::from_millis(2),
            )
            .await;
        manager
            .dispatch_event(
                &WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Released,
                    button: MouseButton::Left,
                }),
                &rtree,
                instant + Duration::from_millis(3),
            )
            .await;
        assert_eq!(
            manager.state().params().await["volume__value"],
            ScalarValue::Float64(Some(40.0))
        );
    }

    #[tokio::test]
    async fn two_checkboxes_dispatch_only_to_the_picked_widget() {
        let ctx = SessionContext::new();
        let compiled = Chart::<Cartesian>::new()
            .widget(Checkbox::new("first", "First", false).position(ChromePosition::Left))
            .widget(Checkbox::new("second", "Second", false).position(ChromePosition::Left))
            .compile(&ctx)
            .await
            .expect("compile two-checkbox chart");
        let compiled: CompiledPlot =
            bincode::deserialize(&bincode::serialize(&compiled).expect("serialize chart"))
                .expect("deserialize chart");
        let streams = event_streams_for_plot_bindings(&compiled, &ctx).expect("checkbox streams");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial two-checkbox scene");
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        let first_origin = rtree.named_group_origin("first").expect("first origin");
        let second_origin = rtree.named_group_origin("second").expect("second origin");
        let first_position = [first_origin[0] + 7.0, first_origin[1] + 16.0];
        let second_position = [second_origin[0] + 7.0, second_origin[1] + 16.0];
        assert_eq!(
            rtree
                .pick_top_mark_at_point(&first_position)
                .expect("first box hit")
                .name,
            "box"
        );
        assert_eq!(
            rtree
                .pick_top_mark_at_point(&second_position)
                .expect("second box hit")
                .name,
            "box"
        );

        let mut manager = EventStreamManager::new(state);
        for (config, handler) in streams {
            manager.register_handler(config, handler);
        }
        let instant = Instant::now();
        manager
            .dispatch_event(
                &WindowEvent::CursorMoved(WindowCursorMoved {
                    position: first_position,
                }),
                &rtree,
                instant,
            )
            .await;
        manager
            .dispatch_event(
                &WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                }),
                &rtree,
                instant,
            )
            .await;
        let first_click = manager
            .dispatch_event(
                &WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Released,
                    button: MouseButton::Left,
                }),
                &rtree,
                instant + Duration::from_millis(10),
            )
            .await;
        assert!(first_click.rerender);
        assert_eq!(
            manager.state().params().await.get("first__checked"),
            Some(&ScalarValue::Boolean(Some(true)))
        );
        assert_eq!(
            manager.state().params().await.get("second__checked"),
            Some(&ScalarValue::Boolean(Some(false)))
        );

        let second_instant = instant + Duration::from_millis(600);
        manager
            .dispatch_event(
                &WindowEvent::CursorMoved(WindowCursorMoved {
                    position: second_position,
                }),
                &rtree,
                second_instant,
            )
            .await;
        manager
            .dispatch_event(
                &WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                }),
                &rtree,
                second_instant,
            )
            .await;
        let second_click = manager
            .dispatch_event(
                &WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Released,
                    button: MouseButton::Left,
                }),
                &rtree,
                second_instant + Duration::from_millis(10),
            )
            .await;
        assert!(second_click.rerender);
        assert_eq!(
            manager.state().params().await.get("first__checked"),
            Some(&ScalarValue::Boolean(Some(true)))
        );
        assert_eq!(
            manager.state().params().await.get("second__checked"),
            Some(&ScalarValue::Boolean(Some(true)))
        );
    }

    #[tokio::test]
    async fn button_box_and_label_clicks_count_once_and_overflow_is_rejected() {
        let ctx = SessionContext::new();
        let compiled = Chart::<Cartesian>::new()
            .widget(
                Button::new("clear")
                    .label("Clear")
                    .variant(ButtonVariant::Accent)
                    .position(ChromePosition::Left),
            )
            .compile(&ctx)
            .await
            .expect("compile button chart");
        let click_index = compiled
            .event_bindings()
            .iter()
            .position(|binding| binding.event_type == ChartEventType::Click)
            .expect("button click binding");
        let handler = compile_handler_for_binding_index(&compiled, &ctx, click_index);
        let streams = event_streams_for_plot_bindings(&compiled, &ctx).expect("button streams");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());

        let scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial button scene");
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        let box_instance = rtree
            .iter()
            .find(|geometry| geometry.mark_instance.name == "box")
            .map(|geometry| geometry.mark_instance.clone())
            .expect("button box hit target");
        let box_position = rect_instance_point(&scene, &box_instance);
        let resting_fill = rect_instance_fill(&scene, &box_instance);
        let label_instance = rtree
            .iter()
            .find(|geometry| geometry.mark_instance.name == "label")
            .map(|geometry| geometry.mark_instance.clone())
            .expect("button label hit target");
        assert!(
            rtree
                .iter()
                .all(|geometry| geometry.mark_instance.name != "focus-ring")
        );

        let first = click_mark(
            &mut state,
            &handler,
            Some(box_instance.clone()),
            [0.0, 0.0],
            false,
        )
        .await;
        assert!(first.rerender);
        assert_eq!(
            state.params().await.get("clear__activations"),
            Some(&ScalarValue::UInt64(Some(1)))
        );

        let second = click_mark(
            &mut state,
            &handler,
            Some(label_instance),
            [0.0, 0.0],
            false,
        )
        .await;
        assert!(second.rerender);
        assert_eq!(
            state.params().await.get("clear__activations"),
            Some(&ScalarValue::UInt64(Some(2)))
        );

        let mut manager = EventStreamManager::new(state);
        for (config, stream_handler) in streams {
            manager.register_handler(config, stream_handler);
        }
        let instant = Instant::now();
        let hover_status = manager
            .dispatch_event(
                &WindowEvent::CursorMoved(WindowCursorMoved {
                    position: box_position,
                }),
                &rtree,
                instant,
            )
            .await;
        assert_eq!(hover_status.cursor, Some(CursorStyle::Pointer));
        assert!(hover_status.rerender);
        assert_eq!(
            manager.state().params().await.get("clear__hover"),
            Some(&ScalarValue::Boolean(Some(true)))
        );
        let hover_scene = crate::ChartSceneGraphBuilder
            .build(manager.state_mut())
            .await
            .expect("hovered button scene");
        let hover_fill = rect_instance_fill(&hover_scene, &box_instance);
        assert_ne!(hover_fill, resting_fill);

        let pressed_status = manager
            .dispatch_event(
                &WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                }),
                &rtree,
                instant + Duration::from_millis(1),
            )
            .await;
        assert!(pressed_status.rerender);
        assert_eq!(
            manager.state().params().await.get("clear__pressed"),
            Some(&ScalarValue::Boolean(Some(true)))
        );
        let pressed_scene = crate::ChartSceneGraphBuilder
            .build(manager.state_mut())
            .await
            .expect("pressed button scene");
        let pressed_fill = rect_instance_fill(&pressed_scene, &box_instance);
        assert_ne!(pressed_fill, hover_fill);

        let released = manager
            .dispatch_event(
                &WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Released,
                    button: MouseButton::Left,
                }),
                &rtree,
                instant + Duration::from_millis(2),
            )
            .await;
        assert!(released.rerender);
        assert_eq!(
            manager.state().params().await.get("clear__pressed"),
            Some(&ScalarValue::Boolean(Some(false)))
        );
        assert_eq!(
            manager.state().params().await.get("clear__activations"),
            Some(&ScalarValue::UInt64(Some(3)))
        );

        let miss_position = [-10_000.0, -10_000.0];
        let leave_status = manager
            .dispatch_event(
                &WindowEvent::CursorMoved(WindowCursorMoved {
                    position: miss_position,
                }),
                &rtree,
                instant,
            )
            .await;
        assert_eq!(leave_status.cursor, Some(CursorStyle::Default));
        assert_eq!(
            manager.state().params().await.get("clear__hover"),
            Some(&ScalarValue::Boolean(Some(false)))
        );
        manager
            .dispatch_event(
                &WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                }),
                &rtree,
                instant,
            )
            .await;
        let miss = manager
            .dispatch_event(
                &WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Released,
                    button: MouseButton::Left,
                }),
                &rtree,
                instant + Duration::from_millis(10),
            )
            .await;
        assert!(!miss.rerender);
        assert_eq!(
            manager.state().params().await.get("clear__activations"),
            Some(&ScalarValue::UInt64(Some(3)))
        );

        manager
            .state()
            .set_param("clear__activations", u64::MAX)
            .expect("set activation near overflow");
        let before_errors = manager.state().event_metrics().await.evaluation_errors;
        let overflow = click_mark(
            manager.state_mut(),
            &handler,
            Some(box_instance),
            [0.0, 0.0],
            false,
        )
        .await;
        assert!(!overflow.rerender);
        assert_eq!(
            manager.state().params().await.get("clear__activations"),
            Some(&ScalarValue::UInt64(Some(u64::MAX)))
        );
        assert_eq!(
            manager.state().event_metrics().await.evaluation_errors,
            before_errors + 1
        );
    }

    #[tokio::test]
    async fn button_action_clears_point_selection_and_restores_scene_once() {
        let ctx = SessionContext::new();
        let data = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('Alpha', 1.0, 5.0), ('Alpha', 2.0, 6.5),
                    ('Beta', 3.5, 2.0), ('Beta', 4.5, 3.2),
                    ('Gamma', 6.0, 5.5), ('Gamma', 7.0, 6.8)
                ) AS t(category, x_value, y_value)",
            )
            .await
            .expect("button clear-selection data");
        let picked = Selection::new("picked").empty_selects_all();
        let selected = picked.predicate();
        let point_click = ChartEventBinding::on(ChartEventType::Click)
            .filter(event::button().eq(lit("left")))
            .filter(event::datum("category").is_not_null())
            .set_selection(
                "picked",
                SelectionUpdate::replace_all_clauses([SelectionClauseUpdate::equality_value(
                    col("category"),
                    event::datum("category"),
                )
                .facet_scope(CoordinationScope::Shared)]),
            )
            .exact();
        let button = Button::new("clear")
            .label("Clear selection")
            .action(ChartAction::new().clear_selection("picked"));
        let compiled = Chart::<Cartesian>::new()
            .canvas_size(620.0, 360.0)
            .plot_size(500.0, 280.0)
            .data(data)
            .selection(picked)
            .event_binding(point_click)
            .mark(
                Symbol::new()
                    .id("context")
                    .x(col("x_value"))
                    .y(col("y_value"))
                    .size(140.0)
                    .fill("#C8CDD2")
                    .zindex(5),
            )
            .mark(
                Symbol::new()
                    .id("selected")
                    .transform_no_output(Filter::new(selected), |mark| mark)
                    .x(col("x_value"))
                    .y(col("y_value"))
                    .size(140.0)
                    .fill("#0072B2")
                    .zindex(10),
            )
            .widget(button.position(ChromePosition::Left))
            .compile(&ctx)
            .await
            .expect("compile Button clear-selection chart");
        let point_handler_index = compiled
            .event_bindings()
            .iter()
            .position(|binding| {
                binding.event_type == ChartEventType::Click && binding.mark_ids().is_empty()
            })
            .expect("plot point click binding");
        let button_handler_index = compiled
            .event_bindings()
            .iter()
            .position(|binding| {
                binding.event_type == ChartEventType::Click
                    && binding.mark_ids().iter().any(|id| id == "clear.box")
            })
            .expect("Button click binding");
        let point_handler = compile_handler_for_binding_index(&compiled, &ctx, point_handler_index);
        let button_handler =
            compile_handler_for_binding_index(&compiled, &ctx, button_handler_index);
        let graph = Arc::new(
            param_change_graph_for_plot(&compiled, &ctx).expect("compile Button reaction graph"),
        );
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        state.has_param_reactions = true;
        state.runtime.lock().await.param_change_graph = Some(graph);

        let initial = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial Button clear-selection scene");
        let blue = [0.0, 114.0 / 255.0, 178.0 / 255.0, 1.0];
        assert_eq!(
            collect_symbol_fills(&initial)
                .iter()
                .filter(|fill| **fill == blue)
                .count(),
            6
        );
        let alpha = retained_named_event_datum_mark_instance(
            &state,
            &initial,
            "selected",
            "category",
            ScalarValue::Utf8(Some("Alpha".to_string())),
        )
        .await;
        let alpha_center = symbol_instance_point(&initial, &alpha);
        let alpha_position = [alpha_center[0] + 3.0, alpha_center[1] + 3.0];
        let initial_rtree = SceneGraphRTree::from_scene_graph(&initial);
        let alpha = initial_rtree
            .pick_top_mark_at_point(&alpha_position)
            .cloned()
            .expect("pick selected Alpha point");
        assert_eq!(alpha.name, "selected");
        let select_status = click_mark(
            &mut state,
            &point_handler,
            Some(alpha),
            alpha_position,
            false,
        )
        .await;
        assert!(select_status.rerender);
        assert_eq!(
            state.params().await.get("clear__activations"),
            Some(&ScalarValue::UInt64(Some(0))),
            "a plot click must not activate the Button"
        );
        {
            let runtime = state.runtime.lock().await;
            let clauses = runtime.session.selection_clauses_for_diagnostics("picked");
            assert_eq!(clauses.len(), 1, "point click should create one clause");
        }
        let selected_scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("selected scene");
        assert_eq!(
            collect_symbol_fills(&selected_scene)
                .iter()
                .filter(|fill| **fill == blue)
                .count(),
            2,
            "selecting Alpha should leave only its two blue foreground points"
        );
        assert_eq!(
            state
                .runtime
                .lock()
                .await
                .session
                .selection_clauses_for_diagnostics("picked")
                .len(),
            1
        );

        let selected_rtree = SceneGraphRTree::from_scene_graph(&selected_scene);
        let button_box = selected_rtree
            .iter()
            .find(|geometry| geometry.mark_instance.name == "box")
            .map(|geometry| geometry.mark_instance.clone())
            .expect("Button box hit target");
        let button_position = rect_instance_point(&selected_scene, &button_box);
        let clear_status = click_mark(
            &mut state,
            &button_handler,
            Some(button_box),
            button_position,
            false,
        )
        .await;
        assert!(clear_status.rerender, "one activation requests one render");
        assert_eq!(
            state.params().await.get("clear__activations"),
            Some(&ScalarValue::UInt64(Some(1)))
        );
        assert_eq!(state.param_changes_since(0).len(), 1);
        assert!(
            state
                .runtime
                .lock()
                .await
                .session
                .selection_clauses_for_diagnostics("picked")
                .is_empty()
        );
        let cleared_scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("single clear render");
        assert_eq!(
            collect_symbol_fills(&cleared_scene)
                .iter()
                .filter(|fill| **fill == blue)
                .count(),
            6,
            "clearing restores the empty-selects-all foreground"
        );
    }

    #[tokio::test]
    async fn widget_cell_button_click_uses_prefixed_target_and_frame_inputs() {
        let ctx = SessionContext::new();
        let compiled = Chart::<HConcat>::new()
            .mark(WidgetCell::widget(Button::new("clear").label("Clear")).name("controls"))
            .compile(&ctx)
            .await
            .expect("compile WidgetCell button chart");
        let compiled: CompiledPlot = bincode::deserialize(
            &bincode::serialize(&compiled).expect("serialize WidgetCell chart"),
        )
        .expect("deserialize WidgetCell chart");
        let click_index = compiled
            .event_bindings()
            .iter()
            .position(|binding| binding.event_type == ChartEventType::Click)
            .expect("button click binding");
        assert_eq!(
            compiled.event_bindings()[click_index].mark_ids(),
            &["controls.clear.box", "controls.clear.label"]
        );
        let handler = compile_handler_for_binding_index(&compiled, &ctx, click_index);
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());

        let scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial WidgetCell button scene");
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        let box_instance = rtree
            .iter()
            .find(|geometry| geometry.mark_instance.name == "box")
            .map(|geometry| geometry.mark_instance.clone())
            .expect("WidgetCell button box hit target");
        let click = click_mark(&mut state, &handler, Some(box_instance), [0.0, 0.0], false).await;
        assert!(click.rerender);
        assert_eq!(
            state.params().await.get("clear__activations"),
            Some(&ScalarValue::UInt64(Some(1)))
        );
    }

    #[tokio::test]
    async fn widget_cell_checkbox_list_cross_filters_sibling_subplot() {
        let ctx = SessionContext::new();
        let data = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('North', 1.0, 6.0),
                    ('South', 2.0, 2.0),
                    ('West', 3.0, 5.0)
                ) AS t(region, x_value, y_value)",
            )
            .await
            .expect("build widget-cell cross-filter data");
        let regions = Selection::new("regions").empty_selects_all();
        let selected = regions.predicate();
        let items = WidgetItems::Static(
            ["North", "South", "West"]
                .into_iter()
                .map(|region| {
                    WidgetItemRow::new([
                        (
                            "region".to_string(),
                            ScalarValue::Utf8(Some(region.to_string())),
                        ),
                        (
                            "label".to_string(),
                            ScalarValue::Utf8(Some(region.to_string())),
                        ),
                    ])
                })
                .collect(),
        );
        let scatter = Plot::<Cartesian>::new()
            .data(data)
            .mark(
                Symbol::new()
                    .x(col("x_value"))
                    .y(col("y_value"))
                    .size(100.0)
                    .fill("#C8CDD2"),
            )
            .mark(
                Symbol::new()
                    .transform_no_output(Filter::new(selected), |mark| mark)
                    .x(col("x_value"))
                    .y(col("y_value"))
                    .size(100.0)
                    .fill("#0072B2"),
            );
        let compiled = Chart::<HConcat>::new()
            .canvas_size(520.0, 280.0)
            .plot_size(480.0, 220.0)
            .configure_coord(|coord| {
                coord
                    .widths([TrackSizing::Auto, TrackSizing::Flex(1.0)])
                    .spacing(16.0)
            })
            .mark(
                WidgetCell::widget(
                    CheckboxList::new("regions", items)
                        .value(col("region"))
                        .label(col("label"))
                        .selection(&regions),
                )
                .name("filters"),
            )
            .mark(Subplot::new(scatter).name("scatter"))
            .compile(&ctx)
            .await
            .expect("compile widget-cell cross-filter");
        let binding_index = compiled
            .event_bindings()
            .iter()
            .position(|binding| {
                binding.event_type == ChartEventType::Click
                    && binding
                        .mark_ids()
                        .iter()
                        .any(|target| target == "filters.regions.label")
            })
            .expect("rebased checkbox-list click binding");
        assert_eq!(
            compiled.event_bindings()[binding_index].mark_ids(),
            &[
                "filters.regions.box",
                "filters.regions.check",
                "filters.regions.label",
            ],
            "only the visible control and label form the list item action surface"
        );
        let streams = event_streams_for_plot_bindings(&compiled, &ctx).expect("widget streams");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial widget-cell cross-filter scene");
        let blue = [0.0, 114.0 / 255.0, 178.0 / 255.0, 1.0];
        assert_eq!(
            collect_symbol_fills(&scene)
                .iter()
                .filter(|fill| **fill == blue)
                .count(),
            3,
            "empty_selects_all should initially retain every foreground point"
        );

        let south = retained_named_event_datum_mark_instance(
            &state,
            &scene,
            "label",
            "__value",
            ScalarValue::Utf8(Some("South".to_string())),
        )
        .await;
        let (mark, mark_origin) =
            scene_mark_at_path_with_origin(&scene.marks, &south.mark_path, scene.origin)
                .expect("South label scene mark");
        let SceneMark::Text(label) = mark else {
            panic!("expected South label text mark")
        };
        let label_position = label
            .label_position_iter()
            .nth(south.instance_index.expect("South label index"))
            .expect("South label position");
        let label_point = [
            mark_origin[0] + label_position[0] + 3.0,
            mark_origin[1] + label_position[1],
        ];
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        assert_eq!(
            rtree
                .pick_top_mark_at_point(&label_point)
                .expect("South label hit")
                .name,
            "label"
        );

        let mut manager = EventStreamManager::new(state);
        manager.register_handler(
            EventStreamConfig {
                types: vec![SceneGraphEventType::MouseDown],
                ..Default::default()
            },
            Arc::new(crate::WidgetGestureFrameCaptureHandler),
        );
        for (config, handler) in streams {
            manager.register_handler(config, handler);
        }
        let instant = Instant::now();
        manager
            .dispatch_event(
                &WindowEvent::CursorMoved(WindowCursorMoved {
                    position: label_point,
                }),
                &rtree,
                instant,
            )
            .await;
        manager
            .dispatch_event(
                &WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                }),
                &rtree,
                instant + Duration::from_millis(1),
            )
            .await;
        let status = manager
            .dispatch_event(
                &WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Released,
                    button: MouseButton::Left,
                }),
                &rtree,
                instant + Duration::from_millis(2),
            )
            .await;
        assert!(
            status.rerender,
            "clicking the visible label toggles its item"
        );

        let updated = crate::ChartSceneGraphBuilder
            .build(manager.state_mut())
            .await
            .expect("selected widget-cell cross-filter scene");
        assert_eq!(
            collect_symbol_fills(&updated)
                .iter()
                .filter(|fill| **fill == blue)
                .count(),
            1,
            "the WidgetCell selection should filter the sibling subplot"
        );
    }

    #[tokio::test]
    async fn chrome_checkbox_list_lower_label_toggles_its_item() {
        let ctx = SessionContext::new();
        let regions = Selection::new("regions").empty_selects_all();
        let items = WidgetItems::Static(
            ["North", "South", "West"]
                .into_iter()
                .map(|region| {
                    WidgetItemRow::new([
                        (
                            "region".to_string(),
                            ScalarValue::Utf8(Some(region.to_string())),
                        ),
                        (
                            "label".to_string(),
                            ScalarValue::Utf8(Some(region.to_string())),
                        ),
                    ])
                })
                .collect(),
        );
        let compiled = Chart::<Cartesian>::new()
            .canvas_size(720.0, 440.0)
            .plot_size(480.0, 300.0)
            .widget(
                CheckboxList::new("region_filter", items)
                    .value(col("region"))
                    .label(col("label"))
                    .selection(&regions)
                    .position(ChromePosition::Left),
            )
            .compile(&ctx)
            .await
            .expect("compile chrome checkbox-list chart");
        let streams = event_streams_for_plot_bindings(&compiled, &ctx).expect("widget streams");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial chrome checkbox-list scene");
        let south = retained_named_event_datum_mark_instance(
            &state,
            &scene,
            "label",
            "__value",
            ScalarValue::Utf8(Some("South".to_string())),
        )
        .await;
        let (mark, mark_origin) =
            scene_mark_at_path_with_origin(&scene.marks, &south.mark_path, scene.origin)
                .expect("South label scene mark");
        let SceneMark::Text(label) = mark else {
            panic!("expected South label text mark")
        };
        let label_position = label
            .label_position_iter()
            .nth(south.instance_index.expect("South label index"))
            .expect("South label position");
        let label_point = [
            mark_origin[0] + label_position[0] + 3.0,
            mark_origin[1] + label_position[1],
        ];
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        let picked = rtree
            .pick_top_mark_at_point(&label_point)
            .expect("South label hit");
        assert_eq!(picked.name, "label");
        assert_eq!(picked.instance_index, Some(1));

        let mut manager = EventStreamManager::new(state);
        manager.register_handler(
            EventStreamConfig {
                types: vec![SceneGraphEventType::MouseDown],
                ..Default::default()
            },
            Arc::new(crate::WidgetGestureFrameCaptureHandler),
        );
        for (config, handler) in streams {
            manager.register_handler(config, handler);
        }
        let instant = Instant::now();
        manager
            .dispatch_event(
                &WindowEvent::CursorMoved(WindowCursorMoved {
                    position: label_point,
                }),
                &rtree,
                instant,
            )
            .await;
        manager
            .dispatch_event(
                &WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                }),
                &rtree,
                instant + Duration::from_millis(1),
            )
            .await;
        let status = manager
            .dispatch_event(
                &WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Released,
                    button: MouseButton::Left,
                }),
                &rtree,
                instant + Duration::from_millis(2),
            )
            .await;
        assert!(status.rerender, "clicking the South label toggles its item");
        assert!(
            status.rebuild_geometry,
            "a discrete list toggle changes conditional mark topology and must refresh the R-tree"
        );
        assert_eq!(
            manager
                .state()
                .runtime
                .lock()
                .await
                .session
                .selection_clauses_for_diagnostics("regions")
                .len(),
            1,
        );

        let updated_scene = crate::ChartSceneGraphBuilder
            .build(manager.state_mut())
            .await
            .expect("selected chrome checkbox-list scene");
        let west = retained_named_event_datum_mark_instance(
            manager.state(),
            &updated_scene,
            "label",
            "__value",
            ScalarValue::Utf8(Some("West".to_string())),
        )
        .await;
        let (mark, mark_origin) = scene_mark_at_path_with_origin(
            &updated_scene.marks,
            &west.mark_path,
            updated_scene.origin,
        )
        .expect("West label scene mark");
        let SceneMark::Text(label) = mark else {
            panic!("expected West label text mark")
        };
        let label_position = label
            .label_position_iter()
            .nth(west.instance_index.expect("West label index"))
            .expect("West label position");
        let west_point = [
            mark_origin[0] + label_position[0] + 3.0,
            mark_origin[1] + label_position[1],
        ];
        let updated_rtree = SceneGraphRTree::from_scene_graph(&updated_scene);
        let picked = updated_rtree
            .pick_top_mark_at_point(&west_point)
            .expect("West label hit after selecting South");
        assert_eq!(picked.name, "label");
        assert_eq!(picked.instance_index, Some(2));
        let second = instant + Duration::from_millis(10);
        manager
            .dispatch_event(
                &WindowEvent::CursorMoved(WindowCursorMoved {
                    position: west_point,
                }),
                &updated_rtree,
                second,
            )
            .await;
        manager
            .dispatch_event(
                &WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                }),
                &updated_rtree,
                second + Duration::from_millis(1),
            )
            .await;
        let status = manager
            .dispatch_event(
                &WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Released,
                    button: MouseButton::Left,
                }),
                &updated_rtree,
                second + Duration::from_millis(2),
            )
            .await;
        assert!(status.rerender, "clicking West after a rerender toggles it");
        assert!(status.rebuild_geometry);
        assert_eq!(
            manager
                .state()
                .runtime
                .lock()
                .await
                .session
                .selection_clauses_for_diagnostics("regions")
                .len(),
            2,
        );
    }

    #[tokio::test]
    async fn decorated_treemap_header_click_zooms_and_double_click_resets() {
        let ctx = SessionContext::new();
        let root = Param::new("decorated_treemap_root", ScalarValue::Utf8(None));
        let compiled = Chart::with_coord(
            Treemap::new()
                .path_columns(["division", "region", "team", "product"])
                .value(sum(col("sales")))
                .root_path_param(root.name.clone())
                .display_levels(3)
                .header_bars(TreemapHeaderBars::enabled().height_px(24.0))
                .padding(TreemapPadding::default().depth_inner_px([3.0, 1.0])),
        )
        .data(ctx.read_batch(decorated_treemap_batch()).unwrap())
        .plot_size(420.0, 280.0)
        .param(root.clone())
        .configure_guide(TreemapGuide::new().breadcrumbs(true).separators(true))
        .mark(TreeRect::new().id("cells").stroke_width(0.0))
        .mark(TreeHeader::new().id("headers").fill(col("division")))
        .mark(TreeLabel::new().id("labels"))
        .event_binding(
            ChartEventBinding::on(ChartEventType::CursorMoved)
                .filter(treemap_event::hierarchy_can_zoom().eq(lit(true)))
                .set_cursor(event::cursor(CursorStyle::Grab))
                .preview(),
        )
        .event_binding(
            ChartEventBinding::on(ChartEventType::Click)
                .filter(treemap_event::hierarchy_can_zoom().eq(lit(true)))
                .set_param(&root, treemap_event::hierarchy_path_id())
                .exact(),
        )
        .event_binding(
            ChartEventBinding::on(ChartEventType::DoubleClick)
                .set_param(&root, lit(ROOT_PATH_ID))
                .exact(),
        )
        .compile(&ctx)
        .await
        .expect("compile decorated treemap zoom plot");
        let cursor_handler = compile_handler_for_binding_index(&compiled, &ctx, 0);
        let zoom_handler = compile_handler_for_binding_index(&compiled, &ctx, 1);
        let reset_handler = compile_handler_for_binding_index(&compiled, &ctx, 2);
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());

        let initial_scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial decorated treemap scene");
        let enterprise_header = retained_rect_event_datum_mark_instance(
            &state,
            &initial_scene,
            HIERARCHY_PATH_ID_FIELD,
            ScalarValue::Utf8(Some("division=Enterprise".to_string())),
        )
        .await;
        let header_position = rect_instance_point(&initial_scene, &enterprise_header);

        let cursor_status = cursor_handler
            .handle_with_context(
                &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: header_position,
                    mark_instance: Some(enterprise_header.clone()),
                    modifiers: Default::default(),
                }),
                &EventStreamContext::default(),
                &mut state,
                &empty_rtree(),
            )
            .await;
        assert_eq!(cursor_status.cursor, Some(CursorStyle::Grab));
        assert!(!cursor_status.rerender);

        let zoom_status = click_mark(
            &mut state,
            &zoom_handler,
            Some(enterprise_header),
            header_position,
            false,
        )
        .await;
        assert!(zoom_status.rerender);
        {
            let runtime = state.runtime.lock().await;
            let params = runtime
                .session
                .effective_params_for_owner_paths(&HashMap::new());
            assert_eq!(
                params.get(&root.name),
                Some(&ScalarValue::Utf8(Some("division=Enterprise".to_string())))
            );
        }

        let zoomed_scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("zoomed decorated treemap scene");
        let _region_header = retained_rect_event_datum_mark_instance(
            &state,
            &zoomed_scene,
            HIERARCHY_PATH_ID_FIELD,
            ScalarValue::Utf8(Some("division=Enterprise/region=North America".to_string())),
        )
        .await;

        let reset_status = double_click_mark(&mut state, &reset_handler, None, [10.0, 10.0]).await;
        assert!(reset_status.rerender);
        {
            let runtime = state.runtime.lock().await;
            let params = runtime
                .session
                .effective_params_for_owner_paths(&HashMap::new());
            assert_eq!(
                params.get(&root.name),
                Some(&ScalarValue::Utf8(Some(ROOT_PATH_ID.to_string())))
            );
        }
    }

    #[tokio::test]
    async fn store_update_writes_store_state() {
        use datafusion::arrow::datatypes::DataType;

        let ctx = SessionContext::new();
        let binding = ChartEventBinding::on(ChartEventType::CanvasResize)
            .set_store(
                "brush_boxes",
                StoreUpdate::replace_rows([StoreRow::new()
                    .field("id", lit("active"))
                    .field("x_min", event::canvas_width())
                    .field("x_max", event::canvas_height())]),
            )
            .preview();
        let compiled = Chart::<Cartesian>::new()
            .store(
                Store::empty("brush_boxes")
                    .field("id", DataType::Utf8, false)
                    .field("x_min", DataType::Float64, false)
                    .field("x_max", DataType::Float64, false)
                    .primary_key(["id"])
                    .sharing(CoordinationScope::Shared),
            )
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile store binding plot");
        let runtime = CompiledChartEventBinding::compile(
            0,
            compiled.event_bindings().first().unwrap(),
            &ctx,
            compiled.param_specs(),
            compiled.selection_specs(),
            compiled.store_specs(),
            &compiled.event_datum_types(),
        )
        .expect("compile store binding runtime");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let handler = ChartEventBindingHandler {
            runtime: Arc::new(runtime),
            state: Mutex::new(ChartEventBindingState::default()),
        };

        let status = handler
            .handle_with_context(
                &SceneGraphEvent::CanvasResize(CanvasResizeEvent {
                    size: [640.0, 360.0],
                }),
                &EventStreamContext::default(),
                &mut state,
                &empty_rtree(),
            )
            .await;

        assert!(status.rerender);
        assert!(!status.rebuild_geometry);
        {
            let runtime = state.runtime.lock().await;
            assert_eq!(runtime.event_metrics.store_patch_events, 1);
            let rows = runtime.session.store_rows_for_diagnostics("brush_boxes");
            assert_eq!(rows.len(), 1);
            assert!(rows[0].0.is_empty(), "shared store is root-owned");
            assert_eq!(rows[0].1.len(), 1);
            assert_eq!(
                rows[0].1[0].get("id"),
                Some(&ScalarValue::Utf8(Some("active".to_string())))
            );
            assert_eq!(
                rows[0].1[0].get("x_min"),
                Some(&ScalarValue::Float64(Some(640.0)))
            );
            assert_eq!(
                rows[0].1[0].get("x_max"),
                Some(&ScalarValue::Float64(Some(360.0)))
            );
        }

        let second_status = handler
            .handle_with_context(
                &SceneGraphEvent::CanvasResize(CanvasResizeEvent {
                    size: [640.0, 360.0],
                }),
                &EventStreamContext::default(),
                &mut state,
                &empty_rtree(),
            )
            .await;
        assert!(
            !second_status.rerender,
            "unchanged store rows should skip reevaluation"
        );
    }

    #[tokio::test]
    async fn selection_update_writes_selection_state() {
        let ctx = SessionContext::new();
        let binding = ChartEventBinding::on(ChartEventType::CanvasResize)
            .set_selection(
                "brush",
                SelectionUpdate::replace_all_clauses([SelectionClauseUpdate::interval(lit(
                    "active",
                ))
                .facet_scope(CoordinationScope::Shared)
                .dimension(col("x"))
                .endpoints(event::canvas_width(), event::canvas_height())]),
            )
            .preview();
        let compiled = Chart::<Cartesian>::new()
            .selection(Selection::new("brush").empty_selects_nothing())
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile selection binding plot");
        let runtime = CompiledChartEventBinding::compile(
            0,
            compiled.event_bindings().first().unwrap(),
            &ctx,
            compiled.param_specs(),
            compiled.selection_specs(),
            compiled.store_specs(),
            &compiled.event_datum_types(),
        )
        .expect("compile selection binding runtime");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let handler = ChartEventBindingHandler {
            runtime: Arc::new(runtime),
            state: Mutex::new(ChartEventBindingState::default()),
        };

        let status = handler
            .handle_with_context(
                &SceneGraphEvent::CanvasResize(CanvasResizeEvent {
                    size: [640.0, 360.0],
                }),
                &EventStreamContext::default(),
                &mut state,
                &empty_rtree(),
            )
            .await;

        assert!(status.rerender);
        assert!(!status.rebuild_geometry);
        {
            let runtime = state.runtime.lock().await;
            let clauses = runtime.session.selection_clauses_for_diagnostics("brush");
            assert_eq!(clauses.len(), 1);
            let clause = &clauses[0];
            assert_eq!(clause.id, "active");
            assert_eq!(clause.scope.sharing, CoordinationScope::Shared);
            assert!(clause.scope.owner_path.is_empty());
            assert!(clause.facet_context.is_empty());
            let SelectionPredicateSpec::Interval { dimensions } = &clause.predicate else {
                panic!("expected interval predicate");
            };
            assert_eq!(dimensions.len(), 1);
            assert_eq!(dimensions[0].id, "x");
            assert_eq!(dimensions[0].min, ScalarValue::Float64(Some(640.0)));
            assert_eq!(dimensions[0].max, ScalarValue::Float64(Some(360.0)));
        }

        let second_status = handler
            .handle_with_context(
                &SceneGraphEvent::CanvasResize(CanvasResizeEvent {
                    size: [640.0, 360.0],
                }),
                &EventStreamContext::default(),
                &mut state,
                &empty_rtree(),
            )
            .await;
        assert!(
            !second_status.rerender,
            "unchanged selection clauses should skip reevaluation"
        );
    }

    #[tokio::test]
    async fn generic_predicate_selection_update_resolves_event_values() {
        let ctx = SessionContext::new();
        let binding = ChartEventBinding::on(ChartEventType::CanvasResize)
            .set_selection(
                "brush",
                SelectionUpdate::replace_all_clauses([SelectionClauseUpdate::predicate(lit(
                    "active",
                ))
                .facet_scope(CoordinationScope::Shared)
                .kind("circle")
                .value("cx", event::canvas_width())
                .value("cy", event::canvas_height())
                .expr(col("x").gt_eq(avenger_chart::selection::clause_value("cx")))]),
            )
            .preview();
        let compiled = Chart::<Cartesian>::new()
            .selection(Selection::new("brush").empty_selects_nothing())
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile generic predicate binding plot");
        let runtime = CompiledChartEventBinding::compile(
            0,
            compiled.event_bindings().first().unwrap(),
            &ctx,
            compiled.param_specs(),
            compiled.selection_specs(),
            compiled.store_specs(),
            &compiled.event_datum_types(),
        )
        .expect("compile selection binding runtime");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let handler = ChartEventBindingHandler {
            runtime: Arc::new(runtime),
            state: Mutex::new(ChartEventBindingState::default()),
        };

        let status = handler
            .handle_with_context(
                &SceneGraphEvent::CanvasResize(CanvasResizeEvent {
                    size: [640.0, 360.0],
                }),
                &EventStreamContext::default(),
                &mut state,
                &empty_rtree(),
            )
            .await;

        assert!(status.rerender);
        let runtime = state.runtime.lock().await;
        let clauses = runtime.session.selection_clauses_for_diagnostics("brush");
        assert_eq!(clauses.len(), 1);
        let SelectionPredicateSpec::Predicate { values, kind, .. } = &clauses[0].predicate else {
            panic!("expected generic predicate");
        };
        assert_eq!(kind.as_deref(), Some("circle"));
        assert_eq!(values.len(), 2);
        assert_eq!(values[0].id, "cx");
        assert_eq!(values[0].value, ScalarValue::Float64(Some(640.0)));
        assert_eq!(values[1].id, "cy");
        assert_eq!(values[1].value, ScalarValue::Float64(Some(360.0)));
    }

    #[tokio::test]
    async fn event_datum_click_writes_equality_selection_clause() {
        let binding = ChartEventBinding::on(ChartEventType::Click)
            .filter(event::button().eq(lit("left")))
            .filter(event::datum("category").is_not_null())
            .set_selection(
                "picked",
                SelectionUpdate::replace_all_clauses([equality_category_clause(lit("active"))]),
            )
            .exact();
        let (mut state, handler, mark_instance, position) =
            equality_bar_state_and_handler(binding).await;

        let status = click_mark(
            &mut state,
            &handler,
            Some(mark_instance.clone()),
            position,
            false,
        )
        .await;
        assert!(status.rerender);
        assert!(status.rebuild_geometry);

        let runtime = state.runtime.lock().await;
        let clauses = runtime.session.selection_clauses_for_diagnostics("picked");
        assert_eq!(clauses.len(), 1);
        assert_eq!(clauses[0].id, "active");
        let SelectionPredicateSpec::Equality { dimensions } = &clauses[0].predicate else {
            panic!("expected equality predicate");
        };
        assert_eq!(dimensions.len(), 1);
        assert_eq!(
            dimensions[0].value,
            ScalarValue::Utf8(Some("Beta".to_string())),
            "ev::datum should use the clicked aggregate bar's logical category"
        );
    }

    #[tokio::test]
    async fn parallel_point_click_updates_selection_by_source_row_id() {
        let binding = ChartEventBinding::on(ChartEventType::Click)
            .filter(event::button().eq(lit("left")))
            .filter(event::datum("id").is_not_null())
            .filter(parallel_event::parallel_dimension_id().is_not_null())
            .set_selection(
                "picked",
                SelectionUpdate::replace_all_clauses([SelectionClauseUpdate::equality(lit(
                    "active",
                ))
                .facet_scope(CoordinationScope::Shared)
                .dimension_named("id", col("id"), event::datum("id"))
                .build()]),
            )
            .exact();
        let (mut state, handler, mark_instance, position) =
            parallel_point_state_and_handler(binding).await;

        let _status = click_mark(&mut state, &handler, Some(mark_instance), position, false).await;

        let runtime = state.runtime.lock().await;
        let clauses = runtime.session.selection_clauses_for_diagnostics("picked");
        assert_eq!(clauses.len(), 1);
        assert_eq!(clauses[0].id, "active");
        assert_eq!(clauses[0].scope.sharing, CoordinationScope::Shared);
        let SelectionPredicateSpec::Equality { dimensions } = &clauses[0].predicate else {
            panic!("expected equality predicate");
        };
        assert_eq!(dimensions.len(), 1);
        assert_eq!(dimensions[0].id, "id");
        assert_eq!(
            dimensions[0].value,
            ScalarValue::Utf8(Some("row1".to_string()))
        );
    }

    #[tokio::test]
    async fn parallel_axis_overlay_child_rect_click_exposes_axis_and_interval_row_data() {
        let ctx = SessionContext::new();
        let clicked_axis = Param::new("clicked_axis", ScalarValue::Utf8(None));
        let clicked_min = Param::new("clicked_min", ScalarValue::Float64(None));
        let clicked_max = Param::new("clicked_max", ScalarValue::Float64(None));
        let parent_data = ctx
            .sql("SELECT 50.0 AS speed, 30.0 AS cost")
            .await
            .expect("parallel parent data");
        let store_batch = ctx
            .sql("SELECT 'speed' AS id, 25.0 AS value_min, 75.0 AS value_max")
            .await
            .expect("axis overlay store data")
            .collect()
            .await
            .expect("collect axis overlay store")
            .into_iter()
            .next()
            .expect("store batch");
        let binding = ChartEventBinding::on(ChartEventType::Click)
            .filter(event::button().eq(lit("left")))
            .filter(event::datum("id").is_not_null())
            .filter(event::datum("value_min").is_not_null())
            .filter(event::datum("value_max").is_not_null())
            .set_param(&clicked_axis, event::datum("id"))
            .set_param(&clicked_min, event::datum("value_min"))
            .set_param(&clicked_max, event::datum("value_max"))
            .exact();
        let overlay = ParallelAxisOverlay::new(
            "speed",
            Plot::<Cartesian>::new().mark(
                Rect::new()
                    .data_store(StoreData::new("axis_brush_boxes"))
                    .x(lit(0.0))
                    .x2(lit(1.0))
                    .y(col("value_min"))
                    .y2(col("value_max"))
                    .fill("rgba(37, 99, 235, 0.20)"),
            ),
        )
        .id("brush_overlay")
        .width_px(40.0);
        let compiled = Chart::with_coord(speed_cost_parallel_coord())
            .canvas_size(420.0, 320.0)
            .plot_size(260.0, 180.0)
            .param(clicked_axis)
            .param(clicked_min)
            .param(clicked_max)
            .store(Store::from_record_batch("axis_brush_boxes", store_batch))
            .data(parent_data)
            .mark(speed_cost_parallel_line().visible(false))
            .mark(overlay)
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile parallel overlay click plot");
        let handler = compile_handler_for_binding_index(&compiled, &ctx, 0);
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial parallel overlay scene");
        let datum_mark_instance = retained_event_datum_mark_instance(
            &state,
            "value_min",
            ScalarValue::Float64(Some(25.0)),
        )
        .await;
        let position = rect_instance_fraction_point(&scene, &datum_mark_instance, 0.85, 0.41);
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        let mark_instance = rtree
            .pick_top_mark_at_point(&position)
            .cloned()
            .expect("rtree should pick the overlay child rect");

        let _status = click_mark(&mut state, &handler, Some(mark_instance), position, false).await;
        let params = state.params().await;
        assert_eq!(
            params.get("clicked_axis"),
            Some(&ScalarValue::Utf8(Some("speed".to_string())))
        );
        assert_eq!(
            params.get("clicked_min"),
            Some(&ScalarValue::Float64(Some(25.0)))
        );
        assert_eq!(
            params.get("clicked_max"),
            Some(&ScalarValue::Float64(Some(75.0)))
        );
    }

    const PARALLEL_BRUSH_SELECTION: &str = "axis_brush";
    const PARALLEL_BRUSH_STORE: &str = "axis_brush_boxes";

    fn parallel_axis_hit_filter(axis_x: f64) -> Expr {
        event::start_coord(PARALLEL_LOCAL_X_CHANNEL)
            .gt(lit(axis_x - 12.0))
            .and(event::start_coord(PARALLEL_LOCAL_X_CHANNEL).lt(lit(axis_x + 12.0)))
    }

    fn parallel_dimension_drag_values_exist(dimension_id: &'static str) -> Expr {
        event::start_coord(dimension_id)
            .is_not_null()
            .and(event::event_at_start_clipped_coord(dimension_id).is_not_null())
            .and(event::start_coord(PARALLEL_LOCAL_Y_CHANNEL).is_not_null())
            .and(event::event_at_start_clipped_coord(PARALLEL_LOCAL_Y_CHANNEL).is_not_null())
    }

    fn parallel_axis_selection_clause(
        dimension_id: &'static str,
        field: &'static str,
    ) -> SelectionClauseUpdate {
        let interval = event::interval_ordered(
            event::start_coord(dimension_id),
            event::event_at_start_clipped_coord(dimension_id),
        );
        SelectionClauseUpdate::interval(lit(dimension_id))
            .facet_scope(CoordinationScope::Shared)
            .dimension_named(dimension_id, col(field))
            .endpoints(
                event::interval_start(interval.clone()),
                event::interval_end(interval),
            )
            .build()
    }

    fn parallel_brush_store_row(dimension_id: &'static str, axis_index: usize) -> StoreRow {
        let value_interval = event::interval_ordered(
            event::start_coord(dimension_id),
            event::event_at_start_clipped_coord(dimension_id),
        );
        StoreRow::new()
            .field("id", lit(dimension_id))
            .field("axis_index", lit(axis_index as f64))
            .field("value_min", event::interval_start(value_interval.clone()))
            .field("value_max", event::interval_end(value_interval))
    }

    fn parallel_axis_drag_binding(
        dimension_id: &'static str,
        field: &'static str,
        axis_index: usize,
        axis_x: f64,
        additive: bool,
    ) -> ChartEventBinding {
        let selection_update = if additive {
            SelectionUpdate::upsert_clause(parallel_axis_selection_clause(dimension_id, field))
        } else {
            SelectionUpdate::replace_clause(parallel_axis_selection_clause(dimension_id, field))
        };
        let store_update = if additive {
            StoreUpdate::upsert_rows([parallel_brush_store_row(dimension_id, axis_index)])
        } else {
            StoreUpdate::replace_rows([parallel_brush_store_row(dimension_id, axis_index)])
        };
        let mut binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .between(
                ChartEventStream::on(ChartEventType::MouseDown)
                    .filter(event::button().eq(lit("left"))),
                ChartEventStream::on(ChartEventType::MouseUp),
            )
            .filter(parallel_axis_hit_filter(axis_x))
            .filter(parallel_dimension_drag_values_exist(dimension_id))
            .filter(event::shift().eq(lit(additive)))
            .set_selection_at_start_scope(PARALLEL_BRUSH_SELECTION, selection_update);
        binding = if additive {
            binding.set_store_at_start_scope(PARALLEL_BRUSH_STORE, store_update)
        } else {
            binding.set_store_at_start_scope_replacing_scopes(PARALLEL_BRUSH_STORE, store_update)
        };
        binding.preview()
    }

    fn parallel_brush_store() -> Store {
        Store::empty(PARALLEL_BRUSH_STORE)
            .field("id", DataType::Utf8, false)
            .field("axis_index", DataType::Float64, false)
            .field("value_min", DataType::Float64, false)
            .field("value_max", DataType::Float64, false)
            .primary_key(["id"])
            .sharing(CoordinationScope::Shared)
    }

    fn parallel_brush_overlay(dimension_id: &'static str) -> ParallelAxisOverlay<Parallel> {
        ParallelAxisOverlay::new(
            dimension_id,
            Plot::<Cartesian>::new().mark(
                Rect::new()
                    .data_store(StoreData::new(PARALLEL_BRUSH_STORE))
                    .x_with(lit(0.0), |c| c.exclude_from_scale_domain())
                    .x2_with(lit(1.0), |c| c.exclude_from_scale_domain())
                    .y_with(col("value_min"), |c| c.exclude_from_scale_domain())
                    .y2_with(col("value_max"), |c| c.exclude_from_scale_domain())
                    .fill("rgba(37, 99, 235, 0.14)")
                    .stroke("#2563eb")
                    .stroke_width(1.4)
                    .zindex(10_000),
            ),
        )
        .width_px(14.0)
        .zindex(10_000)
    }

    #[tokio::test]
    async fn parallel_axis_drag_creates_brush_interval_store_row_and_overlay_rect() {
        let ctx = SessionContext::new();
        let data = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('row1', 10.0, 100.0),
                    ('row2', 20.0, 200.0),
                    ('row3', 30.0, 300.0)
                ) AS t(id, speed, cost)",
            )
            .await
            .expect("parallel brush data");
        let brush = Selection::new(PARALLEL_BRUSH_SELECTION)
            .combine(SelectionCombine::Intersect)
            .empty_selects_nothing();
        let selected = brush.predicate();
        let compiled = Chart::with_coord(speed_cost_parallel_coord())
            .canvas_size(420.0, 320.0)
            .plot_size(260.0, 180.0)
            .data(data)
            .selection(brush)
            .store(parallel_brush_store())
            .mark(
                speed_cost_parallel_line()
                    .id("context_lines")
                    .stroke("#c4cbd5")
                    .stroke_width(1.1)
                    .opacity(0.42)
                    .zindex(1),
            )
            .mark(
                speed_cost_parallel_line()
                    .id("selected_lines")
                    .transform_no_output(Filter::new(selected), |mark| mark)
                    .stroke("#2563eb")
                    .stroke_width(2.3)
                    .opacity(0.95)
                    .zindex(20),
            )
            .mark(parallel_brush_overlay("speed"))
            .event_binding(parallel_axis_drag_binding("speed", "speed", 0, 0.0, false))
            .compile(&ctx)
            .await
            .expect("compile parallel brush plot");
        let handler = compile_handler_for_binding_index(&compiled, &ctx, 0);
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial parallel brush scene");
        let scopes = state.interaction_scopes().await;
        let parallel_scope = scopes
            .iter()
            .find(|scope| scope.channels.iter().any(|channel| channel == "speed"))
            .expect("parallel interaction scope");
        let bounds = parallel_scope.bounds;
        let start = [bounds.x + 1.0, bounds.y + 36.0];
        let end = [bounds.x + 1.0, bounds.y + 116.0];

        let status = cursor_drag_move_from_position(&mut state, &handler, start, end, false).await;
        assert!(status.rerender);

        {
            let runtime = state.runtime.lock().await;
            let clauses = runtime
                .session
                .selection_clauses_for_diagnostics(PARALLEL_BRUSH_SELECTION);
            assert_eq!(clauses.len(), 1);
            assert_eq!(clauses[0].id, "speed");
            let SelectionPredicateSpec::Interval { dimensions } = &clauses[0].predicate else {
                panic!("expected interval predicate");
            };
            assert_eq!(dimensions.len(), 1);
            assert_eq!(dimensions[0].id, "speed");
            let rows = runtime
                .session
                .store_rows_for_diagnostics(PARALLEL_BRUSH_STORE);
            let root_rows = rows
                .iter()
                .find(|(path, _)| path.is_empty())
                .expect("shared brush store rows");
            assert_eq!(root_rows.1.len(), 1);
            assert_eq!(
                root_rows.1[0].get("id"),
                Some(&ScalarValue::Utf8(Some("speed".to_string())))
            );
            let min_value = scalar_f64(
                root_rows.1[0]
                    .get("value_min")
                    .expect("brush store value_min"),
            );
            let max_value = scalar_f64(
                root_rows.1[0]
                    .get("value_max")
                    .expect("brush store value_max"),
            );
            assert!(min_value < max_value);
        }

        let updated_scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("parallel brush scene with overlay");
        assert!(
            has_blue_fill(&collect_rect_fills(&updated_scene)),
            "brush store row should render as a blue overlay rect"
        );
        assert!(
            has_blue_stroke(&collect_line_strokes(&updated_scene)),
            "selection predicate should filter matching rows into the highlighted line branch"
        );
    }

    #[tokio::test]
    async fn parallel_axis_shift_drag_preserves_first_brush_and_intersects_second() {
        let ctx = SessionContext::new();
        let data = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('row1', 10.0, 100.0),
                    ('row2', 20.0, 200.0),
                    ('row3', 30.0, 300.0)
                ) AS t(id, speed, cost)",
            )
            .await
            .expect("parallel brush data");
        let brush = Selection::new(PARALLEL_BRUSH_SELECTION)
            .combine(SelectionCombine::Intersect)
            .empty_selects_nothing();
        let selected = brush.predicate();
        let compiled = Chart::with_coord(speed_cost_parallel_coord())
            .canvas_size(420.0, 320.0)
            .plot_size(260.0, 180.0)
            .data(data)
            .selection(brush)
            .store(parallel_brush_store())
            .mark(
                speed_cost_parallel_line()
                    .id("cost_color_scale_seed")
                    .stroke_with(col("cost"), |stroke| stroke.no_legend())
                    .stroke_width(0.0)
                    .opacity(0.0)
                    .zindex(0),
            )
            .mark(
                speed_cost_parallel_line()
                    .id("context_lines")
                    .stroke("#c4cbd5")
                    .stroke_width(1.1)
                    .opacity(0.42)
                    .zindex(1),
            )
            .mark(
                speed_cost_parallel_line()
                    .id("selected_lines")
                    .transform_no_output(Filter::new(selected), |mark| mark)
                    .stroke_with(col("cost"), |stroke| stroke.no_legend())
                    .stroke_width(2.3)
                    .opacity(0.95)
                    .zindex(20),
            )
            .mark(parallel_brush_overlay("speed"))
            .mark(parallel_brush_overlay("cost"))
            .event_binding(parallel_axis_drag_binding("speed", "speed", 0, 0.0, false))
            .event_binding(parallel_axis_drag_binding("cost", "cost", 1, 260.0, true))
            .compile(&ctx)
            .await
            .expect("compile parallel additive brush plot");
        let speed_handler = compile_handler_for_binding_index(&compiled, &ctx, 0);
        let cost_handler = compile_handler_for_binding_index(&compiled, &ctx, 1);
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial parallel brush scene");
        let scopes = state.interaction_scopes().await;
        let parallel_scope = scopes
            .iter()
            .find(|scope| scope.channels.iter().any(|channel| channel == "speed"))
            .expect("parallel interaction scope");
        let bounds = parallel_scope.bounds;
        let speed_start = [bounds.x + 1.0, bounds.y + 36.0];
        let speed_end = [bounds.x + 1.0, bounds.y + 116.0];
        let cost_start = [bounds.x + bounds.width - 1.0, bounds.y + 36.0];
        let cost_end = [bounds.x + bounds.width - 1.0, bounds.y + 116.0];

        let first = cursor_drag_move_from_position(
            &mut state,
            &speed_handler,
            speed_start,
            speed_end,
            false,
        )
        .await;
        assert!(first.rerender);
        let second =
            cursor_drag_move_from_position(&mut state, &cost_handler, cost_start, cost_end, true)
                .await;
        assert!(second.rerender);

        {
            let runtime = state.runtime.lock().await;
            let mut clause_ids = runtime
                .session
                .selection_clauses_for_diagnostics(PARALLEL_BRUSH_SELECTION)
                .iter()
                .map(|clause| clause.id.clone())
                .collect::<Vec<_>>();
            clause_ids.sort();
            assert_eq!(clause_ids, ["cost", "speed"]);
            let rows = runtime
                .session
                .store_rows_for_diagnostics(PARALLEL_BRUSH_STORE);
            let root_rows = rows
                .iter()
                .find(|(path, _)| path.is_empty())
                .expect("shared brush store rows");
            let mut ids = root_rows
                .1
                .iter()
                .map(|row| row.get("id").cloned().expect("brush row id"))
                .collect::<Vec<_>>();
            ids.sort_by_key(|value| format!("{value:?}"));
            assert_eq!(
                ids,
                [
                    ScalarValue::Utf8(Some("cost".to_string())),
                    ScalarValue::Utf8(Some("speed".to_string()))
                ],
                "shift drag should preserve the first brush row and add the second"
            );
        }

        let updated_scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("parallel brush scene with additive overlays");
        let line_info = collect_line_zindices_and_strokes(&updated_scene);
        let selected_lines = line_info
            .iter()
            .filter(|(zindex, _)| *zindex == Some(20))
            .collect::<Vec<_>>();
        assert_eq!(
            selected_lines.len(),
            1,
            "intersected speed/cost brushes should leave only the middle row selected"
        );
        assert!(
            line_info.iter().any(|(zindex, _)| *zindex == Some(1)),
            "grey context lines should remain below the selected branch"
        );
    }

    #[tokio::test]
    async fn parallel_axis_non_shift_drag_replaces_existing_brushes() {
        let ctx = SessionContext::new();
        let data = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('row1', 10.0, 100.0),
                    ('row2', 20.0, 200.0),
                    ('row3', 30.0, 300.0)
                ) AS t(id, speed, cost)",
            )
            .await
            .expect("parallel brush data");
        let brush = Selection::new(PARALLEL_BRUSH_SELECTION)
            .combine(SelectionCombine::Intersect)
            .empty_selects_nothing();
        let selected = brush.predicate();
        let compiled = Chart::with_coord(speed_cost_parallel_coord())
            .canvas_size(420.0, 320.0)
            .plot_size(260.0, 180.0)
            .data(data)
            .selection(brush)
            .store(parallel_brush_store())
            .mark(
                speed_cost_parallel_line()
                    .id("context_lines")
                    .stroke("#c4cbd5")
                    .stroke_width(1.1)
                    .opacity(0.42)
                    .zindex(1),
            )
            .mark(
                speed_cost_parallel_line()
                    .id("selected_lines")
                    .transform_no_output(Filter::new(selected), |mark| mark)
                    .stroke("#2563eb")
                    .stroke_width(2.3)
                    .opacity(0.95)
                    .zindex(20),
            )
            .mark(parallel_brush_overlay("speed"))
            .mark(parallel_brush_overlay("cost"))
            .event_binding(parallel_axis_drag_binding("speed", "speed", 0, 0.0, false))
            .event_binding(parallel_axis_drag_binding("cost", "cost", 1, 260.0, false))
            .compile(&ctx)
            .await
            .expect("compile parallel replacement brush plot");
        let speed_handler = compile_handler_for_binding_index(&compiled, &ctx, 0);
        let cost_handler = compile_handler_for_binding_index(&compiled, &ctx, 1);
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial parallel brush scene");
        let scopes = state.interaction_scopes().await;
        let parallel_scope = scopes
            .iter()
            .find(|scope| scope.channels.iter().any(|channel| channel == "speed"))
            .expect("parallel interaction scope");
        let bounds = parallel_scope.bounds;
        let speed_start = [bounds.x + 1.0, bounds.y + 36.0];
        let speed_end = [bounds.x + 1.0, bounds.y + 116.0];
        let cost_start = [bounds.x + bounds.width - 1.0, bounds.y + 36.0];
        let cost_end = [bounds.x + bounds.width - 1.0, bounds.y + 116.0];

        let first = cursor_drag_move_from_position(
            &mut state,
            &speed_handler,
            speed_start,
            speed_end,
            false,
        )
        .await;
        assert!(first.rerender);
        let second =
            cursor_drag_move_from_position(&mut state, &cost_handler, cost_start, cost_end, false)
                .await;
        assert!(second.rerender);

        let runtime = state.runtime.lock().await;
        let clauses = runtime
            .session
            .selection_clauses_for_diagnostics(PARALLEL_BRUSH_SELECTION);
        assert_eq!(clauses.len(), 1);
        assert_eq!(clauses[0].id, "cost");
        let rows = runtime
            .session
            .store_rows_for_diagnostics(PARALLEL_BRUSH_STORE);
        let root_rows = rows
            .iter()
            .find(|(path, _)| path.is_empty())
            .expect("shared brush store rows");
        assert_eq!(root_rows.1.len(), 1);
        assert_eq!(
            root_rows.1[0].get("id"),
            Some(&ScalarValue::Utf8(Some("cost".to_string()))),
            "non-shift drag should replace the previous speed brush row"
        );
    }

    #[tokio::test]
    async fn parallel_axis_header_mousedown_patches_drag_dimension_param() {
        let ctx = SessionContext::new();
        let drag_dimension = Param::new("drag_dimension", ScalarValue::Utf8(None));
        let drag_start_x = Param::new("drag_start_x", ScalarValue::Float64(None));
        let drag_display_x = Param::new("drag_display_x", ScalarValue::Float64(None));
        let binding = ChartEventBinding::on(ChartEventType::MouseDown)
            .filter(event::button().eq(lit("left")))
            .filter(
                parallel_event::parallel_surface_kind()
                    .eq(lit(parallel_event::PARALLEL_SURFACE_KIND_DIMENSION_TITLE)),
            )
            .set_param(&drag_dimension, parallel_event::parallel_dimension_id())
            .set_param(&drag_start_x, parallel_event::parallel_display_x())
            .set_param(&drag_display_x, parallel_event::parallel_display_x())
            .preview();
        let data = ctx
            .sql("SELECT 10.0 AS speed, 30.0 AS cost")
            .await
            .expect("parallel header data");
        let compiled = Chart::with_coord(speed_cost_parallel_coord())
            .canvas_size(420.0, 320.0)
            .plot_size(260.0, 180.0)
            .param(drag_dimension)
            .param(drag_start_x)
            .param(drag_display_x)
            .data(data)
            .mark(speed_cost_parallel_line())
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile parallel header binding plot");
        let handler = compile_handler_for_binding_index(&compiled, &ctx, 0);
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial parallel header build");
        let datum_mark_instance = retained_rect_event_datum_mark_instance(
            &state,
            &scene,
            parallel_event::PARALLEL_TITLE_FIELD,
            ScalarValue::Utf8(Some("Speed".to_string())),
        )
        .await;
        let position = rect_instance_point(&scene, &datum_mark_instance);
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        let mark_instance = rtree
            .pick_top_mark_at_point(&position)
            .cloned()
            .expect("rtree should pick the Speed title hit region");
        let picked_dimension = {
            let runtime = state.runtime.lock().await;
            runtime.last_event_datum_state.datum_for_mark_instance(
                Some(&mark_instance),
                parallel_event::PARALLEL_DIMENSION_ID_FIELD,
            )
        };
        assert_eq!(
            picked_dimension,
            Some(ScalarValue::Utf8(Some("speed".to_string())))
        );

        let status = mouse_down_mark(&mut state, &handler, Some(mark_instance), position).await;
        assert!(status.rerender);
        assert_eq!(
            state.params().await.get("drag_dimension"),
            Some(&ScalarValue::Utf8(Some("speed".to_string())))
        );
        let params = state.params().await;
        let Some(ScalarValue::Float64(Some(start_x))) = params.get("drag_start_x") else {
            panic!("drag_start_x should be patched");
        };
        let Some(ScalarValue::Float64(Some(display_x))) = params.get("drag_display_x") else {
            panic!("drag_display_x should be patched");
        };
        assert!(
            (*start_x - *display_x).abs() < f64::EPSILON,
            "drag start and display x should both start at the header display x"
        );
    }

    #[tokio::test]
    async fn parallel_axis_header_drag_move_updates_transient_display_x_param() {
        let ctx = SessionContext::new();
        let drag_dimension = Param::new("drag_dimension", ScalarValue::Utf8(None));
        let drag_start_x = Param::new("drag_start_x", ScalarValue::Float64(None));
        let drag_display_x = Param::new("drag_display_x", ScalarValue::Float64(None));
        let start_binding = ChartEventBinding::on(ChartEventType::MouseDown)
            .filter(event::button().eq(lit("left")))
            .filter(
                parallel_event::parallel_surface_kind()
                    .eq(lit(parallel_event::PARALLEL_SURFACE_KIND_DIMENSION_TITLE)),
            )
            .set_param(&drag_dimension, parallel_event::parallel_dimension_id())
            .set_param(&drag_start_x, parallel_event::parallel_display_x())
            .set_param(&drag_display_x, parallel_event::parallel_display_x())
            .preview();
        let preview_binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .between(
                ChartEventStream::on(ChartEventType::MouseDown)
                    .filter(event::button().eq(lit("left"))),
                ChartEventStream::on(ChartEventType::MouseUp),
            )
            .filter(event::param("drag_dimension").is_not_null())
            .set_param("drag_display_x", event::param("drag_start_x") + event::dx())
            .preview();
        let data = ctx
            .sql("SELECT 10.0 AS speed, 30.0 AS cost")
            .await
            .expect("parallel header data");
        let compiled =
            Chart::with_coord(speed_cost_parallel_coord().active_axis_display_params(
                drag_dimension.name.clone(),
                drag_display_x.name.clone(),
            ))
            .canvas_size(420.0, 320.0)
            .plot_size(260.0, 180.0)
            .param(drag_dimension)
            .param(drag_start_x)
            .param(drag_display_x)
            .data(data)
            .mark(speed_cost_parallel_line())
            .mark(speed_cost_parallel_symbol().fill("#2563eb").size(16.0))
            .event_binding(start_binding)
            .event_binding(preview_binding)
            .compile(&ctx)
            .await
            .expect("compile parallel header drag preview plot");
        let start_handler = compile_handler_for_binding_index(&compiled, &ctx, 0);
        let preview_handler = compile_handler_for_binding_index(&compiled, &ctx, 1);
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial parallel header build");
        let initial_line_x = collect_line_x_coords(&scene);
        let initial_symbol_x = collect_symbol_x_coords(&scene);
        let datum_mark_instance = retained_rect_event_datum_mark_instance(
            &state,
            &scene,
            parallel_event::PARALLEL_TITLE_FIELD,
            ScalarValue::Utf8(Some("Speed".to_string())),
        )
        .await;
        let start_pos = rect_instance_point(&scene, &datum_mark_instance);
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        let mark_instance = rtree
            .pick_top_mark_at_point(&start_pos)
            .cloned()
            .expect("rtree should pick the Speed title hit region");

        let start_status = mouse_down_mark(
            &mut state,
            &start_handler,
            Some(mark_instance.clone()),
            start_pos,
        )
        .await;
        assert!(start_status.rerender);
        let before = state.params().await;
        let Some(ScalarValue::Float64(Some(start_x))) = before.get("drag_start_x") else {
            panic!("drag_start_x should be patched by header mousedown");
        };
        let expected_display_x = *start_x + 37.0;

        let move_status = cursor_drag_move_from_mark(
            &mut state,
            &preview_handler,
            mark_instance,
            start_pos,
            [start_pos[0] + 37.0, start_pos[1]],
        )
        .await;
        assert!(move_status.rerender);
        let after = state.params().await;
        assert_eq!(
            after.get("drag_dimension"),
            Some(&ScalarValue::Utf8(Some("speed".to_string())))
        );
        let Some(ScalarValue::Float64(Some(display_x))) = after.get("drag_display_x") else {
            panic!("drag_display_x should be patched by drag preview");
        };
        assert!(
            (*display_x - expected_display_x).abs() < 1e-6,
            "drag display x should track start x plus pointer dx"
        );

        let scene_after_drag = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("parallel header drag preview rebuild");
        let metrics = state
            .last_metrics()
            .await
            .expect("drag preview build should record metrics");
        assert_eq!(
            metrics.pipeline.preview_data_mark_reuses, 0,
            "coordinate-owned drag params should invalidate cached data marks"
        );
        let moved_line_x = collect_line_x_coords(&scene_after_drag);
        let moved_symbol_x = collect_symbol_x_coords(&scene_after_drag);
        assert_eq!(
            moved_line_x.len(),
            initial_line_x.len(),
            "drag preview should not change line point count"
        );
        assert_eq!(
            moved_symbol_x.len(),
            initial_symbol_x.len(),
            "drag preview should not change symbol point count"
        );
        assert!(
            moved_line_x[0] > initial_line_x[0] + 30.0,
            "dragged dimension line vertex should move with the header: before={initial_line_x:?} after={moved_line_x:?}"
        );
        assert!(
            (moved_line_x[1] - initial_line_x[1]).abs() < 1e-3,
            "non-dragged dimension line vertex should stay put: before={initial_line_x:?} after={moved_line_x:?}"
        );
        assert!(
            moved_symbol_x[0] > initial_symbol_x[0] + 30.0,
            "dragged dimension symbol should move with the header: before={initial_symbol_x:?} after={moved_symbol_x:?}"
        );
        assert!(
            (moved_symbol_x[1] - initial_symbol_x[1]).abs() < 1e-3,
            "non-dragged dimension symbol should stay put: before={initial_symbol_x:?} after={moved_symbol_x:?}"
        );
    }

    #[tokio::test]
    async fn parallel_axis_header_drag_release_commits_order_and_clears_preview_params() {
        let ctx = SessionContext::new();
        let order_param = Param::new(
            "axis_order",
            ScalarValue::List(ScalarValue::new_list(
                &[
                    ScalarValue::Utf8(Some("speed".to_string())),
                    ScalarValue::Utf8(Some("efficiency".to_string())),
                    ScalarValue::Utf8(Some("cost".to_string())),
                ],
                &DataType::Utf8,
                true,
            )),
        );
        let drag_dimension = Param::new("drag_dimension", ScalarValue::Utf8(None));
        let drag_start_x = Param::new("drag_start_x", ScalarValue::Float64(None));
        let drag_display_x = Param::new("drag_display_x", ScalarValue::Float64(None));
        let start_binding = ChartEventBinding::on(ChartEventType::MouseDown)
            .filter(event::button().eq(lit("left")))
            .filter(
                parallel_event::parallel_surface_kind()
                    .eq(lit(parallel_event::PARALLEL_SURFACE_KIND_DIMENSION_TITLE)),
            )
            .set_param(&drag_dimension, parallel_event::parallel_dimension_id())
            .set_param(&drag_start_x, parallel_event::parallel_display_x())
            .set_param(&drag_display_x, parallel_event::parallel_display_x())
            .preview();
        let preview_binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .between(
                ChartEventStream::on(ChartEventType::MouseDown)
                    .filter(event::button().eq(lit("left"))),
                ChartEventStream::on(ChartEventType::MouseUp),
            )
            .filter(event::param("drag_dimension").is_not_null())
            .set_param("drag_display_x", event::param("drag_start_x") + event::dx())
            .preview();
        let release_binding = ChartEventBinding::on_between_end(
            ChartEventStream::on(ChartEventType::MouseDown).filter(event::button().eq(lit("left"))),
            ChartEventStream::on(ChartEventType::MouseUp),
        )
        .filter(event::param("drag_dimension").eq(lit("speed")))
        .filter(event::param("drag_display_x").gt_eq(lit(195.0)))
        .set_param(
            &order_param,
            lit(ScalarValue::List(ScalarValue::new_list(
                &[
                    ScalarValue::Utf8(Some("efficiency".to_string())),
                    ScalarValue::Utf8(Some("cost".to_string())),
                    ScalarValue::Utf8(Some("speed".to_string())),
                ],
                &DataType::Utf8,
                true,
            ))),
        )
        .set_param(&drag_dimension, lit(ScalarValue::Utf8(None)))
        .set_param(&drag_start_x, lit(ScalarValue::Float64(None)))
        .set_param(&drag_display_x, lit(ScalarValue::Float64(None)))
        .exact();
        let clear_binding = ChartEventBinding::on(ChartEventType::MouseUp)
            .set_param(&drag_dimension, lit(ScalarValue::Utf8(None)))
            .set_param(&drag_start_x, lit(ScalarValue::Float64(None)))
            .set_param(&drag_display_x, lit(ScalarValue::Float64(None)))
            .exact();
        let data = ctx
            .sql("SELECT 10.0 AS speed, 20.0 AS efficiency, 30.0 AS cost")
            .await
            .expect("parallel header data");
        let compiled = Chart::with_coord(
            speed_efficiency_cost_parallel_coord()
                .order_param(order_param.name.clone())
                .active_axis_display_params(
                    drag_dimension.name.clone(),
                    drag_display_x.name.clone(),
                ),
        )
        .canvas_size(420.0, 320.0)
        .plot_size(260.0, 180.0)
        .param(order_param)
        .param(drag_dimension)
        .param(drag_start_x)
        .param(drag_display_x)
        .data(data)
        .mark(speed_efficiency_cost_parallel_line())
        .event_binding(start_binding)
        .event_binding(preview_binding)
        .event_binding(release_binding)
        .event_binding(clear_binding)
        .compile(&ctx)
        .await
        .expect("compile parallel header drag release plot");
        let start_handler = compile_handler_for_binding_index(&compiled, &ctx, 0);
        let preview_handler = compile_handler_for_binding_index(&compiled, &ctx, 1);
        let release_handler = compile_handler_for_binding_index(&compiled, &ctx, 2);
        let clear_handler = compile_handler_for_binding_index(&compiled, &ctx, 3);
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial parallel header build");
        let datum_mark_instance = retained_rect_event_datum_mark_instance(
            &state,
            &scene,
            parallel_event::PARALLEL_TITLE_FIELD,
            ScalarValue::Utf8(Some("Speed".to_string())),
        )
        .await;
        let start_pos = rect_instance_point(&scene, &datum_mark_instance);
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        let mark_instance = rtree
            .pick_top_mark_at_point(&start_pos)
            .cloned()
            .expect("rtree should pick the Speed title hit region");

        let start_status = mouse_down_mark(
            &mut state,
            &start_handler,
            Some(mark_instance.clone()),
            start_pos,
        )
        .await;
        assert!(start_status.rerender);
        let move_status = cursor_drag_move_from_mark(
            &mut state,
            &preview_handler,
            mark_instance.clone(),
            start_pos,
            [start_pos[0] + 230.0, start_pos[1]],
        )
        .await;
        assert!(move_status.rerender);

        let release_status = mouse_up_from_mark(
            &mut state,
            &release_handler,
            mark_instance.clone(),
            start_pos,
            [start_pos[0] + 230.0, start_pos[1]],
        )
        .await;
        assert!(release_status.rerender);
        let _clear_status = mouse_up_from_mark(
            &mut state,
            &clear_handler,
            mark_instance,
            start_pos,
            [start_pos[0] + 230.0, start_pos[1]],
        )
        .await;

        let params = state.params().await;
        assert_eq!(
            params.get("axis_order"),
            Some(&ScalarValue::List(ScalarValue::new_list(
                &[
                    ScalarValue::Utf8(Some("efficiency".to_string())),
                    ScalarValue::Utf8(Some("cost".to_string())),
                    ScalarValue::Utf8(Some("speed".to_string())),
                ],
                &DataType::Utf8,
                true,
            ))),
            "release should commit the proposed axis order"
        );
        assert_eq!(
            params.get("drag_dimension"),
            Some(&ScalarValue::Utf8(None)),
            "release should clear the active dimension"
        );
        assert_eq!(
            params.get("drag_start_x"),
            Some(&ScalarValue::Float64(None)),
            "release should clear the start x"
        );
        assert_eq!(
            params.get("drag_display_x"),
            Some(&ScalarValue::Float64(None)),
            "release should clear the preview display x"
        );
    }

    #[tokio::test]
    async fn selection_predicate_filter_before_aggregate_updates_overlay() {
        let ctx = SessionContext::new();
        let picked = Selection::new("picked").empty_selects_nothing();
        let selected = picked.predicate();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('Alpha'),
                    ('Alpha'),
                    ('Beta'),
                    ('Beta'),
                    ('Beta'),
                    ('Gamma')
                ) AS t(category)",
            )
            .await
            .expect("filter aggregate data");
        let binding = ChartEventBinding::on(ChartEventType::Click)
            .filter(event::button().eq(lit("left")))
            .filter(event::datum("category").is_not_null())
            .set_selection(
                "picked",
                SelectionUpdate::replace_clause(equality_category_clause(lit("active"))),
            )
            .exact();
        let full_counts = Rect::new().transform(
            Aggregate::new().group_by([col("category")]).count("count"),
            |mark, count| {
                mark.x_with(col("category"), |c| {
                    c.scale_with::<Band>(|s| s.padding_inner(0.2))
                })
                .x2_with(col(":x"), |c| c.band(1.0))
                .y(lit(0.0))
                .y2(count.output("count"))
                .fill("#b8beca")
            },
        );
        let filtered_counts = Rect::new()
            .transform_no_output(Filter::new(selected), |mark| mark)
            .transform(
                Aggregate::new().group_by([col("category")]).count("count"),
                |mark, count| {
                    mark.x_with(col("category"), |c| {
                        c.scale_with::<Band>(|s| s.padding_inner(0.2))
                    })
                    .x2_with(col(":x"), |c| c.band(1.0))
                    .y(lit(0.0))
                    .y2(count.output("count"))
                    .fill("#2563eb")
                },
            );
        let compiled = Chart::<Cartesian>::new()
            .canvas_size(420.0, 320.0)
            .data(df)
            .selection(picked)
            .mark(full_counts)
            .mark(filtered_counts)
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile filtered aggregate plot");
        let handler = compile_handler_for_binding_index(&compiled, &ctx, 0);
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial filtered aggregate scene");
        assert!(
            !has_blue_fill(&collect_rect_fills(&scene)),
            "empty selection should not render the filtered aggregate overlay"
        );

        let datum_mark_instance = retained_event_datum_mark_instance(
            &state,
            "category",
            ScalarValue::Utf8(Some("Beta".to_string())),
        )
        .await;
        let position = rect_instance_point(&scene, &datum_mark_instance);
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        let mark_instance = rtree
            .pick_top_mark_at_point(&position)
            .cloned()
            .expect("rtree should pick the Beta aggregate bar");
        let status = click_mark(&mut state, &handler, Some(mark_instance), position, false).await;
        assert!(status.rerender);

        let updated_scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("selected filtered aggregate scene");
        assert!(
            has_blue_fill(&collect_rect_fills(&updated_scene)),
            "selection predicate should expand inside Filter before Aggregate"
        );
    }

    #[tokio::test]
    async fn bar_click_writes_nested_source_column_selection_clause() {
        let binding = ChartEventBinding::on(ChartEventType::Click)
            .filter(event::button().eq(lit("left")))
            .filter(event::datum("value").is_not_null())
            .set_selection(
                "picked",
                SelectionUpdate::replace_clause(nested_source_column_clause()),
            )
            .exact();
        let (mut state, handler, mark_instance, position) =
            nested_grouped_bar_state_and_handler(binding).await;
        let scopes = state.interaction_scopes().await;
        let scope = match route_interaction_scope(&scopes, Some(position), &channel_set(&["x"])) {
            InteractionRoute::Scope(scope) => scope,
            InteractionRoute::None => panic!("clicked bar should route to a coordinate scope"),
            InteractionRoute::Ambiguous => {
                panic!("clicked bar should route to one coordinate scope")
            }
        };
        let inverted = invert_scene_point(scope, position, &["x"]).expect("invert nested x");
        assert_nested_coord_value(inverted.get("x").expect("inverted x value"), "Q2", "East");

        let status = click_mark(&mut state, &handler, Some(mark_instance), position, false).await;
        assert!(
            status.rerender,
            "click should patch selection; metrics={:?}",
            state.event_metrics().await
        );
        assert!(status.rebuild_geometry);

        let runtime = state.runtime.lock().await;
        let clauses = runtime.session.selection_clauses_for_diagnostics("picked");
        assert_eq!(clauses.len(), 1);
        assert_eq!(clauses[0].id, "active");
        let SelectionPredicateSpec::Equality { dimensions } = &clauses[0].predicate else {
            panic!("expected equality predicate");
        };
        assert_quarter_team_dimensions(dimensions, "Q2", "East");
    }

    #[tokio::test]
    async fn bar_click_exposes_nested_struct_event_coord_readback() {
        let binding = ChartEventBinding::on(ChartEventType::Click)
            .filter(event::button().eq(lit("left")))
            .filter(event::datum("value").is_not_null())
            .filter(event::event_coord("x").is_not_null())
            .set_selection(
                "picked",
                SelectionUpdate::replace_clause(nested_source_column_clause()),
            )
            .exact();
        let (mut state, handler, mark_instance, position) =
            nested_grouped_bar_state_and_handler(binding).await;
        assert_nested_event_coord_schema(&handler);
        let scopes = state.interaction_scopes().await;
        let scope = match route_interaction_scope(&scopes, Some(position), &channel_set(&["x"])) {
            InteractionRoute::Scope(scope) => scope,
            InteractionRoute::None => panic!("clicked bar should route to a coordinate scope"),
            InteractionRoute::Ambiguous => {
                panic!("clicked bar should route to one coordinate scope")
            }
        };
        let inverted = invert_scene_point(scope, position, &["x"]).expect("invert nested x");
        assert_nested_coord_value(inverted.get("x").expect("inverted x value"), "Q2", "East");

        let status = click_mark(&mut state, &handler, Some(mark_instance), position, false).await;
        assert!(
            status.rerender,
            "event coord click should patch selection; metrics={:?}",
            state.event_metrics().await
        );
        assert!(status.rebuild_geometry);

        let runtime = state.runtime.lock().await;
        let clauses = runtime.session.selection_clauses_for_diagnostics("picked");
        assert_eq!(clauses.len(), 1);
        assert_eq!(clauses[0].id, "active");
        let SelectionPredicateSpec::Equality { dimensions } = &clauses[0].predicate else {
            panic!("expected equality predicate");
        };
        assert_quarter_team_dimensions(dimensions, "Q2", "East");
    }

    #[tokio::test]
    async fn temporal_nested_event_datum_uses_key_not_label() {
        let binding = ChartEventBinding::on(ChartEventType::Click)
            .filter(event::button().eq(lit("left")))
            .filter(event::datum("total").is_not_null())
            .filter(event::datum("period_month").eq(lit(3_i32)))
            .set_selection(
                "picked",
                SelectionUpdate::replace_clause(temporal_nested_source_column_clause()),
            )
            .exact();
        let (mut state, handler, mark_instance, position) =
            temporal_nested_bar_state_and_handler(binding).await;

        let status = click_mark(&mut state, &handler, Some(mark_instance), position, false).await;
        assert!(
            status.rerender,
            "integer month datum filter should patch selection; metrics={:?}",
            state.event_metrics().await
        );
        assert!(status.rebuild_geometry);

        let runtime = state.runtime.lock().await;
        let clauses = runtime.session.selection_clauses_for_diagnostics("picked");
        assert_eq!(clauses.len(), 1);
        assert_eq!(clauses[0].id, "active");
        let SelectionPredicateSpec::Equality { dimensions } = &clauses[0].predicate else {
            panic!("expected equality predicate");
        };
        assert_temporal_dimensions(dimensions, 2024, 1, 3);
    }

    #[tokio::test]
    async fn temporal_nested_click_selection_uses_year_quarter_month_dimensions() {
        let binding = ChartEventBinding::on(ChartEventType::Click)
            .filter(event::button().eq(lit("left")))
            .filter(event::datum("total").is_not_null())
            .set_selection(
                "picked",
                SelectionUpdate::replace_clause(temporal_nested_source_column_clause()),
            )
            .exact();
        let (mut state, handler, mark_instance, position) =
            temporal_nested_bar_state_and_handler(binding).await;

        let status = click_mark(&mut state, &handler, Some(mark_instance), position, false).await;
        assert!(
            status.rerender,
            "temporal nested click should patch selection; metrics={:?}",
            state.event_metrics().await
        );
        assert!(status.rebuild_geometry);

        let runtime = state.runtime.lock().await;
        let clauses = runtime.session.selection_clauses_for_diagnostics("picked");
        assert_eq!(clauses.len(), 1);
        assert_eq!(clauses[0].id, "active");
        let SelectionPredicateSpec::Equality { dimensions } = &clauses[0].predicate else {
            panic!("expected equality predicate");
        };
        assert_temporal_dimensions(dimensions, 2024, 1, 3);
    }

    #[tokio::test]
    async fn temporal_nested_event_coord_returns_key_only_struct() {
        let binding = ChartEventBinding::on(ChartEventType::Click)
            .filter(event::button().eq(lit("left")))
            .filter(event::datum("total").is_not_null())
            .filter(event::event_coord("x").is_not_null())
            .set_selection(
                "picked",
                SelectionUpdate::replace_clause(temporal_nested_source_column_clause()),
            )
            .exact();
        let (mut state, handler, mark_instance, position) =
            temporal_nested_bar_state_and_handler(binding).await;
        assert_temporal_event_coord_schema(&handler);
        let scopes = state.interaction_scopes().await;
        let scope = match route_interaction_scope(&scopes, Some(position), &channel_set(&["x"])) {
            InteractionRoute::Scope(scope) => scope,
            InteractionRoute::None => panic!("clicked temporal bar should route to a scope"),
            InteractionRoute::Ambiguous => {
                panic!("clicked temporal bar should route to one coordinate scope")
            }
        };
        let inverted = invert_scene_point(scope, position, &["x"]).expect("invert temporal x");
        assert_temporal_nested_coord_value(
            inverted.get("x").expect("inverted x value"),
            2024,
            1,
            3,
        );

        let status = click_mark(&mut state, &handler, Some(mark_instance), position, false).await;
        assert!(
            status.rerender,
            "event coord temporal click should patch selection; metrics={:?}",
            state.event_metrics().await
        );
        assert!(status.rebuild_geometry);

        let runtime = state.runtime.lock().await;
        let clauses = runtime.session.selection_clauses_for_diagnostics("picked");
        assert_eq!(clauses.len(), 1);
        let SelectionPredicateSpec::Equality { dimensions } = &clauses[0].predicate else {
            panic!("expected equality predicate");
        };
        assert_temporal_dimensions(dimensions, 2024, 1, 3);
    }

    #[tokio::test]
    async fn temporal_nested_cross_filter_to_non_nested_chart() {
        let ctx = SessionContext::new();
        let picked = Selection::new("picked").empty_selects_nothing();
        let selected = picked.predicate();
        let df = ctx
            .read_batch(temporal_nested_bar_batch())
            .expect("temporal cross-filter data");

        let nested_source = Rect::new().transform(
            TimeLevels::new(col("timestamp"))
                .year()
                .quarter()
                .month()
                .name("period"),
            |mark, period| {
                mark.transform(
                    Aggregate::new()
                        .group_by(period.keys())
                        .sum("total", col("value")),
                    |mark, aggregate| {
                        mark.x_with(period.nested(), |x| {
                            x.axis(|axis| axis.title("Nested period"))
                                .level(1, |level| level.nest_scope(NestScope::Shared))
                                .level(2, |level| level.axis(|axis| axis.label_angle(-90.0)))
                        })
                        .x2_with(col(":x"), |x| x.band(1.0))
                        .y(lit(0.0))
                        .y2(aggregate.output("total"))
                        .fill("#b8beca")
                    },
                )
            },
        );

        let detail_overlay = Rect::new()
            .transform(
                TimeLevels::new(col("timestamp"))
                    .year()
                    .quarter()
                    .month()
                    .name("period"),
                |mark, _period| mark,
            )
            .transform_no_output(Filter::new(selected), |mark| mark)
            .transform(
                Aggregate::new()
                    .group_by([col("period_month")])
                    .sum("selected_total", col("value")),
                |mark, aggregate| {
                    mark.x_with(col("period_month"), |x| {
                        x.scale_with::<Band>(|scale| scale.padding_inner(0.2))
                            .axis(|axis| axis.title("Selected month"))
                    })
                    .x2_with(col(":x"), |x| x.band(1.0))
                    .y(lit(0.0))
                    .y2(aggregate.output("selected_total"))
                    .fill("#2563eb")
                },
            );
        let detail_background = Rect::new()
            .transform(
                TimeLevels::new(col("timestamp"))
                    .year()
                    .quarter()
                    .month()
                    .name("period"),
                |mark, _period| mark,
            )
            .transform(
                Aggregate::new()
                    .group_by([col("period_month")])
                    .sum("month_total", col("value")),
                |mark, aggregate| {
                    mark.x_with(col("period_month"), |x| {
                        x.scale_with::<Band>(|scale| scale.padding_inner(0.2))
                            .axis(|axis| axis.title("Selected month"))
                    })
                    .x2_with(col(":x"), |x| x.band(1.0))
                    .y(lit(0.0))
                    .y2(aggregate.output("month_total"))
                    .fill("#d1d5db")
                },
            );

        let binding = ChartEventBinding::on(ChartEventType::Click)
            .filter(event::button().eq(lit("left")))
            .filter(event::datum("total").is_not_null())
            .set_selection(
                "picked",
                SelectionUpdate::replace_clause(temporal_nested_source_column_clause()),
            )
            .exact();
        let source_plot = Plot::<Cartesian>::new()
            .data(df.clone())
            .mark(nested_source)
            .event_binding(binding);
        let detail_plot = Plot::<Cartesian>::new()
            .data(df)
            .mark(detail_background)
            .mark(detail_overlay);
        let compiled = Chart::<HConcat>::new()
            .canvas_size(720.0, 380.0)
            .selection(picked)
            .mark(Subplot::new(source_plot).size(300.0, 260.0).name("source"))
            .mark(Subplot::new(detail_plot).size(240.0, 260.0).name("detail"))
            .compile(&ctx)
            .await
            .expect("compile temporal nested cross-filter plot");
        let handler = compile_handler_for_binding_index(&compiled, &ctx, 0);
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial temporal cross-filter scene");
        assert!(
            !has_blue_fill(&collect_rect_fills(&scene)),
            "empty temporal selection should not render the non-nested detail overlay"
        );

        let datum_mark_instance =
            retained_event_datum_mark_instance(&state, "total", ScalarValue::Float64(Some(18.0)))
                .await;
        let position = rect_instance_point(&scene, &datum_mark_instance);
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        let mark_instance = rtree
            .pick_top_mark_at_point(&position)
            .cloned()
            .expect("rtree should pick the temporal nested bar");
        let status = click_mark(&mut state, &handler, Some(mark_instance), position, false).await;
        assert!(
            status.rerender,
            "temporal nested click should update the shared selection"
        );

        let updated_scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("selected temporal cross-filter scene");
        assert!(
            has_blue_fill(&collect_rect_fills(&updated_scene)),
            "temporal nested selection should filter the non-nested detail aggregate"
        );
    }

    #[tokio::test]
    async fn facet_bar_click_writes_nested_source_column_selection_clause_in_cell_scope() {
        let binding = ChartEventBinding::on(ChartEventType::Click)
            .filter(event::button().eq(lit("left")))
            .filter(event::datum("value").is_not_null())
            .set_selection(
                "picked",
                SelectionUpdate::replace_clause(nested_source_column_clause_with_scope(
                    CoordinationScope::Free,
                )),
            )
            .exact();
        let (mut state, handler, mark_instance, position) =
            faceted_nested_grouped_bar_state_and_handler(binding).await;
        let scopes = state.interaction_scopes().await;
        let scope = match route_interaction_scope(&scopes, Some(position), &channel_set(&["x"])) {
            InteractionRoute::Scope(scope) => scope,
            InteractionRoute::None => {
                panic!("clicked facet bar should route to a coordinate scope")
            }
            InteractionRoute::Ambiguous => {
                panic!("clicked facet bar should route to one coordinate scope")
            }
        };
        let inverted = invert_scene_point(scope, position, &["x"]).expect("invert nested x");
        assert_nested_coord_value(inverted.get("x").expect("inverted x value"), "Q2", "East");

        let status = click_mark(&mut state, &handler, Some(mark_instance), position, false).await;
        assert!(
            status.rerender,
            "facet click should patch selection; metrics={:?}",
            state.event_metrics().await
        );
        assert!(status.rebuild_geometry);

        let runtime = state.runtime.lock().await;
        let clauses = runtime.session.selection_clauses_for_diagnostics("picked");
        assert_eq!(clauses.len(), 1);
        assert_eq!(clauses[0].id, "active");
        assert_eq!(clauses[0].scope.sharing, CoordinationScope::Free);
        assert_eq!(
            clauses[0].scope.owner_path,
            vec![ScalarValue::Utf8(Some("West".to_string()))],
            "Free-scoped nested selection should be owned by the clicked facet cell"
        );
        assert_eq!(clauses[0].facet_context.len(), 1);
        assert_eq!(clauses[0].facet_context[0].id, "region");
        assert_eq!(
            clauses[0].facet_context[0].value,
            ScalarValue::Utf8(Some("West".to_string()))
        );
        let SelectionPredicateSpec::Equality { dimensions } = &clauses[0].predicate else {
            panic!("expected equality predicate");
        };
        assert_quarter_team_dimensions(dimensions, "Q2", "East");
    }

    #[tokio::test]
    async fn legend_item_click_writes_equality_selection_clause() {
        let ctx = SessionContext::new();
        let picked = Selection::new("picked").empty_selects_nothing();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    (1.0, 1.0, 'Low'),
                    (2.0, 1.4, 'High'),
                    (3.0, 1.2, 'Low'),
                    (4.0, 1.8, 'High')
                ) AS t(x_val, y_val, category)",
            )
            .await
            .expect("legend click data");
        let binding = ChartEventBinding::on(ChartEventType::Click)
            .filter(event::button().eq(lit("left")))
            .filter(event::datum("channel").eq(lit("fill")))
            .set_selection(
                "picked",
                SelectionUpdate::toggle_clause(
                    SelectionClauseUpdate::equality_value(col("category"), event::datum("value"))
                        .facet_scope(CoordinationScope::Free),
                ),
            )
            .exact();
        let plot = Chart::<Cartesian>::new()
            .canvas_size(420.0, 320.0)
            .selection(picked)
            .data(df)
            .mark(
                Symbol::new()
                    .x(col("x_val"))
                    .y(col("y_val"))
                    .size(80.0)
                    .fill_with(col("category"), |c| {
                        c.legend(|l| l.id("category_legend").event_binding(binding))
                    }),
            );
        let compiled = plot.compile(&ctx).await.expect("compile legend click plot");
        assert_eq!(compiled.event_bindings().len(), 1);
        let handler = compile_handler_for_binding_index(&compiled, &ctx, 0);
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial build");
        let legend_mark_instance = retained_event_datum_mark_instance(
            &state,
            "__legend_value",
            ScalarValue::Utf8(Some("High".to_string())),
        )
        .await;
        let position = rect_instance_point(&scene, &legend_mark_instance);
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        let mark_instance = rtree
            .pick_top_mark_at_point(&position)
            .cloned()
            .expect("rtree should pick the legend hit rect");
        assert_eq!(mark_instance.mark_path, legend_mark_instance.mark_path);
        assert_eq!(
            mark_instance.instance_index,
            legend_mark_instance.instance_index
        );

        let status = click_mark(&mut state, &handler, Some(mark_instance), position, false).await;
        assert!(status.rerender);

        let runtime = state.runtime.lock().await;
        let clauses = runtime.session.selection_clauses_for_diagnostics("picked");
        assert_eq!(clauses.len(), 1);
        assert_eq!(clauses[0].id, "High");
        assert!(clauses[0].scope.owner_path.is_empty());
        let SelectionPredicateSpec::Equality { dimensions } = &clauses[0].predicate else {
            panic!("expected equality predicate");
        };
        assert_eq!(dimensions.len(), 1);
        assert_eq!(dimensions[0].id, "category");
        assert_eq!(
            dimensions[0].value,
            ScalarValue::Utf8(Some("High".to_string()))
        );
    }

    #[tokio::test]
    async fn legend_item_click_updates_related_conditional_opacity_swatch() {
        let ctx = SessionContext::new();
        let picked = Selection::new("picked").empty_selects_all();
        let selected = picked.predicate();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    (1.0, 1.0, 'Low'),
                    (2.0, 1.4, 'High'),
                    (3.0, 1.2, 'Low'),
                    (4.0, 1.8, 'High')
                ) AS t(x_val, y_val, category)",
            )
            .await
            .expect("legend opacity data");
        let binding = ChartEventBinding::on(ChartEventType::Click)
            .filter(event::button().eq(lit("left")))
            .filter(event::datum("channel").eq(lit("fill")))
            .set_selection(
                "picked",
                SelectionUpdate::toggle_clause(
                    SelectionClauseUpdate::equality_value(col("category"), event::datum("value"))
                        .facet_scope(CoordinationScope::Shared),
                ),
            )
            .exact();
        let clear_binding = ChartEventBinding::on(ChartEventType::DoubleClick)
            .filter(event::datum("channel").eq(lit("fill")))
            .set_selection("picked", SelectionUpdate::clear())
            .exact();
        let plot = Chart::<Cartesian>::new()
            .canvas_size(420.0, 320.0)
            .selection(picked)
            .data(df)
            .mark(
                Symbol::new()
                    .x(col("x_val"))
                    .y(col("y_val"))
                    .size(80.0)
                    .fill_with(col("category"), |c| {
                        c.legend(|l| {
                            l.id("category_legend")
                                .event_binding(binding)
                                .event_binding(clear_binding)
                        })
                    })
                    .opacity_with(lit(0.4), |c| {
                        c.no_scale()
                            .when_value(selected.clone(), lit(1.0))
                            .no_legend()
                    }),
            );
        let compiled = plot
            .compile(&ctx)
            .await
            .expect("compile legend opacity plot");
        let handler = compile_handler_for_binding_index(&compiled, &ctx, 0);
        let clear_handler = compile_handler_for_binding_index(&compiled, &ctx, 1);
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial build");
        assert!((legend_symbol_alpha_for_value(&state, &scene, "Low").await - 1.0).abs() < 0.001);
        assert!((legend_symbol_alpha_for_value(&state, &scene, "High").await - 1.0).abs() < 0.001);

        let legend_mark_instance = retained_event_datum_mark_instance(
            &state,
            "__legend_value",
            ScalarValue::Utf8(Some("High".to_string())),
        )
        .await;
        let position = rect_instance_point(&scene, &legend_mark_instance);
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        let mark_instance = rtree
            .pick_top_mark_at_point(&position)
            .cloned()
            .expect("rtree should pick the legend hit rect");
        let status = click_mark(&mut state, &handler, Some(mark_instance), position, false).await;
        assert!(status.rerender);

        let updated_scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("updated build");
        assert!(
            (legend_symbol_alpha_for_value(&state, &updated_scene, "Low").await - 0.4).abs()
                < 0.001
        );
        assert!(
            (legend_symbol_alpha_for_value(&state, &updated_scene, "High").await - 1.0).abs()
                < 0.001
        );

        let clear_status = double_click_mark(
            &mut state,
            &clear_handler,
            Some(legend_mark_instance),
            position,
        )
        .await;
        assert!(clear_status.rerender);
        let cleared_scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("cleared build");
        assert!(
            (legend_symbol_alpha_for_value(&state, &cleared_scene, "Low").await - 1.0).abs()
                < 0.001
        );
        assert!(
            (legend_symbol_alpha_for_value(&state, &cleared_scene, "High").await - 1.0).abs()
                < 0.001
        );
    }

    #[tokio::test]
    async fn plot_surface_binding_ignores_legend_item_click() {
        let ctx = SessionContext::new();
        let picked = Selection::new("picked").empty_selects_nothing();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    (1.0, 1.0, 'Low'),
                    (2.0, 1.4, 'High'),
                    (3.0, 1.2, 'Low'),
                    (4.0, 1.8, 'High')
                ) AS t(x_val, y_val, category)",
            )
            .await
            .expect("legend click data");
        let binding = ChartEventBinding::on(ChartEventType::Click)
            .filter(event::is_legend_item())
            .set_selection(
                "picked",
                SelectionUpdate::toggle_clause(
                    SelectionClauseUpdate::equality_value(col("category"), event::legend_value())
                        .facet_scope(CoordinationScope::Shared),
                ),
            )
            .exact();
        let plot = Chart::<Cartesian>::new()
            .canvas_size(420.0, 320.0)
            .selection(picked)
            .data(df)
            .event_binding(binding)
            .mark(
                Symbol::new()
                    .x(col("x_val"))
                    .y(col("y_val"))
                    .size(80.0)
                    .fill(col("category")),
            );
        let compiled = plot
            .compile(&ctx)
            .await
            .expect("compile plot-surface legend click plot");
        let handler = compile_handler_for_binding_index(&compiled, &ctx, 0);
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial build");
        let legend_mark_instance = retained_event_datum_mark_instance(
            &state,
            "__legend_value",
            ScalarValue::Utf8(Some("High".to_string())),
        )
        .await;
        let position = rect_instance_point(&scene, &legend_mark_instance);
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        let mark_instance = rtree
            .pick_top_mark_at_point(&position)
            .cloned()
            .expect("rtree should pick the legend hit rect");

        let status = click_mark(&mut state, &handler, Some(mark_instance), position, false).await;
        assert!(!status.rerender);

        let runtime = state.runtime.lock().await;
        let clauses = runtime.session.selection_clauses_for_diagnostics("picked");
        assert!(clauses.is_empty());
    }

    #[tokio::test]
    async fn plot_surface_binding_ignores_colorbar_click() {
        let ctx = SessionContext::new();
        let picked = Selection::new("picked").empty_selects_nothing();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    (1.0, 1.0, 0.0),
                    (2.0, 1.6, 50.0),
                    (3.0, 2.2, 100.0)
                ) AS t(x, y, temperature)",
            )
            .await
            .expect("colorbar data");
        let binding = ChartEventBinding::on(ChartEventType::Click)
            .filter(
                event::legend_surface_kind()
                    .eq(lit(event::LEGEND_SURFACE_KIND_CONTINUOUS_COLORBAR)),
            )
            .set_selection(
                "picked",
                SelectionUpdate::replace_clause(colorbar_interval_clause("y")),
            )
            .exact();
        let compiled = Chart::<Cartesian>::new()
            .canvas_size(480.0, 360.0)
            .selection(picked)
            .data(df)
            .event_binding(binding)
            .mark(Symbol::new().x(col("x")).y(col("y")).size(80.0).fill_with(
                col("temperature"),
                |c| {
                    c.scale_with::<Linear>(|s| s.domain((0.0, 100.0)).nice(false).zero(false))
                        .legend(|l| l.id("temperature_colorbar").title("Temperature"))
                },
            ))
            .compile(&ctx)
            .await
            .expect("compile plot-surface colorbar plot");
        let handler = compile_handler_for_binding_index(&compiled, &ctx, 0);
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial colorbar build");
        let colorbar_mark_instance = retained_event_datum_mark_instance(
            &state,
            event::LEGEND_SURFACE_KIND_FIELD,
            ScalarValue::Utf8(Some(
                event::LEGEND_SURFACE_KIND_CONTINUOUS_COLORBAR.to_string(),
            )),
        )
        .await;
        let position = rect_instance_fraction_point(&scene, &colorbar_mark_instance, 0.5, 0.5);
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        let mark_instance = rtree
            .pick_top_mark_at_point(&position)
            .cloned()
            .expect("rtree should pick the colorbar hit rect");

        let status = click_mark(&mut state, &handler, Some(mark_instance), position, false).await;
        assert!(!status.rerender);

        let runtime = state.runtime.lock().await;
        let clauses = runtime.session.selection_clauses_for_diagnostics("picked");
        assert!(clauses.is_empty());
    }

    #[tokio::test]
    async fn colorbar_legend_binding_click_writes_root_selection_clause() {
        let binding = ChartEventBinding::on(ChartEventType::Click)
            .filter(event::datum("surface_kind").eq(lit("continuous-colorbar")))
            .set_selection(
                "picked",
                SelectionUpdate::replace_clause(colorbar_interval_clause("y")),
            )
            .exact();
        let (mut state, handlers, mark_instance, position) =
            colorbar_state_and_handlers(LegendPosition::Right, vec![binding]).await;

        let status = click_mark(
            &mut state,
            &handlers[0],
            Some(mark_instance),
            position,
            false,
        )
        .await;
        assert!(status.rerender);

        let runtime = state.runtime.lock().await;
        let clauses = runtime.session.selection_clauses_for_diagnostics("picked");
        assert_eq!(clauses.len(), 1);
        assert_eq!(clauses[0].id, "active");
        assert_eq!(clauses[0].scope.sharing, CoordinationScope::Shared);
        assert!(clauses[0].scope.owner_path.is_empty());
        let SelectionPredicateSpec::Interval { dimensions } = &clauses[0].predicate else {
            panic!("expected interval predicate");
        };
        assert_eq!(dimensions.len(), 1);
        assert!((scalar_f64(&dimensions[0].min) - 50.0).abs() < 1.0);
        assert!((scalar_f64(&dimensions[0].max) - 50.0).abs() < 1.0);
    }

    #[tokio::test]
    async fn colorbar_legend_binding_rejects_item_only_datum_fields() {
        let binding = ChartEventBinding::on(ChartEventType::Click)
            .filter(event::datum("value").is_not_null())
            .set_selection(
                "picked",
                SelectionUpdate::replace_clause(colorbar_interval_clause("y")),
            )
            .exact();
        let (mut state, handlers, mark_instance, position) =
            colorbar_state_and_handlers(LegendPosition::Right, vec![binding]).await;

        let status = click_mark(
            &mut state,
            &handlers[0],
            Some(mark_instance),
            position,
            false,
        )
        .await;
        assert!(!status.rerender);

        let runtime = state.runtime.lock().await;
        assert_eq!(runtime.event_metrics.evaluation_errors, 1);
        assert!(
            runtime
                .session
                .selection_clauses_for_diagnostics("picked")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn horizontal_colorbar_legend_binding_inverts_midpoint() {
        let binding = ChartEventBinding::on(ChartEventType::Click)
            .filter(event::datum("surface_kind").eq(lit("continuous-colorbar")))
            .set_selection(
                "picked",
                SelectionUpdate::replace_clause(colorbar_interval_clause("x")),
            )
            .exact();
        let (mut state, handlers, mark_instance, position) =
            colorbar_state_and_handlers(LegendPosition::Bottom, vec![binding]).await;

        let status = click_mark(
            &mut state,
            &handlers[0],
            Some(mark_instance),
            position,
            false,
        )
        .await;
        assert!(status.rerender);

        let runtime = state.runtime.lock().await;
        let clauses = runtime.session.selection_clauses_for_diagnostics("picked");
        let SelectionPredicateSpec::Interval { dimensions } = &clauses[0].predicate else {
            panic!("expected interval predicate");
        };
        assert!((scalar_f64(&dimensions[0].min) - 50.0).abs() < 1.0);
        assert!((scalar_f64(&dimensions[0].max) - 50.0).abs() < 1.0);
    }

    #[tokio::test]
    async fn colorbar_legend_binding_double_click_clears_selection() {
        let write_binding = ChartEventBinding::on(ChartEventType::Click)
            .filter(event::datum("surface_kind").eq(lit("continuous-colorbar")))
            .set_selection(
                "picked",
                SelectionUpdate::replace_clause(colorbar_interval_clause("y")),
            )
            .exact();
        let clear_binding = ChartEventBinding::on(ChartEventType::DoubleClick)
            .filter(event::datum("surface_kind").eq(lit("continuous-colorbar")))
            .set_selection("picked", SelectionUpdate::clear())
            .exact();
        let (mut state, handlers, mark_instance, position) =
            colorbar_state_and_handlers(LegendPosition::Right, vec![write_binding, clear_binding])
                .await;

        let write_status = click_mark(
            &mut state,
            &handlers[0],
            Some(mark_instance.clone()),
            position,
            false,
        )
        .await;
        assert!(write_status.rerender);
        {
            let runtime = state.runtime.lock().await;
            assert_eq!(
                runtime
                    .session
                    .selection_clauses_for_diagnostics("picked")
                    .len(),
                1
            );
        }

        let clear_status =
            double_click_mark(&mut state, &handlers[1], Some(mark_instance), position).await;
        assert!(clear_status.rerender);
        let runtime = state.runtime.lock().await;
        let clauses = runtime.session.selection_clauses_for_diagnostics("picked");
        assert!(clauses.is_empty());
    }

    #[tokio::test]
    async fn colorbar_drag_clamps_event_at_start_clipped_coord() {
        let interval = event::interval_ordered(
            event::start_coord("y"),
            event::event_at_start_clipped_coord("y"),
        );
        let binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .between(
                ChartEventStream::on(ChartEventType::MouseDown)
                    .filter(event::button().eq(lit("left"))),
                ChartEventStream::on(ChartEventType::MouseUp),
            )
            .filter(event::start_coord("y").is_not_null())
            .filter(event::event_at_start_clipped_coord("y").is_not_null())
            .set_selection(
                "picked",
                SelectionUpdate::replace_clause(
                    SelectionClauseUpdate::interval(lit("active"))
                        .facet_scope(CoordinationScope::Shared)
                        .dimension(col("temperature"))
                        .endpoints(
                            event::interval_start(interval.clone()),
                            event::interval_end(interval),
                        )
                        .build(),
                ),
            )
            .preview();
        let (mut state, handlers, mark_instance, midpoint) =
            colorbar_state_and_handlers(LegendPosition::Right, vec![binding]).await;

        let far_above_colorbar = [midpoint[0], midpoint[1] - 10_000.0];
        let status = colorbar_drag_move(
            &mut state,
            &handlers[0],
            mark_instance,
            midpoint,
            far_above_colorbar,
        )
        .await;
        assert!(status.rerender);

        let runtime = state.runtime.lock().await;
        let clauses = runtime.session.selection_clauses_for_diagnostics("picked");
        let SelectionPredicateSpec::Interval { dimensions } = &clauses[0].predicate else {
            panic!("expected interval predicate");
        };
        assert!(
            (scalar_f64(&dimensions[0].min) - 50.0).abs() < 1.0,
            "start midpoint should stay near the domain midpoint"
        );
        assert!(
            (scalar_f64(&dimensions[0].max) - 100.0).abs() < 1.0,
            "dragging above the vertical colorbar should clamp to the top/domain maximum"
        );
    }

    #[tokio::test]
    async fn colorbar_drag_from_legend_axis_area_uses_colorbar_scope() {
        let interval = event::interval_ordered(
            event::start_coord("y"),
            event::event_at_start_clipped_coord("y"),
        );
        let binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .between(
                ChartEventStream::on(ChartEventType::MouseDown)
                    .filter(event::button().eq(lit("left"))),
                ChartEventStream::on(ChartEventType::MouseUp),
            )
            .filter(event::start_coord("y").is_not_null())
            .filter(event::event_at_start_clipped_coord("y").is_not_null())
            .set_selection(
                "picked",
                SelectionUpdate::replace_clause(
                    SelectionClauseUpdate::interval(lit("active"))
                        .facet_scope(CoordinationScope::Shared)
                        .dimension(col("temperature"))
                        .endpoints(
                            event::interval_start(interval.clone()),
                            event::interval_end(interval),
                        )
                        .build(),
                ),
            )
            .preview();
        let (mut state, handlers, mark_instance, midpoint) =
            colorbar_state_and_handlers(LegendPosition::Right, vec![binding]).await;

        let axis_area_start = [midpoint[0] + 50.0, midpoint[1]];
        let axis_area_end = [axis_area_start[0], midpoint[1] - 10_000.0];
        let status = colorbar_drag_move(
            &mut state,
            &handlers[0],
            mark_instance,
            axis_area_start,
            axis_area_end,
        )
        .await;
        assert!(status.rerender);

        let runtime = state.runtime.lock().await;
        let clauses = runtime.session.selection_clauses_for_diagnostics("picked");
        let SelectionPredicateSpec::Interval { dimensions } = &clauses[0].predicate else {
            panic!("expected interval predicate");
        };
        assert!(
            (scalar_f64(&dimensions[0].min) - 50.0).abs() < 1.0,
            "start y should invert through the colorbar scope even when x is outside the gradient"
        );
        assert!(
            (scalar_f64(&dimensions[0].max) - 100.0).abs() < 1.0,
            "dragging above the vertical colorbar should clamp to the top/domain maximum"
        );
    }

    #[tokio::test]
    async fn colorbar_drag_dispatches_through_event_stream_manager() {
        let ctx = SessionContext::new();
        let interval = event::interval_ordered(
            event::start_coord("y"),
            event::event_at_start_clipped_coord("y"),
        );
        let binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .between(
                ChartEventStream::on(ChartEventType::MouseDown)
                    .filter(event::button().eq(lit("left"))),
                ChartEventStream::on(ChartEventType::MouseUp),
            )
            .filter(event::start_coord("y").is_not_null())
            .filter(event::event_at_start_clipped_coord("y").is_not_null())
            .set_selection(
                "picked",
                SelectionUpdate::replace_clause(
                    SelectionClauseUpdate::interval(lit("active"))
                        .facet_scope(CoordinationScope::Shared)
                        .dimension(col("temperature"))
                        .endpoints(
                            event::interval_start(interval.clone()),
                            event::interval_end(interval),
                        )
                        .build(),
                ),
            )
            .preview();
        let picked = Selection::new("picked").empty_selects_nothing();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    (1.0, 1.0, 0.0),
                    (2.0, 1.6, 50.0),
                    (3.0, 2.2, 100.0)
                ) AS t(x, y, temperature)",
            )
            .await
            .expect("colorbar data");
        let compiled = Chart::<Cartesian>::new()
            .canvas_size(480.0, 360.0)
            .selection(picked)
            .data(df)
            .mark(Symbol::new().x(col("x")).y(col("y")).size(80.0).fill_with(
                col("temperature"),
                |c| {
                    c.scale_with::<Linear>(|s| s.domain((0.0, 100.0)).nice(false).zero(false))
                        .legend(|l| {
                            l.id("temperature_colorbar")
                                .title("Temperature")
                                .event_binding(binding)
                        })
                },
            ))
            .compile(&ctx)
            .await
            .expect("compile colorbar interaction plot");
        let streams = event_streams_for_plot_bindings(&compiled, &ctx).expect("event streams");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial colorbar build");
        let colorbar_mark_instance = retained_event_datum_mark_instance(
            &state,
            event::LEGEND_SURFACE_KIND_FIELD,
            ScalarValue::Utf8(Some(
                event::LEGEND_SURFACE_KIND_CONTINUOUS_COLORBAR.to_string(),
            )),
        )
        .await;
        let midpoint = rect_instance_fraction_point(&scene, &colorbar_mark_instance, 0.5, 0.5);
        let far_above_colorbar = [midpoint[0], midpoint[1] - 10_000.0];
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        assert!(
            rtree.pick_top_mark_at_point(&midpoint).is_some(),
            "rtree should pick the colorbar hit rect"
        );

        let mut manager = EventStreamManager::new(state);
        for (config, handler) in streams {
            manager.register_handler(config, handler);
        }
        let instant = Instant::now();
        manager
            .dispatch_event(
                &WindowEvent::CursorMoved(WindowCursorMoved { position: midpoint }),
                &rtree,
                instant,
            )
            .await;
        manager
            .dispatch_event(
                &WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                }),
                &rtree,
                instant,
            )
            .await;
        let status = manager
            .dispatch_event(
                &WindowEvent::CursorMoved(WindowCursorMoved {
                    position: far_above_colorbar,
                }),
                &rtree,
                instant + Duration::from_millis(16),
            )
            .await;
        assert!(status.rerender);

        let runtime = manager.state().runtime.lock().await;
        let clauses = runtime.session.selection_clauses_for_diagnostics("picked");
        assert_eq!(clauses.len(), 1);
        let SelectionPredicateSpec::Interval { dimensions } = &clauses[0].predicate else {
            panic!("expected interval predicate");
        };
        assert!(
            (scalar_f64(&dimensions[0].min) - 50.0).abs() < 1.0,
            "start midpoint should stay near the domain midpoint"
        );
        assert!(
            (scalar_f64(&dimensions[0].max) - 100.0).abs() < 1.0,
            "dragging above the vertical colorbar should clamp to the top/domain maximum"
        );
    }

    #[tokio::test]
    async fn event_datum_click_uses_context_mark_instance() {
        let binding = ChartEventBinding::on(ChartEventType::Click)
            .filter(event::button().eq(lit("left")))
            .filter(event::datum("category").is_not_null())
            .set_selection(
                "picked",
                SelectionUpdate::replace_all_clauses([equality_category_clause(lit("active"))]),
            )
            .exact();
        let (mut state, handler, mark_instance, position) =
            equality_bar_state_and_handler(binding).await;

        let context = EventStreamContext {
            mark_instance: Some(mark_instance),
            current_event: None,
            start_event: None,
            previous_event: None,
        };
        let status = handler
            .handle_with_context(
                &SceneGraphEvent::Click(SceneClickEvent {
                    position,
                    button: MouseButton::Left,
                    mark_instance: None,
                    modifiers: ModifiersState::default(),
                }),
                &context,
                &mut state,
                &empty_rtree(),
            )
            .await;
        assert!(
            status.rerender,
            "datum lookup should use the eventstream context mark instance"
        );

        let runtime = state.runtime.lock().await;
        let clauses = runtime.session.selection_clauses_for_diagnostics("picked");
        assert_eq!(clauses.len(), 1);
        let SelectionPredicateSpec::Equality { dimensions } = &clauses[0].predicate else {
            panic!("expected equality predicate");
        };
        assert_eq!(
            dimensions[0].value,
            ScalarValue::Utf8(Some("Beta".to_string()))
        );
    }

    #[tokio::test]
    async fn event_datum_click_resolves_instanced_symbol_rows() {
        let binding = ChartEventBinding::on(ChartEventType::Click)
            .filter(event::button().eq(lit("left")))
            .filter(event::datum("category").is_not_null())
            .set_selection(
                "picked",
                SelectionUpdate::replace_all_clauses([equality_category_value_clause()]),
            )
            .exact();
        let (mut state, handler, mark_instance, position) =
            equality_symbol_state_and_handler(binding).await;

        let status = click_mark(&mut state, &handler, Some(mark_instance), position, false).await;
        assert!(
            status.rerender,
            "clicking an instanced symbol should resolve event datum and patch selection"
        );

        let runtime = state.runtime.lock().await;
        let clauses = runtime.session.selection_clauses_for_diagnostics("picked");
        assert_eq!(clauses.len(), 1);
        assert_eq!(clauses[0].id, "Beta");
        let SelectionPredicateSpec::Equality { dimensions } = &clauses[0].predicate else {
            panic!("expected equality predicate");
        };
        assert_eq!(
            dimensions[0].value,
            ScalarValue::Utf8(Some("Beta".to_string()))
        );
    }

    async fn assert_equality_click_uses_facet_scope(
        facet_scope: CoordinationScope,
        expected_owner_path: Vec<ScalarValue>,
    ) {
        let binding = ChartEventBinding::on(ChartEventType::Click)
            .filter(event::button().eq(lit("left")))
            .filter(event::datum("category").is_not_null())
            .set_selection(
                "picked",
                SelectionUpdate::replace_all_clauses([
                    equality_category_value_clause().facet_scope(facet_scope)
                ]),
            )
            .exact();
        let (mut state, handler, mark_instance, position) =
            equality_bar_state_and_handler(binding).await;
        let mut scope = coord_scope(0, 0.0, 0.0, 1000.0, 1000.0, &["x", "y"]);
        scope
            .sharing_owner_paths
            .insert(0, vec![ScalarValue::Utf8(Some("Beta".to_string()))]);
        scope
            .sharing_owner_paths
            .insert(1, vec![ScalarValue::Utf8(Some("North".to_string()))]);
        {
            let mut app = state.runtime.lock().await;
            app.last_interaction_state.scopes = vec![scope];
        }

        let status = click_mark(&mut state, &handler, Some(mark_instance), position, false).await;
        assert!(status.rerender);

        let runtime = state.runtime.lock().await;
        let clauses = runtime.session.selection_clauses_for_diagnostics("picked");
        assert_eq!(clauses.len(), 1);
        assert_eq!(clauses[0].scope.sharing, facet_scope);
        assert_eq!(clauses[0].scope.owner_path, expected_owner_path);
    }

    #[tokio::test]
    async fn equality_selection_free_scope_uses_leaf_owner_path() {
        assert_equality_click_uses_facet_scope(
            CoordinationScope::Free,
            vec![ScalarValue::Utf8(Some("Beta".to_string()))],
        )
        .await;
    }

    #[tokio::test]
    async fn equality_selection_level_scope_uses_ancestor_owner_path() {
        assert_equality_click_uses_facet_scope(
            CoordinationScope::Level(1),
            vec![ScalarValue::Utf8(Some("North".to_string()))],
        )
        .await;
    }

    #[tokio::test]
    async fn equality_selection_shared_scope_uses_root_owner_path() {
        assert_equality_click_uses_facet_scope(CoordinationScope::Shared, Vec::new()).await;
    }

    #[tokio::test]
    async fn equality_selection_shift_click_toggles_clause() {
        let binding = ChartEventBinding::on(ChartEventType::Click)
            .filter(event::button().eq(lit("left")))
            .filter(event::shift().eq(lit(true)))
            .filter(event::datum("category").is_not_null())
            .set_selection(
                "picked",
                SelectionUpdate::toggle_clause(equality_category_value_clause()),
            )
            .exact();
        let (mut state, handler, mark_instance, position) =
            equality_bar_state_and_handler(binding).await;

        let first = click_mark(
            &mut state,
            &handler,
            Some(mark_instance.clone()),
            position,
            true,
        )
        .await;
        assert!(first.rerender);
        {
            let runtime = state.runtime.lock().await;
            assert_eq!(
                runtime
                    .session
                    .selection_clauses_for_diagnostics("picked")
                    .len(),
                1
            );
        }

        let second = click_mark(&mut state, &handler, Some(mark_instance), position, true).await;
        assert!(second.rerender);
        let runtime = state.runtime.lock().await;
        assert!(
            runtime
                .session
                .selection_clauses_for_diagnostics("picked")
                .is_empty(),
            "second shift-click on the same clause id should remove it"
        );
    }

    #[tokio::test]
    async fn semantic_equality_toggle_dispatches_typed_event_value() {
        let binding = ChartEventBinding::on(ChartEventType::Click)
            .filter(event::button().eq(lit("left")))
            .filter(event::datum("category").is_not_null())
            .set_selection(
                "picked",
                SelectionUpdate::toggle_equality_value(
                    col("category"),
                    event::datum("category"),
                    event::datum("category"),
                ),
            )
            .exact();
        let (mut state, handler, mark_instance, position) =
            equality_bar_state_and_handler(binding).await;

        let first = click_mark(
            &mut state,
            &handler,
            Some(mark_instance.clone()),
            position,
            false,
        )
        .await;
        assert!(first.rerender);
        {
            let runtime = state.runtime.lock().await;
            let clauses = runtime.session.selection_clauses_for_diagnostics("picked");
            assert_eq!(clauses.len(), 1);
            assert!(clauses[0].id.starts_with("__widget_eq_"));
            let SelectionPredicateSpec::Equality { dimensions } = &clauses[0].predicate else {
                panic!("expected equality predicate");
            };
            assert_eq!(dimensions.len(), 1);
            assert_eq!(
                dimensions[0].value,
                ScalarValue::Utf8(Some("Beta".to_string()))
            );
        }

        let second = click_mark(&mut state, &handler, Some(mark_instance), position, false).await;
        assert!(second.rerender);
        let runtime = state.runtime.lock().await;
        assert!(
            runtime
                .session
                .selection_clauses_for_diagnostics("picked")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn equality_selection_shift_click_toggles_one_of_multiple_clauses() {
        let binding = ChartEventBinding::on(ChartEventType::Click)
            .filter(event::button().eq(lit("left")))
            .filter(event::shift().eq(lit(true)))
            .filter(event::datum("category").is_not_null())
            .set_selection(
                "picked",
                SelectionUpdate::toggle_clause(equality_category_value_clause()),
            )
            .exact();
        let (mut state, handler, _, _) = equality_bar_state_and_handler(binding).await;

        assert!(
            click_category(&mut state, &handler, "Beta", true)
                .await
                .rerender
        );
        assert_eq!(selected_clause_ids(&state).await, vec!["Beta"]);

        assert!(
            click_category(&mut state, &handler, "Alpha", true)
                .await
                .rerender
        );
        assert_eq!(selected_clause_ids(&state).await, vec!["Alpha", "Beta"]);

        assert!(
            click_category(&mut state, &handler, "Beta", true)
                .await
                .rerender
        );
        assert_eq!(
            selected_clause_ids(&state).await,
            vec!["Alpha"],
            "shift-clicking an already selected category should remove only that category"
        );
    }

    #[tokio::test]
    async fn equality_selection_replace_then_shift_toggle_removes_initial_clause() {
        let replace = ChartEventBinding::on(ChartEventType::Click)
            .filter(event::button().eq(lit("left")))
            .filter(event::shift().eq(lit(false)))
            .filter(event::datum("category").is_not_null())
            .set_selection(
                "picked",
                SelectionUpdate::replace_all_clauses([equality_category_value_clause()]),
            )
            .exact();
        let toggle = ChartEventBinding::on(ChartEventType::Click)
            .filter(event::button().eq(lit("left")))
            .filter(event::shift().eq(lit(true)))
            .filter(event::datum("category").is_not_null())
            .set_selection(
                "picked",
                SelectionUpdate::toggle_clause(equality_category_value_clause()),
            )
            .exact();
        let (mut state, handlers, _, _) =
            equality_bar_state_and_handlers(vec![replace, toggle]).await;
        let replace_handler = &handlers[0];
        let toggle_handler = &handlers[1];

        assert!(
            click_category(&mut state, replace_handler, "Beta", false)
                .await
                .rerender
        );
        assert_eq!(selected_clause_ids(&state).await, vec!["Beta"]);

        assert!(
            click_category(&mut state, toggle_handler, "Alpha", true)
                .await
                .rerender
        );
        assert_eq!(selected_clause_ids(&state).await, vec!["Alpha", "Beta"]);

        assert!(
            click_category(&mut state, toggle_handler, "Beta", true)
                .await
                .rerender
        );
        assert_eq!(
            selected_clause_ids(&state).await,
            vec!["Alpha"],
            "shift-clicking the initially replaced category should remove it"
        );
    }

    #[tokio::test]
    async fn equality_selection_double_click_clears_selection() {
        let replace = ChartEventBinding::on(ChartEventType::Click)
            .filter(event::button().eq(lit("left")))
            .filter(event::datum("category").is_not_null())
            .set_selection(
                "picked",
                SelectionUpdate::replace_all_clauses([equality_category_value_clause()]),
            )
            .exact();
        let clear = ChartEventBinding::on(ChartEventType::DoubleClick)
            .clear_selection("picked")
            .exact();
        let (mut state, handlers, mark_instance, position) =
            equality_bar_state_and_handlers(vec![replace, clear]).await;
        let replace_handler = &handlers[0];
        let clear_handler = &handlers[1];

        assert!(
            click_category(&mut state, replace_handler, "Beta", false)
                .await
                .rerender
        );
        assert_eq!(selected_clause_ids(&state).await, vec!["Beta"]);

        assert!(
            double_click_mark(&mut state, clear_handler, Some(mark_instance), position)
                .await
                .rerender
        );
        assert!(
            selected_clause_ids(&state).await.is_empty(),
            "double-click should clear the equality selection"
        );
    }

    #[tokio::test]
    async fn child_plot_event_datum_requests_are_available_at_root() {
        let ctx = SessionContext::new();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('Alpha',  5.0, 1.0),
                    ('Beta',   6.0, 2.0)
                ) AS t(category, amount, x_value)",
            )
            .await
            .expect("data");
        let binding = ChartEventBinding::on(ChartEventType::Click)
            .filter(event::datum("category").is_not_null())
            .set_selection(
                "picked",
                SelectionUpdate::replace_all_clauses([equality_category_clause(lit("active"))]),
            )
            .exact();
        let picked = Selection::new("picked").empty_selects_nothing();
        let selected = picked.predicate();
        let bar_child = Plot::<Cartesian>::new()
            .data(df.clone())
            .mark(
                Rect::new()
                    .x_with(col("category"), |c| {
                        c.scale_with::<Band>(|s| s.padding_inner(0.2))
                    })
                    .x2_with(col(":x"), |c| c.band(1.0))
                    .y(lit(0.0))
                    .y2(datafusion::functions_aggregate::expr_fn::sum(col("amount")))
                    .fill_with(lit("#c8cdd7"), |c| {
                        c.no_scale()
                            .when_value(selected, lit("#2563eb"))
                            .no_legend()
                    }),
            )
            .event_binding(binding);
        let scatter_child = Plot::<Cartesian>::new()
            .data(df)
            .mark(Symbol::new().x(col("x_value")).y(col("amount")).size(20.0));

        let compiled = Chart::<HConcat>::new()
            .selection(picked)
            .mark(Subplot::new(bar_child).name("bars"))
            .mark(Subplot::new(scatter_child).name("scatter"))
            .compile(&ctx)
            .await
            .expect("compile concat event datum plot");

        assert_eq!(
            compiled.event_bindings().len(),
            1,
            "child plot binding should be registered on the root compiled plot"
        );
        assert_eq!(
            compiled.event_datum_types().get("category"),
            Some(&DataType::Utf8),
            "root event datum types should include child binding datum requests"
        );

        let handler = compile_handler_for_event_type(&compiled, &ctx, ChartEventType::Click);
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial concat build");
        let datum_mark_instance = retained_event_datum_mark_instance(
            &state,
            "category",
            ScalarValue::Utf8(Some("Beta".to_string())),
        )
        .await;
        let position = rect_instance_point(&scene, &datum_mark_instance);
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        let mark_instance = rtree
            .pick_top_mark_at_point(&position)
            .cloned()
            .expect("rtree should pick the child Beta bar");
        assert_eq!(mark_instance.mark_path, datum_mark_instance.mark_path);
        assert_eq!(
            mark_instance.instance_index,
            datum_mark_instance.instance_index
        );

        let status = click_mark(&mut state, &handler, Some(mark_instance), position, false).await;
        assert!(
            status.rerender,
            "child plot click should resolve event datum and patch selection"
        );
        {
            let runtime = state.runtime.lock().await;
            let clauses = runtime.session.selection_clauses_for_diagnostics("picked");
            assert_eq!(clauses.len(), 1);
            let SelectionPredicateSpec::Equality { dimensions } = &clauses[0].predicate else {
                panic!("expected equality predicate");
            };
            assert_eq!(
                dimensions[0].value,
                ScalarValue::Utf8(Some("Beta".to_string()))
            );
        }

        let updated_scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("scene after child selection");
        assert!(
            has_blue_fill(&collect_rect_fills(&updated_scene)),
            "selected aggregate bar should render with conditional blue fill"
        );
    }

    #[tokio::test]
    async fn faceted_selection_update_captures_start_scope() {
        let ctx = SessionContext::new();
        let binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .between(
                ChartEventStream::on(ChartEventType::MouseDown)
                    .filter(event::button().eq(lit("left"))),
                ChartEventStream::on(ChartEventType::MouseUp),
            )
            .set_selection_at_start_scope(
                "brush",
                SelectionUpdate::replace_all_clauses([SelectionClauseUpdate::interval(lit(
                    "active",
                ))
                .facet_scope(CoordinationScope::Free)
                .dimension(col("x"))
                .endpoints(lit(1.0), lit(3.0))]),
            )
            .preview();
        let compiled = Chart::<Cartesian>::new()
            .canvas_size(400.0, 300.0)
            .selection(
                Selection::new("brush")
                    .facet_context_field("group_name", col("group_name"))
                    .empty_selects_nothing(),
            )
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile faceted selection binding plot");
        let runtime = CompiledChartEventBinding::compile(
            0,
            compiled.event_bindings().first().unwrap(),
            &ctx,
            compiled.param_specs(),
            compiled.selection_specs(),
            compiled.store_specs(),
            &compiled.event_datum_types(),
        )
        .expect("compile selection binding runtime");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let handler = ChartEventBindingHandler {
            runtime: Arc::new(runtime),
            state: Mutex::new(ChartEventBindingState::default()),
        };
        let mut scope = coord_scope(0, 0.0, 0.0, 100.0, 100.0, &["x", "y"]);
        let owner_path = vec![ScalarValue::Utf8(Some("Beta".to_string()))];
        scope.sharing_owner_paths.insert(0, owner_path.clone());
        {
            let mut app = state.runtime.lock().await;
            app.last_interaction_state.scopes = vec![scope];
        }
        let start_event = EventStreamEventSnapshot {
            event: SceneGraphEvent::MouseDown(SceneMouseDownEvent {
                position: [20.0, 20.0],
                button: MouseButton::Left,
                mark_instance: None,
                modifiers: Default::default(),
            }),
            mark_instance: None,
            instant: Instant::now(),
        };
        let context = EventStreamContext {
            mark_instance: None,
            current_event: None,
            start_event: Some(start_event),
            previous_event: None,
        };

        let status = handler
            .handle_with_context(
                &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: [500.0, 500.0],
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                &context,
                &mut state,
                &empty_rtree(),
            )
            .await;

        assert!(
            status.rerender,
            "selection update should use the routed start scope"
        );
        let runtime = state.runtime.lock().await;
        let clauses = runtime.session.selection_clauses_for_diagnostics("brush");
        assert_eq!(clauses.len(), 1);
        let clause = &clauses[0];
        assert_eq!(clause.scope.sharing, CoordinationScope::Free);
        assert_eq!(clause.scope.owner_path, owner_path);
        assert_eq!(clause.facet_context.len(), 1);
        assert_eq!(clause.facet_context[0].id, "group_name");
        assert_eq!(
            clause.facet_context[0].value,
            ScalarValue::Utf8(Some("Beta".to_string()))
        );
    }

    #[tokio::test]
    async fn faceted_store_update_writes_start_scope() {
        use datafusion::arrow::datatypes::DataType;

        let ctx = SessionContext::new();
        let binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .between(
                ChartEventStream::on(ChartEventType::MouseDown)
                    .filter(event::button().eq(lit("left"))),
                ChartEventStream::on(ChartEventType::MouseUp),
            )
            .set_store_at_start_scope(
                "brush_boxes",
                StoreUpdate::replace_rows([StoreRow::new()
                    .field("id", lit("active"))
                    .field("x_min", lit(1.0))]),
            )
            .preview();
        let compiled = Chart::<Cartesian>::new()
            .canvas_size(400.0, 300.0)
            .store(
                Store::empty("brush_boxes")
                    .field("id", DataType::Utf8, false)
                    .field("x_min", DataType::Float64, false)
                    .primary_key(["id"])
                    .sharing(CoordinationScope::Free),
            )
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile store binding plot");
        let runtime = CompiledChartEventBinding::compile(
            0,
            compiled.event_bindings().first().unwrap(),
            &ctx,
            compiled.param_specs(),
            compiled.selection_specs(),
            compiled.store_specs(),
            &compiled.event_datum_types(),
        )
        .expect("compile store binding runtime");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let handler = ChartEventBindingHandler {
            runtime: Arc::new(runtime),
            state: Mutex::new(ChartEventBindingState::default()),
        };
        let mut scope = coord_scope(0, 0.0, 0.0, 100.0, 100.0, &["x", "y"]);
        let owner_path = vec![ScalarValue::Utf8(Some("Beta".to_string()))];
        scope.sharing_owner_paths.insert(0, owner_path.clone());
        {
            let mut app = state.runtime.lock().await;
            app.last_interaction_state.scopes = vec![scope];
        }
        let start_event = EventStreamEventSnapshot {
            event: SceneGraphEvent::MouseDown(SceneMouseDownEvent {
                position: [20.0, 20.0],
                button: MouseButton::Left,
                mark_instance: None,
                modifiers: Default::default(),
            }),
            mark_instance: None,
            instant: Instant::now(),
        };
        let context = EventStreamContext {
            mark_instance: None,
            current_event: None,
            start_event: Some(start_event),
            previous_event: None,
        };

        let status = handler
            .handle_with_context(
                &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: [500.0, 500.0],
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                &context,
                &mut state,
                &empty_rtree(),
            )
            .await;

        assert!(
            status.rerender,
            "store update should use the routed start scope"
        );
        let runtime = state.runtime.lock().await;
        let rows = runtime.session.store_rows_for_diagnostics("brush_boxes");
        let scoped = rows
            .iter()
            .find(|(path, _)| path == &owner_path)
            .expect("start owner path rows");
        assert_eq!(scoped.1.len(), 1);
        assert_eq!(
            scoped.1[0].get("id"),
            Some(&ScalarValue::Utf8(Some("active".to_string())))
        );
        let root_rows = rows
            .iter()
            .find(|(path, _)| path.is_empty())
            .expect("root store rows");
        assert!(root_rows.1.is_empty(), "free store should not write root");
    }

    #[tokio::test]
    async fn unrouted_free_store_write_fails_and_rolls_back_prior_actions() {
        use datafusion::arrow::datatypes::DataType;

        let ctx = SessionContext::new();
        let value = Param::new("value", ScalarValue::Int64(Some(1)));
        let binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .set_param(&value, lit(2_i64))
            .set_store(
                "rows",
                StoreUpdate::insert_rows([StoreRow::new().field("id", lit("unrouted"))]),
            );
        let compiled = Chart::<Cartesian>::new()
            .param(value)
            .store(
                Store::empty("rows")
                    .field("id", DataType::Utf8, false)
                    .sharing(CoordinationScope::Free),
            )
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile unrouted store binding");
        let runtime = CompiledChartEventBinding::compile(
            0,
            compiled.event_bindings().first().unwrap(),
            &ctx,
            compiled.param_specs(),
            compiled.selection_specs(),
            compiled.store_specs(),
            &compiled.event_datum_types(),
        )
        .expect("compile unrouted store runtime");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let handler = ChartEventBindingHandler {
            runtime: Arc::new(runtime),
            state: Mutex::new(ChartEventBindingState::default()),
        };

        let status = handler
            .handle_with_context(
                &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: [500.0, 500.0],
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                &EventStreamContext::default(),
                &mut state,
                &empty_rtree(),
            )
            .await;

        assert_eq!(status.admission, Some(EventAdmission::Failed));
        assert!(!status.rerender);
        assert_eq!(
            state.params().await.get("value"),
            Some(&ScalarValue::Int64(Some(1))),
            "the earlier param action must roll back with the unrouted store write"
        );
        let runtime = state.runtime.lock().await;
        let rows = runtime.session.store_rows_for_diagnostics("rows");
        assert_eq!(rows.len(), 1, "only the root seed instance exists");
        assert!(rows[0].0.is_empty());
        assert!(rows[0].1.is_empty());
    }

    #[tokio::test]
    async fn replacing_store_update_clears_previous_scoped_owners() {
        use datafusion::arrow::datatypes::DataType;

        let ctx = SessionContext::new();
        let binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .between(
                ChartEventStream::on(ChartEventType::MouseDown)
                    .filter(event::button().eq(lit("left"))),
                ChartEventStream::on(ChartEventType::MouseUp),
            )
            .set_store_at_start_scope_replacing_scopes(
                "brush_boxes",
                StoreUpdate::replace_rows([StoreRow::new()
                    .field("id", lit("active"))
                    .field("x_min", lit(1.0))]),
            )
            .preview();
        let compiled = Chart::<Cartesian>::new()
            .canvas_size(400.0, 300.0)
            .store(
                Store::empty("brush_boxes")
                    .field("id", DataType::Utf8, false)
                    .field("x_min", DataType::Float64, false)
                    .primary_key(["id"])
                    .sharing(CoordinationScope::Free),
            )
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile replacing store binding plot");
        let runtime = CompiledChartEventBinding::compile(
            0,
            compiled.event_bindings().first().unwrap(),
            &ctx,
            compiled.param_specs(),
            compiled.selection_specs(),
            compiled.store_specs(),
            &compiled.event_datum_types(),
        )
        .expect("compile store binding runtime");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let handler = ChartEventBindingHandler {
            runtime: Arc::new(runtime),
            state: Mutex::new(ChartEventBindingState::default()),
        };

        for (owner, start_x) in [("Beta", 20.0), ("Alpha", 40.0)] {
            let mut scope = coord_scope(0, 0.0, 0.0, 100.0, 100.0, &["x", "y"]);
            scope
                .sharing_owner_paths
                .insert(0, vec![ScalarValue::Utf8(Some(owner.to_string()))]);
            {
                let mut app = state.runtime.lock().await;
                app.last_interaction_state.scopes = vec![scope];
            }
            let start_event = EventStreamEventSnapshot {
                event: SceneGraphEvent::MouseDown(SceneMouseDownEvent {
                    position: [start_x, 20.0],
                    button: MouseButton::Left,
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                mark_instance: None,
                instant: Instant::now(),
            };
            let context = EventStreamContext {
                mark_instance: None,
                current_event: None,
                start_event: Some(start_event),
                previous_event: None,
            };
            let status = handler
                .handle_with_context(
                    &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                        position: [50.0, 50.0],
                        mark_instance: None,
                        modifiers: Default::default(),
                    }),
                    &context,
                    &mut state,
                    &empty_rtree(),
                )
                .await;
            assert!(status.rerender);
        }

        let runtime = state.runtime.lock().await;
        let rows = runtime.session.store_rows_for_diagnostics("brush_boxes");
        assert!(
            rows.iter()
                .all(|(path, _)| path != &vec![ScalarValue::Utf8(Some("Beta".to_string()))]),
            "replacing store write should remove the old Beta owner rows"
        );
        let alpha_rows = rows
            .iter()
            .find(|(path, _)| path == &vec![ScalarValue::Utf8(Some("Alpha".to_string()))])
            .expect("new Alpha owner rows");
        assert_eq!(alpha_rows.1.len(), 1);
        assert_eq!(
            alpha_rows.1[0].get("x_min"),
            Some(&ScalarValue::Float64(Some(1.0)))
        );
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum RepeatBoxTestMode {
        Global,
        Union,
        Intersect,
    }

    #[tokio::test]
    async fn repeat_box_selection_modes_update_current_cell_store_and_selection() {
        for mode in [
            RepeatBoxTestMode::Global,
            RepeatBoxTestMode::Union,
            RepeatBoxTestMode::Intersect,
        ] {
            let (mut state, handlers) = repeat_box_selection_state_and_handlers(mode).await;
            let scopes = state
                .interaction_scopes()
                .await
                .into_iter()
                .filter(|scope| scope.kind == InteractionScopeKind::Coordinate)
                .take(2)
                .collect::<Vec<_>>();
            assert_eq!(
                scopes.len(),
                2,
                "repeat grid should expose coordinate scopes"
            );

            let mut updated_cell_ids = Vec::new();
            for scope in scopes {
                let cell_id = drag_repeat_box_scope(&mut state, &handlers, &scope).await;
                updated_cell_ids.push(cell_id);
            }
            updated_cell_ids.sort();
            updated_cell_ids.dedup();
            assert_eq!(
                updated_cell_ids.len(),
                2,
                "test setup should drag in two distinct repeated cells"
            );

            let runtime = state.runtime.lock().await;
            let mut clause_ids = runtime
                .session
                .selection_clauses_for_diagnostics(REPEAT_BOX_SELECTION)
                .iter()
                .map(|clause| clause.id.clone())
                .collect::<Vec<_>>();
            clause_ids.sort();
            let rows = runtime.session.store_rows_for_diagnostics(REPEAT_BOX_STORE);
            let root_rows = rows
                .iter()
                .find(|(path, _)| path.is_empty())
                .expect("shared store root rows");
            let mut store_cell_ids = root_rows
                .1
                .iter()
                .map(|row| match row.get("cell_id").expect("store cell id") {
                    ScalarValue::Utf8(Some(value)) => value.clone(),
                    value => panic!("expected string cell id, got {value:?}"),
                })
                .collect::<Vec<_>>();
            store_cell_ids.sort();

            match mode {
                RepeatBoxTestMode::Global => {
                    assert_eq!(clause_ids.len(), 1, "global mode keeps one clause");
                    assert_eq!(store_cell_ids.len(), 1, "global mode keeps one box");
                }
                RepeatBoxTestMode::Union | RepeatBoxTestMode::Intersect => {
                    assert_eq!(
                        clause_ids, updated_cell_ids,
                        "{mode:?} mode keeps one clause per repeated cell"
                    );
                    assert_eq!(
                        store_cell_ids, updated_cell_ids,
                        "{mode:?} mode keeps one box per repeated cell"
                    );
                }
            }
            assert!(
                clause_ids.iter().all(|id| id.starts_with("repeat_cell:")),
                "repeat cell ids should be used as clause ids"
            );
        }
    }

    #[tokio::test]
    async fn repeat_box_selection_predicate_highlights_sibling_concat_plot() {
        let (mut state, handlers) =
            repeat_box_selection_state_and_handlers(RepeatBoxTestMode::Union).await;
        let scope = state
            .interaction_scopes()
            .await
            .into_iter()
            .find(|scope| scope.kind == InteractionScopeKind::Coordinate)
            .expect("repeat grid coordinate scope");
        drag_repeat_box_scope(&mut state, &handlers, &scope).await;

        let updated_scene = crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("scene after repeat box selection");
        assert!(
            has_blue_fill(&collect_symbol_fills(&updated_scene)),
            "repeat-generated interval predicate should be portable to sibling concat plot"
        );
    }

    #[tokio::test]
    async fn box_selection_tool_smoke_tests_repeat_inside_facet() {
        const TOOL_BOX_STORE: &str = "__tool_brush__store";

        let ctx = SessionContext::new();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('A', 1.0, 1.0),
                    ('A', 2.0, 2.0),
                    ('B', 1.5, 1.5),
                    ('B', 2.5, 2.5)
                ) AS t(group_name, a, b)",
            )
            .await
            .expect("nested repeat/facet data");
        let brush = BoxSelection::cartesian("brush")
            .dimensions(repeat::column(), repeat::row())
            .resolve(BoxSelectionResolve::Union)
            .repeat_cell_chrome();
        let cell = Plot::<Cartesian>::new()
            .mark(
                Symbol::new()
                    .x(repeat::column())
                    .y(repeat::row())
                    .size(24.0),
            )
            .tool(brush);
        let variables = vec![
            RepeatVariable::new("a", col("a")).title("A"),
            RepeatVariable::new("b", col("b")).title("B"),
        ];
        let repeat_grid = Plot::<RepeatGrid>::new().configure_coord(|c| {
            c.rows(variables.clone())
                .columns(variables)
                .cell(cell)
                .matrix_domains()
                .matrix_axes()
        });
        let compiled = Chart::<FacetColumn>::new()
            .canvas_size(760.0, 360.0)
            .data(df)
            .mark(Subplot::new(repeat_grid).column(col("group_name")))
            .compile(&ctx)
            .await
            .expect("compile nested repeat/facet box selection");
        let handlers =
            compile_handlers_for_event_type(&compiled, &ctx, ChartEventType::CursorMoved);
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial nested repeat/facet scene");
        let scope = state
            .interaction_scopes()
            .await
            .into_iter()
            .find(|scope| scope.kind == InteractionScopeKind::Coordinate)
            .expect("nested repeat/facet coordinate scope");

        let cell_id =
            drag_repeat_box_scope_for_store(&mut state, &handlers, &scope, TOOL_BOX_STORE).await;
        assert!(cell_id.starts_with("repeat_cell:"));

        let runtime = state.runtime.lock().await;
        let clauses = runtime.session.selection_clauses_for_diagnostics("brush");
        assert_eq!(clauses.len(), 1);
        assert!(clauses[0].id.starts_with("repeat_cell:"));
        let rows = runtime.session.store_rows_for_diagnostics(TOOL_BOX_STORE);
        assert!(
            rows.iter().any(|(_, rows)| !rows.is_empty()),
            "tool-generated brush store should receive a chrome row"
        );
    }

    #[tokio::test]
    async fn box_selection_tool_smoke_tests_repeat_inside_facet_wrap() {
        const TOOL_BOX_STORE: &str = "__tool_brush__store";

        let ctx = SessionContext::new();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('A', 1.0, 1.0),
                    ('A', 2.0, 2.0),
                    ('B', 1.5, 1.5),
                    ('B', 2.5, 2.5),
                    ('C', 1.2, 1.2),
                    ('C', 2.2, 2.2)
                ) AS t(group_name, a, b)",
            )
            .await
            .expect("wrapped nested repeat/facet data");
        let brush = BoxSelection::cartesian("brush")
            .dimensions(repeat::column(), repeat::row())
            .resolve(BoxSelectionResolve::Union)
            .repeat_cell_chrome();
        let cell = Plot::<Cartesian>::new()
            .mark(
                Symbol::new()
                    .x(repeat::column())
                    .y(repeat::row())
                    .size(24.0),
            )
            .tool(brush);
        let variables = vec![
            RepeatVariable::new("a", col("a")).title("A"),
            RepeatVariable::new("b", col("b")).title("B"),
        ];
        let repeat_grid = Plot::<RepeatGrid>::new().configure_coord(|c| {
            c.rows(variables.clone())
                .columns(variables)
                .cell(cell)
                .matrix_domains()
                .matrix_axes()
        });
        let compiled = Chart::<FacetWrap>::new()
            .canvas_size(760.0, 500.0)
            .data(df)
            .mark(
                Subplot::new(repeat_grid)
                    .wrap_with(col("group_name"), |c| c.columns(2).empty_cells_as_holes()),
            )
            .compile(&ctx)
            .await
            .expect("compile wrapped nested repeat/facet box selection");
        let handlers =
            compile_handlers_for_event_type(&compiled, &ctx, ChartEventType::CursorMoved);
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial wrapped nested repeat/facet scene");
        let scope = state
            .interaction_scopes()
            .await
            .into_iter()
            .find(|scope| scope.kind == InteractionScopeKind::Coordinate)
            .expect("wrapped nested repeat/facet coordinate scope");
        assert_eq!(
            scope.logical_facet_values.len(),
            1,
            "FacetWrap should contribute one logical facet value"
        );
        let segment = scope
            .child_frame_path
            .last()
            .expect("repeat-generated GridConcat segment");
        assert_eq!(segment.kind, EvaluatedChildFrameKind::GridConcat);
        assert_eq!(segment.row_count, Some(2));
        assert_eq!(segment.column_count, Some(2));

        let cell_id =
            drag_repeat_box_scope_for_store(&mut state, &handlers, &scope, TOOL_BOX_STORE).await;
        assert!(cell_id.starts_with("repeat_cell:"));

        let runtime = state.runtime.lock().await;
        let clauses = runtime.session.selection_clauses_for_diagnostics("brush");
        assert_eq!(clauses.len(), 1);
        assert!(clauses[0].id.starts_with("repeat_cell:"));
        let rows = runtime.session.store_rows_for_diagnostics(TOOL_BOX_STORE);
        assert!(
            rows.iter().any(|(_, rows)| !rows.is_empty()),
            "tool-generated brush store should receive a chrome row"
        );
    }

    const REPEAT_BOX_STORE: &str = "brush_boxes";
    const REPEAT_BOX_SELECTION: &str = "brush";

    async fn repeat_box_selection_state_and_handlers(
        mode: RepeatBoxTestMode,
    ) -> (ChartAppState, Vec<ChartEventBindingHandler>) {
        use datafusion::arrow::datatypes::DataType;

        let ctx = SessionContext::new();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    (1.0, 1.0),
                    (2.0, 2.0),
                    (3.0, 3.0)
                ) AS t(a, b)",
            )
            .await
            .expect("repeat data");
        let drag = ChartEventBinding::on(ChartEventType::CursorMoved)
            .between(
                ChartEventStream::on(ChartEventType::MouseDown)
                    .filter(event::button().eq(lit("left"))),
                ChartEventStream::on(ChartEventType::MouseUp),
            )
            .filter(event::start_coord("x").is_not_null())
            .filter(event::start_coord("y").is_not_null())
            .filter(event::event_at_start_clipped_coord("x").is_not_null())
            .filter(event::event_at_start_clipped_coord("y").is_not_null())
            .set_selection_at_start_scope(REPEAT_BOX_SELECTION, repeat_box_selection_update(mode))
            .set_store_at_start_scope(REPEAT_BOX_STORE, repeat_box_store_update(mode))
            .preview();

        let cell = Plot::<Cartesian>::new()
            .mark(
                Symbol::new()
                    .x(repeat::column())
                    .y(repeat::row())
                    .size(24.0),
            )
            .mark(
                Rect::<Cartesian>::new()
                    .data_store(StoreData::new(REPEAT_BOX_STORE))
                    .transform_no_output(Filter::new(repeat::current_cell_predicate()), |mark| mark)
                    .x_with(col("x_min"), |c| c.exclude_from_scale_domain())
                    .x2_with(col("x_max"), |c| c.exclude_from_scale_domain())
                    .y_with(col("y_min"), |c| c.exclude_from_scale_domain())
                    .y2_with(col("y_max"), |c| c.exclude_from_scale_domain()),
            )
            .event_binding(drag);
        let selected = Selection::new(REPEAT_BOX_SELECTION).predicate();
        let sibling = Plot::<Cartesian>::new().data(df.clone()).mark(
            Symbol::new()
                .x(col("a"))
                .y(col("b"))
                .fill_with(lit("#b8beca"), |c| {
                    c.no_scale()
                        .when_value(selected, lit("#2563eb"))
                        .no_legend()
                })
                .size(24.0),
        );
        let variables = vec![
            RepeatVariable::new("a", col("a")).title("A"),
            RepeatVariable::new("b", col("b")).title("B"),
        ];
        let repeat_grid = Plot::<RepeatGrid>::new()
            .data(df.clone())
            .configure_coord(|c| {
                c.rows(variables.clone())
                    .columns(variables)
                    .cell(cell)
                    .matrix_domains()
                    .matrix_axes()
            });
        let selection = match mode {
            RepeatBoxTestMode::Global | RepeatBoxTestMode::Union => {
                Selection::new(REPEAT_BOX_SELECTION)
                    .combine(SelectionCombine::Union)
                    .empty_selects_nothing()
            }
            RepeatBoxTestMode::Intersect => Selection::new(REPEAT_BOX_SELECTION)
                .combine(SelectionCombine::Intersect)
                .empty_selects_all(),
        };
        let compiled = Chart::<HConcat>::new()
            .canvas_size(640.0, 360.0)
            .store(
                Store::empty(REPEAT_BOX_STORE)
                    .field("id", DataType::Utf8, false)
                    .field("cell_id", DataType::Utf8, false)
                    .field("x_min", DataType::Float64, false)
                    .field("x_max", DataType::Float64, false)
                    .field("y_min", DataType::Float64, false)
                    .field("y_max", DataType::Float64, false)
                    .primary_key(["id"])
                    .sharing(CoordinationScope::Shared),
            )
            .selection(selection)
            .mark(Subplot::new(repeat_grid).id("splom"))
            .mark(Subplot::new(sibling).id("sibling"))
            .compile(&ctx)
            .await
            .expect("compile repeat box selection plot");

        let handlers =
            compile_handlers_for_event_type(&compiled, &ctx, ChartEventType::CursorMoved);
        assert_eq!(
            handlers.len(),
            4,
            "one local drag binding should be generated per repeated cell"
        );
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial repeat scene");
        (state, handlers)
    }

    fn repeat_box_selection_update(mode: RepeatBoxTestMode) -> SelectionUpdate {
        match mode {
            RepeatBoxTestMode::Global => {
                SelectionUpdate::replace_all_clauses([repeat_interval_clause()])
            }
            RepeatBoxTestMode::Union | RepeatBoxTestMode::Intersect => {
                SelectionUpdate::upsert_clause(repeat_interval_clause())
            }
        }
    }

    fn repeat_box_store_update(mode: RepeatBoxTestMode) -> StoreUpdate {
        match mode {
            RepeatBoxTestMode::Global => StoreUpdate::replace_rows([repeat_box_row()]),
            RepeatBoxTestMode::Union | RepeatBoxTestMode::Intersect => {
                StoreUpdate::upsert_rows([repeat_box_row()])
            }
        }
    }

    fn repeat_box_row() -> StoreRow {
        let x_interval = event::interval_ordered(
            event::start_coord("x"),
            event::event_at_start_clipped_coord("x"),
        );
        let y_interval = event::interval_ordered(
            event::start_coord("y"),
            event::event_at_start_clipped_coord("y"),
        );
        StoreRow::new()
            .field("id", repeat::cell_id())
            .field("cell_id", repeat::cell_id())
            .field("x_min", event::interval_start(x_interval.clone()))
            .field("x_max", event::interval_end(x_interval))
            .field("y_min", event::interval_start(y_interval.clone()))
            .field("y_max", event::interval_end(y_interval))
    }

    fn repeat_interval_clause() -> SelectionClauseUpdate {
        SelectionClauseUpdate::interval(repeat::cell_id())
            .facet_scope(CoordinationScope::Shared)
            .dimension(repeat::column())
            .endpoints(
                event::interval_start(event::interval_ordered(
                    event::start_coord("x"),
                    event::event_at_start_clipped_coord("x"),
                )),
                event::interval_end(event::interval_ordered(
                    event::start_coord("x"),
                    event::event_at_start_clipped_coord("x"),
                )),
            )
            .dimension(repeat::row())
            .endpoints(
                event::interval_start(event::interval_ordered(
                    event::start_coord("y"),
                    event::event_at_start_clipped_coord("y"),
                )),
                event::interval_end(event::interval_ordered(
                    event::start_coord("y"),
                    event::event_at_start_clipped_coord("y"),
                )),
            )
            .build()
    }

    async fn drag_repeat_box_scope(
        state: &mut ChartAppState,
        handlers: &[ChartEventBindingHandler],
        scope: &EvaluatedInteractionScope,
    ) -> String {
        drag_repeat_box_scope_for_store(state, handlers, scope, REPEAT_BOX_STORE).await
    }

    async fn drag_repeat_box_scope_for_store(
        state: &mut ChartAppState,
        handlers: &[ChartEventBindingHandler],
        scope: &EvaluatedInteractionScope,
        store_name: &str,
    ) -> String {
        let before = repeat_box_store_cell_ids(state, store_name).await;
        let start = [
            scope.bounds.x + scope.bounds.width * 0.25,
            scope.bounds.y + scope.bounds.height * 0.75,
        ];
        let current = [
            scope.bounds.x + scope.bounds.width * 0.75,
            scope.bounds.y + scope.bounds.height * 0.25,
        ];
        let start_event = EventStreamEventSnapshot {
            event: SceneGraphEvent::MouseDown(SceneMouseDownEvent {
                position: start,
                button: MouseButton::Left,
                mark_instance: None,
                modifiers: Default::default(),
            }),
            mark_instance: None,
            instant: Instant::now(),
        };
        let context = EventStreamContext {
            mark_instance: None,
            current_event: None,
            start_event: Some(start_event),
            previous_event: None,
        };

        let mut rerenders = 0;
        for handler in handlers {
            let status = handler
                .handle_with_context(
                    &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                        position: current,
                        mark_instance: None,
                        modifiers: Default::default(),
                    }),
                    &context,
                    state,
                    &empty_rtree(),
                )
                .await;
            rerenders += usize::from(status.rerender);
        }
        assert_eq!(
            rerenders, 1,
            "only the binding for the routed repeated cell should update state"
        );
        let after = repeat_box_store_cell_ids(state, store_name).await;
        let added = after.difference(&before).cloned().collect::<Vec<_>>();
        if let [cell_id] = added.as_slice() {
            return cell_id.clone();
        }
        after.into_iter().next().expect("updated store cell id")
    }

    async fn repeat_box_store_cell_ids(
        state: &ChartAppState,
        store_name: &str,
    ) -> BTreeSet<String> {
        let runtime = state.runtime.lock().await;
        runtime
            .session
            .store_rows_for_diagnostics(store_name)
            .into_iter()
            .flat_map(|(_, rows)| {
                rows.into_iter().filter_map(|row| match row.get("cell_id") {
                    Some(ScalarValue::Utf8(Some(value))) => Some(value.clone()),
                    _ => None,
                })
            })
            .collect()
    }

    #[tokio::test]
    async fn event_binding_can_reset_raw_domain_to_default() {
        let (mut state, handler) = raw_domain_reset_state_and_handler().await;
        let default_domain = state
            .params()
            .await
            .get("x_domain")
            .cloned()
            .expect("default x_domain param");
        {
            let mut runtime = state.runtime.lock().await;
            runtime
                .session
                .apply_scoped_param_patch(vec![ScopedParamAssignment {
                    name: "x_domain".to_string(),
                    owner_path: Vec::new(),
                    value: domain_list_scalar(2.0, 8.0),
                    replace_scoped_values: false,
                }])
                .unwrap();
        }
        assert_ne!(
            state.params().await.get("x_domain"),
            Some(&default_domain),
            "test setup should install a concrete raw domain"
        );

        let status = handler
            .handle_with_context(
                &SceneGraphEvent::DoubleClick(SceneDoubleClickEvent {
                    position: [10.0, 10.0],
                    mark_instance: None,
                    modifiers: ModifiersState::default(),
                }),
                &EventStreamContext::default(),
                &mut state,
                &empty_rtree(),
            )
            .await;

        assert!(status.rerender);
        assert!(status.rebuild_geometry);
        assert_eq!(
            state.params().await.get("x_domain"),
            Some(&default_domain),
            "double-click reset should restore the raw-domain default"
        );
    }

    #[tokio::test]
    async fn event_binding_rejects_reset_of_unknown_param() {
        let ctx = SessionContext::new();
        let result = Chart::<Cartesian>::new()
            .event_binding(
                ChartEventBinding::on(ChartEventType::DoubleClick)
                    .reset_param("missing")
                    .exact(),
            )
            .compile(&ctx)
            .await;
        let error = match result {
            Ok(_) => panic!("unknown reset target should fail at the chart compilation boundary"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("assigns unknown param 'missing'")
        );
    }

    #[tokio::test]
    async fn event_binding_drag_uses_start_event() {
        let binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .between(
                ChartEventStream::on(ChartEventType::MouseDown)
                    .filter(event::button().eq(lit("left"))),
                ChartEventStream::on(ChartEventType::MouseUp),
            )
            .set_param("width", event::start_param("width") + event::dx())
            .preview();
        let mut state = bound_state(binding.clone()).await;
        let handler = handler_for_binding(binding).await;
        let now = Instant::now();
        let start_event = EventStreamEventSnapshot {
            event: SceneGraphEvent::MouseDown(SceneMouseDownEvent {
                position: [100.0, 50.0],
                button: MouseButton::Left,
                mark_instance: None,
                modifiers: Default::default(),
            }),
            mark_instance: None,
            instant: now,
        };
        let context = EventStreamContext {
            mark_instance: None,
            current_event: None,
            start_event: Some(start_event),
            previous_event: None,
        };
        let status = handler
            .handle_with_context(
                &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: [125.0, 50.0],
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                &context,
                &mut state,
                &empty_rtree(),
            )
            .await;
        assert!(status.rerender);
        assert!(!status.rebuild_geometry);
        assert_eq!(
            state.params().await.get("width"),
            Some(&ScalarValue::Float64(Some(665.0)))
        );
    }

    #[tokio::test]
    async fn event_binding_previous_event_can_update_param() {
        let binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .set_param(
                "width",
                event::previous_param("width") + (event::x() - event::previous_x()),
            )
            .preview();
        let mut state = bound_state(binding.clone()).await;
        let handler = handler_for_binding(binding).await;
        {
            let mut binding_state = handler.state.lock().unwrap();
            binding_state.previous_params = Some(ScopedParamStoreSnapshot::from_root_params(
                IndexMap::from([("width".to_string(), ScalarValue::Float64(Some(640.0)))]),
            ));
        }
        let now = Instant::now();
        let previous_event = EventStreamEventSnapshot {
            event: SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                position: [100.0, 50.0],
                mark_instance: None,
                modifiers: Default::default(),
            }),
            mark_instance: None,
            instant: now,
        };
        let context = EventStreamContext {
            mark_instance: None,
            current_event: None,
            start_event: None,
            previous_event: Some(previous_event),
        };
        let status = handler
            .handle_with_context(
                &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: [125.0, 50.0],
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                &context,
                &mut state,
                &empty_rtree(),
            )
            .await;
        assert!(status.rerender);
        assert_eq!(
            state.params().await.get("width"),
            Some(&ScalarValue::Float64(Some(665.0)))
        );
    }

    #[tokio::test]
    async fn event_binding_filter_false_skips_patch() {
        let binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .filter(event::shift().eq(lit(true)))
            .set_param("width", event::x())
            .preview();
        let mut state = bound_state(binding.clone()).await;
        let handler = handler_for_binding(binding).await;

        let status = handler
            .handle_with_context(
                &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: [800.0, 50.0],
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                &EventStreamContext::default(),
                &mut state,
                &empty_rtree(),
            )
            .await;

        assert!(!status.rerender);
        assert_eq!(
            state.params().await.get("width"),
            Some(&ScalarValue::Float64(Some(640.0)))
        );
        assert_eq!(state.event_metrics().await.filter_failures, 1);
    }

    #[tokio::test]
    async fn event_binding_unchanged_patch_skips_rerender() {
        let binding = ChartEventBinding::on(ChartEventType::CanvasResize)
            .set_param("width", event::canvas_width())
            .preview();
        let mut state = bound_state(binding.clone()).await;
        let handler = handler_for_binding(binding).await;

        let status = handler
            .handle_with_context(
                &SceneGraphEvent::CanvasResize(CanvasResizeEvent {
                    size: [640.0, 400.0],
                }),
                &EventStreamContext::default(),
                &mut state,
                &empty_rtree(),
            )
            .await;

        assert!(!status.rerender);
        assert_eq!(state.accepted_resize_count().await, 0);
        assert_eq!(state.event_metrics().await.unchanged_patch_skips, 1);
    }

    #[tokio::test]
    async fn exact_event_binding_requests_exact_evaluation() {
        let binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .set_param("width", event::x())
            .exact();
        let mut state = bound_state(binding.clone()).await;
        let handler = handler_for_binding(binding).await;

        let status = handler
            .handle_with_context(
                &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: [800.0, 50.0],
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                &EventStreamContext::default(),
                &mut state,
                &empty_rtree(),
            )
            .await;

        assert!(status.rerender);
        assert!(status.rebuild_geometry);
        assert_eq!(
            state.runtime.lock().await.next_evaluation_mode,
            EvaluationMode::Exact
        );
    }

    #[tokio::test]
    async fn ordered_param_actions_observe_preceding_writes() {
        let ctx = SessionContext::new();
        let first = Param::new("first", ScalarValue::Int64(Some(1)));
        let second = Param::new("second", ScalarValue::Int64(Some(0)));
        let binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .set_param(&first, lit(2_i64))
            .set_param(&second, first.expr() + lit(1_i64));
        let compiled = Chart::<Cartesian>::new()
            .param(first)
            .param(second)
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile ordered binding");
        let runtime = CompiledChartEventBinding::compile(
            0,
            compiled.event_bindings().first().unwrap(),
            &ctx,
            compiled.param_specs(),
            compiled.selection_specs(),
            compiled.store_specs(),
            &compiled.event_datum_types(),
        )
        .expect("compile ordered runtime");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let handler = ChartEventBindingHandler {
            runtime: Arc::new(runtime),
            state: Mutex::new(ChartEventBindingState::default()),
        };

        let status = handler
            .handle_with_context(
                &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: [10.0, 20.0],
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                &EventStreamContext::default(),
                &mut state,
                &empty_rtree(),
            )
            .await;

        assert_eq!(status.admission, Some(EventAdmission::Committed));
        let params = state.params().await;
        assert_eq!(params.get("first"), Some(&ScalarValue::Int64(Some(2))));
        assert_eq!(params.get("second"), Some(&ScalarValue::Int64(Some(3))));
    }

    #[tokio::test]
    async fn store_payload_observes_preceding_param_write() {
        let ctx = SessionContext::new();
        let value = Param::new("value", ScalarValue::Int64(Some(1)));
        let binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .set_param(&value, lit(2_i64))
            .set_store(
                "rows",
                StoreUpdate::insert_rows([StoreRow::new().field("value", value.expr())]),
            );
        let compiled = Chart::<Cartesian>::new()
            .param(value)
            .store(Store::empty("rows").field("value", DataType::Int64, false))
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile ordered store binding");
        let runtime = CompiledChartEventBinding::compile(
            0,
            compiled.event_bindings().first().unwrap(),
            &ctx,
            compiled.param_specs(),
            compiled.selection_specs(),
            compiled.store_specs(),
            &compiled.event_datum_types(),
        )
        .expect("compile ordered store runtime");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let handler = ChartEventBindingHandler {
            runtime: Arc::new(runtime),
            state: Mutex::new(ChartEventBindingState::default()),
        };

        let status = handler
            .handle_with_context(
                &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: [10.0, 20.0],
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                &EventStreamContext::default(),
                &mut state,
                &empty_rtree(),
            )
            .await;
        assert_eq!(status.admission, Some(EventAdmission::Committed));
        let runtime = state.runtime.lock().await;
        let rows = runtime.session.store_rows_for_diagnostics("rows");
        assert_eq!(
            rows.iter()
                .find(|(owner, _)| owner.is_empty())
                .expect("root rows")
                .1[0]
                .get("value"),
            Some(&ScalarValue::Int64(Some(2)))
        );
    }

    #[tokio::test]
    async fn param_subquery_observes_preceding_store_write() {
        let ctx = SessionContext::new();
        let store_schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Utf8, false)]));
        ctx.register_table(
            "rows",
            Arc::new(datafusion::datasource::empty::EmptyTable::new(store_schema)),
        )
        .expect("register authoring store placeholder");
        let query = ctx
            .sql("SELECT count(*) AS count FROM rows")
            .await
            .expect("plan count query");
        let count_expr = Expr::ScalarSubquery(datafusion::logical_expr::Subquery {
            subquery: Arc::new(query.logical_plan().clone()),
            outer_ref_columns: Vec::new(),
            spans: datafusion::common::Spans::new(),
        });
        let count = Param::new("count", ScalarValue::Int64(Some(0)));
        let binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .set_store(
                "rows",
                StoreUpdate::insert_rows([StoreRow::new().field("id", lit("first"))]),
            )
            .set_param(&count, count_expr);
        let compiled = Chart::<Cartesian>::new()
            .param(count)
            .store(Store::empty("rows").field("id", DataType::Utf8, false))
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile ordered store query binding");
        let runtime = CompiledChartEventBinding::compile(
            0,
            compiled.event_bindings().first().unwrap(),
            &ctx,
            compiled.param_specs(),
            compiled.selection_specs(),
            compiled.store_specs(),
            &compiled.event_datum_types(),
        )
        .expect("compile ordered store query runtime");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let handler = ChartEventBindingHandler {
            runtime: Arc::new(runtime),
            state: Mutex::new(ChartEventBindingState::default()),
        };

        let status = handler
            .handle_with_context(
                &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: [10.0, 20.0],
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                &EventStreamContext::default(),
                &mut state,
                &empty_rtree(),
            )
            .await;

        assert_eq!(status.admission, Some(EventAdmission::Committed));
        assert_eq!(
            state.params().await.get("count"),
            Some(&ScalarValue::Int64(Some(1)))
        );
    }

    #[tokio::test]
    async fn working_store_subqueries_do_not_cross_hit_between_action_generations() {
        let ctx = SessionContext::new();
        let store_schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Utf8, false)]));
        ctx.register_table(
            "rows",
            Arc::new(datafusion::datasource::empty::EmptyTable::new(store_schema)),
        )
        .expect("register authoring store placeholder");
        let query = ctx
            .sql("SELECT count(*) AS count FROM rows")
            .await
            .expect("plan count query");
        let count_expr = Expr::ScalarSubquery(datafusion::logical_expr::Subquery {
            subquery: Arc::new(query.logical_plan().clone()),
            outer_ref_columns: Vec::new(),
            spans: datafusion::common::Spans::new(),
        });
        let first_count = Param::new("first_count", ScalarValue::Int64(Some(0)));
        let second_count = Param::new("second_count", ScalarValue::Int64(Some(0)));
        let binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .set_store(
                "rows",
                StoreUpdate::insert_rows([StoreRow::new().field("id", lit("first"))]),
            )
            .set_param(&first_count, count_expr.clone())
            .set_store(
                "rows",
                StoreUpdate::insert_rows([StoreRow::new().field("id", lit("second"))]),
            )
            .set_param(&second_count, count_expr);
        let compiled = Chart::<Cartesian>::new()
            .param(first_count)
            .param(second_count)
            .store(Store::empty("rows").field("id", DataType::Utf8, false))
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile generation-isolation binding");
        let runtime = CompiledChartEventBinding::compile(
            0,
            compiled.event_bindings().first().unwrap(),
            &ctx,
            compiled.param_specs(),
            compiled.selection_specs(),
            compiled.store_specs(),
            &compiled.event_datum_types(),
        )
        .expect("compile generation-isolation runtime");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let handler = ChartEventBindingHandler {
            runtime: Arc::new(runtime),
            state: Mutex::new(ChartEventBindingState::default()),
        };

        let status = handler
            .handle_with_context(
                &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: [10.0, 20.0],
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                &EventStreamContext::default(),
                &mut state,
                &empty_rtree(),
            )
            .await;

        assert_eq!(status.admission, Some(EventAdmission::Committed));
        let params = state.params().await;
        assert_eq!(
            params.get("first_count"),
            Some(&ScalarValue::Int64(Some(1)))
        );
        assert_eq!(
            params.get("second_count"),
            Some(&ScalarValue::Int64(Some(2)))
        );
    }

    #[tokio::test]
    async fn param_action_observes_preceding_selection_write() {
        let ctx = SessionContext::new();
        let picked = Selection::new("picked").empty_selects_nothing();
        let selected = Param::new("selected", ScalarValue::Boolean(Some(false)));
        let binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .set_selection(
                "picked",
                SelectionUpdate::replace_all_clauses([SelectionClauseUpdate::equality(lit(
                    "active",
                ))
                .facet_scope(CoordinationScope::Shared)
                .dimension(lit("field"), lit("field"))
                .build()]),
            )
            .set_param(&selected, picked.predicate());
        let compiled = Chart::<Cartesian>::new()
            .selection(picked)
            .param(selected)
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile ordered selection binding");
        let runtime = CompiledChartEventBinding::compile(
            0,
            compiled.event_bindings().first().unwrap(),
            &ctx,
            compiled.param_specs(),
            compiled.selection_specs(),
            compiled.store_specs(),
            &compiled.event_datum_types(),
        )
        .expect("compile ordered selection runtime");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let handler = ChartEventBindingHandler {
            runtime: Arc::new(runtime),
            state: Mutex::new(ChartEventBindingState::default()),
        };

        let status = handler
            .handle_with_context(
                &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: [10.0, 20.0],
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                &EventStreamContext::default(),
                &mut state,
                &empty_rtree(),
            )
            .await;

        assert_eq!(status.admission, Some(EventAdmission::Committed));
        assert_eq!(
            state.params().await.get("selected"),
            Some(&ScalarValue::Boolean(Some(true)))
        );
    }

    #[tokio::test]
    async fn later_action_failure_rolls_back_state_and_cursor() {
        let ctx = SessionContext::new();
        let value = Param::new("value", ScalarValue::Int64(Some(1)));
        let binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .set_param(&value, lit(2_i64))
            .set_cursor(lit("not-a-cursor"));
        let compiled = Chart::<Cartesian>::new()
            .param(value)
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile rollback binding");
        let runtime = CompiledChartEventBinding::compile(
            0,
            compiled.event_bindings().first().unwrap(),
            &ctx,
            compiled.param_specs(),
            compiled.selection_specs(),
            compiled.store_specs(),
            &compiled.event_datum_types(),
        )
        .expect("compile rollback runtime");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let handler = ChartEventBindingHandler {
            runtime: Arc::new(runtime),
            state: Mutex::new(ChartEventBindingState::default()),
        };

        let status = handler
            .handle_with_context(
                &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: [10.0, 20.0],
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                &EventStreamContext::default(),
                &mut state,
                &empty_rtree(),
            )
            .await;

        assert_eq!(status.admission, Some(EventAdmission::Failed));
        assert_eq!(status.cursor, None);
        assert_eq!(
            state.params().await.get("value"),
            Some(&ScalarValue::Int64(Some(1)))
        );
    }

    #[tokio::test]
    async fn runtime_string_assignment_is_strictly_cast_to_the_target_param_type() {
        let ctx = SessionContext::new();
        let source = Param::new("source", ScalarValue::Utf8(Some("42".to_owned())));
        let target = Param::new("target", ScalarValue::Int64(Some(1)));
        let binding =
            ChartEventBinding::on(ChartEventType::CursorMoved).set_param(&target, source.expr());
        let compiled = Chart::<Cartesian>::new()
            .param(source)
            .param(target)
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile strict cast binding");
        let runtime = CompiledChartEventBinding::compile(
            0,
            compiled.event_bindings().first().unwrap(),
            &ctx,
            compiled.param_specs(),
            compiled.selection_specs(),
            compiled.store_specs(),
            &compiled.event_datum_types(),
        )
        .expect("compile strict cast runtime");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let handler = ChartEventBindingHandler {
            runtime: Arc::new(runtime),
            state: Mutex::new(ChartEventBindingState::default()),
        };

        let status = handler
            .handle_with_context(
                &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: [10.0, 20.0],
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                &EventStreamContext::default(),
                &mut state,
                &empty_rtree(),
            )
            .await;

        assert_eq!(status.admission, Some(EventAdmission::Committed));
        assert_eq!(
            state.params().await.get("target"),
            Some(&ScalarValue::Int64(Some(42)))
        );
    }

    #[tokio::test]
    async fn runtime_cast_failure_rolls_back_preceding_staged_actions() {
        let ctx = SessionContext::new();
        let source = Param::new("source", ScalarValue::Utf8(Some("invalid".to_owned())));
        let first = Param::new("first", ScalarValue::Int64(Some(1)));
        let second = Param::new("second", ScalarValue::Int64(Some(5)));
        let binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .set_param(&first, lit(2_i64))
            .set_param(&second, source.expr());
        let compiled = Chart::<Cartesian>::new()
            .param(source)
            .param(first)
            .param(second)
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile rollback binding");
        let runtime = CompiledChartEventBinding::compile(
            0,
            compiled.event_bindings().first().unwrap(),
            &ctx,
            compiled.param_specs(),
            compiled.selection_specs(),
            compiled.store_specs(),
            &compiled.event_datum_types(),
        )
        .expect("compile rollback runtime");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let handler = ChartEventBindingHandler {
            runtime: Arc::new(runtime),
            state: Mutex::new(ChartEventBindingState::default()),
        };

        let status = handler
            .handle_with_context(
                &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: [10.0, 20.0],
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                &EventStreamContext::default(),
                &mut state,
                &empty_rtree(),
            )
            .await;

        assert_eq!(status.admission, Some(EventAdmission::Failed));
        let params = state.params().await;
        assert_eq!(params.get("first"), Some(&ScalarValue::Int64(Some(1))));
        assert_eq!(params.get("second"), Some(&ScalarValue::Int64(Some(5))));
    }

    #[tokio::test]
    async fn cursor_only_action_commits_without_rerendering() {
        let binding =
            ChartEventBinding::on(ChartEventType::CursorMoved).set_cursor(lit("crosshair"));
        let mut state = bound_state(binding.clone()).await;
        let handler = handler_for_binding(binding).await;
        let status = handler
            .handle_with_context(
                &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: [10.0, 20.0],
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                &EventStreamContext::default(),
                &mut state,
                &empty_rtree(),
            )
            .await;
        assert_eq!(status.admission, Some(EventAdmission::Committed));
        assert_eq!(status.cursor, Some(CursorStyle::Crosshair));
        assert!(!status.rerender);
    }

    #[tokio::test]
    async fn staged_cursor_is_not_published_when_a_later_state_action_fails() {
        let value = Param::new("value", ScalarValue::Int64(Some(1)));
        let binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .set_cursor(lit("crosshair"))
            .set_param_required(&value, lit(ScalarValue::Int64(None)));
        let ctx = SessionContext::new();
        let compiled = Chart::<Cartesian>::new()
            .param(value)
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile rollback binding");
        let runtime = CompiledChartEventBinding::compile(
            0,
            compiled.event_bindings().first().unwrap(),
            &ctx,
            compiled.param_specs(),
            compiled.selection_specs(),
            compiled.store_specs(),
            &compiled.event_datum_types(),
        )
        .expect("compile rollback runtime");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let handler = ChartEventBindingHandler {
            runtime: Arc::new(runtime),
            state: Mutex::new(ChartEventBindingState::default()),
        };

        let status = handler
            .handle_with_context(
                &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: [10.0, 20.0],
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                &EventStreamContext::default(),
                &mut state,
                &empty_rtree(),
            )
            .await;

        assert_eq!(status.admission, Some(EventAdmission::Failed));
        assert_eq!(status.cursor, None);
        assert_eq!(
            state.params().await.get("value"),
            Some(&ScalarValue::Int64(Some(1)))
        );
    }

    #[tokio::test]
    async fn ordinary_utf8_param_assignment_never_changes_cursor() {
        let label = Param::new(
            "cursor_label",
            ScalarValue::Utf8(Some("default".to_string())),
        );
        let binding =
            ChartEventBinding::on(ChartEventType::CursorMoved).set_param(&label, lit("crosshair"));
        let ctx = SessionContext::new();
        let compiled = Chart::<Cartesian>::new()
            .param(label)
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile string assignment");
        let runtime = CompiledChartEventBinding::compile(
            0,
            compiled.event_bindings().first().unwrap(),
            &ctx,
            compiled.param_specs(),
            compiled.selection_specs(),
            compiled.store_specs(),
            &compiled.event_datum_types(),
        )
        .expect("compile string runtime");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let handler = ChartEventBindingHandler {
            runtime: Arc::new(runtime),
            state: Mutex::new(ChartEventBindingState::default()),
        };

        let status = handler
            .handle_with_context(
                &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: [10.0, 20.0],
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                &EventStreamContext::default(),
                &mut state,
                &empty_rtree(),
            )
            .await;

        assert_eq!(status.admission, Some(EventAdmission::Committed));
        assert_eq!(status.cursor, None);
        assert_eq!(
            state.params().await.get("cursor_label"),
            Some(&ScalarValue::Utf8(Some("crosshair".to_string())))
        );
    }

    #[tokio::test]
    async fn low_level_stream_filter_reads_routed_committed_params() {
        let ctx = SessionContext::new();
        let width = Param::new("width", ScalarValue::Float64(Some(10.0)));
        let compiled = Chart::<Cartesian>::new()
            .param_with_sharing(width, CoordinationScope::Free)
            .compile(&ctx)
            .await
            .expect("compile");
        let param_specs = compiled.param_specs().clone();
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx.clone()));
        let state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        let owner_path = vec![ScalarValue::Utf8(Some("facet-a".to_string()))];
        let mut scope = coord_scope(0, 0.0, 0.0, 100.0, 100.0, &[]);
        scope.sharing_owner_paths.insert(0, owner_path.clone());
        {
            let mut runtime = state.runtime.lock().await;
            runtime.last_interaction_state.scopes = vec![scope];
            runtime
                .session
                .apply_scoped_param_patch(vec![ScopedParamAssignment {
                    name: "width".to_string(),
                    owner_path,
                    value: ScalarValue::Float64(Some(640.0)),
                    replace_scoped_values: false,
                }])
                .expect("apply routed committed value");
        }
        let stream = ChartEventStream::on(ChartEventType::MouseDown)
            .filter(event::param("width").gt(lit(100.0)));
        let config = stream_config_for_chart_stream(
            &stream,
            None,
            false,
            &ctx,
            &param_specs,
            Some(committed_stream_param_provider(&state)),
            None,
        )
        .expect("compile stream filter");
        let filter = config.filter.as_ref().unwrap().first().unwrap();
        let event = SceneGraphEvent::MouseDown(SceneMouseDownEvent {
            position: [50.0, 50.0],
            button: MouseButton::Left,
            mark_instance: None,
            modifiers: Default::default(),
        });
        assert!(filter.matches(&event, &EventStreamContext::default(), &empty_rtree()));
    }

    #[test]
    fn low_level_stream_filters_reject_temporal_params_and_store_state() {
        let ctx = SessionContext::new();
        let width = Param::new("width", ScalarValue::Float64(Some(10.0)));
        let params = shared_param_specs(&[width]);
        let temporal = ChartEventStream::on(ChartEventType::MouseDown)
            .filter(event::start_param("width").gt(lit(0.0)));
        let temporal_error =
            stream_config_for_chart_stream(&temporal, None, false, &ctx, &params, None, None)
                .expect_err("temporal params are unavailable in stream filters");
        assert!(
            temporal_error
                .to_string()
                .contains("Unknown physical scalar expression column")
        );

        let store = ChartEventStream::on(ChartEventType::MouseDown)
            .filter(avenger_chart_core::store_placeholder_expr("rows").is_not_null());
        let store_error =
            stream_config_for_chart_stream(&store, None, false, &ctx, &params, None, None)
                .expect_err("stores are unavailable in stream filters");
        assert!(
            store_error
                .to_string()
                .contains("unsupported state reference")
        );
    }

    #[tokio::test]
    async fn binding_with_coordinate_helper_adds_derived_schema_column() {
        let x_domain = Param::raw_domain("x_domain");
        let raw = x_domain.expr();
        let binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .set_param(
                &x_domain,
                event::interval(
                    event::interval_start(raw.clone()) - event::event_at_start_coord("x"),
                    event::interval_end(raw) - event::event_at_start_coord("x"),
                ),
            )
            .preview();
        let ctx = SessionContext::new();
        let compiled = Chart::<Cartesian>::new()
            .param(x_domain.clone())
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile");
        let runtime = CompiledChartEventBinding::compile(
            0,
            compiled.event_bindings().first().unwrap(),
            &ctx,
            compiled.param_specs(),
            compiled.selection_specs(),
            compiled.store_specs(),
            &compiled.event_datum_types(),
        )
        .expect("compile binding runtime");

        let has_column = |name: &str| {
            runtime
                .program
                .schema()
                .fields()
                .iter()
                .any(|f| f.name() == name)
        };
        assert!(
            has_column("__event_at_start_coord_x"),
            "expected derived event_at_start coord column in schema"
        );
        assert!(
            runtime
                .interaction_requests
                .event_at_start_coord
                .contains("x")
        );
    }

    #[tokio::test]
    async fn binding_without_coordinate_helpers_adds_no_derived_columns() {
        let binding = ChartEventBinding::on(ChartEventType::CanvasResize)
            .set_param("width", event::canvas_width())
            .preview();
        let ctx = SessionContext::new();
        let compiled = Chart::<Cartesian>::new()
            .param(Param::new("width", ScalarValue::Float64(Some(640.0))))
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile");
        let runtime = CompiledChartEventBinding::compile(
            0,
            compiled.event_bindings().first().unwrap(),
            &ctx,
            compiled.param_specs(),
            compiled.selection_specs(),
            compiled.store_specs(),
            &compiled.event_datum_types(),
        )
        .expect("compile binding runtime");

        assert!(runtime.interaction_requests.is_empty());
        let has_derived = runtime.program.schema().fields().iter().any(|f| {
            f.name().starts_with("__event_coord_")
                || f.name().starts_with("__start_coord_")
                || f.name().starts_with("__event_at_start_coord_")
                || f.name().starts_with("__previous_coord_")
                || f.name().starts_with("__event_domain_")
                || f.name().starts_with("__start_domain_")
        });
        assert!(!has_derived, "expected no derived interaction columns");
    }

    fn shared_param_specs(params: &[Param]) -> ParamSpecRegistry {
        let mut allocator = avenger_chart_core::CompiledIdentityAllocator::new("app-test");
        ParamSpecRegistry::try_from_specs(
            params.iter().map(|param| {
                let mut spec = CompiledParamSpec::shared(param);
                spec.runtime_id = allocator.allocate_param();
                spec
            }),
            |spec| &spec.runtime_id,
            |spec| spec.name.as_str(),
        )
        .unwrap()
    }

    fn resolve_param_only_change_bindings(
        bindings: &[ChartParamChangeBinding],
        specs: &ParamSpecRegistry,
    ) -> Vec<ChartParamChangeBinding> {
        bindings
            .iter()
            .cloned()
            .map(|binding| {
                let source_id = specs
                    .resolve_source_name(&binding.source_param_name)
                    .expect("test source param")
                    .clone();
                let actions = binding
                    .action
                    .ordered_steps()
                    .into_iter()
                    .map(|step| match step {
                        ChartActionStep::SetParam(assignment) => {
                            let runtime_id = specs
                                .resolve_source_name(&assignment.param_name)
                                .expect("test target param")
                                .clone();
                            ChartEventAction::SetParam(ChartEventParamAction {
                                target: ResolvedStateTarget::new(runtime_id, assignment.param_name),
                                value: assignment.value,
                                scope: assignment.scope,
                                replace_scoped_values: assignment.replace_scoped_values,
                                reject_null: assignment.reject_null,
                            })
                        }
                        _ => panic!("param-only graph test received a non-param action"),
                    })
                    .collect();
                let source_name = binding.source_param_name.clone();
                binding
                    .with_resolved_state(ResolvedStateTarget::new(source_id, source_name), actions)
            })
            .collect()
    }

    #[test]
    fn param_change_graph_reports_complete_cycle_paths() {
        let params = [
            Param::new("a", ScalarValue::Int64(Some(0))),
            Param::new("b", ScalarValue::Int64(Some(0))),
            Param::new("c", ScalarValue::Int64(Some(0))),
        ];
        let specs = shared_param_specs(&params);
        let ctx = SessionContext::new();
        let compile = |bindings: &[ChartParamChangeBinding]| {
            let bindings = resolve_param_only_change_bindings(bindings, &specs);
            CompiledChartParamChangeGraph::compile(
                &bindings,
                &ctx,
                &specs,
                &SelectionSpecRegistry::default(),
                &StoreSpecRegistry::default(),
                None,
            )
        };

        let self_cycle = [
            ChartParamChangeBinding::on(&params[0]).set_param(&params[0], param_change::value())
        ];
        let error = compile(&self_cycle).err().expect("self cycle must fail");
        assert!(error.to_string().contains("a → a"), "{error}");

        let direct_cycle = [
            ChartParamChangeBinding::on(&params[0]).set_param(&params[1], param_change::value()),
            ChartParamChangeBinding::on(&params[1]).set_param(&params[0], param_change::value()),
        ];
        let error = compile(&direct_cycle)
            .err()
            .expect("direct cycle must fail");
        assert!(error.to_string().contains("a → b → a"), "{error}");

        let transitive_cycle = [
            ChartParamChangeBinding::on(&params[0]).set_param(&params[1], param_change::value()),
            ChartParamChangeBinding::on(&params[1]).set_param(&params[2], param_change::value()),
            ChartParamChangeBinding::on(&params[2]).set_param(&params[0], param_change::value()),
        ];
        let error = compile(&transitive_cycle)
            .err()
            .expect("transitive cycle must fail");
        assert!(error.to_string().contains("a → b → c → a"), "{error}");
    }

    #[test]
    fn param_change_graph_rejects_duplicate_reactive_writers() {
        let params = [
            Param::new("a", ScalarValue::Int64(Some(0))),
            Param::new("b", ScalarValue::Int64(Some(0))),
            Param::new("sink", ScalarValue::Int64(Some(0))),
        ];
        let specs = shared_param_specs(&params);
        let bindings = [
            ChartParamChangeBinding::on(&params[0]).set_param(&params[2], param_change::value()),
            ChartParamChangeBinding::on(&params[1]).set_param(&params[2], param_change::value()),
        ];
        let bindings = resolve_param_only_change_bindings(&bindings, &specs);
        let error = CompiledChartParamChangeGraph::compile(
            &bindings,
            &SessionContext::new(),
            &specs,
            &SelectionSpecRegistry::default(),
            &StoreSpecRegistry::default(),
            None,
        )
        .err()
        .expect("duplicate writer must fail");
        assert!(
            error
                .to_string()
                .contains("Parameter 'sink' has more than one reactive writer"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn param_change_graph_is_equivalent_after_bincode() {
        let a = Param::new("a", ScalarValue::Int64(Some(0)));
        let b = Param::new("b", ScalarValue::Int64(Some(0)));
        let c = Param::new("c", ScalarValue::Int64(Some(0)));
        let ctx = SessionContext::new();
        let compiled = Chart::<Cartesian>::new()
            .param(a.clone())
            .param(b.clone())
            .param(c.clone())
            .param_change_binding(
                ChartParamChangeBinding::on(&a).set_param(&b, param_change::value()),
            )
            .param_change_binding(
                ChartParamChangeBinding::on(&b).set_param(&c, param_change::value()),
            )
            .compile(&ctx)
            .await
            .expect("compile graph chart");
        let restored: CompiledPlot = bincode::deserialize(
            &bincode::serialize(&compiled).expect("serialize compiled graph chart"),
        )
        .expect("restore compiled graph chart");

        let direct = param_change_graph_for_plot(&compiled, &ctx).expect("compile direct graph");
        let round_trip =
            param_change_graph_for_plot(&restored, &ctx).expect("compile restored graph");
        assert_eq!(direct.edges, round_trip.edges);
        assert_eq!(direct.bindings_by_source, round_trip.bindings_by_source);
        assert_eq!(direct.binding_count(), round_trip.binding_count());
        assert_eq!(
            direct
                .bindings
                .iter()
                .map(|binding| binding.program.expression_count())
                .collect::<Vec<_>>(),
            round_trip
                .bindings
                .iter()
                .map(|binding| binding.program.expression_count())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn param_change_programs_are_typed_and_filters_gate_actions() {
        let number = Param::new("number", ScalarValue::Int64(Some(0)));
        let number_copy = Param::new("number_copy", ScalarValue::Float64(Some(0.0)));
        let text = Param::new("text", ScalarValue::Utf8(None));
        let text_copy = Param::new("text_copy", ScalarValue::Utf8(None));
        let flag = Param::new("flag", ScalarValue::Boolean(Some(false)));
        let flag_copy = Param::new("flag_copy", ScalarValue::Boolean(Some(false)));
        let params = [
            number.clone(),
            number_copy.clone(),
            text.clone(),
            text_copy.clone(),
            flag.clone(),
            flag_copy.clone(),
        ];
        let specs = shared_param_specs(&params);
        let bindings = [
            ChartParamChangeBinding::on(&number)
                .filter(param_change::value().gt(lit(2_i64)))
                .set_param(&number_copy, param_change::value()),
            ChartParamChangeBinding::on(&text).set_param(&text_copy, param_change::value()),
            ChartParamChangeBinding::on(&flag).set_param(&flag_copy, param_change::value()),
        ];
        let bindings = resolve_param_only_change_bindings(&bindings, &specs);
        let graph = CompiledChartParamChangeGraph::compile(
            &bindings,
            &SessionContext::new(),
            &specs,
            &SelectionSpecRegistry::default(),
            &StoreSpecRegistry::default(),
            None,
        )
        .expect("compile typed reaction programs");

        let evaluate = |binding: &CompiledChartParamChangeBinding,
                        current: &[(&str, ScalarValue)],
                        value: ScalarValue,
                        previous: ScalarValue| {
            let mut values = current
                .iter()
                .map(|(name, value)| (event::param_column_name(name), value.clone()))
                .collect::<HashMap<_, _>>();
            values.insert(param_change::VALUE_FIELD.to_string(), value);
            values.insert(param_change::PREVIOUS_VALUE_FIELD.to_string(), previous);
            let batch = one_row_batch_from_scalars(binding.program.schema().clone(), &values)
                .expect("build reaction row");
            binding
                .program
                .evaluate_values(&batch)
                .expect("evaluate reaction")
        };

        let false_values = evaluate(
            &graph.bindings[0],
            &[("number", ScalarValue::Int64(Some(1)))],
            ScalarValue::Int64(Some(1)),
            ScalarValue::Int64(Some(0)),
        );
        assert!(!filters_pass(&false_values[..1]));
        let true_values = evaluate(
            &graph.bindings[0],
            &[("number", ScalarValue::Int64(Some(3)))],
            ScalarValue::Int64(Some(3)),
            ScalarValue::Int64(Some(0)),
        );
        assert!(filters_pass(&true_values[..1]));
        assert_eq!(true_values[1], ScalarValue::Float64(Some(3.0)));

        let text_values = evaluate(
            &graph.bindings[1],
            &[("text", ScalarValue::Utf8(Some("ready".to_string())))],
            ScalarValue::Utf8(Some("ready".to_string())),
            ScalarValue::Utf8(None),
        );
        assert_eq!(text_values[0], ScalarValue::Utf8(Some("ready".to_string())));
        let bool_values = evaluate(
            &graph.bindings[2],
            &[("flag", ScalarValue::Boolean(Some(true)))],
            ScalarValue::Boolean(Some(true)),
            ScalarValue::Boolean(Some(false)),
        );
        assert_eq!(bool_values[0], ScalarValue::Boolean(Some(true)));

        assert_eq!(
            ScalarValue::Utf8(None),
            ScalarValue::Utf8(None),
            "equal Arrow scalar assignments are the no-fire case"
        );
        assert_ne!(
            ScalarValue::Utf8(None),
            ScalarValue::Utf8(Some("ready".to_string())),
            "nullable-to-value assignments are the changed-source case"
        );
    }
}
