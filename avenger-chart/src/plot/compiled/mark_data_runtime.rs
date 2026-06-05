//! Shared mark-channel preparation for measurement and rendering.
//!
//! Rendering and coordinate-system measurement sometimes need the same prepared
//! channel batches. Keeping this logic here avoids duplicating scale expression
//! handling between mark rendering and child-frame container measurement.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use avenger_common::types::ColorOrGradient;
use datafusion::{
    arrow::{
        array::Int32Array,
        compute::concat_batches,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    common::{DFSchema, ScalarValue},
    dataframe::DataFrame,
    logical_expr::{EmptyRelation, Expr, LogicalPlan, col, lit, when},
    prelude::SessionContext,
};
use datafusion_common::tree_node::{Transformed, TreeNode};
use datafusion_proto::protobuf::{LogicalExprNode, LogicalPlanNode};
use indexmap::IndexMap;

use avenger_chart_core::{
    CompiledSelectionSpec, DataTransformExecutionContext, DataTransformStage, DerivedScalarMap,
    FacetDataScope, MarkDataMode, SelectionClause, SelectionCombine, SelectionPredicateSpec,
    SharingLevel, color::parse_color_string, contains_aggregate, params_to_datafusion,
    selection_clause_value_id_from_placeholder, selection_id_from_predicate_placeholder,
};

use crate::{
    channel::{
        resolution::resolve_all_channel_refs,
        value::{ChannelValue, ConditionalValue, strip_trailing_numbers},
    },
    error::AvengerChartError,
    facet::data_scope::{FacetDataScopeContext, inherited_data_for_scope},
    marks::CompiledMark,
    render::{EvaluationContext, EvaluationMetrics, RenderState},
    scales::{ConfiguredScaleDataFusionExt, ConfiguredScaleWithSpec},
    serialization::{LogicalExprNodeExt, LogicalPlanNodeExt},
};

/// Mark data and channels after container data scope and aggregate preparation.
#[derive(Clone)]
pub(crate) struct PreparedLogicalMarkData {
    pub(crate) dataframe: Option<DataFrame>,
    pub(crate) channels: IndexMap<String, ChannelValue>,
    /// Data after scope selection and transforms, before aggregate channel
    /// preparation. Scale-domain inference can use this for non-aggregate
    /// channels so transform-produced helper columns remain available.
    pub(crate) domain_dataframe: Option<DataFrame>,
    pub(crate) domain_channels: IndexMap<String, ChannelValue>,
    pub(crate) derived_scalars: DerivedScalarMap,
}

/// Prepared data for mark evaluation.
pub(crate) struct PreparedMarkData {
    /// Array data batch (multiple rows), or None if all channels are scalar.
    pub(crate) data_batch: Option<RecordBatch>,
    /// Requested logical datum rows in rendered instance order.
    pub(crate) event_datum_batch: Option<RecordBatch>,
    /// Scalar data batch (single row) for channels that do not vary per mark.
    pub(crate) scalar_batch: RecordBatch,
    /// Render state with plot dimensions and configured scales.
    pub(crate) render_state: RenderState,
}

pub(crate) struct MarkDataRequest<'a> {
    pub(crate) mark: &'a dyn CompiledMark,
    pub(crate) plot_data: Option<&'a LogicalPlanNode>,
    pub(crate) provided_plot_df: Option<&'a DataFrame>,
    pub(crate) facet_data_scope: Option<FacetDataScopeContext<'a>>,
    pub(crate) prepared_logical: Option<&'a PreparedLogicalMarkData>,
    pub(crate) eval_ctx: &'a EvaluationContext,
    pub(crate) evaluation_metrics: Option<Arc<Mutex<EvaluationMetrics>>>,
    pub(crate) scales: &'a HashMap<String, ConfiguredScaleWithSpec>,
    pub(crate) plot_width: f32,
    pub(crate) plot_height: f32,
}

pub(crate) struct LogicalMarkDataRequest<'a> {
    pub(crate) mark: &'a dyn CompiledMark,
    pub(crate) plot_data: Option<&'a LogicalPlanNode>,
    pub(crate) provided_plot_df: Option<&'a DataFrame>,
    pub(crate) facet_data_scope: Option<FacetDataScopeContext<'a>>,
    pub(crate) eval_ctx: &'a EvaluationContext,
}

fn record_mark_data_full_collect(metrics: &Option<Arc<Mutex<EvaluationMetrics>>>) {
    if let Some(metrics) = metrics {
        metrics
            .lock()
            .expect("evaluation metrics lock poisoned")
            .record_mark_data_full_collect();
    }
}

fn record_mark_data_array_collect(metrics: &Option<Arc<Mutex<EvaluationMetrics>>>) {
    if let Some(metrics) = metrics {
        metrics
            .lock()
            .expect("evaluation metrics lock poisoned")
            .record_mark_data_array_collect();
    }
}

fn record_mark_data_scalar_collect(metrics: &Option<Arc<Mutex<EvaluationMetrics>>>) {
    if let Some(metrics) = metrics {
        metrics
            .lock()
            .expect("evaluation metrics lock poisoned")
            .record_mark_data_scalar_collect();
    }
}

fn channel_exprs_reference_columns(
    channels: &IndexMap<String, ChannelValue>,
    ctx: &SessionContext,
) -> bool {
    channels.values().any(|channel_value| {
        channel_value
            .all_exprs(ctx)
            .into_iter()
            .any(|expr| !expr.column_refs().is_empty())
    })
}

fn aggregate_channels_need_preparation(
    channels: &IndexMap<String, ChannelValue>,
    ctx: &SessionContext,
) -> bool {
    channels
        .values()
        .flat_map(|channel_value| channel_value.all_exprs(ctx))
        .any(|expr| contains_aggregate(&expr))
}

async fn apply_mark_data_transforms(
    dataframe: Option<DataFrame>,
    transforms: &[DataTransformStage],
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    facet_data_scope: Option<FacetDataScopeContext<'_>>,
    mark_facet_data_scope: FacetDataScope,
) -> Result<(Option<DataFrame>, DerivedScalarMap), AvengerChartError> {
    if transforms.is_empty() {
        return Ok((dataframe, DerivedScalarMap::new()));
    }
    let dataframe = dataframe.unwrap_or_else(|| empty_dataframe(ctx));
    let transforms = scoped_transform_stages(transforms, mark_facet_data_scope)?;
    let transform_ctx = DataTransformExecutionContext {
        session_context: ctx,
        params,
    };
    let mut dataframe = dataframe;
    let mut derived_scalars = DerivedScalarMap::new();
    let mut current_level = transforms
        .first()
        .map(|stage| stage.level)
        .unwrap_or_else(|| mark_facet_data_scope.sharing_level());

    dataframe = filter_dataframe_to_transform_scope(
        dataframe,
        facet_data_scope,
        current_level,
        "initial transform scope",
    )?;

    for stage in transforms {
        if stage.level > current_level {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Transform stage scope {:?} is broader than the preceding stage scope {:?}; transform scopes must stay the same or get narrower through a chain",
                stage.level, current_level
            )));
        }
        if stage.level < current_level {
            dataframe = filter_dataframe_to_transform_scope(
                dataframe,
                facet_data_scope,
                stage.level,
                "narrower transform scope",
            )?;
            current_level = stage.level;
        }

        let result = stage.transform.apply(dataframe, &transform_ctx).await?;
        dataframe = result.dataframe;
        for (id, expr) in result.derived_scalars {
            if derived_scalars.insert(id.clone(), expr).is_some() {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Derived scalar '{id}' was produced more than once in the same data scope"
                )));
            }
        }
    }

    let final_level = mark_facet_data_scope.sharing_level();
    if final_level < current_level {
        dataframe = filter_dataframe_to_transform_scope(
            dataframe,
            facet_data_scope,
            final_level,
            "final mark facet data scope",
        )?;
    }

    Ok((Some(dataframe), derived_scalars))
}

struct ScopedTransformStage<'a> {
    level: SharingLevel,
    transform: &'a dyn avenger_chart_core::CompiledDataTransform,
}

fn scoped_transform_stages(
    transforms: &[DataTransformStage],
    mark_facet_data_scope: FacetDataScope,
) -> Result<Vec<ScopedTransformStage<'_>>, AvengerChartError> {
    let mark_level = mark_facet_data_scope.sharing_level();
    let mut stages = Vec::with_capacity(transforms.len());
    let mut previous_level: Option<SharingLevel> = None;
    for stage in transforms {
        let stage_level = SharingLevel::from(stage.scope);
        let effective_level = stage_level.max(mark_level);
        if let Some(previous_level) = previous_level
            && effective_level > previous_level
        {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Transform stage scope {:?} is broader than the preceding stage scope {:?}; transform scopes must stay the same or get narrower through a chain",
                effective_level, previous_level
            )));
        }
        previous_level = Some(effective_level);
        stages.push(ScopedTransformStage {
            level: effective_level,
            transform: stage.transform.as_ref(),
        });
    }
    Ok(stages)
}

fn transform_initial_facet_scope(
    transforms: &[DataTransformStage],
    mark_facet_data_scope: FacetDataScope,
) -> FacetDataScope {
    let mut level = mark_facet_data_scope.sharing_level();
    for stage in transforms {
        level = level.max(SharingLevel::from(stage.scope));
    }
    FacetDataScope::from_sharing_level(level)
}

