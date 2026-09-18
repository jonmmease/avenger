mod common;
use common::{compare, context, materialize};
use datafusion::{
    arrow::{
        array::{Array, ArrayRef, StructArray, UInt64Array},
        datatypes::{Field, Schema},
        ipc::{reader::StreamReader, writer::StreamWriter},
        record_batch::RecordBatch,
    },
    common::Result,
    datasource::MemTable,
    prelude::SessionContext,
};
use std::{collections::HashMap, io::Cursor, sync::Arc};

fn register_state(ctx: &SessionContext, name: &str, state: StructArray) -> Result<()> {
    let schema = Arc::new(Schema::new(vec![Field::new(
        "s",
        state.data_type().clone(),
        false,
    )]));
    let batch = RecordBatch::try_new(Arc::clone(&schema), vec![Arc::new(state)])?;
    ctx.register_table(
        name,
        Arc::new(MemTable::try_new(schema, vec![vec![batch]])?),
    )?;
    Ok(())
}

#[tokio::test]
async fn rejects_cross_family_states_even_with_identical_payload_types() -> Result<()> {
    let ctx = context(2)?;
    materialize(&ctx, "states", "SELECT sumState(x) AS sum_s, minState(x) AS min_s, varSampState(x) AS var_s, stddevSampState(x) AS std_s FROM t").await?;
    for sql in [
        "SELECT minMerge(sum_s) FROM states",
        "SELECT sumFinalize(min_s) FROM states",
        "SELECT stddevSampMerge(var_s) FROM states",
        "SELECT varSampMergeState(std_s) FROM states",
    ] {
        let error = ctx.sql(sql).await.unwrap_err();
        assert!(error.to_string().contains("state family"), "{error}");
    }
    Ok(())
}

#[tokio::test]
async fn rejects_invalid_identity_metadata_and_required_payload_values() -> Result<()> {
    let ctx = context(2)?;
    let batches = materialize(&ctx, "source_state", "SELECT varPopState(x) AS s FROM t").await?;
    let outer = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<StructArray>()
        .unwrap();
    for (i, kind) in [
        "missing", "invalid", "version", "layout", "result", "inputs", "engine",
    ]
    .iter()
    .enumerate()
    {
        let mut field = outer.fields()[0].as_ref().clone();
        match *kind {
            "missing" => field = field.with_metadata(HashMap::new()),
            "invalid" => {
                field = field.with_metadata(HashMap::from([(
                    "datafusion-aggregate-state:signature".into(),
                    "bad json".into(),
                )]))
            }
            "version" => field = field.with_name("varPop_state_v2"),
            "layout" => field = field.with_nullable(true),
            "result" | "inputs" | "engine" => {
                let mut metadata = field.metadata().clone();
                let signature = metadata
                    .get_mut("datafusion-aggregate-state:signature")
                    .unwrap();
                *signature = match *kind {
                    "result" => signature.replace("\"result\":\"Float64\"", "\"result\":\"Int64\""),
                    "inputs" => {
                        signature.replace("\"inputs\":[\"Float64\"]", "\"inputs\":[\"Utf8\"]")
                    }
                    _ => signature.replace("54.1.0", "53.0.0"),
                };
                field = field.with_metadata(metadata);
            }
            _ => unreachable!(),
        }
        let state =
            StructArray::try_new(vec![Arc::new(field)].into(), outer.columns().to_vec(), None)?;
        let name = format!("bad{i}");
        register_state(&ctx, &name, state)?;
        assert!(
            ctx.sql(&format!("SELECT varPopMerge(s) FROM {name}"))
                .await
                .is_err(),
            "accepted {kind}"
        );
    }
    let inner = outer
        .column(0)
        .as_any()
        .downcast_ref::<StructArray>()
        .unwrap();
    let mut columns = inner.columns().to_vec();
    columns[0] = Arc::new(UInt64Array::new_null(inner.len()));
    let payload: ArrayRef = Arc::new(StructArray::try_new(inner.fields().clone(), columns, None)?);
    register_state(
        &ctx,
        "bad_values",
        StructArray::try_new(outer.fields().clone(), vec![payload], None)?,
    )?;
    let error = ctx
        .sql("SELECT varPopMerge(s) FROM bad_values")
        .await?
        .collect()
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("missing required payload"),
        "{error}"
    );
    Ok(())
}

