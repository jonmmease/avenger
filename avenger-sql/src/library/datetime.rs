//! The date and time functions, and the date parts that they take.
//!
//! A `DATE` is a `Date32`, a `DATETIME` a `Timestamp(Microsecond, None)`, and a `TIMESTAMP` a
//! `Timestamp(Microsecond, "UTC")`. A function that takes a time zone takes it as a string
//! literal, such as `'America/Los_Angeles'`, because it becomes part of an Arrow type, and its
//! default is the session's, UTC. `WEEK` starts on Sunday, `WEEK(MONDAY)` on the day it names,
//! and `ISOWEEK` and `ISOYEAR` follow ISO 8601. `DATE_DIFF` counts the boundaries of its part
//! between two dates, and `TIMESTAMP_DIFF` the whole parts of the time between two instants.

use std::sync::Arc;

use datafusion::{
    arrow::datatypes::{DataType, TimeUnit},
    functions::expr_fn as f,
    logical_expr::{Expr, Operator},
};

use super::{cast, lit_i64, lit_str, Call, CallError, Function};
use crate::types;

/// A day of the week, which starts a `WEEK(day)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Weekday {
    Sunday,
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
}

impl Weekday {
    /// The day named `name`, without regard to case.
    pub fn parse(name: &str) -> Option<Self> {
        let day = match name.to_ascii_uppercase().as_str() {
            "SUNDAY" => Self::Sunday,
            "MONDAY" => Self::Monday,
            "TUESDAY" => Self::Tuesday,
            "WEDNESDAY" => Self::Wednesday,
            "THURSDAY" => Self::Thursday,
            "FRIDAY" => Self::Friday,
            "SATURDAY" => Self::Saturday,
            _ => return None,
        };
        Some(day)
    }

    /// The day's number from Sunday, 0, to Saturday, 6.
    fn number(self) -> i64 {
        self as i64
    }
}

/// A part of a date or a time, as in `DATE_TRUNC(d, MONTH)` or `EXTRACT(HOUR FROM t)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DatePart {
    Microsecond,
    Millisecond,
    Second,
    Minute,
    Hour,
    DayOfWeek,
    Day,
    DayOfYear,
    /// A week that starts on the day, which is Sunday for `WEEK`.
    Week(Weekday),
    IsoWeek,
    Month,
    Quarter,
    Year,
    IsoYear,
    Date,
    Time,
    Datetime,
}

impl DatePart {
    /// The part named `name`, as in `MONTH`, with the day of `WEEK(day)`, without regard to case.
    pub fn parse(name: &str, day: Option<&str>) -> Option<Self> {
        let upper = name.to_ascii_uppercase();
        if let Some(day) = day {
            return (upper == "WEEK").then(|| Weekday::parse(day).map(Self::Week))?;
        }
        let part = match upper.as_str() {
            "MICROSECOND" => Self::Microsecond,
            "MILLISECOND" => Self::Millisecond,
            "SECOND" => Self::Second,
            "MINUTE" => Self::Minute,
            "HOUR" => Self::Hour,
            "DAYOFWEEK" => Self::DayOfWeek,
            "DAY" => Self::Day,
            "DAYOFYEAR" => Self::DayOfYear,
            "WEEK" => Self::Week(Weekday::Sunday),
            "ISOWEEK" => Self::IsoWeek,
            "MONTH" => Self::Month,
            "QUARTER" => Self::Quarter,
            "YEAR" => Self::Year,
            "ISOYEAR" => Self::IsoYear,
            "DATE" => Self::Date,
            "TIME" => Self::Time,
            "DATETIME" => Self::Datetime,
            _ => return None,
        };
        Some(part)
    }

    /// The part's name as GoogleSQL writes it.
    pub fn name(self) -> String {
        match self {
            Self::Week(Weekday::Sunday) => "WEEK".to_string(),
            Self::Week(day) => format!("WEEK({})", format!("{day:?}").to_ascii_uppercase()),
            other => format!("{other:?}").to_ascii_uppercase(),
        }
    }

    /// The number of microseconds in the part, for a part of fixed length.
    pub(crate) fn micros(self) -> Option<i64> {
        let micros = match self {
            Self::Microsecond => 1,
            Self::Millisecond => 1_000,
            Self::Second => 1_000_000,
            Self::Minute => 60_000_000,
            Self::Hour => 3_600_000_000,
            Self::Day => 86_400_000_000,
            _ => return None,
        };
        Some(micros)
    }

