//! `bin`: the bins of values and options, the window partitions of a call, the placeholder
//! windows of a projection, non-finite values, a volatile input, histograms of bins, and the
//! checks of the options. A test builds a [`BinCall`] in Rust, registers the plan of
//! [`bin::plan`] as a view, and reads the view with DataFusion's SQL. The test of placeholder
//! windows plans the windows of [`bin::window`] through [`plan_windows`].

mod common;

use std::sync::Arc;

use avenger_datafusion_transforms::bin::{self, BinCall, BinOptions};
use avenger_datafusion_transforms::{OptionError, StructGroupSplit, plan_windows};
use common::{expr, register, session, view};
use datafusion::arrow::array::{Array, ArrayRef, AsArray, Float64Array, Int64Array};
use datafusion::arrow::datatypes::{Float64Type, Int64Type, UInt64Type};
use datafusion::arrow::util::pretty::pretty_format_batches;
use datafusion::common::{Column, Result};
use datafusion::functions_window::expr_fn::row_number;
use datafusion::logical_expr::{Expr, ExprFunctionExt, LogicalPlanBuilder};
use datafusion::prelude::{SessionContext, col};

/// A bin: its start and its end, or None.
type Bin = Option<(f64, f64)>;

/// Registers as the view `name` the query `input` with the column `b`, the bins of `call`.
async fn binned(ctx: &SessionContext, name: &str, input: &str, call: BinCall) -> Result<()> {
    view(ctx, name, input, |plan| {
        bin::plan(plan, call, Column::from_name("b"))
    })
    .await
}

/// A call of `bin` of the column `x` without partitions.
fn call(options: BinOptions) -> BinCall {
    BinCall {
        value: col("x"),
        partition: vec![],
        options,
    }
}

