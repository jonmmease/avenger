use std::{
    collections::{BTreeSet, HashSet},
    sync::Arc,
};

use avenger_chart_core::{
    AvengerChartError, CompiledDataContext, CompiledMark, CompiledPositionedSubplot,
    CompiledSubplotChildPlot, CompiledSubplotPayload, DataTransformExecutionContext,
    ExecutionShape, LogicalPlanNodeExt, MarkDataMode, TimeContext, apply_compiled_data_transforms,
    derived_scalar_id_from_placeholder,
};
use avenger_datafusion_partial_eval::{BakeReport, BakedSubtree, partial_evaluate_set};
use datafusion::{
    dataframe::DataFrame,
    logical_expr::{Expr, LogicalPlan},
    prelude::SessionContext,
};
use datafusion_common::tree_node::{TreeNode, TreeNodeRecursion};
use datafusion_proto::protobuf::LogicalPlanNode;
use indexmap::IndexMap;
use serde_json::Value;

use crate::bake::{
    BakeContextId, BakePolicy, BakedTableManifestEntry, ContextBakeStatus, EmitForm,
    FixedParamBinding, NotBakedReason, PlotBakeReport,
};

use super::{CompiledPlot, compiled_subplot_payload_child_plot_arc};
use crate::facet::marks::{
    CompiledFacetColumnSubplot, CompiledFacetRowSubplot, CompiledFacetWrapSubplot,
};

#[derive(Clone)]
struct BakeTarget {
    plot_path: Vec<usize>,
    kind: BakeTargetKind,
}

#[derive(Clone)]
enum BakeTargetKind {
    PlotData,
    MarkGroup {
        index: usize,
        /// Emit the residual as the group's base plan while PRESERVING the
        /// group's live transform chain (base-only bake), instead of
        /// replacing the whole chain with its pre-evaluated output.
        keep_transforms: bool,
    },
}

#[derive(Clone)]
struct AssembledContext {
    target: BakeTarget,
    context_id: BakeContextId,
    plan: LogicalPlan,
    base_table_names: Vec<String>,
    /// Session tables the ORIGINAL transform chain reads besides its input.
    /// Relevant whenever the chain remains live after the bake: base-only
    /// emits keep their stages, and pass-through contexts keep the whole
    /// original chain.
    live_stage_tables: Vec<String>,
}

struct Assembly {
    contexts: Vec<AssembledContext>,
    statuses: Vec<ContextBakeStatus>,
    /// Session tables read by transform chains that stay live on NOT-baked
    /// contexts (skipped groups keep their original stages).
    live_stage_tables: Vec<String>,
}

