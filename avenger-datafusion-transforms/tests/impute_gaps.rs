//! `impute` with `fill_gaps` and `partition_by`: the keys of a sequence that no row has, for
//! time units, bins, dates, intervals, and numbers, in partitions that impute separately.
//!
//! Each case builds an `ImputeCall` with `common::impute_call` from the names of its columns and
//! the SQL of its named arguments over the schema of its input query, plans it with
//! `impute::plan`, and registers the plan as a view, which DataFusion's SQL reads. The keys of
//! time units and bins come from views of `timeunit::plan` and `bin::plan`.

mod common;

use avenger_datafusion_transforms::bin::{self, BinOptions};
use avenger_datafusion_transforms::stack::{self, StackOffset};
use avenger_datafusion_transforms::table_function::ArgAt;
use avenger_datafusion_transforms::timeunit::{TimeUnitOptions, WeekStart};
use common::{exec, impute_call, impute_error, imputed, rows, session, time_units, unit};
use datafusion::common::Result;
use datafusion::prelude::SessionContext;

/// The key, group, and value columns of a call.
type Columns<'a> = (&'a str, &'a [&'a str], &'a str);

/// Named arguments: their names and their SQL.
type Named<'a> = &'a [(&'a str, &'a str)];

/// The named argument `value => 0`.
const VALUE: (&str, &str) = ("value", "0");

/// The named arguments `value => 0` and `fill_gaps => sql`.
fn gaps(sql: &str) -> [(&str, &str); 2] {
    [VALUE, ("fill_gaps", sql)]
}

/// The rows that `sql` reads from the view `imputed` of the call of `impute` over the query
/// `input`, or the error of running the call. A call that does not plan panics.
async fn read(
    ctx: &SessionContext,
    input: &str,
    cols: Columns<'_>,
    named: Named<'_>,
    sql: &str,
) -> Result<Vec<Vec<String>>> {
    let call = impute_call(ctx, input, cols.0, cols.1, cols.2, named).await;
    imputed(ctx, "imputed", input, call.unwrap()).await.unwrap();
    rows(ctx, sql).await
}

/// The tables `sales` and `obs`, and the view `sales_months` of `sales` with the column `u`,
/// the month of `t`.
async fn context() -> Result<SessionContext> {
    let ctx = session();
    exec(
        &ctx,
        "CREATE TABLE sales (t TIMESTAMP, d DATE, series VARCHAR, units DOUBLE) AS VALUES
         (TIMESTAMP '2024-01-05 10:00:00', DATE '2024-01-01', 'A', 5.0),
         (TIMESTAMP '2024-02-10 10:00:00', DATE '2024-02-01', 'A', 10.0),
         (TIMESTAMP '2024-04-20 10:00:00', DATE '2024-04-01', 'A', 20.0),
         (TIMESTAMP '2024-01-07 10:00:00', DATE '2024-01-01', 'B', 7.0),
         (TIMESTAMP '2024-04-02 10:00:00', DATE '2024-04-01', 'B', 3.0)",
    )
    .await?;
    exec(
        &ctx,
        "CREATE TABLE obs (g BIGINT, x DOUBLE) AS VALUES
         (1, 0.0), (1, 10.0), (1, 100.0),
         (2, 0.0), (2, 0.5), (2, 3.0),
         (NULL, 0.0), (NULL, 40.0)",
    )
    .await?;
    let input = "SELECT series, t, units FROM sales";
    time_units(&ctx, "sales_months", input, "t", &[], unit("month")).await?;
    Ok(ctx)
}

/// The added keys keep the type of the unit: a dictionary from `timeunit`, and a string from a
/// key that a query builds, which fills as a key of `timeunit` does.
#[tokio::test]
async fn unit_types() -> Result<()> {
    let ctx = context().await?;
    let months = "SELECT u, units FROM sales_months WHERE series = 'A'";
    let built = "SELECT named_struct('start', date_trunc('month', t), 'end', date_trunc('month', t) + INTERVAL '1 month', 'unit', 'month') AS u, units FROM sales WHERE series = 'A'";
    let sql = "SELECT u['start'], u['unit'], arrow_typeof(u['unit']) FROM imputed";
    for (input, ty) in [(months, "Dictionary(Int32, Utf8)"), (built, "Utf8")] {
        let got = read(&ctx, input, ("u", &[], "units"), &gaps("true"), sql).await?;
        let got: Vec<String> = got.iter().map(|r| r.join(" ")).collect();
        let want = ["01", "02", "03", "04"].map(|m| format!("2024-{m}-01T00:00:00 month {ty}"));
        assert_eq!(got, want, "{input}");
    }
    Ok(())
}

