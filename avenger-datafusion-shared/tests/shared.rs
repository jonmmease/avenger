//! A shared input runs once for all its readers, and a cheap input is computed again in place.

use std::sync::Arc;

use avenger_datafusion_shared::{InlineShared, PruneShared, SharedExtensionPlanner, shared};
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
    ctx.add_optimizer_rule(Arc::new(PruneShared));
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

/// The id of the shared node `plan`.
fn id(plan: &LogicalPlan) -> u64 {
    let LogicalPlan::Extension(extension) = plan else {
        panic!("a shared node, not {plan}");
    };
    let node = extension.node.as_any();
    node.downcast_ref::<avenger_datafusion_shared::SharedNode>()
        .expect("a shared node")
        .id
}

#[tokio::test]
async fn equal_inputs_give_equal_ids_and_volatile_inputs_ids_of_their_own() -> Result<()> {
    let ctx = context().await?;
    let plan = |sql: &'static str| {
        let ctx = ctx.clone();
        async move {
            Ok::<_, datafusion::error::DataFusionError>(ctx.sql(sql).await?.into_unoptimized_plan())
        }
    };
    // Two plans of one query, as two analyses of one script make them, share an id.
    let sql = "SELECT column1 * 2 AS r FROM t WHERE column1 > 1";
    let (a, b) = (shared(plan(sql).await?), shared(plan(sql).await?));
    assert_eq!(id(&a), id(&b));
    assert_eq!(
        format!("{}", a.display_indent()),
        format!("{}", b.display_indent())
    );
    // Another input has another id.
    let other = shared(plan("SELECT column1 * 3 AS r FROM t").await?);
    assert_ne!(id(&a), id(&other));
    // Two copies of a volatile input never share a run.
    let sql = "SELECT random() AS r FROM t";
    let (a, b) = (shared(plan(sql).await?), shared(plan(sql).await?));
    assert_ne!(id(&a), id(&b));
    Ok(())
}

/// The names of the columns of the input of each shared node of `plan`, optimized in `ctx`, and
/// the rows of `plan`.
async fn shared_columns(
    ctx: &SessionContext,
    plan: LogicalPlan,
) -> Result<(Vec<Vec<String>>, String)> {
    use datafusion::common::tree_node::{TreeNode, TreeNodeRecursion};
    let optimized = ctx.state().optimize(&plan)?;
    let mut columns = Vec::new();
    optimized.apply(|node| {
        if let LogicalPlan::Extension(extension) = node {
            let node = extension.node.as_any();
            if let Some(node) = node.downcast_ref::<avenger_datafusion_shared::SharedNode>() {
                let names = node
                    .input
                    .schema()
                    .fields()
                    .iter()
                    .map(|f| f.name().clone());
                columns.push(names.collect());
            }
        }
        Ok(TreeNodeRecursion::Continue)
    })?;
    let rows = ctx.execute_logical_plan(plan).await?.collect().await?;
    Ok((columns, pretty_format_batches(&rows)?.to_string()))
}

/// A shared input of three columns, which an aggregate computes, so it stays shared.
async fn three_columns(ctx: &SessionContext) -> Result<LogicalPlan> {
    let sql = "SELECT column1 AS a, column1 * 2 AS b, column1 * 3 AS c FROM t GROUP BY column1";
    Ok(shared(ctx.sql(sql).await?.into_unoptimized_plan()))
}

#[tokio::test]
async fn a_shared_input_computes_the_columns_that_its_readers_read() -> Result<()> {
    use datafusion::functions_aggregate::expr_fn::sum;
    use datafusion::prelude::col;
    let ctx = context().await?;
    let input = three_columns(&ctx).await?;
    // One reader takes `a`, and the other sums `b`, so no reader reads `c`.
    let left = LogicalPlanBuilder::from(input.clone())
        .project(vec![col("a")])?
        .build()?;
    let right = LogicalPlanBuilder::from(input)
        .aggregate(
            Vec::<datafusion::logical_expr::Expr>::new(),
            vec![sum(col("b")).alias("total")],
        )?
        .build()?;
    let plan = LogicalPlanBuilder::from(left)
        .cross_join(right)?
        .sort(vec![col("a").sort(true, false)])?
        .build()?;
    let (columns, rows) = shared_columns(&ctx, plan).await?;
    assert!(!columns.is_empty());
    for names in &columns {
        assert_eq!(names, &["a", "b"]);
    }
    let expected = "\
+---+-------+
| a | total |
+---+-------+
| 1 | 12    |
| 2 | 12    |
| 3 | 12    |
+---+-------+";
    assert_eq!(rows, expected);
    Ok(())
}

#[tokio::test]
async fn a_reader_that_passes_its_columns_up_keeps_every_column() -> Result<()> {
    use datafusion::optimizer::{OptimizerContext, OptimizerRule};
    use datafusion::prelude::{col, lit};
    let ctx = context().await?;
    let input = three_columns(&ctx).await?;
    // A filter passes every column up, so the rule can't tell what the plan above it reads.
    let left = LogicalPlanBuilder::from(input.clone())
        .filter(col("b").gt(lit(2)))?
        .project(vec![col("a")])?
        .build()?;
    let right = LogicalPlanBuilder::from(input)
        .project(vec![col("a").alias("other")])?
        .build()?;
    let plan = LogicalPlanBuilder::from(left).cross_join(right)?.build()?;
    let pruned = PruneShared.rewrite(plan, &OptimizerContext::new())?;
    assert!(!pruned.transformed, "{}", pruned.data.display_indent());
    Ok(())
}