impl CompiledPlot {
    /// Bake this compiled plot into a new compiled plot.
    ///
    /// Baking partially evaluates the plot's DataFusion data pipelines using
    /// `avenger-datafusion-partial-eval`: deterministic, parameter-independent
    /// subtrees are materialized into embedded tables, while residual work that
    /// still depends on live params remains symbolic.
    pub async fn bake(
        &self,
        ctx: &SessionContext,
        policy: &BakePolicy,
    ) -> Result<(CompiledPlot, PlotBakeReport), AvengerChartError> {
        let assembly = assemble(self, ctx).await;
        let partial_policy = policy.to_partial_eval_policy(
            self.runtime_unfoldable_tables(),
            crate::bake::unique_bake_table_prefix(),
        );

        let (residuals, partial_report) = if assembly.contexts.is_empty() {
            (Vec::new(), BakeReport::empty())
        } else {
            partial_evaluate_set(
                assembly
                    .contexts
                    .iter()
                    .map(|context| context.plan.clone())
                    .collect(),
                ctx,
                &partial_policy,
            )
            .await?
        };

        let mut baked = self.clone();
        let mut statuses = assembly.statuses;
        let baked_table_names = partial_report
            .baked
            .iter()
            .map(|baked| baked.table_name.clone())
            .collect::<HashSet<_>>();
        let skip_reasons = partial_report
            .skipped
            .iter()
            .map(|skip| format!("{:?}", skip.reason))
            .collect::<Vec<_>>();
        let mut any_context_baked = false;
        let mut all_baked_self_contained = true;
        // Session tables still read by chains that remain live after the bake
        // (skipped contexts and preserved pass-through chains).
        let mut live_table_refs = assembly.live_stage_tables.clone();
        // Baked tables actually referenced by an EMITTED residual; anything
        // else in the crate report is dead weight for this plot's manifest.
        let mut manifest_names: HashSet<String> = HashSet::new();

        for (assembled, residual) in assembly.contexts.iter().zip(residuals.iter()) {
            match primary_baked_table(assembled, residual, &partial_report.baked) {
                PrimaryMatch::One(primary) => {
                    emit_proto_context(&mut baked, &assembled.target, residual, ctx)?;
                    let keeps_live_stages = matches!(
                        assembled.target.kind,
                        BakeTargetKind::MarkGroup {
                            keep_transforms: true,
                            ..
                        }
                    );
                    let self_contained = residual_self_contained(residual, &baked_table_names)
                        && (!keeps_live_stages || assembled.live_stage_tables.is_empty());
                    if keeps_live_stages {
                        live_table_refs.extend(assembled.live_stage_tables.iter().cloned());
                    }
                    manifest_names.extend(
                        table_names(residual)
                            .into_iter()
                            .filter(|name| baked_table_names.contains(name)),
                    );
                    all_baked_self_contained &= self_contained;
                    any_context_baked = true;
                    statuses.push(ContextBakeStatus::Baked {
                        context_id: assembled.context_id.clone(),
                        emit_form: EmitForm::Proto,
                        primary_table: primary.table_name.clone(),
                        self_contained,
                    });
                }
                PrimaryMatch::None => {
                    live_table_refs.extend(assembled.live_stage_tables.iter().cloned());
                    statuses.push(ContextBakeStatus::NotBaked {
                        context_id: assembled.context_id.clone(),
                        reason: NotBakedReason::BaseNotFolded {
                            skipped: skip_reasons.clone(),
                        },
                    });
                }
                PrimaryMatch::Multiple(table_names) => {
                    live_table_refs.extend(assembled.live_stage_tables.iter().cloned());
                    statuses.push(ContextBakeStatus::NotBaked {
                        context_id: assembled.context_id.clone(),
                        reason: NotBakedReason::MultiplePrimaryTables { table_names },
                    });
                }
            }
        }

        baked.baked_tables = partial_report
            .baked
            .iter()
            .filter(|table| manifest_names.contains(&table.table_name))
            .map(|table| {
                BakedTableManifestEntry::from_batches(
                    table.table_name.clone(),
                    table.schema.clone(),
                    &table.batches,
                    table.rows,
                    table.bytes,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;

        let report = PlotBakeReport {
            as_of: partial_report.as_of,
            source_tables: partial_report.source_tables,
            fixed_params_applied: partial_report
                .fixed_params_applied
                .into_iter()
                .map(|(name, value)| FixedParamBinding {
                    name,
                    value: format!("{value:?}"),
                })
                .collect(),
            unused_fixed_params: partial_report.unused_fixed_params,
            remaining_params: partial_report.remaining_params,
            contexts: statuses,
            self_contained: any_context_baked
                && all_baked_self_contained
                && live_table_refs.is_empty(),
        };
        baked.bake_report = Some(report.clone());
        Ok((baked, report))
    }

    /// Runtime-mutable table names that the partial evaluator must never fold.
    ///
    /// Currently empty: stores and selections never appear as named table
    /// scans in compiled plans. Store-backed contexts materialize per
    /// evaluation from `ScopedStoreState` (and are skipped whole via
    /// [`NotBakedReason::StoreData`]), and selection clauses enter compiled
    /// expressions as placeholders that stay live through a bake. This hook
    /// exists so future revisioned runtime sources that DO surface as named
    /// scans are excluded here rather than via a caller-settable policy field.
    fn runtime_unfoldable_tables(&self) -> HashSet<String> {
        HashSet::new()
    }
}

async fn assemble(compiled: &CompiledPlot, ctx: &SessionContext) -> Assembly {
    let mut assembly = Assembly {
        contexts: Vec::new(),
        statuses: Vec::new(),
        live_stage_tables: Vec::new(),
    };
    assemble_plot_tree(compiled, ctx, Vec::new(), None, false, &mut assembly).await;
    assembly
}

async fn assemble_plot_tree(
    compiled: &CompiledPlot,
    ctx: &SessionContext,
    plot_path: Vec<usize>,
    inherited_plot_plan: Option<LogicalPlan>,
    under_facet: bool,
    assembly: &mut Assembly,
) {
    let explicit_plot_plan = match compiled_plot_plan(compiled, ctx) {
        Ok(plan) => plan,
        Err(reason) => {
            assembly.statuses.push(ContextBakeStatus::NotBaked {
                context_id: plot_data_context_id(&plot_path),
                reason,
            });
            None
        }
    };
    let inherited_or_explicit_plan = explicit_plot_plan
        .clone()
        .or_else(|| inherited_plot_plan.clone());

    match explicit_plot_plan {
        Some(plan) => match assemble_plot_data_plan(plot_path.clone(), plan) {
            Ok(context) => assembly.contexts.push(context),
            Err(reason) => assembly.statuses.push(ContextBakeStatus::NotBaked {
                context_id: plot_data_context_id(&plot_path),
                reason,
            }),
        },
        None => assembly.statuses.push(ContextBakeStatus::NotBaked {
            context_id: plot_data_context_id(&plot_path),
            reason: NotBakedReason::NoData,
        }),
    }

    for (index, group) in compiled.mark_groups.iter().enumerate() {
        let target = BakeTarget {
            plot_path: plot_path.clone(),
            kind: BakeTargetKind::MarkGroup {
                index,
                keep_transforms: false,
            },
        };
        let context_id = mark_group_context_id(&plot_path, index, group.id.clone());
        match assemble_data_context(
            &group.data,
            target,
            context_id.clone(),
            ctx,
            compiled.time_context.clone(),
            group.data_mode,
            inherited_or_explicit_plan.as_ref(),
            under_facet,
        )
        .await
        {
            Ok(Some(context)) => assembly.contexts.push(context),
            // Groups without their own plan or transform chain inherit the
            // parent plot's data and are not separate bake contexts.
            Ok(None) => assembly.statuses.push(ContextBakeStatus::NotBaked {
                context_id,
                reason: NotBakedReason::NoData,
            }),
            Err(reason) => {
                // The skipped context keeps its original transform chain
                // live; any session tables those stages read stay required
                // at evaluation time.
                assembly
                    .live_stage_tables
                    .extend(live_stage_session_tables(group.data.transforms()));
                assembly
                    .statuses
                    .push(ContextBakeStatus::NotBaked { context_id, reason });
            }
        }
    }

    for (mark_index, mark) in compiled.marks.iter().enumerate() {
        let Some(payload) = subplot_payload_for_mark(mark.as_ref()) else {
            continue;
        };
        let child_plot = compiled_subplot_payload_child_plot_arc(payload);
        let child_inherited_plan = payload
            .inherits_parent_data()
            .then(|| inherited_or_explicit_plan.clone())
            .flatten();
        let child_under_facet = under_facet || mark_evaluates_child_per_cell(mark.as_ref());
        let mut child_path = plot_path.clone();
        child_path.push(mark_index);
        Box::pin(assemble_plot_tree(
            child_plot.as_ref(),
            ctx,
            child_path,
            child_inherited_plan,
            child_under_facet,
            assembly,
        ))
        .await;
    }
}

/// Whether a subplot mark evaluates its child plot once per facet cell (as
/// opposed to once, globally). Facet subplots always do; positioned subplots
/// do when they partition parent data.
fn mark_evaluates_child_per_cell(mark: &dyn CompiledMark) -> bool {
    if matches!(mark.mark_type(), "facet_col" | "facet_row" | "facet_wrap") {
        return true;
    }
    mark.as_positioned_subplot()
        .is_some_and(|subplot| subplot.partition_expr().is_some())
}

fn compiled_plot_plan(
    compiled: &CompiledPlot,
    ctx: &SessionContext,
) -> Result<Option<LogicalPlan>, NotBakedReason> {
    compiled
        .data
        .as_ref()
        .map(|node| node.to_logical_plan(ctx).map_err(assembly_error))
        .transpose()
}

fn assemble_plot_data_plan(
    plot_path: Vec<usize>,
    plan: LogicalPlan,
) -> Result<AssembledContext, NotBakedReason> {
    if plan_consumes_derived_scalars(&plan) {
        return Err(NotBakedReason::DerivedScalars);
    }
    let base_table_names = table_names(&plan);
    Ok(AssembledContext {
        target: BakeTarget {
            plot_path: plot_path.clone(),
            kind: BakeTargetKind::PlotData,
        },
        context_id: plot_data_context_id(&plot_path),
        plan,
        base_table_names,
        live_stage_tables: Vec::new(),
    })
}

fn plot_data_context_id(plot_path: &[usize]) -> BakeContextId {
    if plot_path.is_empty() {
        BakeContextId::PlotData
    } else {
        BakeContextId::ChildPlotData {
            subplot_path: plot_path.to_vec(),
        }
    }
}

fn mark_group_context_id(plot_path: &[usize], index: usize, id: Option<String>) -> BakeContextId {
    if plot_path.is_empty() {
        BakeContextId::MarkGroup { index, id }
    } else {
        BakeContextId::ChildMarkGroup {
            subplot_path: plot_path.to_vec(),
            index,
            id,
        }
    }
}

async fn assemble_data_context(
    data_context: &CompiledDataContext,
    target: BakeTarget,
    context_id: BakeContextId,
    ctx: &SessionContext,
    time_context: TimeContext,
    data_mode: MarkDataMode,
    inherited_base_plan: Option<&LogicalPlan>,
    under_facet: bool,
) -> Result<Option<AssembledContext>, NotBakedReason> {
    if data_context.store_data().is_some() {
        return Err(NotBakedReason::StoreData);
    }
    if data_mode == MarkDataMode::Unit {
        return Ok(None);
    };
    let live_stage_tables = live_stage_session_tables(data_context.transforms());

    if under_facet {
        // A chain in a per-cell plot evaluates against facet-scoped data, and
        // shared-scale domain inference additionally evaluates it at the
        // sharing-owner scope (e.g. a global aggregate over all cells). No
        // single pre-evaluated table reproduces both, so the chain must stay
        // live. Groups with an explicit base plan get a base-only bake (the
        // base folds, the chain keeps running on it); inherited groups are
        // served by the enclosing plot-data bake instead.
        return match data_context.logical_plan_node() {
            Some(node) => {
                let base_plan = node.to_logical_plan(ctx).map_err(assembly_error)?;
                if plan_consumes_derived_scalars(&base_plan) {
                    return Err(NotBakedReason::DerivedScalars);
                }
                let base_table_names = table_names(&base_plan);
                let mut target = target;
                if let BakeTargetKind::MarkGroup {
                    keep_transforms, ..
                } = &mut target.kind
                {
                    *keep_transforms = true;
                }
                Ok(Some(AssembledContext {
                    target,
                    context_id,
                    plan: base_plan,
                    base_table_names,
                    live_stage_tables,
                }))
            }
            None if data_context.transforms().is_empty() => Ok(None),
            None => Err(NotBakedReason::FacetScopedTransforms),
        };
    }

    for (stage_index, stage) in data_context.transforms().iter().enumerate() {
        if stage.transform.execution_shape() != ExecutionShape::PlanRewrite {
            return Err(NotBakedReason::PlanBreakStage {
                stage_index,
                stage_type: transform_tag(stage),
            });
        }
    }

    let base_plan = if let Some(node) = data_context.logical_plan_node() {
        node.to_logical_plan(ctx).map_err(assembly_error)?
    } else if data_context.transforms().is_empty() {
        return Ok(None);
    } else if let Some(inherited_base_plan) = inherited_base_plan {
        inherited_base_plan.clone()
    } else {
        return Ok(None);
    };
    let base_table_names = table_names(&base_plan);
    let dataframe = DataFrame::new(ctx.state(), base_plan);
    let params = IndexMap::new();
    let execution_ctx = DataTransformExecutionContext {
        session_context: ctx,
        params: &params,
        time_context,
        facet_context: None,
    };
    let result =
        apply_compiled_data_transforms(dataframe, data_context.transforms(), &execution_ctx)
            .await
            .map_err(assembly_error)?;
    if !result.derived_scalars.is_empty() {
        return Err(NotBakedReason::DerivedScalars);
    }
    // In-chain consumption is resolved by `apply_compiled_data_transforms`
    // above; any derived-scalar placeholder that survives assembly is consumed
    // from OUTSIDE this chain and must keep the context live.
    if plan_consumes_derived_scalars(result.dataframe.logical_plan()) {
        return Err(NotBakedReason::DerivedScalars);
    }

    Ok(Some(AssembledContext {
        target,
        context_id,
        plan: result.dataframe.logical_plan().clone(),
        base_table_names,
        live_stage_tables,
    }))
}

/// Session tables read by a transform chain's stages besides their inputs.
fn live_stage_session_tables(transforms: &[avenger_chart_core::DataTransformStage]) -> Vec<String> {
    transforms
        .iter()
        .flat_map(|stage| stage.transform.referenced_session_tables())
        .collect()
}

fn emit_proto_context(
    compiled: &mut CompiledPlot,
    target: &BakeTarget,
    residual: &LogicalPlan,
    ctx: &SessionContext,
) -> Result<(), AvengerChartError> {
    let node = LogicalPlanNode::from_logical_plan(residual)?;
    let mut apply = |plot: &mut CompiledPlot| match &target.kind {
        BakeTargetKind::PlotData => {
            let original = plot.data.take();
            plot.data = Some(node.clone());
            // Subplot marks snapshot the plot's data plan into their
            // mark-local contexts at compile time (facet guides measure from
            // that snapshot). Retarget snapshots that byte-match the replaced
            // plan, or the baked plot keeps a live source-table reference
            // that fails open during guide measurement in a fresh session.
            if let Some(original) = original.as_ref() {
                retarget_subplot_data_snapshots(plot, original, &node, ctx)?;
            }
            Ok(())
        }
        BakeTargetKind::MarkGroup {
            index,
            keep_transforms,
        } => {
            let group = plot.mark_groups.get_mut(*index).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Compiled mark group index {index} is out of bounds during bake emit"
                ))
            })?;
            let transforms = if *keep_transforms {
                group.data.transforms().to_vec()
            } else {
                Vec::new()
            };
            group.data = CompiledDataContext::from_logical_plan_node_with_pattern_channels(
                Some(node.clone()),
                transforms,
                group.data.channels().clone(),
                group.data.pattern_channels().clone(),
            );
            Ok(())
        }
    };
    mutate_plot_at_path(compiled, &target.plot_path, &mut apply, ctx)
}