/// Months of `timeunit` per series: every series gets a row at every month from January to
/// April, March too, which no series has, so that each layer of a stack has every month.
#[tokio::test]
async fn time_unit_months() -> Result<()> {
    let ctx = context().await?;
    let monthly = "SELECT series, u, sum(units) AS units FROM sales_months GROUP BY series, u";
    let cols = ("u", &["series"][..], "units");
    let sql = "SELECT series, u['start'] AS month, units FROM imputed";
    let got = read(&ctx, monthly, cols, &gaps("true"), sql).await?;
    let want = [
        ["A", "2024-01-01T00:00:00", "5.0"],
        ["A", "2024-02-01T00:00:00", "10.0"],
        ["A", "2024-03-01T00:00:00", "0.0"],
        ["A", "2024-04-01T00:00:00", "20.0"],
        ["B", "2024-01-01T00:00:00", "7.0"],
        ["B", "2024-02-01T00:00:00", "0.0"],
        ["B", "2024-03-01T00:00:00", "0.0"],
        ["B", "2024-04-01T00:00:00", "3.0"],
    ];
    assert_eq!(got, want);

    // The added keys are whole time units: the end of March and the unit name.
    let sql = "SELECT u['end'] || ' ' || u['unit'] FROM imputed WHERE u['start'] = TIMESTAMP '2024-03-01'";
    let end = "2024-04-01T00:00:00 month";
    assert_eq!(rows(&ctx, sql).await?.concat(), [end, end]);

    // Stacked: each month has a layer of each series.
    ctx.register_udwf(stack::window(StackOffset::Zero).as_ref().clone());
    let sql = "SELECT u['start'] AS month, count(*) FROM (SELECT u, stack(units) OVER (PARTITION BY u ORDER BY series) AS y
         FROM imputed) GROUP BY u ORDER BY 1";
    let stacked = rows(&ctx, sql).await?;
    assert_eq!(stacked.len(), 4);
    assert!(stacked.iter().all(|r| r[1] == "2"));

    // A sequence: all of 2024's first half, by the key's own unit.
    let half = gaps("named_struct('start', TIMESTAMP '2024-01-01', 'end', TIMESTAMP '2024-07-01')");
    let sql = "SELECT DISTINCT u['start'] FROM imputed";
    assert_eq!(read(&ctx, monthly, cols, &half, sql).await?.len(), 6);
    Ok(())
}

/// A DATE key names its unit.
#[tokio::test]
async fn date_keys() -> Result<()> {
    let ctx = context().await?;
    let input = "SELECT d, series, sum(units) AS units FROM sales GROUP BY d, series";
    let cols = ("d", &["series"][..], "units");
    let sql = "SELECT DISTINCT d FROM imputed";
    let got = read(&ctx, input, cols, &gaps("'month'"), sql).await?;
    let want = ["2024-01-01", "2024-02-01", "2024-03-01", "2024-04-01"];
    assert_eq!(got.concat(), want);

    // With the end of the year, half-open: 2024-12-31 ends with December.
    let year = gaps("named_struct('end', DATE '2024-12-31', 'step', 'month')");
    let got = read(&ctx, input, cols, &year, sql).await?.concat();
    assert_eq!(got.len(), 12);
    assert_eq!(got.last().unwrap(), "2024-12-01");

    // Two series of four months each, and the added rows are flagged.
    let named = [VALUE, ("fill_gaps", "'month'"), ("flag", "imputed")];
    let sql = "SELECT imputed, count(*) FROM imputed GROUP BY imputed";
    let n = read(&ctx, input, cols, &named, sql).await?;
    assert_eq!(n, [["false", "5"], ["true", "3"]]);
    Ok(())
}

