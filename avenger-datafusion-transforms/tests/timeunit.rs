//! `timeunit`: units that tile time across changes of the UTC offset, expected units, fiscal
//! units, DATE and civil values, the choice of automatic units, the rule against the window
//! evaluator, and errors.

mod common;

use std::sync::Arc;

use avenger_datafusion_transforms::{session, timeunit};
use chrono::{DateTime, NaiveDate, TimeZone};
use common::register;
use datafusion::arrow::array::{
    Array, ArrayRef, AsArray, Date32Array, Int64Array, TimestampMillisecondArray,
};
use datafusion::arrow::compute::cast;
use datafusion::arrow::datatypes::{DataType, Date32Type, Int64Type};
use datafusion::common::Result;
use datafusion::prelude::SessionContext;

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

type Unit = (i64, i64, String);

/// The rows of a query whose column `u` is a time unit: (start, end, unit), None for NULL.
async fn units(ctx: &SessionContext, sql: &str) -> Result<Vec<Option<Unit>>> {
    let mut out = Vec::new();
    for b in timeunit::sql(ctx, sql).await?.collect().await? {
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
    let ctx = session(true);
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
        register(
            &ctx,
            "c",
            vec![
                (
                    "i",
                    Arc::new(Int64Array::from_iter_values(0..values.len() as i64)),
                ),
                ("t", ts(values.clone(), tz)),
            ],
        )?;
        for unit in ["15 minutes", "hour", "3 hours", "day", "week", "month"] {
            let got = units(
                &ctx,
                &format!("SELECT i, timeunit(t, 'unit', '{unit}') AS u FROM c ORDER BY i"),
            )
            .await?;
            let mut tiles: Vec<(i64, i64)> = Vec::new();
            for (v, u) in values.iter().zip(&got) {
                let (s, e, _) = u.clone().unwrap();
                assert!(
                    s <= *v && *v < e,
                    "{tz:?} '{unit}' {}: [{}, {})",
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
                    "{tz:?} '{unit}': [{}, {}) then [{}, {})",
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

/// One expected unit: (value, time zone, options, start, end, unit).
type Expect = (
    &'static str,
    Option<&'static str>,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
);

#[tokio::test]
async fn expected_units() -> Result<()> {
    let cases: [Expect; 37] = [
        // New York, fall back on 2024-11-03 at 06:00Z: 01:00 to 01:59 occur two times.
        (
            "2024-11-03T05:30:00Z",
            NY,
            "'unit', 'hour'",
            "2024-11-03T05:00:00Z",
            "2024-11-03T06:00:00Z",
            "hour",
        ),
        (
            "2024-11-03T06:30:00Z",
            NY,
            "'unit', 'hour'",
            "2024-11-03T06:00:00Z",
            "2024-11-03T07:00:00Z",
            "hour",
        ),
        (
            "2024-11-03T06:30:00Z",
            NY,
            "'unit', '3 hours'",
            "2024-11-03T04:00:00Z",
            "2024-11-03T08:00:00Z",
            "3 hours",
        ),
        (
            "2024-11-03T05:59:00Z",
            NY,
            "'unit', '15 minutes'",
            "2024-11-03T05:45:00Z",
            "2024-11-03T06:00:00Z",
            "15 minutes",
        ),
        (
            "2024-11-03T06:05:00Z",
            NY,
            "'unit', '15 minutes'",
            "2024-11-03T06:00:00Z",
            "2024-11-03T06:15:00Z",
            "15 minutes",
        ),
        (
            "2024-11-03T12:00:00Z",
            NY,
            "'unit', 'day'",
            "2024-11-03T04:00:00Z",
            "2024-11-04T05:00:00Z",
            "day",
        ),
        // New York, spring forward on 2024-03-10 at 07:00Z: 02:00 to 02:59 do not occur.
        (
            "2024-03-10T06:30:00Z",
            NY,
            "'unit', 'hour'",
            "2024-03-10T06:00:00Z",
            "2024-03-10T07:00:00Z",
            "hour",
        ),
        (
            "2024-03-10T07:30:00Z",
            NY,
            "'unit', 'hour'",
            "2024-03-10T07:00:00Z",
            "2024-03-10T08:00:00Z",
            "hour",
        ),
        (
            "2024-03-10T06:30:00Z",
            NY,
            "'unit', '3 hours'",
            "2024-03-10T05:00:00Z",
            "2024-03-10T07:00:00Z",
            "3 hours",
        ),
        (
            "2024-03-10T07:30:00Z",
            NY,
            "'unit', '3 hours'",
            "2024-03-10T07:00:00Z",
            "2024-03-10T10:00:00Z",
            "3 hours",
        ),
        (
            "2024-03-10T06:30:00Z",
            NY,
            "'unit', '2 hours'",
            "2024-03-10T05:00:00Z",
            "2024-03-10T07:00:00Z",
            "2 hours",
        ),
        (
            "2024-03-10T07:30:00Z",
            NY,
            "'unit', '2 hours'",
            "2024-03-10T07:00:00Z",
            "2024-03-10T08:00:00Z",
            "2 hours",
        ),
        (
            "2024-03-10T12:00:00Z",
            NY,
            "'unit', 'day'",
            "2024-03-10T05:00:00Z",
            "2024-03-11T04:00:00Z",
            "day",
        ),
        (
            "2024-03-10T12:00:00Z",
            NY,
            "'unit', 'month'",
            "2024-03-01T05:00:00Z",
            "2024-04-01T04:00:00Z",
            "month",
        ),
        // Kathmandu, UTC+05:45.
        (
            "2024-03-10T12:30:00Z",
            KTM,
            "'unit', 'hour'",
            "2024-03-10T12:15:00Z",
            "2024-03-10T13:15:00Z",
            "hour",
        ),
        (
            "2024-03-10T12:30:00Z",
            KTM,
            "'unit', '30 minutes'",
            "2024-03-10T12:15:00Z",
            "2024-03-10T12:45:00Z",
            "30 minutes",
        ),
        (
            "2024-03-10T12:30:00Z",
            KTM,
            "'unit', 'day'",
            "2024-03-09T18:15:00Z",
            "2024-03-10T18:15:00Z",
            "day",
        ),
        // Civil time: no zone, no changes of offset.
        (
            "2024-03-10T02:30:00Z",
            None,
            "'unit', 'hour'",
            "2024-03-10T02:00:00Z",
            "2024-03-10T03:00:00Z",
            "hour",
        ),
        // Weeks, and weeks with a count from the reference years.
        (
            "2024-03-13T12:00:00Z",
            None,
            "'unit', 'week'",
            "2024-03-11T00:00:00Z",
            "2024-03-18T00:00:00Z",
            "week",
        ),
        (
            "2024-03-13T12:00:00Z",
            None,
            "'unit', 'week', 'week_start', 'sunday'",
            "2024-03-10T00:00:00Z",
            "2024-03-17T00:00:00Z",
            "week",
        ),
        (
            "2024-01-10T12:00:00Z",
            None,
            "'unit', '2 weeks'",
            "2024-01-01T00:00:00Z",
            "2024-01-15T00:00:00Z",
            "2 weeks",
        ),
        // A count ends at the parent unit: '2 days' on the 31st.
        (
            "2024-01-31T12:00:00Z",
            None,
            "'unit', '2 days'",
            "2024-01-31T00:00:00Z",
            "2024-02-01T00:00:00Z",
            "2 days",
        ),
        (
            "2024-11-15T12:00:00Z",
            None,
            "'unit', '5 months'",
            "2024-11-01T00:00:00Z",
            "2025-01-01T00:00:00Z",
            "5 months",
        ),
        // Fiscal units.
        (
            "2024-11-15T12:00:00Z",
            None,
            "'unit', 'fiscal_quarter', 'fiscal_year_start', 'october'",
            "2024-10-01T00:00:00Z",
            "2025-01-01T00:00:00Z",
            "fiscal_quarter",
        ),
        (
            "2024-09-15T12:00:00Z",
            None,
            "'unit', 'fiscal_quarter', 'fiscal_year_start', 'october'",
            "2024-07-01T00:00:00Z",
            "2024-10-01T00:00:00Z",
            "fiscal_quarter",
        ),
        (
            "2024-11-15T12:00:00Z",
            None,
            "'unit', 'fiscal_year', 'fiscal_year_start', 'october'",
            "2024-10-01T00:00:00Z",
            "2025-10-01T00:00:00Z",
            "fiscal_year",
        ),
        (
            "2024-03-15T12:00:00Z",
            None,
            "'unit', 'fiscal_quarter', 'fiscal_year_start', 'february'",
            "2024-02-01T00:00:00Z",
            "2024-05-01T00:00:00Z",
            "fiscal_quarter",
        ),
        (
            "2024-01-15T12:00:00Z",
            None,
            "'unit', '2 fiscal_quarters', 'fiscal_year_start', 'february'",
            "2023-08-01T00:00:00Z",
            "2024-02-01T00:00:00Z",
            "2 fiscal_quarters",
        ),
        // FY2024 of a February year is 2023-02 to 2024-01; two fiscal years start at even ones.
        (
            "2024-01-15T12:00:00Z",
            None,
            "'unit', '2 fiscal_years', 'fiscal_year_start', 'february'",
            "2023-02-01T00:00:00Z",
            "2025-02-01T00:00:00Z",
            "2 fiscal_years",
        ),
        // Collapsing units on the reference years: 2024 for Monday weeks, 2012 for Sunday.
        (
            "2023-07-04T15:00:00Z",
            None,
            "'unit', 'month_of_year'",
            "2024-07-01T00:00:00Z",
            "2024-08-01T00:00:00Z",
            "month_of_year",
        ),
        (
            "2023-07-04T15:00:00Z",
            None,
            "'unit', 'month_of_year', 'week_start', 'sunday'",
            "2012-07-01T00:00:00Z",
            "2012-08-01T00:00:00Z",
            "month_of_year",
        ),
        (
            "2023-01-01T15:00:00Z",
            None,
            "'unit', 'week_of_year'",
            "2023-12-25T00:00:00Z",
            "2024-01-01T00:00:00Z",
            "week_of_year",
        ),
        (
            "2023-01-02T15:00:00Z",
            None,
            "'unit', 'week_of_year'",
            "2024-01-01T00:00:00Z",
            "2024-01-08T00:00:00Z",
            "week_of_year",
        ),
        (
            "2023-07-04T15:00:00Z",
            None,
            "'unit', 'day_of_week'",
            "2024-01-02T00:00:00Z",
            "2024-01-03T00:00:00Z",
            "day_of_week",
        ),
        (
            "2023-03-01T15:00:00Z",
            None,
            "'unit', 'day_of_year'",
            "2024-02-29T00:00:00Z",
            "2024-03-01T00:00:00Z",
            "day_of_year",
        ),
        (
            "2023-11-15T15:00:00Z",
            None,
            "'unit', 'fiscal_quarter_of_year', 'fiscal_year_start', 'october'",
            "2024-10-01T00:00:00Z",
            "2025-01-01T00:00:00Z",
            "fiscal_quarter_of_year",
        ),
        (
            "2024-11-03T06:30:00Z",
            NY,
            "'unit', 'hour_of_day'",
            "2024-01-01T06:00:00Z",
            "2024-01-01T07:00:00Z",
            "hour_of_day",
        ),
    ];
    let ctx = session(true);
    for (value, tz, options, start, end, unit) in cases {
        register(&ctx, "c", vec![("t", ts(vec![ms(value)], tz))])?;
        let got = units(&ctx, &format!("SELECT timeunit(t, {options}) AS u FROM c")).await?;
        let (s, e, u) = got[0].clone().unwrap();
        assert_eq!(
            (iso(s), iso(e), u.as_str()),
            (iso(ms(start)), iso(ms(end)), unit),
            "{value} {tz:?} ({options})"
        );
    }
    Ok(())
}

#[tokio::test]
async fn dates_stay_dates() -> Result<()> {
    let ctx = session(true);
    register(
        &ctx,
        "c",
        vec![("t", Arc::new(Date32Array::from(vec![days(2024, 3, 13)])))],
    )?;
    let b = &timeunit::sql(&ctx, "SELECT timeunit(t, 'unit', 'quarter') AS u FROM c")
        .await?
        .collect()
        .await?[0];
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

/// The field's metadata names the options that the unit needs to be read. The planned schema
/// of the window form has `{}`: a window function cannot see its constant arguments when it
/// computes its field.
#[tokio::test]
async fn metadata() -> Result<()> {
    let ctx = session(true);
    register(
        &ctx,
        "c",
        vec![("t", ts(vec![ms("2024-03-13T00:00:00Z")], None))],
    )?;
    let tag = r#"{"week_start":"monday"}"#;
    for (sql, planned) in [
        (
            "SELECT u, count(*) AS n FROM (SELECT timeunit(t, 'unit', 'quarter') AS u FROM c) GROUP BY u",
            tag,
        ),
        (
            "SELECT u, count(*) AS n FROM (SELECT timeunit(t, 'maxbins', 10) OVER () AS u FROM c) GROUP BY u",
            "{}",
        ),
    ] {
        let df = timeunit::sql(&ctx, sql).await?;
        let key =
            |m: &std::collections::HashMap<String, String>| m.get("avenger.time_unit").cloned();
        assert_eq!(
            key(df.schema().field(0).metadata()).as_deref(),
            Some(planned),
            "{sql}"
        );
        let batches = df.collect().await?;
        assert_eq!(
            key(batches[0].schema().field(0).metadata()).as_deref(),
            Some(tag),
            "{sql}"
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

fn sorted(mut v: Vec<Option<Unit>>) -> Vec<Option<Unit>> {
    v.sort();
    v
}

#[tokio::test]
async fn automatic_units() -> Result<()> {
    let (rules, plain) = (session(true), session(false));
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
    type Case<'a> = (&'a str, Vec<i64>, Option<&'a str>, &'a str, &'a str);
    let cases: Vec<Case> = vec![
        (
            "monthly, 2 years, maxbins 100",
            months(2023, 24, 1),
            None,
            "'maxbins', 100",
            "month",
        ),
        (
            "quarterly, 5 years, maxbins 100",
            months(2020, 20, 3),
            None,
            "'maxbins', 100",
            "quarter",
        ),
        (
            "annual, 20 years, maxbins 100",
            months(2000, 20, 12),
            None,
            "'maxbins', 100",
            "year",
        ),
        (
            "daily at noon, 10 days",
            series("2024-01-01T12:00:00Z", D, 10),
            None,
            "",
            "day",
        ),
        (
            "weekly Wednesdays, 1 year, maxbins 365",
            series("2024-01-03T00:00:00Z", 7 * D, 52),
            None,
            "'maxbins', 365",
            "week",
        ),
        (
            "every 15 minutes, 1 day, maxbins 200",
            series("2024-01-01T00:00:00Z", 15 * 60_000, 96),
            None,
            "'maxbins', 200",
            "15 minutes",
        ),
        (
            "every 10 minutes, 1 day, maxbins 200",
            series("2024-01-01T00:00:00Z", 10 * 60_000, 144),
            None,
            "'maxbins', 200",
            "30 minutes",
        ),
        (
            "every 2 hours, 2 days, maxbins 100",
            series("2024-01-01T00:00:00Z", 2 * H, 24),
            None,
            "'maxbins', 100",
            "6 hours",
        ),
        (
            "scattered, 4 years",
            scattered("2020-01-01T00:00:00Z", 4 * 365 * D, 1000),
            None,
            "",
            "quarter",
        ),
        (
            "scattered, 4 years, fiscal October",
            scattered("2020-01-01T00:00:00Z", 4 * 365 * D, 1000),
            None,
            "'fiscal_year_start', 'october'",
            "fiscal_quarter",
        ),
        (
            "scattered, 40 years",
            scattered("1980-01-01T00:00:00Z", 40 * 365 * D, 1000),
            None,
            "",
            "2 years",
        ),
        (
            "scattered, 1000 years",
            scattered("1500-01-01T00:00:00Z", 1000 * 365 * D, 1000),
            None,
            "",
            "50 years",
        ),
        (
            "scattered, 10 days, min_unit day",
            scattered("2024-01-01T00:00:00Z", 10 * D, 1000),
            None,
            "'min_unit', 'day'",
            "day",
        ),
        (
            "scattered, 10 days",
            scattered("2024-01-01T00:00:00Z", 10 * D, 1000),
            None,
            "",
            "12 hours",
        ),
        (
            "scattered, 2 seconds",
            scattered("2024-01-01T00:00:00Z", 2000, 1000),
            None,
            "",
            "100 milliseconds",
        ),
        (
            "one value at noon",
            vec![ms("2024-05-05T12:00:00Z")],
            None,
            "",
            "day",
        ),
        (
            "one value on January 1",
            vec![ms("2024-01-01T00:00:00Z")],
            None,
            "",
            "year",
        ),
        (
            "New York local midnights, 60 days, maxbins 1000",
            ny_days,
            NY,
            "'maxbins', 1000",
            "day",
        ),
    ];
    for (name, values, tz, options, expected) in cases {
        let comma = if options.is_empty() { "" } else { ", " };
        let sql = format!("SELECT timeunit(t{comma}{options}) OVER () AS u FROM c");
        register(&rules, "c", vec![("t", ts(values.clone(), tz))])?;
        register(&plain, "c", vec![("t", ts(values, tz))])?;
        let a = units(&rules, &sql).await?;
        let b = units(&plain, &sql).await?;
        assert_eq!(a[0].as_ref().unwrap().2, expected, "{name}");
        assert_eq!(
            sorted(a),
            sorted(b),
            "{name}: the rule and the window evaluator differ"
        );
    }

    // DATE values: daily dates over 10 days choose days, not hours.
    let d0 = days(2024, 1, 1);
    register(
        &rules,
        "c",
        vec![(
            "t",
            Arc::new(Date32Array::from(
                (0..10).map(|i| d0 + i).collect::<Vec<_>>(),
            )),
        )],
    )?;
    let a = units(
        &rules,
        "SELECT timeunit(t, 'maxbins', 100) OVER () AS u FROM c",
    )
    .await?;
    assert_eq!(a[0].as_ref().unwrap().2, "day");

    // PARTITION BY: a monthly and an hourly partition, and a partition with a NULL key.
    let mut values = months(2023, 24, 1);
    let mut g: Vec<Option<i64>> = vec![Some(1); values.len()];
    let hourly = series("2024-01-01T00:00:00Z", H, 48);
    g.extend(vec![None; hourly.len()]);
    values.extend(hourly);
    for ctx in [&rules, &plain] {
        register(
            ctx,
            "c",
            vec![
                ("g", Arc::new(Int64Array::from(g.clone()))),
                ("t", ts(values.clone(), None)),
            ],
        )?;
    }
    let sql = "SELECT g, timeunit(t, 'maxbins', 100) OVER (PARTITION BY g) AS u FROM c ORDER BY g NULLS LAST, t";
    let a = units(&rules, sql).await?;
    assert_eq!(a, units(&plain, sql).await?);
    assert_eq!(a[0].as_ref().unwrap().2, "month");
    assert_eq!(a[24].as_ref().unwrap().2, "hour");
    Ok(())
}

#[tokio::test]
async fn errors() -> Result<()> {
    let ctx = session(true);
    register(
        &ctx,
        "c",
        vec![
            ("t", ts(vec![0], None)),
            ("d", Arc::new(Date32Array::from(vec![0]))),
        ],
    )?;
    for (sql, message) in [
        (
            "SELECT timeunit(t, 'unit', 'month', 'maxbins', 20) OVER () FROM c",
            "takes 'unit' or 'maxbins', not both",
        ),
        (
            "SELECT timeunit(t, 'unit', 'fiscal_quarter') FROM c",
            "needs 'fiscal_year_start'; without it, use 'quarter'",
        ),
        (
            "SELECT timeunit(d, 'unit', 'hour') FROM c",
            "shorter than one day, so it does not apply to a DATE",
        ),
        (
            "SELECT timeunit(d, 'min_unit', '15 minutes') OVER () FROM c",
            "shorter than one day, so it does not apply to a DATE",
        ),
        (
            "SELECT timeunit(t, 'unit', 'fortnight') FROM c",
            "unknown time unit 'fortnight'",
        ),
        (
            "SELECT timeunit(t, 'unit', '3 month_of_year') FROM c",
            "'month_of_year' takes no count",
        ),
        ("SELECT timeunit(t) FROM c", "needs an OVER clause"),
        (
            "SELECT timeunit(t, 'unit', 'week', 'week_start', 'friday') FROM c",
            "must be 'monday' or 'sunday', not 'friday'",
        ),
        (
            "SELECT timeunit(t, 'unit', 'quarter', 'fiscal_year_start', 'smarch') FROM c",
            "must be the name of a month",
        ),
        (
            "SELECT timeunit(t, 'maxbins', 0) OVER () FROM c",
            "must be a positive integer, not 0",
        ),
        (
            "SELECT timeunit(t, 'unit', '25 hours') FROM c",
            "restarts at each day, so its count is at most 24",
        ),
        (
            "SELECT timeunit(t, 'unit', '4000000 months') FROM c",
            "restarts at each year, so its count is at most 12",
        ),
        (
            "SELECT timeunit(t, 'unit', '300000 years') FROM c",
            "too close to the limits",
        ),
        (
            "SELECT timeunit(d, 'unit', '200000000 weeks') FROM c",
            "too close to the limits",
        ),
        (
            "SELECT timeunit(arrow_cast(t, 'Timestamp(Second, None)'), 'unit', '500 milliseconds') FROM c",
            "shorter than one second",
        ),
        (
            "SELECT timeunit(arrow_cast(100000000, 'Date32'), 'unit', 'month') FROM c",
            "too close to the limits",
        ),
        (
            "SELECT timeunit(arrow_cast(9223372036854775000, 'Timestamp(Nanosecond, None)'), 'unit', 'year') FROM c",
            "too close to the limits",
        ),
        (
            "SELECT timeunit(t, 'unit', 'month') OVER (ORDER BY t) FROM c",
            "the window of timeunit cannot have an ORDER BY",
        ),
    ] {
        let error = match timeunit::sql(&ctx, sql).await {
            Ok(df) => df.collect().await.err(),
            Err(e) => Some(e),
        };
        let error = error
            .unwrap_or_else(|| panic!("no error: {sql}"))
            .to_string();
        assert!(error.contains(message), "{sql}: {error}");
    }
    Ok(())
}

/// Nanosecond timestamps over more than 292 years, whose differences overflow i64, and
/// values before 1970.
#[tokio::test]
async fn long_spans() -> Result<()> {
    let ctx = session(true);
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
    let a = units(&ctx, "SELECT timeunit(t) OVER () AS u FROM c").await?;
    assert_eq!(a[0].as_ref().unwrap().2, "20 years");
    let a = units(&ctx, "SELECT timeunit(t, 'unit', 'month') AS u FROM c").await?;
    assert_eq!(a[0].as_ref().unwrap().0, ns("1700-03-01T00:00:00Z"));
    assert_eq!(a[1].as_ref().unwrap().1, ns("1850-08-01T00:00:00Z"));
    Ok(())
}

/// A unit of seconds where the offset changes by a number of seconds (New York's local mean
/// time ended at noon on 1883-11-18): the units do not overlap.
#[tokio::test]
async fn local_mean_time() -> Result<()> {
    let ctx = session(true);
    let values = series("1883-11-18T16:58:00Z", 1000, 240);
    register(
        &ctx,
        "c",
        vec![
            ("i", Arc::new(Int64Array::from_iter_values(0..240))),
            ("t", ts(values.clone(), NY)),
        ],
    )?;
    let a = units(
        &ctx,
        "SELECT i, timeunit(t, 'unit', '15 seconds') AS u FROM c ORDER BY i",
    )
    .await?;
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

/// A window node with timeunit and other functions, and a volatile input.
#[tokio::test]
async fn mixed_and_volatile() -> Result<()> {
    let ctx = session(true);
    register(
        &ctx,
        "c",
        vec![("t", ts(series("2024-01-01T00:00:00Z", H, 100), None))],
    )?;
    let sql = "SELECT timeunit(t) OVER () AS u, count(*) OVER () AS n FROM c";
    let a = units(&ctx, sql).await?;
    assert_eq!(a.len(), 100);
    assert_eq!(a[0].as_ref().unwrap().2, "6 hours");
    let sql = "SELECT u['start'] <= t AND t < u['end'] AS ok FROM (SELECT t, timeunit(t) OVER () AS u FROM (SELECT arrow_cast(arrow_cast(t, 'Int64') + arrow_cast(random() * 1e12, 'Int64'), 'Timestamp(Millisecond, None)') AS t FROM c))";
    for batch in timeunit::sql(&ctx, sql).await?.collect().await? {
        assert_eq!(batch.column(0).as_boolean().false_count(), 0);
    }
    Ok(())
}
