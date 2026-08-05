//! Shared mark-channel preparation for measurement and rendering.
//!
//! Rendering and coordinate-system measurement sometimes need the same prepared
//! channel batches. Keeping this logic here avoids duplicating scale expression
//! handling between mark rendering and child-frame container measurement.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use avenger_color::parse_color_string;
use datafusion::{
    arrow::{
        array::Int32Array,
        compute::concat_batches,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    common::{Column, DFSchema, ScalarValue},
    dataframe::DataFrame,
    logical_expr::{EmptyRelation, Expr, LogicalPlan, Operator, cast, col, lit, when},
    prelude::SessionContext,
};
use datafusion_common::tree_node::{Transformed, TreeNode};
use datafusion_proto::protobuf::{LogicalExprNode, LogicalPlanNode};
use indexmap::IndexMap;

use avenger_chart_core::{
    CompiledDataContext, CompiledSelectionSpec, CompiledStateRegistry, CompiledViewScope,
    CompiledViewSpec, DataTransformExecutionContext, DataTransformFacetContext, DataTransformStage,
    DerivedScalarMap, FacetDataScope, MarkDataMode, MaterializationPolicy, MaterializationResult,
    SelectionClause, SelectionCombine, SelectionPredicateSpec, SelectionRef, SharingLevel,
    ViewMaterializationContext, ViewMaterializationRequest, ViewStalePolicy,
    collect_derived_scalar_ids, contains_aggregate, detail_array_column_name,
    item_frame_column_refs, params_to_datafusion, resolve_known_derived_scalars,
    resolve_known_derived_scalars_in_channel_value, selection_clause_value_id_from_placeholder,
    selection_field_expr_fingerprint, selection_id_from_equality_membership_field_placeholder,
    selection_id_from_equality_membership_value_placeholder,
    selection_id_from_predicate_placeholder,
};
#[cfg(test)]
use avenger_common::time::Duration;
use avenger_common::time::Instant;

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

use super::materialization::{MaterializationStatus, PREVIEW_CONSUME_STABILITY};

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

fn exact_schema_col(name: impl Into<String>) -> Expr {
    // Arrow schemas are case-sensitive. DataFusion's SQL-oriented `col()`
    // helper normalizes unquoted identifiers, so it is not safe when rebuilding
    // an expression from an already-decoded schema field name.
    Expr::Column(Column::new_unqualified(name.into()))
}

/// Data prepared by a compiled `MarkGroup` before child mark-local transforms
/// and channel aggregate preparation run.
#[derive(Clone)]
pub(crate) struct PreparedBaseData {
    pub(crate) dataframe: Option<DataFrame>,
    pub(crate) derived_scalars: DerivedScalarMap,
    pub(crate) facet_data_scope: FacetDataScope,
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
    pub(crate) coord_transform: Option<&'a dyn avenger_chart_core::CoordinateSystemTransformCore>,
    pub(crate) plot_data: Option<&'a LogicalPlanNode>,
    pub(crate) provided_plot_df: Option<&'a DataFrame>,
    pub(crate) facet_data_scope: Option<FacetDataScopeContext<'a>>,
    pub(crate) prepared_logical: Option<&'a PreparedLogicalMarkData>,
    pub(crate) prepared_base: Option<&'a PreparedBaseData>,
    pub(crate) eval_ctx: &'a EvaluationContext,
    pub(crate) evaluation_metrics: Option<Arc<Mutex<EvaluationMetrics>>>,
    pub(crate) scales: &'a HashMap<String, ConfiguredScaleWithSpec>,
    pub(crate) plot_width: f32,
    pub(crate) plot_height: f32,
    /// The nearest enclosing group view scope, when this mark sits inside a
    /// viewed `MarkGroup`. Carries the group's shared view-local chain,
    /// which runs once per (plot, group, facet path) per evaluation; its
    /// output dataframe and derived scalars feed every child's view chain.
    pub(crate) group_view: Option<GroupViewMarkContext<'a>>,
}

/// Reference to the enclosing group view scope for a mark.
#[derive(Clone, Copy)]
pub(crate) struct GroupViewMarkContext<'a> {
    /// Identity of the owning `CompiledPlot`, disambiguating group indices
    /// across nested child plots sharing one evaluation context.
    pub(crate) plot_identity: usize,
    pub(crate) group_index: usize,
    pub(crate) scope: &'a CompiledViewScope,
}

/// Once-per-evaluation output of a group's shared view-local chain.
pub(crate) struct GroupViewPrepared {
    pub(crate) dataframe: Option<DataFrame>,
    /// Scalars produced by the shared chain (chain-produced only; the group
    /// base scalars seed the chain and are merged separately).
    pub(crate) derived_scalars: DerivedScalarMap,
    /// Resolved view param values the shared chain was computed with, used
    /// to assert that reusing children agree (they share the cell's scales,
    /// so a mismatch means a bug).
    view_params: Vec<(String, ScalarValue)>,
    /// Scale inference deliberately observes materialization state without
    /// scheduling missing work. Do not reuse that empty/read-only result for
    /// a render pass that must enqueue the materialization.
    materialization_handling: ViewMaterializationHandling,
}

pub(crate) type GroupViewDataCacheHandle =
    Arc<Mutex<HashMap<super::MarkGroupDataCacheKey, Arc<GroupViewPrepared>>>>;

fn view_param_snapshot(
    view_scope: &CompiledViewScope,
    eval_ctx: &EvaluationContext,
) -> Vec<(String, ScalarValue)> {
    let prefix = format!("__avenger_view_{}_", view_scope.spec.source_name());
    eval_ctx
        .params()
        .iter()
        .filter(|(name, _)| name.starts_with(&prefix))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

/// Run (or reuse) the group's shared view-local chain for the current
/// evaluation. Memo-on-first-use: the first child whose view preparation
/// needs the group result computes it with its own view-param context;
/// later children reuse it.
async fn prepare_group_view_data(
    group_view: &GroupViewMarkContext<'_>,
    base_prepared: &PreparedLogicalMarkData,
    request: &MarkDataRequest<'_>,
    view_eval_ctx: &EvaluationContext,
    materialization_handling: ViewMaterializationHandling,
) -> Result<Arc<GroupViewPrepared>, AvengerChartError> {
    let ctx = request.eval_ctx.session_context.as_ref();
    let facet_path: Vec<String> = request
        .facet_data_scope
        .as_ref()
        .map(|scope| {
            scope
                .full_path
                .iter()
                .map(|value| format!("{value:?}"))
                .collect()
        })
        .unwrap_or_default();
    let key = super::MarkGroupDataCacheKey {
        plot_identity: group_view.plot_identity,
        group_index: group_view.group_index,
        facet_path,
    };

    let current_view_params = view_param_snapshot(group_view.scope, view_eval_ctx);
    if let Some(cached) = view_eval_ctx
        .group_view_data_cache
        .lock()
        .expect("group view data cache lock poisoned")
        .get(&key)
        .cloned()
    {
        // Children within one pass share the cell's scales and resolve
        // identical view params, so a hit with matching params is a safe
        // reuse. A mismatch in params or materialization handling means a
        // different pass of the same evaluation (for example read-only scale
        // inference vs scheduling render) — recompute so each pass sees its
        // own view state and side-effect policy.
        if cached.view_params == current_view_params
            && cached.materialization_handling == materialization_handling
        {
            return Ok(cached);
        }
    }

    let (dataframe, derived_scalars) = apply_view_mark_data_transforms(
        base_prepared.dataframe.clone(),
        group_view.scope.data.transforms(),
        ctx,
        view_eval_ctx,
        request.facet_data_scope,
        request.mark.state().facet_data_scope,
        group_view.scope,
        materialization_handling,
        &base_prepared.derived_scalars,
    )
    .await?;

    let prepared = Arc::new(GroupViewPrepared {
        dataframe,
        derived_scalars,
        view_params: current_view_params,
        materialization_handling,
    });
    view_eval_ctx
        .group_view_data_cache
        .lock()
        .expect("group view data cache lock poisoned")
        .insert(key, prepared.clone());
    Ok(prepared)
}

pub(crate) struct LogicalMarkDataRequest<'a> {
    pub(crate) mark: &'a dyn CompiledMark,
    pub(crate) plot_data: Option<&'a LogicalPlanNode>,
    pub(crate) provided_plot_df: Option<&'a DataFrame>,
    pub(crate) facet_data_scope: Option<FacetDataScopeContext<'a>>,
    pub(crate) prepared_base: Option<&'a PreparedBaseData>,
    pub(crate) eval_ctx: &'a EvaluationContext,
}

pub(crate) struct BaseDataRequest<'a> {
    pub(crate) data_context: &'a CompiledDataContext,
    pub(crate) data_mode: MarkDataMode,
    pub(crate) facet_data_scope: FacetDataScope,
    pub(crate) plot_data: Option<&'a LogicalPlanNode>,
    pub(crate) provided_plot_df: Option<&'a DataFrame>,
    pub(crate) inherited_base: Option<&'a PreparedBaseData>,
    pub(crate) facet_data_scope_context: Option<FacetDataScopeContext<'a>>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ViewMaterializationHandling {
    RenderAndSchedule,
    ScaleInferenceReadOnly,
    PreviewRetargetScheduleOnly,
}

impl ViewMaterializationHandling {
    fn emits_request(self) -> bool {
        matches!(
            self,
            Self::RenderAndSchedule | Self::PreviewRetargetScheduleOnly
        )
    }

    fn enqueues_cache_miss(self) -> bool {
        matches!(
            self,
            Self::RenderAndSchedule | Self::PreviewRetargetScheduleOnly
        )
    }

    fn uses_stale_fallback(self) -> bool {
        matches!(
            self,
            Self::RenderAndSchedule
                | Self::ScaleInferenceReadOnly
                | Self::PreviewRetargetScheduleOnly
        )
    }
}

enum TransformChainMode<'a> {
    Ordinary {
        eval_ctx: &'a EvaluationContext,
    },
    View {
        eval_ctx: &'a EvaluationContext,
        view_scope: &'a CompiledViewScope,
        materialization_handling: ViewMaterializationHandling,
    },
}

impl<'a> TransformChainMode<'a> {
    fn eval_ctx(&self) -> &'a EvaluationContext {
        match self {
            Self::Ordinary { eval_ctx } | Self::View { eval_ctx, .. } => eval_ctx,
        }
    }

    fn initial_scope_label(&self) -> &'static str {
        match self {
            Self::Ordinary { .. } => "initial transform scope",
            Self::View { .. } => "initial view transform scope",
        }
    }

    fn narrower_scope_label(&self) -> &'static str {
        match self {
            Self::Ordinary { .. } => "narrower transform scope",
            Self::View { .. } => "narrower view transform scope",
        }
    }

    fn final_scope_label(&self) -> &'static str {
        match self {
            Self::Ordinary { .. } => "final mark facet data scope",
            Self::View { .. } => "final view mark facet data scope",
        }
    }
}

struct TransformChainOutcome {
    dataframe: Option<DataFrame>,
    derived_scalars: DerivedScalarMap,
    materialization_request_count: usize,
    /// Whether any view materialization in the chain displayed its DESIRED
    /// (current-key) result, as opposed to a stale fallback or empty payload.
    desired_materialization_ready: bool,
}

async fn apply_mark_data_transforms(
    dataframe: Option<DataFrame>,
    transforms: &[DataTransformStage],
    ctx: &SessionContext,
    eval_ctx: &EvaluationContext,
    facet_data_scope: Option<FacetDataScopeContext<'_>>,
    mark_facet_data_scope: FacetDataScope,
    seed_derived_scalars: &DerivedScalarMap,
) -> Result<(Option<DataFrame>, DerivedScalarMap), AvengerChartError> {
    let outcome = execute_transform_chain(
        dataframe,
        transforms,
        ctx,
        facet_data_scope,
        mark_facet_data_scope,
        TransformChainMode::Ordinary { eval_ctx },
        seed_derived_scalars,
    )
    .await?;
    Ok((outcome.dataframe, outcome.derived_scalars))
}