#[tokio::test]
async fn rejects_inconsistent_average_payload_before_decimal_division() -> Result<()> {
    let ctx = context(2)?;
    let batches = materialize(
        &ctx,
        "good",
        "SELECT avgState(CAST(x AS DECIMAL(12,2))) AS s FROM t",
    )
    .await?;
    let outer = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<StructArray>()
        .unwrap();
    let inner = outer
        .column(0)
        .as_any()
        .downcast_ref::<StructArray>()
        .unwrap();
    let mut columns = inner.columns().to_vec();
    columns[0] = Arc::new(UInt64Array::new_null(inner.len()));
    let payload: ArrayRef = Arc::new(StructArray::try_new(inner.fields().clone(), columns, None)?);
    register_state(
        &ctx,
        "bad",
        StructArray::try_new(outer.fields().clone(), vec![payload], None)?,
    )?;
    for sql in [
        "SELECT avgFinalize(s) FROM bad",
        "SELECT avgMerge(s) FROM bad",
        "SELECT avgMergeState(s) FROM bad",
    ] {
        let error = ctx.sql(sql).await?.collect().await.unwrap_err();
        assert!(
            error.to_string().contains("validity are inconsistent"),
            "{error}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn decimal_state_metadata_survives_ipc_into_a_fresh_context() -> Result<()> {
    let ctx = context(3)?;
    let batches = materialize(
        &ctx,
        "states",
        "SELECT cell, avgState(CAST(x AS DECIMAL(12,2))) AS s FROM t GROUP BY cell",
    )
    .await?;
    let mut bytes = Vec::new();
    {
        let mut writer = StreamWriter::try_new(&mut bytes, batches[0].schema().as_ref())?;
        for batch in &batches {
            writer.write(batch)?;
        }
        writer.finish()?;
    }
    let reader = StreamReader::try_new(Cursor::new(bytes), None)?;
    let schema = reader.schema();
    let restored = reader.collect::<std::result::Result<Vec<_>, _>>()?;
    let fresh = context(2)?;
    fresh.register_table(
        "restored",
        Arc::new(MemTable::try_new(schema, vec![restored])?),
    )?;
    compare(
        &fresh,
        "SELECT avgMerge(renamed) AS v FROM (SELECT s AS renamed FROM restored)",
        "SELECT avg(CAST(x AS DECIMAL(12,2))) AS v FROM t",
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn state_types_are_independent_of_expression_alias_and_keep_missing_states_null() -> Result<()>
{
    let ctx = context(2)?;
    let a = ctx.sql("SELECT avgState(x) AS a FROM t").await?;
    let b = ctx.sql("SELECT avgState(x + 1) AS b FROM t").await?;
    assert_eq!(
        a.schema().field(0).data_type(),
        b.schema().field(0).data_type()
    );
    materialize(
        &ctx,
        "states",
        "SELECT cell, countState() AS n, avgState(x) AS a FROM t GROUP BY cell",
    )
    .await?;
    let nulls = ctx.sql("SELECT countFinalize(CASE WHEN false THEN n END), avgFinalize(CASE WHEN false THEN a END) FROM states").await?.collect().await?;
    for batch in nulls {
        for col in batch.columns() {
            assert_eq!(col.null_count(), batch.num_rows());
        }
    }
    compare(
        &ctx,
        "SELECT countMerge(CASE WHEN false THEN n END) AS v FROM states",
        "SELECT count(*) AS v FROM t WHERE false",
    )
    .await?;
    Ok(())
}