fn mutate_plot_at_path<F>(
    plot: &mut CompiledPlot,
    path: &[usize],
    apply: &mut F,
    ctx: &SessionContext,
) -> Result<(), AvengerChartError>
where
    F: FnMut(&mut CompiledPlot) -> Result<(), AvengerChartError>,
{
    let Some((&mark_index, rest)) = path.split_first() else {
        return apply(plot);
    };
    let replacement = {
        let mark = plot.marks.get(mark_index).ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Compiled subplot mark index {mark_index} is out of bounds during bake emit"
            ))
        })?;
        let payload = subplot_payload_for_mark(mark.as_ref()).ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Compiled mark index {mark_index} does not own a child plot during bake emit"
            ))
        })?;
        let mut child_plot = compiled_subplot_payload_child_plot_arc(payload)
            .as_ref()
            .clone();
        mutate_plot_at_path(&mut child_plot, rest, apply, ctx)?;
        rebuild_subplot_mark_with_child(mark.as_ref(), child_plot, ctx)?
    };
    plot.marks[mark_index] = replacement;
    Ok(())
}

/// Rewrite mark-local data-context SNAPSHOTS of the plot's replaced data
/// plan on subplot marks. Compile snapshots the plot data into these
/// contexts (facet band guides measure partition values from them); after a
/// plot-data emit they would otherwise keep referencing the original source
/// tables, which do not exist in a consuming session.
fn retarget_subplot_data_snapshots(
    plot: &mut CompiledPlot,
    original: &LogicalPlanNode,
    replacement: &LogicalPlanNode,
    ctx: &SessionContext,
) -> Result<(), AvengerChartError> {
    for index in 0..plot.marks.len() {
        let context = plot.marks[index].as_ref().data_context();
        if context.store_data().is_some() || context.logical_plan_node() != Some(original) {
            continue;
        }
        plot.marks[index] = mark_with_retargeted_data_node(&plot.marks[index], replacement, ctx)?;
    }
    // The compiled coordinate guide is built at compile time and captures its
    // own copy of the facet data plan (facet band guides measure partition
    // values from it).
    if let Some(guide) = plot.compiled_guide.as_ref()
        && let Some(retargeted) = guide.with_retargeted_data_plan(original, replacement)
    {
        plot.compiled_guide = Some(retargeted);
    }
    Ok(())
}

