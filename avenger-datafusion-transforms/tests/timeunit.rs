//! `timeunit`: calls that `timeunit::plan` adds to an input, read with DataFusion's SQL over a
//! view of the plan. The tests cover units that tile time across changes of the UTC offset,
//! expected units, fiscal units, DATE and civil values, the metadata of the result, the choice
//! of automatic units over the window partitions, and errors.

mod common;

use std::sync::Arc;

use avenger_datafusion_transforms::timeunit::{self, TimeUnitOptions, Unit, WeekStart};
use chrono::{DateTime, NaiveDateTime, TimeZone};
use common::{register, rows, session, time_units, unit};
use datafusion::arrow::array::timezone::Tz;
use datafusion::arrow::array::{
    Array, ArrayRef, AsArray, Date32Array, Int64Array, TimestampMillisecondArray,
    TimestampNanosecondArray, TimestampSecondArray,
};
use datafusion::arrow::compute::cast;
use datafusion::arrow::datatypes::{DataType, Int64Type};
use datafusion::common::Result;
use datafusion::prelude::SessionContext;

const M: i64 = 60_000;
const H: i64 = 60 * M;
const D: i64 = 24 * H;
const Y: i64 = 365 * D;
const NY: Option<&str> = Some("America/New_York");
const KTM: Option<&str> = Some("Asia/Kathmandu");

/// The query of the table `c`, the input of the calls.
const INPUT: &str = "SELECT * FROM c";

/// A time unit: (start, end, unit).
type Span = (i64, i64, String);

/// Timestamps in milliseconds in the time zone `tz`.
fn ts(values: Vec<i64>, tz: Option<&str>) -> ArrayRef {
    Arc::new(TimestampMillisecondArray::from(values).with_timezone_opt(tz.map(Arc::from)))
}

/// The milliseconds of the UTC time `s`, such as `2024-03-10T12:30`.
fn ms(s: &str) -> i64 {
    let t = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M").unwrap();
    t.and_utc().timestamp_millis()
}

