//! `timeunit`: calls that `timeunit::plan` adds to an input, read with DataFusion's SQL over a
//! view of the plan. The tests cover units that tile time across changes of the UTC offset,
//! expected units, fiscal units, DATE and civil values, the metadata of the result, the choice
//! of automatic units over the window partitions, and errors.

mod common;

use std::sync::Arc;

use avenger_datafusion_transforms::timeunit::{
    self, TimeUnitCall, TimeUnitOptions, Unit, WeekStart,
};
use chrono::{DateTime, NaiveDate, TimeZone};
use common::{register, session};
use datafusion::arrow::array::{
    Array, ArrayRef, AsArray, Date32Array, Int64Array, TimestampMillisecondArray,
};
use datafusion::arrow::compute::cast;
use datafusion::arrow::datatypes::{DataType, Date32Type, Int64Type, TimeUnit as ArrowUnit};
use datafusion::common::{Column, Result};
use datafusion::logical_expr::Expr;
use datafusion::prelude::{SessionContext, col};

fn ts(values: Vec<i64>, tz: Option<&str>) -> ArrayRef {
    Arc::new(TimestampMillisecondArray::from(values).with_timezone_opt(tz.map(Arc::from)))
}

fn i64s(a: &ArrayRef) -> Vec<i64> {
    let a = match a.data_type() {
        DataType::Date32 => cast(&cast(a, &DataType::Int32).unwrap(), &DataType::Int64).unwrap(),
        _ => cast(a, &DataType::Int64).unwrap(),
    };
    a.as_primitive::<Int64Type>().values().to_vec()
}

/// The options of the unit `text`, such as `'3 months'`.
fn unit(text: &str) -> TimeUnitOptions {
    TimeUnitOptions {
        unit: Some(Unit::parse(text).unwrap()),
        ..Default::default()
    }
}

/// The options of an automatic unit with at most `n` units.
fn maxbins(n: i64) -> TimeUnitOptions {
    TimeUnitOptions {
        maxbins: Some(n),
        ..Default::default()
    }
}

/// `options` with weeks that start on Sunday.
fn sunday(options: TimeUnitOptions) -> TimeUnitOptions {
    TimeUnitOptions {
        week_start: WeekStart::Sunday,
        ..options
    }
}

/// `options` with fiscal years that start in the month `month`.
fn fiscal(month: i64, options: TimeUnitOptions) -> TimeUnitOptions {
    TimeUnitOptions {
        fiscal_year_start: Some(month),
        ..options
    }
}

/// The query of the table `c`, the input of the calls.
const INPUT: &str = "SELECT * FROM c";

/// Registers as the view `units` the rows of the query `input` with the column `u`, the time
/// unit of the expression `value` by `options` over the window partitions of `partition`.
async fn plan_units(
    ctx: &SessionContext,
    input: &str,
    value: &str,
    partition: Vec<Expr>,
    options: TimeUnitOptions,
) -> Result<()> {
    let call = TimeUnitCall {
        value: common::expr(ctx, input, value).await?,
        partition,
        options,
    };
    common::view(ctx, "units", input, |plan| {
        timeunit::plan(plan, call, Column::from_name("u"))
    })
    .await
}

/// A time unit: (start, end, unit).
type Span = (i64, i64, String);

/// The rows of a query whose column `u` is a time unit, None for NULL.
async fn units(ctx: &SessionContext, sql: &str) -> Result<Vec<Option<Span>>> {
    let mut out = Vec::new();
    for b in ctx.sql(sql).await?.collect().await? {
        let u = b.column_by_name("u").expect("a column u").as_struct();
        let (s, e) = (i64s(u.column(0)), i64s(u.column(1)));
        let names = cast(u.column(2), &DataType::Utf8)?;
        let names = names.as_string::<i32>();
        for i in 0..b.num_rows() {
            out.push(
                u.is_valid(i)
                    .then(|| (s[i], e[i], names.value(i).to_string())),
            );
        }
    }
    Ok(out)
}

/// The time units of the column `t` of the table `c` by `options`, in the order of `t`.
async fn units_of(ctx: &SessionContext, options: TimeUnitOptions) -> Result<Vec<Option<Span>>> {
    plan_units(ctx, INPUT, "t", vec![], options).await?;
    units(ctx, "SELECT u FROM units ORDER BY t").await
}