    /// The granularity of DataFusion's `date_trunc` for the part, when it has one.
    fn granularity(self) -> Option<&'static str> {
        let granularity = match self {
            Self::Microsecond => "microsecond",
            Self::Millisecond => "millisecond",
            Self::Second => "second",
            Self::Minute => "minute",
            Self::Hour => "hour",
            Self::Day => "day",
            Self::Month => "month",
            Self::Quarter => "quarter",
            Self::Year => "year",
            _ => return None,
        };
        Some(granularity)
    }
}

/// The doc of a time zone parameter.
const ZONE: &str =
    "The time zone, a string literal such as `'America/Los_Angeles'`. When omitted, \
                    UTC, the session's default.";
const FORMAT: &str = "The format, whose elements, such as `%Y` and `%m`, are those of `strftime`.";

pub(super) fn functions() -> Vec<Function> {
    vec![
        Function::scalar("CURRENT_DATE", "Returns the current date.")
            .param("time_zone", ZONE)
            .form("([time_zone: CONST STRING]) -> DATE", |call| {
                let zone = time_zone(call, 0)?;
                Ok(cast(local(f::now(), &zone), DataType::Date32))
            })
            .safe(),
        Function::scalar(
            "CURRENT_DATETIME",
            "Returns the current date and time, in civil time.",
        )
        .param("time_zone", ZONE)
        .form("([time_zone: CONST STRING]) -> DATETIME", |call| {
            let zone = time_zone(call, 0)?;
            Ok(local(f::now(), &zone))
        })
        .safe(),
        Function::scalar("CURRENT_TIMESTAMP", "Returns the current instant.")
            .form("() -> TIMESTAMP", |_| {
                Ok(cast(f::now(), types::timestamp()))
            })
            .safe(),
        Function::scalar(
            "DATE",
            "Returns a date from its parts, or the date of a value.",
        )
        .param("year", "The year.")
        .param("month", "The month, from 1 to 12.")
        .param("day", "The day of the month.")
        .param(
            "value",
            "A `TIMESTAMP`, whose date is its date in the time zone, a `DATETIME`, a `DATE`, \
                 or a `STRING` such as `'2024-05-17'`.",
        )
        .param("time_zone", ZONE)
        .form("(year: INT64, month: INT64, day: INT64) -> DATE", |call| {
            Ok(f::make_date(
                cast(call.arg(0), DataType::Int32),
                cast(call.arg(1), DataType::Int32),
                cast(call.arg(2), DataType::Int32),
            ))
        })
        .form(
            "(value: TIMESTAMP[, time_zone: CONST STRING]) -> DATE",
            |call| {
                let zone = time_zone(call, 1)?;
                Ok(cast(local(call.arg(0), &zone), DataType::Date32))
            },
        )
        .form("(value: DATETIME) -> DATE", |call| {
            Ok(cast(call.arg(0), DataType::Date32))
        })
        .form("(value: DATE) -> DATE", |call| Ok(call.arg(0)))
        .form("(value: STRING) -> DATE", |call| {
            Ok(cast(call.arg(0), DataType::Date32))
        })
        .example("DATE(2024, 5, 17)", "DATE '2024-05-17'")
        .example(
            "DATE(TIMESTAMP '2024-05-18 03:00:00+00', 'America/Los_Angeles')",
            "DATE '2024-05-17'",
        )
        .example("DATE(DATETIME '2024-05-17 23:00:00')", "DATE '2024-05-17'"),
        Function::scalar(
            "DATETIME",
            "Returns a date and time from its parts, or the date and time of a value.",
        )
        .param("year", "The year.")
        .param("month", "The month, from 1 to 12.")
        .param("day", "The day of the month.")
        .param("hour", "The hour, from 0 to 23.")
        .param("minute", "The minute, from 0 to 59.")
        .param("second", "The second, from 0 to 59.")
        .param(
            "value",
            "A `DATE`, which starts at midnight, a `TIMESTAMP`, whose civil time is its time in \
             the time zone, a `DATETIME`, or a `STRING` such as `'2024-05-17 13:45:10'`.",
        )
        .param(
            "time",
            "The time of day of the date. When omitted, midnight.",
        )
        .param("time_zone", ZONE)
        .form(
            "(year: INT64, month: INT64, day: INT64, hour: INT64, minute: INT64, second: INT64) \
             -> DATETIME",
            |call| {
                let date = f::make_date(
                    cast(call.arg(0), DataType::Int32),
                    cast(call.arg(1), DataType::Int32),
                    cast(call.arg(2), DataType::Int32),
                );
                let seconds = days(date) * lit_i64(86_400)
                    + call.arg(3) * lit_i64(3_600)
                    + call.arg(4) * lit_i64(60)
                    + call.arg(5);
                Ok(datetime_of_micros(seconds * lit_i64(1_000_000)))
            },
        )
        .form("(value: DATE[, time: TIME]) -> DATETIME", |call| {
            let date = cast(call.arg(0), types::datetime());
            Ok(match call.value(1) {
                Some(time) => {
                    datetime_of_micros(micros(date) + cast(time.clone(), DataType::Int64))
                }
                None => date,
            })
        })
        .form(
            "(value: TIMESTAMP[, time_zone: CONST STRING]) -> DATETIME",
            |call| {
                let zone = time_zone(call, 1)?;
                Ok(local(call.arg(0), &zone))
            },
        )
        .form("(value: DATETIME) -> DATETIME", |call| Ok(call.arg(0)))
        .form("(value: STRING) -> DATETIME", |call| {
            Ok(cast(call.arg(0), types::datetime()))
        })
        .example(
            "DATETIME(2024, 5, 17, 13, 45, 10)",
            "DATETIME '2024-05-17 13:45:10'",
        )
        .example(
            "DATETIME(DATE '2024-05-17')",
            "DATETIME '2024-05-17 00:00:00'",
        ),
        Function::scalar("TIMESTAMP", "Returns the instant of a value.")
            .param(
                "value",
                "A `STRING` such as `'2024-05-17 13:45:10'`, a `DATE`, which starts at midnight, \
                 a `DATETIME`, each in the time zone, or a `TIMESTAMP`.",
            )
            .param("time_zone", ZONE)
            .form(
                "(value: STRING[, time_zone: CONST STRING]) -> TIMESTAMP",
                |call| {
                    let zone = time_zone(call, 1)?;
                    Ok(instant(f::to_timestamp(vec![call.arg(0)]), &zone))
                },
            )
            .form(
                "(value: DATE[, time_zone: CONST STRING]) -> TIMESTAMP",
                |call| {
                    let zone = time_zone(call, 1)?;
                    Ok(instant(cast(call.arg(0), types::datetime()), &zone))
                },
            )
            .form(
                "(value: DATETIME[, time_zone: CONST STRING]) -> TIMESTAMP",
                |call| {
                    let zone = time_zone(call, 1)?;
                    Ok(instant(call.arg(0), &zone))
                },
            )
            .form("(value: TIMESTAMP) -> TIMESTAMP", |call| Ok(call.arg(0)))
            .example(
                "TIMESTAMP('2024-05-17 13:45:10')",
                "TIMESTAMP '2024-05-17 13:45:10+00'",
            )
            .example(
                "TIMESTAMP(DATE '2024-05-17', 'America/Los_Angeles')",
                "TIMESTAMP '2024-05-17 07:00:00+00'",
            ),
        Function::scalar(
            "DATE_TRUNC",
            "Truncates a date, or a date and time, to the start of a part.",
        )
        .param("value", "The `DATE`, `DATETIME` or `TIMESTAMP`.")
        .param(
            "part",
            "The part: `DAY`, `WEEK`, which starts on Sunday, `WEEK(day)`, which starts on the \
             day, `ISOWEEK`, `MONTH`, `QUARTER`, `YEAR` or `ISOYEAR`, and for a `DATETIME` or a \
             `TIMESTAMP`, a part of the time, from `MICROSECOND` to `HOUR`.",
        )
        .param("time_zone", ZONE)
        .form("(value: DATE, part: PART(DATE)) -> DATE", |call| {
            date_trunc(call, call.arg(0), call.date_part(1))
        })
        .form(
            "(value: DATETIME, part: PART(DATETIME)) -> DATETIME",
            |call| datetime_trunc(call, call.arg(0), call.date_part(1)),
        )
        .form(
            "(value: TIMESTAMP, part: PART(DATETIME)[, time_zone: CONST STRING]) -> TIMESTAMP",
            |call| timestamp_trunc(call, call.arg(0), call.date_part(1), 2),
        )
        .safe()
        .example("DATE_TRUNC(DATE '2024-05-17', MONTH)", "DATE '2024-05-01'")
        .example("DATE_TRUNC(DATE '2024-05-17', WEEK)", "DATE '2024-05-12'")
        .example(
            "DATE_TRUNC(DATE '2024-05-17', WEEK(MONDAY))",
            "DATE '2024-05-13'",
        ),
        Function::scalar(
            "DATETIME_TRUNC",
            "Truncates a date and time to the start of a part.",
        )
        .param(
            "value",
            "The `DATETIME`, or a `TIMESTAMP`, which truncates in the time zone.",
        )
        .param(
            "part",
            "The part, from `MICROSECOND` to `ISOYEAR`, as for `DATE_TRUNC`.",
        )
        .param("time_zone", ZONE)
        .form(
            "(value: DATETIME, part: PART(DATETIME)) -> DATETIME",
            |call| datetime_trunc(call, call.arg(0), call.date_part(1)),
        )
        .form(
            "(value: TIMESTAMP, part: PART(DATETIME)[, time_zone: CONST STRING]) -> TIMESTAMP",
            |call| timestamp_trunc(call, call.arg(0), call.date_part(1), 2),
        )
        .safe()
        .example(
            "DATETIME_TRUNC(DATETIME '2024-05-17 13:45:10', HOUR)",
            "DATETIME '2024-05-17 13:00:00'",
        ),
        Function::scalar(
            "TIMESTAMP_TRUNC",
            "Truncates an instant to the start of a part, in a time zone.",
        )
        .param("value", "The `TIMESTAMP`, or a `DATETIME`.")
        .param(
            "part",
            "The part, from `MICROSECOND` to `ISOYEAR`, as for `DATE_TRUNC`.",
        )
        .param("time_zone", ZONE)
        .form(
            "(value: TIMESTAMP, part: PART(DATETIME)[, time_zone: CONST STRING]) -> TIMESTAMP",
            |call| timestamp_trunc(call, call.arg(0), call.date_part(1), 2),
        )
        .form(
            "(value: DATETIME, part: PART(DATETIME)) -> DATETIME",
            |call| datetime_trunc(call, call.arg(0), call.date_part(1)),
        )
        .safe()
        .example(
            "TIMESTAMP_TRUNC(TIMESTAMP '2024-05-17 13:45:10+00', HOUR)",
            "TIMESTAMP '2024-05-17 13:00:00+00'",
        )
        .example(
            "TIMESTAMP_TRUNC(TIMESTAMP '2024-05-18 03:00:00+00', DAY, 'America/Los_Angeles')",
            "TIMESTAMP '2024-05-17 07:00:00+00'",
        ),
        shift(
            "DATE_ADD",
            "Adds an interval to a date.",
            Operator::Plus,
            &["DATE", "DATETIME", "TIMESTAMP"],
        )
        .example(
            "DATE_ADD(DATE '2024-01-31', INTERVAL 1 MONTH)",
            "DATE '2024-02-29'",
        ),
        shift(
            "DATE_SUB",
            "Subtracts an interval from a date.",
            Operator::Minus,
            &["DATE", "DATETIME", "TIMESTAMP"],
        )
        .example(
            "DATE_SUB(DATE '2024-03-01', INTERVAL 1 DAY)",
            "DATE '2024-02-29'",
        ),
        shift(
            "DATETIME_ADD",
            "Adds an interval to a date and time.",
            Operator::Plus,
            &["DATETIME", "TIMESTAMP"],
        )
        .example(
            "DATETIME_ADD(DATETIME '2024-05-17 13:45:10', INTERVAL 30 MINUTE)",
            "DATETIME '2024-05-17 14:15:10'",
        ),
        shift(
            "DATETIME_SUB",
            "Subtracts an interval from a date and time.",
            Operator::Minus,
            &["DATETIME", "TIMESTAMP"],
        )
        .example(
            "DATETIME_SUB(DATETIME '2024-05-17 13:45:10', INTERVAL 2 HOUR)",
            "DATETIME '2024-05-17 11:45:10'",
        ),
        shift(
            "TIMESTAMP_ADD",
            "Adds an interval to an instant.",
            Operator::Plus,
            &["TIMESTAMP", "DATETIME"],
        )
        .example(
            "TIMESTAMP_ADD(TIMESTAMP '2024-05-17 13:45:10+00', INTERVAL 30 MINUTE)",
            "TIMESTAMP '2024-05-17 14:15:10+00'",
        ),
        shift(
            "TIMESTAMP_SUB",
            "Subtracts an interval from an instant.",
            Operator::Minus,
            &["TIMESTAMP", "DATETIME"],
        )
        .example(
            "TIMESTAMP_SUB(TIMESTAMP '2024-05-17 13:45:10+00', INTERVAL 1 DAY)",
            "TIMESTAMP '2024-05-16 13:45:10+00'",
        ),
        Function::scalar(
            "DATE_DIFF",
            "Returns the number of boundaries of a part between two dates.",
        )
        .description(
            "Counts the starts of the part after the start date, up to and with the end date, \
             so `DATE_DIFF` of January 1 and the December 31 before it in `YEAR` is 1. The \
             result is negative when the end date comes first.",
        )
        .param(
            "end_date",
            "The later date, a `DATE`, a `DATETIME` or a `TIMESTAMP`.",
        )
        .param("start_date", "The earlier date, of the type of `end_date`.")
        .param(
            "part",
            "The part: `DAY`, `WEEK`, `WEEK(day)`, `ISOWEEK`, `MONTH`, `QUARTER`, `YEAR` or \
             `ISOYEAR`.",
        )
        .form(
            "(end_date: T, start_date: T, part: PART(DATE)) -> INT64 where T: DATE | DATETIME \
             | TIMESTAMP",
            |call| {
                let end = cast(call.arg(0), DataType::Date32);
                let start = cast(call.arg(1), DataType::Date32);
                date_diff(call, end, start, call.date_part(2))
            },
        )
        .safe()
        .example("DATE_DIFF(DATE '2024-03-01', DATE '2024-01-01', DAY)", "60")
        .example(
            "DATE_DIFF(DATE '2024-03-01', DATE '2024-01-31', MONTH)",
            "2",
        )
        .example("DATE_DIFF(DATE '2024-01-01', DATE '2023-12-31', YEAR)", "1"),
        Function::scalar(
            "TIMESTAMP_DIFF",
            "Returns the number of whole parts of the time between two instants.",
        )
        .description("The result truncates toward zero, and is negative when the end comes first.")
        .param(
            "end_timestamp",
            "The later instant, a `TIMESTAMP` or a `DATETIME`.",
        )
        .param(
            "start_timestamp",
            "The earlier instant, of the type of `end_timestamp`.",
        )
        .param(
            "part",
            "The part, of a fixed length: `MICROSECOND`, `MILLISECOND`, `SECOND`, `MINUTE`, \
             `HOUR` or `DAY`, which is 24 hours.",
        )
        .form(
            "(end_timestamp: T, start_timestamp: T, part: PART(FIXED)) -> INT64 where T: \
             TIMESTAMP | DATETIME",
            |call| {
                let unit = call
                    .date_part(2)
                    .micros()
                    .expect("a part of a fixed length");
                let difference = instant_micros(call.arg(0)) - instant_micros(call.arg(1));
                // Integer division truncates toward zero, as GoogleSQL counts whole parts.
                Ok(difference / lit_i64(unit))
            },
        )
        .safe()
        .example(
            "TIMESTAMP_DIFF(TIMESTAMP '2024-05-17 13:45:10+00', TIMESTAMP '2024-05-17 \
             12:00:00+00', MINUTE)",
            "105",
        )
        .example(
            "TIMESTAMP_DIFF(TIMESTAMP '2024-05-17 13:45:10+00', TIMESTAMP '2024-05-17 \
             12:00:00+00', HOUR)",
            "1",
        ),
        Function::scalar("FORMAT_DATE", "Formats a date as a string.")
            .param("format", FORMAT)
            .param(
                "value",
                "The `DATE`, or a `DATETIME` or a `TIMESTAMP`, whose date it formats.",
            )
            .param("time_zone", ZONE)
            .form("(format: STRING, value: DATE) -> STRING", |call| {
                Ok(f::to_char(call.arg(1), call.arg(0)))
            })
            .form("(format: STRING, value: DATETIME) -> STRING", |call| {
                Ok(f::to_char(call.arg(1), call.arg(0)))
            })
            .form(
                "(format: STRING, value: TIMESTAMP[, time_zone: CONST STRING]) -> STRING",
                |call| {
                    let zone = time_zone(call, 2)?;
                    Ok(f::to_char(local(call.arg(1), &zone), call.arg(0)))
                },
            )
            .note(
                "The format follows `strftime`, as DataFusion's `to_char` does, so GoogleSQL's \
                 own elements, such as `%Q`, are not elements of it.",
            )
            .example("FORMAT_DATE('%Y/%m/%d', DATE '2024-05-07')", "'2024/05/07'")
            .example("FORMAT_DATE('%b %e', DATE '2024-05-07')", "'May  7'"),
        Function::scalar("FORMAT_DATETIME", "Formats a date and time as a string.")
            .param("format", FORMAT)
            .param(
                "value",
                "The `DATETIME`, or a `TIMESTAMP` in the time zone.",
            )
            .param("time_zone", ZONE)
            .form("(format: STRING, value: DATETIME) -> STRING", |call| {
                Ok(f::to_char(call.arg(1), call.arg(0)))
            })
            .form(
                "(format: STRING, value: TIMESTAMP[, time_zone: CONST STRING]) -> STRING",
                |call| {
                    let zone = time_zone(call, 2)?;
                    Ok(f::to_char(local(call.arg(1), &zone), call.arg(0)))
                },
            )
            .example(
                "FORMAT_DATETIME('%Y-%m-%d %H:%M', DATETIME '2024-05-17 13:45:10')",
                "'2024-05-17 13:45'",
            ),
        Function::scalar(
            "FORMAT_TIMESTAMP",
            "Formats an instant as a string, in a time zone.",
        )
        .param("format", FORMAT)
        .param("value", "The `TIMESTAMP`, or a `DATETIME`.")
        .param("time_zone", ZONE)
        .form(
            "(format: STRING, value: TIMESTAMP[, time_zone: CONST STRING]) -> STRING",
            |call| {
                let zone = time_zone(call, 2)?;
                Ok(f::to_char(local(call.arg(1), &zone), call.arg(0)))
            },
        )
        .form("(format: STRING, value: DATETIME) -> STRING", |call| {
            Ok(f::to_char(call.arg(1), call.arg(0)))
        })
        .example(
            "FORMAT_TIMESTAMP('%Y-%m-%d %H:%M:%S', TIMESTAMP '2024-05-17 13:45:10+00')",
            "'2024-05-17 13:45:10'",
        )
        .example(
            "FORMAT_TIMESTAMP('%H:%M', TIMESTAMP '2024-05-17 13:45:10+00', \
             'America/Los_Angeles')",
            "'06:45'",
        ),
        Function::scalar("PARSE_DATE", "Parses a date from a string in a format.")
            .description("The string matches the whole format.")
            .param("format", FORMAT)
            .param("value", "The string.")
            .form("(format: STRING, value: STRING) -> DATE", |call| {
                Ok(f::to_date(vec![call.arg(1), call.arg(0)]))
            })
            .example("PARSE_DATE('%Y%m%d', '20240517')", "DATE '2024-05-17'"),
        Function::scalar(
            "PARSE_DATETIME",
            "Parses a date and time from a string in a format.",
        )
        .param("format", FORMAT)
        .param("value", "The string.")
        .form("(format: STRING, value: STRING) -> DATETIME", |call| {
            Ok(cast(
                f::to_timestamp(vec![call.arg(1), call.arg(0)]),
                types::datetime(),
            ))
        })
        .example(
            "PARSE_DATETIME('%Y-%m-%d %H:%M', '2024-05-17 13:45')",
            "DATETIME '2024-05-17 13:45:00'",
        ),
        Function::scalar(
            "PARSE_TIMESTAMP",
            "Parses an instant from a string in a format and a time zone.",
        )
        .param("format", FORMAT)
        .param(
            "value",
            "The string. An offset in it, such as `+02:00`, takes the place of the time zone.",
        )
        .param("time_zone", ZONE)
        .form(
            "(format: STRING, value: STRING[, time_zone: CONST STRING]) -> TIMESTAMP",
            |call| {
                let zone = time_zone(call, 2)?;
                Ok(instant(
                    f::to_timestamp(vec![call.arg(1), call.arg(0)]),
                    &zone,
                ))
            },
        )
        .example(
            "PARSE_TIMESTAMP('%Y-%m-%d %H:%M:%S', '2024-05-17 13:45:10')",
            "TIMESTAMP '2024-05-17 13:45:10+00'",
        )
        .example(
            "PARSE_TIMESTAMP('%Y-%m-%d %H:%M:%S', '2024-05-17 13:45:10', 'America/Los_Angeles')",
            "TIMESTAMP '2024-05-17 20:45:10+00'",
        ),
        epoch(
            "UNIX_SECONDS",
            "Returns the number of seconds since 1970-01-01 00:00:00 UTC of an instant.",
            1_000_000,
        )
        .example(
            "UNIX_SECONDS(TIMESTAMP '2024-01-01 00:00:00+00')",
            "1704067200",
        ),
        epoch(
            "UNIX_MILLIS",
            "Returns the number of milliseconds since 1970-01-01 00:00:00 UTC of an instant.",
            1_000,
        )
        .example(
            "UNIX_MILLIS(TIMESTAMP '2024-01-01 00:00:00+00')",
            "1704067200000",
        ),
        epoch(
            "UNIX_MICROS",
            "Returns the number of microseconds since 1970-01-01 00:00:00 UTC of an instant.",
            1,
        )
        .example(
            "UNIX_MICROS(TIMESTAMP '2024-01-01 00:00:00+00')",
            "1704067200000000",
        ),
        from_epoch(
            "TIMESTAMP_SECONDS",
            "Returns the instant of a number of seconds since 1970-01-01 00:00:00 UTC.",
            1_000_000,
        )
        .example(
            "TIMESTAMP_SECONDS(1704067200)",
            "TIMESTAMP '2024-01-01 00:00:00+00'",
        ),
        from_epoch(
            "TIMESTAMP_MILLIS",
            "Returns the instant of a number of milliseconds since 1970-01-01 00:00:00 UTC.",
            1_000,
        )
        .example(
            "TIMESTAMP_MILLIS(1704067200000)",
            "TIMESTAMP '2024-01-01 00:00:00+00'",
        ),
        from_epoch(
            "TIMESTAMP_MICROS",
            "Returns the instant of a number of microseconds since 1970-01-01 00:00:00 UTC.",
            1,
        )
        .example(
            "TIMESTAMP_MICROS(1704067200000000)",
            "TIMESTAMP '2024-01-01 00:00:00+00'",
        ),
    ]
}

