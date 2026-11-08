//! `impute` with `fill_gaps` and `partition_by`: the keys of a sequence that no row has, for
//! time units, bins, dates, intervals, and numbers, in partitions that impute separately.

mod common;

use avenger_datafusion_transforms::session;
use datafusion::arrow::array::Array;
use datafusion::arrow::util::display::array_value_to_string;
use datafusion::common::Result;
use datafusion::prelude::SessionContext;

/// The rows of a query as strings, sorted.
async fn rows(ctx: &SessionContext, sql: &str) -> Result<Vec<Vec<String>>> {
    let mut out = Vec::new();
    for b in ctx.sql(sql).await?.collect().await? {
        for i in 0..b.num_rows() {
            out.push(
                b.columns()
                    .iter()
                    .map(|c| {
                        if c.is_null(i) {
                            "NULL".to_string()
                        } else {
                            array_value_to_string(c, i).unwrap()
                        }
                    })
                    .collect(),
            );
        }
    }
    out.sort();
    Ok(out)
}

/// The values of the first column of a query, sorted.
async fn column(ctx: &SessionContext, sql: &str) -> Result<Vec<String>> {
    Ok(rows(ctx, sql)
        .await?
        .into_iter()
        .map(|r| r[0].clone())
        .collect())
}

async fn error(ctx: &SessionContext, sql: &str) -> String {
    common::error(ctx, sql)
        .await
        .unwrap_or_else(|| panic!("no error: {sql}"))
}

async fn context() -> Result<SessionContext> {
    let ctx = session(true);
    ctx.sql(
        "CREATE TABLE sales (t TIMESTAMP, d DATE, series VARCHAR, units DOUBLE) AS VALUES
         (TIMESTAMP '2024-01-05 10:00:00', DATE '2024-01-01', 'A', 5.0),
         (TIMESTAMP '2024-02-10 10:00:00', DATE '2024-02-01', 'A', 10.0),
         (TIMESTAMP '2024-04-20 10:00:00', DATE '2024-04-01', 'A', 20.0),
         (TIMESTAMP '2024-01-07 10:00:00', DATE '2024-01-01', 'B', 7.0),
         (TIMESTAMP '2024-04-02 10:00:00', DATE '2024-04-01', 'B', 3.0)",
    )
    .await?
    .collect()
    .await?;
    ctx.sql(
        "CREATE TABLE obs (g BIGINT, x DOUBLE) AS VALUES
         (1, 0.0), (1, 10.0), (1, 100.0),
         (2, 0.0), (2, 0.5), (2, 3.0),
         (NULL, 0.0), (NULL, 40.0)",
    )
    .await?
    .collect()
    .await?;
    Ok(ctx)
}

/// Months of `timeunit` per series: every series gets a row at every month from January to
/// April, March too, which no series has, so that each layer of a stack has every month.
#[tokio::test]
async fn time_unit_months() -> Result<()> {
    let ctx = context().await?;
    let monthly = "(SELECT series, u, sum(units) AS units FROM
         (SELECT series, timeunit(t, 'unit', 'month') OVER () AS u, units FROM sales) GROUP BY series, u)";
    let got = rows(&ctx, &format!(
        "SELECT series, u['start'] AS month, units FROM impute({monthly}, u, [series], units, value => 0, fill_gaps => true)"
    ))
    .await?;
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
    assert_eq!(got, want.map(|r| r.map(String::from).to_vec()));

    // The added keys are whole time units: the end of March and the unit name.
    let ends = column(&ctx, &format!(
        "SELECT u['end'] || ' ' || u['unit'] FROM impute({monthly}, u, [series], units, value => 0, fill_gaps => true) WHERE u['start'] = TIMESTAMP '2024-03-01'"
    ))
    .await?;
    assert_eq!(
        ends,
        ["2024-04-01T00:00:00 month", "2024-04-01T00:00:00 month"]
    );

    // Stacked: each month has a layer of each series.
    let stacked = rows(&ctx, &format!(
        "SELECT u['start'] AS month, count(*) FROM (SELECT u, stack(units) OVER (PARTITION BY u ORDER BY series) AS y
         FROM impute({monthly}, u, [series], units, value => 0, fill_gaps => true)) GROUP BY u ORDER BY 1"
    ))
    .await?;
    assert_eq!(stacked.len(), 4);
    assert!(stacked.iter().all(|r| r[1] == "2"));

    // A sequence: all of 2024's first half, by the key's own unit.
    let half = column(&ctx, &format!(
        "SELECT DISTINCT u['start'] FROM impute({monthly}, u, [series], units, value => 0,
            fill_gaps => named_struct('start', TIMESTAMP '2024-01-01', 'end', TIMESTAMP '2024-07-01'))"
    ))
    .await?;
    assert_eq!(half.len(), 6);
    Ok(())
}

