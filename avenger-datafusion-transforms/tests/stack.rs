//! `stack`: the stacks of each offset, and the error of a value that is not a number. A test
//! builds the window function of [`stack::window`] in Rust over the plan of a query.

mod common;

use avenger_datafusion_transforms::stack::{self, StackOffset};
use common::{exec, rows, session, view};
use datafusion::common::Result;
use datafusion::logical_expr::{ExprFunctionExt, LogicalPlanBuilder};
use datafusion::prelude::{col, lit};

/// The stacks of each offset: layers in the order of `ORDER BY`, negative values stacked
/// down from zero, no layer for NULL or NaN, and normalized and centered stacks of the
/// absolute values.
#[tokio::test]
async fn offsets() -> Result<()> {
    let ctx = session();
    let table = "CREATE TABLE c AS SELECT * FROM (VALUES (0, 1, 1, 1.0), (1, 1, 2, 2.0), \
        (2, 1, 3, -3.0), (3, 2, 1, 4.0), (4, 2, 2, NULL), (5, 2, 3, 6.0), \
        (6, 2, 4, 'NaN'::double)) AS c(i, x, s, v)";
    exec(&ctx, table).await?;
    // The offset, then the layer of each row of `c` in the order of `i`: `start end`, or `-`.
    let cases = [
        ("zero", "0 1, 1 3, 0 -3, 0 4, -, 4 10, -"),
        ("normalize", "0 1/6, 1/6 0.5, 0.5 1, 0 0.4, -, 0.4 1, -"),
        ("center", "-3 -2, -2 0, 0 3, -5 -1, -, -1 5, -"),
    ];
    for (offset, want) in cases {
        let window = stack::window(StackOffset::from_name(offset).expect("an offset"))
            .call(vec![col("v")])
            .partition_by(vec![col("x")])
            .order_by(vec![col("s").sort(true, false)])
            .build()?;
        view(&ctx, "y", "SELECT * FROM c", |plan| {
            LogicalPlanBuilder::from(plan)
                .window(vec![window.alias("y")])?
                .build()
        })
        .await?;
        let sql = "SELECT i, CASE WHEN y IS NULL THEN '-' \
            ELSE concat(y['start'], ' ', y['end']) END FROM y";
        // A number as Rust prints it, and 1/6 as `1/6`.
        let short = |v: &str| match v.parse::<f64>() {
            Ok(v) if (v - 1.0 / 6.0).abs() < 1e-12 => "1/6".to_string(),
            Ok(v) => v.to_string(),
            Err(_) => v.to_string(),
        };
        let layers = rows(&ctx, sql).await?.into_iter();
        let layers = layers.map(|r| r[1].split(' ').map(short).collect::<Vec<_>>().join(" "));
        assert_eq!(layers.collect::<Vec<_>>().join(", "), want, "{offset}");
    }
    Ok(())
}

/// `stack` stacks numbers only.
#[tokio::test]
async fn errors() -> Result<()> {
    let ctx = session();
    let window = stack::window(StackOffset::Zero).call(vec![lit("a")]);
    let e = (ctx.sql("SELECT 2.0 AS v").await?)
        .window(vec![window.alias("y")])
        .expect_err("a string value");
    let message = "stack stacks numbers, not Utf8";
    assert!(e.to_string().contains(message), "{e}");
    Ok(())
}