async fn execute_transform_chain(
    dataframe: Option<DataFrame>,
    transforms: &[DataTransformStage],
    ctx: &SessionContext,
    facet_data_scope: Option<FacetDataScopeContext<'_>>,
    mark_facet_data_scope: FacetDataScope,
    mode: TransformChainMode<'_>,
    seed_derived_scalars: &DerivedScalarMap,
) -> Result<TransformChainOutcome, AvengerChartError> {
    if transforms.is_empty() {
        return Ok(TransformChainOutcome {
            dataframe,
            derived_scalars: DerivedScalarMap::new(),
            materialization_request_count: 0,
            desired_materialization_ready: false,
        });
    }
    let dataframe = dataframe.unwrap_or_else(|| empty_dataframe(ctx));
    let transforms = scoped_transform_stages(transforms, mark_facet_data_scope)?;
    let mut dataframe = dataframe;
    // Chain-produced scalars only. The seed (scalars inherited from prepared
    // base / mark-group data) participates in stage-expr resolution and
    // collision detection via `known_derived_scalars`, but is never returned:
    // call sites merge inherited scalars themselves after the chain.
    let mut derived_scalars = DerivedScalarMap::new();
    let mut known_derived_scalars = seed_derived_scalars.clone();
    let mut desired_materialization_ready = false;
    let mut materialization_request_count = 0;
    let mut current_level = transforms
        .first()
        .map(|stage| stage.level)
        .unwrap_or_else(|| mark_facet_data_scope.sharing_level());

    dataframe = filter_dataframe_to_transform_scope(
        dataframe,
        facet_data_scope,
        current_level,
        mode.initial_scope_label(),
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
                mode.narrower_scope_label(),
            )?;
            current_level = stage.level;
        }

        let available_columns = dataframe
            .schema()
            .fields()
            .iter()
            .map(|field| field.name().clone())
            .collect::<HashSet<_>>();
        let transform = stage
            .transform
            .map_exprs(&mut |expr| {
                let expr = resolve_known_derived_scalars(expr, &known_derived_scalars)?;
                // Stages run in order, so a derived-scalar reference that is
                // still unresolved here can never be satisfied (unlike
                // channels, which may consume scalars produced by later
                // stages).
                if let Some(id) = collect_derived_scalar_ids(&expr)?.into_iter().next() {
                    return Err(AvengerChartError::DataFusionError(
                        datafusion::error::DataFusionError::Plan(format!(
                            "Derived scalar '{id}' was referenced but not produced in this data scope"
                        )),
                    ));
                }
                expand_selection_predicates(expr, mode.eval_ctx(), Some(&available_columns))
            })
            .map_err(explain_stage_subquery_serialization_error)?;
        let facet_context =
            transform_facet_context(facet_data_scope, current_level, mark_facet_data_scope);
        let transform_ctx = DataTransformExecutionContext {
            session_context: ctx,
            params: mode.eval_ctx().params(),
            time_context: mode.eval_ctx().time_context().clone(),
            facet_context: facet_context.clone(),
        };

        if let TransformChainMode::View {
            eval_ctx,
            view_scope,
            materialization_handling,
        } = &mode
        {
            let materialization_ctx = ViewMaterializationContext {
                session_context: ctx,
                params: eval_ctx.params(),
                time_context: eval_ctx.time_context().clone(),
                facet_context: facet_context.as_ref(),
                policy: materialization_policy_for_view(view_scope),
                priority: eval_ctx.materialization_priority(),
            };
            if let Some(mut materialization) =
                transform.view_materialization_request(&dataframe, &materialization_ctx)?
            {
                // Identity from the UNRESOLVED stage transform: the request
                // above was built from the derived-scalar-resolved copy, and
                // baked-in scalar values (an eager in-view count feeding a
                // density normalizer, say) would otherwise churn the identity
                // with every count change, leaving stale-result fallback with
                // nothing to re-display mid-gesture.
                if let Some(identity) = stage
                    .transform
                    .view_materialization_identity(&dataframe, &materialization_ctx)?
                {
                    materialization.request.identity = Some(identity);
                }
                materialization.request.policy = materialization_ctx.policy;
                materialization.request.priority = materialization_ctx.priority;
                if materialization_handling.emits_request() {
                    eval_ctx.request_materialization(materialization.request.clone());
                    materialization_request_count += 1;
                }
                let (display_dataframe, desired_ready) = dataframe_for_materialization_display(
                    &materialization,
                    ctx,
                    eval_ctx,
                    *materialization_handling,
                )?;
                dataframe = display_dataframe;
                desired_materialization_ready |= desired_ready;
                continue;
            }

            // Scalar-producing stages (an eager in-view ScalarAggregate
            // feeding a density normalizer or adaptive gate): during
            // previews, serve the derived scalars from the materialization
            // cache instead of executing the aggregation synchronously
            // inside the evaluation — that synchronous collect is a
            // per-throttle-window frame hitch at large row counts. Only
            // RetargetCached views opt in (`allow_stale`), the same contract
            // that lets the displayed raster go stale mid-gesture.
            if materialization_ctx.policy.allow_stale
                && let Some(mut scalar_materialization) = transform
                    .view_scalar_materialization_request(&dataframe, &materialization_ctx)?
            {
                // Identity from the UNRESOLVED stage transform, same law as
                // display materializations: resolved params/derived scalars
                // baked into the identity would churn it every frame.
                if let Some(identity) = stage
                    .transform
                    .view_materialization_identity(&dataframe, &materialization_ctx)?
                {
                    scalar_materialization.request.identity = Some(identity);
                }
                scalar_materialization.request.priority = materialization_ctx.priority;
                scalar_materialization.request.policy = materialization_ctx.policy;

                // Preview evaluations (negative priority) take the async
                // path. Exact evaluations fall through to the synchronous
                // evaluation below — their scalars must be exact — and warm
                // the cache for the first preview.
                if scalar_materialization.request.priority < 0.0 {
                    if materialization_handling.emits_request() {
                        eval_ctx.request_materialization(scalar_materialization.request.clone());
                        materialization_request_count += 1;
                    }
                    if let Some(scalars) = scalars_from_materialization_display(
                        &scalar_materialization,
                        eval_ctx,
                        *materialization_handling,
                    )? {
                        eval_ctx.record_view_scalar_async_use();
                        insert_chain_derived_scalars(
                            scalars,
                            &mut known_derived_scalars,
                            &mut derived_scalars,
                        )?;
                        continue;
                    }
                    // Cold start: no materialized value for this identity
                    // yet — evaluate synchronously once and warm the cache.
                    eval_ctx.record_view_scalar_sync();
                }

                let result = transform.apply(dataframe, &transform_ctx).await?;
                dataframe = result.dataframe;
                if let Some(cache) = eval_ctx.materialization_cache() {
                    match avenger_chart_transforms::scalar_batch_from_literals(
                        &scalar_materialization.measure_names,
                        &result.derived_scalars,
                    ) {
                        Ok(batch) => cache
                            .lock()
                            .expect("materialization cache lock poisoned")
                            .mark_ready(
                                &scalar_materialization.request,
                                MaterializationResult::RecordBatch(batch),
                            ),
                        Err(err) => tracing::debug!(
                            target: "avenger_chart::materialization",
                            error = %err,
                            "failed to warm scalar materialization cache from eager result"
                        ),
                    }
                }
                insert_chain_derived_scalars(
                    result.derived_scalars,
                    &mut known_derived_scalars,
                    &mut derived_scalars,
                )?;
                continue;
            }
        }

        let result = transform.apply(dataframe, &transform_ctx).await?;
        dataframe = result.dataframe;
        insert_chain_derived_scalars(
            result.derived_scalars,
            &mut known_derived_scalars,
            &mut derived_scalars,
        )?;
    }

    let final_level = mark_facet_data_scope.sharing_level();
    if final_level < current_level {
        dataframe = filter_dataframe_to_transform_scope(
            dataframe,
            facet_data_scope,
            final_level,
            mode.final_scope_label(),
        )?;
    }

    Ok(TransformChainOutcome {
        dataframe: Some(dataframe),
        derived_scalars,
        materialization_request_count,
        desired_materialization_ready,
    })
}

fn materialization_policy_for_view(view_scope: &CompiledViewScope) -> MaterializationPolicy {
    let policy = view_scope.spec.policy();
    MaterializationPolicy {
        allow_stale: matches!(policy.stale_policy, ViewStalePolicy::RetargetCached),
        throttle: policy.throttle,
        debounce: policy.debounce,
    }
}

fn dataframe_from_materialization_result(
    result: MaterializationResult,
    ctx: &SessionContext,
) -> Result<DataFrame, AvengerChartError> {
    match result {
        MaterializationResult::RecordBatch(batch) => ctx
            .read_batch(batch)
            .map_err(AvengerChartError::DataFusionError),
        MaterializationResult::RgbaImage(_) => Err(AvengerChartError::InvalidArgument(
            "View-local data materialization expected a RecordBatch result, got RgbaImage"
                .to_string(),
        )),
    }
}

/// Derived-scalar literals for a scalar-producing view stage from the
/// materialization cache: the desired key's ready batch, else the newest
/// ready batch for the stage's identity (a slightly stale value from earlier
/// in the gesture). `None` means no materialized value exists yet — the
/// caller falls back to synchronous evaluation.
///
/// Unlike display materializations, scalar readiness must NOT feed
/// `desired_materialization_ready` (which declines mark retargeting to
/// consume a fresh display): a fresh scalar only changes future child keys.
/// No settled-pin either — scalars are not displayed, so newest-ready always
/// wins; pinning would only delay adaptive gate flips.
fn scalars_from_materialization_display(
    materialization: &avenger_chart_core::ViewScalarMaterialization,
    eval_ctx: &EvaluationContext,
    handling: ViewMaterializationHandling,
) -> Result<Option<DerivedScalarMap>, AvengerChartError> {
    let Some(cache) = eval_ctx.materialization_cache() else {
        return Ok(None);
    };
    let mut cache = cache.lock().expect("materialization cache lock poisoned");

    let scalars_from_result = |result: MaterializationResult| match result {
        MaterializationResult::RecordBatch(batch) => {
            avenger_chart_transforms::scalar_literals_from_batch(
                &materialization.measure_names,
                &batch,
            )
        }
        MaterializationResult::RgbaImage(_) => Err(AvengerChartError::InvalidArgument(
            "Scalar materialization expected a RecordBatch result, got RgbaImage".to_string(),
        )),
    };

    if let Some(result) = cache.get_ready(&materialization.request.key) {
        return scalars_from_result(result).map(Some);
    }

    if handling.enqueues_cache_miss() {
        // `enqueue` re-queues an errored entry (so a transient failure retries)
        // and therefore never returns `Error`, so observe the prior failure
        // here before it is overwritten. Otherwise `materialize_errors` never
        // increments and a broken materialization is invisible in the metrics.
        if matches!(
            cache.status(&materialization.request.key),
            MaterializationStatus::Error(_)
        ) {
            eval_ctx.record_materialization_error();
        }
        match cache.enqueue(materialization.request.clone()) {
            MaterializationStatus::Queued => eval_ctx.record_materialization_queued(),
            MaterializationStatus::Running => eval_ctx.record_materialization_running(),
            MaterializationStatus::Ready | MaterializationStatus::Missing => {}
            MaterializationStatus::Error(_) => {}
        }
    }

    if materialization.request.policy.allow_stale
        && let Some(identity) = &materialization.request.identity
        && let Some((_key, result)) = cache.stale_fallback_ready(identity, false)
    {
        return scalars_from_result(result).map(Some);
    }

    Ok(None)
}

/// Insert chain-produced derived scalars with duplicate detection.
fn insert_chain_derived_scalars(
    produced: DerivedScalarMap,
    known_derived_scalars: &mut DerivedScalarMap,
    derived_scalars: &mut DerivedScalarMap,
) -> Result<(), AvengerChartError> {
    for (id, expr) in produced {
        if known_derived_scalars
            .insert(id.clone(), expr.clone())
            .is_some()
        {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Derived scalar '{id}' was produced more than once in the same data scope"
            )));
        }
        derived_scalars.insert(id, expr);
    }
    Ok(())
}

/// Display dataframe for a view materialization plus whether the desired
/// key itself was ready (as opposed to a stale fallback or empty payload).
fn dataframe_for_materialization_display(
    materialization: &ViewMaterializationRequest,
    ctx: &SessionContext,
    eval_ctx: &EvaluationContext,
    handling: ViewMaterializationHandling,
) -> Result<(DataFrame, bool), AvengerChartError> {
    let Some(cache) = eval_ctx.materialization_cache() else {
        eval_ctx.record_materialization_cache_miss();
        return Ok((
            materialization
                .empty_dataframe
                .clone()
                .unwrap_or_else(|| empty_dataframe(ctx)),
            false,
        ));
    };

    let mut cache = cache.lock().expect("materialization cache lock poisoned");

    // For preview retarget decisions, track how long this identity's desired
    // key has been unchanged. While the view params are still moving
    // frame-to-frame, a ready result is NOT reported as consumable — the
    // preview keeps retargeting the cached scene — and a delayed
    // re-evaluation is requested so a hold/release still consumes it.
    let preview_key_stable_for = (handling
        == ViewMaterializationHandling::PreviewRetargetScheduleOnly
        && materialization.request.priority < 0.0)
        .then(|| cache.note_preview_desired_key(&materialization.request, Instant::now()));

    if let Some(result) = cache.get_ready(&materialization.request.key) {
        eval_ctx.record_materialization_cache_hit();
        eval_ctx.record_materialization_ready_used();
        let consume_now = match preview_key_stable_for {
            Some(stable_for) if stable_for < PREVIEW_CONSUME_STABILITY => {
                cache.defer_preview_consume(
                    &materialization.request,
                    PREVIEW_CONSUME_STABILITY - stable_for,
                );
                false
            }
            _ => true,
        };
        return dataframe_from_materialization_result(result, ctx).map(|df| (df, consume_now));
    }

    eval_ctx.record_materialization_cache_miss();
    if handling.enqueues_cache_miss() {
        // `enqueue` re-queues an errored entry (so a transient failure retries)
        // and therefore never returns `Error`, so observe the prior failure
        // here before it is overwritten. Otherwise `materialize_errors` never
        // increments and a broken materialization is invisible in the metrics.
        if matches!(
            cache.status(&materialization.request.key),
            MaterializationStatus::Error(_)
        ) {
            eval_ctx.record_materialization_error();
        }
        match cache.enqueue(materialization.request.clone()) {
            MaterializationStatus::Queued => eval_ctx.record_materialization_queued(),
            MaterializationStatus::Running => eval_ctx.record_materialization_running(),
            MaterializationStatus::Ready => {
                eval_ctx.record_materialization_cache_hit();
                eval_ctx.record_materialization_ready_used();
            }
            MaterializationStatus::Missing => {}
            MaterializationStatus::Error(_) => {}
        }
    }

    // The settled-result pin only makes sense on the retarget path, where
    // one cached scene stays on screen mid-gesture; full re-render paths
    // must track the newest ready result or they flash between the settled
    // raster and just-completed mid-gesture ones.
    let prefer_settled_fallback = handling
        == ViewMaterializationHandling::PreviewRetargetScheduleOnly
        && materialization.request.priority < 0.0;
    if handling.uses_stale_fallback()
        && materialization.request.policy.allow_stale
        && let Some(identity) = &materialization.request.identity
        && let Some((_key, result)) = cache.stale_fallback_ready(identity, prefer_settled_fallback)
    {
        if handling != ViewMaterializationHandling::ScaleInferenceReadOnly {
            eval_ctx.record_materialization_stale_fallback_used();
        }
        return dataframe_from_materialization_result(result, ctx).map(|df| (df, false));
    }

    tracing::debug!(
        target: "avenger_chart::materialization",
        key = %materialization.request.key,
        handling = ?handling,
        allow_stale = materialization.request.policy.allow_stale,
        has_identity = materialization.request.identity.is_some(),
        fallback_available = materialization
            .request
            .identity
            .as_ref()
            .and_then(|identity| cache.stale_fallback_ready(identity, prefer_settled_fallback))
            .is_some(),
        "materialization display fell through to empty payload"
    );
    Ok((
        materialization
            .empty_dataframe
            .clone()
            .unwrap_or_else(|| empty_dataframe(ctx)),
        false,
    ))
}

async fn apply_view_mark_data_transforms(
    dataframe: Option<DataFrame>,
    transforms: &[DataTransformStage],
    ctx: &SessionContext,
    eval_ctx: &EvaluationContext,
    facet_data_scope: Option<FacetDataScopeContext<'_>>,
    mark_facet_data_scope: FacetDataScope,
    view_scope: &CompiledViewScope,
    materialization_handling: ViewMaterializationHandling,
    seed_derived_scalars: &DerivedScalarMap,
) -> Result<(Option<DataFrame>, DerivedScalarMap), AvengerChartError> {
    let outcome = execute_transform_chain(
        dataframe,
        transforms,
        ctx,
        facet_data_scope,
        mark_facet_data_scope,
        TransformChainMode::View {
            eval_ctx,
            view_scope,
            materialization_handling,
        },
        seed_derived_scalars,
    )
    .await?;
    Ok((outcome.dataframe, outcome.derived_scalars))
}