/// A DATE key names its unit.
#[tokio::test]
async fn date_keys() -> Result<()> {
    let ctx = context().await?;
    let input = "(SELECT d, series, sum(units) AS units FROM sales GROUP BY d, series)";
    let got = column(&ctx, &format!(
        "SELECT DISTINCT d FROM impute({input}, d, [series], units, value => 0, fill_gaps => 'month')"
    ))
    .await?;
    assert_eq!(
        got,
        ["2024-01-01", "2024-02-01", "2024-03-01", "2024-04-01"]
    );

    // With the end of the year, half-open: 2024-12-31 ends with December.
    let got = column(
        &ctx,
        &format!(
            "SELECT DISTINCT d FROM impute({input}, d, [series], units, value => 0,
            fill_gaps => named_struct('end', DATE '2024-12-31', 'step', 'month'))"
        ),
    )
    .await?;
    assert_eq!(got.len(), 12);
    assert_eq!(got.last().unwrap(), "2024-12-01");

    // Two series of four months each, and the added rows are flagged.
    let n = rows(&ctx, &format!(
        "SELECT imputed, count(*) FROM impute({input}, d, [series], units, value => 0, fill_gaps => 'month', flag => imputed) GROUP BY imputed"
    ))
    .await?;
    assert_eq!(
        n,
        [["false", "5"], ["true", "3"]].map(|r| r.map(String::from).to_vec())
    );
    Ok(())
}

/// Bins: the empty bins between the bins of the rows, and with `bin_params`, the empty bins
/// at the ends of the mesh, with the keys that `bin` gives values in them.
#[tokio::test]
async fn bins() -> Result<()> {
    let ctx = session(true);
    ctx.sql("CREATE TABLE v (x DOUBLE) AS VALUES (5.0), (25.0), (26.0), (55.0)")
        .await?
        .collect()
        .await?;
    let hist = "(SELECT b, count(*) AS n FROM (SELECT bin(x, 'extent', [0, 100], 'maxbins', 10) OVER () AS b FROM v) GROUP BY b)";
    let starts = |fill: &str| {
        format!(
            "SELECT b['start'] FROM impute({hist}, b, [], n, value => 0, fill_gaps => {fill}) ORDER BY 1"
        )
    };
    let between: Vec<String> = column(&ctx, &starts("true")).await?;
    assert_eq!(between, ["0.0", "10.0", "20.0", "30.0", "40.0", "50.0"]);

    // The complete mesh, and the bits of each key are those of `bin`.
    let mesh = rows(&ctx, &format!(
        "SELECT b['start'], b['end'] FROM impute({hist}, b, [], n, value => 0, fill_gaps => bin_params(0, 100, 'maxbins', 10))"
    ))
    .await?;
    let binned = rows(&ctx,
        "SELECT DISTINCT b['start'], b['end'] FROM (SELECT bin(x, 'extent', [0, 100], 'maxbins', 10) OVER () AS b
         FROM (SELECT CAST(value AS DOUBLE) * 10 + 5 AS x FROM generate_series(0, 9)))",
    )
    .await?;
    assert_eq!(mesh, binned);
    assert_eq!(mesh.len(), 10);

    // A range narrower than the data: keys outside it stay, and no gaps are filled there.
    let narrow = column(&ctx, &starts("named_struct('start', 20.0, 'end', 40.0)")).await?;
    assert_eq!(narrow, ["0.0", "20.0", "30.0", "50.0"]);
    // Only an end: from the least key.
    let to = column(&ctx, &starts("named_struct('end', 80.0)")).await?;
    assert_eq!(to.len(), 8);
    Ok(())
}