fn filter_dataframe_to_transform_scope(
    dataframe: DataFrame,
    facet_data_scope: Option<FacetDataScopeContext<'_>>,
    level: SharingLevel,
    label: &str,
) -> Result<DataFrame, AvengerChartError> {
    let Some(scope) = facet_data_scope else {
        return Ok(dataframe);
    };
    if scope.full_path.is_empty() {
        return Ok(dataframe);
    }
    let Some(predicate) = scope
        .facet_tree
        .cell_predicate(scope.full_path, level.raw())
    else {
        return Ok(dataframe);
    };
    let available_columns = dataframe
        .schema()
        .fields()
        .iter()
        .map(|field| field.name().clone())
        .collect::<HashSet<_>>();
    if !expr_columns_are_available(&predicate, Some(&available_columns)) {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Transform output cannot be filtered to {label}; it no longer contains the facet columns required for this scope"
        )));
    }
    dataframe
        .filter(predicate)
        .map_err(AvengerChartError::DataFusionError)
}

fn validate_runtime_aggregate_channels(
    channels: &IndexMap<String, ChannelValue>,
    ctx: &SessionContext,
) -> Result<(), AvengerChartError> {
    for (channel_name, channel_value) in channels {
        if matches!(channel_value, ChannelValue::Conditional { .. })
            && channel_value
                .all_exprs(ctx)
                .into_iter()
                .any(|expr| contains_aggregate(&expr))
        {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Aggregate marks with aggregate expressions inside conditional channel `{channel_name}` are not supported yet"
            )));
        }
    }
    Ok(())
}

fn validate_runtime_aggregate_conditional_columns(
    channels: &IndexMap<String, ChannelValue>,
    ctx: &SessionContext,
    schema: &DFSchema,
) -> Result<(), AvengerChartError> {
    let available_columns = schema
        .fields()
        .iter()
        .map(|field| field.name().clone())
        .collect::<HashSet<_>>();
    for (channel_name, channel_value) in channels {
        if !matches!(channel_value, ChannelValue::Conditional { .. }) {
            continue;
        }
        for expr in channel_value.all_exprs(ctx) {
            if !expr_columns_are_available(&expr, Some(&available_columns)) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Aggregate mark conditional channel `{channel_name}` references columns that are not available after aggregation"
                )));
            }
        }
    }
    Ok(())
}

fn expand_selection_predicates_in_channels(
    channels: IndexMap<String, ChannelValue>,
    eval_ctx: &EvaluationContext,
    available_columns: Option<&HashSet<String>>,
) -> Result<IndexMap<String, ChannelValue>, AvengerChartError> {
    if eval_ctx.scoped_selection_store.is_none() {
        return Ok(channels);
    }
    let ctx = eval_ctx.session_context.as_ref();
    let mut updated = IndexMap::new();
    for (name, value) in channels {
        let mapped = match value {
            ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                scale_config,
                legend_config,
                axis_config,
                share_mode,
                transform_scope,
            } => {
                let expanded =
                    expand_selection_predicates(expr.to_expr(ctx)?, eval_ctx, available_columns)?;
                ChannelValue::Scaled {
                    expr: LogicalExprNode::from_expr(expanded)?,
                    scale_name,
                    band,
                    scale_config,
                    legend_config,
                    axis_config,
                    share_mode,
                    transform_scope,
                }
            }
            ChannelValue::Value { expr } => {
                let expanded =
                    expand_selection_predicates(expr.to_expr(ctx)?, eval_ctx, available_columns)?;
                ChannelValue::Value {
                    expr: LogicalExprNode::from_expr(expanded)?,
                }
            }
            ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                legend_config,
                axis_config,
                share_mode,
                transform_scope,
            } => {
                let expanded_conditions = conditions
                    .into_iter()
                    .map(|(condition, value)| {
                        let condition = expand_selection_predicates(
                            condition.to_expr(ctx)?,
                            eval_ctx,
                            available_columns,
                        )?;
                        let value = match value {
                            ConditionalValue::Scaled { expr } => ConditionalValue::Scaled {
                                expr: LogicalExprNode::from_expr(expand_selection_predicates(
                                    expr.to_expr(ctx)?,
                                    eval_ctx,
                                    available_columns,
                                )?)?,
                            },
                            ConditionalValue::Value { expr } => ConditionalValue::Value {
                                expr: LogicalExprNode::from_expr(expand_selection_predicates(
                                    expr.to_expr(ctx)?,
                                    eval_ctx,
                                    available_columns,
                                )?)?,
                            },
                        };
                        Ok::<_, AvengerChartError>((LogicalExprNode::from_expr(condition)?, value))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let expanded_otherwise = match otherwise {
                    ConditionalValue::Scaled { expr } => ConditionalValue::Scaled {
                        expr: LogicalExprNode::from_expr(expand_selection_predicates(
                            expr.to_expr(ctx)?,
                            eval_ctx,
                            available_columns,
                        )?)?,
                    },
                    ConditionalValue::Value { expr } => ConditionalValue::Value {
                        expr: LogicalExprNode::from_expr(expand_selection_predicates(
                            expr.to_expr(ctx)?,
                            eval_ctx,
                            available_columns,
                        )?)?,
                    },
                };
                ChannelValue::Conditional {
                    conditions: expanded_conditions,
                    otherwise: expanded_otherwise,
                    scale_config,
                    legend_config,
                    axis_config,
                    share_mode,
                    transform_scope,
                }
            }
        };
        updated.insert(name, mapped);
    }
    Ok(updated)
}

pub(crate) fn expand_selection_predicates(
    expr: Expr,
    eval_ctx: &EvaluationContext,
    available_columns: Option<&HashSet<String>>,
) -> Result<Expr, AvengerChartError> {
    expand_selection_predicates_with_fallback_specs(expr, eval_ctx, available_columns, None)
}

pub(crate) fn expand_selection_predicates_with_fallback_specs(
    expr: Expr,
    eval_ctx: &EvaluationContext,
    available_columns: Option<&HashSet<String>>,
    fallback_specs: Option<&IndexMap<String, CompiledSelectionSpec>>,
) -> Result<Expr, AvengerChartError> {
    let ctx = eval_ctx.session_context.as_ref();
    expr.transform(|candidate| {
        if let Expr::Placeholder(placeholder) = &candidate
            && let Some(selection_id) = selection_id_from_predicate_placeholder(&placeholder.id)
        {
            let replacement = selection_predicate_expr(
                selection_id,
                eval_ctx,
                ctx,
                available_columns,
                fallback_specs,
            )
            .map_err(|err| datafusion::error::DataFusionError::Plan(err.to_string()))?;
            return Ok(Transformed::yes(replacement));
        }
        Ok(Transformed::no(candidate))
    })
    .map(|transformed| transformed.data)
    .map_err(AvengerChartError::DataFusionError)
}

fn selection_predicate_expr(
    selection_id: &str,
    eval_ctx: &EvaluationContext,
    ctx: &SessionContext,
    available_columns: Option<&HashSet<String>>,
    fallback_specs: Option<&IndexMap<String, CompiledSelectionSpec>>,
) -> Result<Expr, AvengerChartError> {
    let (spec, clauses) = if let Some(selection_store) = eval_ctx.scoped_selection_store.as_ref() {
        let Some(spec) = selection_store.specs().get(selection_id) else {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Selection predicate references unknown selection '{selection_id}'"
            )));
        };
        let clauses = selection_store
            .clauses_for_selection(selection_id)
            .unwrap_or_default();
        (spec, clauses)
    } else if let Some(specs) = fallback_specs {
        let Some(spec) = specs.get(selection_id) else {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Selection predicate references unknown selection '{selection_id}'"
            )));
        };
        (spec, Vec::new())
    } else {
        return Ok(lit(false));
    };
    if clauses.is_empty() {
        return Ok(lit(matches!(
            spec.empty,
            avenger_chart_core::EmptySelectionBehavior::SelectAll
        )));
    }

    let mut exprs = clauses
        .iter()
        .map(|clause| selection_clause_predicate_expr(spec, clause, ctx, available_columns));
    let mut result = exprs.next().transpose()?.unwrap_or_else(|| lit(false));
    for expr in exprs {
        result = match spec.combine {
            SelectionCombine::Union => result.or(expr?),
            SelectionCombine::Intersect => result.and(expr?),
        };
    }
    Ok(result)
}

fn selection_clause_predicate_expr(
    spec: &CompiledSelectionSpec,
    clause: &SelectionClause,
    ctx: &SessionContext,
    available_columns: Option<&HashSet<String>>,
) -> Result<Expr, AvengerChartError> {
    let mut expr = selection_clause_row_predicate_expr(&clause.predicate, ctx, available_columns)?;
    for facet_value in &clause.facet_context {
        if facet_value.value.is_null() {
            continue;
        }
        if let Some(facet) = spec
            .facet_context
            .iter()
            .find(|facet| facet.id == facet_value.id)
        {
            let facet_expr = facet.field_expr.to_expr(ctx)?;
            if !expr_columns_are_available(&facet_expr, available_columns) {
                return Ok(lit(false));
            }
            expr = expr.and(facet_expr.eq(lit(facet_value.value.clone())));
        }
    }
    Ok(expr)
}

fn selection_clause_row_predicate_expr(
    predicate: &SelectionPredicateSpec,
    ctx: &SessionContext,
    available_columns: Option<&HashSet<String>>,
) -> Result<Expr, AvengerChartError> {
    match predicate {
        SelectionPredicateSpec::Interval { dimensions } => {
            interval_selection_clause_expr(dimensions, ctx, available_columns)
        }
        SelectionPredicateSpec::Equality { dimensions } => {
            equality_selection_clause_expr(dimensions, ctx, available_columns)
        }
        SelectionPredicateSpec::Predicate { values, expr, .. } => {
            generic_selection_clause_expr(values, expr, ctx, available_columns)
        }
    }
}