/// Compiled transform and channel expressions are stored in protobuf form,
/// which the pinned DataFusion version cannot serialize scalar subqueries
/// into. When a lazy `ScalarAggregate` scalar gets resolved into a stage or
/// channel expression, the rewrite fails with an opaque proto error — map it
/// to an actionable message. (DataFusion >= 54 serializes `ScalarSubquery`,
/// at which point these paths succeed and the wrapper is inert.)
fn explain_stage_subquery_serialization_error(err: AvengerChartError) -> AvengerChartError {
    let message = err.to_string();
    if message.contains("ScalarSubquery") && message.contains("Proto serialization") {
        AvengerChartError::InvalidArgument(format!(
            "A lazily evaluated derived scalar (for example a lazy ScalarAggregate measure) \
             was referenced by a transform stage or channel expression, but compiled \
             expressions cannot represent scalar subqueries in this DataFusion version. \
             Use the default eager evaluation mode. Underlying error: {message}"
        ))
    } else {
        err
    }
}

fn merge_derived_scalars(
    mut base: DerivedScalarMap,
    additions: DerivedScalarMap,
) -> Result<DerivedScalarMap, AvengerChartError> {
    for (id, expr) in additions {
        if base.insert(id.clone(), expr).is_some() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Derived scalar '{id}' was produced more than once in the same data scope"
            )));
        }
    }
    Ok(base)
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

fn transform_facet_context(
    facet_data_scope: Option<FacetDataScopeContext<'_>>,
    transform_level: SharingLevel,
    mark_facet_data_scope: FacetDataScope,
) -> Option<DataTransformFacetContext> {
    let scope = facet_data_scope?;
    let final_mark_level = mark_facet_data_scope.sharing_level();
    let partition_exprs = scope.facet_tree.partition_exprs_between_sharing_levels(
        scope.full_path,
        transform_level,
        final_mark_level,
    );
    Some(DataTransformFacetContext {
        transform_level,
        final_mark_level,
        partition_exprs,
    })
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
        tracing::debug!(
            target: "avenger_chart::facet_scope",
            ?level,
            full_path = ?scope.full_path,
            label,
            "facet cell predicate unavailable; transform scope narrowing skipped"
        );
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
    tracing::debug!(
        target: "avenger_chart::facet_scope",
        ?level,
        full_path = ?scope.full_path,
        %predicate,
        label,
        "transform scope narrowing applied"
    );
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

fn validate_no_item_frame_refs_in_mark_channels(
    mark: &dyn CompiledMark,
    channels: &IndexMap<String, ChannelValue>,
    ctx: &SessionContext,
) -> Result<(), AvengerChartError> {
    for (channel_name, channel_value) in channels {
        validate_no_item_frame_refs_in_exprs(
            mark,
            channel_name,
            "channel expression",
            channel_value.all_exprs(ctx),
        )?;
        if let Some(config) = channel_value.get_scale_config() {
            validate_no_item_frame_refs_in_exprs(
                mark,
                channel_name,
                "scale configuration",
                config.all_exprs(ctx),
            )?;
        }
        if let Some(axis) = channel_value.get_axis_config() {
            validate_no_item_frame_refs_in_exprs(
                mark,
                channel_name,
                "axis configuration",
                axis.all_exprs(ctx),
            )?;
        }
        if let Some(legend) = channel_value.get_legend_config() {
            validate_no_item_frame_refs_in_exprs(
                mark,
                channel_name,
                "legend configuration",
                legend.all_exprs(ctx),
            )?;
        }
    }
    for (channel_name, axis) in &mark.state().axis_configs {
        validate_no_item_frame_refs_in_exprs(
            mark,
            channel_name,
            "axis configuration",
            axis.all_exprs(ctx),
        )?;
    }
    Ok(())
}

fn validate_no_item_frame_refs_in_exprs(
    mark: &dyn CompiledMark,
    channel_name: &str,
    context: &str,
    exprs: impl IntoIterator<Item = Expr>,
) -> Result<(), AvengerChartError> {
    for expr in exprs {
        let refs = item_frame_column_refs(&expr);
        if let Some(reference) = refs.first() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Item-frame expression reference '{reference}' is only valid inside mark effect closures; mark '{}' channel '{}' {context} is evaluated before item frames exist",
                mark.mark_type(),
                channel_name
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
                position_boundary,
                scale_config,
                nested_band_config,
                legend_config,
                axis_config,
                domain_coordination,
                transform_scope,
                scale_domain_inference,
            } => {
                let expanded =
                    expand_selection_predicates(expr.to_expr(ctx)?, eval_ctx, available_columns)?;
                ChannelValue::Scaled {
                    expr: LogicalExprNode::from_expr(expanded)?,
                    scale_name,
                    position_boundary,
                    scale_config,
                    nested_band_config,
                    legend_config,
                    axis_config,
                    domain_coordination,
                    transform_scope,
                    scale_domain_inference,
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
                nested_band_config,
                legend_config,
                axis_config,
                domain_coordination,
                transform_scope,
                scale_domain_inference,
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
                    nested_band_config,
                    legend_config,
                    axis_config,
                    domain_coordination,
                    transform_scope,
                    scale_domain_inference,
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
    fallback_specs: Option<&CompiledStateRegistry<SelectionRef, CompiledSelectionSpec>>,
) -> Result<Expr, AvengerChartError> {
    let ctx = eval_ctx.session_context.as_ref();
    expr.transform(|candidate| {
        if let Some((selection_id, field_expr, value_expr)) =
            selection_equality_membership_marker(&candidate)
        {
            let replacement = selection_equality_membership_expr(
                selection_id,
                field_expr,
                value_expr,
                eval_ctx,
                fallback_specs,
            )
            .map_err(|err| datafusion::error::DataFusionError::Plan(err.to_string()))?;
            return Ok(Transformed::yes(replacement));
        }
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

pub(crate) fn expand_selection_predicates_with_lookup(
    expr: Expr,
    ctx: &SessionContext,
    available_columns: Option<&HashSet<String>>,
    mut lookup: impl FnMut(&str) -> Option<(CompiledSelectionSpec, Vec<SelectionClause>)>,
) -> Result<Expr, AvengerChartError> {
    expr.transform(|candidate| {
        if let Some((selection_id, field_expr, value_expr)) =
            selection_equality_membership_marker(&candidate)
        {
            let Some((_spec, clauses)) = lookup(selection_id) else {
                return Err(datafusion::error::DataFusionError::Plan(format!(
                    "Selection membership references unknown selection '{selection_id}'"
                )));
            };
            let replacement =
                selection_equality_membership_expr_from_clauses(field_expr, value_expr, &clauses)
                    .map_err(|err| datafusion::error::DataFusionError::Plan(err.to_string()))?;
            return Ok(Transformed::yes(replacement));
        }
        if let Expr::Placeholder(placeholder) = &candidate
            && let Some(selection_id) = selection_id_from_predicate_placeholder(&placeholder.id)
        {
            let Some((spec, clauses)) = lookup(selection_id) else {
                return Err(datafusion::error::DataFusionError::Plan(format!(
                    "Selection predicate references unknown selection '{selection_id}'"
                )));
            };
            let replacement =
                selection_predicate_expr_from_parts(&spec, &clauses, ctx, available_columns)
                    .map_err(|err| datafusion::error::DataFusionError::Plan(err.to_string()))?;
            return Ok(Transformed::yes(replacement));
        }
        Ok(Transformed::no(candidate))
    })
    .map(|transformed| transformed.data)
    .map_err(AvengerChartError::DataFusionError)
}

fn selection_equality_membership_marker(expr: &Expr) -> Option<(&str, Expr, Expr)> {
    let Expr::BinaryExpr(combined) = expr else {
        return None;
    };
    if combined.op != Operator::And {
        return None;
    }

    let direct = || {
        let (field_selection, field_expr) =
            selection_equality_membership_field_marker(combined.left.as_ref())?;
        let (value_selection, value_expr) =
            selection_equality_membership_value_marker(combined.right.as_ref())?;
        (field_selection == value_selection).then_some((field_selection, field_expr, value_expr))
    };
    direct().or_else(|| {
        let (value_selection, value_expr) =
            selection_equality_membership_value_marker(combined.left.as_ref())?;
        let (field_selection, field_expr) =
            selection_equality_membership_field_marker(combined.right.as_ref())?;
        (field_selection == value_selection).then_some((field_selection, field_expr, value_expr))
    })
}

fn selection_equality_membership_field_marker(expr: &Expr) -> Option<(&str, Expr)> {
    selection_equality_membership_operand(
        expr,
        selection_id_from_equality_membership_field_placeholder,
    )
}

fn selection_equality_membership_value_marker(expr: &Expr) -> Option<(&str, Expr)> {
    selection_equality_membership_operand(
        expr,
        selection_id_from_equality_membership_value_placeholder,
    )
}

fn selection_equality_membership_operand<'a>(
    expr: &'a Expr,
    parse_placeholder: impl Fn(&'a str) -> Option<&'a str>,
) -> Option<(&'a str, Expr)> {
    let Expr::BinaryExpr(binary) = expr else {
        return None;
    };
    if binary.op != Operator::Eq {
        return None;
    }
    match (binary.left.as_ref(), binary.right.as_ref()) {
        (operand, Expr::Placeholder(placeholder)) => {
            parse_placeholder(&placeholder.id).map(|selection_id| (selection_id, operand.clone()))
        }
        (Expr::Placeholder(placeholder), operand) => {
            parse_placeholder(&placeholder.id).map(|selection_id| (selection_id, operand.clone()))
        }
        _ => None,
    }
}

fn selection_equality_membership_expr(
    selection_id: &str,
    field_expr: Expr,
    value_expr: Expr,
    eval_ctx: &EvaluationContext,
    fallback_specs: Option<&CompiledStateRegistry<SelectionRef, CompiledSelectionSpec>>,
) -> Result<Expr, AvengerChartError> {
    let clauses = if let Some(selection_store) = eval_ctx.scoped_selection_store.as_ref() {
        if selection_store.spec_for_source_name(selection_id).is_none() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Selection membership references unknown selection '{selection_id}'"
            )));
        }
        selection_store
            .clauses_for_selection(selection_id)
            .unwrap_or_default()
    } else if let Some(specs) = fallback_specs {
        if !specs.contains_key(selection_id) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Selection membership references unknown selection '{selection_id}'"
            )));
        }
        Vec::new()
    } else {
        return Ok(lit(false));
    };

    selection_equality_membership_expr_from_clauses(field_expr, value_expr, &clauses)
}

fn selection_equality_membership_expr_from_clauses(
    field_expr: Expr,
    value_expr: Expr,
    clauses: &[SelectionClause],
) -> Result<Expr, AvengerChartError> {
    let field_fingerprint =
        selection_field_expr_fingerprint(&LogicalExprNode::from_expr(field_expr)?);
    let matching_values = clauses.iter().filter_map(|clause| {
        let SelectionPredicateSpec::Equality { dimensions } = &clause.predicate else {
            return None;
        };
        let [dimension] = dimensions.as_slice() else {
            return None;
        };
        (!dimension.value.is_null()
            && selection_field_expr_fingerprint(&dimension.field_expr) == field_fingerprint)
            .then(|| dimension.value.clone())
    });

    Ok(matching_values.fold(lit(false), |membership, value| {
        membership.or(value_expr.clone().eq(lit(value)))
    }))
}

