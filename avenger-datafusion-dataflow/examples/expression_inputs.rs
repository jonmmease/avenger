//! Run with `cargo run -p avenger-datafusion-dataflow --example expression_inputs`.
use avenger_datafusion_dataflow::{
    arrow::{
        datatypes::{DataType, Field, Schema},
        util::pretty::pretty_format_batches,
    },
    datafusion::{
        logical_expr::{col, lit, LogicalPlanBuilder},
        prelude::{CsvReadOptions, SessionContext},
    },
    DataflowBuilder, Result, Runtime,
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    let schema = Schema::new(vec![
        Field::new("region", DataType::Utf8, false),
        Field::new("price", DataType::Int64, false),
        Field::new("quantity", DataType::Int64, false),
    ]);
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/data/sales.csv");
    let plan = SessionContext::new()
        .read_csv(path, CsvReadOptions::new().schema(&schema))
        .await?
        .into_unoptimized_plan();
    let mut flow = DataflowBuilder::new();
    let sales = flow.add_plan("sales", plan)?;
    let selection = flow.expr_input("selection", DataType::Boolean)?;
    let measure = flow.expr_input("measure", DataType::Int64)?;
    let visible = flow.add_plan(
        "visible",
        LogicalPlanBuilder::from(sales.plan_ref())
            .filter(selection.expr_ref())?
            .project(vec![col("region"), measure.expr_ref().alias("value")])?
            .build()?,
    )?;
    let expected_schema = Arc::new(visible.schema().as_arrow().clone());
    let output = flow.table_output("rows", &visible)?;
    let prepared = Runtime::new(Default::default())?
        .prepare(&flow.finish()?)
        .await?;
    let a = prepared
        .inputs()
        .expr(&selection, col("price").gt_eq(lit(40_i64)))?
        .expr(&measure, col("price") * col("quantity"))?
        .finish()?;
    let b = a
        .edit()
        .expr(&selection, col("quantity").gt_eq(lit(3_i64)))?
        .expr(&measure, col("price"))?
        .finish()?;
    for (index, (label, inputs)) in [
        ("A: revenue for prices >= 40", &a),
        ("B: price for quantity >= 3", &b),
        ("A again", &a),
    ]
    .into_iter()
    .enumerate()
    {
        let result = prepared.query(&[output], &[], inputs).await?;
        let table = result.table(&output)?;
        assert_eq!(table.num_rows(), 3);
        assert_eq!(table.schema(), &expected_schema);
        let values = table
            .batches()
            .iter()
            .flat_map(|batch| {
                batch
                    .column(1)
                    .as_any()
                    .downcast_ref::<avenger_datafusion_dataflow::arrow::array::Int64Array>()
                    .unwrap()
                    .values()
                    .to_vec()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            values,
            if index == 1 {
                vec![50, 30, 10]
            } else {
                vec![120, 80, 150]
            }
        );
        assert_eq!(result.report().source_executions, usize::from(index == 0));
        assert_eq!(result.report().physical_plans, [2, 1, 0][index]);
        println!("{label}\n{}", pretty_format_batches(table.batches())?);
        println!(
            "source executions={}, physical plans={}, cache hits={}\n",
            result.report().source_executions,
            result.report().physical_plans,
            result.report().cache_hits
        );
    }
    Ok(())
}
