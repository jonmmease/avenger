use std::collections::{BTreeSet, HashSet};

use avenger_chart_core::{
    AvengerChartError, CompiledDataContext, DataTransformExecutionContext, ExecutionShape,
    LogicalPlanNodeExt, TimeContext, apply_compiled_data_transforms,
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

use super::CompiledPlot;

#[derive(Clone)]
enum BakeTarget {
    PlotData,
    MarkGroup(usize),
}

#[derive(Clone)]
struct AssembledContext {
    target: BakeTarget,
    context_id: BakeContextId,
    plan: LogicalPlan,
    base_table_names: Vec<String>,
}

struct Assembly {
    contexts: Vec<AssembledContext>,
    statuses: Vec<ContextBakeStatus>,
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
        let partial_policy = policy.to_partial_eval_policy(self.runtime_unfoldable_tables());

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

        for (assembled, residual) in assembly.contexts.iter().zip(residuals.iter()) {
            match primary_baked_table(assembled, &partial_report.baked) {
                PrimaryMatch::One(primary) => {
                    emit_proto_context(&mut baked, &assembled.target, residual)?;
                    let self_contained = residual_self_contained(residual, &baked_table_names);
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
                    statuses.push(ContextBakeStatus::NotBaked {
                        context_id: assembled.context_id.clone(),
                        reason: NotBakedReason::BaseNotFolded {
                            skipped: skip_reasons.clone(),
                        },
                    });
                }
                PrimaryMatch::Multiple(table_names) => {
                    statuses.push(ContextBakeStatus::NotBaked {
                        context_id: assembled.context_id.clone(),
                        reason: NotBakedReason::MultiplePrimaryTables { table_names },
                    });
                }
            }
        }

        baked.baked_tables = if any_context_baked {
            partial_report
                .baked
                .iter()
                .map(|table| {
                    BakedTableManifestEntry::from_batches(
                        table.table_name.clone(),
                        table.schema.clone(),
                        &table.batches,
                        table.rows,
                        table.bytes,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?
        } else {
            Vec::new()
        };

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
            self_contained: any_context_baked && all_baked_self_contained,
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
    let mut contexts = Vec::new();
    let mut statuses = Vec::new();

    match assemble_plot_data(compiled, ctx).await {
        Ok(Some(context)) => contexts.push(context),
        Ok(None) => statuses.push(ContextBakeStatus::NotBaked {
            context_id: BakeContextId::PlotData,
            reason: NotBakedReason::NoData,
        }),
        Err(reason) => statuses.push(ContextBakeStatus::NotBaked {
            context_id: BakeContextId::PlotData,
            reason,
        }),
    }

    for (index, group) in compiled.mark_groups.iter().enumerate() {
        let context_id = BakeContextId::MarkGroup {
            index,
            id: group.id.clone(),
        };
        match assemble_data_context(
            &group.data,
            BakeTarget::MarkGroup(index),
            context_id.clone(),
            ctx,
            compiled.time_context.clone(),
        )
        .await
        {
            Ok(Some(context)) => contexts.push(context),
            // Groups without their own plan inherit the parent plot's data;
            // report them so every context is accounted for.
            Ok(None) => statuses.push(ContextBakeStatus::NotBaked {
                context_id,
                reason: NotBakedReason::NoData,
            }),
            Err(reason) => statuses.push(ContextBakeStatus::NotBaked { context_id, reason }),
        }
    }

    Assembly { contexts, statuses }
}

async fn assemble_plot_data(
    compiled: &CompiledPlot,
    ctx: &SessionContext,
) -> Result<Option<AssembledContext>, NotBakedReason> {
    let Some(node) = compiled.data.as_ref() else {
        return Ok(None);
    };
    let plan = node.to_logical_plan(ctx).map_err(assembly_error)?;
    if plan_consumes_derived_scalars(&plan) {
        return Err(NotBakedReason::DerivedScalars);
    }
    let base_table_names = table_names(&plan);
    Ok(Some(AssembledContext {
        target: BakeTarget::PlotData,
        context_id: BakeContextId::PlotData,
        plan,
        base_table_names,
    }))
}

async fn assemble_data_context(
    data_context: &CompiledDataContext,
    target: BakeTarget,
    context_id: BakeContextId,
    ctx: &SessionContext,
    time_context: TimeContext,
) -> Result<Option<AssembledContext>, NotBakedReason> {
    if data_context.store_data().is_some() {
        return Err(NotBakedReason::StoreData);
    }
    let Some(node) = data_context.logical_plan_node() else {
        return Ok(None);
    };
    for (stage_index, stage) in data_context.transforms().iter().enumerate() {
        if stage.transform.execution_shape() != ExecutionShape::PlanRewrite {
            return Err(NotBakedReason::PlanBreakStage {
                stage_index,
                stage_type: transform_tag(stage),
            });
        }
    }

    let base_plan = node.to_logical_plan(ctx).map_err(assembly_error)?;
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
    }))
}

fn emit_proto_context(
    compiled: &mut CompiledPlot,
    target: &BakeTarget,
    residual: &LogicalPlan,
) -> Result<(), AvengerChartError> {
    let node = LogicalPlanNode::from_logical_plan(residual)?;
    match target {
        BakeTarget::PlotData => {
            compiled.data = Some(node);
        }
        BakeTarget::MarkGroup(index) => {
            let group = compiled.mark_groups.get_mut(*index).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Compiled mark group index {index} is out of bounds during bake emit"
                ))
            })?;
            group.data = CompiledDataContext::from_logical_plan_node_with_pattern_channels(
                Some(node),
                Vec::new(),
                group.data.channels().clone(),
                group.data.pattern_channels().clone(),
            );
        }
    }
    Ok(())
}

enum PrimaryMatch<'a> {
    None,
    One(&'a BakedSubtree),
    Multiple(Vec<String>),
}

fn primary_baked_table<'a>(
    context: &AssembledContext,
    baked: &'a [BakedSubtree],
) -> PrimaryMatch<'a> {
    if context.base_table_names.is_empty() {
        return PrimaryMatch::None;
    }

    let matches = baked
        .iter()
        .filter(|table| {
            context
                .base_table_names
                .iter()
                .all(|name| table.source_tables.iter().any(|source| source == name))
        })
        .collect::<Vec<_>>();

    match matches.as_slice() {
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
    table_names(plan)
        .into_iter()
        .all(|name| baked_table_names.contains(&name))
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
