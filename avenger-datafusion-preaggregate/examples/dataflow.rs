//! Warm a dataflow query, then bind changing predicates without reinstalling it.
use avenger_datafusion_dataflow::{DataflowBuilder, Runtime, TableSnapshot};
use avenger_datafusion_preaggregate::{
    dataflow::Query, FilterQuery, PreaggregatePlanner, QueryPolicy,
};
use datafusion::{
    arrow::{
        array::{Int32Array, StringArray},
        datatypes::DataType,
        record_batch::RecordBatch,
        util::pretty::print_batches,
    },
    functions_aggregate::expr_fn::count,
    logical_expr::{col, lit, LogicalPlanBuilder},
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let batch = RecordBatch::try_from_iter(vec![
        (
            "delay",
            Arc::new(Int32Array::from(vec![0, 10, 20, 30, 40, 50])) as _,
        ),
        (
            "carrier",
            Arc::new(StringArray::from(vec!["AA", "DL", "AA", "UA", "DL", "UA"])) as _,
        ),
    ])?;
    let mut builder = DataflowBuilder::new();
    let rows = builder.table_snapshot(
        "flights",
        TableSnapshot::from_batches(batch.schema(), vec![batch])?,
    )?;
    let fixed = builder.expr_input("other_selections", DataType::Boolean)?;
    let source = LogicalPlanBuilder::from(rows.plan_ref())
        .filter(fixed.expr_ref())?
        .build()?;
    let query = FilterQuery::new(source, |rows| {
        LogicalPlanBuilder::from(rows)
            .aggregate(
                vec![col("carrier")],
                vec![count(lit(1_i64)).alias("flights")],
            )?
            .sort(vec![col("carrier").sort(true, true)])?
            .build()
    })?;
    let query = PreaggregatePlanner::default().prepare(query, vec![col("delay")])?;
    let airlines = Query::install(&mut builder, "airlines", query)?;
    let flow = Runtime::new(Default::default())?
        .prepare(&builder.finish()?)
        .await?;
    let idle = airlines.bind(lit(true))?;
    let inputs = idle
        .apply(flow.inputs().expr(&fixed, lit(true))?)?
        .finish()?;

    if let Some(states) = idle.materialization_output() {
        let result = flow.query(&[states], &[], &inputs).await?;
        println!(
            "Warm-up before any brush: {:?}",
            result.report().executed_nodes
        );
    }

    for (label, lower, upper, other, policy) in [
        ("Brush [0, 30)", 0, 30, lit(true), QueryPolicy::Auto),
        ("Drag [10, 40)", 10, 40, lit(true), QueryPolicy::Auto),
        (
            "Other selection changes to AA",
            10,
            40,
            col("carrier").eq(lit("AA")),
            QueryPolicy::Auto,
        ),
        (
            "Same selection, forced direct",
            10,
            40,
            col("carrier").eq(lit("AA")),
            QueryPolicy::ForceDirect,
        ),
    ] {
        let changing = col("delay")
            .gt_eq(lit(lower))
            .and(col("delay").lt(lit(upper)));
        let binding = airlines.bind_with_policy(changing, policy)?;
        let inputs = binding
            .apply(inputs.edit().expr(&fixed, other)?)?
            .finish()?;
        let result = flow.query(&[binding.output()], &[], &inputs).await?;
        println!("\n{label}: {:?}", binding.diagnostics());
        println!(
            "Executed: {:?}; cache hits: {}",
            result.report().executed_nodes,
            result.report().cache_hits
        );
        print_batches(result.table(&binding.output())?.batches())?;
    }
    Ok(())
}
