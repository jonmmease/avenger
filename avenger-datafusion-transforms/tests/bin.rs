//! `bin`: the bins of values and options, the window partitions of a call, the placeholder
//! windows of a projection, non-finite values, a volatile input, histograms of bins, and the
//! checks of the options. A test registers the plan of [`bin::plan`] as a view and reads the
//! view with DataFusion's SQL. The test of placeholder windows plans the windows of
//! [`bin::window`] through [`plan_windows`].

mod common;

use std::sync::Arc;

use avenger_datafusion_transforms::bin::{self, BinCall, BinOptions};
use avenger_datafusion_transforms::{OptionError, StructGroupSplit, plan_windows};
use common::{exec, rows, session, view};
use datafusion::common::{Column, Result};
use datafusion::functions_window::expr_fn::row_number;
use datafusion::logical_expr::{Expr, ExprFunctionExt, LogicalPlanBuilder};
use datafusion::prelude::{SessionContext, col};

/// The table `v` of the values that `expected_bins` and `histogram` bin, the last one NULL.
const V: &str = "CREATE TABLE v AS SELECT * FROM (VALUES (0, 0.0), (1, 10.0), (2, 20.0), \
    (3, 42.0), (4, 55.0), (5, 60.0), (6, 80.0), (7, 100.0), (8, NULL)) AS v(i, x)";

/// The table `t` of the partitions of `g`, with a NULL key and a NULL value.
const T: &str = "CREATE TABLE t AS SELECT * FROM (VALUES (0, 1, 0.0), (1, 1, 10.0), \
    (2, 1, 20.0), (3, 2, 100.0), (4, 2, 150.0), (5, 2, 200.0), (6, NULL, 3.0), \
    (7, NULL, 7.0), (8, 1, NULL)) AS t(i, g, x)";

/// The bins of each window partition of `t` by `g`, with at most 10 bins: the NULL key is a
/// partition of its own.
const BY_G: &str = "0 2, 10 12, 18 20, 100 110, 150 160, 190 200, 3 3.5, 6.5 7, -";

/// The bins of the column `b` of the view `view`, in the order of `i`: `start end`, or `-` for
/// no bin, with numbers as Rust prints them.
async fn bins(ctx: &SessionContext, view: &str, b: &str) -> Result<String> {
    let sql = format!(
        "SELECT i, CASE WHEN {b} IS NULL THEN '-' \
         ELSE concat({b}['start'], ' ', {b}['end']) END FROM {view}"
    );
    // A number, such as `5.0`, as Rust prints it, such as `5`.
    let short = |v: &str| v.parse::<f64>().map_or(v.to_string(), |v| v.to_string());
    let bins = rows(ctx, &sql).await?.into_iter();
    let bins = bins.map(|r| r[1].split(' ').map(short).collect::<Vec<_>>().join(" "));
    Ok(bins.collect::<Vec<_>>().join(", "))
}

/// A change of the default options.
type Change = fn(&mut BinOptions);

/// The default options with the change `f`.
fn options(f: Change) -> BinOptions {
    let mut o = BinOptions::default();
    f(&mut o);
    o
}

/// The bins of values for the options of each case, and the parameters of a fractional step.
#[tokio::test]
async fn expected_bins() -> Result<()> {
    let ctx = session();
    exec(&ctx, V).await?;
    let cases: [(Change, &str); 6] = [
        // 20 bins at most: bins of 5 from 0, and the greatest value in the last bin.
        (
            |_| {},
            "0 5, 10 15, 20 25, 40 45, 55 60, 60 65, 80 85, 95 100, -",
        ),
        // 10 bins at most: bins of 10.
        (
            |o| o.maxbins = 10.0,
            "0 10, 10 20, 20 30, 40 50, 50 60, 60 70, 80 90, 90 100, -",
        ),
        // An explicit step.
        (
            |o| o.step = Some(25.0),
            "0 25, 0 25, 0 25, 25 50, 50 75, 50 75, 75 100, 75 100, -",
        ),
        // An explicit extent: values outside it have no bin.
        (
            |o| (o.extent, o.maxbins) = (Some([20.0, 80.0]), 6.0),
            "-, -, 20 30, 40 50, 50 60, 60 70, 70 80, -, -",
        ),
        // An extent that is not nice: bins of 10 from its start, to whole steps past its end.
        (
            |o| (o.extent, o.maxbins, o.nice) = (Some([3.0, 97.0]), 10.0, false),
            "-, 3 13, 13 23, 33 43, 53 63, 53 63, 73 83, 93 103, -",
        ),
        // An anchor moves the bins up to it, so 0 falls below the first bin.
        (
            |o| o.anchor = Some(2.5),
            "-, 7.5 12.5, 17.5 22.5, 37.5 42.5, 52.5 57.5, 57.5 62.5, 77.5 82.5, 97.5 102.5, -",
        ),
    ];
    for (f, want) in cases {
        let options = options(f);
        common::bins(&ctx, "binned", "SELECT * FROM v", "x", &[], options.clone()).await?;
        assert_eq!(bins(&ctx, "binned", "b").await?, want, "{options:?}");
    }

    // A fractional step: the start stays at the minimum, and the end is a whole step past the
    // maximum.
    let p = bin::bin_params(0.5, 9.7, &BinOptions::default());
    assert_eq!((p.start, p.end, p.step), (0.5, 10.0, 0.5));
    Ok(())
}