/// Partitions: each partition has its own bins, its own gaps, and its own groups. The input
/// runs in several DataFusion partitions, and a NULL partition column is one partition.
#[tokio::test]
async fn partitions() -> Result<()> {
    let ctx = context().await?;
    ctx.sql("SET datafusion.execution.target_partitions = 4")
        .await?
        .collect()
        .await?;
    let hist = "(SELECT g, b, count(*) AS n FROM (SELECT g, bin(x, 'maxbins', 10) OVER (PARTITION BY g) AS b FROM obs) GROUP BY g, b)";
    let got = rows(&ctx, &format!(
        "SELECT g, count(*), sum(n), min(b['end'] - b['start']) FROM impute({hist}, b, [], n, value => 0, fill_gaps => true, partition_by => g) GROUP BY g"
    ))
    .await?;
    // g = 1: bins 0 to 90 by 10; g = 2: bins 0 to 2.5 by 0.5; NULL: bins 0 to 35 by 5.
    let want = [
        ["1", "10", "3", "10.0"],
        ["2", "6", "3", "0.5"],
        ["NULL", "8", "2", "5.0"],
    ];
    assert_eq!(got, want.map(|r| r.map(String::from).to_vec()));

    // The partitions run apart: the input is hash partitioned on the partition column.
    let plan = rows(&ctx, &format!("EXPLAIN SELECT * FROM impute({hist}, b, [], n, value => 0, fill_gaps => true, partition_by => g)")).await?;
    let plan: String = plan.iter().map(|r| r.join(" ")).collect();
    assert!(
        plan.contains("RepartitionExec: partitioning=Hash([g@"),
        "{plan}"
    );

    // Without the partitions, the bins of the partitions differ in width.
    let e = error(
        &ctx,
        &format!("SELECT * FROM impute({hist}, b, [], n, value => 0, fill_gaps => true)"),
    )
    .await;
    assert!(
        e.contains("intervals of one width") && e.contains("partition_by"),
        "{e}"
    );

    // A series that does not occur in a partition gets no rows in it.
    let input = "(SELECT d, series, sum(units) AS units, CASE WHEN series = 'A' THEN 'left' ELSE 'right' END AS facet FROM sales GROUP BY d, series)";
    let got = rows(&ctx, &format!(
        "SELECT facet, series, count(*) FROM impute({input}, d, [series], units, value => 0, fill_gaps => 'month', partition_by => facet) GROUP BY facet, series"
    ))
    .await?;
    assert_eq!(
        got,
        [["left", "A", "4"], ["right", "B", "4"]].map(|r| r.map(String::from).to_vec())
    );
    Ok(())
}

/// Calendars: hours on a day of New York with a repeated hour, Sunday weeks and fiscal
/// quarters from the options of `timeunit`.
#[tokio::test]
async fn calendars() -> Result<()> {
    let ctx = session(true);
    ctx.sql(
        "CREATE TABLE ts AS SELECT arrow_cast(column1, 'Timestamp(Millisecond, Some(\"America/New_York\"))') AS t FROM (VALUES
         (TIMESTAMP '2024-11-03 04:30:00'), (TIMESTAMP '2024-11-04 04:30:00'))",
    )
    .await?
    .collect()
    .await?;
    let hours = column(&ctx,
        "SELECT u['start'] FROM impute((SELECT timeunit(t, 'unit', 'hour') OVER () AS u, 1 AS n FROM ts), u, [], n, value => 0, fill_gaps => true)",
    )
    .await?;
    // 00:00 to 23:00 local, with 01:00 two times.
    assert_eq!(hours.len(), 25);

    ctx.sql(
        "CREATE TABLE days (t TIMESTAMP) AS VALUES (TIMESTAMP '2024-03-05'), (TIMESTAMP '2024-03-26'),
         (TIMESTAMP '2024-10-15'), (TIMESTAMP '2025-06-01')",
    )
    .await?
    .collect()
    .await?;
    let weeks = column(&ctx,
        "SELECT u['start'] FROM impute((SELECT timeunit(t, 'unit', 'week', 'week_start', 'sunday') OVER () AS u, 1 AS n FROM days WHERE t < TIMESTAMP '2024-04-01'),
            u, [], n, value => 0, fill_gaps => true)",
    )
    .await?;
    assert_eq!(
        weeks,
        [
            "2024-03-03T00:00:00",
            "2024-03-10T00:00:00",
            "2024-03-17T00:00:00",
            "2024-03-24T00:00:00"
        ]
    );
    let quarters = column(&ctx,
        "SELECT u['start'] FROM impute((SELECT timeunit(t, 'unit', 'fiscal_quarter', 'fiscal_year_start', 'october') OVER () AS u, 1 AS n FROM days WHERE t >= TIMESTAMP '2024-10-01'),
            u, [], n, value => 0, fill_gaps => true)",
    )
    .await?;
    assert_eq!(
        quarters,
        [
            "2024-10-01T00:00:00",
            "2025-01-01T00:00:00",
            "2025-04-01T00:00:00"
        ]
    );
    Ok(())
}

