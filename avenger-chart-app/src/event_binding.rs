use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use avenger_app::error::AvengerAppError;
use avenger_chart::{
    event::{
        self, ChartEventAssignmentScope, ChartEventBinding, ChartEventEvaluationMode,
        ChartEventScopeTarget, ChartEventStream, ChartEventType, InteractionColumnRequests,
    },
    plot::{
        CompiledPlot, ScopedParamAssignment, ScopedParamStoreSnapshot, ScopedStoreAssignment,
        SelectionAssignment, SelectionStateUpdate, StoreStateUpdate,
    },
    render::{EvaluatedInteractionScope, EvaluationMode},
    serialization::LogicalExprNodeExt,
};
use avenger_chart_core::{
    CompiledParamSpec, CompiledScalarExpressionProgram, CompiledSelectionSpec, CompiledStoreSpec,
    InteractionPointInversionRequest, PhysicalScalarExpressionSpec, PhysicalScalarProgramOptions,
    PlaceholderColumn, ResolvedSelectionClauseScope, SelectionClause, SelectionClauseUpdate,
    SelectionFacetContextValue, SelectionIntervalDimensionUpdate, SelectionIntervalDimensionValue,
    SelectionPredicateSpec, SelectionPredicateUpdate, SelectionUpdate, SelectionValueExpr, Sharing,
    StoreFieldPatch, StoreKey, StoreRow, StoreRowValue, StoreUpdate, StoreValueExpr,
    collect_placeholder_ids, one_row_batch_from_scalars, schema_from_fields,
};
use avenger_common::cursor::CursorStyle;
use avenger_common::time::Instant;
use avenger_eventstream::{
    manager::EventStreamHandler,
    scene::{ModifiersState, SceneGraphEvent, SceneGraphEventType},
    stream::{
        EventStreamConfig, EventStreamContext, EventStreamEventSnapshot, EventStreamFilter,
        UpdateStatus,
    },
    window::{Key, MouseButton, MouseScrollDelta},
};
use avenger_geometry::rtree::SceneGraphRTree;
use datafusion::{
    arrow::{
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    error::DataFusionError,
    logical_expr::Expr,
    prelude::SessionContext,
    scalar::ScalarValue,
};
use indexmap::IndexMap;

use crate::ChartAppState;

pub(crate) fn event_streams_for_plot_bindings(
    compiled_plot: &CompiledPlot,
    ctx: &SessionContext,
) -> Result<
    Vec<(
        EventStreamConfig,
        Arc<dyn EventStreamHandler<ChartAppState>>,
    )>,
    AvengerAppError,
> {
    event_streams_for_bindings(
        compiled_plot.event_bindings(),
        ctx,
        compiled_plot.param_specs(),
        compiled_plot.selection_specs(),
        compiled_plot.store_specs(),
        compiled_plot.cursor_params(),
    )
}

pub(crate) fn event_streams_for_bindings(
    bindings: &[ChartEventBinding],
    ctx: &SessionContext,
    param_specs: &IndexMap<String, CompiledParamSpec>,
    selection_specs: &IndexMap<String, CompiledSelectionSpec>,
    store_specs: &IndexMap<String, CompiledStoreSpec>,
    cursor_params: &[String],
) -> Result<
    Vec<(
        EventStreamConfig,
        Arc<dyn EventStreamHandler<ChartAppState>>,
    )>,
    AvengerAppError,
> {
    let mut streams = Vec::new();
    for (binding_index, binding) in bindings.iter().enumerate() {
        let runtime = Arc::new(CompiledChartEventBinding::compile(
            binding_index,
            binding,
            ctx,
            param_specs,
            selection_specs,
            store_specs,
            cursor_params,
        )?);
        streams.push((
            runtime.event_stream_config.clone(),
            Arc::new(ChartEventBindingHandler {
                runtime: runtime.clone(),
                state: Mutex::new(ChartEventBindingState::default()),
            }) as Arc<dyn EventStreamHandler<ChartAppState>>,
        ));

        if binding.settle_exact {
            if let Some(between) = &binding.between {
                let end_config =
                    stream_config_for_chart_stream(&between.end, None, false, ctx, param_specs)?;
                streams.push((
                    end_config,
                    Arc::new(ChartEventExactOnlyHandler)
                        as Arc<dyn EventStreamHandler<ChartAppState>>,
                ));
            }
        }
    }
    Ok(streams)
}

struct CompiledChartEventBinding {
    binding_index: usize,
    event_stream_config: EventStreamConfig,
    program: CompiledScalarExpressionProgram,
    filter_count: usize,
    assignments: Vec<CompiledParamAssignment>,
    store_assignments: Vec<CompiledStoreAssignment>,
    selection_assignments: Vec<CompiledSelectionAssignment>,
    evaluation_mode: ChartEventEvaluationMode,
    interaction_requests: InteractionColumnRequests,
    scope_target: Option<ChartEventScopeTarget>,
    scope_target_uses_start_scope: bool,
    cursor_params: Arc<HashSet<String>>,
}

struct CompiledParamAssignment {
    param_name: String,
    sharing: Sharing,
    default_value: ScalarValue,
    scope: ChartEventAssignmentScope,
    replace_scoped_values: bool,
}

#[derive(Clone)]
struct CompiledStoreAssignment {
    store_name: String,
    sharing: Sharing,
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
    selection_id: String,
    spec: CompiledSelectionSpec,
    scope: ChartEventAssignmentScope,
    update: CompiledSelectionUpdate,
}

#[derive(Clone)]
enum CompiledSelectionUpdate {
    Clear,
    ClearInScope {
        scope: Sharing,
    },
    ReplaceAllClauses {
        clauses: Vec<CompiledSelectionClause>,
    },
    ReplaceClausesInScope {
        scope: Sharing,
        clauses: Vec<CompiledSelectionClause>,
    },
    UpsertClauses {
        clauses: Vec<CompiledSelectionClause>,
    },
    DeleteClauses {
        ids: Vec<usize>,
    },
}

#[derive(Clone)]
struct CompiledSelectionClause {
    id: usize,
    facet_scope: Sharing,
    predicate: CompiledSelectionPredicate,
}

#[derive(Clone)]
enum CompiledSelectionPredicate {
    Interval {
        dimensions: Vec<CompiledSelectionIntervalDimension>,
    },
}

#[derive(Clone)]
struct CompiledSelectionIntervalDimension {
    id: String,
    field_expr: datafusion_proto::protobuf::LogicalExprNode,
    min: usize,
    max: usize,
}

struct StoreExpressionAssignment {
    store_name: String,
    sharing: Sharing,
    scope: ChartEventAssignmentScope,
    replace_scoped_values: bool,
    update: StoreExpressionUpdate,
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
    selection_id: String,
    spec: CompiledSelectionSpec,
    scope: ChartEventAssignmentScope,
    update: SelectionExpressionUpdate,
}

enum SelectionExpressionUpdate {
    Clear,
    ClearInScope {
        scope: Sharing,
    },
    ReplaceAllClauses {
        clauses: Vec<SelectionExpressionClause>,
    },
    ReplaceClausesInScope {
        scope: Sharing,
        clauses: Vec<SelectionExpressionClause>,
    },
    UpsertClauses {
        clauses: Vec<SelectionExpressionClause>,
    },
    DeleteClauses {
        ids: Vec<Expr>,
    },
}

struct SelectionExpressionClause {
    id: Expr,
    facet_scope: Sharing,
    predicate: SelectionExpressionPredicate,
}

enum SelectionExpressionPredicate {
    Interval {
        dimensions: Vec<SelectionExpressionIntervalDimension>,
    },
}

struct SelectionExpressionIntervalDimension {
    id: String,
    field_expr: datafusion_proto::protobuf::LogicalExprNode,
    min: Expr,
    max: Expr,
}

impl CompiledChartEventBinding {
    fn compile(
        binding_index: usize,
        binding: &ChartEventBinding,
        ctx: &SessionContext,
        param_specs: &IndexMap<String, CompiledParamSpec>,
        selection_specs: &IndexMap<String, CompiledSelectionSpec>,
        store_specs: &IndexMap<String, CompiledStoreSpec>,
        cursor_params: &[String],
    ) -> Result<Self, AvengerAppError> {
        binding
            .validate()
            .map_err(|err| AvengerAppError::InternalError(err.to_string()))?;
        for assignment in &binding.assignments {
            if !param_specs.contains_key(&assignment.param_name) {
                return Err(AvengerAppError::InternalError(format!(
                    "Chart event binding assigns unknown param '{}'",
                    assignment.param_name
                )));
            }
        }
        for assignment in &binding.store_assignments {
            if !store_specs.contains_key(&assignment.store_name) {
                return Err(AvengerAppError::InternalError(format!(
                    "Chart event binding updates unknown store '{}'",
                    assignment.store_name
                )));
            }
        }
        for assignment in &binding.selection_assignments {
            if !selection_specs.contains_key(&assignment.selection_id) {
                return Err(AvengerAppError::InternalError(format!(
                    "Chart event binding updates unknown selection '{}'",
                    assignment.selection_id
                )));
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
        let mut assignment_exprs = Vec::new();
        for assignment in &binding.assignments {
            let expr = assignment
                .expr
                .to_expr(ctx)
                .map_err(|err| AvengerAppError::InternalError(err.to_string()))?;
            assignment_exprs.push((
                assignment.param_name.clone(),
                expr,
                assignment.scope,
                assignment.replace_scoped_values,
            ));
        }
        let mut store_exprs = Vec::new();
        for assignment in &binding.store_assignments {
            let store = store_specs
                .get(&assignment.store_name)
                .expect("store assignment validated");
            store_exprs.push(StoreExpressionAssignment {
                store_name: assignment.store_name.clone(),
                sharing: store.sharing,
                scope: assignment.scope,
                replace_scoped_values: assignment.replace_scoped_values,
                update: compile_store_expression_update(store, &assignment.update, ctx)?,
            });
        }
        let mut selection_exprs = Vec::new();
        for assignment in &binding.selection_assignments {
            let spec = selection_specs
                .get(&assignment.selection_id)
                .expect("selection assignment validated");
            selection_exprs.push(SelectionExpressionAssignment {
                selection_id: assignment.selection_id.clone(),
                spec: spec.clone(),
                scope: assignment.scope,
                update: compile_selection_expression_update(&assignment.update, ctx)?,
            });
        }

        let mut scan_exprs = filter_exprs.clone();
        scan_exprs.extend(assignment_exprs.iter().map(|(_, expr, _, _)| expr.clone()));
        for assignment in &store_exprs {
            scan_exprs.extend(store_expression_update_exprs(&assignment.update));
        }
        for assignment in &selection_exprs {
            scan_exprs.extend(selection_expression_update_exprs(&assignment.update));
        }
        let mut interaction_requests = event::scan_interaction_columns(&scan_exprs);
        for assignment in &store_exprs {
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
        for assignment in &selection_exprs {
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

        let schema = event_schema(param_specs, &interaction_requests);
        let allowed_columns = schema
            .fields()
            .iter()
            .map(|field| field.name().clone())
            .collect::<HashSet<_>>();
        let placeholder_columns = param_specs.keys().map(|param| {
            PlaceholderColumn::new(format!("${param}"), event::param_column_name(param))
        });
        let mut specs = Vec::new();
        for (index, expr) in filter_exprs.into_iter().enumerate() {
            specs.push(
                PhysicalScalarExpressionSpec::new(format!("filter_{index}"), expr)
                    .with_expected_type(DataType::Boolean),
            );
        }
        let filter_count = specs.len();
        let mut assignments = Vec::new();
        for (param_name, expr, scope, replace_scoped_values) in assignment_exprs {
            let spec = param_specs
                .get(&param_name)
                .expect("assignment param validated");
            let target_type = spec.default.data_type();
            let sharing = spec.sharing;
            specs.push(
                PhysicalScalarExpressionSpec::new(format!("assign_{param_name}"), expr)
                    .with_expected_type(target_type)
                    .with_nullable_cast(),
            );
            assignments.push(CompiledParamAssignment {
                param_name,
                sharing,
                default_value: spec.default.clone(),
                scope,
                replace_scoped_values,
            });
        }
        let mut store_assignments = Vec::new();
        for assignment in store_exprs {
            let update = append_store_expression_update_specs(
                &assignment.store_name,
                assignment.update,
                &mut specs,
                filter_count,
            );
            store_assignments.push(CompiledStoreAssignment {
                store_name: assignment.store_name,
                sharing: assignment.sharing,
                scope: assignment.scope,
                replace_scoped_values: assignment.replace_scoped_values,
                update,
            });
        }
        let mut selection_assignments = Vec::new();
        for assignment in selection_exprs {
            let update = append_selection_expression_update_specs(
                &assignment.selection_id,
                assignment.update,
                &mut specs,
                filter_count,
            );
            selection_assignments.push(CompiledSelectionAssignment {
                selection_id: assignment.selection_id,
                spec: assignment.spec,
                scope: assignment.scope,
                update,
            });
        }
        let program = CompiledScalarExpressionProgram::compile(
            ctx,
            schema,
            specs,
            PhysicalScalarProgramOptions::default()
                .with_allowed_columns(allowed_columns)
                .with_placeholder_columns(placeholder_columns),
        )
        .map_err(|err| AvengerAppError::InternalError(err.to_string()))?;
        let event_stream_config = event_stream_config_for_binding(binding, ctx, param_specs)?;

        Ok(Self {
            binding_index,
            event_stream_config,
            program,
            filter_count,
            assignments,
            store_assignments,
            selection_assignments,
            evaluation_mode: binding.evaluation_mode,
            interaction_requests,
            scope_target: binding.scope_target.clone(),
            scope_target_uses_start_scope: binding.between.is_some(),
            cursor_params: Arc::new(cursor_params.iter().cloned().collect()),
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
            .with_expected_type(field.expected_type)
            .with_nullable_cast(),
        );
        fields.push((field.name, value_index));
    }
    CompiledStoreRow { fields }
}

fn compile_selection_expression_update(
    update: &SelectionUpdate,
    ctx: &SessionContext,
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
        SelectionUpdate::DeleteClauses { ids } => SelectionExpressionUpdate::DeleteClauses {
            ids: ids
                .iter()
                .map(|id| selection_value_expr_to_expr(id, ctx))
                .collect::<Result<Vec<_>, _>>()?,
        },
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

fn selection_value_expr_to_expr(
    value: &SelectionValueExpr,
    ctx: &SessionContext,
) -> Result<Expr, AvengerAppError> {
    value
        .expr
        .to_expr(ctx)
        .map_err(|err| AvengerAppError::InternalError(err.to_string()))
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
        | SelectionExpressionUpdate::UpsertClauses { clauses } => {
            for clause in clauses {
                collect_selection_clause_exprs(clause, exprs);
            }
        }
        SelectionExpressionUpdate::DeleteClauses { ids } => {
            exprs.extend(ids.iter().cloned());
        }
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
    }
}

fn selection_update_needs_scope(update: &SelectionExpressionUpdate) -> bool {
    match update {
        SelectionExpressionUpdate::Clear => false,
        SelectionExpressionUpdate::ClearInScope { scope } => scope.to_level() != u8::MAX,
        SelectionExpressionUpdate::ReplaceAllClauses { clauses }
        | SelectionExpressionUpdate::UpsertClauses { clauses } => clauses
            .iter()
            .any(|clause| clause.facet_scope.to_level() != u8::MAX),
        SelectionExpressionUpdate::ReplaceClausesInScope { scope, clauses } => {
            scope.to_level() != u8::MAX
                || clauses
                    .iter()
                    .any(|clause| clause.facet_scope.to_level() != u8::MAX)
        }
        SelectionExpressionUpdate::DeleteClauses { .. } => false,
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

#[derive(Default)]
struct ChartEventBindingState {
    active_start: Option<Instant>,
    next_start_event_id: u64,
    active_start_event_id: Option<u64>,
    start_params: Option<ScopedParamStoreSnapshot>,
    previous_params: Option<ScopedParamStoreSnapshot>,
    start_scope: Option<EvaluatedInteractionScope>,
    previous_scope: Option<EvaluatedInteractionScope>,
}

struct ChartEventBindingHandler {
    runtime: Arc<CompiledChartEventBinding>,
    state: Mutex<ChartEventBindingState>,
}

#[async_trait]
impl EventStreamHandler<ChartAppState> for ChartEventBindingHandler {
    async fn handle(
        &self,
        _event: &SceneGraphEvent,
        _state: &mut ChartAppState,
        _rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        UpdateStatus::default()
    }

    async fn handle_with_context(
        &self,
        event: &SceneGraphEvent,
        context: &EventStreamContext,
        state: &mut ChartAppState,
        _rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        let mut app = state.runtime.lock().await;
        let eval_start = Instant::now();

        let requests = &self.runtime.interaction_requests;
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
                InteractionRoute::None | InteractionRoute::Ambiguous => None,
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
                    binding_state.start_scope = if routing_enabled {
                        match route_interaction_scope(
                            &app.last_interaction_state.scopes,
                            start.event.position(),
                            &required_channels,
                        ) {
                            InteractionRoute::Scope(scope) => Some(scope.clone()),
                            InteractionRoute::None | InteractionRoute::Ambiguous => None,
                        }
                    } else {
                        None
                    };
                    binding_state.previous_params = None;
                    binding_state.previous_scope = None;
                }
                None => {
                    binding_state.active_start = None;
                    binding_state.active_start_event_id = None;
                    binding_state.start_params = None;
                    binding_state.start_scope = None;
                }
                _ => {}
            }
        }

        let (start_snapshot, previous_snapshot, start_scope, previous_scope, start_event_id) = {
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
            )
        };

        if !target_scope_matches(
            self.runtime.scope_target.as_ref(),
            self.runtime.scope_target_uses_start_scope,
            current_scope.as_ref(),
            start_scope.as_ref(),
        ) {
            record_event_eval_elapsed(&mut app.event_metrics, eval_start);
            return UpdateStatus::default();
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

        let batch = match event_record_batch(
            self.runtime.program.schema().clone(),
            event,
            context,
            EventBatchInputs {
                current_params: &current_params,
                start_params: start_params.as_ref(),
                previous_params: previous_params.as_ref(),
                interaction_values: &interaction_values,
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
                return UpdateStatus::default();
            }
        };
        app.event_metrics.event_batches_evaluated += 1;

        let values = match self.runtime.program.evaluate_values(&batch) {
            Ok(values) => values,
            Err(err) => {
                app.event_metrics.evaluation_errors += 1;
                record_event_eval_elapsed(&mut app.event_metrics, eval_start);
                tracing::warn!(
                    target: "avenger_chart_app::event_binding",
                    binding = self.runtime.binding_index,
                    error = %err,
                    "failed to evaluate chart event expressions"
                );
                return UpdateStatus::default();
            }
        };
        app.event_metrics.physical_expression_evaluations +=
            self.runtime.program.expression_count();

        if !filters_pass(&values[..self.runtime.filter_count]) {
            app.event_metrics.filter_failures += 1;
            record_event_eval_elapsed(&mut app.event_metrics, eval_start);
            return UpdateStatus::default();
        }
        app.event_metrics.filter_passes += 1;

        let mut patch: Vec<ScopedParamAssignment> = Vec::new();
        for (assignment, value) in self
            .runtime
            .assignments
            .iter()
            .zip(values[self.runtime.filter_count..].iter())
        {
            // Derived interaction columns are null when the gesture has no routed
            // scope (e.g. a drag that started outside any plot area). Such an
            // assignment evaluates to a null or null-element value; writing it
            // would corrupt the target param, so treat it as a no-op unless the
            // binding intentionally writes the target's list-shaped default.
            if !assignment_value_is_writable(value, &assignment.default_value) {
                tracing::debug!(
                    target: "avenger_chart_app::event_binding",
                    binding = self.runtime.binding_index,
                    param = %assignment.param_name,
                    "skipping null/degenerate assignment value"
                );
                continue;
            }
            let assignment_scope = match assignment.scope {
                ChartEventAssignmentScope::Current => current_scope.as_ref(),
                ChartEventAssignmentScope::Start => start_scope.as_ref(),
            };
            let Some(owner_path) = assignment_owner_path(assignment.sharing, assignment_scope)
            else {
                // Non-shared param with no routed scope: skip rather than write
                // to the wrong owner.
                tracing::debug!(
                    target: "avenger_chart_app::event_binding",
                    binding = self.runtime.binding_index,
                    param = %assignment.param_name,
                    "skipping scoped assignment with no routed scope"
                );
                continue;
            };
            let comparison_owner_paths = assignment_scope
                .map(|scope| scope.sharing_owner_paths.clone())
                .unwrap_or_default();
            let comparison_params = app
                .session
                .effective_params_for_owner_paths(&comparison_owner_paths);
            if assignment.replace_scoped_values
                || comparison_params.get(&assignment.param_name) != Some(value)
            {
                patch.push(ScopedParamAssignment {
                    name: assignment.param_name.clone(),
                    owner_path,
                    value: value.clone(),
                    replace_scoped_values: assignment.replace_scoped_values,
                });
            }
        }

        let mut store_patch: Vec<ScopedStoreAssignment> = Vec::new();
        for assignment in &self.runtime.store_assignments {
            let assignment_scope = match assignment.scope {
                ChartEventAssignmentScope::Current => current_scope.as_ref(),
                ChartEventAssignmentScope::Start => start_scope.as_ref(),
            };
            let Some(owner_path) = assignment_owner_path(assignment.sharing, assignment_scope)
            else {
                tracing::debug!(
                    target: "avenger_chart_app::event_binding",
                    binding = self.runtime.binding_index,
                    store = %assignment.store_name,
                    "skipping scoped store assignment with no routed scope"
                );
                continue;
            };
            let Some(update) = store_state_update_from_values(
                &assignment.update,
                &values,
                self.runtime.filter_count,
            ) else {
                continue;
            };
            store_patch.push(ScopedStoreAssignment {
                store_name: assignment.store_name.clone(),
                owner_path,
                replace_scoped_values: assignment.replace_scoped_values,
                update,
            });
        }

        let mut selection_patch: Vec<SelectionAssignment> = Vec::new();
        for assignment in &self.runtime.selection_assignments {
            let assignment_scope = match assignment.scope {
                ChartEventAssignmentScope::Current => current_scope.as_ref(),
                ChartEventAssignmentScope::Start => start_scope.as_ref(),
            };
            let Some(update) = selection_state_update_from_values(
                &assignment.update,
                &assignment.spec,
                &values,
                self.runtime.filter_count,
                assignment_scope,
            ) else {
                tracing::debug!(
                    target: "avenger_chart_app::event_binding",
                    binding = self.runtime.binding_index,
                    selection = %assignment.selection_id,
                    "skipping selection assignment with no routed scope or null values"
                );
                continue;
            };
            selection_patch.push(SelectionAssignment {
                selection_id: assignment.selection_id.clone(),
                update,
            });
        }

        let cursor = cursor_from_patch(&patch, &self.runtime.cursor_params);
        let visual_patch_count = patch
            .iter()
            .filter(|assignment| !self.runtime.cursor_params.contains(&assignment.name))
            .count();
        let mut should_rerender = visual_patch_count > 0;
        if !patch.is_empty() {
            app.event_metrics.param_patch_events += 1;
            app.event_metrics.params_patched += patch.len();
            app.session.apply_scoped_param_patch(patch);
        }
        let store_changed = if store_patch.is_empty() {
            false
        } else {
            match app.session.apply_scoped_store_patch(store_patch) {
                Ok(changed) => {
                    app.event_metrics.store_patch_events += 1;
                    changed
                }
                Err(err) => {
                    app.event_metrics.evaluation_errors += 1;
                    tracing::warn!(
                        target: "avenger_chart_app::event_binding",
                        binding = self.runtime.binding_index,
                        error = %err,
                        "failed to apply store event patch"
                    );
                    false
                }
            }
        };
        let selection_changed = if selection_patch.is_empty() {
            false
        } else {
            match app.session.apply_selection_patch(selection_patch) {
                Ok(changed) => changed,
                Err(err) => {
                    app.event_metrics.evaluation_errors += 1;
                    tracing::warn!(
                        target: "avenger_chart_app::event_binding",
                        binding = self.runtime.binding_index,
                        error = %err,
                        "failed to apply selection event patch"
                    );
                    false
                }
            }
        };
        should_rerender |= store_changed;
        should_rerender |= selection_changed;
        if self.runtime.assignments.is_empty()
            && self.runtime.evaluation_mode == ChartEventEvaluationMode::Exact
        {
            should_rerender = true;
        }
        if !should_rerender {
            if !self.runtime.assignments.is_empty() {
                app.event_metrics.unchanged_patch_skips += 1;
            }
            record_event_eval_elapsed(&mut app.event_metrics, eval_start);
            return UpdateStatus {
                cursor,
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

        app.next_evaluation_mode = match self.runtime.evaluation_mode {
            ChartEventEvaluationMode::Preview => EvaluationMode::Preview,
            ChartEventEvaluationMode::Exact => EvaluationMode::Exact,
        };
        record_event_eval_elapsed(&mut app.event_metrics, eval_start);

        {
            let mut binding_state = self
                .state
                .lock()
                .expect("chart event binding lock poisoned");
            binding_state.previous_params = Some(app.session.snapshot_scoped_params());
            binding_state.previous_scope = current_scope.clone();
        }

        UpdateStatus {
            rerender: true,
            rebuild_geometry: self.runtime.evaluation_mode == ChartEventEvaluationMode::Exact,
            cursor,
            ..Default::default()
        }
    }
}

fn cursor_from_patch(
    patch: &[ScopedParamAssignment],
    cursor_params: &HashSet<String>,
) -> Option<CursorStyle> {
    patch
        .iter()
        .rev()
        .find(|assignment| cursor_params.contains(&assignment.name))
        .and_then(|assignment| match &assignment.value {
            ScalarValue::Utf8(Some(value)) | ScalarValue::LargeUtf8(Some(value)) => {
                CursorStyle::from_name(value)
            }
            _ => None,
        })
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
) -> Option<SelectionStateUpdate> {
    Some(match update {
        CompiledSelectionUpdate::Clear => SelectionStateUpdate::Clear,
        CompiledSelectionUpdate::ClearInScope {
            scope: clause_scope,
        } => SelectionStateUpdate::ClearInScope {
            scope_owner_path: selection_owner_path(*clause_scope, scope)?,
        },
        CompiledSelectionUpdate::ReplaceAllClauses { clauses } => {
            SelectionStateUpdate::ReplaceAllClauses {
                clauses: selection_clauses_from_values(clauses, spec, values, filter_count, scope)?,
            }
        }
        CompiledSelectionUpdate::ReplaceClausesInScope {
            scope: replace_scope,
            clauses,
        } => SelectionStateUpdate::ReplaceClausesInScope {
            scope_owner_path: selection_owner_path(*replace_scope, scope)?,
            clauses: selection_clauses_from_values(clauses, spec, values, filter_count, scope)?,
        },
        CompiledSelectionUpdate::UpsertClauses { clauses } => SelectionStateUpdate::UpsertClauses {
            clauses: selection_clauses_from_values(clauses, spec, values, filter_count, scope)?,
        },
        CompiledSelectionUpdate::DeleteClauses { ids } => SelectionStateUpdate::DeleteClauses {
            ids: ids
                .iter()
                .map(|id| selection_clause_id_from_value(values.get(filter_count + *id)?))
                .collect::<Option<Vec<_>>>()?,
        },
    })
}

fn selection_clauses_from_values(
    clauses: &[CompiledSelectionClause],
    spec: &CompiledSelectionSpec,
    values: &[ScalarValue],
    filter_count: usize,
    scope: Option<&EvaluatedInteractionScope>,
) -> Option<Vec<SelectionClause>> {
    clauses
        .iter()
        .map(|clause| selection_clause_from_values(clause, spec, values, filter_count, scope))
        .collect()
}

fn selection_clause_from_values(
    clause: &CompiledSelectionClause,
    spec: &CompiledSelectionSpec,
    values: &[ScalarValue],
    filter_count: usize,
    scope: Option<&EvaluatedInteractionScope>,
) -> Option<SelectionClause> {
    let id = selection_clause_id_from_value(values.get(filter_count + clause.id)?)?;
    let owner_path = selection_owner_path(clause.facet_scope, scope)?;
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
    sharing: Sharing,
    scope: Option<&EvaluatedInteractionScope>,
) -> Option<Vec<ScalarValue>> {
    assignment_owner_path(sharing, scope)
}

fn selection_facet_context_values(
    spec: &CompiledSelectionSpec,
    owner_path: &[ScalarValue],
) -> Vec<SelectionFacetContextValue> {
    spec.facet_context
        .iter()
        .zip(owner_path.iter())
        .filter_map(|(facet, value)| {
            (!value.is_null()).then(|| SelectionFacetContextValue {
                id: facet.id.clone(),
                value: value.clone(),
            })
        })
        .collect()
}

fn selection_clause_id_from_value(value: &ScalarValue) -> Option<String> {
    match value {
        ScalarValue::Utf8(Some(value)) | ScalarValue::LargeUtf8(Some(value)) => Some(value.clone()),
        ScalarValue::Null
        | ScalarValue::Utf8(None)
        | ScalarValue::LargeUtf8(None)
        | ScalarValue::Binary(None)
        | ScalarValue::LargeBinary(None)
        | ScalarValue::FixedSizeBinary(_, None) => None,
        other => Some(format!("{other:?}")),
    }
}

fn record_event_eval_elapsed(metrics: &mut crate::ChartEventMetrics, start: Instant) {
    metrics.total_eval_us += start.elapsed().as_micros() as u64;
}

struct ChartEventExactOnlyHandler;

#[async_trait]
impl EventStreamHandler<ChartAppState> for ChartEventExactOnlyHandler {
    async fn handle(
        &self,
        _event: &SceneGraphEvent,
        state: &mut ChartAppState,
        _rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        let mut app = state.runtime.lock().await;
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
/// makes domain-list expressions evaluate to a null or null-element list. Writing
/// those would corrupt the target param (and can fail downstream scale building),
/// so they are treated as no-ops unless the binding explicitly writes the
/// param's list-shaped default value. Raw-domain reset bindings use that path to
/// clear interaction domains back to the inferred/default scale domains.
fn assignment_value_is_writable(value: &ScalarValue, default_value: &ScalarValue) -> bool {
    use datafusion::arrow::array::Array;
    if value == default_value
        && matches!(
            default_value,
            ScalarValue::List(_) | ScalarValue::LargeList(_) | ScalarValue::FixedSizeList(_)
        )
    {
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
/// `Shared` params always write the root path. Non-shared params require a
/// routed scope; without one, returns `None` so the caller skips the write.
fn assignment_owner_path(
    sharing: Sharing,
    scope: Option<&EvaluatedInteractionScope>,
) -> Option<Vec<ScalarValue>> {
    let level = sharing.to_level();
    if level == u8::MAX {
        return Some(Vec::new());
    }
    match scope {
        Some(scope) => Some(
            scope
                .sharing_owner_paths
                .get(&level)
                .cloned()
                .unwrap_or_default(),
        ),
        None => None,
    }
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
    scope
        .coord_node_path
        .starts_with(&target.coord_node_path_prefix)
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

fn event_stream_config_for_binding(
    binding: &ChartEventBinding,
    ctx: &SessionContext,
    param_specs: &IndexMap<String, CompiledParamSpec>,
) -> Result<EventStreamConfig, AvengerAppError> {
    let mut config = EventStreamConfig {
        types: vec![scene_event_type_from_chart(binding.event_type)],
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
            )?),
            Box::new(stream_config_for_chart_stream(
                &between.end,
                None,
                false,
                ctx,
                param_specs,
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
    param_specs: &IndexMap<String, CompiledParamSpec>,
) -> Result<EventStreamConfig, AvengerAppError> {
    let mut config = EventStreamConfig {
        types: stream
            .event_type
            .map(|event_type| vec![scene_event_type_from_chart(event_type)])
            .unwrap_or_default(),
        source_group: stream.source_group.clone(),
        mark_paths: stream.mark_paths.clone(),
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
        )?]);
    }
    Ok(config)
}

fn compile_low_level_stream_filter(
    stream: &ChartEventStream,
    ctx: &SessionContext,
    _param_specs: &IndexMap<String, CompiledParamSpec>,
) -> Result<EventStreamFilter, AvengerAppError> {
    let schema = event_schema(
        &IndexMap::new(),
        &event::InteractionColumnRequests::default(),
    );
    let allowed_columns = schema
        .fields()
        .iter()
        .map(|field| field.name().clone())
        .collect::<HashSet<_>>();
    let mut specs = Vec::new();
    for (index, filter) in stream.filters.iter().enumerate() {
        let expr = filter
            .to_expr(ctx)
            .map_err(|err| AvengerAppError::InternalError(err.to_string()))?;
        let placeholders = collect_placeholder_ids(&expr)
            .map_err(|err| AvengerAppError::InternalError(err.to_string()))?;
        if !placeholders.is_empty() {
            return Err(AvengerAppError::InternalError(
                "Chart event stream start/end filters cannot reference params yet".to_string(),
            ));
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
            PhysicalScalarProgramOptions::default().with_allowed_columns(allowed_columns),
        )
        .map_err(|err| AvengerAppError::InternalError(err.to_string()))?,
    );
    Ok(EventStreamFilter::context(move |event, context, _rtree| {
        let params = IndexMap::new();
        let interaction_values = HashMap::new();
        let batch = match event_record_batch(
            program.schema().clone(),
            event,
            context,
            EventBatchInputs {
                current_params: &params,
                start_params: None,
                previous_params: None,
                interaction_values: &interaction_values,
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
    param_specs: &IndexMap<String, CompiledParamSpec>,
    interaction: &event::InteractionColumnRequests,
) -> Arc<Schema> {
    let mut fields = vec![
        Field::new(event::EVENT_TYPE_FIELD, DataType::Utf8, true),
        Field::new(event::EVENT_X_FIELD, DataType::Float64, true),
        Field::new(event::EVENT_Y_FIELD, DataType::Float64, true),
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
        let data_type = spec.default.data_type();
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

    // Derived coordinate columns invert to a single channel value (Float64).
    for channel in interaction.current_coord.iter() {
        fields.push(Field::new(
            event::event_coord_column_name(channel),
            DataType::Float64,
            true,
        ));
    }
    for channel in interaction.start_coord.iter() {
        fields.push(Field::new(
            event::start_coord_column_name(channel),
            DataType::Float64,
            true,
        ));
    }
    for channel in interaction.event_at_start_coord.iter() {
        fields.push(Field::new(
            event::event_at_start_coord_column_name(channel),
            DataType::Float64,
            true,
        ));
    }
    for channel in interaction.event_at_start_clipped_coord.iter() {
        fields.push(Field::new(
            event::event_at_start_clipped_coord_column_name(channel),
            DataType::Float64,
            true,
        ));
    }
    for channel in interaction.previous_coord.iter() {
        fields.push(Field::new(
            event::previous_coord_column_name(channel),
            DataType::Float64,
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

    schema_from_fields(fields)
}

/// Pre-resolved inputs for building a one-row event batch.
struct EventBatchInputs<'a> {
    current_params: &'a IndexMap<String, ScalarValue>,
    start_params: Option<&'a IndexMap<String, ScalarValue>>,
    previous_params: Option<&'a IndexMap<String, ScalarValue>>,
    interaction_values: &'a HashMap<String, ScalarValue>,
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
    use std::collections::BTreeSet;

    use avenger_chart::layout::LayoutBounds;
    use avenger_chart::prelude::*;
    use avenger_chart::render::{InteractionScopeId, InteractionScopeKind};
    use avenger_eventstream::{
        scene::{
            SceneCursorMovedEvent, SceneDoubleClickEvent, SceneMouseDownEvent, SceneMouseUpEvent,
        },
        window::{CanvasResizeEvent, MouseButton},
    };

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
            coord_transform: Box::new(Cartesian),
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
    fn assignment_writable_allows_raw_domain_default_reset() {
        let default = Param::raw_domain("x_domain").default;
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
        let compiled = Plot::<Cartesian>::new()
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
        faceted_pan_state_and_handler_with_sharing(Sharing::Shared).await
    }

    async fn faceted_pan_state_and_handler_with_sharing(
        sharing: Sharing,
    ) -> (ChartAppState, ChartEventBindingHandler) {
        let ctx = SessionContext::new();
        let x_domain = Param::raw_domain("x_domain");
        // A `Shared` scale drives one root domain for all cells; a `Free` scale
        // pans only the cell under the pointer. The tool mirrors this sharing
        // for its raw-domain param.
        let share_scale = sharing.to_level() == u8::MAX;
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('A', 0.0, 0.0), ('A', 10.0, 10.0),
                    ('B', 0.0, 1.0), ('B', 10.0, 9.0)
                ) AS t(group_name, x, y)",
            )
            .await
            .expect("data");
        let compiled = Plot::<FacetColumn>::new()
            .canvas_size(640.0, 320.0)
            .data(df)
            .mark(
                Subplot::new(
                    Plot::<Cartesian>::new()
                        .mark(
                            Symbol::new()
                                .x_with(col("x"), move |c| {
                                    let c = c.scale_with::<Linear>(|s| s.nice(false).zero(false));
                                    if share_scale { c.share_scale() } else { c }
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
        let runtime = CompiledChartEventBinding::compile(
            binding_index,
            &compiled.event_bindings()[binding_index],
            ctx,
            compiled.param_specs(),
            compiled.selection_specs(),
            compiled.store_specs(),
            compiled.cursor_params(),
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
        let compiled = Plot::<Cartesian>::new()
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
                    .assignments
                    .iter()
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
            compiled.cursor_params(),
        )
        .expect("compile release binding runtime");
        let reset_runtime = CompiledChartEventBinding::compile(
            reset_index,
            &compiled.event_bindings()[reset_index],
            &ctx,
            compiled.param_specs(),
            compiled.selection_specs(),
            compiled.store_specs(),
            compiled.cursor_params(),
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
        use avenger_app::app::SceneGraphBuilder;

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
        use avenger_app::app::SceneGraphBuilder;

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
        use avenger_app::app::SceneGraphBuilder;

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
        use avenger_app::app::SceneGraphBuilder;

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
        use avenger_app::app::SceneGraphBuilder;

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
        use avenger_app::app::SceneGraphBuilder;

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
        use avenger_app::app::SceneGraphBuilder;

        let (mut state, handler) = faceted_pan_state_and_handler_with_sharing(Sharing::Free).await;
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

    async fn bound_state(binding: ChartEventBinding) -> ChartAppState {
        let ctx = SessionContext::new();
        let width = Param::new("width", ScalarValue::Float64(Some(640.0)));
        let compiled = Plot::<Cartesian>::new()
            .add_param(width.clone())
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
        let compiled = Plot::<Cartesian>::new()
            .add_param(Param::new("width", ScalarValue::Float64(Some(640.0))))
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
            compiled.cursor_params(),
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
            .set_param(&x_domain, lit(x_domain.default.clone()))
            .exact();
        let compiled = Plot::<Cartesian>::new()
            .canvas_size(400.0, 300.0)
            .add_param(x_domain)
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
            compiled.cursor_params(),
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
        assert_eq!(metrics.event_batches_evaluated, 1);
        assert_eq!(metrics.param_patch_events, 1);
        assert_eq!(metrics.params_patched, 1);
    }

    #[tokio::test]
    async fn cursor_only_patch_updates_cursor_without_rerender() {
        let ctx = SessionContext::new();
        let cursor = Param::cursor("cursor", CursorStyle::Default);
        let compiled = Plot::<Cartesian>::new()
            .add_param(cursor.clone())
            .cursor_param(cursor.name.clone())
            .event_binding(
                ChartEventBinding::on(ChartEventType::CursorMoved)
                    .set_param(&cursor, event::cursor(CursorStyle::Crosshair))
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
            compiled.cursor_params(),
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
        assert_eq!(
            state.params().await.get("cursor"),
            Some(&ScalarValue::Utf8(Some("crosshair".to_string())))
        );
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
        let compiled = Plot::<Cartesian>::new()
            .add_store(
                Store::empty("brush_boxes")
                    .field("id", DataType::Utf8, false)
                    .field("x_min", DataType::Float64, false)
                    .field("x_max", DataType::Float64, false)
                    .primary_key(["id"])
                    .sharing(Sharing::Shared),
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
            compiled.cursor_params(),
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
                .facet_scope(Sharing::Shared)
                .dimension(col("x"))
                .endpoints(event::canvas_width(), event::canvas_height())]),
            )
            .preview();
        let compiled = Plot::<Cartesian>::new()
            .add_selection(Selection::new("brush").empty_selects_nothing())
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
            compiled.cursor_params(),
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
            assert_eq!(clause.scope.sharing, Sharing::Shared);
            assert!(clause.scope.owner_path.is_empty());
            assert!(clause.facet_context.is_empty());
            let SelectionPredicateSpec::Interval { dimensions } = &clause.predicate;
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
                .facet_scope(Sharing::Free)
                .dimension(col("x"))
                .endpoints(lit(1.0), lit(3.0))]),
            )
            .preview();
        let compiled = Plot::<Cartesian>::new()
            .canvas_size(400.0, 300.0)
            .add_selection(
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
            compiled.cursor_params(),
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
        assert_eq!(clause.scope.sharing, Sharing::Free);
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
        let compiled = Plot::<Cartesian>::new()
            .canvas_size(400.0, 300.0)
            .add_store(
                Store::empty("brush_boxes")
                    .field("id", DataType::Utf8, false)
                    .field("x_min", DataType::Float64, false)
                    .primary_key(["id"])
                    .sharing(Sharing::Free),
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
            compiled.cursor_params(),
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
        let compiled = Plot::<Cartesian>::new()
            .canvas_size(400.0, 300.0)
            .add_store(
                Store::empty("brush_boxes")
                    .field("id", DataType::Utf8, false)
                    .field("x_min", DataType::Float64, false)
                    .primary_key(["id"])
                    .sharing(Sharing::Free),
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
            compiled.cursor_params(),
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
                }]);
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
    async fn low_level_stream_filter_rejects_param_columns() {
        let binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .between(
                ChartEventStream::on(ChartEventType::MouseDown)
                    .filter(event::start_param("width").gt(lit(0.0))),
                ChartEventStream::on(ChartEventType::MouseUp),
            )
            .set_param("width", event::x())
            .preview();
        let ctx = SessionContext::new();
        let compiled = Plot::<Cartesian>::new()
            .add_param(Param::new("width", ScalarValue::Float64(Some(640.0))))
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile");

        let err = match CompiledChartEventBinding::compile(
            0,
            compiled.event_bindings().first().unwrap(),
            &ctx,
            compiled.param_specs(),
            compiled.selection_specs(),
            compiled.store_specs(),
            compiled.cursor_params(),
        ) {
            Ok(_) => panic!("param columns should be rejected in low-level filters"),
            Err(err) => err,
        };

        assert!(
            err.to_string()
                .contains("Unknown physical scalar expression column")
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
        let compiled = Plot::<Cartesian>::new()
            .add_param(x_domain.clone())
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
            compiled.cursor_params(),
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
        let compiled = Plot::<Cartesian>::new()
            .add_param(Param::new("width", ScalarValue::Float64(Some(640.0))))
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
            compiled.cursor_params(),
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
}