/// Bins: the empty bins between the bins of the rows, and with the bins of `bin_params`, the
/// empty bins at the ends of the mesh, with the keys that `bin` gives values in them.
#[tokio::test]
async fn bins() -> Result<()> {
    let ctx = session();
    let sql = "CREATE TABLE v (x DOUBLE) AS VALUES (5.0), (25.0), (26.0), (55.0)";
    exec(&ctx, sql).await?;
    let options = BinOptions {
        extent: Some([0.0, 100.0]),
        maxbins: 10.0,
        ..Default::default()
    };
    common::bins(&ctx, "v_bins", "SELECT x FROM v", "x", &[], options.clone()).await?;
    let hist = "SELECT b, count(*) AS n FROM v_bins GROUP BY b";
    let b = ("b", &[][..], "n");
    let starts = async |fill: &str| {
        let sql = "SELECT b['start'] FROM imputed ORDER BY 1";
        read(&ctx, hist, b, &gaps(fill), sql)
            .await
            .map(|r| r.concat())
    };
    let between = starts("true").await?;
    assert_eq!(between, ["0.0", "10.0", "20.0", "30.0", "40.0", "50.0"]);

    // The complete mesh, and the bits of each key are those of `bin`.
    let tens = BinOptions {
        maxbins: 10.0,
        ..Default::default()
    };
    let p = bin::bin_params(0.0, 100.0, &tens);
    let mesh = format!(
        "named_struct('start', {:?}, 'end', {:?}, 'step', {:?})",
        p.start, p.end, p.step
    );
    let sql = "SELECT b['start'], b['end'] FROM imputed";
    let mesh = read(&ctx, hist, b, &gaps(&mesh), sql).await?;
    let input = "SELECT CAST(value AS DOUBLE) * 10 + 5 AS x FROM generate_series(0, 9)";
    common::bins(&ctx, "mesh_bins", input, "x", &[], options).await?;
    let binned = rows(&ctx, "SELECT DISTINCT b['start'], b['end'] FROM mesh_bins").await?;
    assert_eq!(mesh, binned);
    assert_eq!(mesh.len(), 10);

    // A range narrower than the data: keys outside it stay, and no gaps are filled there.
    let narrow = starts("named_struct('start', 20.0, 'end', 40.0)").await?;
    assert_eq!(narrow, ["0.0", "20.0", "30.0", "50.0"]);
    // Only an end: from the least key.
    let to = starts("named_struct('end', 80.0)").await?;
    assert_eq!(to.len(), 8);
    Ok(())
}

/// Partitions: each partition has its own bins, its own gaps, and its own groups. The input
/// runs in several DataFusion partitions, and a NULL partition column is one partition.
#[tokio::test]
async fn partitions() -> Result<()> {
    let ctx = context().await?;
    exec(&ctx, "SET datafusion.execution.target_partitions = 4").await?;
    let options = BinOptions {
        maxbins: 10.0,
        ..Default::default()
    };
    let obs = "SELECT g, x FROM obs";
    common::bins(&ctx, "obs_bins", obs, "x", &["g"], options).await?;
    let hist = "SELECT g, b, count(*) AS n FROM obs_bins GROUP BY g, b";
    let b = ("b", &[][..], "n");
    let by_g = [VALUE, ("fill_gaps", "true"), ("partition_by", "g")];
    let sql = "SELECT g, count(*), sum(n), min(b['end'] - b['start']) FROM imputed GROUP BY g";
    let got = read(&ctx, hist, b, &by_g, sql).await?;
    // g = 1: bins 0 to 90 by 10; g = 2: bins 0 to 2.5 by 0.5; NULL: bins 0 to 35 by 5.
    let want = [
        ["1", "10", "3", "10.0"],
        ["2", "6", "3", "0.5"],
        ["NULL", "8", "2", "5.0"],
    ];
    assert_eq!(got, want);

    // The partitions run apart: the input is hash partitioned on the partition column.
    let plan = rows(&ctx, "EXPLAIN SELECT * FROM imputed").await?;
    let plan: String = plan.iter().map(|r| r.join(" ")).collect();
    let hash = "RepartitionExec: partitioning=Hash([g@";
    assert!(plan.contains(hash), "{plan}");

    // Without the partitions, the bins of the partitions differ in width.
    let e = read(&ctx, hist, b, &gaps("true"), "SELECT * FROM imputed").await;
    let e = e.unwrap_err().to_string();
    assert!(
        e.contains("intervals of one width") && e.contains("partition_by"),
        "{e}"
    );

    // A series that does not occur in a partition gets no rows in it.
    let input = "SELECT d, series, sum(units) AS units, CASE WHEN series = 'A' THEN 'left' ELSE 'right' END AS facet FROM sales GROUP BY d, series";
    let named = [VALUE, ("fill_gaps", "'month'"), ("partition_by", "facet")];
    let sql = "SELECT facet, series, count(*) FROM imputed GROUP BY facet, series";
    let got = read(&ctx, input, ("d", &["series"], "units"), &named, sql).await?;
    assert_eq!(got, [["left", "A", "4"], ["right", "B", "4"]]);
    Ok(())
}