/// Retarget a mark's data context to `replacement`, preserving transforms
/// and channels. Fast path: the mark's own `with_data_context` (a struct
/// clone — every built-in mark implements it). Fallback for mark types
/// without it (e.g. external crates): a serde round-trip deep clone (the
/// same machinery that ships compiled plots, so it is lossless), whose
/// freshly deserialized `Arc` is uniquely owned so the trait-level
/// `state_mut` reaches the same storage that `data_context()` reads —
/// including subplot payload state. The fallback serializes the OLD
/// snapshot it is about to discard, so it can be slow for marks carrying
/// large inline sources; `with_data_context` is the supported remedy.
fn mark_with_retargeted_data_node(
    mark: &Arc<dyn CompiledMark>,
    replacement: &LogicalPlanNode,
    ctx: &SessionContext,
) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
    let context = mark.data_context();
    let retargeted_context = CompiledDataContext::from_logical_plan_node_with_pattern_channels(
        Some(replacement.clone()),
        context.transforms().to_vec(),
        context.channels().clone(),
        context.pattern_channels().clone(),
    );
    // facet_wrap marks RENDER through `physical_subplot`, a lowering that
    // captures the state's data snapshot in its own synthetic mark and
    // guide. A state-only retarget (the generic paths below) would leave
    // the render path on the stale plan, so wrap marks regenerate the
    // physical from the retargeted state instead.
    if mark.mark_type() == "facet_wrap" {
        let subplot = mark
            .as_any()
            .downcast_ref::<CompiledFacetWrapSubplot>()
            .ok_or_else(|| {
                AvengerChartError::InternalError(
                    "facet_wrap mark did not downcast during bake retarget".to_string(),
                )
            })?;
        let mut state = subplot.payload.compiled_state().clone();
        state.data = retargeted_context;
        let child = compiled_subplot_payload_child_plot_arc(&subplot.payload);
        return Ok(Arc::new(subplot.rebuilt_with(state, child, ctx)?));
    }
    if let Some(retargeted) = mark.with_data_context(retargeted_context.clone()) {
        return Ok(retargeted);
    }

    let encoded = bincode::serialize(mark).map_err(|err| {
        AvengerChartError::InternalError(format!(
            "Failed to serialize mark for bake snapshot retargeting: {err}"
        ))
    })?;
    let mut cloned: Arc<dyn CompiledMark> = bincode::deserialize(&encoded).map_err(|err| {
        AvengerChartError::InternalError(format!(
            "Failed to deserialize mark for bake snapshot retargeting: {err}"
        ))
    })?;
    Arc::get_mut(&mut cloned)
        .ok_or_else(|| {
            AvengerChartError::InternalError(
                "Freshly deserialized mark was not uniquely owned during bake emit".to_string(),
            )
        })?
        .state_mut()
        .data = retargeted_context;
    Ok(cloned)
}

fn subplot_payload_for_mark(mark: &dyn CompiledMark) -> Option<&CompiledSubplotPayload> {
    if let Some(subplot) = mark.as_positioned_subplot() {
        return Some(subplot.payload());
    }
    match mark.mark_type() {
        "facet_col" => mark
            .as_any()
            .downcast_ref::<CompiledFacetColumnSubplot>()
            .map(|subplot| &subplot.payload),
        "facet_row" => mark
            .as_any()
            .downcast_ref::<CompiledFacetRowSubplot>()
            .map(|subplot| &subplot.payload),
        "facet_wrap" => mark
            .as_any()
            .downcast_ref::<CompiledFacetWrapSubplot>()
            .map(|subplot| &subplot.payload),
        _ => None,
    }
}

fn rebuild_subplot_mark_with_child(
    mark: &dyn CompiledMark,
    child_plot: CompiledPlot,
    ctx: &SessionContext,
) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
    let child_concrete = Arc::new(child_plot);
    let child_arc: Arc<dyn CompiledSubplotChildPlot> = child_concrete.clone();
    match mark.mark_type() {
        "facet_col" => {
            let subplot = mark
                .as_any()
                .downcast_ref::<CompiledFacetColumnSubplot>()
                .ok_or_else(|| {
                    AvengerChartError::InternalError(
                        "facet_col mark did not downcast during bake emit".to_string(),
                    )
                })?;
            let mut rebuilt = subplot.clone();
            rebuilt.payload = payload_with_child_plot(&subplot.payload, child_arc);
            return Ok(Arc::new(rebuilt));
        }
        "facet_row" => {
            let subplot = mark
                .as_any()
                .downcast_ref::<CompiledFacetRowSubplot>()
                .ok_or_else(|| {
                    AvengerChartError::InternalError(
                        "facet_row mark did not downcast during bake emit".to_string(),
                    )
                })?;
            let mut rebuilt = subplot.clone();
            rebuilt.payload = payload_with_child_plot(&subplot.payload, child_arc);
            return Ok(Arc::new(rebuilt));
        }
        "facet_wrap" => {
            let subplot = mark
                .as_any()
                .downcast_ref::<CompiledFacetWrapSubplot>()
                .ok_or_else(|| {
                    AvengerChartError::InternalError(
                        "facet_wrap mark did not downcast during bake emit".to_string(),
                    )
                })?;
            // Regenerate the physical lowering from the new child; a
            // payload-only rebuild would leave the RENDER path on the old
            // child plot.
            let rebuilt = subplot.rebuilt_with(
                subplot.payload.compiled_state().clone(),
                child_concrete,
                ctx,
            )?;
            return Ok(Arc::new(rebuilt));
        }
        _ => {}
    }
    if let Some(subplot) = mark.as_positioned_subplot() {
        let payload = payload_with_child_plot(subplot.payload(), child_arc);
        return Ok(Arc::new(CompiledPositionedSubplot::new(
            payload,
            subplot.spec().clone(),
            subplot.child_plot_size().clone(),
            subplot.partition_expr().cloned(),
        )));
    }
    Err(AvengerChartError::InternalError(
        "Compiled mark does not own a replaceable child plot during bake emit".to_string(),
    ))
}

fn payload_with_child_plot(
    payload: &CompiledSubplotPayload,
    child_plot: Arc<dyn CompiledSubplotChildPlot>,
) -> CompiledSubplotPayload {
    CompiledSubplotPayload::new(
        payload.compiled_state().clone(),
        child_plot,
        payload.label().map(ToOwned::to_owned),
        payload.key().map(ToOwned::to_owned),
        payload.data_source(),
    )
}