/// The bins of a query of the columns `i` and a bin, in the order of `i`.
async fn bins(ctx: &SessionContext, sql: &str) -> Result<Vec<Bin>> {
    let mut out = Vec::new();
    for b in ctx.sql(sql).await?.collect().await? {
        let i = b.column(0).as_primitive::<Int64Type>();
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
    Ok(out.into_iter().map(|(_, b)| b).collect())
}

/// The table `v` of the values that `expected_bins` and `histogram` bin, with one NULL value
/// when `null` is true.
fn values(ctx: &SessionContext, null: bool) -> Result<()> {
    let mut x = [0.0, 10.0, 20.0, 42.0, 55.0, 60.0, 80.0, 100.0]
        .map(Some)
        .to_vec();
    if null {
        x.push(None);
    }
    register(
        ctx,
        "v",
        vec![
            (
                "i",
                Arc::new(Int64Array::from_iter_values(0..x.len() as i64)) as ArrayRef,
            ),
            ("x", Arc::new(Float64Array::from(x))),
        ],
    )
}

/// The bins of values for the options of each case, and the parameters of a fractional step.
#[tokio::test]
async fn expected_bins() -> Result<()> {
    let ctx = session();
    values(&ctx, false)?;
    // The options, then the bin of each value of `x`, in order.
    let cases: [(BinOptions, [Bin; 8]); 6] = [
        // 20 bins at most: bins of 5 from 0, and the greatest value in the last bin.
        (
            BinOptions::default(),
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
            BinOptions {
                maxbins: 10.0,
                ..Default::default()
            },
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
            BinOptions {
                step: Some(25.0),
                ..Default::default()
            },
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
            BinOptions {
                extent: Some([20.0, 80.0]),
                maxbins: 6.0,
                ..Default::default()
            },
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
        // An extent that is not nice: bins of 10 from its start, to whole steps past its end.
        (
            BinOptions {
                extent: Some([3.0, 97.0]),
                maxbins: 10.0,
                nice: false,
                ..Default::default()
            },
            [
                None,
                Some((3.0, 13.0)),
                Some((13.0, 23.0)),
                Some((33.0, 43.0)),
                Some((53.0, 63.0)),
                Some((53.0, 63.0)),
                Some((73.0, 83.0)),
                Some((93.0, 103.0)),
            ],
        ),
        // An anchor moves the bins up to it, so 0 falls below the first bin.
        (
            BinOptions {
                anchor: Some(2.5),
                ..Default::default()
            },
            [
                None,
                Some((7.5, 12.5)),
                Some((17.5, 22.5)),
                Some((37.5, 42.5)),
                Some((52.5, 57.5)),
                Some((57.5, 62.5)),
                Some((77.5, 82.5)),
                Some((97.5, 102.5)),
            ],
        ),
    ];
    for (options, want) in cases {
        binned(&ctx, "binned", "SELECT * FROM v", call(options.clone())).await?;
        let got = bins(&ctx, "SELECT i, b FROM binned").await?;
        assert_eq!(got, want, "{options:?}");
    }

    // A fractional step: the start stays at the minimum, and the end is a whole step past the
    // maximum.
    let p = bin::bin_params(0.5, 9.7, &BinOptions::default());
    assert_eq!((p.start, p.end, p.step), (0.5, 10.0, 0.5));
    Ok(())
}

/// The table `t` of the partitions of `g`, with a NULL key and a NULL value.
fn partitioned(ctx: &SessionContext) -> Result<()> {
    register(
        ctx,
        "t",
        vec![
            ("i", Arc::new(Int64Array::from_iter_values(0..9))),
            (
                "g",
                Arc::new(Int64Array::from(vec![
                    Some(1),
                    Some(1),
                    Some(1),
                    Some(2),
                    Some(2),
                    Some(2),
                    None,
                    None,
                    Some(1),
                ])),
            ),
            (
                "x",
                Arc::new(Float64Array::from(vec![
                    Some(0.0),
                    Some(10.0),
                    Some(20.0),
                    Some(100.0),
                    Some(150.0),
                    Some(200.0),
                    Some(3.0),
                    Some(7.0),
                    None,
                ])),
            ),
        ],
    )
}

/// The bins of each window partition of `t` by `g`, with at most 10 bins: the NULL key is a
/// partition of its own.
const BY_G: [Bin; 9] = [
    Some((0.0, 2.0)),
    Some((10.0, 12.0)),
    Some((18.0, 20.0)),
    Some((100.0, 110.0)),
    Some((150.0, 160.0)),
    Some((190.0, 200.0)),
    Some((3.0, 3.5)),
    Some((6.5, 7.0)),
    None,
];

/// The extent of each window partition: partitions of a column with a NULL key, and of an
/// expression that is not a column.
#[tokio::test]
async fn partitions() -> Result<()> {
    let ctx = session();
    partitioned(&ctx)?;
    let options = BinOptions {
        maxbins: 10.0,
        ..Default::default()
    };
    let by = |partition: Expr| BinCall {
        value: col("x"),
        partition: vec![partition],
        options: options.clone(),
    };
    binned(&ctx, "binned", "SELECT * FROM t", by(col("g"))).await?;
    let got = bins(&ctx, "SELECT i, b FROM binned").await?;
    assert_eq!(got, BY_G);

    // `coalesce(g, 1)` puts the values of the NULL key, 3 and 7, in the partition of 1, whose
    // bins are bins of 2 from 0.
    let partition = expr(&ctx, "SELECT * FROM t", "coalesce(g, 1)").await?;
    binned(&ctx, "binned", "SELECT * FROM t", by(partition)).await?;
    let got = bins(&ctx, "SELECT i, b FROM binned").await?;
    let mut want = BY_G;
    want[6] = Some((2.0, 4.0));
    want[7] = Some((6.0, 8.0));
    assert_eq!(got, want);
    Ok(())
}

/// The placeholder windows of a projection: two calls over one partition, which one aggregate
/// serves, among another window function, which `plan_windows` leaves to a window node.
#[tokio::test]
async fn placeholder_windows() -> Result<()> {
    let ctx = session();
    partitioned(&ctx)?;
    let placeholder = |maxbins: f64| {
        let options = BinOptions {
            maxbins,
            ..Default::default()
        };
        bin::window(options)
            .call(vec![col("x")])
            .partition_by(vec![col("g")])
            .build()
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
        let n = plan
            .schema()
            .columns()
            .pop()
            .expect("the column of row_number");
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
    assert_eq!(bins(&ctx, "SELECT i, b FROM windows").await?, BY_G);
    // At most 4 bins: bins of 5 from 0, of 50 from 100, and of 1 from 3.
    let want: [Bin; 9] = [
        Some((0.0, 5.0)),
        Some((10.0, 15.0)),
        Some((15.0, 20.0)),
        Some((100.0, 150.0)),
        Some((150.0, 200.0)),
        Some((150.0, 200.0)),
        Some((3.0, 4.0)),
        Some((6.0, 7.0)),
        None,
    ];
    assert_eq!(bins(&ctx, "SELECT i, c FROM windows").await?, want);
    let batches = ctx
        .sql("SELECT i, n FROM windows ORDER BY i")
        .await?
        .collect()
        .await?;
    let n: Vec<u64> = batches
        .iter()
        .flat_map(|b| b.column(1).as_primitive::<UInt64Type>().values().to_vec())
        .collect();
    assert_eq!(n, (1..=9).collect::<Vec<u64>>());
    Ok(())
}

/// The extent is that of the finite values, and NULL, NaN, and infinite values have no bin.
#[tokio::test]
async fn non_finite_values() -> Result<()> {
    let ctx = session();
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
    binned(
        &ctx,
        "binned",
        "SELECT * FROM t",
        call(BinOptions::default()),
    )
    .await?;
    let got = bins(&ctx, "SELECT i, b FROM binned").await?;
    assert_eq!(got, [Some((0.0, 0.5)), Some((9.5, 10.0)), None, None, None]);
    Ok(())
}

/// A histogram of bins: [`StructGroupSplit`] groups by whether a bin is NULL and by its fields,
/// then makes the bin again.
#[tokio::test]
async fn histogram() -> Result<()> {
    let ctx = session();
    ctx.add_optimizer_rule(Arc::new(StructGroupSplit));
    values(&ctx, true)?;
    let options = BinOptions {
        step: Some(25.0),
        ..Default::default()
    };
    binned(&ctx, "binned", "SELECT * FROM v", call(options)).await?;
    let sql = "SELECT b, count(*) AS n FROM binned GROUP BY b ORDER BY b['start'] NULLS FIRST";
    let explain = ctx.sql(&format!("EXPLAIN {sql}")).await?.collect().await?;
    let explain = pretty_format_batches(&explain)?.to_string();
    assert!(explain.contains("__make_nullable_struct"), "{explain}");
    let got = pretty_format_batches(&ctx.sql(sql).await?.collect().await?)?.to_string();
    let want = [
        "+---------------------------+---+",
        "| b                         | n |",
        "+---------------------------+---+",
        "|                           | 1 |",
        "| {start: 0.0, end: 25.0}   | 3 |",
        "| {start: 25.0, end: 50.0}  | 1 |",
        "| {start: 50.0, end: 75.0}  | 2 |",
        "| {start: 75.0, end: 100.0} | 2 |",
        "+---------------------------+---+",
    ]
    .join("\n");
    assert_eq!(got, want);
    Ok(())
}

/// The checks of the options name the option, and `bin::plan` fails with the check's message.
#[tokio::test]
async fn option_errors() -> Result<()> {
    let options = |f: fn(&mut BinOptions)| {
        let mut o = BinOptions::default();
        f(&mut o);
        o
    };
    let cases: [(BinOptions, &str, &str); 10] = [
        (
            options(|o| o.base = 1.0),
            "base",
            "The base of bin must be a number greater than 1, not 1",
        ),
        (
            options(|o| o.base = 0.5),
            "base",
            "The base of bin must be a number greater than 1, not 0.5",
        ),
        (
            options(|o| o.base = 0.0),
            "base",
            "The base of bin must be a number greater than 1, not 0",
        ),
        (
            options(|o| o.maxbins = -3.0),
            "maxbins",
            "The maxbins of bin must be a positive number, not -3",
        ),
        (
            options(|o| o.maxbins = 0.0),
            "maxbins",
            "The maxbins of bin must be a positive number, not 0",
        ),
        (
            options(|o| o.step = Some(0.0)),
            "step",
            "The step of bin must be a positive number, not 0",
        ),
        (
            options(|o| o.step = Some(-1.0)),
            "step",
            "The step of bin must be a positive number, not -1",
        ),
        (
            options(|o| o.divide = vec![0.0]),
            "divide",
            "The divide of bin must be a positive number, not 0",
        ),
        (
            options(|o| o.steps = Some(vec![5.0, 1.0])),
            "steps",
            "The steps of bin must be in increasing order",
        ),
        (
            options(|o| o.extent = Some([1.0, f64::NAN])),
            "extent",
            "The extent of bin must be two finite numbers, not [1, NaN]",
        ),
    ];
    for (options, option, message) in cases {
        let want = OptionError {
            option,
            message: message.to_string(),
        };
        assert_eq!(options.check(), Err(want), "{options:?}");
    }

    let ctx = session();
    values(&ctx, false)?;
    let options = BinOptions {
        maxbins: 0.0,
        ..Default::default()
    };
    let e = binned(&ctx, "binned", "SELECT * FROM v", call(options))
        .await
        .expect_err("a maxbins of 0");
    assert!(
        e.to_string()
            .contains("The maxbins of bin must be a positive number, not 0"),
        "{e}"
    );
    Ok(())
}

/// The bin of each row of a volatile input contains the row's value: the extent and the rows
/// are of one read of the input.
#[tokio::test]
async fn volatile_input() -> Result<()> {
    let ctx = session();
    register(
        &ctx,
        "t",
        vec![("i", Arc::new(Int64Array::from_iter_values(0..3)))],
    )?;
    let input = "SELECT i, random() * 100 AS x FROM t";
    binned(&ctx, "binned", input, call(BinOptions::default())).await?;
    let sql =
        "SELECT count(*) FROM binned WHERE b IS NULL OR NOT (b['start'] <= x AND x <= b['end'])";
    let batches = ctx.sql(sql).await?.collect().await?;
    assert_eq!(batches[0].column(0).as_primitive::<Int64Type>().value(0), 0);
    Ok(())
}
