use std::sync::Arc;

use avenger_datafusion_dataflow::{
    arrow::{
        array::Int64Array,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
        util::pretty::pretty_format_batches,
    },
    datafusion::{
        functions_aggregate::expr_fn::sum,
        logical_expr::{col, Expr, LogicalPlanBuilder},
    },
    DataflowBuilder, Result, Runtime, TableSnapshot, TableStore,
};

#[tokio::main]
async fn main() -> Result<()> {
    let schema = Arc::new(Schema::new(vec![Field::new(
        "value",
        DataType::Int64,
        false,
    )]));
    let batch = |values: Vec<i64>| {
        RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(values))])
    };
    let mut builder = DataflowBuilder::new();
    let source = builder.table_input("source", schema.clone())?;
    let total = builder.add_plan(
        "total",
        LogicalPlanBuilder::from(source.plan_ref())
            .aggregate(Vec::<Expr>::new(), vec![sum(col("value")).alias("total")])?
            .build()?,
    )?;
    let output = builder.table_output("total", &total)?;
    let runtime = Runtime::new(Default::default())?;
    let flow = runtime.prepare(&builder.finish()?).await?;
    let store = TableStore::new(TableSnapshot::empty(schema.clone()));

    let first_snapshot = store.append_batch(batch(vec![1, 2])?)?;
    let first_inputs = flow.inputs().table(&source, first_snapshot)?.finish()?;
    let first = flow.query(&[output], &[], &first_inputs).await?;
    println!(
        "First snapshot:\n{}",
        pretty_format_batches(first.table(&output)?.batches())?
    );

    let second_snapshot = store.append_batches(vec![batch(vec![3])?, batch(vec![4, 5])?])?;
    println!(
        "Second snapshot contains {} rows in {} batches",
        second_snapshot.num_rows(),
        second_snapshot.batch_iter().count(),
    );
    let second_inputs = first_inputs
        .edit()
        .table(&source, second_snapshot)?
        .finish()?;
    let second = flow.query(&[output], &[], &second_inputs).await?;
    println!(
        "Second snapshot:\n{}",
        pretty_format_batches(second.table(&output)?.batches())?
    );

    let previous = flow.query(&[output], &[], &first_inputs).await?;
    println!(
        "First snapshot again:\n{}",
        pretty_format_batches(previous.table(&output)?.batches())?
    );
    Ok(())
}