/// Intervals of dates that a query builds, and numbers with a step.
#[tokio::test]
async fn intervals_and_numbers() -> Result<()> {
    let ctx = session(true);
    ctx.sql("CREATE TABLE e (t TIMESTAMP, year BIGINT) AS VALUES (TIMESTAMP '2024-01-01 05:00:00', 2001), (TIMESTAMP '2024-01-04 07:00:00', 2004)")
        .await?
        .collect()
        .await?;
    let days = "(SELECT named_struct('start', date_bin(INTERVAL '1 day', t), 'end', date_bin(INTERVAL '1 day', t) + INTERVAL '1 day') AS k, 1 AS n FROM e)";
    let got = column(
        &ctx,
        &format!("SELECT k['start'] FROM impute({days}, k, [], n, value => 0, fill_gaps => true)"),
    )
    .await?;
    assert_eq!(got.len(), 4);
    let got = column(&ctx, &format!(
        "SELECT k['start'] FROM impute({days}, k, [], n, value => 0, fill_gaps => named_struct('end', TIMESTAMP '2024-01-08', 'step', INTERVAL '1 day'))"
    ))
    .await?;
    assert_eq!(got.len(), 7);
    let years = column(&ctx,
        "SELECT year FROM impute((SELECT year, 1 AS n FROM e), year, [], n, value => 0, fill_gaps => named_struct('step', 1))",
    )
    .await?;
    assert_eq!(years, ["2001", "2002", "2003", "2004"]);
    Ok(())
}

#[tokio::test]
async fn errors() -> Result<()> {
    let ctx = context().await?;
    ctx.sql("CREATE TABLE mid (d DATE, n BIGINT) AS VALUES (DATE '2024-03-15', 1), (DATE '2024-05-01', 2)").await?.collect().await?;
    let monthly = "(SELECT timeunit(t, 'unit', 'month') OVER () AS u, units FROM sales)";
    for (sql, message) in [
        (
            "SELECT * FROM impute((SELECT * FROM mid), d, [], n, value => 0, fill_gaps => 'month')",
            "is not the start of a 'month'",
        ),
        (
            "SELECT * FROM impute((SELECT named_struct('start', TIMESTAMP '2024-03-03', 'end', TIMESTAMP '2024-03-10', 'unit', 'week') AS k, 1 AS n), k, [], n, value => 0, fill_gaps => true)",
            "is not a unit 'week' of its calendar options",
        ),
        (
            &format!(
                "SELECT * FROM impute({monthly}, u, [], units, value => 0, fill_gaps => 'day')"
            ),
            "differs from the unit 'month'",
        ),
        (
            "SELECT * FROM impute((SELECT * FROM mid), d, [], n, value => 0, fill_gaps => 'fiscal_quarter')",
            "needs a fiscal year start",
        ),
        (
            "SELECT * FROM impute((SELECT * FROM mid), d, [], n, value => 0, fill_gaps => 'month_of_year')",
            "does not apply to a date key",
        ),
        (
            "SELECT * FROM impute((SELECT * FROM mid), d, [], n, value => 0, fill_gaps => 'hour')",
            "shorter than one day",
        ),
        (
            "SELECT * FROM impute((SELECT * FROM mid), d, [], n, value => 0, fill_gaps => true)",
            "needs a unit",
        ),
        (
            "SELECT * FROM impute((SELECT * FROM mid), n, [], d, value => DATE '2024-01-01', fill_gaps => true)",
            "needs a step",
        ),
        (
            "SELECT * FROM impute((SELECT series, units FROM sales), series, [], units, value => 0, fill_gaps => true)",
            "fills the gaps of bins",
        ),
        (
            "SELECT * FROM impute((SELECT * FROM mid), d, [], n, value => 0, fill_gaps => 'month', keys => [DATE '2024-03-01'])",
            "`keys` replaces",
        ),
        (
            "SELECT * FROM impute((SELECT * FROM mid), d, [], n, value => 0, fill_gaps => named_struct('stop', DATE '2024-12-01', 'step', 'month'))",
            "unknown field `stop`",
        ),
        (
            "SELECT * FROM impute((SELECT * FROM sales), d, [series], units, value => 0, partition_by => series)",
            "is also the key, the value column, or a group column",
        ),
        (
            "SELECT * FROM impute((SELECT timeunit(t, 'unit', 'month') OVER () AS u, 1 AS n FROM sales UNION ALL SELECT timeunit(t, 'unit', 'day') OVER () AS u, 1 AS n FROM sales), u, [], n, value => 0, fill_gaps => true)",
            "the keys have the units",
        ),
        (
            "SELECT * FROM impute((SELECT * FROM mid), d, [], n, value => 0, fill_gaps => named_struct('start', DATE '2024-01-01', 'end', DATE '2026-01-01', 'step', 'month'))",
            "is not the start of a 'month'",
        ),
    ] {
        let e = error(&ctx, sql).await;
        assert!(e.contains(message), "{sql}: {e}");
    }
    // The limit of positions.
    ctx.sql("CREATE TABLE far (t TIMESTAMP, n BIGINT) AS VALUES (TIMESTAMP '2020-01-01', 1), (TIMESTAMP '2024-01-01', 1)").await?.collect().await?;
    let e = error(
        &ctx,
        "SELECT * FROM impute((SELECT * FROM far), t, [], n, value => 0, fill_gaps => 'second')",
    )
    .await;
    assert!(e.contains("more than 1000000"), "{e}");
    Ok(())
}