/// The extent of each window partition: partitions of a column with a NULL key, and of an
/// expression that is not a column.
#[tokio::test]
async fn partitions() -> Result<()> {
    let ctx = session();
    exec(&ctx, T).await?;
    let options = options(|o| o.maxbins = 10.0);
    common::bins(
        &ctx,
        "binned",
        "SELECT * FROM t",
        "x",
        &["g"],
        options.clone(),
    )
    .await?;
    assert_eq!(bins(&ctx, "binned", "b").await?, BY_G);

    // `coalesce(g, 1)` puts the values of the NULL key, 3 and 7, in the partition of 1, whose
    // bins are bins of 2 from 0.
    let partition = common::expr(&ctx, "SELECT * FROM t", "coalesce(g, 1)").await?;
    let call = BinCall {
        value: col("x"),
        partition: vec![partition],
        options,
    };
    view(&ctx, "binned", "SELECT * FROM t", |plan| {
        bin::plan(plan, call, Column::from_name("b"))
    })
    .await?;
    let want = BY_G.replace("3 3.5, 6.5 7", "2 4, 6 8");
    assert_eq!(bins(&ctx, "binned", "b").await?, want);
    Ok(())
}

/// The placeholder windows of a projection: two calls over one partition, which one aggregate
/// serves, among another window function, which `plan_windows` leaves to a window node.
#[tokio::test]
async fn placeholder_windows() -> Result<()> {
    let ctx = session();
    exec(&ctx, T).await?;
    let placeholder = |maxbins: f64| {
        let options = BinOptions {
            maxbins,
            ..Default::default()
        };
        let window = bin::window(options).call(vec![col("x")]);
        window.partition_by(vec![col("g")]).build()
    };
    let (b, c) = (placeholder(10.0)?, placeholder(4.0)?);
    let n = row_number()
        .order_by(vec![col("i").sort(true, false)])
        .build()?;
    let column = |e: &Expr| Expr::Column(Column::from_name(e.schema_name().to_string()));
    view(&ctx, "windows", "SELECT * FROM t", |plan| {
        let (plan, rest) = plan_windows(plan, vec![b.clone(), c.clone(), n.clone()])?;
        assert_eq!(rest, vec![n.clone()]);
        // The window node adds the column of `row_number` last.
        let plan = LogicalPlanBuilder::window_plan(plan, rest)?;
        let n = plan.schema().columns().pop().expect("a column");
        LogicalPlanBuilder::from(plan)
            .project(vec![
                col("i"),
                column(&b).alias("b"),
                column(&c).alias("c"),
                Expr::Column(n).alias("n"),
            ])?
            .build()
    })
    .await?;
    assert_eq!(bins(&ctx, "windows", "b").await?, BY_G);
    // At most 4 bins: bins of 5 from 0, of 50 from 100, and of 1 from 3.
    let want = "0 5, 10 15, 15 20, 100 150, 150 200, 150 200, 3 4, 6 7, -";
    assert_eq!(bins(&ctx, "windows", "c").await?, want);
    let n = rows(&ctx, "SELECT i, n - i FROM windows").await?;
    assert_eq!(
        n,
        (0..9)
            .map(|i| vec![i.to_string(), "1".into()])
            .collect::<Vec<_>>()
    );
    Ok(())
}

