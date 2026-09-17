//! Run with `cargo run --release -p avenger-datafusion-dataflow --example parameterized_graph`.
use std::sync::Arc;

use avenger_datafusion_dataflow::{
    arrow::{
        array::StringArray,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
        util::pretty::pretty_format_batches,
    },
    datafusion::{
        common::ScalarValue,
        functions_aggregate::expr_fn::{max, sum},
        logical_expr::{col, scalar_subquery, Expr, JoinType, LogicalPlanBuilder},
        prelude::{CsvReadOptions, SessionContext},
    },
    DataflowBuilder, DataflowResult, Result, Runtime, RuntimeConfig, ScalarOutput, TableOutput,
    TableSnapshot, TableStore,
};

#[tokio::main]
async fn main() -> Result<()> {
    let sales_schema = Arc::new(Schema::new(vec![
        Field::new("region", DataType::Utf8, false),
        Field::new("price", DataType::Int64, false),
        Field::new("quantity", DataType::Int64, false),
    ]));
    let region_schema = Arc::new(Schema::new(vec![Field::new(
        "region",
        DataType::Utf8,
        false,
    )]));

    let mut graph = DataflowBuilder::new();
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/data/sales.csv");
    let sales = graph.add_plan(
        "sales",
        SessionContext::new()
            .read_csv(
                path,
                CsvReadOptions::new().schema(&sales_schema).has_header(true),
            )
            .await?
            .into_unoptimized_plan(),
    )?;
    let selected_regions = graph.table_input("selected_regions", region_schema.clone())?;
    let fraction = graph.scalar_input("fraction", DataType::Float64)?;

    let totals = graph.add_plan(
        "totals",
        LogicalPlanBuilder::from(sales.plan_ref())
            .project(vec![
                col("region"),
                (col("price") * col("quantity")).alias("revenue"),
            ])?
            .aggregate(
                vec![col("region")],
                vec![sum(col("revenue")).alias("total")],
            )?
            .build()?,
    )?;
    let maximum_table = graph.add_plan(
        "maximum_table",
        LogicalPlanBuilder::from(totals.plan_ref())
            .aggregate(Vec::<Expr>::new(), vec![max(col("total")).alias("maximum")])?
            .build()?,
    )?;
    let maximum = graph.add_scalar(
        "maximum",
        scalar_subquery(Arc::new(maximum_table.plan_ref())),
    )?;
    let threshold = graph.add_scalar("threshold", maximum.expr_ref() * fraction.expr_ref())?;
    let visible = graph.add_plan(
        "visible",
        LogicalPlanBuilder::from(totals.plan_ref())
            .join(
                selected_regions.plan_ref(),
                JoinType::LeftSemi,
                (vec!["region"], vec!["region"]),
                None,
            )?
            .filter(col("total").gt(threshold.expr_ref()))?
            .sort(vec![col("region").sort(true, false)])?
            .build()?,
    )?;
    let rows_output = graph.table_output("visible_rows", &visible)?;
    let threshold_output = graph.scalar_output("threshold", &threshold)?;
    let definition = graph.finish()?;

    let runtime = Runtime::new(RuntimeConfig::default())?;
    let prepared = runtime.prepare(&definition).await?;
    println!("{}", prepared.explain());

    let selection = RecordBatch::try_new(
        region_schema.clone(),
        vec![Arc::new(StringArray::from(vec!["East", "West", "North"]))],
    )?;
    let selection_store = TableStore::new(TableSnapshot::from_batches(
        region_schema.clone(),
        vec![selection],
    )?);
    let inputs = prepared
        .inputs()
        .table(&selected_regions, selection_store.snapshot())?
        .scalar(&fraction, ScalarValue::Float64(Some(0.5)))?
        .finish()?;

    let first = prepared
        .query(&[rows_output], &[threshold_output], &inputs)
        .await?;
    show(
        "Fraction 0.5, three selected regions",
        &first,
        rows_output,
        threshold_output,
    )?;
    assert_eq!(first.table(&rows_output)?.num_rows(), 2);
    assert_eq!(
        first.scalar(&threshold_output)?,
        &ScalarValue::Float64(Some(100.0))
    );

    let next_inputs = inputs
        .edit()
        .scalar(&fraction, ScalarValue::Float64(Some(0.8)))?
        .finish()?;
    let second = prepared
        .query(&[rows_output], &[threshold_output], &next_inputs)
        .await?;
    show(
        "Fraction 0.8, same tables",
        &second,
        rows_output,
        threshold_output,
    )?;
    assert_eq!(second.table(&rows_output)?.num_rows(), 1);

    let west = RecordBatch::try_new(
        region_schema.clone(),
        vec![Arc::new(StringArray::from(vec!["West"]))],
    )?;
    selection_store.replace(TableSnapshot::from_batches(region_schema, vec![west])?)?;
    let west_inputs = inputs
        .edit()
        .table(&selected_regions, selection_store.snapshot())?
        .finish()?;
    let third = prepared
        .query(&[rows_output], &[threshold_output], &west_inputs)
        .await?;
    show(
        "Fraction 0.5, only West selected",
        &third,
        rows_output,
        threshold_output,
    )?;
    assert_eq!(third.table(&rows_output)?.num_rows(), 1);
    println!("The original inputs still hold the original selection snapshot.");
    println!("Resident named results skip their prerequisites. Cache misses still create physical plans.");
    Ok(())
}

fn show(
    label: &str,
    result: &DataflowResult,
    rows: TableOutput,
    threshold: ScalarOutput,
) -> Result<()> {
    println!("{label}: threshold={}", result.scalar(&threshold)?);
    println!("{}", pretty_format_batches(result.table(&rows)?.batches())?);
    println!(
        "Executed: {:?}, physical plans: {}, cache hits: {}, source executions: {}\n",
        result.report().executed_nodes,
        result.report().physical_plans,
        result.report().cache_hits,
        result.report().source_executions
    );
    Ok(())
}