/// `DATE_ADD(value, interval_value)` and its kin, which add an `INTERVAL` to a value of each of
/// the types `types`.
fn shift(name: &str, summary: &str, op: Operator, types: &[&str]) -> Function {
    let mut function = Function::scalar(name, summary)
        .description(
            "Months and years keep the day of the month, or take the last day of a shorter \
             month.",
        )
        .param("value", "The value.")
        .param("interval_value", "The interval, as in `INTERVAL 1 MONTH`.")
        .safe();
    for ty in types {
        function = function.form(
            &format!("(value: {ty}, interval_value: INTERVAL) -> {ty}"),
            move |call| {
                Ok(datafusion::logical_expr::binary_expr(
                    call.arg(0),
                    op,
                    call.arg(1),
                ))
            },
        );
    }
    function
}

/// `UNIX_SECONDS(value)` and its kin, the number of `unit` microseconds since the epoch.
fn epoch(name: &str, summary: &str, unit: i64) -> Function {
    Function::scalar(name, summary)
        .description("Truncates toward zero.")
        .param("value", "The instant.")
        .form("(value: TIMESTAMP) -> INT64", move |call| {
            Ok(instant_micros(call.arg(0)) / lit_i64(unit))
        })
        .safe()
}

/// `TIMESTAMP_SECONDS(value)` and its kin, the instant of a number of `unit` microseconds since
/// the epoch.
fn from_epoch(name: &str, summary: &str, unit: i64) -> Function {
    Function::scalar(name, summary)
        .param("value", "The number.")
        .form("(value: INT64) -> TIMESTAMP", move |call| {
            Ok(instant_of_micros(call.arg(0) * lit_i64(unit)))
        })
        .safe()
}