enum PrimaryMatch<'a> {
    None,
    One(&'a BakedSubtree),
    Multiple(Vec<String>),
}

fn primary_baked_table<'a>(
    context: &AssembledContext,
    residual: &LogicalPlan,
    baked: &'a [BakedSubtree],
) -> PrimaryMatch<'a> {
    if context.base_table_names.is_empty() {
        return PrimaryMatch::None;
    }

    let all_matches = primary_matches(context, baked);
    let residual_table_names = table_names(residual).into_iter().collect::<HashSet<_>>();
    let referenced = all_matches
        .iter()
        .copied()
        .filter(|table| residual_table_names.contains(&table.table_name))
        .collect::<Vec<_>>();
    if !referenced.is_empty() {
        return primary_match_from_tables(referenced);
    }

    primary_match_from_tables(all_matches)
}

fn primary_matches<'a>(
    context: &AssembledContext,
    baked: &'a [BakedSubtree],
) -> Vec<&'a BakedSubtree> {
    baked
        .iter()
        .filter(|table| {
            context
                .base_table_names
                .iter()
                .all(|name| table.source_tables.iter().any(|source| source == name))
        })
        .collect::<Vec<_>>()
}

fn primary_match_from_tables(tables: Vec<&BakedSubtree>) -> PrimaryMatch<'_> {
    match tables.as_slice() {
        [] => PrimaryMatch::None,
        [table] => PrimaryMatch::One(table),
        tables => PrimaryMatch::Multiple(
            tables
                .iter()
                .map(|table| table.table_name.clone())
                .collect::<Vec<_>>(),
        ),
    }
}

/// Whether the plan contains placeholders that consume derived scalars.
fn plan_consumes_derived_scalars(plan: &LogicalPlan) -> bool {
    let mut found = false;
    let _ = plan.apply(|node| {
        for expr in node.expressions() {
            let _ = expr.apply(|candidate| {
                if let Expr::Placeholder(placeholder) = candidate
                    && derived_scalar_id_from_placeholder(&placeholder.id).is_some()
                {
                    found = true;
                    return Ok(TreeNodeRecursion::Stop);
                }
                Ok(TreeNodeRecursion::Continue)
            });
            if found {
                return Ok(TreeNodeRecursion::Stop);
            }
        }
        Ok(TreeNodeRecursion::Continue)
    });
    found
}

fn residual_self_contained(plan: &LogicalPlan, baked_table_names: &HashSet<String>) -> bool {
    table_names(plan).into_iter().all(|name| {
        // Anonymous scans (`?table?`) carry inline providers that serialize
        // with the plan; only NAMED scans need a manifest entry behind them.
        name == datafusion::logical_expr::UNNAMED_TABLE || baked_table_names.contains(&name)
    })
}

fn table_names(plan: &LogicalPlan) -> Vec<String> {
    let mut tables = BTreeSet::new();
    let _ = plan.apply(|node| {
        if let LogicalPlan::TableScan(scan) = node {
            tables.insert(scan.table_name.to_string());
        }
        Ok(TreeNodeRecursion::Continue)
    });
    tables.into_iter().collect()
}

fn transform_tag(stage: &avenger_chart_core::DataTransformStage) -> Option<String> {
    let value = serde_json::to_value(&stage.transform).ok()?;
    match value {
        Value::Object(map) => map
            .get("type")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        _ => None,
    }
}

