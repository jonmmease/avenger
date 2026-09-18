use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{Float64Array, Int32Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
        util::pretty::print_batches,
    },
    common::{Result, ScalarValue},
    datasource::MemTable,
    logical_expr::LogicalPlan,
    prelude::{SessionConfig, SessionContext},
};

pub fn context() -> Result<SessionContext> {
    let ctx = SessionContext::new_with_config(SessionConfig::new().with_target_partitions(2));
    let schema = Arc::new(Schema::new(vec![
        Field::new("airline", DataType::Utf8, false),
        Field::new("delay_cell", DataType::Int32, false),
        Field::new("distance", DataType::Float64, true),
    ]));
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(StringArray::from(vec![
                "A", "A", "A", "A", "A", "B", "B", "B", "B", "B", "B",
            ])),
            Arc::new(Int32Array::from(vec![0, 0, 1, 2, 2, 0, 1, 1, 1, 2, 2])),
            Arc::new(Float64Array::from(vec![
                Some(100.),
                Some(300.),
                Some(500.),
                Some(700.),
                None,
                Some(200.),
                Some(400.),
                Some(600.),
                Some(800.),
                Some(900.),
                Some(1100.),
            ])),
        ],
    )?;
    ctx.register_table(
        "flights",
        Arc::new(MemTable::try_new(schema, vec![vec![batch]])?),
    )?;
    Ok(ctx)
}

pub async fn materialize(ctx: &SessionContext, name: &str, plan: LogicalPlan) -> Result<()> {
    let frame = ctx.execute_logical_plan(plan).await?;
    let schema = Arc::new(frame.schema().as_arrow().clone());
    let batches = frame.collect().await?;
    let rows: usize = batches.iter().map(RecordBatch::num_rows).sum();
    ctx.register_table(name, Arc::new(MemTable::try_new(schema, vec![batches])?))?;
    println!("Materialized {name}: {rows} rows. Subsequent scans read these batches.\n");
    Ok(())
}

pub async fn show(
    ctx: &SessionContext,
    title: &str,
    plan: LogicalPlan,
) -> Result<Vec<RecordBatch>> {
    println!("{title}");
    let batches = ctx.execute_logical_plan(plan).await?.collect().await?;
    print_batches(&batches)?;
    println!();
    Ok(batches)
}

pub async fn verify(
    ctx: &SessionContext,
    actual: &[RecordBatch],
    native: LogicalPlan,
) -> Result<()> {
    let expected = ctx.execute_logical_plan(native).await?.collect().await?;
    fn rows(batches: &[RecordBatch]) -> Result<Vec<Vec<ScalarValue>>> {
        batches
            .iter()
            .flat_map(|b| {
                (0..b.num_rows()).map(|i| {
                    b.columns()
                        .iter()
                        .map(|c| ScalarValue::try_from_array(c, i))
                        .collect()
                })
            })
            .collect()
    }
    let actual = rows(actual)?;
    let expected = rows(&expected)?;
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(&expected) {
        assert_eq!(actual.len(), expected.len());
        for (a, b) in actual.iter().zip(expected) {
            match (a, b) {
                (ScalarValue::Float64(Some(a)), ScalarValue::Float64(Some(b))) => {
                    assert!((a - b).abs() <= 1e-10 * (1. + b.abs()), "{a} != {b}");
                }
                _ => assert_eq!(a, b),
            }
        }
    }
    println!("Verified against aggregation of the original rows.\n");
    Ok(())
}