/// The time zone of the parameter at `index`, a string literal, or the default.
fn time_zone(call: &Call, index: usize) -> Result<String, CallError> {
    let Some(value) = call.value(index) else {
        return Ok(call.context.time_zone.to_string());
    };
    match value {
        Expr::Literal(value, _) => match value.try_as_str() {
            Some(Some(zone)) => Ok(zone.to_string()),
            _ => Err(call.error_at(index, "a time zone is a string")),
        },
        _ => Err(call.error_at(
            index,
            "avenger-sql takes a time zone as a string literal, such as 'America/New_York'",
        )),
    }
}

/// The civil date and time in the time zone `zone` of the instant `value`.
fn local(value: Expr, zone: &str) -> Expr {
    let zoned = cast(
        value,
        DataType::Timestamp(TimeUnit::Microsecond, Some(Arc::from(zone))),
    );
    cast(f::to_local_time(vec![zoned]), types::datetime())
}

/// The instant of the civil date and time `value` in the time zone `zone`.
fn instant(value: Expr, zone: &str) -> Expr {
    let civil = cast(value, types::datetime());
    let zoned = cast(
        civil,
        DataType::Timestamp(TimeUnit::Microsecond, Some(Arc::from(zone))),
    );
    cast(zoned, types::timestamp())
}

