//! `stack`: the stacks of each offset, and its errors.

mod common;

use std::sync::Arc;

use common::{error, register, session};
use datafusion::arrow::array::{Array, AsArray, Float64Array, Int64Array};
use datafusion::arrow::datatypes::Float64Type;
use datafusion::common::Result;

/// The stacks of each offset: layers in the order of `ORDER BY`, negative values stacked
/// down from zero, no layer for NULL or NaN, and normalized and centered stacks of the
/// absolute values.
/// A layer: its start and its end, or None.
type Layer = Option<(f64, f64)>;

#[tokio::test]
async fn offsets() -> Result<()> {
    let ctx = session(true);
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
        let sql = format!(
            "SELECT i, stack(v, 'offset', '{offset}') OVER (PARTITION BY x ORDER BY s) AS y FROM c ORDER BY i"
        );
        let mut got = Vec::new();
        for b in ctx.sql(&sql).await?.collect().await? {
            let y = b.column(1).as_struct();
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

#[tokio::test]
async fn errors() -> Result<()> {
    let ctx = session(true);
    ctx.sql("CREATE TABLE e AS VALUES (1, 2.0)")
        .await?
        .collect()
        .await?;
    for (args, message) in [
        (
            "column2, 'offset', 'middle'",
            "must be 'zero', 'center', or 'normalize', not middle",
        ),
        ("column2, 'offset'", "pairs of a name and a value"),
        (
            "column2, 'baseline', 'zero'",
            "unknown stack option 'baseline'",
        ),
        (
            "column2, 'offset', 'zero', 'offset', 'center'",
            "the stack option 'offset' is given twice",
        ),
        (
            "column2, 'offset', column1",
            "stack options must be constants",
        ),
        ("'a'", "stack stacks numbers, not Utf8"),
    ] {
        let sql = format!("SELECT stack({args}) OVER () FROM e");
        let e = error(&ctx, &sql)
            .await
            .unwrap_or_else(|| panic!("no error: {sql}"));
        assert!(e.contains(message), "{sql}: {e}");
    }
    Ok(())
}
