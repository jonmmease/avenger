//! Shared inputs: the plans of `bin` and `lump` read an expensive input through one
//! `SharedExec`, whose readers see one run of it, and a cheap input is computed again in place.

mod common;

use std::sync::Arc;

use avenger_datafusion_transforms::bin::BinOptions;
use avenger_datafusion_transforms::lump::{self, Keep, LumpCall, Nulls};
use common::{bins, register, session, view};
use datafusion::arrow::array::{ArrayRef, Float64Array, Int64Array};
use datafusion::arrow::util::pretty::pretty_format_batches;
use datafusion::common::{Column, Result};
use datafusion::prelude::{SessionContext, col};

fn context() -> Result<SessionContext> {
    let ctx = session();
    register(
        &ctx,
        "t",
        vec![
            (
                "g",
                Arc::new(Int64Array::from(vec![1, 1, 2, 2, 3])) as ArrayRef,
            ),
            (
                "x",
                Arc::new(Float64Array::from(vec![1.0, 2.0, 10.0, 20.0, 50.0])),
            ),
        ],
    )?;
    Ok(ctx)
}

/// The physical plan and the rows of `sql`.
async fn run(ctx: &SessionContext, sql: &str) -> Result<(String, String)> {
    let explain = ctx.sql(&format!("EXPLAIN {sql}")).await?.collect().await?;
    let explain = pretty_format_batches(&explain)?.to_string();
    let rows = pretty_format_batches(&ctx.sql(sql).await?.collect().await?)?.to_string();
    Ok((explain, rows))
}

#[tokio::test]
async fn an_aggregate_is_shared_by_its_readers() -> Result<()> {
    let ctx = context()?;
    let input = "SELECT g, sum(x) AS x FROM t GROUP BY g";
    bins(&ctx, "binned", input, "x", &[], BinOptions::default()).await?;
    let (explain, rows) = run(&ctx, "SELECT g, x, b FROM binned ORDER BY g").await?;
    assert!(explain.contains("SharedExec: id="), "{explain}");
    let want = [
        "+---+------+--------------------------+",
        "| g | x    | b                        |",
        "+---+------+--------------------------+",
        "| 1 | 3.0  | {start: 0.0, end: 5.0}   |",
        "| 2 | 30.0 | {start: 30.0, end: 35.0} |",
        "| 3 | 50.0 | {start: 45.0, end: 50.0} |",
        "+---+------+--------------------------+",
    ]
    .join("\n");
    assert_eq!(rows, want);
    Ok(())
}

#[tokio::test]
async fn a_filtered_scan_of_a_memory_table_is_computed_again() -> Result<()> {
    let ctx = context()?;
    let input = "SELECT g, x FROM t WHERE x > 1";
    bins(&ctx, "binned", input, "x", &[], BinOptions::default()).await?;
    let (explain, rows) = run(&ctx, "SELECT count(b) AS n FROM binned").await?;
    assert!(!explain.contains("SharedExec"), "{explain}");
    assert!(rows.contains("| 4 |"), "{rows}");
    Ok(())
}

#[tokio::test]
async fn the_readers_of_a_lump_share_its_input() -> Result<()> {
    let ctx = context()?;
    let input = "SELECT g, sum(x) AS x FROM t GROUP BY g";
    view(&ctx, "lumped", input, |input| {
        let call = LumpCall {
            category: col("g"),
            partition: vec![],
            weight: None,
            keep: Keep::First(1),
            other: "Other".to_string(),
            null_category: Nulls::Keep,
        };
        lump::plan(input, call, Column::from_name("l"))
    })
    .await?;
    let (explain, rows) = run(&ctx, "SELECT g, l['other'] AS other FROM lumped ORDER BY g").await?;
    assert!(explain.contains("SharedExec: id="), "{explain}");
    let want = [
        "+---+-------+",
        "| g | other |",
        "+---+-------+",
        "| 1 |       |",
        "| 2 | Other |",
        "| 3 | Other |",
        "+---+-------+",
    ]
    .join("\n");
    assert_eq!(rows, want);
    Ok(())
}
