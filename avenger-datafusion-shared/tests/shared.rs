//! A shared input runs once for all its readers, and a cheap input is computed again in place.

use std::sync::Arc;

use avenger_datafusion_shared::{InlineShared, SharedExtensionPlanner, shared};
use datafusion::arrow::util::pretty::pretty_format_batches;
use datafusion::catalog::Session;
use datafusion::common::Result;
use datafusion::execution::SessionStateBuilder;
use datafusion::execution::context::QueryPlanner;
use datafusion::logical_expr::{LogicalPlan, LogicalPlanBuilder};
use datafusion::physical_plan::ExecutionPlan;
use datafusion::physical_planner::{DefaultPhysicalPlanner, PhysicalPlanner};
use datafusion::prelude::SessionContext;

#[derive(Debug)]
struct Planner;

#[async_trait::async_trait]
impl QueryPlanner for Planner {
    async fn create_physical_plan(
        &self,
        plan: &LogicalPlan,
        session: &dyn Session,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        DefaultPhysicalPlanner::with_extension_planners(vec![Arc::new(
            SharedExtensionPlanner::default(),
        )])
        .create_physical_plan(plan, session)
        .await
    }
}

async fn context() -> Result<SessionContext> {
    let state = SessionStateBuilder::new()
        .with_default_features()
        .with_query_planner(Arc::new(Planner))
        .build();
    let ctx = SessionContext::new_with_state(state);
    ctx.add_optimizer_rule(Arc::new(InlineShared));
    ctx.sql("CREATE TABLE t AS VALUES (1), (2), (3)")
        .await?
        .collect()
        .await?;
    Ok(ctx)
}

/// The physical plan, and the number of rows and of distinct values of `r`, of the union of two
/// readers of the input `sql`.
async fn union_of_two_readers(ctx: &SessionContext, sql: &str) -> Result<(String, String)> {
    let input = shared(ctx.sql(sql).await?.into_unoptimized_plan());
    let plan = LogicalPlanBuilder::from(input.clone())
        .union(input)?
        .build()?;
    let df = ctx.execute_logical_plan(plan).await?;
    ctx.register_table("readers", df.into_view())?;
    let sql = "SELECT count(*) AS n, count(DISTINCT r) AS distinct FROM readers";
    let explain = ctx.sql(&format!("EXPLAIN {sql}")).await?.collect().await?;
    let rows = ctx.sql(sql).await?.collect().await?;
    ctx.deregister_table("readers")?;
    Ok((
        pretty_format_batches(&explain)?.to_string(),
        pretty_format_batches(&rows)?.to_string(),
    ))
}

#[tokio::test]
async fn the_readers_of_a_volatile_input_read_one_run() -> Result<()> {
    let ctx = context().await?;
    let (explain, rows) = union_of_two_readers(&ctx, "SELECT random() AS r FROM t").await?;
    assert!(explain.contains("SharedExec"), "{explain}");
    // Six rows of three values: each reader reads the same three random numbers.
    assert!(rows.contains("| 6 | 3        |"), "{rows}");
    Ok(())
}

#[tokio::test]
async fn a_filtered_scan_of_a_memory_table_is_computed_again() -> Result<()> {
    let ctx = context().await?;
    let sql = "SELECT column1 AS r FROM t WHERE column1 > 1";
    let (explain, rows) = union_of_two_readers(&ctx, sql).await?;
    assert!(!explain.contains("SharedExec"), "{explain}");
    assert!(rows.contains("| 4 | 2        |"), "{rows}");
    Ok(())
}