/// The microseconds since the epoch of the instant `value`.
fn instant_micros(value: Expr) -> Expr {
    micros(cast(value, types::timestamp()))
}

/// The microseconds of a timestamp of microseconds.
fn micros(value: Expr) -> Expr {
    cast(value, DataType::Int64)
}

fn instant_of_micros(micros: Expr) -> Expr {
    cast(micros, types::timestamp())
}

fn datetime_of_micros(micros: Expr) -> Expr {
    cast(micros, types::datetime())
}

/// The days since 1970-01-01 of the date `date`.
fn days(date: Expr) -> Expr {
    cast(cast(date, DataType::Int32), DataType::Int64)
}

fn date_of_days(days: Expr) -> Expr {
    cast(cast(days, DataType::Int32), DataType::Date32)
}

/// The day number of the start of the week, starting on `start`, of the day number `days`.
/// 1970-01-01, day 0, is a Thursday.
fn week_start(days: Expr, start: Weekday) -> Expr {
    let offset = (days.clone() + lit_i64(4 - start.number())) % lit_i64(7);
    // The remainder of a negative day number is negative, so it adds 7 before the remainder.
    days - (offset + lit_i64(7)) % lit_i64(7)
}

/// The year of the date `date`.
fn year(date: Expr) -> Expr {
    cast(f::date_part(lit_str("year"), date), DataType::Int64)
}