fn interval_selection_clause_expr(
    dimensions: &[avenger_chart_core::SelectionIntervalDimensionValue],
    ctx: &SessionContext,
    available_columns: Option<&HashSet<String>>,
) -> Result<Expr, AvengerChartError> {
    if dimensions.is_empty() {
        return Ok(lit(false));
    }
    let mut expr = lit(true);
    for dimension in dimensions {
        if dimension.min.is_null() || dimension.max.is_null() {
            return Ok(lit(false));
        }
        let value = dimension.field_expr.to_expr(ctx)?;
        if !expr_columns_are_available(&value, available_columns) {
            return Ok(lit(false));
        }
        expr = expr
            .and(value.clone().gt_eq(lit(dimension.min.clone())))
            .and(value.lt_eq(lit(dimension.max.clone())));
    }
    Ok(expr)
}

fn equality_selection_clause_expr(
    dimensions: &[avenger_chart_core::SelectionEqualityDimensionValue],
    ctx: &SessionContext,
    available_columns: Option<&HashSet<String>>,
) -> Result<Expr, AvengerChartError> {
    if dimensions.is_empty() {
        return Ok(lit(false));
    }
    let mut expr = lit(true);
    for dimension in dimensions {
        if dimension.value.is_null() {
            return Ok(lit(false));
        }
        let value = dimension.field_expr.to_expr(ctx)?;
        if !expr_columns_are_available(&value, available_columns) {
            return Ok(lit(false));
        }
        expr = expr.and(value.eq(lit(dimension.value.clone())));
    }
    Ok(expr)
}

fn generic_selection_clause_expr(
    values: &[avenger_chart_core::SelectionPredicateValue],
    expr: &LogicalExprNode,
    ctx: &SessionContext,
    available_columns: Option<&HashSet<String>>,
) -> Result<Expr, AvengerChartError> {
    let has_null_value = values.iter().any(|value| value.value.is_null());
    let value_map = values
        .iter()
        .map(|value| (value.id.as_str(), value.value.clone()))
        .collect::<HashMap<_, _>>();
    let expr = expr.to_expr(ctx)?;
    let expr = expr
        .transform(|candidate| {
            if let Expr::Placeholder(placeholder) = &candidate
                && let Some(value_id) = selection_clause_value_id_from_placeholder(&placeholder.id)
            {
                let Some(value) = value_map.get(value_id) else {
                    return Err(datafusion::error::DataFusionError::Plan(format!(
                        "Selection predicate references undeclared clause value '{value_id}'"
                    )));
                };
                return Ok(Transformed::yes(lit(value.clone())));
            }
            Ok(Transformed::no(candidate))
        })
        .map(|transformed| transformed.data)
        .map_err(AvengerChartError::DataFusionError)?;
    if has_null_value {
        return Ok(lit(false));
    }
    if !expr_columns_are_available(&expr, available_columns) {
        return Ok(lit(false));
    }
    Ok(expr)
}

fn expr_columns_are_available(expr: &Expr, available_columns: Option<&HashSet<String>>) -> bool {
    let refs = expr.column_refs();
    if refs.is_empty() {
        return true;
    }
    let Some(available_columns) = available_columns else {
        return false;
    };
    refs.into_iter()
        .all(|column| available_columns.contains(column.name()))
}

fn store_dataframe(
    data: &avenger_chart_core::StoreData,
    facet_data_scope: Option<FacetDataScopeContext<'_>>,
    eval_ctx: &EvaluationContext,
) -> Result<DataFrame, AvengerChartError> {
    let ctx = eval_ctx.session_context.as_ref();
    let Some(store_state) = eval_ctx.scoped_store_state.as_ref() else {
        return Ok(empty_dataframe(ctx));
    };
    let sharing_owner_paths = store_data_sharing_owner_paths(eval_ctx, facet_data_scope);
    let batch = store_state.materialize_store_data(data, &sharing_owner_paths)?;
    Ok(ctx.read_batch(batch)?)
}

fn store_data_sharing_owner_paths(
    eval_ctx: &EvaluationContext,
    facet_data_scope: Option<FacetDataScopeContext<'_>>,
) -> HashMap<u8, Vec<ScalarValue>> {
    let Some(scope) = facet_data_scope else {
        return HashMap::new();
    };
    if scope.full_path.is_empty() {
        return HashMap::new();
    }
    let logical_depth = eval_ctx.facet_tree.logical_depth_for_path(scope.full_path);
    let mut owner_paths = HashMap::new();
    for level in 0..=logical_depth.min(u8::MAX as usize) {
        let level = level as u8;
        owner_paths.insert(
            level,
            eval_ctx
                .facet_tree
                .sharing_owner_path(scope.full_path, level),
        );
    }
    owner_paths
}

fn dataframe_for_mark(
    mark: &dyn CompiledMark,
    plot_data: Option<&LogicalPlanNode>,
    provided_plot_df: Option<&DataFrame>,
    facet_data_scope: Option<FacetDataScopeContext<'_>>,
    mark_facet_data_scope: FacetDataScope,
    channels: &IndexMap<String, ChannelValue>,
    ctx: &SessionContext,
    eval_ctx: &EvaluationContext,
) -> Result<Option<DataFrame>, AvengerChartError> {
    if let Some(store_data) = mark.data_context().store_data() {
        return store_dataframe(store_data, facet_data_scope, eval_ctx).map(Some);
    }

    if mark.state().data_mode == MarkDataMode::Unit {
        return Ok(None);
    }

    if let Some(mark_df) = mark.data_context().dataframe_with_context(ctx) {
        return Ok(Some(mark_df));
    }

    if let Some(df_override) =
        inherited_data_for_scope(provided_plot_df, mark_facet_data_scope, facet_data_scope)?
    {
        return Ok(Some(df_override));
    }

    if !channel_exprs_reference_columns(channels, ctx) {
        return Ok(None);
    }
    if let Some(df) = plot_data.and_then(|node| {
        node.to_logical_plan(ctx)
            .ok()
            .map(|plan| DataFrame::new(ctx.state().clone(), plan))
    }) {
        return Ok(Some(df));
    }
    Err(AvengerChartError::InternalError(
        "Mark expressions reference columns but no data is available".to_string(),
    ))
}

fn empty_dataframe(ctx: &SessionContext) -> DataFrame {
    DataFrame::new(
        ctx.state().clone(),
        LogicalPlan::EmptyRelation(EmptyRelation {
            produce_one_row: false,
            schema: Arc::new(DFSchema::empty()),
        }),
    )
}

/// Resolve channel references and run aggregate channel preparation on the
/// selected mark data. This intentionally happens at runtime so faceted marks
/// aggregate after their facet data scope has been selected.
pub(crate) async fn prepare_logical_mark_data(
    request: LogicalMarkDataRequest<'_>,
) -> Result<PreparedLogicalMarkData, AvengerChartError> {
    let ctx = request.eval_ctx.session_context.as_ref();
    let channels = resolve_all_channel_refs(request.mark.data_context().channels(), ctx)?;
    let transform_initial_scope = transform_initial_facet_scope(
        request.mark.data_context().transforms(),
        request.mark.state().facet_data_scope,
    );
    let dataframe = dataframe_for_mark(
        request.mark,
        request.plot_data,
        request.provided_plot_df,
        request.facet_data_scope,
        transform_initial_scope,
        &channels,
        ctx,
        request.eval_ctx,
    )?;
    let (dataframe, derived_scalars) = apply_mark_data_transforms(
        dataframe,
        request.mark.data_context().transforms(),
        ctx,
        request.eval_ctx.params(),
        request.facet_data_scope,
        request.mark.state().facet_data_scope,
    )
    .await?;
    let available_columns = dataframe.as_ref().map(|df| {
        df.schema()
            .fields()
            .iter()
            .map(|field| field.name().clone())
            .collect::<HashSet<_>>()
    });
    let channels = expand_selection_predicates_in_channels(
        channels,
        request.eval_ctx,
        available_columns.as_ref(),
    )?;
    let domain_dataframe = dataframe.clone();
    let domain_channels = channels.clone();

    if !aggregate_channels_need_preparation(&channels, ctx) {
        return Ok(PreparedLogicalMarkData {
            dataframe,
            channels,
            domain_dataframe,
            domain_channels,
            derived_scalars,
        });
    }

    validate_runtime_aggregate_channels(&channels, ctx)?;
    let df = dataframe.unwrap_or_else(|| empty_dataframe(ctx));

    let mut unique_group_exprs = IndexMap::new();
    let mut unique_agg_exprs = IndexMap::new();
    enum AggregateChannelInfo {
        Direct {
            name: String,
            expr: Expr,
            is_aggregate: bool,
            is_literal: bool,
            value: ChannelValue,
        },
        Conditional {
            name: String,
            value: ChannelValue,
        },
    }

    let mut channel_info: Vec<AggregateChannelInfo> = Vec::new();

    for (channel_name, channel_value) in &channels {
        if matches!(channel_value, ChannelValue::Conditional { .. }) {
            channel_info.push(AggregateChannelInfo::Conditional {
                name: channel_name.clone(),
                value: channel_value.clone(),
            });
            continue;
        }

        let Some(expr) = channel_value.expr(ctx) else {
            continue;
        };
        let is_aggregate = contains_aggregate(&expr);
        let is_literal = matches!(expr, Expr::Literal(_, _));

        if is_aggregate {
            if !unique_agg_exprs.contains_key(&expr) {
                unique_agg_exprs.insert(expr.clone(), unique_agg_exprs.len());
            }
        } else if !is_literal && !unique_group_exprs.contains_key(&expr) {
            unique_group_exprs.insert(expr.clone(), unique_group_exprs.len());
        }

        channel_info.push(AggregateChannelInfo::Direct {
            name: channel_name.clone(),
            expr,
            is_aggregate,
            is_literal,
            value: channel_value.clone(),
        });
    }

    if unique_agg_exprs.is_empty() {
        return Ok(PreparedLogicalMarkData {
            dataframe: Some(df),
            channels,
            domain_dataframe,
            domain_channels,
            derived_scalars,
        });
    }

    let group_by_exprs: Vec<Expr> = unique_group_exprs.keys().cloned().collect();
    let agg_exprs: Vec<Expr> = unique_agg_exprs.keys().cloned().collect();
    let agg_df = df.aggregate(group_by_exprs.clone(), agg_exprs.clone())?;
    let schema = agg_df.schema();
    validate_runtime_aggregate_conditional_columns(&channels, ctx, schema)?;
    let mut updated_channels = IndexMap::new();

    for info in channel_info {
        match info {
            AggregateChannelInfo::Conditional { name, value } => {
                updated_channels.insert(name, value);
            }
            AggregateChannelInfo::Direct {
                name,
                expr,
                is_aggregate,
                is_literal,
                value,
            } => {
                if is_literal {
                    updated_channels.insert(name, value);
                } else if is_aggregate {
                    let agg_index = unique_agg_exprs.get(&expr).unwrap();
                    let field_index = group_by_exprs.len() + agg_index;
                    let field_name = schema.field(field_index).name().clone();
                    let new_expr =
                        LogicalExprNode::from_expr(datafusion::prelude::col(&field_name))?;
                    updated_channels.insert(name, value.with_expr(new_expr));
                } else {
                    let group_index = unique_group_exprs.get(&expr).unwrap();
                    let field_name = schema.field(*group_index).name().clone();
                    let new_expr =
                        LogicalExprNode::from_expr(datafusion::prelude::col(&field_name))?;
                    updated_channels.insert(name, value.with_expr(new_expr));
                }
            }
        }
    }

    Ok(PreparedLogicalMarkData {
        dataframe: Some(agg_df),
        channels: updated_channels,
        domain_dataframe,
        domain_channels,
        derived_scalars,
    })
}