/// The error of planning or running the time units of the expression `value` over the table
/// `c` by `options`.
async fn error(ctx: &SessionContext, value: &str, options: TimeUnitOptions) -> Option<String> {
    let run = async {
        plan_units(ctx, INPUT, value, vec![], options).await?;
        ctx.sql("SELECT u FROM units").await?.collect().await
    };
    run.await.err().map(|e| e.to_string())
}

fn iso(ms: i64) -> String {
    DateTime::from_timestamp_millis(ms)
        .unwrap()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

fn ms(s: &str) -> i64 {
    DateTime::parse_from_rfc3339(s).unwrap().timestamp_millis()
}

fn days(y: i32, m: u32, d: u32) -> i32 {
    (NaiveDate::from_ymd_opt(y, m, d).unwrap() - NaiveDate::from_ymd_opt(1970, 1, 1).unwrap())
        .num_days() as i32
}

const H: i64 = 3_600_000;
const D: i64 = 24 * H;
const NY: Option<&str> = Some("America/New_York");
const KTM: Option<&str> = Some("Asia/Kathmandu");

/// The units of a zone contain their values and tile time: in New York, Kathmandu (+05:45),
/// Lord Howe Island (a change of offset of 30 minutes), and UTC, around each change of offset
/// of 2024, each unit starts where the one before it ends, and each day of a change of offset
/// has its 23, 23.5, 24.5, or 25 hours.
#[tokio::test]
async fn units_tile_time() -> Result<()> {
    let ctx = session();
    let mut values = Vec::new();
    for day in [
        "2024-03-09T00:00:00Z",
        "2024-11-02T00:00:00Z",
        "2024-04-06T00:00:00Z",
        "2024-10-05T00:00:00Z",
    ] {
        values.extend(series(day, 7 * 60_000, 3 * 24 * 60 / 7));
    }
    values.sort();
    for tz in [NY, KTM, Some("Australia/Lord_Howe"), Some("UTC")] {
        register(&ctx, "c", vec![("t", ts(values.clone(), tz))])?;
        for text in [
            "15 minutes",
            "45 minutes",
            "hour",
            "3 hours",
            "5 hours",
            "36 hours",
            "day",
            "2 days",
            "week",
            "month",
        ] {
            let got = units_of(&ctx, unit(text)).await?;
            let mut tiles: Vec<(i64, i64)> = Vec::new();
            for (v, u) in values.iter().zip(&got) {
                let (s, e, _) = u.clone().unwrap();
                assert!(
                    s <= *v && *v < e,
                    "{tz:?} '{text}' {}: [{}, {})",
                    iso(*v),
                    iso(s),
                    iso(e)
                );
                tiles.push((s, e));
            }
            tiles.sort();
            tiles.dedup();
            // Units meet, or no value lies between them.
            for w in tiles.windows(2) {
                let (a, b) = (w[0], w[1]);
                let between = values.iter().any(|v| a.1 <= *v && *v < b.0);
                assert!(
                    a.1 == b.0 || (a.1 < b.0 && !between),
                    "{tz:?} '{text}': [{}, {}) then [{}, {})",
                    iso(a.0),
                    iso(a.1),
                    iso(b.0),
                    iso(b.1)
                );
            }
        }
    }
    Ok(())
}

/// Arrow's time zones: a fixed offset, such as `+05:45`, has one offset, and a time zone that
/// Arrow does not read fails the plan.
#[tokio::test]
async fn zones() -> Result<()> {
    let ctx = session();
    for tz in ["+05:45", "+0545"] {
        register(
            &ctx,
            "c",
            vec![("t", ts(vec![ms("2024-03-10T12:30:00Z")], Some(tz)))],
        )?;
        let got = units_of(&ctx, unit("hour")).await?;
        let (s, e, _) = got[0].clone().unwrap();
        assert_eq!(
            (iso(s), iso(e)),
            (
                iso(ms("2024-03-10T12:15:00Z")),
                iso(ms("2024-03-10T13:15:00Z"))
            ),
            "{tz}"
        );
    }
    for tz in ["Z", "+5:45", "Mars/Olympus_Mons"] {
        register(
            &ctx,
            "c",
            vec![("t", ts(vec![ms("2024-03-10T12:30:00Z")], Some(tz)))],
        )?;
        let e = plan_units(&ctx, INPUT, "t", vec![], unit("hour"))
            .await
            .err()
            .unwrap_or_else(|| panic!("no error: {tz}"))
            .to_string();
        assert!(
            e.contains(&format!("unknown time zone '{tz}'")),
            "{tz}: {e}"
        );
    }
    Ok(())
}

/// One expected unit: (value, time zone, options, start, end, unit).
type Expect = (
    &'static str,
    Option<&'static str>,
    TimeUnitOptions,
    &'static str,
    &'static str,
    &'static str,
);

#[tokio::test]
async fn expected_units() -> Result<()> {
    let cases: [Expect; 42] = [
        // New York, fall back on 2024-11-03 at 06:00Z: 01:00 to 01:59 occur two times.
        (
            "2024-11-03T05:30:00Z",
            NY,
            unit("hour"),
            "2024-11-03T05:00:00Z",
            "2024-11-03T06:00:00Z",
            "hour",
        ),
        (
            "2024-11-03T06:30:00Z",
            NY,
            unit("hour"),
            "2024-11-03T06:00:00Z",
            "2024-11-03T07:00:00Z",
            "hour",
        ),
        (
            "2024-11-03T06:30:00Z",
            NY,
            unit("3 hours"),
            "2024-11-03T04:00:00Z",
            "2024-11-03T08:00:00Z",
            "3 hours",
        ),
        (
            "2024-11-03T05:59:00Z",
            NY,
            unit("15 minutes"),
            "2024-11-03T05:45:00Z",
            "2024-11-03T06:00:00Z",
            "15 minutes",
        ),
        (
            "2024-11-03T06:05:00Z",
            NY,
            unit("15 minutes"),
            "2024-11-03T06:00:00Z",
            "2024-11-03T06:15:00Z",
            "15 minutes",
        ),
        (
            "2024-11-03T12:00:00Z",
            NY,
            unit("day"),
            "2024-11-03T04:00:00Z",
            "2024-11-04T05:00:00Z",
            "day",
        ),
        // New York, spring forward on 2024-03-10 at 07:00Z: 02:00 to 02:59 do not occur.
        (
            "2024-03-10T06:30:00Z",
            NY,
            unit("hour"),
            "2024-03-10T06:00:00Z",
            "2024-03-10T07:00:00Z",
            "hour",
        ),
        (
            "2024-03-10T07:30:00Z",
            NY,
            unit("hour"),
            "2024-03-10T07:00:00Z",
            "2024-03-10T08:00:00Z",
            "hour",
        ),
        (
            "2024-03-10T06:30:00Z",
            NY,
            unit("3 hours"),
            "2024-03-10T05:00:00Z",
            "2024-03-10T07:00:00Z",
            "3 hours",
        ),
        (
            "2024-03-10T07:30:00Z",
            NY,
            unit("3 hours"),
            "2024-03-10T07:00:00Z",
            "2024-03-10T10:00:00Z",
            "3 hours",
        ),
        (
            "2024-03-10T06:30:00Z",
            NY,
            unit("2 hours"),
            "2024-03-10T05:00:00Z",
            "2024-03-10T07:00:00Z",
            "2 hours",
        ),
        (
            "2024-03-10T07:30:00Z",
            NY,
            unit("2 hours"),
            "2024-03-10T07:00:00Z",
            "2024-03-10T08:00:00Z",
            "2 hours",
        ),
        (
            "2024-03-10T12:00:00Z",
            NY,
            unit("day"),
            "2024-03-10T05:00:00Z",
            "2024-03-11T04:00:00Z",
            "day",
        ),
        (
            "2024-03-10T12:00:00Z",
            NY,
            unit("month"),
            "2024-03-01T05:00:00Z",
            "2024-04-01T04:00:00Z",
            "month",
        ),
        // Kathmandu, UTC+05:45.
        (
            "2024-03-10T12:30:00Z",
            KTM,
            unit("hour"),
            "2024-03-10T12:15:00Z",
            "2024-03-10T13:15:00Z",
            "hour",
        ),
        (
            "2024-03-10T12:30:00Z",
            KTM,
            unit("30 minutes"),
            "2024-03-10T12:15:00Z",
            "2024-03-10T12:45:00Z",
            "30 minutes",
        ),
        (
            "2024-03-10T12:30:00Z",
            KTM,
            unit("day"),
            "2024-03-09T18:15:00Z",
            "2024-03-10T18:15:00Z",
            "day",
        ),
        // Civil time: no zone, no changes of offset.
        (
            "2024-03-10T02:30:00Z",
            None,
            unit("hour"),
            "2024-03-10T02:00:00Z",
            "2024-03-10T03:00:00Z",
            "hour",
        ),
        // Weeks, and weeks with a count from the reference years.
        (
            "2024-03-13T12:00:00Z",
            None,
            unit("week"),
            "2024-03-11T00:00:00Z",
            "2024-03-18T00:00:00Z",
            "week",
        ),
        (
            "2024-03-13T12:00:00Z",
            None,
            sunday(unit("week")),
            "2024-03-10T00:00:00Z",
            "2024-03-17T00:00:00Z",
            "week",
        ),
        (
            "2024-01-10T12:00:00Z",
            None,
            unit("2 weeks"),
            "2024-01-01T00:00:00Z",
            "2024-01-15T00:00:00Z",
            "2 weeks",
        ),
        // A count steps on across its parent units: days and units shorter than a day from
        // the first day of a week of the reference year, months from January of year 0.
        (
            "2024-01-31T12:00:00Z",
            None,
            unit("2 days"),
            "2024-01-31T00:00:00Z",
            "2024-02-02T00:00:00Z",
            "2 days",
        ),
        (
            "2024-01-31T12:00:00Z",
            None,
            unit("7 days"),
            "2024-01-29T00:00:00Z",
            "2024-02-05T00:00:00Z",
            "7 days",
        ),
        (
            "2024-01-31T12:00:00Z",
            None,
            sunday(unit("7 days")),
            "2024-01-28T00:00:00Z",
            "2024-02-04T00:00:00Z",
            "7 days",
        ),
        (
            "2024-01-01T01:10:00Z",
            None,
            unit("45 minutes"),
            "2024-01-01T00:45:00Z",
            "2024-01-01T01:30:00Z",
            "45 minutes",
        ),
        (
            "2024-03-10T02:30:00Z",
            None,
            unit("90 minutes"),
            "2024-03-10T01:30:00Z",
            "2024-03-10T03:00:00Z",
            "90 minutes",
        ),
        (
            "2024-11-15T12:00:00Z",
            None,
            unit("5 months"),
            "2024-08-01T00:00:00Z",
            "2025-01-01T00:00:00Z",
            "5 months",
        ),
        // Fiscal units.
        (
            "2024-11-15T12:00:00Z",
            None,
            fiscal(10, unit("fiscal_quarter")),
            "2024-10-01T00:00:00Z",
            "2025-01-01T00:00:00Z",
            "fiscal_quarter",
        ),
        (
            "2024-09-15T12:00:00Z",
            None,
            fiscal(10, unit("fiscal_quarter")),
            "2024-07-01T00:00:00Z",
            "2024-10-01T00:00:00Z",
            "fiscal_quarter",
        ),
        (
            "2024-11-15T12:00:00Z",
            None,
            fiscal(10, unit("fiscal_year")),
            "2024-10-01T00:00:00Z",
            "2025-10-01T00:00:00Z",
            "fiscal_year",
        ),
        (
            "2024-03-15T12:00:00Z",
            None,
            fiscal(2, unit("fiscal_quarter")),
            "2024-02-01T00:00:00Z",
            "2024-05-01T00:00:00Z",
            "fiscal_quarter",
        ),
        (
            "2024-01-15T12:00:00Z",
            None,
            fiscal(2, unit("2 fiscal_quarters")),
            "2023-08-01T00:00:00Z",
            "2024-02-01T00:00:00Z",
            "2 fiscal_quarters",
        ),
        // FY2024 of a February year is 2023-02 to 2024-01; two fiscal years start at even ones.
        (
            "2024-01-15T12:00:00Z",
            None,
            fiscal(2, unit("2 fiscal_years")),
            "2023-02-01T00:00:00Z",
            "2025-02-01T00:00:00Z",
            "2 fiscal_years",
        ),
        // Collapsing units on the reference years: 2024 for Monday weeks, 2012 for Sunday.
        (
            "2023-07-04T15:00:00Z",
            None,
            unit("month_of_year"),
            "2024-07-01T00:00:00Z",
            "2024-08-01T00:00:00Z",
            "month_of_year",
        ),
        (
            "2023-07-04T15:00:00Z",
            None,
            sunday(unit("month_of_year")),
            "2012-07-01T00:00:00Z",
            "2012-08-01T00:00:00Z",
            "month_of_year",
        ),
        (
            "2023-01-01T15:00:00Z",
            None,
            unit("week_of_year"),
            "2023-12-25T00:00:00Z",
            "2024-01-01T00:00:00Z",
            "week_of_year",
        ),
        (
            "2023-01-02T15:00:00Z",
            None,
            unit("week_of_year"),
            "2024-01-01T00:00:00Z",
            "2024-01-08T00:00:00Z",
            "week_of_year",
        ),
        (
            "2023-07-04T15:00:00Z",
            None,
            unit("day_of_week"),
            "2024-01-02T00:00:00Z",
            "2024-01-03T00:00:00Z",
            "day_of_week",
        ),
        (
            "2023-03-01T15:00:00Z",
            None,
            unit("day_of_year"),
            "2024-02-29T00:00:00Z",
            "2024-03-01T00:00:00Z",
            "day_of_year",
        ),
        // The same value by month and day: March 1, not the 60th day of a leap year.
        (
            "2023-03-01T15:00:00Z",
            None,
            unit("month_day"),
            "2024-03-01T00:00:00Z",
            "2024-03-02T00:00:00Z",
            "month_day",
        ),
        (
            "2023-11-15T15:00:00Z",
            None,
            fiscal(10, unit("fiscal_quarter_of_year")),
            "2024-10-01T00:00:00Z",
            "2025-01-01T00:00:00Z",
            "fiscal_quarter_of_year",
        ),
        (
            "2024-11-03T06:30:00Z",
            NY,
            unit("hour_of_day"),
            "2024-01-01T06:00:00Z",
            "2024-01-01T07:00:00Z",
            "hour_of_day",
        ),
    ];
    let ctx = session();
    for (value, tz, options, start, end, name) in cases {
        register(&ctx, "c", vec![("t", ts(vec![ms(value)], tz))])?;
        let got = units_of(&ctx, options.clone()).await?;
        let (s, e, u) = got[0].clone().unwrap();
        assert_eq!(
            (iso(s), iso(e), u.as_str()),
            (iso(ms(start)), iso(ms(end)), name),
            "{value} {tz:?} ({options:?})"
        );
    }
    Ok(())
}

#[tokio::test]
async fn dates_stay_dates() -> Result<()> {
    let ctx = session();
    register(
        &ctx,
        "c",
        vec![("t", Arc::new(Date32Array::from(vec![days(2024, 3, 13)])))],
    )?;
    plan_units(&ctx, INPUT, "t", vec![], unit("quarter")).await?;
    let b = &ctx.sql("SELECT u FROM units").await?.collect().await?[0];
    let u = b.column(0).as_struct();
    assert_eq!(u.column(0).data_type(), &DataType::Date32);
    assert_eq!(
        u.column(0).as_primitive::<Date32Type>().value(0),
        days(2024, 1, 1)
    );
    assert_eq!(
        u.column(1).as_primitive::<Date32Type>().value(0),
        days(2024, 4, 1)
    );
    Ok(())
}

/// The field's metadata names the options that the unit needs to be read, in the planned
/// schema and in the batches, through a query that groups by the unit.
#[tokio::test]
async fn metadata() -> Result<()> {
    let ctx = session();
    register(
        &ctx,
        "c",
        vec![("t", ts(vec![ms("2024-03-13T00:00:00Z")], None))],
    )?;
    for (options, tag) in [
        (unit("quarter"), r#"{"week_start":"monday"}"#),
        (
            sunday(fiscal(10, maxbins(10))),
            r#"{"week_start":"sunday","fiscal_year_start":10}"#,
        ),
    ] {
        plan_units(&ctx, INPUT, "t", vec![], options.clone()).await?;
        let df = ctx
            .sql("SELECT u, count(*) AS n FROM units GROUP BY u")
            .await?;
        let key = |m: &std::collections::HashMap<String, String>| m.get(timeunit::TAG).cloned();
        assert_eq!(
            key(df.schema().field(0).metadata()).as_deref(),
            Some(tag),
            "{options:?}"
        );
        let batches = df.collect().await?;
        assert_eq!(
            key(batches[0].schema().field(0).metadata()).as_deref(),
            Some(tag),
            "{options:?}"
        );
    }
    Ok(())
}

fn series(start: &str, step_ms: i64, n: i64) -> Vec<i64> {
    (0..n).map(|i| ms(start) + i * step_ms).collect()
}

fn months(start_year: i32, n: i32, every: u32) -> Vec<i64> {
    (0..n)
        .map(|i| {
            let m = (i as u32) * every;
            let d = NaiveDate::from_ymd_opt(start_year + (m / 12) as i32, m % 12 + 1, 1).unwrap();
            d.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp_millis()
        })
        .collect()
}

fn scattered(start: &str, span_ms: i64, n: i64) -> Vec<i64> {
    let mut s: u64 = 42;
    (0..n)
        .map(|_| {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ms(start) + ((s >> 11) as f64 / (1u64 << 53) as f64 * span_ms as f64) as i64
        })
        .collect()
}

#[tokio::test]
async fn automatic_units() -> Result<()> {
    let ctx = session();
    // New York local midnights for two months: 23- and 25-hour days in between.
    let tz: datafusion::arrow::array::timezone::Tz = "America/New_York".parse().unwrap();
    let ny_days: Vec<i64> = (0..60)
        .map(|i| {
            let d = NaiveDate::from_ymd_opt(2024, 10, 1).unwrap() + chrono::Duration::days(i);
            tz.from_local_datetime(&d.and_hms_opt(0, 0, 0).unwrap())
                .earliest()
                .unwrap()
                .timestamp_millis()
        })
        .collect();
    let min_unit = |text: &str| TimeUnitOptions {
        min_unit: Some(Unit::parse(text).unwrap()),
        ..Default::default()
    };
    type Case<'a> = (&'a str, Vec<i64>, Option<&'a str>, TimeUnitOptions, &'a str);
    let cases: Vec<Case> = vec![
        (
            "monthly, 2 years, maxbins 100",
            months(2023, 24, 1),
            None,
            maxbins(100),
            "month",
        ),
        (
            "quarterly, 5 years, maxbins 100",
            months(2020, 20, 3),
            None,
            maxbins(100),
            "quarter",
        ),
        (
            "annual, 20 years, maxbins 100",
            months(2000, 20, 12),
            None,
            maxbins(100),
            "year",
        ),
        (
            "daily at noon, 10 days",
            series("2024-01-01T12:00:00Z", D, 10),
            None,
            TimeUnitOptions::default(),
            "day",
        ),
        (
            "weekly Wednesdays, 1 year, maxbins 365",
            series("2024-01-03T00:00:00Z", 7 * D, 52),
            None,
            maxbins(365),
            "week",
        ),
        (
            "every 15 minutes, 1 day, maxbins 200",
            series("2024-01-01T00:00:00Z", 15 * 60_000, 96),
            None,
            maxbins(200),
            "15 minutes",
        ),
        (
            "every 10 minutes, 1 day, maxbins 200",
            series("2024-01-01T00:00:00Z", 10 * 60_000, 144),
            None,
            maxbins(200),
            "30 minutes",
        ),
        (
            "every 2 hours, 2 days, maxbins 100",
            series("2024-01-01T00:00:00Z", 2 * H, 24),
            None,
            maxbins(100),
            "6 hours",
        ),
        (
            "every hour, 100 hours",
            series("2024-01-01T00:00:00Z", H, 100),
            None,
            TimeUnitOptions::default(),
            "6 hours",
        ),
        (
            "scattered, 4 years",
            scattered("2020-01-01T00:00:00Z", 4 * 365 * D, 1000),
            None,
            TimeUnitOptions::default(),
            "quarter",
        ),
        (
            "scattered, 4 years, fiscal October",
            scattered("2020-01-01T00:00:00Z", 4 * 365 * D, 1000),
            None,
            fiscal(10, TimeUnitOptions::default()),
            "fiscal_quarter",
        ),
        (
            "scattered, 40 years",
            scattered("1980-01-01T00:00:00Z", 40 * 365 * D, 1000),
            None,
            TimeUnitOptions::default(),
            "2 years",
        ),
        (
            "scattered, 1000 years",
            scattered("1500-01-01T00:00:00Z", 1000 * 365 * D, 1000),
            None,
            TimeUnitOptions::default(),
            "50 years",
        ),
        (
            "scattered, 10 days, min_unit day",
            scattered("2024-01-01T00:00:00Z", 10 * D, 1000),
            None,
            min_unit("day"),
            "day",
        ),
        (
            "scattered, 10 days",
            scattered("2024-01-01T00:00:00Z", 10 * D, 1000),
            None,
            TimeUnitOptions::default(),
            "12 hours",
        ),
        (
            "scattered, 2 seconds",
            scattered("2024-01-01T00:00:00Z", 2000, 1000),
            None,
            TimeUnitOptions::default(),
            "100 milliseconds",
        ),
        (
            "one value at noon",
            vec![ms("2024-05-05T12:00:00Z")],
            None,
            TimeUnitOptions::default(),
            "day",
        ),
        (
            "one value on January 1",
            vec![ms("2024-01-01T00:00:00Z")],
            None,
            TimeUnitOptions::default(),
            "year",
        ),
        (
            "New York local midnights, 60 days, maxbins 1000",
            ny_days,
            NY,
            maxbins(1000),
            "day",
        ),
    ];
    for (name, values, tz, options, expected) in cases {
        register(&ctx, "c", vec![("t", ts(values, tz))])?;
        let got = units_of(&ctx, options).await?;
        assert!(
            got.iter().all(|u| u.as_ref().unwrap().2 == expected),
            "{name}: {:?}",
            got[0]
        );
    }

    // DATE values: daily dates over 10 days choose days, not hours.
    let d0 = days(2024, 1, 1);
    register(
        &ctx,
        "c",
        vec![(
            "t",
            Arc::new(Date32Array::from(
                (0..10).map(|i| d0 + i).collect::<Vec<_>>(),
            )),
        )],
    )?;
    let got = units_of(&ctx, maxbins(100)).await?;
    assert_eq!(got[0].as_ref().unwrap().2, "day");

    // Window partitions: a monthly and an hourly partition, and a partition with a NULL key.
    let mut values = months(2023, 24, 1);
    let mut g: Vec<Option<i64>> = vec![Some(1); values.len()];
    let hourly = series("2024-01-01T00:00:00Z", H, 48);
    g.extend(vec![None; hourly.len()]);
    values.extend(hourly);
    register(
        &ctx,
        "c",
        vec![
            ("g", Arc::new(Int64Array::from(g))),
            ("t", ts(values, None)),
        ],
    )?;
    plan_units(&ctx, INPUT, "t", vec![col("g")], maxbins(100)).await?;
    let got = units(&ctx, "SELECT u FROM units ORDER BY g NULLS LAST, t").await?;
    assert!(got[..24].iter().all(|u| u.as_ref().unwrap().2 == "month"));
    assert!(got[24..].iter().all(|u| u.as_ref().unwrap().2 == "hour"));
    Ok(())
}

/// Options that contradict each other or are out of range, and units that do not apply to the
/// type of the value: `check` or `check_type` names the option, and planning fails with the
/// same message.
#[tokio::test]
async fn option_errors() -> Result<()> {
    let ctx = session();
    register(
        &ctx,
        "c",
        vec![
            ("t", ts(vec![0], None)),
            ("d", Arc::new(Date32Array::from(vec![0]))),
        ],
    )?;
    let millis = DataType::Timestamp(ArrowUnit::Millisecond, None);
    let seconds = DataType::Timestamp(ArrowUnit::Second, None);
    // (value, its type, options, option, message)
    let cases: [(&str, DataType, TimeUnitOptions, &str, &str); 7] = [
        (
            "t",
            millis.clone(),
            TimeUnitOptions {
                maxbins: Some(20),
                ..unit("month")
            },
            "maxbins",
            "takes unit or maxbins, not both",
        ),
        (
            "t",
            millis.clone(),
            unit("fiscal_quarter"),
            "unit",
            "needs fiscal_year_start; without it, use 'quarter'",
        ),
        (
            "t",
            millis.clone(),
            fiscal(13, unit("quarter")),
            "fiscal_year_start",
            "must be the number of a month from 1 to 12, such as 10 for October, not 13",
        ),
        (
            "t",
            millis,
            maxbins(0),
            "maxbins",
            "must be a positive integer, not 0",
        ),
        (
            "d",
            DataType::Date32,
            unit("hour"),
            "unit",
            "shorter than one day, so it does not apply to a DATE",
        ),
        (
            "d",
            DataType::Date32,
            TimeUnitOptions {
                min_unit: Some(Unit::parse("15 minutes")?),
                ..Default::default()
            },
            "min_unit",
            "shorter than one day, so it does not apply to a DATE",
        ),
        (
            "arrow_cast(t, 'Timestamp(Second, None)')",
            seconds,
            unit("500 milliseconds"),
            "unit",
            "shorter than one second",
        ),
    ];
    for (value, dt, options, option, message) in cases {
        let e = options
            .check()
            .and_then(|()| options.check_type(&dt))
            .expect_err(&format!("no error: {options:?}"));
        assert_eq!(e.option, option, "{options:?}");
        assert!(e.message.contains(message), "{options:?}: {}", e.message);
        let e = error(&ctx, value, options.clone())
            .await
            .unwrap_or_else(|| panic!("no planning error: {options:?}"));
        assert!(e.contains(message), "{options:?}: {e}");
    }
    Ok(())
}

/// Unit texts that `Unit::parse` rejects.
#[test]
fn unit_errors() {
    for (text, message) in [
        ("fortnight", "unknown time unit 'fortnight'"),
        ("3 month_of_year", "'month_of_year' takes no count"),
        (
            "0 hours",
            "the count of the unit '0 hours' must be a positive integer",
        ),
    ] {
        let e = Unit::parse(text)
            .expect_err(&format!("no error: {text}"))
            .to_string();
        assert!(e.contains(message), "{text}: {e}");
    }
}

/// Units and values too close to the limits of the dates that a type can represent.
#[tokio::test]
async fn limits() -> Result<()> {
    let ctx = session();
    register(
        &ctx,
        "c",
        vec![
            ("t", ts(vec![0], None)),
            ("d", Arc::new(Date32Array::from(vec![0]))),
        ],
    )?;
    for (value, text) in [
        ("t", "4000000 months"),
        ("t", "300000 years"),
        ("d", "200000000 weeks"),
        ("arrow_cast(100000000, 'Date32')", "month"),
        (
            "arrow_cast(9223372036854775000, 'Timestamp(Nanosecond, None)')",
            "year",
        ),
    ] {
        let e = error(&ctx, value, unit(text))
            .await
            .unwrap_or_else(|| panic!("no error: {value} '{text}'"));
        assert!(
            e.contains("too close to the limits"),
            "{value} '{text}': {e}"
        );
    }
    Ok(())
}

/// Nanosecond timestamps over more than 292 years, whose differences overflow i64, and
/// values before 1970.
#[tokio::test]
async fn long_spans() -> Result<()> {
    let ctx = session();
    let ns = |s: &str| {
        DateTime::parse_from_rfc3339(s)
            .unwrap()
            .timestamp_nanos_opt()
            .unwrap()
    };
    let values = vec![
        ns("1700-03-01T00:00:00Z"),
        ns("1850-07-01T00:00:00Z"),
        ns("2020-01-01T00:00:00Z"),
    ];
    let t: ArrayRef = Arc::new(datafusion::arrow::array::TimestampNanosecondArray::from(
        values,
    ));
    register(&ctx, "c", vec![("t", t)])?;
    let a = units_of(&ctx, TimeUnitOptions::default()).await?;
    assert_eq!(a[0].as_ref().unwrap().2, "20 years");
    let a = units_of(&ctx, unit("month")).await?;
    assert_eq!(a[0].as_ref().unwrap().0, ns("1700-03-01T00:00:00Z"));
    assert_eq!(a[1].as_ref().unwrap().1, ns("1850-08-01T00:00:00Z"));
    Ok(())
}

/// A unit of seconds where the offset changes by a number of seconds (New York's local mean
/// time ended at noon on 1883-11-18): the units do not overlap.
#[tokio::test]
async fn local_mean_time() -> Result<()> {
    let ctx = session();
    let values = series("1883-11-18T16:58:00Z", 1000, 240);
    register(&ctx, "c", vec![("t", ts(values.clone(), NY))])?;
    let a = units_of(&ctx, unit("15 seconds")).await?;
    for (v, u) in values.iter().zip(&a) {
        let (s, e, _) = u.clone().unwrap();
        assert!(s <= *v && *v < e, "{}: [{}, {})", iso(*v), iso(s), iso(e));
    }
    for w in a.windows(2) {
        let (s0, e0, _) = w[0].clone().unwrap();
        let (s1, _, _) = w[1].clone().unwrap();
        assert!(
            s1 == s0 || s1 >= e0,
            "[{}, {}) overlaps [{}, ...)",
            iso(s0),
            iso(e0),
            iso(s1)
        );
    }
    Ok(())
}

/// A volatile input: the unit of each row contains the row's value.
#[tokio::test]
async fn volatile_input() -> Result<()> {
    let ctx = session();
    register(
        &ctx,
        "c",
        vec![("t", ts(series("2024-01-01T00:00:00Z", H, 100), None))],
    )?;
    let input = "SELECT arrow_cast(arrow_cast(t, 'Int64') + arrow_cast(random() * 1e12, 'Int64'), 'Timestamp(Millisecond, None)') AS t FROM c";
    plan_units(&ctx, input, "t", vec![], TimeUnitOptions::default()).await?;
    let sql = "SELECT u['start'] <= t AND t < u['end'] AS ok FROM units";
    for batch in ctx.sql(sql).await?.collect().await? {
        assert_eq!(batch.column(0).as_boolean().false_count(), 0);
    }
    Ok(())
}
