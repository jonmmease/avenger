//! `stack` against stacks computed with the code of Vega's stack transform, and its errors.

mod common;

use std::sync::Arc;

use avenger_datafusion_transforms::session;
use common::{error, fixture, num, register, same};
use datafusion::arrow::array::{AsArray, Float64Array, Int64Array};
use datafusion::arrow::datatypes::Float64Type;
use datafusion::common::Result;

#[tokio::test]
async fn vega() -> Result<()> {
    let cases = fixture("stack.json");
    let ctx = session(true);
    for (c, case) in cases.as_array().unwrap().iter().enumerate() {
        let rows = case["rows"].as_array().unwrap();
        let col = |k: &str| -> Vec<i64> { rows.iter().map(|r| r[k].as_i64().unwrap()).collect() };
        register(
            &ctx,
            "c",
            vec![
                (
                    "i",
                    Arc::new(Int64Array::from_iter_values(0..rows.len() as i64)),
                ),
                ("g", Arc::new(Int64Array::from(col("g")))),
                ("s", Arc::new(Int64Array::from(col("s")))),
                (
                    "v",
                    Arc::new(Float64Array::from(
                        rows.iter().map(|r| num(&r["v"])).collect::<Vec<_>>(),
                    )),
                ),
            ],
        )?;
        let offset = case["offset"].as_str().unwrap();
        let window = if case["groups"] == 1 {
            "ORDER BY s"
        } else {
            "PARTITION BY g ORDER BY s"
        };
        let sql = format!(
            "SELECT i, stack(v, 'offset', '{offset}') OVER ({window}) AS y FROM c ORDER BY i"
        );
        let mut i = 0;
        for b in ctx.sql(&sql).await?.collect().await? {
            let y = b.column(1).as_struct();
            let (y0, y1) = (
                y.column(0).as_primitive::<Float64Type>(),
                y.column(1).as_primitive::<Float64Type>(),
            );
            for j in 0..b.num_rows() {
                let e = case["stacks"][i].as_array().unwrap();
                let (e0, e1) = (num(&e[0]).unwrap(), num(&e[1]).unwrap());
                assert!(
                    same(y0.value(j), e0) && same(y1.value(j), e1),
                    "case {c} ({} {offset}) row {i}: [{:?}, {:?}], Vega [{e0:?}, {e1:?}]",
                    case["kind"],
                    y0.value(j),
                    y1.value(j)
                );
                i += 1;
            }
        }
        assert_eq!(i, rows.len());
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
