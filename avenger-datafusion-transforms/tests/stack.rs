//! `stack`: the stacks of each offset, and the error of a value that is not a number. A test
//! builds the window function of [`stack::window`] in Rust over a DataFrame of a table.

mod common;

use std::sync::Arc;

use avenger_datafusion_transforms::stack::{self, StackOffset};
use common::{register, session};
use datafusion::arrow::array::{Array, AsArray, Float64Array, Int64Array};
use datafusion::arrow::datatypes::Float64Type;
use datafusion::common::Result;
use datafusion::logical_expr::ExprFunctionExt;
use datafusion::prelude::{col, lit};

/// A layer: its start and its end, or None.
type Layer = Option<(f64, f64)>;

/// The stacks of each offset: layers in the order of `ORDER BY`, negative values stacked
/// down from zero, no layer for NULL or NaN, and normalized and centered stacks of the
/// absolute values.
#[tokio::test]
async fn offsets() -> Result<()> {
    let ctx = session();
    register(
        &ctx,
        "c",
        vec![
            ("i", Arc::new(Int64Array::from_iter_values(0..7))),
            ("x", Arc::new(Int64Array::from(vec![1, 1, 1, 2, 2, 2, 2]))),
            ("s", Arc::new(Int64Array::from(vec![1, 2, 3, 1, 2, 3, 4]))),
            (
                "v",
                Arc::new(Float64Array::from(vec![
                    Some(1.0),
                    Some(2.0),
                    Some(-3.0),
                    Some(4.0),
                    None,
                    Some(6.0),
                    Some(f64::NAN),
                ])),
            ),
        ],
    )?;
    let cases: [(&str, [Layer; 7]); 3] = [
        (
            "zero",
            [
                Some((0.0, 1.0)),
                Some((1.0, 3.0)),
                Some((0.0, -3.0)),
                Some((0.0, 4.0)),
                None,
                Some((4.0, 10.0)),
                None,
            ],
        ),
        (
            "normalize",
            [
                Some((0.0, 1.0 / 6.0)),
                Some((1.0 / 6.0, 0.5)),
                Some((0.5, 1.0)),
                Some((0.0, 0.4)),
                None,
                Some((0.4, 1.0)),
                None,
            ],
        ),
        (
            "center",
            [
                Some((-3.0, -2.0)),
                Some((-2.0, 0.0)),
                Some((0.0, 3.0)),
                Some((-5.0, -1.0)),
                None,
                Some((-1.0, 5.0)),
                None,
            ],
        ),
    ];
    for (offset, want) in cases {
        let window = stack::window(StackOffset::from_name(offset).expect("an offset"))
            .call(vec![col("v")])
            .partition_by(vec![col("x")])
            .order_by(vec![col("s").sort(true, false)])
            .build()?;
        let batches = ctx
            .table("c")
            .await?
            .window(vec![window.alias("y")])?
            .sort(vec![col("i").sort(true, false)])?
            .select_columns(&["y"])?
            .collect()
            .await?;
        let mut got = Vec::new();
        for b in batches {
            let y = b.column(0).as_struct();
            let (y0, y1) = (
                y.column(0).as_primitive::<Float64Type>(),
                y.column(1).as_primitive::<Float64Type>(),
            );
            got.extend(
                (0..b.num_rows()).map(|j| y.is_valid(j).then(|| (y0.value(j), y1.value(j)))),
            );
        }
        assert_eq!(got.len(), 7);
        for (g, w) in got.iter().zip(&want) {
            let close = match (g, w) {
                (Some(g), Some(w)) => (g.0 - w.0).abs() < 1e-12 && (g.1 - w.1).abs() < 1e-12,
                (None, None) => true,
                _ => false,
            };
            assert!(close, "{offset}: {got:?}, want {want:?}");
        }
    }
    Ok(())
}

/// `stack` stacks numbers only.
#[tokio::test]
async fn errors() -> Result<()> {
    let ctx = session();
    register(
        &ctx,
        "e",
        vec![("v", Arc::new(Float64Array::from(vec![2.0])))],
    )?;
    let window = stack::window(StackOffset::Zero).call(vec![lit("a")]);
    let e = ctx
        .table("e")
        .await?
        .window(vec![window.alias("y")])
        .expect_err("a string value");
    assert!(
        e.to_string().contains("stack stacks numbers, not Utf8"),
        "{e}"
    );
    Ok(())
}