/// The extent is that of the finite values, and NULL, NaN, and infinite values have no bin.
#[tokio::test]
async fn non_finite_values() -> Result<()> {
    let ctx = session();
    let input = "SELECT * FROM (VALUES (0, 0.0), (1, 10.0), (2, 'NaN'::double), \
        (3, 'Infinity'::double), (4, NULL)) AS t(i, x)";
    common::bins(&ctx, "binned", input, "x", &[], BinOptions::default()).await?;
    assert_eq!(bins(&ctx, "binned", "b").await?, "0 0.5, 9.5 10, -, -, -");
    Ok(())
}

/// A histogram of bins: [`StructGroupSplit`] groups by whether a bin is NULL and by its fields,
/// then makes the bin again.
#[tokio::test]
async fn histogram() -> Result<()> {
    let ctx = session();
    ctx.add_optimizer_rule(Arc::new(StructGroupSplit));
    exec(&ctx, V).await?;
    let options = options(|o| o.step = Some(25.0));
    common::bins(&ctx, "binned", "SELECT * FROM v", "x", &[], options).await?;
    let sql = "SELECT b, count(*) FROM binned GROUP BY b";
    let explain = rows(&ctx, &format!("EXPLAIN {sql}")).await?;
    assert!(
        explain
            .concat()
            .iter()
            .any(|r| r.contains("__make_nullable_struct")),
        "{explain:?}"
    );
    let want = [
        ["NULL", "1"],
        ["{start: 0.0, end: 25.0}", "3"],
        ["{start: 25.0, end: 50.0}", "1"],
        ["{start: 50.0, end: 75.0}", "2"],
        ["{start: 75.0, end: 100.0}", "2"],
    ];
    assert_eq!(rows(&ctx, sql).await?, want);
    Ok(())
}

/// The checks of the options name the option, and `bin::plan` fails with the check's message.
#[tokio::test]
async fn option_errors() -> Result<()> {
    // The changes of the options, the option, and the message after "The {option} of bin must be".
    let cases: [(Change, &str, &str); 10] = [
        (|o| o.base = 1.0, "base", "a number greater than 1, not 1"),
        (|o| o.base = 0.5, "base", "a number greater than 1, not 0.5"),
        (|o| o.base = 0.0, "base", "a number greater than 1, not 0"),
        (|o| o.maxbins = -3.0, "maxbins", "a positive number, not -3"),
        (|o| o.maxbins = 0.0, "maxbins", "a positive number, not 0"),
        (|o| o.step = Some(0.0), "step", "a positive number, not 0"),
        (|o| o.step = Some(-1.0), "step", "a positive number, not -1"),
        (
            |o| o.divide = vec![0.0],
            "divide",
            "a positive number, not 0",
        ),
        (
            |o| o.steps = Some(vec![5.0, 1.0]),
            "steps",
            "in increasing order",
        ),
        (
            |o| o.extent = Some([1.0, f64::NAN]),
            "extent",
            "two finite numbers, not [1, NaN]",
        ),
    ];
    for (f, option, message) in cases {
        let message = format!("The {option} of bin must be {message}");
        let options = options(f);
        assert_eq!(
            options.check(),
            Err(OptionError { option, message }),
            "{options:?}"
        );
    }

    let ctx = session();
    exec(&ctx, V).await?;
    let options = options(|o| o.maxbins = 0.0);
    let e = common::bins(&ctx, "binned", "SELECT * FROM v", "x", &[], options)
        .await
        .expect_err("a maxbins of 0");
    let message = "The maxbins of bin must be a positive number, not 0";
    assert!(e.to_string().contains(message), "{e}");
    Ok(())
}

/// The bin of each row of a volatile input contains the row's value: the extent and the rows
/// are of one read of the input.
#[tokio::test]
async fn volatile_input() -> Result<()> {
    let ctx = session();
    let input = "SELECT i, random() * 100 AS x FROM (VALUES (0), (1), (2)) AS t(i)";
    common::bins(&ctx, "binned", input, "x", &[], BinOptions::default()).await?;
    let sql =
        "SELECT count(*) FROM binned WHERE b IS NULL OR NOT (b['start'] <= x AND x <= b['end'])";
    assert_eq!(rows(&ctx, sql).await?, [["0"]]);
    Ok(())
}
