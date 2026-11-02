#![allow(dead_code)]

use datafusion::{
    arrow::{
        array::{BooleanArray, Float64Array, Int32Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    common::{Result, ScalarValue},
    datasource::MemTable,
    prelude::{SessionConfig, SessionContext},
};
use std::sync::Arc;

pub fn context(partitions: usize) -> Result<SessionContext> {
    let mut ctx = SessionContext::new_with_config(
        SessionConfig::new()
            .with_target_partitions(partitions)
            .with_batch_size(3),
    );
    avenger_datafusion_aggregate_state::register_all(&mut ctx)?;
    let schema = Arc::new(Schema::new(vec![
        Field::new("g", DataType::Utf8, false),
        Field::new("cell", DataType::Int32, false),
        Field::new("x", DataType::Float64, true),
        Field::new("keep", DataType::Boolean, true),
    ]));
    let groups = [
        "a", "a", "a", "a", "a", "a", "b", "b", "nulls", "nulls", "one",
    ];
    let cells = [0, 0, 0, 1, 2, 2, 0, 1, 0, 1, 1];
    let values = [
        Some(2.),
        Some(4.),
        Some(6.),
        Some(18.),
        Some(22.),
        None,
        Some(-3.),
        Some(7.),
        None,
        None,
        Some(12.),
    ];
    let keep = [
        Some(true),
        Some(false),
        None,
        Some(true),
        Some(false),
        Some(true),
        Some(true),
        Some(true),
        Some(true),
        Some(false),
        Some(true),
    ];
    let mut batches = vec![vec![]; partitions];
    for start in (0..values.len()).step_by(3) {
        let end = (start + 3).min(values.len());
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(StringArray::from(groups[start..end].to_vec())),
                Arc::new(Int32Array::from(cells[start..end].to_vec())),
                Arc::new(Float64Array::from(values[start..end].to_vec())),
                Arc::new(BooleanArray::from(keep[start..end].to_vec())),
            ],
        )?;
        batches[(start / 3) % partitions].push(batch);
    }
    ctx.register_table("t", Arc::new(MemTable::try_new(schema, batches)?))?;
    Ok(ctx)
}

pub async fn materialize(ctx: &SessionContext, name: &str, sql: &str) -> Result<Vec<RecordBatch>> {
    let frame = ctx.sql(sql).await?;
    let schema = Arc::new(frame.schema().as_arrow().clone());
    let batches = frame.collect().await?;
    ctx.register_table(
        name,
        Arc::new(MemTable::try_new(schema, vec![batches.clone()])?),
    )?;
    Ok(batches)
}

pub async fn compare(ctx: &SessionContext, actual: &str, expected: &str) -> Result<()> {
    eprintln!("comparing {actual} against {expected}");
    let actual = ctx.sql(actual).await?;
    let expected = ctx.sql(expected).await?;
    assert_eq!(
        actual.schema().as_arrow(),
        expected.schema().as_arrow(),
        "{actual:?}\n{expected:?}"
    );
    assert_results(&actual.collect().await?, &expected.collect().await?, 1e-10)?;
    Ok(())
}

pub fn rows(batches: &[RecordBatch]) -> Result<Vec<Vec<ScalarValue>>> {
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

pub fn assert_results(
    actual: &[RecordBatch],
    expected: &[RecordBatch],
    tolerance: f64,
) -> Result<()> {
    let actual = rows(actual)?;
    let expected = rows(expected)?;
    assert_eq!(actual.len(), expected.len());
    for (a, b) in actual.iter().zip(&expected) {
        assert_eq!(a.len(), b.len());
        for (a, b) in a.iter().zip(b) {
            match (a, b) {
                (ScalarValue::Float64(Some(x)), ScalarValue::Float64(Some(y))) => {
                    assert!(
                        x == y
                            || x.is_nan() && y.is_nan()
                            || x.is_finite()
                                && y.is_finite()
                                && (x - y).abs() <= tolerance * (1. + y.abs()),
                        "{x} != {y}"
                    );
                }
                _ => assert_eq!(a, b),
            }
        }
    }
    Ok(())
}