/// Apply a scale transformation to a channel expression.
pub(crate) fn apply_channel_scale(
    channel_name: &str,
    channel_value: &ChannelValue,
    scales: &HashMap<String, ConfiguredScaleWithSpec>,
    ctx: &SessionContext,
) -> Result<Expr, AvengerChartError> {
    match channel_value {
        ChannelValue::Value { expr } => expr.to_expr(ctx),
        ChannelValue::Conditional {
            conditions,
            otherwise,
            ..
        } => {
            let convert_color_literal = |expr: &Expr| -> Expr {
                if let Expr::Literal(scalar_value, _) = expr
                    && let ScalarValue::Utf8(Some(s)) = scalar_value
                    && let Some(color_or_gradient) = parse_color_string(s)
                    && let ColorOrGradient::Color(rgba) = color_or_gradient
                {
                    let values: Vec<ScalarValue> = rgba
                        .into_iter()
                        .map(|v| ScalarValue::Float32(Some(v)))
                        .collect();
                    let list_array = ScalarValue::new_list_nullable(&values, &DataType::Float32);
                    return lit(ScalarValue::List(list_array));
                }
                expr.clone()
            };

            let scale_key = strip_trailing_numbers(channel_name).to_string();
            let needs_color_conversion = matches!(channel_name, "fill" | "stroke" | "color");

            let apply_to_conditional =
                |cond_val: &ConditionalValue| -> Result<Expr, AvengerChartError> {
                    match cond_val {
                        ConditionalValue::Scaled { expr } => {
                            if let Some(scale) = scales.get(&scale_key) {
                                expr.to_expr(ctx).and_then(|e| scale.to_expr(e))
                            } else {
                                expr.to_expr(ctx)
                            }
                        }
                        ConditionalValue::Value { expr } => {
                            let expr_df = expr.to_expr(ctx)?;
                            if needs_color_conversion {
                                Ok(convert_color_literal(&expr_df))
                            } else {
                                Ok(expr_df)
                            }
                        }
                    }
                };

            let first_cond = &conditions[0];
            let first_value = apply_to_conditional(&first_cond.1)?;
            let first_cond_expr = first_cond.0.to_expr(ctx)?;
            let mut case_expr = when(first_cond_expr, first_value);

            for (condition, value) in &conditions[1..] {
                let scaled_value = apply_to_conditional(value)?;
                let condition_expr = condition.to_expr(ctx)?;
                case_expr = case_expr.when(condition_expr, scaled_value);
            }

            let otherwise_value = apply_to_conditional(otherwise)?;
            Ok(case_expr.otherwise(otherwise_value)?)
        }
        ChannelValue::Scaled {
            expr,
            scale_name,
            band,
            ..
        } => {
            let default_scale_name = strip_trailing_numbers(channel_name).to_string();
            let scale_key = scale_name.as_ref().unwrap_or(&default_scale_name);
            let scale = scales.get(scale_key).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Scale '{}' not found for channel '{}'",
                    scale_key, channel_name
                ))
            })?;

            let expr_df = expr.to_expr(ctx)?;
            if let Some(band) = band {
                scale.to_expr_with_band(expr_df.clone(), *band)
            } else {
                scale.to_expr(expr_df)
            }
        }
    }
}