fn month(date: Expr) -> Expr {
    cast(f::date_part(lit_str("month"), date), DataType::Int64)
}

/// The ISO 8601 year of the day number `days`: the year of the Thursday of its week.
fn iso_year(days: Expr) -> Expr {
    year(date_of_days(week_start(days, Weekday::Monday) + lit_i64(3)))
}

/// The day number of the first day of the ISO 8601 year of the day number `days`: the Monday of
/// the week of its January 4.
fn iso_year_start(days_of: Expr) -> Expr {
    let january_4 = f::make_date(
        cast(iso_year(days_of), DataType::Int32),
        cast(lit_i64(1), DataType::Int32),
        cast(lit_i64(4), DataType::Int32),
    );
    week_start(days(january_4), Weekday::Monday)
}

fn date_trunc(call: &Call, date: Expr, part: DatePart) -> Result<Expr, CallError> {
    let truncated = match part {
        DatePart::Day => date,
        DatePart::Week(start) => date_of_days(week_start(days(date), start)),
        DatePart::IsoWeek => date_of_days(week_start(days(date), Weekday::Monday)),
        DatePart::IsoYear => date_of_days(iso_year_start(days(date))),
        DatePart::Month | DatePart::Quarter | DatePart::Year => {
            let granularity = part.granularity().expect("a part of date_trunc");
            cast(
                f::date_trunc(lit_str(granularity), cast(date, types::datetime())),
                DataType::Date32,
            )
        }
        _ => {
            return Err(call.error_at(
                1,
                format!(
                    "DATE_TRUNC truncates a date to a DAY, WEEK, ISOWEEK, MONTH, QUARTER, YEAR or ISOYEAR, not {}",
                    part.name()
                ),
            ))
        }
    };
    Ok(truncated)
}

