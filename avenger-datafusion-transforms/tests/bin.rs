//! `bin` and `bin_params`: the bins of values and options, the rules against the window
//! evaluator, non-finite values, and errors.

mod common;

use std::sync::Arc;

use common::{error, register, session};
use datafusion::arrow::array::{Array, ArrayRef, AsArray, Float64Array, Int64Array};
use datafusion::arrow::datatypes::Float64Type;
use datafusion::common::Result;
use datafusion::prelude::SessionContext;

/// The bins of values, and the parameters of `bin_params`, for the options of each case.
/// A bin: its start and its end, or None.
type Bin = Option<(f64, f64)>;

#[tokio::test]
async fn expected_bins() -> Result<()> {
    let ctx = session(true);
    register(
        &ctx,
        "v",
        vec![
            (
                "i",
                Arc::new(Int64Array::from_iter_values(0..8)) as ArrayRef,
            ),
            (
                "x",
                Arc::new(Float64Array::from(vec![
                    0.0, 10.0, 20.0, 42.0, 55.0, 60.0, 80.0, 100.0,
                ])),
            ),
        ],
    )?;
    // The options, then the bin of each value of `x`, in order.
    let cases: [(&str, [Bin; 8]); 4] = [
        // 20 bins at most: bins of 5 from 0, and the greatest value in the last bin.
        (
            "",
            [
                Some((0.0, 5.0)),
                Some((10.0, 15.0)),
                Some((20.0, 25.0)),
                Some((40.0, 45.0)),
                Some((55.0, 60.0)),
                Some((60.0, 65.0)),
                Some((80.0, 85.0)),
                Some((95.0, 100.0)),
            ],
        ),
        // 10 bins at most: bins of 10.
        (
            ", 'maxbins', 10",
            [
                Some((0.0, 10.0)),
                Some((10.0, 20.0)),
                Some((20.0, 30.0)),
                Some((40.0, 50.0)),
                Some((50.0, 60.0)),
                Some((60.0, 70.0)),
                Some((80.0, 90.0)),
                Some((90.0, 100.0)),
            ],
        ),
        // An explicit step.
        (
            ", 'step', 25",
            [
                Some((0.0, 25.0)),
                Some((0.0, 25.0)),
                Some((0.0, 25.0)),
                Some((25.0, 50.0)),
                Some((50.0, 75.0)),
                Some((50.0, 75.0)),
                Some((75.0, 100.0)),
                Some((75.0, 100.0)),
            ],
        ),
        // An explicit extent: values outside it have no bin.
        (
            ", 'extent', [20, 80], 'maxbins', 6",
            [
                None,
                None,
                Some((20.0, 30.0)),
                Some((40.0, 50.0)),
                Some((50.0, 60.0)),
                Some((60.0, 70.0)),
                Some((70.0, 80.0)),
                None,
            ],
        ),
    ];
    for (options, want) in cases {
        let got = bins(
            &ctx,
            &format!("SELECT i, bin(x{options}) OVER () AS b FROM v"),
        )
        .await?;
        let got: Vec<Option<(f64, f64)>> = got.into_iter().map(|(_, b)| b).collect();
        assert_eq!(got, want, "bin(x{options})");
    }

    // The parameters of an extent: the start, the end of the last bin, and the step.
    for (call, want) in [
        ("bin_params(0, 100)", (0.0, 100.0, 5.0)),
        ("bin_params(0, 100, 'maxbins', 10)", (0.0, 100.0, 10.0)),
        ("bin_params(0.5, 9.7)", (0.5, 10.0, 0.5)),
        ("bin_params(0, 1, 'step', 0.25)", (0.0, 1.0, 0.25)),
    ] {
        let batches = ctx
            .sql(&format!("SELECT {call} AS p"))
            .await?
            .collect()
            .await?;
        let p = batches[0].column(0).as_struct();
        let f = |k: usize| p.column(k).as_primitive::<Float64Type>().value(0);
        assert_eq!((f(0), f(1), f(2)), want, "{call}");
    }
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

    // A histogram, through StructGroupSplit, which groups by whether a bin is NULL and by its
    // fields, then makes the bin again.
    let explain = rules
        .sql("EXPLAIN SELECT b, count(*) FROM (SELECT bin(x) OVER () AS b FROM t) GROUP BY b")
        .await?
        .collect()
        .await?;
    let explain = datafusion::arrow::util::pretty::pretty_format_batches(&explain)?.to_string();
    assert!(explain.contains("__make_nullable_struct"), "{explain}");
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
        ("x, 'maxbins', 0", "'maxbins' must be a positive number"),
        ("x, 'base', 0", "'base' must be a number greater than 1"),
        ("x, 'step', 0", "'step' must be a positive number"),
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

/// The extent is that of the finite values, and NULL, NaN, and infinite values have no bin.
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
        assert_eq!(got[2].1, None, "rules {rules}");
        assert_eq!(got[3].1, None, "rules {rules}");
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