/// Calendars: hours on a day of New York with a repeated hour, Sunday weeks and fiscal
/// quarters from the options of `timeunit`.
#[tokio::test]
async fn calendars() -> Result<()> {
    let ctx = session();
    exec(&ctx,
        "CREATE TABLE ts AS SELECT arrow_cast(column1, 'Timestamp(Millisecond, Some(\"America/New_York\"))') AS t FROM (VALUES
         (TIMESTAMP '2024-11-03 04:30:00'), (TIMESTAMP '2024-11-04 04:30:00'))",
    )
    .await?;
    let starts = async |view: &str| {
        let input = format!("SELECT u, 1 AS n FROM {view}");
        let sql = "SELECT u['start'] FROM imputed";
        read(&ctx, &input, ("u", &[], "n"), &gaps("true"), sql)
            .await
            .map(|r| r.concat())
    };
    time_units(&ctx, "ts_hours", "SELECT t FROM ts", "t", &[], unit("hour")).await?;
    // 00:00 to 23:00 local, with 01:00 two times.
    assert_eq!(starts("ts_hours").await?.len(), 25);

    exec(
        &ctx,
        "CREATE TABLE days (t TIMESTAMP) AS VALUES (TIMESTAMP '2024-03-05'), (TIMESTAMP '2024-03-26'),
         (TIMESTAMP '2024-10-15'), (TIMESTAMP '2025-06-01')",
    )
    .await?;
    let input = "SELECT t FROM days WHERE t < TIMESTAMP '2024-04-01'";
    let sundays = TimeUnitOptions {
        week_start: WeekStart::Sunday,
        ..unit("week")
    };
    time_units(&ctx, "days_weeks", input, "t", &[], sundays).await?;
    let weeks = ["03", "10", "17", "24"].map(|d| format!("2024-03-{d}T00:00:00"));
    assert_eq!(starts("days_weeks").await?, weeks);
    let input = "SELECT t FROM days WHERE t >= TIMESTAMP '2024-10-01'";
    let fiscal = TimeUnitOptions {
        fiscal_year_start: Some(10),
        ..unit("fiscal_quarter")
    };
    time_units(&ctx, "days_quarters", input, "t", &[], fiscal).await?;
    let quarters = ["2024-10", "2025-01", "2025-04"].map(|m| format!("{m}-01T00:00:00"));
    assert_eq!(starts("days_quarters").await?, quarters);
    Ok(())
}

/// Intervals of dates that a query builds, and numbers with a step.
#[tokio::test]
async fn intervals_and_numbers() -> Result<()> {
    let ctx = session();
    exec(&ctx, "CREATE TABLE e (t TIMESTAMP, year BIGINT) AS VALUES (TIMESTAMP '2024-01-01 05:00:00', 2001), (TIMESTAMP '2024-01-04 07:00:00', 2004)")
        .await?;
    let days = "SELECT named_struct('start', date_bin(INTERVAL '1 day', t), 'end', date_bin(INTERVAL '1 day', t) + INTERVAL '1 day') AS k, 1 AS n FROM e";
    let starts = "SELECT k['start'] FROM imputed";
    let got = read(&ctx, days, ("k", &[], "n"), &gaps("true"), starts).await?;
    assert_eq!(got.len(), 4);
    let week = gaps("named_struct('end', TIMESTAMP '2024-01-08', 'step', INTERVAL '1 day')");
    let got = read(&ctx, days, ("k", &[], "n"), &week, starts).await?;
    assert_eq!(got.len(), 7);
    let (input, sql) = ("SELECT year, 1 AS n FROM e", "SELECT year FROM imputed");
    let named = gaps("named_struct('step', 1)");
    let years = read(&ctx, input, ("year", &[], "n"), &named, sql).await?;
    assert_eq!(years.concat(), ["2001", "2002", "2003", "2004"]);
    Ok(())
}