/// The UTC time of the milliseconds `ms`, for messages.
fn iso(ms: i64) -> String {
    let t = DateTime::from_timestamp_millis(ms).unwrap();
    t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// `n` values from the UTC time `start`, `step` milliseconds apart.
fn series(start: &str, step: i64, n: i64) -> Vec<i64> {
    (0..n).map(|i| ms(start) + i * step).collect()
}

/// The first days of `n` months from January of `year`, `every` months apart.
fn months(year: i32, n: i32, every: i32) -> Vec<i64> {
    let month = |m: i32| format!("{}-{:02}-01T00:00", year + m / 12, m % 12 + 1);
    (0..n).map(|i| ms(&month(i * every))).collect()
}

/// `n` values of a fixed pseudo-random sequence from the UTC time `start` over `span`
/// milliseconds.
fn scattered(start: &str, span: i64, n: i64) -> Vec<i64> {
    let mut s: u64 = 42;
    (0..n)
        .map(|_| {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ms(start) + ((s >> 11) as f64 / (1u64 << 53) as f64 * span as f64) as i64
        })
        .collect()
}

/// The options of an automatic unit with at most `n` units.
fn maxbins(n: i64) -> TimeUnitOptions {
    TimeUnitOptions {
        maxbins: Some(n),
        ..Default::default()
    }
}

/// The options of an automatic unit no shorter than the unit `text`.
fn min_unit(text: &str) -> TimeUnitOptions {
    TimeUnitOptions {
        min_unit: unit(text).unit,
        ..Default::default()
    }
}

/// `options` with weeks that start on Sunday.
fn sunday(options: TimeUnitOptions) -> TimeUnitOptions {
    TimeUnitOptions {
        week_start: Some(WeekStart::Sunday),
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

/// The integers of dates or timestamps.
fn i64s(a: &ArrayRef) -> Vec<i64> {
    let a = match a.data_type() {
        DataType::Date32 => cast(&cast(a, &DataType::Int32).unwrap(), &DataType::Int64).unwrap(),
        _ => cast(a, &DataType::Int64).unwrap(),
    };
    a.as_primitive::<Int64Type>().values().to_vec()
}

/// The units of the column `u` of the query `sql`, none of them NULL.
async fn units(ctx: &SessionContext, sql: &str) -> Result<Vec<Span>> {
    let mut out = Vec::new();
    for b in ctx.sql(sql).await?.collect().await? {
        let u = b.column_by_name("u").expect("a column u").as_struct();
        assert_eq!(u.null_count(), 0, "a NULL unit");
        let names = cast(u.column(2), &DataType::Utf8)?;
        let names = names
            .as_string::<i32>()
            .iter()
            .map(|n| n.unwrap().to_string());
        let spans = i64s(u.column(0)).into_iter().zip(i64s(u.column(1)));
        out.extend(spans.zip(names).map(|((s, e), n)| (s, e, n)));
    }
    Ok(out)
}

/// The time units of the column `t` of the table `c` by `options`, in the order of `t`.
async fn units_of(ctx: &SessionContext, options: TimeUnitOptions) -> Result<Vec<Span>> {
    time_units(ctx, "units", INPUT, "t", &[], options).await?;
    units(ctx, "SELECT u FROM units ORDER BY t").await
}

/// The time unit by `options` of the UTC time `value` in the time zone `tz`.
async fn unit_of(
    ctx: &SessionContext,
    value: &str,
    tz: Option<&str>,
    options: TimeUnitOptions,
) -> Result<Span> {
    register(ctx, "c", vec![("t", ts(vec![ms(value)], tz))])?;
    Ok(units_of(ctx, options).await?.remove(0))
}

/// The error of planning or running the time units of the expression `value` over the table
/// `c` by `options`.
async fn error(ctx: &SessionContext, value: &str, options: TimeUnitOptions) -> String {
    let case = format!("{value} ({options:?})");
    let run = async {
        time_units(ctx, "units", INPUT, value, &[], options).await?;
        common::exec(ctx, "SELECT u FROM units").await
    };
    let e = run.await.expect_err(&format!("no error: {case}"));
    e.to_string()
}

/// The units of a zone contain their values and tile time: in New York, Kathmandu (+05:45),
/// Lord Howe Island (a change of offset of 30 minutes), and UTC, around each change of offset
/// of 2024, each unit starts where the one before it ends, and each day of a change of offset
/// has its 23, 23.5, 24.5, or 25 hours.
#[tokio::test]
async fn units_tile_time() -> Result<()> {
    let ctx = session();
    let mut values = Vec::new();
    for day in ["03-09", "11-02", "04-06", "10-05"] {
        values.extend(series(&format!("2024-{day}T00:00"), 7 * M, 3 * 24 * 60 / 7));
    }
    values.sort();
    let texts = "15 minutes,45 minutes,hour,3 hours,5 hours,36 hours,day,2 days,week,month";
    for tz in [NY, KTM, Some("Australia/Lord_Howe"), Some("UTC")] {
        register(&ctx, "c", vec![("t", ts(values.clone(), tz))])?;
        for text in texts.split(',') {
            let mut tiles = Vec::new();
            for (v, (s, e, _)) in values.iter().zip(units_of(&ctx, unit(text)).await?) {
                let case = format!("{tz:?} '{text}' {}", iso(*v));
                assert!(s <= *v && *v < e, "{case}: [{}, {})", iso(s), iso(e));
                tiles.push((s, e));
            }
            tiles.sort();
            tiles.dedup();
            // Units meet, or no value lies between them.
            for w in tiles.windows(2) {
                let ((s0, e0), (s1, e1)) = (w[0], w[1]);
                let between = values.iter().any(|v| e0 <= *v && *v < s1);
                let (a, b) = ((iso(s0), iso(e0)), (iso(s1), iso(e1)));
                assert!(
                    e0 == s1 || (e0 < s1 && !between),
                    "{tz:?} '{text}': {a:?} then {b:?}"
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
        let (s, e, _) = unit_of(&ctx, "2024-03-10T12:30", Some(tz), unit("hour")).await?;
        let expected = (ms("2024-03-10T12:15"), ms("2024-03-10T13:15"));
        assert_eq!((iso(s), iso(e)), (iso(expected.0), iso(expected.1)), "{tz}");
    }
    for tz in ["Z", "+5:45", "Mars/Olympus_Mons"] {
        register(&ctx, "c", vec![("t", ts(vec![0], Some(tz)))])?;
        let e = time_units(&ctx, "units", INPUT, "t", &[], unit("hour")).await;
        let e = e.expect_err(tz).to_string();
        assert!(
            e.contains(&format!("unknown time zone '{tz}'")),
            "{tz}: {e}"
        );
    }
    Ok(())
}

/// Units of UTC times, from their start to their end in UTC.
#[tokio::test]
async fn expected_units() -> Result<()> {
    // Weeks that start on Monday or Sunday, and fiscal years that start in October or February.
    let mon = TimeUnitOptions {
        week_start: Some(WeekStart::Monday),
        ..Default::default()
    };
    let sun = sunday(mon.clone());
    let (oct, feb) = (fiscal(10, mon.clone()), fiscal(2, mon.clone()));
    // (value, time zone, unit, other options, start, end)
    let cases: [(_, _, _, &TimeUnitOptions, _, _); 42] = [
        // New York, fall back on 2024-11-03 at 06:00Z: 01:00 to 01:59 occur two times.
        (
            "2024-11-03T05:30",
            NY,
            "hour",
            &mon,
            "2024-11-03T05:00",
            "2024-11-03T06:00",
        ),
        (
            "2024-11-03T06:30",
            NY,
            "hour",
            &mon,
            "2024-11-03T06:00",
            "2024-11-03T07:00",
        ),
        (
            "2024-11-03T06:30",
            NY,
            "3 hours",
            &mon,
            "2024-11-03T04:00",
            "2024-11-03T08:00",
        ),
        (
            "2024-11-03T05:59",
            NY,
            "15 minutes",
            &mon,
            "2024-11-03T05:45",
            "2024-11-03T06:00",
        ),
        (
            "2024-11-03T06:05",
            NY,
            "15 minutes",
            &mon,
            "2024-11-03T06:00",
            "2024-11-03T06:15",
        ),
        (
            "2024-11-03T12:00",
            NY,
            "day",
            &mon,
            "2024-11-03T04:00",
            "2024-11-04T05:00",
        ),
        // New York, spring forward on 2024-03-10 at 07:00Z: 02:00 to 02:59 do not occur.
        (
            "2024-03-10T06:30",
            NY,
            "hour",
            &mon,
            "2024-03-10T06:00",
            "2024-03-10T07:00",
        ),
        (
            "2024-03-10T07:30",
            NY,
            "hour",
            &mon,
            "2024-03-10T07:00",
            "2024-03-10T08:00",
        ),
        (
            "2024-03-10T06:30",
            NY,
            "3 hours",
            &mon,
            "2024-03-10T05:00",
            "2024-03-10T07:00",
        ),
        (
            "2024-03-10T07:30",
            NY,
            "3 hours",
            &mon,
            "2024-03-10T07:00",
            "2024-03-10T10:00",
        ),
        (
            "2024-03-10T06:30",
            NY,
            "2 hours",
            &mon,
            "2024-03-10T05:00",
            "2024-03-10T07:00",
        ),
        (
            "2024-03-10T07:30",
            NY,
            "2 hours",
            &mon,
            "2024-03-10T07:00",
            "2024-03-10T08:00",
        ),
        (
            "2024-03-10T12:00",
            NY,
            "day",
            &mon,
            "2024-03-10T05:00",
            "2024-03-11T04:00",
        ),
        (
            "2024-03-10T12:00",
            NY,
            "month",
            &mon,
            "2024-03-01T05:00",
            "2024-04-01T04:00",
        ),
        // Kathmandu, UTC+05:45.
        (
            "2024-03-10T12:30",
            KTM,
            "hour",
            &mon,
            "2024-03-10T12:15",
            "2024-03-10T13:15",
        ),
        (
            "2024-03-10T12:30",
            KTM,
            "30 minutes",
            &mon,
            "2024-03-10T12:15",
            "2024-03-10T12:45",
        ),
        (
            "2024-03-10T12:30",
            KTM,
            "day",
            &mon,
            "2024-03-09T18:15",
            "2024-03-10T18:15",
        ),
        // Civil time: no zone, no changes of offset.
        (
            "2024-03-10T02:30",
            None,
            "hour",
            &mon,
            "2024-03-10T02:00",
            "2024-03-10T03:00",
        ),
        // Weeks, and weeks with a count from the reference years.
        (
            "2024-03-13T12:00",
            None,
            "week",
            &mon,
            "2024-03-11T00:00",
            "2024-03-18T00:00",
        ),
        (
            "2024-03-13T12:00",
            None,
            "week",
            &sun,
            "2024-03-10T00:00",
            "2024-03-17T00:00",
        ),
        (
            "2024-01-10T12:00",
            None,
            "2 weeks",
            &mon,
            "2024-01-01T00:00",
            "2024-01-15T00:00",
        ),
        // A count steps on across its parent units: days and units shorter than a day from
        // January 1, 2012, which the first day of a week does not move, and months from January
        // of year 0.
        (
            "2024-01-31T12:00",
            None,
            "2 days",
            &mon,
            "2024-01-30T00:00",
            "2024-02-01T00:00",
        ),
        (
            "2024-01-31T12:00",
            None,
            "7 days",
            &mon,
            "2024-01-28T00:00",
            "2024-02-04T00:00",
        ),
        (
            "2024-01-31T12:00",
            None,
            "7 days",
            &sun,
            "2024-01-28T00:00",
            "2024-02-04T00:00",
        ),
        (
            "2024-01-01T01:10",
            None,
            "45 minutes",
            &mon,
            "2024-01-01T00:45",
            "2024-01-01T01:30",
        ),
        (
            "2024-03-10T02:30",
            None,
            "90 minutes",
            &mon,
            "2024-03-10T01:30",
            "2024-03-10T03:00",
        ),
        (
            "2024-11-15T12:00",
            None,
            "5 months",
            &mon,
            "2024-08-01T00:00",
            "2025-01-01T00:00",
        ),
        // Fiscal units.
        (
            "2024-11-15T12:00",
            None,
            "fiscal_quarter",
            &oct,
            "2024-10-01T00:00",
            "2025-01-01T00:00",
        ),
        (
            "2024-09-15T12:00",
            None,
            "fiscal_quarter",
            &oct,
            "2024-07-01T00:00",
            "2024-10-01T00:00",
        ),
        (
            "2024-11-15T12:00",
            None,
            "fiscal_year",
            &oct,
            "2024-10-01T00:00",
            "2025-10-01T00:00",
        ),
        (
            "2024-03-15T12:00",
            None,
            "fiscal_quarter",
            &feb,
            "2024-02-01T00:00",
            "2024-05-01T00:00",
        ),
        (
            "2024-01-15T12:00",
            None,
            "2 fiscal_quarters",
            &feb,
            "2023-08-01T00:00",
            "2024-02-01T00:00",
        ),
        // FY2024 of a February year is 2023-02 to 2024-01; two fiscal years start at even ones.
        (
            "2024-01-15T12:00",
            None,
            "2 fiscal_years",
            &feb,
            "2023-02-01T00:00",
            "2025-02-01T00:00",
        ),
        // Collapsing units on the reference year 2012, and the units of weeks on the year of
        // the first day of their weeks: 2024 for Monday, 2012 for Sunday.
        (
            "2023-07-04T15:00",
            None,
            "month_of_year",
            &mon,
            "2012-07-01T00:00",
            "2012-08-01T00:00",
        ),
        (
            "2023-07-04T15:00",
            None,
            "month_of_year",
            &sun,
            "2012-07-01T00:00",
            "2012-08-01T00:00",
        ),
        (
            "2023-01-01T15:00",
            None,
            "week_of_year",
            &mon,
            "2023-12-25T00:00",
            "2024-01-01T00:00",
        ),
        (
            "2023-01-02T15:00",
            None,
            "week_of_year",
            &mon,
            "2024-01-01T00:00",
            "2024-01-08T00:00",
        ),
        (
            "2023-07-04T15:00",
            None,
            "day_of_week",
            &mon,
            "2024-01-02T00:00",
            "2024-01-03T00:00",
        ),
        (
            "2023-03-01T15:00",
            None,
            "day_of_year",
            &mon,
            "2012-02-29T00:00",
            "2012-03-01T00:00",
        ),
        // The same value by month and day: March 1, not the 60th day of a leap year.
        (
            "2023-03-01T15:00",
            None,
            "month_day",
            &mon,
            "2012-03-01T00:00",
            "2012-03-02T00:00",
        ),
        (
            "2023-11-15T15:00",
            None,
            "fiscal_quarter_of_year",
            &oct,
            "2012-10-01T00:00",
            "2013-01-01T00:00",
        ),
        (
            "2024-11-03T06:30",
            NY,
            "hour_of_day",
            &mon,
            "2012-01-01T06:00",
            "2012-01-01T07:00",
        ),
    ];
    let ctx = session();
    for (value, tz, text, other, start, end) in cases {
        let options = TimeUnitOptions {
            unit: unit(text).unit,
            ..other.clone()
        };
        // The row's unit names the calendar of the call.
        let named = (options.unit.unwrap()).text_in(
            options.week_start.unwrap_or_default(),
            options.fiscal_year_start,
        );
        let (s, e, u) = unit_of(&ctx, value, tz, options).await?;
        assert_eq!(
            (iso(s), iso(e), u.as_str()),
            (iso(ms(start)), iso(ms(end)), named.as_str()),
            "{value} {tz:?} '{text}' ({other:?})"
        );
    }
    Ok(())
}

/// The unit of a DATE is a pair of DATEs.
#[tokio::test]
async fn dates_stay_dates() -> Result<()> {
    let ctx = session();
    let input = "SELECT DATE '2024-03-13' AS t";
    time_units(&ctx, "units", input, "t", &[], unit("quarter")).await?;
    let sql = "SELECT arrow_typeof(u['start']), u['start'], u['end'] FROM units";
    assert_eq!(
        rows(&ctx, sql).await?,
        [["Date32", "2024-01-01", "2024-04-01"]]
    );
    Ok(())
}

/// The text of a unit names the calendar of a unit of weeks and of a fiscal unit, after the
/// count, and a parameter out of range or of another unit is an error.
#[test]
fn the_text_of_a_unit_names_its_calendar() {
    use timeunit::UnitCalendar::{FiscalYearStart, WeekStart as Starts};
    for (text, calendar, written) in [
        ("week", None, "week(1)"),
        ("week(2)", Some(Starts(WeekStart::Monday)), "week(2)"),
        (
            "2 weeks(7)",
            Some(Starts(WeekStart::Saturday)),
            "2 weeks(7)",
        ),
        (
            "week_of_year(3)",
            Some(Starts(WeekStart::Tuesday)),
            "week_of_year(3)",
        ),
        (
            "day_of_week(1)",
            Some(Starts(WeekStart::Sunday)),
            "day_of_week(1)",
        ),
        (
            "fiscal_quarter(10)",
            Some(FiscalYearStart(10)),
            "fiscal_quarter(10)",
        ),
        (
            "2 fiscal_years(4)",
            Some(FiscalYearStart(4)),
            "2 fiscal_years(4)",
        ),
        (
            "fiscal_quarter_of_year(7)",
            Some(FiscalYearStart(7)),
            "fiscal_quarter_of_year(7)",
        ),
        ("month", None, "month"),
        ("3 days", None, "3 days"),
    ] {
        let (unit, named) = Unit::parse(text).unwrap();
        assert_eq!(named, calendar, "{text}");
        let mut options = TimeUnitOptions::default();
        options.take_calendar("unit", named).unwrap();
        let month = options.fiscal_year_start;
        assert_eq!(
            unit.text_in(options.week_start.unwrap_or_default(), month),
            written
        );
    }
    for (text, message) in [
        ("week(0)", "from 1 for Sunday to 7 for Saturday, not 0"),
        ("week(8)", "from 1 for Sunday to 7 for Saturday, not 8"),
        ("fiscal_year(13)", "from 1 to 12, not 13"),
        ("month(2)", "takes no parameter"),
        ("week(monday)", "is a number"),
    ] {
        let error = Unit::parse(text).unwrap_err().to_string();
        assert!(error.contains(message), "{text}: {error}");
    }
    // A unit's calendar sets the option, and one that contradicts it is an error.
    let mut options = TimeUnitOptions {
        week_start: Some(WeekStart::Sunday),
        ..Default::default()
    };
    let (_, calendar) = Unit::parse("week(2)").unwrap();
    let error = options.take_calendar("unit", calendar).unwrap_err();
    assert_eq!(error.option, "unit");
}

/// Weeks, `week_of_year`, and `day_of_week` start on each of the seven days, and the reference
/// year of a week's collapsing units starts on its first day.
#[tokio::test]
async fn weeks_start_on_each_day() -> Result<()> {
    let ctx = session();
    // Wednesday, March 13, 2024, and the first day of its week from Sunday to Saturday.
    let starts = [
        "2024-03-10",
        "2024-03-11",
        "2024-03-12",
        "2024-03-13",
        "2024-03-07",
        "2024-03-08",
        "2024-03-09",
    ];
    let years = [2012, 2024, 2008, 2020, 2004, 2016, 2000];
    for (day, (start, year)) in (1..=7).zip(starts.iter().zip(years)) {
        let text = format!("week({day})");
        let (s, e, u) = unit_of(&ctx, "2024-03-13T12:00", None, unit(&text)).await?;
        let first = ms(&format!("{start}T00:00"));
        assert_eq!(
            (s, e, u.as_str()),
            (first, first + 7 * 86_400_000, text.as_str())
        );
        // The day of the week is the day of the reference year at its index in the week.
        let text = format!("day_of_week({day})");
        let (s, _, _) = unit_of(&ctx, "2024-03-13T12:00", None, unit(&text)).await?;
        let index = (3 - (day - 1) as i64).rem_euclid(7);
        assert_eq!(
            s,
            ms(&format!("{year}-01-01T00:00")) + index * 86_400_000,
            "{text}"
        );
    }
    Ok(())
}

/// The result's field holds no metadata, and each row's unit names its calendar: the first day
/// of the weeks of a unit of weeks, and the first month of the fiscal years of a fiscal unit.
#[tokio::test]
async fn units_name_their_calendars() -> Result<()> {
    let ctx = session();
    register(
        &ctx,
        "c",
        vec![("t", ts(vec![ms("2024-03-13T00:00")], None))],
    )?;
    for (options, text) in [
        (unit("quarter"), "quarter"),
        (unit("week"), "week(1)"),
        (unit("2 weeks(2)"), "2 weeks(2)"),
        (unit("day_of_week(7)"), "day_of_week(7)"),
        (fiscal(10, unit("fiscal_quarter")), "fiscal_quarter(10)"),
        (unit("fiscal_year(4)"), "fiscal_year(4)"),
    ] {
        time_units(&ctx, "units", INPUT, "t", &[], options.clone()).await?;
        let df = ctx.sql("SELECT u.unit AS unit FROM units").await?;
        let planned = ctx.sql("SELECT u FROM units").await?;
        assert!(
            planned.schema().field(0).metadata().is_empty(),
            "{options:?}"
        );
        let batches = df.collect().await?;
        let units = batches[0].column(0);
        let units = cast(units, &DataType::Utf8)?;
        assert_eq!(units.as_string::<i32>().value(0), text, "{options:?}");
    }
    Ok(())
}

#[tokio::test]
async fn automatic_units() -> Result<()> {
    let ctx = session();
    // New York local midnights for two months: 23- and 25-hour days in between.
    let tz: Tz = "America/New_York".parse().unwrap();
    let start = NaiveDateTime::parse_from_str("2024-10-01T00:00", "%Y-%m-%dT%H:%M").unwrap();
    let ny_days = (0..60)
        .map(|i| tz.from_local_datetime(&(start + chrono::Duration::days(i))))
        .map(|t| t.earliest().unwrap().timestamp_millis())
        .collect();
    let auto = TimeUnitOptions::default;
    let cases = [
        // Monthly, quarterly, and annual values.
        (months(2023, 24, 1), None, maxbins(100), "month"),
        (months(2020, 20, 3), None, maxbins(100), "quarter"),
        (months(2000, 20, 12), None, maxbins(100), "year"),
        // Daily values at noon, and weekly values on Wednesdays.
        (series("2024-01-01T12:00", D, 10), None, auto(), "day"),
        (
            series("2024-01-03T00:00", 7 * D, 52),
            None,
            maxbins(365),
            "week(1)",
        ),
        (
            series("2024-01-01T00:00", 15 * M, 96),
            None,
            maxbins(200),
            "15 minutes",
        ),
        (
            series("2024-01-01T00:00", 10 * M, 144),
            None,
            maxbins(200),
            "30 minutes",
        ),
        (
            series("2024-01-01T00:00", 2 * H, 24),
            None,
            maxbins(100),
            "6 hours",
        ),
        (series("2024-01-01T00:00", H, 100), None, auto(), "6 hours"),
        (
            scattered("2020-01-01T00:00", 4 * Y, 1000),
            None,
            auto(),
            "quarter",
        ),
        (
            scattered("2020-01-01T00:00", 4 * Y, 1000),
            None,
            fiscal(10, auto()),
            "fiscal_quarter(10)",
        ),
        (
            scattered("1980-01-01T00:00", 40 * Y, 1000),
            None,
            auto(),
            "2 years",
        ),
        (
            scattered("1500-01-01T00:00", 1000 * Y, 1000),
            None,
            auto(),
            "50 years",
        ),
        (
            scattered("2024-01-01T00:00", 10 * D, 1000),
            None,
            min_unit("day"),
            "day",
        ),
        (
            scattered("2024-01-01T00:00", 10 * D, 1000),
            None,
            auto(),
            "12 hours",
        ),
        (
            scattered("2024-01-01T00:00", 2000, 1000),
            None,
            auto(),
            "100 milliseconds",
        ),
        // One value at noon, and one value on January 1.
        (vec![ms("2024-05-05T12:00")], None, auto(), "day"),
        (vec![ms("2024-01-01T00:00")], None, auto(), "year"),
        (ny_days, NY, maxbins(1000), "day"),
    ];
    for (values, tz, options, expected) in cases {
        let case = format!("{} values from {} {tz:?}", values.len(), iso(values[0]));
        register(&ctx, "c", vec![("t", ts(values, tz))])?;
        let got = units_of(&ctx, options.clone()).await?;
        let wrong = got.iter().find(|u| u.2 != expected);
        assert!(wrong.is_none(), "{case} ({options:?}): {wrong:?}");
    }

    // DATE values: daily dates over 10 days choose days, not hours.
    let dates = "SELECT unnest(range(DATE '2024-01-01', DATE '2024-01-11', INTERVAL '1 day')) AS t";
    time_units(&ctx, "units", dates, "t", &[], maxbins(100)).await?;
    let sql = "SELECT arrow_typeof(t), u['unit'], count(*) FROM units GROUP BY 1, 2";
    assert_eq!(rows(&ctx, sql).await?, [["Date32", "day", "10"]]);

    // Window partitions: a monthly partition, and an hourly partition with a NULL key.
    let g = [Some(1); 24]
        .into_iter()
        .chain([None; 48])
        .collect::<Vec<_>>();
    let t = [months(2023, 24, 1), series("2024-01-01T00:00", H, 48)].concat();
    register(
        &ctx,
        "c",
        vec![("g", Arc::new(Int64Array::from(g))), ("t", ts(t, None))],
    )?;
    time_units(&ctx, "units", INPUT, "t", &["g"], maxbins(100)).await?;
    let sql = "SELECT g, u['unit'], count(*) FROM units GROUP BY g, u['unit']";
    assert_eq!(
        rows(&ctx, sql).await?,
        [["1", "month", "24"], ["NULL", "hour", "48"]]
    );
    Ok(())
}

/// Options that contradict each other or are out of range, and units that do not apply to the
/// type of the value: `check` or `check_type` names the option, and planning fails with the
/// same message.
#[tokio::test]
async fn option_errors() -> Result<()> {
    let ctx = session();
    // Timestamps in milliseconds, DATEs, and timestamps in seconds.
    register(
        &ctx,
        "c",
        vec![
            ("t", ts(vec![0], None)),
            ("d", Arc::new(Date32Array::from(vec![0]))),
            ("s", Arc::new(TimestampSecondArray::from(vec![0]))),
        ],
    )?;
    let schema = ctx.table("c").await?.schema().clone();
    let both = TimeUnitOptions {
        maxbins: Some(20),
        ..unit("month")
    };
    let month = "must be the number of a month from 1 to 12, such as 10 for October, not 13";
    let date = "shorter than one day, so it does not apply to a DATE";
    // (column, options, option, message)
    let cases = [
        ("t", both, "maxbins", "takes unit or maxbins, not both"),
        (
            "t",
            unit("fiscal_quarter"),
            "unit",
            "needs fiscal_year_start; without it, use 'quarter'",
        ),
        ("t", fiscal(13, unit("quarter")), "fiscal_year_start", month),
        (
            "t",
            maxbins(0),
            "maxbins",
            "must be a positive integer, not 0",
        ),
        ("d", unit("hour"), "unit", date),
        ("d", min_unit("15 minutes"), "min_unit", date),
        (
            "s",
            unit("500 milliseconds"),
            "unit",
            "shorter than one second",
        ),
    ];
    for (value, options, option, message) in cases {
        let dt = schema.field_with_unqualified_name(value)?.data_type();
        let e = options.check().and_then(|()| options.check_type(dt));
        let e = e.expect_err(&format!("no error: {options:?}"));
        assert_eq!(e.option, option, "{options:?}");
        assert!(e.message.contains(message), "{options:?}: {}", e.message);
        let e = error(&ctx, value, options.clone()).await;
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
        let e = Unit::parse(text).expect_err(text).to_string();
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
        let e = error(&ctx, value, unit(text)).await;
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
    let ns = |s: &str| ms(s) * 1_000_000;
    let values = ["1700-03-01T00:00", "1850-07-01T00:00", "2020-01-01T00:00"].map(ns);
    let t = TimestampNanosecondArray::from(values.to_vec());
    register(&ctx, "c", vec![("t", Arc::new(t))])?;
    let a = units_of(&ctx, TimeUnitOptions::default()).await?;
    assert_eq!(a[0].2, "20 years");
    let a = units_of(&ctx, unit("month")).await?;
    assert_eq!(a[0].0, ns("1700-03-01T00:00"));
    assert_eq!(a[1].1, ns("1850-08-01T00:00"));
    Ok(())
}

/// A unit of seconds where the offset changes by a number of seconds (New York's local mean
/// time ended at noon on 1883-11-18): the units do not overlap.
#[tokio::test]
async fn local_mean_time() -> Result<()> {
    let ctx = session();
    let values = series("1883-11-18T16:58", 1000, 240);
    register(&ctx, "c", vec![("t", ts(values.clone(), NY))])?;
    let a = units_of(&ctx, unit("15 seconds")).await?;
    for (v, (s, e, _)) in values.iter().zip(&a) {
        assert!(s <= v && v < e, "{}: [{}, {})", iso(*v), iso(*s), iso(*e));
    }
    for w in a.windows(2) {
        let ((s0, e0, _), (s1, ..)) = (&w[0], &w[1]);
        let case = format!("[{}, {}) then [{}, ...)", iso(*s0), iso(*e0), iso(*s1));
        assert!(s1 == s0 || s1 >= e0, "{case}");
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
        vec![("t", ts(series("2024-01-01T00:00", H, 100), None))],
    )?;
    let input = "SELECT arrow_cast(arrow_cast(t, 'Int64') + arrow_cast(random() * 1e12, 'Int64'), 'Timestamp(Millisecond, None)') AS t FROM c";
    time_units(&ctx, "units", input, "t", &[], TimeUnitOptions::default()).await?;
    let sql = "SELECT u['start'] <= t AND t < u['end'] AS ok FROM units";
    for batch in ctx.sql(sql).await?.collect().await? {
        assert_eq!(batch.column(0).as_boolean().false_count(), 0);
    }
    Ok(())
}
