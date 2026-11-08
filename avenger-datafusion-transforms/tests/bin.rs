//! `bin` and `bin_params` against the bins of vega-statistics, with and without the rules,
//! and the rules against the window evaluator.

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use avenger_datafusion_transforms::session;
use common::{error, fixture, num, register, same};
use datafusion::arrow::array::{Array, ArrayRef, AsArray, Float64Array, Int64Array};
use datafusion::arrow::datatypes::Float64Type;
use datafusion::common::Result;
use datafusion::prelude::SessionContext;
use serde_json::Value;

fn sql_value(v: &Value) -> String {
    match v {
        Value::Bool(b) => b.to_string(),
        Value::Array(a) => format!(
            "[{}]",
            a.iter()
                .map(|x| format!("{:?}", num(x).unwrap()))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => format!("{:?}", num(v).unwrap()),
    }
}

/// `, 'name', value` for each option, without `extent` if `with_extent` is false.
fn sql_options(options: &serde_json::Map<String, Value>, with_extent: bool) -> String {
    options
        .iter()
        .filter(|(k, _)| with_extent || k.as_str() != "extent")
        .map(|(k, v)| format!(", '{k}', {}", sql_value(v)))
        .collect()
}

/// The cases where the bins differ from Vega's: tiny extents, where V8's `Math.pow`, which
/// is not correctly rounded, picks another step than `libm::pow`.
const POW_CASES: [usize; 9] = [94, 190, 192, 193, 195, 199, 200, 202, 203];

#[tokio::test]
async fn vega() -> Result<()> {
    let cases = fixture("bin.json");
    let cases = cases.as_array().unwrap();
    let mut differ = BTreeSet::new();
    for rules in [false, true] {
        let ctx = session(rules);
        for (c, case) in cases.iter().enumerate() {
            let options = case["options"].as_object().unwrap();
            let values: Vec<f64> = case["values"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| num(v).unwrap())
                .collect();
            register(
                &ctx,
                "v",
                vec![
                    (
                        "i",
                        Arc::new(Int64Array::from_iter_values(0..values.len() as i64)) as ArrayRef,
                    ),
                    ("x", Arc::new(Float64Array::from(values))),
                ],
            )?;
            let sql = format!(
                "SELECT i, bin(x{}) OVER () AS b FROM v ORDER BY i",
                sql_options(options, true)
            );
            let mut row = 0;
            for b in ctx.sql(&sql).await?.collect().await? {
                let bins = b.column(1).as_struct();
                let starts = bins.column(0).as_primitive::<Float64Type>();
                let ends = bins.column(1).as_primitive::<Float64Type>();
                for j in 0..b.num_rows() {
                    let expected = case["bins"][row].as_array().unwrap();
                    if !same(starts.value(j), num(&expected[0]).unwrap())
                        || !same(ends.value(j), num(&expected[1]).unwrap())
                    {
                        differ.insert(c);
                    }
                    row += 1;
                }
            }

            // bin_params of the case's extent; it has no `extent` option.
            let extent = case["extent"].as_array().unwrap();
            let sql = format!(
                "SELECT bin_params({:?}, {:?}{}) AS p",
                num(&extent[0]).unwrap(),
                num(&extent[1]).unwrap(),
                sql_options(options, false)
            );
            let batches = ctx.sql(&sql).await?.collect().await?;
            let p = batches[0].column(0).as_struct();
            for (k, name) in ["start", "stop", "step"].iter().enumerate() {
                if !same(
                    p.column(k).as_primitive::<Float64Type>().value(0),
                    num(&case["params"][name]).unwrap(),
                ) {
                    differ.insert(c);
                }
            }
        }
    }
    assert_eq!(differ, BTreeSet::from(POW_CASES));
    Ok(())
}

/// The bins of a query as sorted rows of (i, start, end), None for NULL.
async fn bins(ctx: &SessionContext, sql: &str) -> Result<Vec<(i64, Option<(f64, f64)>)>> {
    let mut out = Vec::new();
    for b in ctx.sql(sql).await?.collect().await? {
        let i = b
            .column(0)
            .as_primitive::<datafusion::arrow::datatypes::Int64Type>();
        let s = b.column(1).as_struct();
        let (lo, hi) = (
            s.column(0).as_primitive::<Float64Type>(),
            s.column(1).as_primitive::<Float64Type>(),
        );
        for j in 0..b.num_rows() {
            out.push((
                i.value(j),
                s.is_valid(j).then(|| (lo.value(j), hi.value(j))),
            ));
        }
    }
    out.sort_by_key(|r| r.0);
    Ok(out)
}

/// The rule gives the window evaluator's bins: partitions with NULL keys, NULL values, an
/// explicit extent, and several calls in one window.
#[tokio::test]
async fn rules_match_the_evaluator() -> Result<()> {
    let (rules, plain) = (session(true), session(false));
    let n = 200;
    let g: Vec<Option<i64>> = (0..n)
        .map(|i| if i % 7 == 0 { None } else { Some(i % 3) })
        .collect();
    let x: Vec<Option<f64>> = (0..n)
        .map(|i| {
            if i % 11 == 0 {
                None
            } else {
                Some((i * 37 % 101) as f64 * (1.0 + (i % 3) as f64))
            }
        })
        .collect();
    for ctx in [&rules, &plain] {
        register(
            ctx,
            "t",
            vec![
                ("i", Arc::new(Int64Array::from_iter_values(0..n))),
                ("g", Arc::new(Int64Array::from(g.clone()))),
                ("x", Arc::new(Float64Array::from(x.clone()))),
            ],
        )?;
    }
    for sql in [
        "SELECT i, bin(x) OVER () AS b FROM t",
        "SELECT i, bin(x, 'maxbins', 7) OVER (PARTITION BY g) AS b FROM t",
        "SELECT i, bin(x, 'extent', [0, 50], 'maxbins', 5) OVER (PARTITION BY g) AS b FROM t",
        "SELECT i, bin(x, 'maxbins', 30) OVER (PARTITION BY g) AS b, bin(x, 'maxbins', 4) OVER (PARTITION BY g) AS c FROM t",
        "SELECT i, b FROM (SELECT i, bin(x) OVER (PARTITION BY g) AS b FROM t) WHERE b IS NOT NULL",
    ] {
        assert_eq!(bins(&rules, sql).await?, bins(&plain, sql).await?, "{sql}");
    }

    // A histogram, through StructGroupSplit.
    let sql = "SELECT b, count(*) AS n FROM (SELECT bin(x) OVER (PARTITION BY g) AS b FROM t WHERE x IS NOT NULL) GROUP BY b ORDER BY b['start'], b['end']";
    let a = rules.sql(sql).await?.collect().await?;
    let b = plain.sql(sql).await?.collect().await?;
    assert_eq!(
        datafusion::arrow::util::pretty::pretty_format_batches(&a)?.to_string(),
        datafusion::arrow::util::pretty::pretty_format_batches(&b)?.to_string()
    );
    Ok(())
}

#[tokio::test]
async fn errors() -> Result<()> {
    let ctx = session(true);
    register(
        &ctx,
        "t",
        vec![("x", Arc::new(Float64Array::from(vec![1.0, 2.0])))],
    )?;
    for (args, message) in [
        ("x, 'maxbins'", "pairs of a name and a value"),
        ("x, 'bins', 3", "unknown bin option 'bins'"),
        ("x, 'maxbins', 'many'", "maxbins"),
        ("x, 'base', 1", "'base' must be a number greater than 1"),
        ("x, 'base', 0.5", "'base' must be a number greater than 1"),
        ("x, 'maxbins', -3", "'maxbins' must be a positive number"),
        ("x, 'divide', [0]", "'divide' must be a positive number"),
        ("x, 'step', -1", "'step' must be a positive number"),
        ("x, 'steps', [5, 1]", "'steps' must be in increasing order"),
        (
            "x, 'extent', [1, 'NaN'::double]",
            "'extent' must be two finite numbers",
        ),
    ] {
        let sql = format!("SELECT bin({args}) OVER () FROM t");
        let e = error(&ctx, &sql)
            .await
            .unwrap_or_else(|| panic!("no error: {sql}"));
        assert!(e.contains(message), "{sql}: {e}");
    }
    for (sql, message) in [
        (
            "SELECT bin(x) OVER (ORDER BY x) FROM t",
            "the window of bin cannot have an ORDER BY",
        ),
        (
            "SELECT bin(x) OVER (ROWS BETWEEN 1 PRECEDING AND 1 FOLLOWING) FROM t",
            "it cannot have a frame",
        ),
        (
            "SELECT bin_params(0, 1, 'extent', [0, 1]) FROM t",
            "bin_params takes the extent as its first two arguments",
        ),
    ] {
        let e = error(&ctx, sql)
            .await
            .unwrap_or_else(|| panic!("no error: {sql}"));
        assert!(e.contains(message), "{sql}: {e}");
    }
    Ok(())
}

/// The extent is that of the finite values: NaN takes no part, and an infinite value bins to
/// an infinite bin.
#[tokio::test]
async fn non_finite_values() -> Result<()> {
    for rules in [true, false] {
        let ctx = session(rules);
        let x = vec![
            Some(0.0),
            Some(10.0),
            Some(f64::NAN),
            Some(f64::INFINITY),
            None,
        ];
        register(
            &ctx,
            "t",
            vec![
                ("i", Arc::new(Int64Array::from_iter_values(0..5))),
                ("x", Arc::new(Float64Array::from(x))),
            ],
        )?;
        let got = bins(&ctx, "SELECT i, bin(x) OVER () AS b FROM t").await?;
        assert_eq!(got[0].1, Some((0.0, 0.5)), "rules {rules}");
        assert_eq!(got[1].1, Some((9.5, 10.0)), "rules {rules}");
        assert!(
            got[2].1.is_some_and(|(a, b)| a.is_nan() && b.is_nan()),
            "rules {rules}"
        );
        assert_eq!(got[3].1.map(|b| b.0), Some(f64::INFINITY), "rules {rules}");
        assert_eq!(got[4].1, None, "rules {rules}");
    }
    Ok(())
}

/// A window node with bin and other functions, and a volatile input, which the rule leaves
/// to the window evaluator rather than read two times.
#[tokio::test]
async fn mixed_and_volatile() -> Result<()> {
    let (rules, plain) = (session(true), session(false));
    for ctx in [&rules, &plain] {
        register(
            ctx,
            "t",
            vec![
                ("i", Arc::new(Int64Array::from_iter_values(0..50))),
                (
                    "x",
                    Arc::new(Float64Array::from_iter_values(
                        (0..50).map(|i| (i * 7 % 50) as f64),
                    )),
                ),
            ],
        )?;
    }
    let sql = "SELECT i, bin(x) OVER () AS b, count(*) OVER () AS n FROM t";
    assert_eq!(bins(&rules, sql).await?, bins(&plain, sql).await?);
    let plan = rules
        .sql(&format!("EXPLAIN {sql}"))
        .await?
        .collect()
        .await?;
    let plan = datafusion::arrow::util::pretty::pretty_format_batches(&plan)?.to_string();
    assert!(plan.contains("__bin_apply"), "{plan}");

    // Each row's bin contains its own value, which a second read of random() would not give.
    let sql = "SELECT b['start'] <= r AND r <= b['end'] AS ok FROM (SELECT r, bin(r) OVER () AS b FROM (SELECT random() * 100 AS r FROM t))";
    for batch in rules.sql(sql).await?.collect().await? {
        assert_eq!(batch.column(0).as_boolean().false_count(), 0);
    }
    Ok(())
}
