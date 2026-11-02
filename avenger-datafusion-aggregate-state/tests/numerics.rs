mod common;

use common::{compare, context, materialize};
use datafusion::{
    arrow::{
        array::{Float64Array, Int32Array},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    common::Result,
    datasource::MemTable,
    prelude::SessionContext,
};
use std::sync::Arc;

fn numbers(ctx: &SessionContext, values: &[Option<f64>], cells: &[i32]) -> Result<()> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, true),
        Field::new("cell", DataType::Int32, false),
    ]));
    let mut partitions = vec![vec![]; 4];
    for (i, start) in (0..values.len()).step_by(37).enumerate() {
        let end = (start + 37).min(values.len());
        partitions[i % 4].push(RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(Float64Array::from(values[start..end].to_vec())),
                Arc::new(Int32Array::from(cells[start..end].to_vec())),
            ],
        )?);
    }
    ctx.register_table("numbers", Arc::new(MemTable::try_new(schema, partitions)?))?;
    Ok(())
}

#[tokio::test]
async fn floating_special_values_follow_native_global_and_grouped_paths() -> Result<()> {
    for values in [
        vec![Some(f64::NAN)],
        vec![Some(f64::INFINITY)],
        vec![Some(f64::NEG_INFINITY)],
        vec![Some(-0.), Some(0.)],
        vec![Some(f64::INFINITY), Some(3.)],
        vec![Some(f64::NEG_INFINITY), Some(f64::INFINITY)],
        vec![Some(f64::NAN), None, Some(4.)],
    ] {
        let ctx = context(1)?;
        numbers(&ctx, &values, &vec![0; values.len()])?;
        for family in [
            "sum",
            "min",
            "max",
            "avg",
            "var_pop",
            "var_samp",
            "stddev_pop",
            "stddev_samp",
        ] {
            ctx.deregister_table("states")?;
            materialize(
                &ctx,
                "states",
                &format!("SELECT cell, {family}_state(x) AS s FROM numbers GROUP BY cell"),
            )
            .await?;
            compare(
                &ctx,
                &format!("SELECT cell, {family}_merge(s) AS v FROM states GROUP BY cell"),
                &format!("SELECT cell, {family}(x) AS v FROM numbers GROUP BY cell"),
            )
            .await?;
            // Native grouped extrema use finite initial bounds. For an all-infinite
            // cell, the exported state consequently differs from a scalar scan.
            let expected = if matches!(family, "min" | "max") {
                format!("SELECT {family}(v) AS v FROM (SELECT cell, {family}(x) AS v FROM numbers GROUP BY cell)")
            } else {
                format!("SELECT {family}(x) AS v FROM numbers")
            };
            compare(
                &ctx,
                &format!("SELECT {family}_merge(s) AS v FROM states"),
                &expected,
            )
            .await?;
            ctx.deregister_table("global_state")?;
            materialize(
                &ctx,
                "global_state",
                &format!("SELECT {family}_state(x) AS s FROM numbers"),
            )
            .await?;
            compare(
                &ctx,
                &format!("SELECT {family}_merge(s) AS v FROM global_state"),
                &format!("SELECT {family}(x) AS v FROM numbers"),
            )
            .await?;
        }
    }
    Ok(())
}

#[tokio::test]
async fn exact_types_numeric_limits_and_decimal_empty_states() -> Result<()> {
    let ctx = context(4)?;
    for (i, expr) in [
        "CAST(x AS BIGINT)",
        "CAST(abs(x) AS BIGINT UNSIGNED)",
        "CAST(x AS REAL)",
        "CAST(x AS DECIMAL(12,2))",
        "CAST(x AS DECIMAL(38,4))",
        "CAST('9223372036854775807' AS BIGINT)",
        "CAST('18446744073709551615' AS BIGINT UNSIGNED)",
    ]
    .iter()
    .enumerate()
    {
        let table = format!("limits_{i}");
        materialize(
            &ctx,
            &table,
            &format!("SELECT cell, sum_state({expr}) AS s FROM t GROUP BY cell"),
        )
        .await?;
        compare(
            &ctx,
            &format!("SELECT sum_merge(s) AS v FROM {table}"),
            &format!("SELECT sum({expr}) AS v FROM t"),
        )
        .await?;
        compare(
            &ctx,
            &format!("SELECT cell, sum_merge(s) AS v FROM {table} GROUP BY cell ORDER BY cell"),
            &format!("SELECT cell, sum({expr}) AS v FROM t GROUP BY cell ORDER BY cell"),
        )
        .await?;
    }
    materialize(
        &ctx,
        "empty_decimal",
        "SELECT avg_state(CAST(x AS DECIMAL(12,2))) AS s FROM t WHERE false",
    )
    .await?;
    compare(
        &ctx,
        "SELECT avg_merge(s) AS v FROM empty_decimal",
        "SELECT avg(CAST(x AS DECIMAL(12,2))) AS v FROM t WHERE false",
    )
    .await?;
    compare(
        &ctx,
        "SELECT 1 AS one_group, avg_merge(s) AS v FROM empty_decimal GROUP BY one_group",
        "SELECT 1 AS one_group, avg(CAST(NULL AS DECIMAL(12,2))) AS v FROM t GROUP BY one_group",
    )
    .await?;
    Ok(())
}