fn assembly_error(err: AvengerChartError) -> NotBakedReason {
    NotBakedReason::AssemblyError {
        message: err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use avenger_chart_core::{
        CoordinationScope, DataTransform, DataTransformCompileContext, DataTransformStage,
    };
    use avenger_chart_transforms::{Aggregate, Calculate, Filter, Kde, Sql};
    use datafusion::{
        arrow::{
            array::{Float64Array, StringArray},
            datatypes::{DataType, Field, Schema},
            record_batch::RecordBatch,
        },
        common::ScalarValue,
        prelude::{col, lit},
    };

    use crate::prelude::*;

    use super::*;

    fn sales_batch() -> RecordBatch {
        RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("region", DataType::Utf8, false),
                Field::new("value", DataType::Float64, false),
            ])),
            vec![
                Arc::new(StringArray::from(vec!["EU", "EU", "NA", "NA"])) as _,
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0])) as _,
            ],
        )
        .expect("sales batch")
    }

    async fn register_sales(ctx: &SessionContext) -> DataFrame {
        ctx.register_batch("sales", sales_batch())
            .expect("register sales");
        ctx.table("sales").await.expect("sales table")
    }

    fn stage<T>(transform: T) -> DataTransformStage
    where
        T: DataTransform,
    {
        let (compiled, _) = transform
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free))
            .expect("compile transform");
        DataTransformStage::new(CoordinationScope::Free, compiled)
    }

    fn data_context(
        dataframe: DataFrame,
        transforms: Vec<DataTransformStage>,
    ) -> CompiledDataContext {
        CompiledDataContext::new(Some(dataframe), transforms, IndexMap::new())
    }

    #[tokio::test]
    async fn plan_pure_chain_assembles_with_unbound_placeholders()
    -> Result<(), Box<dyn std::error::Error>> {
        let ctx = SessionContext::new();
        let dataframe = register_sales(&ctx).await;
        let min = Param::new("min", ScalarValue::Float64(Some(0.0)));
        let data_context = data_context(
            dataframe,
            vec![
                stage(Filter::new(col("value").gt(min.expr()))),
                stage(Calculate::new().expr("double_value", col("value") * lit(2.0))),
                stage(
                    Aggregate::new()
                        .group_by([col("region")])
                        .sum("total", col("double_value")),
                ),
            ],
        );

        let assembled = match assemble_data_context(
            &data_context,
            BakeTarget {
                plot_path: Vec::new(),
                kind: BakeTargetKind::MarkGroup {
                    index: 0,
                    keep_transforms: false,
                },
            },
            BakeContextId::MarkGroup {
                index: 0,
                id: Some("synthetic".to_string()),
            },
            &ctx,
            TimeContext::default(),
            MarkDataMode::Inherit,
            None,
            false,
        )
        .await
        {
            Ok(Some(assembled)) => assembled,
            Ok(None) => panic!("expected assembled context"),
            Err(reason) => panic!("unexpected not-baked reason: {reason:?}"),
        };

        let display = assembled.plan.display_indent().to_string();
        assert_eq!(assembled.base_table_names, vec!["sales".to_string()]);
        assert!(display.contains("Aggregate"), "{display}");
        assert!(display.contains("Filter"), "{display}");
        assert!(display.contains("double_value"), "{display}");
        assert!(display.contains("$min"), "{display}");
        Ok(())
    }

    #[tokio::test]
    async fn plan_break_stage_reports_not_baked_reason() -> Result<(), Box<dyn std::error::Error>> {
        let ctx = SessionContext::new();
        let dataframe = register_sales(&ctx).await;
        let data_context = data_context(
            dataframe,
            vec![stage(
                Kde::new(col("value")).bandwidth(lit(1.0)).steps(lit(8_i64)),
            )],
        );

        let err = match assemble_data_context(
            &data_context,
            BakeTarget {
                plot_path: Vec::new(),
                kind: BakeTargetKind::MarkGroup {
                    index: 0,
                    keep_transforms: false,
                },
            },
            BakeContextId::MarkGroup { index: 0, id: None },
            &ctx,
            TimeContext::default(),
            MarkDataMode::Inherit,
            None,
            false,
        )
        .await
        {
            Ok(_) => panic!("kde should remain a plan break"),
            Err(reason) => reason,
        };

        assert_eq!(
            err,
            NotBakedReason::PlanBreakStage {
                stage_index: 0,
                stage_type: Some("kde".to_string()),
            }
        );
        Ok(())
    }

    #[tokio::test]
    async fn sql_stage_assembles_via_parse_and_splice_apply()
    -> Result<(), Box<dyn std::error::Error>> {
        let ctx = SessionContext::new();
        let dataframe = register_sales(&ctx).await;
        let data_context = data_context(
            dataframe,
            vec![stage(Sql::new(
                "SELECT region, value * 2.0 AS adjusted \
                 FROM input WHERE value >= $min",
            ))],
        );

        let assembled = match assemble_data_context(
            &data_context,
            BakeTarget {
                plot_path: Vec::new(),
                kind: BakeTargetKind::MarkGroup {
                    index: 0,
                    keep_transforms: false,
                },
            },
            BakeContextId::MarkGroup { index: 0, id: None },
            &ctx,
            TimeContext::default(),
            MarkDataMode::Inherit,
            None,
            false,
        )
        .await
        {
            Ok(Some(assembled)) => assembled,
            Ok(None) => panic!("expected assembled sql context"),
            Err(reason) => panic!("unexpected not-baked reason: {reason:?}"),
        };

        let display = assembled.plan.display_indent().to_string();
        assert_eq!(assembled.base_table_names, vec!["sales".to_string()]);
        assert!(display.contains("SubqueryAlias: input"), "{display}");
        assert!(display.contains("adjusted"), "{display}");
        assert!(display.contains("$min"), "{display}");
        Ok(())
    }

    async fn shared_chain_plot(ctx: &SessionContext) -> Result<CompiledPlot, AvengerChartError> {
        ctx.register_batch("sales", sales_batch())?;
        let sql = "SELECT region, SUM(value) AS total FROM sales GROUP BY region ORDER BY region";
        let left = ctx.sql(sql).await?;
        let right = ctx.sql(sql).await?;

        crate::plot::Chart::<Cartesian>::new()
            .mark(
                MarkGroup::new()
                    .data(left)
                    .mark(Symbol::new().x(col("total")).y(col("total")).size(48.0)),
            )
            .mark(
                MarkGroup::new()
                    .data(right)
                    .mark(Symbol::new().x(col("total")).y(col("total")).size(24.0)),
            )
            .compile(ctx)
            .await
    }

    #[tokio::test]
    async fn faceted_inherited_chain_stays_live_and_plot_data_assembles()
    -> Result<(), Box<dyn std::error::Error>> {
        let ctx = SessionContext::new();
        let data = register_sales(&ctx).await;
        let leaf = crate::plot::Plot::<Cartesian>::new().mark(MarkGroup::new().transform(
            Aggregate::new().sum("total", col("value")),
            |group, aggregate| {
                group.mark(
                    Symbol::new()
                        .x(aggregate.output("total"))
                        .y(aggregate.output("total"))
                        .size(48.0),
                )
            },
        ));
        let compiled = crate::plot::Chart::<FacetColumn>::new()
            .data(data)
            .mark(Subplot::new(leaf).column(col("region")))
            .compile(&ctx)
            .await?;

        let assembly = assemble(&compiled, &ctx).await;
        // The inherited per-cell chain stays live: it is a status, not a
        // context, and the only assembled context is the root plot data.
        assert!(
            assembly.statuses.iter().any(|status| matches!(
                status,
                ContextBakeStatus::NotBaked {
                    context_id: BakeContextId::ChildMarkGroup { subplot_path, index: 0, .. },
                    reason: NotBakedReason::FacetScopedTransforms,
                } if subplot_path == &[0]
            )),
            "statuses: {:#?}",
            assembly.statuses
        );
        assert_eq!(assembly.contexts.len(), 1, "{:#?}", assembly.statuses);
        assert!(matches!(
            assembly.contexts[0].context_id,
            BakeContextId::PlotData
        ));
        Ok(())
    }

    #[tokio::test]
    async fn facet_wrap_inherited_chain_stays_live_and_plot_data_assembles()
    -> Result<(), Box<dyn std::error::Error>> {
        let ctx = SessionContext::new();
        let data = register_sales(&ctx).await;
        let leaf = crate::plot::Plot::<Cartesian>::new().mark(MarkGroup::new().transform(
            Aggregate::new().sum("total", col("value")),
            |group, aggregate| {
                group.mark(
                    Symbol::new()
                        .x(aggregate.output("total"))
                        .y(aggregate.output("total"))
                        .size(48.0),
                )
            },
        ));
        let compiled = crate::plot::Chart::<FacetWrap>::new()
            .data(data)
            .mark(Subplot::new(leaf).wrap_with(col("region"), |c| c.columns(2)))
            .compile(&ctx)
            .await?;

        let assembly = assemble(&compiled, &ctx).await;
        assert!(
            assembly.statuses.iter().any(|status| matches!(
                status,
                ContextBakeStatus::NotBaked {
                    context_id: BakeContextId::ChildMarkGroup { subplot_path, index: 0, .. },
                    reason: NotBakedReason::FacetScopedTransforms,
                } if subplot_path == &[0]
            )),
            "statuses: {:#?}",
            assembly.statuses
        );
        assert_eq!(assembly.contexts.len(), 1, "{:#?}", assembly.statuses);
        assert!(matches!(
            assembly.contexts[0].context_id,
            BakeContextId::PlotData
        ));
        Ok(())
    }

    /// Wrap marks render through `physical_subplot`; a plot-data emit must
    /// leave BOTH the payload snapshot and the regenerated physical lowering
    /// (its synthetic column mark) pointing at the baked residual, with the
    /// child plot Arc passed through untouched.
    #[tokio::test]
    async fn facet_wrap_bake_regenerates_physical_subplot() -> Result<(), Box<dyn std::error::Error>>
    {
        let ctx = SessionContext::new();
        ctx.register_batch("sales", sales_batch())?;
        let data = ctx.sql("SELECT * FROM sales WHERE value > $min").await?;
        let leaf = crate::plot::Plot::<Cartesian>::new().mark(MarkGroup::new().transform(
            Aggregate::new().sum("total", col("value")),
            |group, aggregate| {
                group.mark(
                    Symbol::new()
                        .x(aggregate.output("total"))
                        .y(aggregate.output("total"))
                        .size(48.0),
                )
            },
        ));
        let compiled = crate::plot::Chart::<FacetWrap>::new()
            .data(data)
            .mark(Subplot::new(leaf).wrap_with(col("region"), |c| c.columns(2)))
            .compile(&ctx)
            .await?;
        let original_node = compiled.data.clone().expect("plot data");
        let original_wrap = compiled.marks[0]
            .as_any()
            .downcast_ref::<CompiledFacetWrapSubplot>()
            .expect("wrap mark");
        let original_child = compiled_subplot_payload_child_plot_arc(&original_wrap.payload);

        let (baked, report) = compiled.bake(&ctx, &BakePolicy::default()).await?;
        assert!(report.self_contained, "{:#?}", report.contexts);
        let baked_node = baked.data.as_ref().expect("baked plot data");
        assert_ne!(baked_node, &original_node);

        let wrap = baked.marks[0]
            .as_any()
            .downcast_ref::<CompiledFacetWrapSubplot>()
            .expect("baked wrap mark");
        // Payload snapshot retargeted...
        assert_eq!(
            wrap.payload.compiled_state().data.logical_plan_node(),
            Some(baked_node)
        );
        // ...the physical lowering's synthetic column mark regenerated from
        // the retargeted state...
        let synthetic = wrap.physical_subplot().marks[0].as_ref();
        assert_eq!(
            synthetic.data_context().logical_plan_node(),
            Some(baked_node)
        );
        // ...the child plot passed through by Arc identity (nothing inside
        // it was baked in this chart)...
        assert!(Arc::ptr_eq(
            &compiled_subplot_payload_child_plot_arc(&wrap.payload),
            &original_child
        ));
        // ...and no mark anywhere still targets the original plan.
        assert!(
            !baked
                .marks
                .iter()
                .chain(wrap.physical_subplot().marks.iter())
                .any(|mark| mark.data_context().logical_plan_node() == Some(&original_node))
        );
        Ok(())
    }

    #[tokio::test]
    async fn faceted_explicit_group_gets_base_only_bake() -> Result<(), Box<dyn std::error::Error>>
    {
        let ctx = SessionContext::new();
        let dataframe = register_sales(&ctx).await;
        let data_context = data_context(
            dataframe,
            vec![stage(
                Aggregate::new()
                    .group_by([col("region")])
                    .sum("total", col("value")),
            )],
        );

        let assembled = match assemble_data_context(
            &data_context,
            BakeTarget {
                plot_path: vec![0],
                kind: BakeTargetKind::MarkGroup {
                    index: 0,
                    keep_transforms: false,
                },
            },
            BakeContextId::ChildMarkGroup {
                subplot_path: vec![0],
                index: 0,
                id: None,
            },
            &ctx,
            TimeContext::default(),
            MarkDataMode::Inherit,
            None,
            true,
        )
        .await
        {
            Ok(Some(assembled)) => assembled,
            Ok(None) => panic!("expected base-only assembled context"),
            Err(reason) => panic!("unexpected not-baked reason: {reason:?}"),
        };

        // The assembled plan is the base only; the chain stays live and is
        // preserved through emit.
        assert!(matches!(
            assembled.target.kind,
            BakeTargetKind::MarkGroup {
                index: 0,
                keep_transforms: true,
            }
        ));
        let display = assembled.plan.display_indent().to_string();
        assert!(!display.contains("Aggregate"), "{display}");
        assert_eq!(assembled.base_table_names, vec!["sales".to_string()]);
        Ok(())
    }

    #[tokio::test]
    async fn bake_uses_one_partial_eval_set_for_cross_context_dedup()
    -> Result<(), Box<dyn std::error::Error>> {
        let ctx = SessionContext::new();
        let compiled = shared_chain_plot(&ctx).await?;

        let assembly = assemble(&compiled, &ctx).await;
        assert_eq!(assembly.contexts.len(), 2);
        let (_, partial_report) = partial_evaluate_set(
            assembly
                .contexts
                .iter()
                .map(|context| context.plan.clone())
                .collect(),
            &ctx,
            &BakePolicy::default().to_partial_eval_policy(
                compiled.runtime_unfoldable_tables(),
                crate::bake::unique_bake_table_prefix(),
            ),
        )
        .await?;
        assert_eq!(partial_report.baked.len(), 1);
        assert!(partial_report.baked[0].occurrences >= 2);

        let (baked, report) = compiled.bake(&ctx, &BakePolicy::default()).await?;
        assert_eq!(baked.baked_tables.len(), 1);
        let primary_tables = report
            .contexts
            .iter()
            .filter_map(|status| match status {
                ContextBakeStatus::Baked { primary_table, .. } => Some(primary_table.clone()),
                ContextBakeStatus::NotBaked { .. } => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(primary_tables.len(), 2);
        assert_eq!(primary_tables[0], primary_tables[1]);
        Ok(())
    }

    /// Compile snapshots the plot's data plan into subplot mark-local
    /// contexts and into the compiled facet guide; a plot-data emit must
    /// retarget both, or the baked plot keeps live source-table references
    /// that fail open during facet guide measurement in a fresh session.
    #[tokio::test]
    async fn plot_data_bake_retargets_subplot_and_guide_snapshots()
    -> Result<(), Box<dyn std::error::Error>> {
        let ctx = SessionContext::new();
        ctx.register_batch("sales", sales_batch())?;
        let data = ctx.sql("SELECT * FROM sales WHERE value > $min").await?;
        let leaf = crate::plot::Plot::<Cartesian>::new().mark(MarkGroup::new().transform(
            Aggregate::new().sum("total", col("value")),
            |group, aggregate| {
                group.mark(
                    Symbol::new()
                        .x(aggregate.output("total"))
                        .y(aggregate.output("total"))
                        .size(48.0),
                )
            },
        ));
        let compiled = crate::plot::Chart::<FacetColumn>::new()
            .data(data)
            .mark(Subplot::new(leaf).column(col("region")))
            .compile(&ctx)
            .await?;

        // Compile produced byte-equal snapshots of the plot data plan.
        let original_node = compiled.data.clone().expect("plot data");
        assert_eq!(
            compiled.marks[0]
                .as_ref()
                .data_context()
                .logical_plan_node(),
            Some(&original_node)
        );

        let (baked, _) = compiled.bake(&ctx, &BakePolicy::default()).await?;
        let baked_plot_node = baked.data.as_ref().expect("baked plot data");
        assert_ne!(baked_plot_node, &original_node);

        // The mark snapshot now points at the residual...
        assert_eq!(
            baked.marks[0].as_ref().data_context().logical_plan_node(),
            Some(baked_plot_node)
        );
        // ...and inside the facet child, nothing still targets the original.
        assert!(
            !baked
                .marks
                .iter()
                .any(|mark| mark.data_context().logical_plan_node() == Some(&original_node))
        );
        // ...and it decodes in a fresh session once the manifest registers
        // (the residual references only baked tables).
        let client_ctx = SessionContext::new();
        assert!(baked_plot_node.to_logical_plan(&client_ctx).is_err());
        crate::bake::register_baked_tables(&client_ctx, &baked.baked_tables)?;
        let plan = baked_plot_node.to_logical_plan(&client_ctx)?;
        assert!(
            table_names(&plan)
                .iter()
                .all(|name| name.starts_with("__pe_baked_")),
            "{:?}",
            table_names(&plan)
        );
        Ok(())
    }

    /// PLAIN marks also receive compile-time snapshots of the plot data
    /// (evaluation prefers the mark's own context over plot data, so an
    /// un-retargeted snapshot keeps the baked chart on the ORIGINAL plan —
    /// re-executing folded work and, for inline sources, embedding every raw
    /// row in the artifact).
    #[tokio::test]
    async fn plot_data_bake_retargets_plain_mark_snapshots()
    -> Result<(), Box<dyn std::error::Error>> {
        let ctx = SessionContext::new();
        // Unnamed in-memory source: serializes INLINE, so a leaked snapshot
        // would visibly bloat the artifact.
        let raw = ctx.read_batch(big_batch())?;
        let data = raw
            .aggregate(
                vec![col("region")],
                vec![datafusion::functions_aggregate::expr_fn::sum(col("value")).alias("total")],
            )?
            .filter(col("total").gt(datafusion::prelude::placeholder("$min")))?;
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .data(data)
            .mark(Symbol::new().x(col("total")).y(col("total")).size(48.0))
            .compile(&ctx)
            .await?;
        let original_node = compiled.data.clone().expect("plot data");
        assert_eq!(
            compiled.marks[0]
                .as_ref()
                .data_context()
                .logical_plan_node(),
            Some(&original_node),
            "compile should snapshot plot data into the plain mark"
        );

        let (baked, report) = compiled.bake(&ctx, &BakePolicy::default()).await?;
        assert!(report.self_contained, "{:#?}", report.contexts);
        let baked_plot_node = baked.data.as_ref().expect("baked plot data");
        assert_eq!(
            baked.marks[0].as_ref().data_context().logical_plan_node(),
            Some(baked_plot_node)
        );

        // The artifact must not carry the raw rows through a leaked
        // snapshot: it stays well under the raw source's serialized size.
        let artifact_len = bincode::serialize(&baked)?.len();
        let unbaked_len = bincode::serialize(&compiled)?.len();
        assert!(
            artifact_len * 2 < unbaked_len,
            "artifact {artifact_len} bytes vs unbaked {unbaked_len} bytes"
        );
        Ok(())
    }

    #[tokio::test]
    async fn bakes_use_disjoint_table_names_across_bakes() -> Result<(), Box<dyn std::error::Error>>
    {
        let ctx = SessionContext::new();
        let compiled = shared_chain_plot(&ctx).await?;

        let (first, _) = compiled.bake(&ctx, &BakePolicy::default()).await?;
        let (second, _) = compiled.bake(&ctx, &BakePolicy::default()).await?;
        let first_names = first
            .baked_tables
            .iter()
            .map(|entry| entry.name.clone())
            .collect::<HashSet<_>>();
        let second_names = second
            .baked_tables
            .iter()
            .map(|entry| entry.name.clone())
            .collect::<HashSet<_>>();

        assert!(!first_names.is_empty());
        assert!(
            first_names.is_disjoint(&second_names),
            "{first_names:?} vs {second_names:?}"
        );
        Ok(())
    }

    fn big_batch() -> RecordBatch {
        let regions = (0..4096)
            .map(|index| if index % 2 == 0 { "EU" } else { "NA" })
            .collect::<Vec<_>>();
        let values = (0..4096).map(|index| index as f64).collect::<Vec<_>>();
        RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("region", DataType::Utf8, false),
                Field::new("value", DataType::Float64, false),
            ])),
            vec![
                Arc::new(StringArray::from(regions)) as _,
                Arc::new(Float64Array::from(values)) as _,
            ],
        )
        .expect("big batch")
    }

    /// A baked table referenced only by a NOT-emitted (pass-through) residual
    /// must not ship in the plot manifest.
    #[tokio::test]
    async fn manifest_drops_tables_unreferenced_by_emitted_residuals()
    -> Result<(), Box<dyn std::error::Error>> {
        let ctx = SessionContext::new();
        ctx.register_batch("sales", sales_batch())?;
        ctx.register_batch("big", big_batch())?;
        ctx.register_batch("side", sales_batch())?;

        // Group A: folds fully and is emitted.
        let folding = ctx.sql("SELECT region, value FROM sales").await?;
        // Group B: the join's side folds within budget, the big base does
        // not, so B passes through — its baked side table is dead weight.
        let partial = ctx
            .sql(
                "SELECT big.value, side.value AS threshold \
                 FROM big JOIN side ON big.region = side.region",
            )
            .await?;
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .mark(
                MarkGroup::new()
                    .data(folding)
                    .mark(Symbol::new().x(col("value")).y(col("value")).size(48.0)),
            )
            .mark(
                MarkGroup::new()
                    .data(partial)
                    .mark(Symbol::new().x(col("value")).y(col("threshold")).size(24.0)),
            )
            .compile(&ctx)
            .await?;

        let policy = BakePolicy {
            max_baked_bytes_per_subtree: 8 * 1024,
            ..BakePolicy::default()
        };
        let (baked, report) = compiled.bake(&ctx, &policy).await?;

        let emitted_primaries = report
            .contexts
            .iter()
            .filter_map(|status| match status {
                ContextBakeStatus::Baked { primary_table, .. } => Some(primary_table.clone()),
                ContextBakeStatus::NotBaked { .. } => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(emitted_primaries.len(), 1, "{:#?}", report.contexts);
        assert!(report.contexts.iter().any(|status| matches!(
            status,
            ContextBakeStatus::NotBaked {
                reason: NotBakedReason::BaseNotFolded { .. },
                ..
            }
        )));
        // The manifest holds ONLY the emitted context's table, even though
        // the crate report also baked B's side subtree.
        let manifest_names = baked
            .baked_tables
            .iter()
            .map(|entry| entry.name.clone())
            .collect::<Vec<_>>();
        assert_eq!(manifest_names, emitted_primaries);
        Ok(())
    }

    /// An unnamed inline scan (`?table?`) carries its provider with the plan
    /// and must not flag the residual as needing external tables.
    #[tokio::test]
    async fn residual_self_contained_allows_unnamed_scans() -> Result<(), Box<dyn std::error::Error>>
    {
        let ctx = SessionContext::new();
        let plan = ctx.read_batch(sales_batch())?.logical_plan().clone();
        assert_eq!(
            table_names(&plan),
            vec![datafusion::logical_expr::UNNAMED_TABLE.to_string()]
        );
        assert!(residual_self_contained(&plan, &HashSet::new()));
        Ok(())
    }
}