fn selection_predicate_expr(
    selection_id: &str,
    eval_ctx: &EvaluationContext,
    ctx: &SessionContext,
    available_columns: Option<&HashSet<String>>,
    fallback_specs: Option<&CompiledStateRegistry<SelectionRef, CompiledSelectionSpec>>,
) -> Result<Expr, AvengerChartError> {
    let (spec, clauses) = if let Some(selection_store) = eval_ctx.scoped_selection_store.as_ref() {
        let Some(spec) = selection_store.spec_for_source_name(selection_id) else {
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
    selection_predicate_expr_from_parts(spec, &clauses, ctx, available_columns)
}

fn selection_predicate_expr_from_parts(
    spec: &CompiledSelectionSpec,
    clauses: &[SelectionClause],
    ctx: &SessionContext,
    available_columns: Option<&HashSet<String>>,
) -> Result<Expr, AvengerChartError> {
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
    force_rows: bool,
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

    if !force_rows && !channel_exprs_reference_columns(channels, ctx) {
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

fn validate_narrowed_scope(
    requested: FacetDataScope,
    inherited: FacetDataScope,
    label: &str,
) -> Result<(), AvengerChartError> {
    if requested.sharing_level() > inherited.sharing_level() {
        return Err(AvengerChartError::InvalidArgument(format!(
            "{label} cannot broaden inherited facet data scope from {:?} to {:?}",
            inherited.sharing_level(),
            requested.sharing_level()
        )));
    }
    Ok(())
}

fn dataframe_for_base_data(
    request: &BaseDataRequest<'_>,
    ctx: &SessionContext,
) -> Result<Option<DataFrame>, AvengerChartError> {
    if let Some(store_data) = request.data_context.store_data() {
        return store_dataframe(
            store_data,
            request.facet_data_scope_context,
            request.eval_ctx,
        )
        .map(Some);
    }

    if request.data_mode == MarkDataMode::Unit {
        return Ok(None);
    }

    if let Some(df) = request.data_context.dataframe_with_context(ctx) {
        return inherited_data_for_scope(
            Some(&df),
            request.facet_data_scope,
            request.facet_data_scope_context,
        );
    }

    if let Some(inherited) = request.inherited_base {
        validate_narrowed_scope(
            request.facet_data_scope,
            inherited.facet_data_scope,
            "Inherited MarkGroup",
        )?;
        return inherited_data_for_scope(
            inherited.dataframe.as_ref(),
            request.facet_data_scope,
            request.facet_data_scope_context,
        );
    }

    if let Some(df_override) = inherited_data_for_scope(
        request.provided_plot_df,
        request.facet_data_scope,
        request.facet_data_scope_context,
    )? {
        return Ok(Some(df_override));
    }

    if let Some(df) = request.plot_data.and_then(|node| {
        node.to_logical_plan(ctx)
            .ok()
            .map(|plan| DataFrame::new(ctx.state().clone(), plan))
    }) {
        return inherited_data_for_scope(
            Some(&df),
            request.facet_data_scope,
            request.facet_data_scope_context,
        );
    }

    Ok(None)
}

pub(crate) async fn prepare_base_data(
    request: BaseDataRequest<'_>,
) -> Result<PreparedBaseData, AvengerChartError> {
    let ctx = request.eval_ctx.session_context.as_ref();
    let dataframe = dataframe_for_base_data(&request, ctx)?;
    let empty_seed = DerivedScalarMap::new();
    let seed_derived_scalars = request
        .inherited_base
        .map(|inherited| &inherited.derived_scalars)
        .unwrap_or(&empty_seed);
    let (dataframe, derived_scalars) = apply_mark_data_transforms(
        dataframe,
        request.data_context.transforms(),
        ctx,
        request.eval_ctx,
        request.facet_data_scope_context,
        request.facet_data_scope,
        seed_derived_scalars,
    )
    .await?;
    let derived_scalars = match request.inherited_base {
        Some(inherited) => {
            merge_derived_scalars(inherited.derived_scalars.clone(), derived_scalars)?
        }
        None => derived_scalars,
    };
    Ok(PreparedBaseData {
        dataframe,
        derived_scalars,
        facet_data_scope: request.facet_data_scope,
    })
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

async fn finalize_logical_mark_data(
    dataframe: Option<DataFrame>,
    channels: IndexMap<String, ChannelValue>,
    derived_scalars: DerivedScalarMap,
    ctx: &SessionContext,
) -> Result<PreparedLogicalMarkData, AvengerChartError> {
    // Resolve chain-produced derived scalars into channel data expressions
    // once, so every downstream consumer (domain inference, render channel
    // collection, sorting, aggregate preparation) sees resolved values.
    let channels = if derived_scalars.is_empty() {
        channels
    } else {
        channels
            .into_iter()
            .map(|(name, value)| {
                let value =
                    resolve_known_derived_scalars_in_channel_value(value, &derived_scalars, ctx)
                        .map_err(explain_stage_subquery_serialization_error)?;
                Ok((name, value))
            })
            .collect::<Result<IndexMap<_, _>, AvengerChartError>>()?
    };
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
                    let new_expr = LogicalExprNode::from_expr(exact_schema_col(field_name))?;
                    updated_channels.insert(name, value.with_expr(new_expr));
                } else {
                    let group_index = unique_group_exprs.get(&expr).unwrap();
                    let field_name = schema.field(*group_index).name().clone();
                    let new_expr = LogicalExprNode::from_expr(exact_schema_col(field_name))?;
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

/// Resolve channel references and run aggregate channel preparation on the
/// selected mark data. This intentionally happens at runtime so faceted marks
/// aggregate after their facet data scope has been selected.
pub(crate) async fn prepare_logical_mark_data(
    request: LogicalMarkDataRequest<'_>,
) -> Result<PreparedLogicalMarkData, AvengerChartError> {
    let ctx = request.eval_ctx.session_context.as_ref();
    let channels = resolve_all_channel_refs(request.mark.data_context().channels(), ctx)?;
    validate_no_item_frame_refs_in_mark_channels(request.mark, &channels, ctx)?;
    let force_rows_for_view = request.mark.state().view.is_some();
    let transform_initial_scope = transform_initial_facet_scope(
        request.mark.data_context().transforms(),
        request.mark.state().facet_data_scope,
    );
    let (dataframe, inherited_derived_scalars) = if let Some(prepared_base) = request.prepared_base
    {
        validate_narrowed_scope(
            transform_initial_scope,
            prepared_base.facet_data_scope,
            "Child mark",
        )?;
        let needs_rows = channel_exprs_reference_columns(&channels, ctx)
            || aggregate_channels_need_preparation(&channels, ctx)
            || force_rows_for_view
            || !request.mark.data_context().transforms().is_empty();
        let dataframe = if needs_rows {
            inherited_data_for_scope(
                prepared_base.dataframe.as_ref(),
                transform_initial_scope,
                request.facet_data_scope,
            )?
        } else {
            None
        };
        (dataframe, prepared_base.derived_scalars.clone())
    } else {
        (
            dataframe_for_mark(
                request.mark,
                request.plot_data,
                request.provided_plot_df,
                request.facet_data_scope,
                transform_initial_scope,
                &channels,
                force_rows_for_view,
                ctx,
                request.eval_ctx,
            )?,
            DerivedScalarMap::new(),
        )
    };
    let (dataframe, derived_scalars) = apply_mark_data_transforms(
        dataframe,
        request.mark.data_context().transforms(),
        ctx,
        request.eval_ctx,
        request.facet_data_scope,
        request.mark.state().facet_data_scope,
        &inherited_derived_scalars,
    )
    .await?;
    let derived_scalars = merge_derived_scalars(inherited_derived_scalars, derived_scalars)?;
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
    finalize_logical_mark_data(dataframe, channels, derived_scalars, ctx).await
}

fn rounded_pixel_count(size: f32) -> u32 {
    if size.is_finite() {
        size.round().max(1.0).min(u32::MAX as f32) as u32
    } else {
        1
    }
}

fn resolved_view_params(
    spec: &CompiledViewSpec,
    scales: &HashMap<String, ConfiguredScaleWithSpec>,
    plot_width: f32,
    plot_height: f32,
) -> Result<IndexMap<String, ScalarValue>, AvengerChartError> {
    match spec {
        CompiledViewSpec::Cartesian(_) => {
            let x_scale = scales
                .get("x")
                .ok_or_else(|| AvengerChartError::ScaleNotFound("x".to_string()))?;
            let y_scale = scales
                .get("y")
                .ok_or_else(|| AvengerChartError::ScaleNotFound("y".to_string()))?;
            // f64 accessor: lossless for coordinate-owned domains installed
            // as Float64 arrays (Geo viewports at Web-Mercator magnitudes
            // quantize near meter scale through f32); identical (widened)
            // for domains stored as f32.
            let (x_domain_start, x_domain_end) = x_scale
                .configured()
                .numeric_interval_domain_f64()
                .map_err(AvengerChartError::ScaleError)?;
            let (y_domain_start, y_domain_end) = y_scale
                .configured()
                .numeric_interval_domain_f64()
                .map_err(AvengerChartError::ScaleError)?;
            let (x_range_start, x_range_end) = x_scale
                .configured()
                .numeric_interval_range()
                .map_err(AvengerChartError::ScaleError)?;
            let (y_range_start, y_range_end) = y_scale
                .configured()
                .numeric_interval_range()
                .map_err(AvengerChartError::ScaleError)?;

            tracing::debug!(
                view_id = %spec.runtime_id(),
                view_name = spec.source_name(),
                x_domain_start,
                x_domain_end,
                y_domain_start,
                y_domain_end,
                plot_width,
                plot_height,
                "resolved view params"
            );
            let view_ref = spec.view_ref();
            let mut params = IndexMap::new();
            params.insert(
                view_ref.x().param_name("domain_start"),
                ScalarValue::Float64(Some(x_domain_start)),
            );
            params.insert(
                view_ref.x().param_name("domain_end"),
                ScalarValue::Float64(Some(x_domain_end)),
            );
            params.insert(
                view_ref.x().param_name("range_start"),
                ScalarValue::Float64(Some(x_range_start as f64)),
            );
            params.insert(
                view_ref.x().param_name("range_end"),
                ScalarValue::Float64(Some(x_range_end as f64)),
            );
            params.insert(
                view_ref.x().param_name("pixels"),
                ScalarValue::UInt32(Some(rounded_pixel_count(plot_width))),
            );
            params.insert(
                view_ref.y().param_name("domain_start"),
                ScalarValue::Float64(Some(y_domain_start)),
            );
            params.insert(
                view_ref.y().param_name("domain_end"),
                ScalarValue::Float64(Some(y_domain_end)),
            );
            params.insert(
                view_ref.y().param_name("range_start"),
                ScalarValue::Float64(Some(y_range_start as f64)),
            );
            params.insert(
                view_ref.y().param_name("range_end"),
                ScalarValue::Float64(Some(y_range_end as f64)),
            );
            params.insert(
                view_ref.y().param_name("pixels"),
                ScalarValue::UInt32(Some(rounded_pixel_count(plot_height))),
            );
            Ok(params)
        }
        CompiledViewSpec::PixelFrame(_) => {
            let view_ref = spec.view_ref();
            let mut params = IndexMap::new();
            for (axis, extent) in [(view_ref.x(), plot_width), (view_ref.y(), plot_height)] {
                params.insert(
                    axis.param_name("domain_start"),
                    ScalarValue::Float64(Some(0.0)),
                );
                params.insert(
                    axis.param_name("domain_end"),
                    ScalarValue::Float64(Some(extent as f64)),
                );
                params.insert(
                    axis.param_name("range_start"),
                    ScalarValue::Float64(Some(0.0)),
                );
                params.insert(
                    axis.param_name("range_end"),
                    ScalarValue::Float64(Some(extent as f64)),
                );
                params.insert(
                    axis.param_name("pixels"),
                    ScalarValue::UInt32(Some(rounded_pixel_count(extent))),
                );
            }
            Ok(params)
        }
    }
}

pub(crate) fn eval_ctx_with_view_params(
    eval_ctx: &EvaluationContext,
    view_scope: &CompiledViewScope,
    scales: &HashMap<String, ConfiguredScaleWithSpec>,
    plot_width: f32,
    plot_height: f32,
) -> Result<EvaluationContext, AvengerChartError> {
    let mut params = eval_ctx.params().clone();
    params.extend(resolved_view_params(
        &view_scope.spec,
        scales,
        plot_width,
        plot_height,
    )?);
    Ok(eval_ctx.with_params(params))
}

pub(crate) async fn prepare_view_logical_mark_data(
    mark: &dyn CompiledMark,
    view_scope: &CompiledViewScope,
    base_prepared: &PreparedLogicalMarkData,
    request: &MarkDataRequest<'_>,
    view_eval_ctx: &EvaluationContext,
    materialization_handling: ViewMaterializationHandling,
) -> Result<PreparedLogicalMarkData, AvengerChartError> {
    let ctx = request.eval_ctx.session_context.as_ref();
    let view_channels = resolve_all_channel_refs(view_scope.data.channels(), ctx)?;
    validate_no_item_frame_refs_in_mark_channels(mark, &view_channels, ctx)?;

    // Inside a viewed group, the child's view chain starts from the group's
    // shared view-local output (dataframe + derived scalars).
    let group_view_prepared = match request.group_view.as_ref() {
        Some(group_view) => Some(
            prepare_group_view_data(
                group_view,
                base_prepared,
                request,
                view_eval_ctx,
                materialization_handling,
            )
            .await?,
        ),
        None => None,
    };
    let inherited_derived_scalars = match group_view_prepared.as_ref() {
        Some(shared) => merge_derived_scalars(
            base_prepared.derived_scalars.clone(),
            shared.derived_scalars.clone(),
        )?,
        None => base_prepared.derived_scalars.clone(),
    };

    let dataframe = if let Some(store_data) = view_scope.data.store_data() {
        Some(store_dataframe(
            store_data,
            request.facet_data_scope,
            view_eval_ctx,
        )?)
    } else if let Some(dataframe) = view_scope.data.dataframe_with_context(ctx) {
        Some(dataframe)
    } else if let Some(shared) = group_view_prepared.as_ref() {
        shared.dataframe.clone()
    } else {
        base_prepared.dataframe.clone()
    };

    let (dataframe, view_derived_scalars) = apply_view_mark_data_transforms(
        dataframe,
        view_scope.data.transforms(),
        ctx,
        view_eval_ctx,
        request.facet_data_scope,
        mark.state().facet_data_scope,
        view_scope,
        materialization_handling,
        &inherited_derived_scalars,
    )
    .await?;
    let derived_scalars = merge_derived_scalars(inherited_derived_scalars, view_derived_scalars)?;
    let available_columns = dataframe.as_ref().map(|df| {
        df.schema()
            .fields()
            .iter()
            .map(|field| field.name().clone())
            .collect::<HashSet<_>>()
    });
    let view_channels = expand_selection_predicates_in_channels(
        view_channels,
        view_eval_ctx,
        available_columns.as_ref(),
    )?;

    let mut channels = base_prepared.channels.clone();
    channels.extend(view_channels);
    finalize_logical_mark_data(dataframe, channels, derived_scalars, ctx).await
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ViewMaterializationSchedule {
    pub(crate) request_count: usize,
    pub(crate) can_retarget_cached_scene: bool,
}

pub(crate) async fn schedule_view_materializations_for_mark(
    request: MarkDataRequest<'_>,
) -> Result<ViewMaterializationSchedule, AvengerChartError> {
    let mark = request.mark;
    let Some(view_scope) = mark.state().view.as_ref() else {
        return Ok(ViewMaterializationSchedule::default());
    };
    if !matches!(
        view_scope.spec.policy().stale_policy,
        ViewStalePolicy::RetargetCached
    ) {
        return Ok(ViewMaterializationSchedule::default());
    }

    // The schedule-only pass runs the view chain (including eager scalar
    // aggregations over the source data) just to compute materialization
    // keys, so a view throttle rate-limits the pass itself, not only the
    // request starts. Skipped frames stay on the pure retarget path; the
    // recorded wakeup re-runs the pass once the window elapses so the
    // settled view state still gets scheduled and consumed.
    if let Some(throttle) = view_scope.spec.policy().throttle
        && let Some(cache) = request.eval_ctx.materialization_cache()
    {
        let mut identity = format!(
            "preview-schedule:{}:{}",
            view_scope.spec.source_name(),
            mark.state().mark_index()
        );
        if let Some(scope) = request.facet_data_scope.as_ref() {
            use std::fmt::Write as _;
            let _ = write!(identity, ":{:?}", scope.full_path);
        }
        let should_run = cache
            .lock()
            .expect("materialization cache lock poisoned")
            .should_run_preview_schedule(
                &avenger_chart_core::MaterializationIdentity::new(identity),
                throttle,
                Instant::now(),
            );
        if !should_run {
            return Ok(ViewMaterializationSchedule {
                request_count: 0,
                can_retarget_cached_scene: true,
            });
        }
    }

    let prepared_storage;
    let base_prepared = if let Some(prepared) = request.prepared_logical {
        prepared
    } else {
        prepared_storage = prepare_logical_mark_data(LogicalMarkDataRequest {
            mark,
            plot_data: request.plot_data,
            provided_plot_df: request.provided_plot_df,
            facet_data_scope: request.facet_data_scope,
            prepared_base: request.prepared_base,
            eval_ctx: request.eval_ctx,
        })
        .await?;
        &prepared_storage
    };

    let view_eval_ctx = eval_ctx_with_view_params(
        request.eval_ctx,
        view_scope,
        request.scales,
        request.plot_width,
        request.plot_height,
    )?;
    let (request_count, desired_ready) = schedule_view_materialization_transforms(
        mark,
        view_scope,
        base_prepared,
        &request,
        &view_eval_ctx,
    )
    .await?;

    // `RetargetCached` is an explicit opt-in to stale previews: the cached
    // scene may be retargeted through the current scales while fresh
    // view-dependent results (materialized or synchronous) are pending, so
    // retargeting is allowed even for view chains without materialized
    // transforms. Non-RetargetCached views returned early above and always
    // rebuild during Preview. When the DESIRED materialization is already
    // ready, decline retargeting so the preview rebuilds data marks (under
    // the reused layout profile) and consumes the fresh result — otherwise a
    // session that never settles exactly would keep showing stale data.
    Ok(ViewMaterializationSchedule {
        request_count,
        can_retarget_cached_scene: !desired_ready,
    })
}

async fn schedule_view_materialization_transforms(
    mark: &dyn CompiledMark,
    view_scope: &CompiledViewScope,
    base_prepared: &PreparedLogicalMarkData,
    request: &MarkDataRequest<'_>,
    view_eval_ctx: &EvaluationContext,
) -> Result<(usize, bool), AvengerChartError> {
    let ctx = request.eval_ctx.session_context.as_ref();
    if view_scope.data.transforms().is_empty() {
        return Ok((0, false));
    }

    // Mirror full view evaluation: children of a viewed group start from the
    // group's shared view-local output so this schedule-only path resolves
    // identical stage expressions and computes identical materialization
    // keys.
    let group_view_prepared = match request.group_view.as_ref() {
        Some(group_view) => Some(
            prepare_group_view_data(
                group_view,
                base_prepared,
                request,
                view_eval_ctx,
                ViewMaterializationHandling::PreviewRetargetScheduleOnly,
            )
            .await?,
        ),
        None => None,
    };
    let inherited_derived_scalars = match group_view_prepared.as_ref() {
        Some(shared) => merge_derived_scalars(
            base_prepared.derived_scalars.clone(),
            shared.derived_scalars.clone(),
        )?,
        None => base_prepared.derived_scalars.clone(),
    };

    let dataframe = if let Some(store_data) = view_scope.data.store_data() {
        Some(store_dataframe(
            store_data,
            request.facet_data_scope,
            view_eval_ctx,
        )?)
    } else if let Some(dataframe) = view_scope.data.dataframe_with_context(ctx) {
        Some(dataframe)
    } else if let Some(shared) = group_view_prepared.as_ref() {
        shared.dataframe.clone()
    } else {
        base_prepared.dataframe.clone()
    };

    let outcome = execute_transform_chain(
        dataframe,
        view_scope.data.transforms(),
        ctx,
        request.facet_data_scope,
        mark.state().facet_data_scope,
        TransformChainMode::View {
            eval_ctx: view_eval_ctx,
            view_scope,
            materialization_handling: ViewMaterializationHandling::PreviewRetargetScheduleOnly,
        },
        &inherited_derived_scalars,
    )
    .await?;

    Ok((
        outcome.materialization_request_count,
        outcome.desired_materialization_ready,
    ))
}

/// Apply a scale transformation to a channel expression.
fn evaluate_channel_without_scale(
    channel_value: &ChannelValue,
    ctx: &SessionContext,
) -> Result<Expr, AvengerChartError> {
    match channel_value {
        ChannelValue::Value { expr } | ChannelValue::Scaled { expr, .. } => expr.to_expr(ctx),
        ChannelValue::Conditional {
            conditions,
            otherwise,
            ..
        } => {
            let direct_expr = |value: &ConditionalValue| match value {
                ConditionalValue::Scaled { expr } | ConditionalValue::Value { expr } => {
                    expr.to_expr(ctx)
                }
            };
            let first = conditions.first().ok_or_else(|| {
                AvengerChartError::InternalError(
                    "Conditional channel has no conditions".to_string(),
                )
            })?;
            let mut case_expr = when(first.0.to_expr(ctx)?, direct_expr(&first.1)?);
            for (condition, value) in &conditions[1..] {
                case_expr = case_expr.when(condition.to_expr(ctx)?, direct_expr(value)?);
            }
            Ok(case_expr.otherwise(direct_expr(otherwise)?)?)
        }
    }
}

fn prepare_channel_expr(
    channel_name: &str,
    channel_value: &ChannelValue,
    coord_transform: Option<&dyn avenger_chart_core::CoordinateSystemTransformCore>,
    scales: &HashMap<String, ConfiguredScaleWithSpec>,
    ctx: &SessionContext,
) -> Result<Expr, AvengerChartError> {
    if coord_transform.is_some_and(|transform| !transform.channel_uses_scale(channel_name)) {
        evaluate_channel_without_scale(channel_value, ctx)
    } else {
        apply_channel_scale(channel_name, channel_value, scales, ctx)
    }
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
                    && let Some(rgba) = parse_color_string(s)
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
            let scale = scales.get(&scale_key);
            let range_type = scale.map(|scale| scale.configured().config.range.data_type().clone());

            let apply_to_conditional =
                |cond_val: &ConditionalValue| -> Result<Expr, AvengerChartError> {
                    match cond_val {
                        ConditionalValue::Scaled { expr } => {
                            if let Some(scale) = scale {
                                let scaled = expr.to_expr(ctx).and_then(|e| scale.to_expr(e))?;
                                // Discrete scales use Arrow dictionary arrays internally.
                                // Conditional CASE branches must agree on their physical
                                // output type, so erase that storage optimization at this
                                // boundary and use the configured range's logical type.
                                Ok(cast(
                                    scaled,
                                    range_type
                                        .clone()
                                        .expect("configured scale has a range type"),
                                ))
                            } else {
                                expr.to_expr(ctx)
                            }
                        }
                        ConditionalValue::Value { expr } => {
                            let expr_df = expr.to_expr(ctx)?;
                            match range_type.as_ref() {
                                Some(DataType::List(_)) if needs_color_conversion => {
                                    Ok(convert_color_literal(&expr_df))
                                }
                                Some(range_type) => Ok(cast(expr_df, range_type.clone())),
                                None => Ok(expr_df),
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
            position_boundary,
            ..
        } => {
            let default_scale_name = strip_trailing_numbers(channel_name).to_string();
            let scale_key = scale_name.as_ref().unwrap_or(&default_scale_name);
            let scale = scales.get(scale_key).ok_or_else(|| {
                let available_scales = scales.keys().cloned().collect::<Vec<_>>().join(", ");
                AvengerChartError::InternalError(format!(
                    "Scale '{}' not found for channel '{}' (available scales: [{}])",
                    scale_key, channel_name, available_scales
                ))
            })?;

            let expr_df = expr.to_expr(ctx)?;
            if let Some(boundary) = position_boundary {
                scale.to_expr_with_position_boundary(expr_df, boundary, ctx)
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
    let mark = request.mark;

    let prepared_storage;
    let mut prepared_logical = if let Some(prepared) = request.prepared_logical {
        prepared
    } else {
        prepared_storage = prepare_logical_mark_data(LogicalMarkDataRequest {
            mark,
            plot_data: request.plot_data,
            provided_plot_df: request.provided_plot_df,
            facet_data_scope: request.facet_data_scope,
            prepared_base: request.prepared_base,
            eval_ctx: request.eval_ctx,
        })
        .await?;
        &prepared_storage
    };

    let view_eval_ctx_storage;
    let view_prepared_storage;
    let mut eval_ctx = request.eval_ctx;
    if let Some(view_scope) = mark.state().view.as_ref() {
        view_eval_ctx_storage = eval_ctx_with_view_params(
            request.eval_ctx,
            view_scope,
            request.scales,
            request.plot_width,
            request.plot_height,
        )?;
        view_prepared_storage = prepare_view_logical_mark_data(
            mark,
            view_scope,
            prepared_logical,
            &request,
            &view_eval_ctx_storage,
            ViewMaterializationHandling::RenderAndSchedule,
        )
        .await?;
        eval_ctx = &view_eval_ctx_storage;
        prepared_logical = &view_prepared_storage;
    }

    let params = eval_ctx.params();
    let channels = &prepared_logical.channels;
    let df_ref = prepared_logical.dataframe.clone();
    validate_mark_detail_fields(mark, df_ref.as_ref())?;

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
    let scale_error_context = || {
        let facet_path = request
            .facet_data_scope
            .as_ref()
            .map(|scope| format!("{:?}", scope.full_path))
            .unwrap_or_else(|| "None".to_string());
        let available_scales = request
            .scales
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        (facet_path, available_scales)
    };
    let mut prepared_channel_names = HashSet::new();
    for channel_desc in &supported_channels {
        if let Some(channel_value) = channels.get(channel_desc.name) {
            prepared_channel_names.insert(channel_desc.name.to_string());
            let scaled_expr = prepare_channel_expr(
                channel_desc.name,
                channel_value,
                request.coord_transform,
                request.scales,
                ctx,
            )
            .map_err(|err| {
                let (facet_path, available_scales) = scale_error_context();
                AvengerChartError::InternalError(format!(
                    "failed to scale mark '{}' channel '{}' at facet path {} with scales [{}]: {}",
                    mark.mark_type(),
                    channel_desc.name,
                    facet_path,
                    available_scales,
                    err
                ))
            })?;
            if channel_desc.allow_column_ref && scaled_expr.any_column_refs() {
                array_channels.push((channel_desc.name.to_string(), scaled_expr));
                has_array_data = true;
            } else {
                scalar_channels.push((channel_desc.name.to_string(), scaled_expr));
            }
        }
    }
    if let Some(coord_transform) = request.coord_transform {
        for (channel_name, channel_value) in channels.iter() {
            if prepared_channel_names.contains(channel_name) {
                continue;
            }
            let base_channel = strip_trailing_numbers(channel_name);
            if !coord_transform.is_position_scale_channel(base_channel) {
                continue;
            }
            prepared_channel_names.insert(channel_name.to_string());
            let scaled_expr =
                prepare_channel_expr(
                    channel_name,
                    channel_value,
                    request.coord_transform,
                    request.scales,
                    ctx,
                )
                .map_err(
                    |err| {
                        let (facet_path, available_scales) = scale_error_context();
                        AvengerChartError::InternalError(format!(
                            "failed to scale mark '{}' dynamic position channel '{}' at facet path {} with scales [{}]: {}",
                            mark.mark_type(),
                            channel_name,
                            facet_path,
                            available_scales,
                            err
                        ))
                    },
                )?;
            if scaled_expr.any_column_refs() {
                array_channels.push((channel_name.to_string(), scaled_expr));
                has_array_data = true;
            } else {
                scalar_channels.push((channel_name.to_string(), scaled_expr));
            }
        }
    }
    if mark.details_partition_continuous_geometry()
        && let Some(details) = mark.state().details.as_deref()
    {
        for (index, field) in details.iter().enumerate() {
            array_channels.push((detail_array_column_name(index), col(field)));
            has_array_data = true;
        }
    }

    let available_fields = df
        .schema()
        .fields()
        .iter()
        .map(|field| field.name().clone())
        .collect::<HashSet<_>>();
    let event_datum_columns = request
        .eval_ctx
        .event_datum_fields
        .keys()
        .filter(|field| available_fields.contains(*field))
        .enumerate()
        .map(|(index, field)| {
            (
                field.clone(),
                format!("__avenger_event_datum_column_{index}"),
            )
        })
        .collect::<Vec<_>>();

    let (data_batch, event_datum_batch) = if mark.wants_full_data_batch() || has_array_data {
        let datafusion_params = params_to_datafusion(params);
        let mut data_select_exprs = if mark.wants_full_data_batch() {
            record_mark_data_full_collect(&request.evaluation_metrics);
            if array_channels.is_empty() {
                df.schema()
                    .fields()
                    .iter()
                    .map(|field| exact_schema_col(field.name().clone()))
                    .collect::<Vec<_>>()
            } else {
                full_data_select_exprs(df.as_ref(), &array_channels)?
            }
        } else {
            record_mark_data_array_collect(&request.evaluation_metrics);
            array_channels
                .iter()
                .map(|(name, expr)| expr.clone().alias(name))
                .collect::<Vec<_>>()
        };
        let data_column_count = data_select_exprs.len();
        // A rendered instance index addresses both batches. Keep mark channels
        // and event datum fields in one DataFusion collection so unordered
        // plans (notably aggregates) cannot assign different row orders to
        // separate executions of the same logical plan.
        data_select_exprs.extend(
            event_datum_columns
                .iter()
                .map(|(field, alias)| exact_schema_col(field).alias(alias)),
        );
        let selected = (*df).clone().select(data_select_exprs)?;
        let selected_schema = Arc::new(selected.schema().as_arrow().clone());
        let batch = if let Some(param_values) = datafusion_params {
            selected.with_param_values(param_values)?.collect().await?
        } else {
            selected.collect().await?
        };
        let combined_batch = if batch.is_empty() {
            RecordBatch::new_empty(selected_schema)
        } else {
            let schema = batch[0].schema();
            concat_batches(&schema, &batch)?
        };
        let (data_batch, event_datum_batch) = split_mark_and_event_datum_batch(
            combined_batch,
            data_column_count,
            &event_datum_columns,
        )?;
        (Some(data_batch), event_datum_batch)
    } else {
        let event_datum_batch = if event_datum_columns.is_empty() {
            None
        } else {
            let select_exprs = event_datum_columns
                .iter()
                .map(|(field, _)| col(field).alias(field))
                .collect::<Vec<_>>();
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
        };
        (None, event_datum_batch)
    };

    let mut scalar_select_exprs = vec![];
    for (name, expr) in &scalar_channels {
        scalar_select_exprs.push(expr.clone().alias(name));
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

fn split_mark_and_event_datum_batch(
    combined: RecordBatch,
    data_column_count: usize,
    event_datum_columns: &[(String, String)],
) -> Result<(RecordBatch, Option<RecordBatch>), AvengerChartError> {
    let combined_schema = combined.schema();
    let data_schema = Arc::new(Schema::new(
        combined_schema
            .fields()
            .iter()
            .take(data_column_count)
            .cloned()
            .collect::<Vec<_>>(),
    ));
    let data_batch = RecordBatch::try_new(
        data_schema,
        combined
            .columns()
            .iter()
            .take(data_column_count)
            .cloned()
            .collect(),
    )?;

    if event_datum_columns.is_empty() || combined.num_rows() == 0 {
        return Ok((data_batch, None));
    }

    let mut event_fields = Vec::with_capacity(event_datum_columns.len());
    let mut event_columns = Vec::with_capacity(event_datum_columns.len());
    for (datum_field, internal_alias) in event_datum_columns {
        let index = combined_schema.index_of(internal_alias)?;
        event_fields.push(combined_schema.field(index).clone().with_name(datum_field));
        event_columns.push(combined.column(index).clone());
    }
    let event_datum_batch =
        RecordBatch::try_new(Arc::new(Schema::new(event_fields)), event_columns)?;
    Ok((data_batch, Some(event_datum_batch)))
}

fn full_data_select_exprs(
    df: &DataFrame,
    array_channels: &[(String, Expr)],
) -> Result<Vec<Expr>, AvengerChartError> {
    let array_channel_names = array_channels
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<HashSet<_>>();
    let mut select_exprs = df
        .schema()
        .fields()
        .iter()
        .filter_map(|field| {
            let name = field.name();
            (!array_channel_names.contains(name.as_str())).then(|| exact_schema_col(name.clone()))
        })
        .collect::<Vec<_>>();
    select_exprs.extend(
        array_channels
            .iter()
            .map(|(name, expr)| expr.clone().alias(name)),
    );
    Ok(select_exprs)
}

fn validate_mark_detail_fields(
    mark: &dyn CompiledMark,
    dataframe: Option<&DataFrame>,
) -> Result<(), AvengerChartError> {
    let Some(details) = mark.state().details.as_deref() else {
        return Ok(());
    };
    if details.is_empty() {
        return Ok(());
    }

    let mut seen = HashSet::new();
    for field in details {
        if field.is_empty() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "{} mark details must be non-empty field names",
                mark.mark_type()
            )));
        }
        if !seen.insert(field) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "{} mark details contain duplicate field '{field}'",
                mark.mark_type()
            )));
        }
    }

    let Some(dataframe) = dataframe else {
        return Err(AvengerChartError::InvalidArgument(format!(
            "{} mark details require data fields, but this mark has no prepared dataframe",
            mark.mark_type()
        )));
    };
    let available = dataframe
        .schema()
        .fields()
        .iter()
        .map(|field| field.name().clone())
        .collect::<HashSet<_>>();
    let missing = details
        .iter()
        .filter(|field| !available.contains(*field))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        let mut available = available.into_iter().collect::<Vec<_>>();
        available.sort();
        return Err(AvengerChartError::InvalidArgument(format!(
            "{} mark details requested field(s) {} but the prepared mark data exposes [{}]",
            mark.mark_type(),
            missing.join(", "),
            available.join(", ")
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use async_trait::async_trait;
    use avenger_chart_core::nested;
    use avenger_chart_scales::NestedBand;
    use avenger_chart_transforms::{
        Aggregate, Bin, Calculate, Filter, Fold, Impute, JoinAggregate, Select, Window,
    };
    use avenger_scales::scales::nested_band::NestedBandScale;
    use avenger_scales::scales::{ConfiguredScale, ScaleConfig};
    use datafusion::{
        arrow::{
            array::{Array, ArrayRef, BooleanArray, Float64Array, StringArray, StructArray},
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
    use serde::{Deserialize, Serialize};

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
        parallel::{Parallel, ParallelLine, generated_dimension_channel},
        plot::{
            compiled::materialization::MaterializationCache,
            compiled::session::{
                ScopedSelectionStore, ScopedStoreAssignment, ScopedStoreState, SelectionAssignment,
                SelectionStateUpdate, StoreStateUpdate,
            },
        },
        scales::{Linear, Scale, ScaleRangeBinding, ScaleSpec},
        serialization::{LogicalExprNodeExt, LogicalPlanNodeExt},
        theme::Theme,
        zerod::ZeroDCoord,
    };
    use avenger_chart_core::{
        CoordinateSystem, CoordinationScope, DataTransformResult, Param,
        ResolvedSelectionClauseScope, STORE_NAME_COLUMN, STORE_OWNER_KEY_COLUMN,
        STORE_REVISION_COLUMN, Selection, SelectionEqualityDimensionValue, Store, StoreData,
        StoreRowValue, View, detail_array_column_name,
    };
    use avenger_chart_marks::{Area, Rect};

    fn eval_context(session: Arc<SessionContext>) -> EvaluationContext {
        EvaluationContext::new(
            Arc::new(Theme::light()),
            session,
            IndexMap::new(),
            Arc::new(EvaluatedFacetTree::empty()),
        )
    }

    fn equality_clause(
        id: &str,
        field: &str,
        value: &str,
    ) -> Result<SelectionClause, AvengerChartError> {
        Ok(SelectionClause {
            id: id.to_string(),
            scope: ResolvedSelectionClauseScope {
                sharing: CoordinationScope::Shared,
                owner_path: Vec::new(),
            },
            predicate: SelectionPredicateSpec::Equality {
                dimensions: vec![SelectionEqualityDimensionValue {
                    id: field.to_string(),
                    field_expr: LogicalExprNode::from_expr(col(field))?,
                    value: ScalarValue::Utf8(Some(value.to_string())),
                }],
            },
            facet_context: Vec::new(),
        })
    }

    async fn equality_membership_values(
        selection: &Selection,
        clauses: Vec<SelectionClause>,
    ) -> Result<Vec<bool>, AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let mut spec = selection.compile()?;
        spec.runtime_id = avenger_chart_core::CompiledIdentityAllocator::new("membership-test")
            .allocate_selection();
        let specs = IndexMap::from([(spec.runtime_id.clone(), spec)]);
        let mut store = ScopedSelectionStore::new(specs);
        store.apply_selection_patch([SelectionAssignment {
            selection_id: selection.id.clone(),
            update: SelectionStateUpdate::ReplaceAllClauses { clauses },
        }])?;
        let eval_ctx = eval_context(session.clone()).with_scoped_selection_store(Arc::new(store));
        let batch = RecordBatch::try_from_iter(vec![(
            "__value",
            Arc::new(StringArray::from(vec!["A", "B"])) as ArrayRef,
        )])?;
        let df = session.read_batch(batch)?;
        let membership = expand_selection_predicates(
            selection.contains_equality_value(col("category"), col("__value")),
            &eval_ctx,
            None,
        )?;
        let batches = df
            .select(vec![membership.alias("checked")])?
            .collect()
            .await?;
        let values = batches[0]
            .column_by_name("checked")
            .expect("checked result")
            .as_any()
            .downcast_ref::<BooleanArray>()
            .expect("boolean checked result");
        Ok((0..values.len()).map(|index| values.value(index)).collect())
    }

    #[tokio::test]
    async fn equality_membership_is_false_for_empty_select_all_selection()
    -> Result<(), AvengerChartError> {
        let selection = Selection::new("picked").empty_selects_all();
        assert_eq!(
            equality_membership_values(&selection, Vec::new()).await?,
            vec![false, false]
        );
        Ok(())
    }

    #[tokio::test]
    async fn equality_membership_matches_typed_value_and_exact_field_only()
    -> Result<(), AvengerChartError> {
        let selection = Selection::new("picked").empty_selects_all();
        let mut compound = equality_clause("compound", "category", "B")?;
        let SelectionPredicateSpec::Equality { dimensions } = &mut compound.predicate else {
            unreachable!("test helper creates equality predicate")
        };
        dimensions.push(SelectionEqualityDimensionValue {
            id: "region".to_string(),
            field_expr: LogicalExprNode::from_expr(col("region"))?,
            value: ScalarValue::Utf8(Some("north".to_string())),
        });
        assert_eq!(
            equality_membership_values(
                &selection,
                vec![
                    equality_clause("external-id", "category", "A")?,
                    equality_clause("same-value-other-field", "region", "B")?,
                    compound,
                ],
            )
            .await?,
            vec![true, false]
        );
        Ok(())
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

    fn materialized_batch(xs: Vec<f64>, ys: Vec<f64>) -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new("mx", DataType::Float64, false),
            Field::new("my", DataType::Float64, false),
        ]));
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(xs)) as ArrayRef,
                Arc::new(Float64Array::from(ys)) as ArrayRef,
            ],
        )
        .expect("materialized batch")
    }

    fn empty_materialized_dataframe(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
        ctx.read_batch(RecordBatch::new_empty(
            materialized_batch(Vec::new(), Vec::new()).schema(),
        ))
        .expect("empty materialized dataframe")
    }

    #[derive(Clone)]
    struct FakeMaterializedTransform {
        key: String,
        identity: String,
    }

    impl FakeMaterializedTransform {
        fn new(key: impl Into<String>, identity: impl Into<String>) -> Self {
            Self {
                key: key.into(),
                identity: identity.into(),
            }
        }
    }

    impl avenger_chart_core::DataTransform for FakeMaterializedTransform {
        type Output = ();

        fn into_compiled_and_output(
            self,
            _ctx: avenger_chart_core::DataTransformCompileContext,
        ) -> Result<
            (
                Box<dyn avenger_chart_core::CompiledDataTransform>,
                Self::Output,
            ),
            AvengerChartError,
        > {
            Ok((
                Box::new(CompiledFakeMaterializedTransform {
                    key: self.key,
                    identity: self.identity,
                }),
                (),
            ))
        }
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct CompiledFakeMaterializedTransform {
        key: String,
        identity: String,
    }

    #[typetag::serde(name = "test_fake_materialized")]
    #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
    #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
    impl avenger_chart_core::CompiledDataTransform for CompiledFakeMaterializedTransform {
        fn clone_box(&self) -> Box<dyn avenger_chart_core::CompiledDataTransform> {
            Box::new(self.clone())
        }

        async fn apply(
            &self,
            dataframe: datafusion::dataframe::DataFrame,
            _ctx: &DataTransformExecutionContext<'_>,
        ) -> Result<DataTransformResult, AvengerChartError> {
            Ok(DataTransformResult::dataframe(dataframe))
        }

        fn view_materialization_request(
            &self,
            _dataframe: &datafusion::dataframe::DataFrame,
            ctx: &ViewMaterializationContext<'_>,
        ) -> Result<Option<ViewMaterializationRequest>, AvengerChartError> {
            Ok(Some(ViewMaterializationRequest {
                request: avenger_chart_core::MaterializationRequest::new(
                    self.key.clone(),
                    "fake-materialized",
                    avenger_chart_core::MaterializationOutputKind::RecordBatch,
                )
                .identity(self.identity.clone())
                .policy(ctx.policy.clone()),
                empty_dataframe: Some(empty_materialized_dataframe(ctx.session_context)),
            }))
        }
    }

    fn nested_category_dataframe(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
        let schema = Arc::new(Schema::new(vec![
            Field::new("group", DataType::Utf8, false),
            Field::new("member", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["A", "A", "B"])) as ArrayRef,
                Arc::new(StringArray::from(vec!["one", "two", "one"])) as ArrayRef,
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0])) as ArrayRef,
            ],
        )
        .expect("test nested batch");
        ctx.read_batch(batch).expect("test nested dataframe")
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
        let leaf =
            crate::plot::Plot::<Cartesian>::new().mark(Symbol::new().x(col("x")).y(col("y")));
        let col_plot = crate::plot::Plot::<FacetColumn>::new()
            .mark(Subplot::new(leaf).col_with(col("facet_col"), |c| c));
        let row_plot = crate::plot::Chart::<FacetRow>::new()
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
        linear_scale_with_domain_range((0.0, 10.0), (0.0, 100.0))
    }

    fn linear_scale_with_domain_range(
        domain: (f32, f32),
        range: (f32, f32),
    ) -> ConfiguredScaleWithSpec {
        let scale = Scale::<Linear>::new().into_auto();
        let configured = ConfiguredScale {
            scale_impl: Linear.create_impl(),
            config: ScaleConfig::empty(),
        }
        .with_domain_interval(domain)
        .with_range_interval(range);
        ConfiguredScaleWithSpec::with_range_binding(
            scale,
            configured,
            ScaleRangeBinding::Independent,
        )
    }

    /// View-domain params must round-trip coordinate-owned f64 domains at
    /// full precision: a ~20 m-span Web-Mercator-meter domain (a deep-zoom
    /// Geo viewport) quantizes by ~0.5 m through the f32 domain accessor.
    #[test]
    fn view_domain_params_preserve_f64_precision() {
        let x_domain = (-8_240_553.123_456_7_f64, -8_240_533.123_456_7_f64);
        let y_domain = (4_970_121.987_654_3_f64, 4_970_141.987_654_3_f64);
        let scale_f64 = |domain: (f64, f64), range: (f32, f32)| {
            let scale = Scale::<Linear>::new().into_auto();
            let configured = ConfiguredScale {
                scale_impl: Linear.create_impl(),
                config: ScaleConfig::empty(),
            }
            .with_domain_interval_f64(domain)
            .with_range_interval(range);
            ConfiguredScaleWithSpec::with_range_binding(
                scale,
                configured,
                ScaleRangeBinding::Independent,
            )
        };
        let scales = HashMap::from([
            ("x".to_string(), scale_f64(x_domain, (0.0, 100.0))),
            ("y".to_string(), scale_f64(y_domain, (100.0, 0.0))),
        ]);
        let (spec, view_ref) = avenger_chart_core::ViewSpec::into_compiled_and_ref(
            View::cartesian()
                .id("v")
                .x_domain(col("x"))
                .y_domain(col("y")),
        )
        .expect("compile view spec");
        let params =
            resolved_view_params(&spec, &scales, 100.0, 100.0).expect("resolve view params");

        let get = |name: String| -> f64 {
            match params.get(&name) {
                Some(ScalarValue::Float64(Some(value))) => *value,
                other => panic!("param {name}: unexpected {other:?}"),
            }
        };
        let tolerance = 1e-3; // meters
        assert!((get(view_ref.x().param_name("domain_start")) - x_domain.0).abs() < tolerance);
        assert!((get(view_ref.x().param_name("domain_end")) - x_domain.1).abs() < tolerance);
        assert!((get(view_ref.y().param_name("domain_start")) - y_domain.0).abs() < tolerance);
        assert!((get(view_ref.y().param_name("domain_end")) - y_domain.1).abs() < tolerance);
        // The old f32 hop was meter-scale wrong at this magnitude — the
        // tolerance above genuinely detects a regression.
        assert!((f64::from(x_domain.0 as f32) - x_domain.0).abs() > tolerance);
    }

    async fn prepare_fake_materialized_view_mark(
        cache: Arc<Mutex<MaterializationCache>>,
        key: &str,
        identity: &str,
        preview_cached: bool,
        debounce: Option<Duration>,
    ) -> Result<(PreparedMarkData, EvaluationContext), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let df = xy_dataframe(&session);
        let plot_node = plot_data_node(&df)?;
        let mut view = View::cartesian()
            .id("materialized")
            .x_domain(col("x"))
            .y_domain(col("y"))
            .preview_cached(preview_cached);
        if let Some(debounce) = debounce {
            view = view.debounce(debounce);
        }
        let mark = Symbol::<Cartesian>::new().view(view, |mark, _view| {
            mark.transform_no_output(FakeMaterializedTransform::new(key, identity), |mark| {
                mark.x(ChannelValue::from(col("mx")).no_scale())
                    .y(ChannelValue::from(col("my")).no_scale())
            })
        });
        let compiled_mark = mark.compile_untransformed(&session).await?;
        let eval_ctx = eval_context(session.clone()).with_materialization_cache(cache);
        let scales = HashMap::from([
            (
                "x".to_string(),
                linear_scale_with_domain_range((0.0, 10.0), (0.0, 100.0)),
            ),
            (
                "y".to_string(),
                linear_scale_with_domain_range((0.0, 10.0), (100.0, 0.0)),
            ),
        ]);

        let prepared = prepare_mark_data(MarkDataRequest {
            mark: compiled_mark.as_ref(),
            coord_transform: None,
            plot_data: Some(&plot_node),
            provided_plot_df: None,
            facet_data_scope: None,
            prepared_logical: None,
            prepared_base: None,
            eval_ctx: &eval_ctx,
            evaluation_metrics: None,
            scales: &scales,
            plot_width: 100.0,
            plot_height: 100.0,
            group_view: None,
        })
        .await?
        .expect("prepared data");

        Ok((prepared, eval_ctx))
    }

    fn nested_domain_array(groups: Vec<&str>, members: Vec<&str>) -> ArrayRef {
        Arc::new(StructArray::from(vec![
            (
                Arc::new(Field::new("group", DataType::Utf8, false)),
                Arc::new(StringArray::from(groups)) as ArrayRef,
            ),
            (
                Arc::new(Field::new("member", DataType::Utf8, false)),
                Arc::new(StringArray::from(members)) as ArrayRef,
            ),
        ])) as ArrayRef
    }

    fn nested_band_scale() -> ConfiguredScaleWithSpec {
        let scale = Scale::<NestedBand>::new().into_auto();
        let configured = NestedBandScale::configured(
            nested_domain_array(vec!["A", "A", "B"], vec!["one", "two", "one"]),
            (0.0, 300.0),
        );
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

    fn values_as_string(batch: &RecordBatch, column_name: &str) -> Vec<String> {
        let column = batch
            .column_by_name(column_name)
            .unwrap_or_else(|| panic!("missing column {column_name}"));
        let values = column
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("string values");
        (0..values.len())
            .map(|idx| values.value(idx).to_string())
            .collect()
    }

    #[tokio::test]
    async fn full_data_select_exprs_preserves_raw_columns_and_channel_aliases()
    -> Result<(), AvengerChartError> {
        let session = SessionContext::new();
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("path", DataType::Utf8, false),
                Field::new("MixedCase", DataType::Utf8, false),
                Field::new("fill", DataType::Utf8, false),
                Field::new("category", DataType::Utf8, false),
                Field::new("value", DataType::Float64, false),
            ])),
            vec![
                Arc::new(StringArray::from(vec!["root/A", "root/B"])),
                Arc::new(StringArray::from(vec!["exact-A", "exact-B"])),
                Arc::new(StringArray::from(vec!["raw-red", "raw-blue"])),
                Arc::new(StringArray::from(vec!["prepared-red", "prepared-blue"])),
                Arc::new(Float64Array::from(vec![1.0, 2.0])),
            ],
        )?;
        let df = session.read_batch(batch)?;
        let selected = df
            .clone()
            .select(full_data_select_exprs(
                &df,
                &[("fill".to_string(), col("category"))],
            )?)?
            .collect()
            .await?;
        let schema = selected[0].schema();
        let field_names = schema
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            field_names,
            vec!["path", "MixedCase", "category", "value", "fill"]
        );
        assert_eq!(
            values_as_string(&selected[0], "fill"),
            vec!["prepared-red", "prepared-blue"]
        );
        assert_eq!(
            values_as_string(&selected[0], "path"),
            vec!["root/A", "root/B"]
        );
        assert_eq!(
            values_as_string(&selected[0], "MixedCase"),
            vec!["exact-A", "exact-B"]
        );
        Ok(())
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
            prepared_base: None,
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

    fn brush_store_state(
        sharing: CoordinationScope,
    ) -> Result<ScopedStoreState, AvengerChartError> {
        let mut spec = Store::empty("brush_boxes")
            .field("id", DataType::Utf8, false)
            .field("x_min", DataType::Float64, false)
            .field("x_max", DataType::Float64, false)
            .primary_key(["id"])
            .sharing(sharing)
            .compile()?;
        spec.runtime_id =
            avenger_chart_core::CompiledIdentityAllocator::new("brush-store-test").allocate_store();
        let specs = IndexMap::from([(spec.runtime_id.clone(), spec)]);
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
            coord_transform: None,
            plot_data: Some(&plot_node),
            provided_plot_df: None,
            facet_data_scope: None,
            prepared_logical: None,
            prepared_base: None,
            eval_ctx: &eval_ctx,
            evaluation_metrics: None,
            scales: &scales,
            plot_width: 100.0,
            plot_height: 100.0,
            group_view: None,
        })
        .await?
        .expect("prepared data");

        let data_batch = prepared.data_batch.expect("array data");
        assert_eq!(values_as_f64(&data_batch, "x"), vec![0.0, 50.0, 100.0]);
        assert_eq!(values_as_f64(&data_batch, "y"), vec![100.0, 50.0, 0.0]);
        Ok(())
    }

    #[tokio::test]
    async fn prepare_mark_data_resolves_view_params_from_configured_scales()
    -> Result<(), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let mark = Rect::<Cartesian>::new().unit_data().view(
            View::cartesian()
                .id("zoom")
                .x_domain(col("x"))
                .y_domain(col("y")),
            |mark, view| {
                mark.x(view.x().domain_start())
                    .x2(view.x().domain_end())
                    .y(view.y().domain_start())
                    .y2(view.y().domain_end())
            },
        );
        let compiled_mark = mark.compile_untransformed(&session).await?;
        let eval_ctx = eval_context(session);
        let scales = HashMap::from([
            (
                "x".to_string(),
                linear_scale_with_domain_range((2.0, 6.0), (0.0, 100.0)),
            ),
            (
                "y".to_string(),
                linear_scale_with_domain_range((20.0, 40.0), (100.0, 0.0)),
            ),
        ]);

        let prepared = prepare_mark_data(MarkDataRequest {
            mark: compiled_mark.as_ref(),
            coord_transform: None,
            plot_data: None,
            provided_plot_df: None,
            facet_data_scope: None,
            prepared_logical: None,
            prepared_base: None,
            eval_ctx: &eval_ctx,
            evaluation_metrics: None,
            scales: &scales,
            plot_width: 321.0,
            plot_height: 123.0,
            group_view: None,
        })
        .await?
        .expect("prepared data");

        assert!(prepared.data_batch.is_none());
        assert_eq!(values_as_f64(&prepared.scalar_batch, "x"), vec![0.0]);
        assert_eq!(values_as_f64(&prepared.scalar_batch, "x2"), vec![100.0]);
        assert_eq!(values_as_f64(&prepared.scalar_batch, "y"), vec![100.0]);
        assert_eq!(values_as_f64(&prepared.scalar_batch, "y2"), vec![0.0]);
        Ok(())
    }

    #[tokio::test]
    async fn prepare_mark_data_applies_view_local_transforms_to_inherited_rows()
    -> Result<(), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let df = xy_dataframe(&session);
        let plot_node = plot_data_node(&df)?;
        let mark = Symbol::<Cartesian>::new().view(
            View::cartesian()
                .id("viewport")
                .x_domain(col("x"))
                .y_domain(col("y")),
            |mark, view| {
                mark.transform_no_output(
                    Calculate::new().expr("x_from_view", col("x") + view.x().domain_start()),
                    |mark| {
                        mark.x(ChannelValue::from(col("x_from_view")).no_scale())
                            .y(ChannelValue::from(col("y")).no_scale())
                    },
                )
            },
        );
        let compiled_mark = mark.compile_untransformed(&session).await?;
        let eval_ctx = eval_context(session);
        let scales = HashMap::from([
            (
                "x".to_string(),
                linear_scale_with_domain_range((2.0, 6.0), (0.0, 100.0)),
            ),
            ("y".to_string(), linear_scale()),
        ]);

        let prepared = prepare_mark_data(MarkDataRequest {
            mark: compiled_mark.as_ref(),
            coord_transform: None,
            plot_data: Some(&plot_node),
            provided_plot_df: None,
            facet_data_scope: None,
            prepared_logical: None,
            prepared_base: None,
            eval_ctx: &eval_ctx,
            evaluation_metrics: None,
            scales: &scales,
            plot_width: 100.0,
            plot_height: 100.0,
            group_view: None,
        })
        .await?
        .expect("prepared data");

        let data_batch = prepared.data_batch.expect("array data");
        assert_eq!(values_as_f64(&data_batch, "x"), vec![2.0, 7.0, 12.0]);
        assert_eq!(values_as_f64(&data_batch, "y"), vec![10.0, 5.0, 0.0]);
        Ok(())
    }

    #[tokio::test]
    async fn prepare_mark_data_uses_ready_view_materialization() -> Result<(), AvengerChartError> {
        let cache = Arc::new(Mutex::new(MaterializationCache::default()));
        let request = avenger_chart_core::MaterializationRequest::new(
            "desired",
            "fake-materialized",
            avenger_chart_core::MaterializationOutputKind::RecordBatch,
        )
        .identity("scope");
        cache.lock().expect("cache lock").mark_ready(
            &request,
            avenger_chart_core::MaterializationResult::RecordBatch(materialized_batch(
                vec![1.0, 2.0],
                vec![3.0, 4.0],
            )),
        );

        let (prepared, eval_ctx) =
            prepare_fake_materialized_view_mark(cache, "desired", "scope", true, None).await?;
        let data_batch = prepared.data_batch.expect("array data");
        assert_eq!(values_as_f64(&data_batch, "x"), vec![1.0, 2.0]);
        assert_eq!(values_as_f64(&data_batch, "y"), vec![3.0, 4.0]);
        assert_eq!(eval_ctx.materialization_requests_snapshot().len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn prepare_mark_data_emits_view_materialization_request_when_missing()
    -> Result<(), AvengerChartError> {
        let cache = Arc::new(Mutex::new(MaterializationCache::default()));

        let (prepared, eval_ctx) =
            prepare_fake_materialized_view_mark(cache.clone(), "missing", "scope", true, None)
                .await?;
        let data_batch = prepared.data_batch.expect("array data");
        assert!(values_as_f64(&data_batch, "x").is_empty());
        assert!(values_as_f64(&data_batch, "y").is_empty());
        assert_eq!(eval_ctx.materialization_requests_snapshot().len(), 1);
        assert_eq!(
            cache
                .lock()
                .expect("cache lock")
                .status(&avenger_chart_core::MaterializationKey::new("missing")),
            MaterializationStatus::Queued
        );
        Ok(())
    }

    #[tokio::test]
    async fn prepare_mark_data_uses_stale_view_materialization_fallback()
    -> Result<(), AvengerChartError> {
        let cache = Arc::new(Mutex::new(MaterializationCache::default()));
        let stale = avenger_chart_core::MaterializationRequest::new(
            "stale",
            "fake-materialized",
            avenger_chart_core::MaterializationOutputKind::RecordBatch,
        )
        .identity("scope");
        cache.lock().expect("cache lock").mark_ready(
            &stale,
            avenger_chart_core::MaterializationResult::RecordBatch(materialized_batch(
                vec![9.0],
                vec![8.0],
            )),
        );

        let (prepared, eval_ctx) =
            prepare_fake_materialized_view_mark(cache.clone(), "desired", "scope", true, None)
                .await?;
        let data_batch = prepared.data_batch.expect("array data");
        assert_eq!(values_as_f64(&data_batch, "x"), vec![9.0]);
        assert_eq!(values_as_f64(&data_batch, "y"), vec![8.0]);
        assert_eq!(eval_ctx.materialization_requests_snapshot().len(), 1);
        assert_eq!(
            cache
                .lock()
                .expect("cache lock")
                .status(&avenger_chart_core::MaterializationKey::new("desired")),
            MaterializationStatus::Queued
        );
        Ok(())
    }

    #[tokio::test]
    async fn prepare_mark_data_hides_stale_view_materialization_when_preview_cache_disabled()
    -> Result<(), AvengerChartError> {
        let cache = Arc::new(Mutex::new(MaterializationCache::default()));
        let stale = avenger_chart_core::MaterializationRequest::new(
            "stale",
            "fake-materialized",
            avenger_chart_core::MaterializationOutputKind::RecordBatch,
        )
        .identity("scope");
        cache.lock().expect("cache lock").mark_ready(
            &stale,
            avenger_chart_core::MaterializationResult::RecordBatch(materialized_batch(
                vec![9.0],
                vec![8.0],
            )),
        );

        let (prepared, eval_ctx) =
            prepare_fake_materialized_view_mark(cache.clone(), "desired", "scope", false, None)
                .await?;
        let data_batch = prepared.data_batch.expect("array data");
        assert!(values_as_f64(&data_batch, "x").is_empty());
        assert!(values_as_f64(&data_batch, "y").is_empty());
        assert_eq!(eval_ctx.materialization_requests_snapshot().len(), 1);
        assert_eq!(
            cache
                .lock()
                .expect("cache lock")
                .status(&avenger_chart_core::MaterializationKey::new("desired")),
            MaterializationStatus::Queued
        );
        Ok(())
    }

    #[tokio::test]
    async fn prepare_mark_data_propagates_view_materialization_debounce_policy()
    -> Result<(), AvengerChartError> {
        let cache = Arc::new(Mutex::new(MaterializationCache::default()));
        let debounce = Duration::from_millis(75);

        let (_prepared, eval_ctx) = prepare_fake_materialized_view_mark(
            cache.clone(),
            "desired",
            "scope",
            true,
            Some(debounce),
        )
        .await?;
        let requests = eval_ctx.materialization_requests_snapshot();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].policy.debounce, Some(debounce));
        assert_eq!(requests[0].policy.throttle, None);

        Ok(())
    }

    #[tokio::test]
    async fn prepare_mark_data_provides_parallel_line_generated_dimensions()
    -> Result<(), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let df = xy_dataframe(&session);
        let plot_node = plot_data_node(&df)?;
        let mark = ParallelLine::new()
            .dimension("x", col("x"))
            .dimension("y", col("y"));
        let compiled_mark = mark.compile_untransformed(&session).await?;
        let coord = Parallel::new()
            .dimension("x")
            .dimension("y")
            .create_transform();
        let eval_ctx = eval_context(session);
        let scales = HashMap::from([
            ("x".to_string(), linear_scale()),
            ("y".to_string(), linear_scale()),
        ]);

        let prepared = prepare_mark_data(MarkDataRequest {
            mark: compiled_mark.as_ref(),
            coord_transform: Some(coord.as_ref()),
            plot_data: Some(&plot_node),
            provided_plot_df: None,
            facet_data_scope: None,
            prepared_logical: None,
            prepared_base: None,
            eval_ctx: &eval_ctx,
            evaluation_metrics: None,
            scales: &scales,
            plot_width: 100.0,
            plot_height: 100.0,
            group_view: None,
        })
        .await?
        .expect("prepared parallel line data");

        let data_batch = prepared.data_batch.expect("parallel line array data");
        let x_generated = generated_dimension_channel("x");
        let y_generated = generated_dimension_channel("y");
        assert_eq!(
            values_as_f64(&data_batch, &x_generated),
            vec![0.0, 50.0, 100.0]
        );
        assert_eq!(
            values_as_f64(&data_batch, &y_generated),
            vec![100.0, 50.0, 0.0]
        );
        assert!(data_batch.column_by_name("x").is_none());
        assert!(data_batch.column_by_name("y").is_none());
        Ok(())
    }

    #[tokio::test]
    async fn prepare_mark_data_evaluates_nested_band_leaf_boundaries()
    -> Result<(), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let df = nested_category_dataframe(&session);
        let plot_node = plot_data_node(&df)?;
        let mark = Rect::new()
            .x_with(nested(["group", "member"]), |x| x.band(0.0))
            .x2_with(col(":x"), |x| x.band(1.0))
            .y_with(lit(0.0), |y| y.no_scale())
            .y2_with(col("value"), |y| y.no_scale());
        let compiled_mark = mark.compile_untransformed(&session).await?;
        let eval_ctx = eval_context(session);
        let scales = HashMap::from([("x".to_string(), nested_band_scale())]);

        let prepared = prepare_mark_data(MarkDataRequest {
            mark: compiled_mark.as_ref(),
            coord_transform: None,
            plot_data: Some(&plot_node),
            provided_plot_df: None,
            facet_data_scope: None,
            prepared_logical: None,
            prepared_base: None,
            eval_ctx: &eval_ctx,
            evaluation_metrics: None,
            scales: &scales,
            plot_width: 300.0,
            plot_height: 100.0,
            group_view: None,
        })
        .await?
        .expect("prepared data");

        let data_batch = prepared.data_batch.expect("array data");
        assert_eq!(values_as_f64(&data_batch, "x"), vec![0.0, 100.0, 200.0]);
        assert_eq!(values_as_f64(&data_batch, "x2"), vec![100.0, 200.0, 300.0]);
        Ok(())
    }

    #[tokio::test]
    async fn prepare_mark_data_evaluates_nested_band_param_boundaries()
    -> Result<(), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let df = nested_category_dataframe(&session);
        let plot_node = plot_data_node(&df)?;
        let band_end = Param::new("band_end", ScalarValue::Float64(Some(1.0)));
        let mark = Rect::new()
            .x_with(nested(["group", "member"]), |x| x.band(0.0))
            .x2_with(col(":x"), |x| x.band(band_end.expr()))
            .y_with(lit(0.0), |y| y.no_scale())
            .y2_with(col("value"), |y| y.no_scale());
        let compiled_mark = mark.compile_untransformed(&session).await?;
        let eval_ctx = eval_context(session.clone()).with_params(IndexMap::from([(
            "band_end".to_string(),
            ScalarValue::Float64(Some(1.0)),
        )]));
        let scales = HashMap::from([("x".to_string(), nested_band_scale())]);

        let prepared = prepare_mark_data(MarkDataRequest {
            mark: compiled_mark.as_ref(),
            coord_transform: None,
            plot_data: Some(&plot_node),
            provided_plot_df: None,
            facet_data_scope: None,
            prepared_logical: None,
            prepared_base: None,
            eval_ctx: &eval_ctx,
            evaluation_metrics: None,
            scales: &scales,
            plot_width: 300.0,
            plot_height: 100.0,
            group_view: None,
        })
        .await?
        .expect("prepared data");

        let data_batch = prepared.data_batch.expect("array data");
        assert_eq!(values_as_f64(&data_batch, "x"), vec![0.0, 100.0, 200.0]);
        assert_eq!(values_as_f64(&data_batch, "x2"), vec![100.0, 200.0, 300.0]);
        Ok(())
    }

    #[tokio::test]
    async fn prepare_mark_data_evaluates_nested_band_level_boundaries()
    -> Result<(), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let df = nested_category_dataframe(&session);
        let plot_node = plot_data_node(&df)?;
        let mark = Rect::new()
            .x_with(nested(["group", "member"]), |x| x.level_band(0, 0.0))
            .x2_with(col(":x"), |x| x.level_band(0, 1.0))
            .y_with(lit(0.0), |y| y.no_scale())
            .y2_with(col("value"), |y| y.no_scale());
        let compiled_mark = mark.compile_untransformed(&session).await?;
        let eval_ctx = eval_context(session);
        let scales = HashMap::from([("x".to_string(), nested_band_scale())]);

        let prepared = prepare_mark_data(MarkDataRequest {
            mark: compiled_mark.as_ref(),
            coord_transform: None,
            plot_data: Some(&plot_node),
            provided_plot_df: None,
            facet_data_scope: None,
            prepared_logical: None,
            prepared_base: None,
            eval_ctx: &eval_ctx,
            evaluation_metrics: None,
            scales: &scales,
            plot_width: 300.0,
            plot_height: 100.0,
            group_view: None,
        })
        .await?
        .expect("prepared data");

        let data_batch = prepared.data_batch.expect("array data");
        assert_eq!(values_as_f64(&data_batch, "x"), vec![0.0, 0.0, 200.0]);
        assert_eq!(values_as_f64(&data_batch, "x2"), vec![200.0, 200.0, 300.0]);
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
            coord_transform: None,
            plot_data: Some(&plot_node),
            provided_plot_df: None,
            facet_data_scope: None,
            prepared_logical: None,
            prepared_base: None,
            eval_ctx: &eval_ctx,
            evaluation_metrics: None,
            scales: &scales,
            plot_width: 100.0,
            plot_height: 100.0,
            group_view: None,
        })
        .await?
        .expect("prepared data");

        let data_batch = prepared.data_batch.expect("inherited array data");
        assert_eq!(values_as_f64(&data_batch, "x"), vec![0.0, 5.0, 10.0]);
        assert!(prepared.scalar_batch.column_by_name("y").is_some());
        Ok(())
    }

    #[test]
    fn mark_details_accept_string_arrays() {
        let mark = Area::<Cartesian>::new().details(["group", "series"]);
        assert_eq!(
            mark.state().details.as_deref(),
            Some(["group".to_string(), "series".to_string()].as_slice())
        );
    }

    #[test]
    fn split_event_datums_preserves_rendered_instance_order() -> Result<(), AvengerChartError> {
        let combined = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("x", DataType::Float64, false),
                Field::new("__avenger_event_datum_column_0", DataType::Utf8, false),
            ])),
            vec![
                Arc::new(Float64Array::from(vec![30.0, 10.0, 20.0])),
                Arc::new(StringArray::from(vec!["March", "January", "February"])),
            ],
        )?;

        let (mark_batch, event_batch) = split_mark_and_event_datum_batch(
            combined,
            1,
            &[(
                "month".to_string(),
                "__avenger_event_datum_column_0".to_string(),
            )],
        )?;

        assert_eq!(values_as_f64(&mark_batch, "x"), vec![30.0, 10.0, 20.0]);
        let event_batch = event_batch.expect("nonempty event datum batch");
        let months = event_batch
            .column_by_name("month")
            .expect("month datum field")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("month string array");
        assert_eq!(
            months.iter().flatten().collect::<Vec<_>>(),
            vec!["March", "January", "February"]
        );
        Ok(())
    }

    #[tokio::test]
    async fn mark_details_are_event_datum_fields_without_bindings() -> Result<(), AvengerChartError>
    {
        let session = SessionContext::new();
        let df = nested_category_dataframe(&session);
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .data(df)
            .mark(
                Symbol::new()
                    .x(col("value"))
                    .y(col("value"))
                    .details(["group"]),
            )
            .compile(&session)
            .await?;

        assert_eq!(
            compiled.event_datum_types().get("group"),
            Some(&DataType::Utf8)
        );
        Ok(())
    }

    #[tokio::test]
    async fn path_mark_details_are_projected_into_array_data() -> Result<(), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let df = nested_category_dataframe(&session);
        let plot_node = plot_data_node(&df)?;
        let mark = Area::<Cartesian>::new()
            .with_channel_value("x", value_channel(col("value")))
            .with_channel_value("y", value_channel(col("value")))
            .details(["group", "member"]);
        let compiled_mark = mark.compile_untransformed(&session).await?;
        let eval_ctx = eval_context(session);
        let scales = HashMap::new();

        let prepared = prepare_mark_data(MarkDataRequest {
            mark: compiled_mark.as_ref(),
            coord_transform: None,
            plot_data: Some(&plot_node),
            provided_plot_df: None,
            facet_data_scope: None,
            prepared_logical: None,
            prepared_base: None,
            eval_ctx: &eval_ctx,
            evaluation_metrics: None,
            scales: &scales,
            plot_width: 100.0,
            plot_height: 100.0,
            group_view: None,
        })
        .await?
        .expect("prepared data");

        let data_batch = prepared.data_batch.expect("array data");
        assert!(
            data_batch
                .column_by_name(&detail_array_column_name(0))
                .is_some()
        );
        assert!(
            data_batch
                .column_by_name(&detail_array_column_name(1))
                .is_some()
        );
        Ok(())
    }

    #[tokio::test]
    async fn missing_mark_detail_field_errors_after_transforms() -> Result<(), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let df = nested_category_dataframe(&session);
        let plot_node = plot_data_node(&df)?;
        let mark = Area::<Cartesian>::new()
            .with_channel_value("x", value_channel(col("value")))
            .with_channel_value("y", value_channel(col("value")))
            .details(["missing"]);
        let compiled_mark = mark.compile_untransformed(&session).await?;
        let eval_ctx = eval_context(session);
        let scales = HashMap::new();

        let result = prepare_mark_data(MarkDataRequest {
            mark: compiled_mark.as_ref(),
            coord_transform: None,
            plot_data: Some(&plot_node),
            provided_plot_df: None,
            facet_data_scope: None,
            prepared_logical: None,
            prepared_base: None,
            eval_ctx: &eval_ctx,
            evaluation_metrics: None,
            scales: &scales,
            plot_width: 100.0,
            plot_height: 100.0,
            group_view: None,
        })
        .await;
        let err = match result {
            Ok(_) => panic!("missing detail should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("missing"), "{err}");
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
            coord_transform: None,
            plot_data: None,
            provided_plot_df: None,
            facet_data_scope: None,
            prepared_logical: None,
            prepared_base: None,
            eval_ctx: &eval_ctx,
            evaluation_metrics: None,
            scales: &scales,
            plot_width: 100.0,
            plot_height: 100.0,
            group_view: None,
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
        let mark = Subplot::<HConcat>::new(crate::plot::Plot::<ZeroDCoord>::new());
        let compiled_mark = mark.compile_untransformed(&session).await?;
        let eval_ctx = eval_context(session);
        let scales = HashMap::new();

        let prepared = prepare_mark_data(MarkDataRequest {
            mark: compiled_mark.as_ref(),
            coord_transform: None,
            plot_data: None,
            provided_plot_df: Some(&df),
            facet_data_scope: None,
            prepared_logical: None,
            prepared_base: None,
            eval_ctx: &eval_ctx,
            evaluation_metrics: None,
            scales: &scales,
            plot_width: 100.0,
            plot_height: 100.0,
            group_view: None,
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
        let mut store_state = brush_store_state(CoordinationScope::Shared)?;
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
            prepared_base: None,
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
            prepared_base: None,
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
        let mut store_state = brush_store_state(CoordinationScope::Shared)?;
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
            coord_transform: None,
            plot_data: None,
            provided_plot_df: None,
            facet_data_scope: None,
            prepared_logical: None,
            prepared_base: None,
            eval_ctx: &eval_ctx,
            evaluation_metrics: None,
            scales: &HashMap::new(),
            plot_width: 100.0,
            plot_height: 100.0,
            group_view: None,
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

        let mut free_store_state = brush_store_state(CoordinationScope::Free)?;
        free_store_state.apply_scoped_patch([
            replace_store_rows(Vec::new(), vec![brush_row("root", 100.0, 101.0)]),
            replace_store_rows(north_west_owner.clone(), vec![brush_row("nw", 1.0, 2.0)]),
            replace_store_rows(north_east_owner.clone(), vec![brush_row("ne", 10.0, 11.0)]),
        ])?;
        let free_eval_ctx = eval_context(session.clone())
            .with_facet_tree(Arc::new(facet_tree.clone()))
            .with_scoped_store_state(Arc::new(free_store_state));

        let mut shared_store_state = brush_store_state(CoordinationScope::Shared)?;
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
                    prepared_base: None,
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
                    prepared_base: None,
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
            prepared_base: None,
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
            prepared_base: None,
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
    async fn shared_impute_grouped_by_facet_columns_narrows_predictably()
    -> Result<(), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let df = scoped_facet_dataframe(&session).await;
        let facet_tree = scoped_facet_tree(df.clone(), &session).await?;
        let full_path = vec![
            ScalarValue::Utf8(Some("North".to_string())),
            ScalarValue::Utf8(Some("West".to_string())),
        ];

        let mark = Symbol::<Cartesian>::new().transform_shared(
            Impute::new(col("x"))
                .key(col("y"))
                .group_by([col("facet_row"), col("facet_col")])
                .value(lit(0.0)),
            |mark, imputed| mark.x(imputed.value()).y(col("y")),
        );
        let mut values =
            prepared_x_values_for_facet_mark(mark, session, &df, &facet_tree, &full_path).await?;
        values.sort_by(f64::total_cmp);

        assert_eq!(values, vec![0.0, 0.0, 0.0, 2.0]);
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
            prepared_base: None,
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
            prepared_base: None,
            eval_ctx: &eval_ctx,
        })
        .await?;
        let values = prepared_channel_values(prepared, &session, "x").await?;

        assert_eq!(values, vec![1000.0]);
        Ok(())
    }
}
