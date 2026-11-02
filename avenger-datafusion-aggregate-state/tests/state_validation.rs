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
    materialize(&ctx, "states", "SELECT sum_state(x) AS sum_s, min_state(x) AS min_s, var_samp_state(x) AS var_s, stddev_samp_state(x) AS std_s FROM t").await?;
    for sql in [
        "SELECT min_merge(sum_s) FROM states",
        "SELECT sum_merge_state(min_s) FROM states",
        "SELECT stddev_samp_merge(var_s) FROM states",
        "SELECT var_samp_merge_state(std_s) FROM states",
    ] {
        let error = ctx.sql(sql).await.unwrap_err();
        assert!(error.to_string().contains("state family"), "{error}");
    }
    Ok(())
}

#[tokio::test]
async fn rejects_invalid_identity_metadata_and_required_payload_values() -> Result<()> {
    let ctx = context(2)?;
    let batches = materialize(&ctx, "source_state", "SELECT var_pop_state(x) AS s FROM t").await?;
    let outer = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<StructArray>()
        .unwrap();
    // A state without its signature, and a state of another DataFusion version.
    for (i, kind) in ["missing", "engine"].iter().enumerate() {
        let mut field = outer.fields()[0].as_ref().clone();
        if *kind == "missing" {
            field = field.with_metadata(HashMap::new());
        } else {
            let mut metadata = field.metadata().clone();
            let signature = metadata
                .get_mut("datafusion-aggregate-state:signature")
                .unwrap();
            *signature = signature.replace(datafusion::DATAFUSION_VERSION, "53.0.0");
            field = field.with_metadata(metadata);
        }
        let state =
            StructArray::try_new(vec![Arc::new(field)].into(), outer.columns().to_vec(), None)?;
        let name = format!("bad{i}");
        register_state(&ctx, &name, state)?;
        assert!(
            ctx.sql(&format!("SELECT var_pop_merge(s) FROM {name}"))
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
        .sql("SELECT var_pop_merge(s) FROM bad_values")
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
        "SELECT avg_state(CAST(x AS DECIMAL(12,2))) AS s FROM t",
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
        "SELECT avg_merge(s) FROM bad",
        "SELECT avg_merge_state(s) FROM bad",
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
        "SELECT cell, avg_state(CAST(x AS DECIMAL(12,2))) AS s FROM t GROUP BY cell",
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
        "SELECT avg_merge(renamed) AS v FROM (SELECT s AS renamed FROM restored)",
        "SELECT avg(CAST(x AS DECIMAL(12,2))) AS v FROM t",
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn state_types_are_independent_of_expression_alias_and_skip_missing_states() -> Result<()> {
    let ctx = context(2)?;
    let a = ctx.sql("SELECT avg_state(x) AS a FROM t").await?;
    let b = ctx.sql("SELECT avg_state(x + 1) AS b FROM t").await?;
    assert_eq!(
        a.schema().field(0).data_type(),
        b.schema().field(0).data_type()
    );
    materialize(
        &ctx,
        "states",
        "SELECT cell, count_state() AS n, avg_state(x) AS a FROM t GROUP BY cell",
    )
    .await?;
    compare(
        &ctx,
        "SELECT count_merge(CASE WHEN false THEN n END) AS v FROM states",
        "SELECT count(*) AS v FROM t WHERE false",
    )
    .await?;
    Ok(())
}
