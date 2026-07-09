#![deny(missing_docs)]

//! Logical-plan partial evaluation for DataFusion plans.
//!
//! This crate folds deterministic, parameter-independent subtrees of a
//! DataFusion [`LogicalPlan`] into in-memory tables. Params and nondeterministic
//! work stay symbolic in the residual plan.
//!
//! The primary entry point is [`partial_evaluate`]. It accepts a logical plan,
//! a [`SessionContext`], and a [`PartialEvalPolicy`]. The crate first binds any
//! fixed params, wraps placeholder- or now-bearing predicate conjuncts in a
//! volatile internal marker so DataFusion keeps them high during bake
//! optimization, runs the analyzer and optimizer without a query execution
//! start time, and then walks the optimized plan bottom-up. Maximal foldable
//! subtrees are executed once and spliced back as `MemTable` scans with
//! projections that restore the original logical schema.
//!
//! The returned residual is still a DataFusion logical plan. Callers can bind
//! the remaining params and let the runtime optimizer plan it for the target
//! environment. The accompanying [`BakeReport`] records what was baked, what
//! was skipped, source table names folded away, fixed params that applied, and
//! live params still present in the residual.

mod analyze;
mod fold;
mod optimize;
mod params;
mod policy;

use datafusion::logical_expr::LogicalPlan;
use datafusion::prelude::SessionContext;

pub use policy::{
    BakeReport, BakedSubtree, PartialEvalOutput, PartialEvalPolicy, SkipReason, SkippedSubtree,
};

/// Partially evaluate a single logical plan.
pub async fn partial_evaluate(
    plan: LogicalPlan,
    ctx: &SessionContext,
    policy: &PartialEvalPolicy,
) -> datafusion::error::Result<PartialEvalOutput> {
    fold::partial_evaluate_impl(plan, ctx, policy).await
}

/// Partially evaluate a set of logical plans with a shared bake registry.
pub async fn partial_evaluate_set(
    plans: Vec<LogicalPlan>,
    ctx: &SessionContext,
    policy: &PartialEvalPolicy,
) -> datafusion::error::Result<(Vec<LogicalPlan>, BakeReport)> {
    fold::partial_evaluate_set_impl(plans, ctx, policy).await
}

#[cfg(test)]
mod tests {
    use datafusion::prelude::{SessionContext, lit};

    use super::*;

    #[tokio::test]
    async fn smoke_partial_eval_executes() {
        let ctx = SessionContext::new();
        let plan = ctx
            .read_empty()
            .unwrap()
            .select(vec![lit(1_i64).alias("one")])
            .unwrap()
            .logical_plan()
            .clone();

        let output = partial_evaluate(plan, &ctx, &PartialEvalPolicy::default())
            .await
            .unwrap();
        let batches = datafusion::dataframe::DataFrame::new(ctx.state(), output.residual)
            .collect()
            .await
            .unwrap();

        assert_eq!(output.report.baked.len(), 1);
        assert_eq!(
            batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
            1
        );
    }
}
