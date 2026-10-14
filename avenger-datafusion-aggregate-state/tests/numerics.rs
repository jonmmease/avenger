mod common;

use common::{compare, context, materialize, rows};
use datafusion::{
    arrow::{
        array::{Float64Array, Int32Array},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    common::{Result, ScalarValue},
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
async fn centered_moments_preserve_small_spread_at_large_offsets() -> Result<()> {
    let offsets: Vec<f64> = (0..1003).map(|i| (i % 11) as f64 - 5.).collect();
    let n = offsets.len() as f64;
    let mean = offsets.iter().sum::<f64>() / n;
    let m2 = offsets.iter().map(|x| (x - mean).powi(2)).sum::<f64>();
    let ctx = context(4)?;
    numbers(
        &ctx,
        &offsets.iter().map(|x| Some(1e12 + x)).collect::<Vec<_>>(),
        &(0..1003)
            .map(|i| if i < 997 { i % 13 } else { 13 })
            .collect::<Vec<_>>(),
    )?;
    for (family, reference) in [
        ("varPop", m2 / n),
        ("varSamp", m2 / (n - 1.)),
        ("stddevPop", (m2 / n).sqrt()),
        ("stddevSamp", (m2 / (n - 1.)).sqrt()),
    ] {
        ctx.deregister_table("states")?;
        materialize(
            &ctx,
            "states",
            &format!("SELECT cell, {family}State(x) AS s FROM numbers GROUP BY cell"),
        )
        .await?;
        let result = ctx
            .sql(&format!("SELECT {family}Merge(s) FROM states"))
            .await?
            .collect()
            .await?;
        let ScalarValue::Float64(Some(value)) = rows(&result)?[0][0] else {
            panic!("expected a finite moment");
        };
        // At 1e12, one input ULP is about 1.2e-4. This absolute bound checks
        // centered-state accuracy against the original small integer offsets.
        assert!(
            (value - reference).abs() < 5e-4,
            "{family}: {value} != {reference}"
        );
    }
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
        for (family, native) in [
            ("sum", "sum"),
            ("min", "min"),
            ("max", "max"),
            ("avg", "avg"),
            ("varPop", "var_pop"),
            ("varSamp", "var_samp"),
            ("stddevPop", "stddev_pop"),
            ("stddevSamp", "stddev_samp"),
        ] {
            ctx.deregister_table("states")?;
            materialize(
                &ctx,
                "states",
                &format!("SELECT cell, {family}State(x) AS s FROM numbers GROUP BY cell"),
            )
            .await?;
            compare(
                &ctx,
                &format!("SELECT cell, {family}Merge(s) AS v FROM states GROUP BY cell"),
                &format!("SELECT cell, {native}(x) AS v FROM numbers GROUP BY cell"),
            )
            .await?;
            // Native grouped extrema use finite initial bounds. For an all-infinite
            // cell, the exported state consequently differs from a scalar scan.
            let expected = if matches!(family, "min" | "max") {
                format!("SELECT {native}(v) AS v FROM (SELECT cell, {native}(x) AS v FROM numbers GROUP BY cell)")
            } else {
                format!("SELECT {native}(x) AS v FROM numbers")
            };
            compare(
                &ctx,
                &format!("SELECT {family}Merge(s) AS v FROM states"),
                &expected,
            )
            .await?;
            compare(
                &ctx,
                &format!("SELECT {family}Finalize(s) AS v FROM states"),
                &expected,
            )
            .await?;
            ctx.deregister_table("global_state")?;
            materialize(
                &ctx,
                "global_state",
                &format!("SELECT {family}State(x) AS s FROM numbers"),
            )
            .await?;
            compare(
                &ctx,
                &format!("SELECT {family}Finalize(s) AS v FROM global_state"),
                &format!("SELECT {native}(x) AS v FROM numbers"),
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
            &format!("SELECT cell, sumState({expr}) AS s FROM t GROUP BY cell"),
        )
        .await?;
        compare(
            &ctx,
            &format!("SELECT sumMerge(s) AS v FROM {table}"),
            &format!("SELECT sum({expr}) AS v FROM t"),
        )
        .await?;
        compare(
            &ctx,
            &format!("SELECT cell, sumMerge(s) AS v FROM {table} GROUP BY cell ORDER BY cell"),
            &format!("SELECT cell, sum({expr}) AS v FROM t GROUP BY cell ORDER BY cell"),
        )
        .await?;
    }
    materialize(
        &ctx,
        "empty_decimal",
        "SELECT avgState(CAST(x AS DECIMAL(12,2))) AS s FROM t WHERE false",
    )
    .await?;
    compare(
        &ctx,
        "SELECT avgFinalize(s) AS v FROM empty_decimal",
        "SELECT avg(CAST(x AS DECIMAL(12,2))) AS v FROM t WHERE false",
    )
    .await?;
    compare(
        &ctx,
        "SELECT 1 AS one_group, avgMerge(s) AS v FROM empty_decimal GROUP BY one_group",
        "SELECT 1 AS one_group, avg(CAST(NULL AS DECIMAL(12,2))) AS v FROM t GROUP BY one_group",
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn decimal_average_overflow_propagates_native_errors() -> Result<()> {
    let ctx = context(2)?;
    let value = "CAST('99999999999999999999999999999999999999' AS DECIMAL(38,0))";
    materialize(&ctx, "huge", &format!("SELECT {value} AS x")).await?;
    assert!(ctx
        .sql("SELECT avg(x) FROM huge")
        .await?
        .collect()
        .await
        .is_err());
    materialize(&ctx, "state", "SELECT avgState(x) AS s FROM huge").await?;
    for sql in [
        "SELECT avgMerge(s) FROM state",
        "SELECT avgFinalize(s) FROM state",
    ] {
        assert!(
            ctx.sql(sql).await?.collect().await.is_err(),
            "accepted {sql}"
        );
    }
    Ok(())
}
