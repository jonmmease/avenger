mod common;

use avenger_datafusion_dataflow::{
    datafusion::logical_expr::{col, LogicalPlanBuilder},
    CachePolicy, DataflowBuilder, ExecutionConfig, Runtime, RuntimeConfig,
};

/// A scan splits one large input batch into slices of `batch_size` rows, and a
/// projection passes the column through. Each slice must be charged for the
/// rows it holds, not for the whole array it shares with the other slices.
#[tokio::test]
async fn streamed_slices_are_charged_for_their_rows() -> Result<(), Box<dyn std::error::Error>> {
    let n = 100_000usize;
    let mut graph = DataflowBuilder::new();
    let source = graph.table_input("source", common::schema())?;
    let projected = graph.add_plan(
        "projected",
        LogicalPlanBuilder::from(source.plan_ref())
            .project(vec![col("value")])?
            .build()?,
    )?;
    let rows = graph.table_output("rows", &projected)?;
    // Four times the column: room for its rows, not for every slice charged
    // the whole column.
    let runtime = Runtime::new(RuntimeConfig {
        execution: ExecutionConfig {
            max_materialized_bytes: 4 * n * std::mem::size_of::<i64>(),
            ..Default::default()
        },
        cache: CachePolicy::Disabled,
        ..Default::default()
    })?;
    let prepared = runtime.prepare(&graph.finish()?).await?;
    let values: Vec<i64> = (0..n as i64).collect();
    let inputs = prepared
        .inputs()
        .table(&source, common::snapshot(&values))?
        .finish()?;
    let result = prepared.query(&[rows.clone()], &[], &inputs).await?;
    assert_eq!(result.table(&rows)?.num_rows(), n);
    Ok(())
}