/// Prepare data batches for a compiled mark.
pub(crate) async fn prepare_mark_data(
    request: MarkDataRequest<'_>,
) -> Result<Option<PreparedMarkData>, AvengerChartError> {
    let ctx = &*request.eval_ctx.session_context;
    let params = &request.eval_ctx.params;
    let mark = request.mark;

    let prepared_storage;
    let prepared_logical = if let Some(prepared) = request.prepared_logical {
        prepared
    } else {
        prepared_storage = prepare_logical_mark_data(LogicalMarkDataRequest {
            mark,
            plot_data: request.plot_data,
            provided_plot_df: request.provided_plot_df,
            facet_data_scope: request.facet_data_scope,
            eval_ctx: request.eval_ctx,
        })
        .await?;
        &prepared_storage
    };

    let channels = &prepared_logical.channels;
    let df_ref = prepared_logical.dataframe.clone();

    let df = if let Some(df_ref) = df_ref {
        if let Some(sort_channel_name) = mark.sorting_channel() {
            if let Some(sort_channel) = channels.get(sort_channel_name) {
                let sort_expr =
                    apply_channel_scale(sort_channel_name, sort_channel, request.scales, ctx)?;
                Arc::new(df_ref.sort(vec![sort_expr.sort(true, false)])?)
            } else {
                Arc::new(df_ref)
            }
        } else {
            Arc::new(df_ref)
        }
    } else {
        let empty_df = ctx
            .sql("SELECT 1 as _dummy")
            .await
            .map_err(AvengerChartError::DataFusionError)?;
        Arc::new(empty_df)
    };

    let supported_channels = mark.supported_channels();
    let mut array_channels = Vec::new();
    let mut scalar_channels = Vec::new();
    let mut has_array_data = false;
    for channel_desc in &supported_channels {
        if let Some(channel_value) = channels.get(channel_desc.name) {
            let scaled_expr =
                apply_channel_scale(channel_desc.name, channel_value, request.scales, ctx)?;
            if channel_desc.allow_column_ref && scaled_expr.any_column_refs() {
                array_channels.push((channel_desc.name, scaled_expr));
                has_array_data = true;
            } else {
                scalar_channels.push((channel_desc.name, scaled_expr));
            }
        }
    }

    let event_datum_batch = if !request.eval_ctx.event_datum_fields.is_empty() {
        let available_fields = df
            .schema()
            .fields()
            .iter()
            .map(|field| field.name().clone())
            .collect::<HashSet<_>>();
        let select_exprs = request
            .eval_ctx
            .event_datum_fields
            .keys()
            .filter(|field| available_fields.contains(*field))
            .map(|field| col(field).alias(field))
            .collect::<Vec<_>>();
        if select_exprs.is_empty() {
            None
        } else {
            let datafusion_params = params_to_datafusion(params);
            let batch = if let Some(param_values) = datafusion_params {
                (*df)
                    .clone()
                    .select(select_exprs)?
                    .with_param_values(param_values)?
                    .collect()
                    .await?
            } else {
                (*df).clone().select(select_exprs)?.collect().await?
            };
            if batch.is_empty() {
                None
            } else {
                let schema = batch[0].schema();
                Some(concat_batches(&schema, &batch)?)
            }
        }
    } else {
        None
    };

    let data_batch = if mark.wants_full_data_batch() {
        let datafusion_params = params_to_datafusion(params);
        record_mark_data_full_collect(&request.evaluation_metrics);
        let batch = if let Some(param_values) = datafusion_params {
            (*df)
                .clone()
                .with_param_values(param_values)?
                .collect()
                .await?
        } else {
            (*df).clone().collect().await?
        };

        if batch.is_empty() {
            let arrow_schema = std::sync::Arc::new(df.schema().as_arrow().clone());
            Some(RecordBatch::new_empty(arrow_schema))
        } else {
            let schema = batch[0].schema();
            Some(concat_batches(&schema, &batch)?)
        }
    } else if has_array_data {
        let mut select_exprs = vec![];
        for (name, expr) in &array_channels {
            select_exprs.push(expr.clone().alias(*name));
        }
        let datafusion_params = params_to_datafusion(params);
        record_mark_data_array_collect(&request.evaluation_metrics);
        let batch = if let Some(param_values) = datafusion_params {
            (*df)
                .clone()
                .select(select_exprs)?
                .with_param_values(param_values)?
                .collect()
                .await?
        } else {
            (*df).clone().select(select_exprs)?.collect().await?
        };
        if batch.is_empty() {
            None
        } else {
            let schema = batch[0].schema();
            Some(concat_batches(&schema, &batch)?)
        }
    } else {
        None
    };

    let mut scalar_select_exprs = vec![];
    for (name, expr) in &scalar_channels {
        scalar_select_exprs.push(expr.clone().alias(*name));
    }
    let scalar_batch = if !scalar_select_exprs.is_empty() {
        let datafusion_params = params_to_datafusion(params);
        record_mark_data_scalar_collect(&request.evaluation_metrics);
        let batch = if let Some(param_values) = datafusion_params {
            (*df)
                .clone()
                .select(scalar_select_exprs)?
                .with_param_values(param_values)?
                .collect()
                .await?
        } else {
            (*df).clone().select(scalar_select_exprs)?.collect().await?
        };
        if batch.is_empty() {
            return Ok(None);
        } else {
            let schema = batch[0].schema();
            concat_batches(&schema, &batch)?
        }
    } else {
        RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new(
                "_dummy",
                DataType::Int32,
                false,
            )])),
            vec![Arc::new(Int32Array::from(vec![0]))],
        )?
    };

    Ok(Some(PreparedMarkData {
        data_batch,
        event_datum_batch,
        scalar_batch,
        render_state: RenderState::new(
            request.plot_width,
            request.plot_height,
            request.scales.clone(),
        ),
    }))
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use avenger_chart_transforms::{
        Aggregate, Bin, Calculate, Filter, Fold, JoinAggregate, Select, Window,
    };
    use avenger_scales::scales::{ConfiguredScale, ScaleConfig};
    use datafusion::{
        arrow::{
            array::Float64Array,
            compute::cast,
            datatypes::{DataType, Field, Schema},
            record_batch::RecordBatch,
        },
        functions_aggregate::average::avg,
        functions_window::expr_fn::row_number,
        logical_expr::{Expr, col},
        prelude::SessionContext,
    };
    use datafusion_proto::protobuf::{LogicalExprNode, LogicalPlanNode};
    use indexmap::IndexMap;

    use super::*;
    use crate::{
        cartesian::{Cartesian, CartesianRectPositionChannels, CartesianSymbolPositionChannels},
        concat::HConcat,
        error::AvengerChartError,
        facet::{
            coord::{FacetColumn, FacetRow},
            data_scope::FacetDataScopeContext,
            evaluated_facet_tree::EvaluatedFacetTree,
            marks::facet::{FacetColumnSubplotChannels, FacetRowSubplotChannels},
        },
        marks::{ChannelValue, Mark, Subplot, symbol::Symbol},
        plot::{
            Plot,
            compiled::session::{ScopedStoreAssignment, ScopedStoreState, StoreStateUpdate},
        },
        scales::{Linear, Scale, ScaleRangeBinding, ScaleSpec},
        serialization::{LogicalExprNodeExt, LogicalPlanNodeExt},
        theme::Theme,
        zerod::ZeroDCoord,
    };
    use avenger_chart_core::{
        STORE_NAME_COLUMN, STORE_OWNER_KEY_COLUMN, STORE_REVISION_COLUMN, Sharing, Store,
        StoreData, StoreRowValue,
    };
    use avenger_chart_marks::Rect;
    fn eval_context(session: Arc<SessionContext>) -> EvaluationContext {
        EvaluationContext::new(
            Arc::new(Theme::light()),
            session,
            IndexMap::new(),
            Arc::new(EvaluatedFacetTree::empty()),
        )
    }

    fn xy_dataframe(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![0.0, 5.0, 10.0])),
                Arc::new(Float64Array::from(vec![10.0, 5.0, 0.0])),
            ],
        )
        .expect("test batch");
        ctx.read_batch(batch).expect("test dataframe")
    }

    async fn scoped_facet_dataframe(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
        ctx.sql(
            "SELECT * FROM (VALUES \
             ('North', 'West', 0.0, 0.2), \
             ('North', 'West', 2.0, 0.4), \
             ('North', 'East', 10.0, 0.6), \
             ('North', 'East', 12.0, 0.8), \
             ('South', 'West', 100.0, 0.2), \
             ('South', 'West', 102.0, 0.4), \
             ('South', 'East', 110.0, 0.6), \
             ('South', 'East', 112.0, 0.8) \
             ) AS t(facet_row, facet_col, x, y)",
        )
        .await
        .expect("scoped facet dataframe")
    }

    async fn scoped_facet_tree(
        df: datafusion::dataframe::DataFrame,
        ctx: &SessionContext,
    ) -> Result<EvaluatedFacetTree, AvengerChartError> {
        let leaf = Plot::<Cartesian>::new().mark(Symbol::new().x(col("x")).y(col("y")));
        let col_plot =
            Plot::<FacetColumn>::new().mark(Subplot::new(leaf).col_with(col("facet_col"), |c| c));
        let row_plot = Plot::<FacetRow>::new()
            .data(df)
            .mark(Subplot::new(col_plot).row_with(col("facet_row"), |c| c));
        let compiled = row_plot.compile(ctx).await?;
        EvaluatedFacetTree::from_compiled_plot(&compiled, ctx).await
    }

    fn plot_data_node(
        df: &datafusion::dataframe::DataFrame,
    ) -> Result<LogicalPlanNode, AvengerChartError> {
        LogicalPlanNode::from_logical_plan(df.logical_plan())
    }

    fn value_channel(expr: Expr) -> ChannelValue {
        ChannelValue::Value {
            expr: LogicalExprNode::from_expr(expr).expect("serialize test expression"),
        }
    }

    fn linear_scale() -> ConfiguredScaleWithSpec {
        let scale = Scale::<Linear>::new().into_auto();
        let configured = ConfiguredScale {
            scale_impl: Linear.create_impl(),
            config: ScaleConfig::empty(),
        }
        .with_domain_interval((0.0, 10.0))
        .with_range_interval((0.0, 100.0));
        ConfiguredScaleWithSpec::with_range_binding(
            scale,
            configured,
            ScaleRangeBinding::Independent,
        )
    }

    fn values_as_f64(batch: &RecordBatch, column_name: &str) -> Vec<f64> {
        let column = batch
            .column_by_name(column_name)
            .unwrap_or_else(|| panic!("missing column {column_name}"));
        let casted = cast(column, &DataType::Float64).expect("cast test values");
        let values = casted
            .as_any()
            .downcast_ref::<Float64Array>()
            .expect("float64 values");
        (0..values.len()).map(|idx| values.value(idx)).collect()
    }

    async fn prepared_channel_values(
        prepared: PreparedLogicalMarkData,
        ctx: &SessionContext,
        channel: &str,
    ) -> Result<Vec<f64>, AvengerChartError> {
        let dataframe = prepared
            .dataframe
            .expect("prepared aggregate should have dataframe");
        let expr = prepared
            .channels
            .get(channel)
            .and_then(|channel_value| channel_value.expr(ctx))
            .expect("prepared channel expression");
        let batches = dataframe
            .select(vec![expr.alias(channel)])?
            .collect()
            .await?;
        let schema = batches[0].schema();
        let batch = concat_batches(&schema, &batches)?;
        Ok(values_as_f64(&batch, channel))
    }

    async fn prepared_x_values_for_facet_mark(
        mark: Symbol<Cartesian>,
        session: Arc<SessionContext>,
        root_df: &datafusion::dataframe::DataFrame,
        facet_tree: &EvaluatedFacetTree,
        full_path: &[ScalarValue],
    ) -> Result<Vec<f64>, AvengerChartError> {
        let leaf_df = root_df.clone().filter(
            facet_tree
                .cell_predicate(full_path, 0)
                .expect("leaf facet predicate"),
        )?;
        let compiled_mark = mark.compile_untransformed(&session).await?;
        let eval_ctx = eval_context(session.clone()).with_facet_tree(Arc::new(facet_tree.clone()));
        let prepared = prepare_logical_mark_data(LogicalMarkDataRequest {
            mark: compiled_mark.as_ref(),
            plot_data: None,
            provided_plot_df: Some(&leaf_df),
            facet_data_scope: Some(FacetDataScopeContext::new(
                facet_tree,
                Some(root_df),
                full_path,
            )),
            eval_ctx: &eval_ctx,
        })
        .await?;
        prepared_channel_values(prepared, &session, "x").await
    }

    async fn collect_prepared_dataframe(
        prepared: PreparedLogicalMarkData,
    ) -> Result<RecordBatch, AvengerChartError> {
        let dataframe = prepared.dataframe.expect("prepared dataframe");
        let batches = dataframe.collect().await?;
        Ok(concat_batches(&batches[0].schema(), &batches)?)
    }

    fn brush_store_state(sharing: Sharing) -> Result<ScopedStoreState, AvengerChartError> {
        let spec = Store::empty("brush_boxes")
            .field("id", DataType::Utf8, false)
            .field("x_min", DataType::Float64, false)
            .field("x_max", DataType::Float64, false)
            .primary_key(["id"])
            .sharing(sharing)
            .compile()?;
        let mut specs = IndexMap::new();
        specs.insert(spec.name.clone(), spec);
        Ok(ScopedStoreState::new(specs))
    }

    fn brush_row(id: &str, x_min: f64, x_max: f64) -> StoreRowValue {
        IndexMap::from([
            ("id".to_string(), ScalarValue::Utf8(Some(id.to_string()))),
            ("x_min".to_string(), ScalarValue::Float64(Some(x_min))),
            ("x_max".to_string(), ScalarValue::Float64(Some(x_max))),
        ])
    }

    fn replace_store_rows(
        owner_path: Vec<ScalarValue>,
        rows: Vec<StoreRowValue>,
    ) -> ScopedStoreAssignment {
        ScopedStoreAssignment {
            store_name: "brush_boxes".to_string(),
            owner_path,
            replace_scoped_values: false,
            update: StoreStateUpdate::ReplaceRows { rows },
        }
    }

    #[tokio::test]
    async fn prepare_mark_data_applies_scaled_position_channels() -> Result<(), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let df = xy_dataframe(&session);
        let plot_node = plot_data_node(&df)?;
        let mark = Symbol::<Cartesian>::new().x(col("x")).y(col("y"));
        let compiled_mark = mark.compile_untransformed(&session).await?;
        let eval_ctx = eval_context(session);
        let scales = HashMap::from([
            ("x".to_string(), linear_scale()),
            ("y".to_string(), linear_scale()),
        ]);

        let prepared = prepare_mark_data(MarkDataRequest {
            mark: compiled_mark.as_ref(),
            plot_data: Some(&plot_node),
            provided_plot_df: None,
            facet_data_scope: None,
            prepared_logical: None,
            eval_ctx: &eval_ctx,
            evaluation_metrics: None,
            scales: &scales,
            plot_width: 100.0,
            plot_height: 100.0,
        })
        .await?
        .expect("prepared data");

        let data_batch = prepared.data_batch.expect("array data");
        assert_eq!(values_as_f64(&data_batch, "x"), vec![0.0, 50.0, 100.0]);
        assert_eq!(values_as_f64(&data_batch, "y"), vec![100.0, 50.0, 0.0]);
        Ok(())
    }

    #[tokio::test]
    async fn prepare_mark_data_uses_inherited_plot_data() -> Result<(), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let df = xy_dataframe(&session);
        let plot_node = plot_data_node(&df)?;
        let mark = Symbol::<Cartesian>::new()
            .with_channel_value("x", value_channel(col("x")))
            .y(5.0);
        let compiled_mark = mark.compile_untransformed(&session).await?;
        let eval_ctx = eval_context(session);
        let scales = HashMap::new();

        let prepared = prepare_mark_data(MarkDataRequest {
            mark: compiled_mark.as_ref(),
            plot_data: Some(&plot_node),
            provided_plot_df: None,
            facet_data_scope: None,
            prepared_logical: None,
            eval_ctx: &eval_ctx,
            evaluation_metrics: None,
            scales: &scales,
            plot_width: 100.0,
            plot_height: 100.0,
        })
        .await?
        .expect("prepared data");

        let data_batch = prepared.data_batch.expect("inherited array data");
        assert_eq!(values_as_f64(&data_batch, "x"), vec![0.0, 5.0, 10.0]);
        assert!(prepared.scalar_batch.column_by_name("y").is_some());
        Ok(())
    }

    #[tokio::test]
    async fn prepare_mark_data_handles_scalar_only_marks() -> Result<(), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let mark = Symbol::<Cartesian>::new().x(2.0).y(3.0);
        let compiled_mark = mark.compile_untransformed(&session).await?;
        let eval_ctx = eval_context(session);
        let scales = HashMap::new();

        let prepared = prepare_mark_data(MarkDataRequest {
            mark: compiled_mark.as_ref(),
            plot_data: None,
            provided_plot_df: None,
            facet_data_scope: None,
            prepared_logical: None,
            eval_ctx: &eval_ctx,
            evaluation_metrics: None,
            scales: &scales,
            plot_width: 100.0,
            plot_height: 100.0,
        })
        .await?
        .expect("prepared data");

        assert!(prepared.data_batch.is_none());
        assert_eq!(prepared.scalar_batch.num_rows(), 1);
        assert_eq!(values_as_f64(&prepared.scalar_batch, "x"), vec![2.0]);
        assert_eq!(values_as_f64(&prepared.scalar_batch, "y"), vec![3.0]);
        Ok(())
    }

    #[tokio::test]
    async fn prepare_mark_data_preserves_full_data_for_container_marks()
    -> Result<(), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let df = xy_dataframe(&session);
        let mark = Subplot::<HConcat>::new(Plot::<ZeroDCoord>::new());
        let compiled_mark = mark.compile_untransformed(&session).await?;
        let eval_ctx = eval_context(session);
        let scales = HashMap::new();

        let prepared = prepare_mark_data(MarkDataRequest {
            mark: compiled_mark.as_ref(),
            plot_data: None,
            provided_plot_df: Some(&df),
            facet_data_scope: None,
            prepared_logical: None,
            eval_ctx: &eval_ctx,
            evaluation_metrics: None,
            scales: &scales,
            plot_width: 100.0,
            plot_height: 100.0,
        })
        .await?
        .expect("prepared data");

        let data_batch = prepared.data_batch.expect("full data batch");
        assert_eq!(data_batch.num_rows(), 3);
        assert!(data_batch.column_by_name("x").is_some());
        assert!(data_batch.column_by_name("y").is_some());
        assert_eq!(prepared.scalar_batch.num_rows(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn prepare_logical_mark_data_reads_store_data_rows() -> Result<(), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let mut store_state = brush_store_state(Sharing::Shared)?;
        store_state.apply_scoped_patch([replace_store_rows(
            Vec::new(),
            vec![brush_row("a", 1.0, 2.0), brush_row("b", 3.0, 4.0)],
        )])?;
        let mark = Symbol::<Cartesian>::new()
            .data_store(StoreData::new("brush_boxes"))
            .with_channel_value("x", value_channel(col("x_min")))
            .y(0.5);
        let compiled_mark = mark.compile_untransformed(&session).await?;

        let eval_ctx =
            eval_context(session.clone()).with_scoped_store_state(Arc::new(store_state.clone()));
        let prepared = prepare_logical_mark_data(LogicalMarkDataRequest {
            mark: compiled_mark.as_ref(),
            plot_data: None,
            provided_plot_df: None,
            facet_data_scope: None,
            eval_ctx: &eval_ctx,
        })
        .await?;
        let values = prepared_channel_values(prepared.clone(), &session, "x").await?;
        assert_eq!(values, vec![1.0, 3.0]);

        let batch = collect_prepared_dataframe(prepared).await?;
        assert_eq!(batch.num_rows(), 2);
        assert!(batch.column_by_name(STORE_NAME_COLUMN).is_some());
        assert!(batch.column_by_name(STORE_OWNER_KEY_COLUMN).is_some());
        assert!(batch.column_by_name(STORE_REVISION_COLUMN).is_some());

        store_state.apply_scoped_patch([replace_store_rows(
            Vec::new(),
            vec![brush_row("c", 5.0, 6.0)],
        )])?;
        let eval_ctx = eval_context(session.clone()).with_scoped_store_state(Arc::new(store_state));
        let prepared = prepare_logical_mark_data(LogicalMarkDataRequest {
            mark: compiled_mark.as_ref(),
            plot_data: None,
            provided_plot_df: None,
            facet_data_scope: None,
            eval_ctx: &eval_ctx,
        })
        .await?;
        let values = prepared_channel_values(prepared, &session, "x").await?;
        assert_eq!(values, vec![5.0]);
        Ok(())
    }

    #[tokio::test]
    async fn prepare_mark_data_renders_rect_rows_from_store_data() -> Result<(), AvengerChartError>
    {
        let session = Arc::new(SessionContext::new());
        let mut store_state = brush_store_state(Sharing::Shared)?;
        store_state.apply_scoped_patch([replace_store_rows(
            Vec::new(),
            vec![brush_row("a", 1.0, 2.0), brush_row("b", 3.0, 4.0)],
        )])?;
        let eval_ctx = eval_context(session.clone()).with_scoped_store_state(Arc::new(store_state));
        let mark = Rect::<Cartesian>::new()
            .data_store(StoreData::new("brush_boxes"))
            .x_with(col("x_min"), |c| c.no_scale())
            .x2_with(col("x_max"), |c| c.no_scale())
            .y_with(lit(0.0), |c| c.no_scale())
            .y2_with(lit(1.0), |c| c.no_scale());
        let compiled_mark = mark.compile_untransformed(&session).await?;

        let prepared = prepare_mark_data(MarkDataRequest {
            mark: compiled_mark.as_ref(),
            plot_data: None,
            provided_plot_df: None,
            facet_data_scope: None,
            prepared_logical: None,
            eval_ctx: &eval_ctx,
            evaluation_metrics: None,
            scales: &HashMap::new(),
            plot_width: 100.0,
            plot_height: 100.0,
        })
        .await?
        .expect("prepared rect data");

        let data_batch = prepared.data_batch.expect("rect array data");
        assert_eq!(data_batch.num_rows(), 2);
        assert_eq!(values_as_f64(&data_batch, "x"), vec![1.0, 3.0]);
        assert_eq!(values_as_f64(&data_batch, "x2"), vec![2.0, 4.0]);
        Ok(())
    }

    #[tokio::test]
    async fn store_data_reads_owner_implied_by_store_sharing() -> Result<(), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let root_df = scoped_facet_dataframe(&session).await;
        let facet_tree = scoped_facet_tree(root_df.clone(), &session).await?;
        let north_west = vec![
            ScalarValue::Utf8(Some("North".to_string())),
            ScalarValue::Utf8(Some("West".to_string())),
        ];
        let north_east = vec![
            ScalarValue::Utf8(Some("North".to_string())),
            ScalarValue::Utf8(Some("East".to_string())),
        ];
        let north_west_owner = facet_tree.sharing_owner_path(&north_west, 0);
        let north_east_owner = facet_tree.sharing_owner_path(&north_east, 0);

        let mut free_store_state = brush_store_state(Sharing::Free)?;
        free_store_state.apply_scoped_patch([
            replace_store_rows(Vec::new(), vec![brush_row("root", 100.0, 101.0)]),
            replace_store_rows(north_west_owner.clone(), vec![brush_row("nw", 1.0, 2.0)]),
            replace_store_rows(north_east_owner.clone(), vec![brush_row("ne", 10.0, 11.0)]),
        ])?;
        let free_eval_ctx = eval_context(session.clone())
            .with_facet_tree(Arc::new(facet_tree.clone()))
            .with_scoped_store_state(Arc::new(free_store_state));

        let mut shared_store_state = brush_store_state(Sharing::Shared)?;
        shared_store_state.apply_scoped_patch([
            replace_store_rows(Vec::new(), vec![brush_row("root", 100.0, 101.0)]),
            replace_store_rows(north_west_owner, vec![brush_row("nw", 1.0, 2.0)]),
            replace_store_rows(north_east_owner, vec![brush_row("ne", 10.0, 11.0)]),
        ])?;
        let shared_eval_ctx = eval_context(session.clone())
            .with_facet_tree(Arc::new(facet_tree.clone()))
            .with_scoped_store_state(Arc::new(shared_store_state));

        let values_for_store = |eval_ctx: EvaluationContext| {
            let session = session.clone();
            let facet_tree = facet_tree.clone();
            let root_df = root_df.clone();
            let north_west = north_west.clone();
            async move {
                let mark = Symbol::<Cartesian>::new()
                    .data_store(StoreData::new("brush_boxes"))
                    .with_channel_value("x", value_channel(col("x_min")))
                    .y(0.5);
                let compiled_mark = mark.compile_untransformed(&session).await?;
                let prepared = prepare_logical_mark_data(LogicalMarkDataRequest {
                    mark: compiled_mark.as_ref(),
                    plot_data: None,
                    provided_plot_df: None,
                    facet_data_scope: Some(FacetDataScopeContext::new(
                        &facet_tree,
                        Some(&root_df),
                        &north_west,
                    )),
                    eval_ctx: &eval_ctx,
                })
                .await?;
                prepared_channel_values(prepared, &session, "x").await
            }
        };

        assert_eq!(values_for_store(free_eval_ctx).await?, vec![1.0]);
        assert_eq!(values_for_store(shared_eval_ctx).await?, vec![100.0]);
        Ok(())
    }

    #[tokio::test]
    async fn prepare_logical_mark_data_aggregates_after_facet_data_scope()
    -> Result<(), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let df = scoped_facet_dataframe(&session).await;
        let facet_tree = scoped_facet_tree(df.clone(), &session).await?;
        let full_path = vec![
            ScalarValue::Utf8(Some("North".to_string())),
            ScalarValue::Utf8(Some("West".to_string())),
        ];
        let leaf_df = df
            .clone()
            .filter(
                facet_tree
                    .cell_predicate(&full_path, 0)
                    .expect("leaf facet predicate"),
            )
            .expect("leaf filtered data");
        let eval_ctx = eval_context(session.clone());

        let aggregate_value_for_scope = |mark: Symbol<Cartesian>| {
            let session = session.clone();
            let eval_ctx = eval_ctx.clone();
            let df = df.clone();
            let leaf_df = leaf_df.clone();
            let facet_tree = facet_tree.clone();
            let full_path = full_path.clone();
            async move {
                let compiled_mark = mark.compile_untransformed(&session).await?;
                let prepared = prepare_logical_mark_data(LogicalMarkDataRequest {
                    mark: compiled_mark.as_ref(),
                    plot_data: None,
                    provided_plot_df: Some(&leaf_df),
                    facet_data_scope: Some(FacetDataScopeContext::new(
                        &facet_tree,
                        Some(&df),
                        &full_path,
                    )),
                    eval_ctx: &eval_ctx,
                })
                .await?;
                let values = prepared_channel_values(prepared, &session, "x").await?;
                Ok::<f64, AvengerChartError>(values[0])
            }
        };

        let filtered = aggregate_value_for_scope(
            Symbol::<Cartesian>::new()
                .x(avg(col("x")))
                .y(lit(0.5))
                .size(120.0),
        )
        .await?;
        let row_level = aggregate_value_for_scope(
            Symbol::<Cartesian>::new()
                .x(avg(col("x")))
                .y(lit(0.5))
                .size(120.0)
                .facet_data_level(1),
        )
        .await?;
        let global = aggregate_value_for_scope(
            Symbol::<Cartesian>::new()
                .x(avg(col("x")))
                .y(lit(0.5))
                .size(120.0)
                .broadcast_to_facets(),
        )
        .await?;

        assert_eq!(filtered, 1.0);
        assert_eq!(row_level, 6.0);
        assert_eq!(global, 56.0);
        Ok(())
    }

    #[tokio::test]
    async fn transform_scope_controls_bin_owner_data() -> Result<(), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let df = scoped_facet_dataframe(&session).await;
        let facet_tree = scoped_facet_tree(df.clone(), &session).await?;
        let north_west = vec![
            ScalarValue::Utf8(Some("North".to_string())),
            ScalarValue::Utf8(Some("West".to_string())),
        ];
        let north_east = vec![
            ScalarValue::Utf8(Some("North".to_string())),
            ScalarValue::Utf8(Some("East".to_string())),
        ];

        let free_values = prepared_x_values_for_facet_mark(
            Symbol::<Cartesian>::new()
                .transform_free(Bin::new(col("x")).maxbins(2), |mark, bin| {
                    mark.x(bin.start()).y(col("y"))
                }),
            session.clone(),
            &df,
            &facet_tree,
            &north_west,
        )
        .await?;
        let row_level_values = prepared_x_values_for_facet_mark(
            Symbol::<Cartesian>::new().transform_level(
                1,
                Bin::new(col("x")).maxbins(2),
                |mark, bin| mark.x(bin.start()).y(col("y")),
            ),
            session.clone(),
            &df,
            &facet_tree,
            &north_east,
        )
        .await?;
        let shared_values = prepared_x_values_for_facet_mark(
            Symbol::<Cartesian>::new()
                .transform_shared(Bin::new(col("x")).maxbins(2), |mark, bin| {
                    mark.x(bin.start()).y(col("y"))
                }),
            session.clone(),
            &df,
            &facet_tree,
            &north_west,
        )
        .await?;

        assert_eq!(free_values, vec![0.0, 1.0]);
        assert_eq!(row_level_values, vec![10.0, 10.0]);
        assert_eq!(shared_values, vec![0.0, 0.0]);
        Ok(())
    }

    #[tokio::test]
    async fn no_output_transform_closure_configures_mark() -> Result<(), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let df = xy_dataframe(&session);
        let eval_ctx = eval_context(session.clone());
        let mark = Symbol::<Cartesian>::new().transform_no_output(
            Calculate::new().expr("x_shifted", col("x") + lit(1.0)),
            |mark| mark.x(col("x_shifted")).y(col("y")),
        );
        let compiled_mark = mark.compile_untransformed(&session).await?;
        let prepared = prepare_logical_mark_data(LogicalMarkDataRequest {
            mark: compiled_mark.as_ref(),
            plot_data: None,
            provided_plot_df: Some(&df),
            facet_data_scope: None,
            eval_ctx: &eval_ctx,
        })
        .await?;

        let values = prepared_channel_values(prepared, &session, "x").await?;
        assert_eq!(values, vec![1.0, 6.0, 11.0]);
        Ok(())
    }

    #[tokio::test]
    async fn shared_calculate_preserves_facet_columns_for_narrowing()
    -> Result<(), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let df = scoped_facet_dataframe(&session).await;
        let facet_tree = scoped_facet_tree(df.clone(), &session).await?;
        let full_path = vec![
            ScalarValue::Utf8(Some("North".to_string())),
            ScalarValue::Utf8(Some("West".to_string())),
        ];

        let mark = Symbol::<Cartesian>::new().transform_shared_no_output(
            Calculate::new().expr("x_plus_y", col("x") + col("y")),
            |mark| mark.x(col("x_plus_y")).y(col("y")),
        );
        let values =
            prepared_x_values_for_facet_mark(mark, session, &df, &facet_tree, &full_path).await?;

        assert_eq!(values, vec![0.2, 2.4]);
        Ok(())
    }

    #[tokio::test]
    async fn filter_transform_preserves_columns() -> Result<(), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let df = xy_dataframe(&session);
        let eval_ctx = eval_context(session.clone());
        let mark = Symbol::<Cartesian>::new()
            .transform_no_output(Filter::new(col("x").gt(lit(0.0))), |mark| {
                mark.x(col("x")).y(col("y"))
            });
        let compiled_mark = mark.compile_untransformed(&session).await?;
        let prepared = prepare_logical_mark_data(LogicalMarkDataRequest {
            mark: compiled_mark.as_ref(),
            plot_data: None,
            provided_plot_df: Some(&df),
            facet_data_scope: None,
            eval_ctx: &eval_ctx,
        })
        .await?;

        let values = prepared_channel_values(prepared, &session, "x").await?;
        assert_eq!(values, vec![5.0, 10.0]);
        Ok(())
    }

    #[tokio::test]
    async fn shared_filter_preserves_facet_columns_for_narrowing() -> Result<(), AvengerChartError>
    {
        let session = Arc::new(SessionContext::new());
        let df = scoped_facet_dataframe(&session).await;
        let facet_tree = scoped_facet_tree(df.clone(), &session).await?;
        let full_path = vec![
            ScalarValue::Utf8(Some("North".to_string())),
            ScalarValue::Utf8(Some("East".to_string())),
        ];

        let mark = Symbol::<Cartesian>::new()
            .transform_shared_no_output(Filter::new(col("x").lt(lit(20.0))), |mark| {
                mark.x(col("x")).y(col("y"))
            });
        let values =
            prepared_x_values_for_facet_mark(mark, session, &df, &facet_tree, &full_path).await?;

        assert_eq!(values, vec![10.0, 12.0]);
        Ok(())
    }

    #[tokio::test]
    async fn shared_select_must_keep_facet_columns_for_narrowing() -> Result<(), AvengerChartError>
    {
        let session = Arc::new(SessionContext::new());
        let df = scoped_facet_dataframe(&session).await;
        let facet_tree = scoped_facet_tree(df.clone(), &session).await?;
        let full_path = vec![
            ScalarValue::Utf8(Some("North".to_string())),
            ScalarValue::Utf8(Some("West".to_string())),
        ];

        let dropped_columns_mark = Symbol::<Cartesian>::new()
            .transform_shared_no_output(Select::new().expr(col("x")).expr(col("y")), |mark| {
                mark.x(col("x")).y(col("y"))
            });
        let err = match prepared_x_values_for_facet_mark(
            dropped_columns_mark,
            session.clone(),
            &df,
            &facet_tree,
            &full_path,
        )
        .await
        {
            Ok(_) => panic!("shared select without facet columns should error"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("no longer contains the facet columns"),
            "{err}"
        );

        let preserved_columns_mark = Symbol::<Cartesian>::new().transform_shared_no_output(
            Select::new()
                .expr(col("facet_row"))
                .expr(col("facet_col"))
                .expr(col("x"))
                .expr(col("y")),
            |mark| mark.x(col("x")).y(col("y")),
        );
        let values = prepared_x_values_for_facet_mark(
            preserved_columns_mark,
            session,
            &df,
            &facet_tree,
            &full_path,
        )
        .await?;
        assert_eq!(values, vec![0.0, 2.0]);
        Ok(())
    }

    #[tokio::test]
    async fn shared_fold_preserves_facet_columns_for_narrowing() -> Result<(), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let df = scoped_facet_dataframe(&session).await;
        let facet_tree = scoped_facet_tree(df.clone(), &session).await?;
        let full_path = vec![
            ScalarValue::Utf8(Some("North".to_string())),
            ScalarValue::Utf8(Some("West".to_string())),
        ];

        let mark = Symbol::<Cartesian>::new().transform_shared(
            Fold::new().field("x", col("x")).field("y", col("y")),
            |mark, fold| mark.x(fold.value()).y(lit(0.5)),
        );
        let mut values =
            prepared_x_values_for_facet_mark(mark, session, &df, &facet_tree, &full_path).await?;
        values.sort_by(f64::total_cmp);

        assert_eq!(values, vec![0.0, 0.2, 0.4, 2.0]);
        Ok(())
    }

    #[tokio::test]
    async fn shared_window_partitioned_by_facet_columns_narrows_predictably()
    -> Result<(), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let df = scoped_facet_dataframe(&session).await;
        let facet_tree = scoped_facet_tree(df.clone(), &session).await?;
        let full_path = vec![
            ScalarValue::Utf8(Some("South".to_string())),
            ScalarValue::Utf8(Some("East".to_string())),
        ];

        let mark = Symbol::<Cartesian>::new().transform_shared_no_output(
            Window::new()
                .partition_by([col("facet_row"), col("facet_col")])
                .order_by([col("x").sort(true, false)])
                .expr("cell_order", row_number()),
            |mark| mark.x(col("cell_order")).y(col("y")),
        );
        let values =
            prepared_x_values_for_facet_mark(mark, session, &df, &facet_tree, &full_path).await?;

        assert_eq!(values, vec![1.0, 2.0]);
        Ok(())
    }

    #[tokio::test]
    async fn transform_stage_scopes_must_not_broaden() -> Result<(), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let df = xy_dataframe(&session);
        let eval_ctx = eval_context(session.clone());
        let mark = Symbol::<Cartesian>::new()
            .transform_free(
                Bin::new(col("x")).maxbins(2).name("local_bin"),
                |mark, bin| mark.x(bin.start()),
            )
            .transform_shared(
                Bin::new(col("x")).maxbins(2).name("shared_bin"),
                |mark, _| mark.y(col("y")),
            );
        let compiled_mark = mark.compile_untransformed(&session).await?;
        let err = match prepare_logical_mark_data(LogicalMarkDataRequest {
            mark: compiled_mark.as_ref(),
            plot_data: None,
            provided_plot_df: Some(&df),
            facet_data_scope: None,
            eval_ctx: &eval_ctx,
        })
        .await
        {
            Ok(_) => panic!("narrow-to-broad transform scopes should error"),
            Err(err) => err,
        };

        assert!(
            err.to_string().contains("broader than the preceding"),
            "{err}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn shared_transform_must_keep_facet_columns_for_narrowing()
    -> Result<(), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let df = scoped_facet_dataframe(&session).await;
        let facet_tree = scoped_facet_tree(df.clone(), &session).await?;
        let full_path = vec![
            ScalarValue::Utf8(Some("North".to_string())),
            ScalarValue::Utf8(Some("West".to_string())),
        ];

        let dropped_columns_mark = Symbol::<Cartesian>::new()
            .transform_shared(Aggregate::new().mean("mean_x", col("x")), |mark, agg| {
                mark.x(agg.output("mean_x")).y(lit(0.5))
            });
        let err = match prepared_x_values_for_facet_mark(
            dropped_columns_mark,
            session.clone(),
            &df,
            &facet_tree,
            &full_path,
        )
        .await
        {
            Ok(_) => panic!("shared aggregate without facet columns should error"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("no longer contains the facet columns"),
            "{err}"
        );

        let grouped_mark = Symbol::<Cartesian>::new().transform_shared(
            Aggregate::new()
                .group_by([col("facet_row"), col("facet_col")])
                .mean("mean_x", col("x")),
            |mark, agg| mark.x(agg.output("mean_x")).y(lit(0.5)),
        );
        let values =
            prepared_x_values_for_facet_mark(grouped_mark, session, &df, &facet_tree, &full_path)
                .await?;
        assert_eq!(values, vec![1.0]);
        Ok(())
    }

    #[tokio::test]
    async fn shared_joinaggregate_without_grouping_remains_facet_addressable()
    -> Result<(), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let df = scoped_facet_dataframe(&session).await;
        let facet_tree = scoped_facet_tree(df.clone(), &session).await?;
        let full_path = vec![
            ScalarValue::Utf8(Some("North".to_string())),
            ScalarValue::Utf8(Some("West".to_string())),
        ];

        let mark = Symbol::<Cartesian>::new().transform_shared_no_output(
            JoinAggregate::new().sum("global_x_total", col("x")),
            |mark| mark.x(col("global_x_total")).y(col("y")),
        );
        let values =
            prepared_x_values_for_facet_mark(mark, session, &df, &facet_tree, &full_path).await?;

        assert_eq!(values, vec![448.0, 448.0]);
        Ok(())
    }

    #[tokio::test]
    async fn prepare_logical_mark_data_keeps_explicit_mark_data_unscoped()
    -> Result<(), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let root_df = scoped_facet_dataframe(&session).await;
        let explicit_df = session
            .sql("SELECT * FROM (VALUES (999.0), (1001.0)) AS t(x)")
            .await
            .expect("explicit mark dataframe");
        let facet_tree = scoped_facet_tree(root_df.clone(), &session).await?;
        let full_path = vec![
            ScalarValue::Utf8(Some("North".to_string())),
            ScalarValue::Utf8(Some("West".to_string())),
        ];
        let leaf_df = root_df
            .clone()
            .filter(
                facet_tree
                    .cell_predicate(&full_path, 0)
                    .expect("leaf facet predicate"),
            )
            .expect("leaf filtered data");
        let mark = Symbol::<Cartesian>::new()
            .data(explicit_df)
            .x(avg(col("x")))
            .y(lit(0.5))
            .broadcast_to_facets();
        let compiled_mark = mark.compile_untransformed(&session).await?;
        let eval_ctx = eval_context(session.clone());
        let prepared = prepare_logical_mark_data(LogicalMarkDataRequest {
            mark: compiled_mark.as_ref(),
            plot_data: None,
            provided_plot_df: Some(&leaf_df),
            facet_data_scope: Some(FacetDataScopeContext::new(
                &facet_tree,
                Some(&root_df),
                &full_path,
            )),
            eval_ctx: &eval_ctx,
        })
        .await?;
        let values = prepared_channel_values(prepared, &session, "x").await?;

        assert_eq!(values, vec![1000.0]);
        Ok(())
    }
}