fn datetime_trunc(call: &Call, value: Expr, part: DatePart) -> Result<Expr, CallError> {
    if let Some(granularity) = part.granularity() {
        return Ok(cast(
            f::date_trunc(lit_str(granularity), value),
            types::datetime(),
        ));
    }
    let date = date_trunc(call, cast(value, DataType::Date32), part)?;
    Ok(cast(date, types::datetime()))
}

/// The instant `value` truncated to `part` in the time zone of the parameter at `zone_index`.
fn timestamp_trunc(
    call: &Call,
    value: Expr,
    part: DatePart,
    zone_index: usize,
) -> Result<Expr, CallError> {
    let zone = time_zone(call, zone_index)?;
    let truncated = datetime_trunc(call, local(value, &zone), part)?;
    Ok(instant(truncated, &zone))
}

/// The number of boundaries of `part` from the date `start` to the date `end`.
fn date_diff(call: &Call, end: Expr, start: Expr, part: DatePart) -> Result<Expr, CallError> {
    let months = |date: Expr| year(date.clone()) * lit_i64(12) + month(date);
    let quarters =
        |date: Expr| year(date.clone()) * lit_i64(4) + (month(date) - lit_i64(1)) / lit_i64(3);
    let difference = match part {
        DatePart::Day => days(end) - days(start),
        DatePart::Week(day) => {
            (week_start(days(end), day) - week_start(days(start), day)) / lit_i64(7)
        }
        DatePart::IsoWeek => {
            (week_start(days(end), Weekday::Monday) - week_start(days(start), Weekday::Monday))
                / lit_i64(7)
        }
        DatePart::Month => months(end) - months(start),
        DatePart::Quarter => quarters(end) - quarters(start),
        DatePart::Year => year(end) - year(start),
        DatePart::IsoYear => iso_year(days(end)) - iso_year(days(start)),
        _ => {
            return Err(call.error_at(
                2,
                format!(
                    "DATE_DIFF counts DAY, WEEK, ISOWEEK, MONTH, QUARTER, YEAR or ISOYEAR, not {}",
                    part.name()
                ),
            ))
        }
    };
    Ok(difference)
}