/// The errors of `fill_gaps` and `partition_by`: those of the arguments, at their arguments,
/// then those of the keys of a partition, as the call runs.
#[tokio::test]
async fn errors() -> Result<()> {
    let ctx = context().await?;
    exec(&ctx, "CREATE TABLE mid (d DATE, n BIGINT) AS VALUES (DATE '2024-03-15', 1), (DATE '2024-05-01', 2)").await?;
    let mid = "SELECT * FROM mid";
    let d = ("d", &[][..], "n");
    // The input, the columns, and the named arguments of a call, and a part of the message of
    // its error, at its second named argument.
    let plan_errors: [(&str, Columns, Named, &str); 9] = [
        (
            mid,
            d,
            &gaps("'fiscal_quarter'"),
            "needs a fiscal year start",
        ),
        (
            mid,
            d,
            &gaps("'month_of_year'"),
            "does not apply to a date key",
        ),
        (mid, d, &gaps("'hour'"), "shorter than one day"),
        (mid, d, &gaps("true"), "needs a unit"),
        (
            mid,
            ("n", &[], "d"),
            &[("value", "DATE '2024-01-01'"), ("fill_gaps", "true")],
            "needs a step",
        ),
        (
            "SELECT series, units FROM sales",
            ("series", &[], "units"),
            &gaps("true"),
            "fills the gaps of bins",
        ),
        (
            mid,
            d,
            &[
                VALUE,
                ("fill_gaps", "'month'"),
                ("keys", "[DATE '2024-03-01']"),
            ],
            "`keys` replaces",
        ),
        (
            mid,
            d,
            &gaps("named_struct('stop', DATE '2024-12-01', 'step', 'month')"),
            "Unknown field `stop`",
        ),
        (
            "SELECT * FROM sales",
            ("d", &["series"], "units"),
            &[VALUE, ("partition_by", "series")],
            "is also a column of another argument",
        ),
    ];
    for (input, (key, groups, value), named, message) in plan_errors {
        let call = impute_call(&ctx, input, key, groups, value, named).await?;
        let e = impute_error(&ctx, input, call).await?;
        assert!(
            e.at == ArgAt::Named(1) && e.message.contains(message),
            "{message}: {e:?}"
        );
    }

    let sales = "SELECT t FROM sales";
    time_units(&ctx, "sales_days", sales, "t", &[], unit("day")).await?;
    exec(&ctx, "CREATE TABLE far (t TIMESTAMP, n BIGINT) AS VALUES (TIMESTAMP '2020-01-01', 1), (TIMESTAMP '2024-01-01', 1)").await?;
    let far = "SELECT * FROM far";
    // The input, the columns, and the fill of the gaps of a call, and a part of the message of
    // the error of running it.
    let run_errors: [(&str, Columns, &str, &str); 6] = [
        (mid, d, "'month'", "is not the start of a 'month'"),
        (
            "SELECT named_struct('start', TIMESTAMP '2024-03-03', 'end', TIMESTAMP '2024-03-10', 'unit', 'week') AS k, 1 AS n",
            ("k", &[], "n"),
            "true",
            "is not a unit 'week' of its calendar options",
        ),
        (
            "SELECT u, units FROM sales_months",
            ("u", &[], "units"),
            "'day'",
            "differs from the unit 'month'",
        ),
        (
            "SELECT u, 1 AS n FROM sales_months UNION ALL SELECT u, 1 AS n FROM sales_days",
            ("u", &[], "n"),
            "true",
            "the keys have the units",
        ),
        (
            mid,
            d,
            "named_struct('start', DATE '2024-01-01', 'end', DATE '2026-01-01', 'step', 'month')",
            "is not the start of a 'month'",
        ),
        // The limit of positions.
        (far, ("t", &[], "n"), "'second'", "more than 1000000"),
    ];
    for (input, cols, fill, message) in run_errors {
        let e = read(&ctx, input, cols, &gaps(fill), "SELECT * FROM imputed").await;
        let e = e.expect_err(input).to_string();
        assert!(e.contains(message), "{input}: {e}");
    }
    Ok(())
}
