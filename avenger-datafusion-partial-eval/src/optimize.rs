//! Bake-specific DataFusion optimization.

use std::collections::BTreeSet;
use std::sync::Arc;

use datafusion::logical_expr::LogicalPlan;
use datafusion::optimizer::OptimizerContext;
use datafusion::prelude::SessionContext;
use datafusion_common::Result;

use crate::PartialEvalPolicy;
use crate::params::{bind_fixed_params, wrap_held_predicates};

pub(crate) struct PreparedPlan {
    pub(crate) plan: LogicalPlan,
    pub(crate) fixed_params_applied: BTreeSet<String>,
}

pub(crate) async fn bake_optimize(plan: LogicalPlan, ctx: &SessionContext) -> Result<LogicalPlan> {
    let state = ctx.state();
    let analyzed = state
        .analyzer()
        .execute_and_check(plan, state.config_options(), |_, _| {})?;
    let optimizer_context =
        OptimizerContext::new_with_config_options(Arc::clone(state.config_options()))
            .without_query_execution_start_time();
    state
        .optimizer()
        .optimize(analyzed, &optimizer_context, |_, _| {})
}

pub(crate) async fn prepare(
    plan: LogicalPlan,
    ctx: &SessionContext,
    policy: &PartialEvalPolicy,
) -> Result<PreparedPlan> {
    let (bound, fixed_params_applied) = bind_fixed_params(plan, &policy.fixed_params)?;
    let held = wrap_held_predicates(bound)?;
    let plan = bake_optimize(held, ctx).await?;
    Ok(PreparedPlan {
        plan,
        fixed_params_applied,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{Float64Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::record_batch::RecordBatch;
    use datafusion::logical_expr::expr::Placeholder;
    use datafusion::logical_expr::{Expr, LogicalPlan};
    use datafusion::prelude::SessionContext;
    use datafusion_common::ScalarValue;
    use datafusion_common::tree_node::{TreeNode, TreeNodeRecursion};

    use super::*;
    use crate::params::{PE_HOLD_UDF_NAME, collect_placeholder_ids};

    fn table_batch() -> RecordBatch {
        RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("x", DataType::Float64, false),
                Field::new("region", DataType::Utf8, false),
            ])),
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])) as _,
                Arc::new(StringArray::from(vec!["EU", "NA", "EU"])) as _,
            ],
        )
        .unwrap()
    }

    async fn ctx_and_plan(sql: &str) -> (SessionContext, LogicalPlan) {
        let ctx = SessionContext::new();
        ctx.register_batch("t", table_batch()).unwrap();
        let plan = ctx.sql(sql).await.unwrap().logical_plan().clone();
        (ctx, plan)
    }

    #[tokio::test]
    async fn prepare_holds_param_conjunct_and_pushes_free_conjunct() {
        let (ctx, plan) = ctx_and_plan("SELECT * FROM t WHERE x < $max AND region = 'EU'").await;
        let prepared = prepare(plan, &ctx, &PartialEvalPolicy::default())
            .await
            .unwrap();
        let display = prepared.plan.display_indent().to_string();

        assert!(display.contains(PE_HOLD_UDF_NAME), "{display}");
        assert!(display.contains("region"), "{display}");
        assert!(
            display.find(PE_HOLD_UDF_NAME) < display.rfind("TableScan").or(Some(usize::MAX)),
            "{display}"
        );
    }

    #[tokio::test]
    async fn prepare_preserves_normal_filter_pushdown() {
        let (ctx, plan) = ctx_and_plan(
            "SELECT * FROM (SELECT region, SUM(x) AS total FROM t GROUP BY region) q WHERE region = 'EU'",
        )
        .await;
        let optimized = bake_optimize(plan, &ctx).await.unwrap();
        let display = optimized.display_indent().to_string();
        let filter_pos = display.find("Filter").expect("filter in plan");
        let aggregate_pos = display.find("Aggregate").expect("aggregate in plan");
        assert!(filter_pos > aggregate_pos, "{display}");
    }

    #[tokio::test]
    async fn prepare_infers_placeholder_type_inside_marker() {
        let (ctx, plan) = ctx_and_plan("SELECT * FROM t WHERE x < $max").await;
        let prepared = prepare(plan, &ctx, &PartialEvalPolicy::default())
            .await
            .unwrap();
        let placeholder_types = placeholder_types(&prepared.plan);
        assert_eq!(placeholder_types, vec![Some(DataType::Float64)]);
    }

    #[tokio::test]
    async fn prepare_keeps_now_symbolic_and_held() {
        let (ctx, plan) = ctx_and_plan(
            "SELECT * FROM t WHERE now() > timestamp '2000-01-01 00:00:00' AND region = 'EU'",
        )
        .await;
        let prepared = prepare(plan, &ctx, &PartialEvalPolicy::default())
            .await
            .unwrap();
        let display = prepared.plan.display_indent().to_string();

        assert!(display.contains(PE_HOLD_UDF_NAME), "{display}");
        assert!(display.to_ascii_lowercase().contains("now()"), "{display}");
        assert!(
            display.contains("now() > TimestampNanosecond(946684800000000000, None)"),
            "{display}"
        );
    }

    #[tokio::test]
    async fn prepare_reports_applied_fixed_params() {
        let (ctx, plan) = ctx_and_plan("SELECT * FROM t WHERE x < $max AND region = $region").await;
        let mut policy = PartialEvalPolicy::default();
        policy
            .fixed_params
            .push(("max".to_string(), ScalarValue::Float64(Some(2.5))));

        let prepared = prepare(plan, &ctx, &policy).await.unwrap();
        assert_eq!(
            prepared.fixed_params_applied,
            BTreeSet::from(["max".to_string()])
        );
        assert_eq!(
            collect_placeholder_ids(&prepared.plan),
            BTreeSet::from(["$region".to_string()])
        );
    }

    fn placeholder_types(plan: &LogicalPlan) -> Vec<Option<DataType>> {
        let mut types = Vec::new();
        let _ = plan.apply_with_subqueries(|node| {
            for expr in node.expressions() {
                let _ = expr.apply(|candidate| {
                    if let Expr::Placeholder(Placeholder { field, .. }) = candidate {
                        types.push(field.as_ref().map(|field| field.data_type().clone()));
                    }
                    Ok(TreeNodeRecursion::Continue)
                });
            }
            Ok(TreeNodeRecursion::Continue)
        });
        types
    }
}
