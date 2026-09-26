//! Run with `cargo run -p avenger-datafusion-dataflow --example additional_dataflow`.
use avenger_datafusion_dataflow::{
    arrow::{
        array::{Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        util::pretty::pretty_format_batches,
    },
    datafusion::{
        common::ScalarValue,
        functions_aggregate::expr_fn::{count, sum},
        logical_expr::{col, lit, scalar_subquery, Expr, LogicalPlanBuilder},
        prelude::{CsvReadOptions, SessionContext},
    },
    DataflowBuilder, Result, Runtime, TableSnapshot,
};
use std::{collections::BTreeMap, sync::Arc};

fn counts(table: &TableSnapshot) -> BTreeMap<String, i64> {
    let mut values = BTreeMap::new();
    for batch in table.batches() {
        let carriers = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let counts = batch
            .column(1)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        for row in 0..batch.num_rows() {
            values.insert(carriers.value(row).into(), counts.value(row));
        }
    }
    values
}

#[tokio::main]
async fn main() -> Result<()> {
    let schema = Schema::new(vec![
        Field::new("carrier", DataType::Utf8, false),
        Field::new("dep_delay", DataType::Int64, false),
        Field::new("distance", DataType::Int64, false),
    ]);
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/data/flights.csv");
    let plan = SessionContext::new()
        .read_csv(path, CsvReadOptions::new().schema(&schema))
        .await?
        .into_unoptimized_plan();
    let mut flow = DataflowBuilder::new();
    let flights = flow.add_plan("flights", plan)?;
    let flights_out = flow.table_output("flights", &flights)?;
    let fixed_filter = flow.expr_input("fixed_filter", DataType::Boolean)?;
    let minimum_count = flow.scalar_input("minimum_count", DataType::Int64)?;
    let total = flow.add_scalar(
        "total",
        scalar_subquery(Arc::new(
            LogicalPlanBuilder::from(flights.plan_ref())
                .aggregate(Vec::<Expr>::new(), vec![count(lit(1_i64))])?
                .build()?,
        )),
    )?;
    let total_out = flow.scalar_output("total", &total)?;
    let runtime = Runtime::new(Default::default())?;
    let base = runtime.prepare(&flow.finish()?).await?;
    let base_inputs = base
        .inputs()
        .expr(&fixed_filter, lit(true))?
        .scalar(&minimum_count, 1_i64.into())?
        .finish()?;
    let warm = base.query(&[], &[total_out], &base_inputs).await?;
    assert_eq!(warm.scalar(&total_out)?, &ScalarValue::from(12_i64));
    println!(
        "Base warm-up: {} source execution\n",
        warm.report().source_executions
    );

    // Build this definition when delay becomes the active interaction.
    let mut additional = DataflowBuilder::with_base(&base.interface());
    let flights = additional.import_table("flights", &flights_out)?;
    let total = additional.import_scalar("total", &total_out)?;
    let total = additional.scalar_output("total", &total)?;
    let active_filter = additional.expr_input("active_filter", DataType::Boolean)?;
    let preaggregate = additional.add_plan(
        "delay_to_carriers",
        LogicalPlanBuilder::from(flights.plan_ref())
            .filter(fixed_filter.expr_ref())?
            .aggregate(
                vec![col("carrier"), col("dep_delay")],
                vec![count(lit(1_i64)).alias("n")],
            )?
            .build()?,
    )?;
    let carrier_counts = additional.add_plan(
        "carrier_counts",
        LogicalPlanBuilder::from(preaggregate.plan_ref())
            .filter(active_filter.expr_ref())?
            .aggregate(vec![col("carrier")], vec![sum(col("n")).alias("count")])?
            .filter(col("count").gt_eq(minimum_count.expr_ref()))?
            .build()?,
    )?;
    let output = additional.table_output("counts", &carrier_counts)?;
    // Keep a direct query to verify the generated pre-aggregation's results.
    let direct = additional.add_plan(
        "direct",
        LogicalPlanBuilder::from(flights.plan_ref())
            .filter(fixed_filter.expr_ref().and(active_filter.expr_ref()))?
            .aggregate(vec![col("carrier")], vec![count(lit(1_i64)).alias("count")])?
            .filter(col("count").gt_eq(minimum_count.expr_ref()))?
            .build()?,
    )?;
    let direct_out = additional.table_output("direct", &direct)?;
    let extension = base.prepare_extension(&additional.finish()?).await?;
    let first = extension
        .inputs()
        .expr(
            &active_filter,
            col("dep_delay").between(lit(10_i64), lit(20_i64)),
        )?
        .finish()?;
    let next = first
        .edit()
        .expr(
            &active_filter,
            col("dep_delay").between(lit(15_i64), lit(30_i64)),
        )?
        .finish()?;
    let shorter = base_inputs
        .edit()
        .expr(&fixed_filter, col("distance").lt(lit(800_i64)))?
        .finish()?;
    for (label, bindings, active, expected_plans) in [
        ("Delay 10 through 20", &base_inputs, &first, 2),
        ("Delay 15 through 30", &base_inputs, &next, 1),
        ("Back to delay 10 through 20", &base_inputs, &first, 0),
        (
            "Distance below 800, delay 10 through 20",
            &shorter,
            &first,
            2,
        ),
    ] {
        let result = extension
            .query(&[output], &[total], bindings, active)
            .await?;
        assert_eq!(result.report().physical_plans, expected_plans);
        assert_eq!(result.report().source_executions, 0);
        assert_eq!(result.scalar(&total)?, &ScalarValue::from(12_i64));
        let direct = extension
            .query(&[direct_out], &[], bindings, active)
            .await?;
        assert_eq!(
            counts(result.table(&output)?),
            counts(direct.table(&direct_out)?)
        );
        println!(
            "{label}\n{}",
            pretty_format_batches(result.table(&output)?.batches())?
        );
        println!(
            "Executed: {:?}\nPhysical plans: {}, cache hits: {}\n",
            result.report().executed_nodes,
            result.report().physical_plans,
            result.report().cache_hits
        );
    }
    Ok(())
}
