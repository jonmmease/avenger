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

use super::{cast, lit_i64, lit_str, Call, CallError, Function, Param};
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
    fn micros(self) -> Option<i64> {
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

pub(super) fn functions() -> Vec<Function> {
    let value = || Param::required("value");
    let part = || Param::date_part("part");
    let zone = || Param::optional("time_zone");
    vec![
        Function::scalar("CURRENT_DATE", vec![zone()], |call| {
            let zone = time_zone(call, 0)?;
            Ok(cast(local(f::now(), &zone), DataType::Date32))
        })
        .safe(),
        Function::scalar("CURRENT_DATETIME", vec![zone()], |call| {
            let zone = time_zone(call, 0)?;
            Ok(local(f::now(), &zone))
        })
        .safe(),
        Function::scalar("CURRENT_TIMESTAMP", vec![], |_| {
            Ok(cast(f::now(), types::timestamp()))
        })
        .safe(),
        // DATE(year, month, day), DATE(timestamp [, time_zone]), or DATE(datetime).
        Function::scalar(
            "DATE",
            vec![
                Param::required("year"),
                Param::optional("month"),
                Param::optional("day"),
            ],
            |call| {
                if let (Some(month), Some(day)) = (call.value(1), call.value(2)) {
                    return Ok(f::make_date(
                        cast(call.arg(0), DataType::Int32),
                        cast(month.clone(), DataType::Int32),
                        cast(day.clone(), DataType::Int32),
                    ));
                }
                let value = call.arg(0);
                match call.type_of(&value)? {
                    DataType::Timestamp(_, Some(_)) => {
                        let zone = time_zone(call, 1)?;
                        Ok(cast(local(value, &zone), DataType::Date32))
                    }
                    DataType::Timestamp(_, None) | DataType::Date32 => {
                        Ok(cast(value, DataType::Date32))
                    }
                    other => Err(call.error_at(
                        0,
                        format!(
                            "DATE takes a year, a month and a day, a TIMESTAMP, or a DATETIME, not {}",
                            types::googlesql_name(&other)
                        ),
                    )),
                }
            },
        ),
        // DATETIME(year, month, day, hour, minute, second), DATETIME(date [, time]), or
        // DATETIME(timestamp [, time_zone]).
        Function::scalar(
            "DATETIME",
            vec![
                Param::required("year"),
                Param::optional("month"),
                Param::optional("day"),
                Param::optional("hour"),
                Param::optional("minute"),
                Param::optional("second"),
            ],
            |call| {
                if let Some(day) = call.value(2) {
                    let date = f::make_date(
                        cast(call.arg(0), DataType::Int32),
                        cast(call.arg(1), DataType::Int32),
                        cast(day.clone(), DataType::Int32),
                    );
                    let int = |index: usize| {
                        cast(
                            call.value(index).cloned().unwrap_or_else(|| lit_i64(0)),
                            DataType::Int64,
                        )
                    };
                    let seconds = days(date) * lit_i64(86_400)
                        + int(3) * lit_i64(3_600)
                        + int(4) * lit_i64(60)
                        + int(5);
                    return Ok(datetime_of_micros(seconds * lit_i64(1_000_000)));
                }
                let value = call.arg(0);
                match call.type_of(&value)? {
                    DataType::Date32 => {
                        let date = cast(value, types::datetime());
                        Ok(match call.value(1) {
                            Some(time) => datetime_of_micros(
                                micros(date)
                                    + cast(time.clone(), DataType::Int64),
                            ),
                            None => date,
                        })
                    }
                    DataType::Timestamp(_, Some(_)) => {
                        let zone = time_zone(call, 1)?;
                        Ok(local(value, &zone))
                    }
                    DataType::Timestamp(_, None) => Ok(cast(value, types::datetime())),
                    other => Err(call.error_at(
                        0,
                        format!(
                            "DATETIME takes the parts of a date and time, a DATE, or a TIMESTAMP, not {}",
                            types::googlesql_name(&other)
                        ),
                    )),
                }
            },
        ),
        // TIMESTAMP(string [, time_zone]), TIMESTAMP(date [, time_zone]), or
        // TIMESTAMP(datetime [, time_zone]).
        Function::scalar("TIMESTAMP", vec![value(), zone()], |call| {
            let value = call.arg(0);
            let zone = time_zone(call, 1)?;
            match call.type_of(&value)? {
                DataType::Utf8 | DataType::Utf8View | DataType::LargeUtf8 => {
                    Ok(instant(f::to_timestamp(vec![value]), &zone))
                }
                DataType::Date32 => Ok(instant(cast(value, types::datetime()), &zone)),
                DataType::Timestamp(_, None) => Ok(instant(value, &zone)),
                DataType::Timestamp(_, Some(_)) => Ok(cast(value, types::timestamp())),
                other => Err(call.error_at(
                    0,
                    format!(
                        "TIMESTAMP takes a STRING, a DATE, or a DATETIME, not {}",
                        types::googlesql_name(&other)
                    ),
                )),
            }
        }),
        Function::scalar("DATE_TRUNC", vec![value(), part()], |call| {
            let value = call.arg(0);
            let part = call.date_part(1).expect("a bound date part");
            match call.type_of(&value)? {
                DataType::Timestamp(_, None) => datetime_trunc(call, value, part),
                DataType::Timestamp(_, Some(_)) => timestamp_trunc(call, value, part, 2),
                _ => date_trunc(call, cast(value, DataType::Date32), part),
            }
        })
        .safe(),
        Function::scalar("DATETIME_TRUNC", vec![value(), part()], |call| {
            let value = cast(call.arg(0), types::datetime());
            datetime_trunc(call, value, call.date_part(1).expect("a bound date part"))
        })
        .safe(),
        Function::scalar("TIMESTAMP_TRUNC", vec![value(), part(), zone()], |call| {
            let part = call.date_part(1).expect("a bound date part");
            timestamp_trunc(call, call.arg(0), part, 2)
        })
        .safe(),
        shift("DATE_ADD", Operator::Plus),
        shift("DATE_SUB", Operator::Minus),
        shift("DATETIME_ADD", Operator::Plus),
        shift("DATETIME_SUB", Operator::Minus),
        shift("TIMESTAMP_ADD", Operator::Plus),
        shift("TIMESTAMP_SUB", Operator::Minus),
        Function::scalar(
            "DATE_DIFF",
            vec![
                Param::required("end_date"),
                Param::required("start_date"),
                part(),
            ],
            |call| {
                let (end, start) = (
                    cast(call.arg(0), DataType::Date32),
                    cast(call.arg(1), DataType::Date32),
                );
                let part = call.date_part(2).expect("a bound date part");
                date_diff(call, end, start, part)
            },
        )
        .safe(),
        Function::scalar(
            "TIMESTAMP_DIFF",
            vec![
                Param::required("end_timestamp"),
                Param::required("start_timestamp"),
                part(),
            ],
            |call| {
                let part = call.date_part(2).expect("a bound date part");
                let Some(unit) = part.micros() else {
                    return Err(call.error_at(
                        2,
                        format!(
                            "TIMESTAMP_DIFF takes a part from MICROSECOND to DAY, not {}",
                            part.name()
                        ),
                    ));
                };
                let difference = instant_micros(call.arg(0)) - instant_micros(call.arg(1));
                // Integer division truncates toward zero, as GoogleSQL counts whole parts.
                Ok(difference / lit_i64(unit))
            },
        )
        .safe(),
        Function::scalar(
            "FORMAT_DATE",
            vec![Param::required("format"), value()],
            |call| Ok(f::to_char(cast(call.arg(1), DataType::Date32), call.arg(0))),
        ),
        Function::scalar(
            "FORMAT_DATETIME",
            vec![Param::required("format"), value()],
            |call| {
                Ok(f::to_char(
                    cast(call.arg(1), types::datetime()),
                    call.arg(0),
                ))
            },
        ),
        Function::scalar(
            "FORMAT_TIMESTAMP",
            vec![Param::required("format"), value(), zone()],
            |call| {
                let zone = time_zone(call, 2)?;
                Ok(f::to_char(local(call.arg(1), &zone), call.arg(0)))
            },
        ),
        Function::scalar(
            "PARSE_DATE",
            vec![Param::required("format"), value()],
            |call| Ok(f::to_date(vec![call.arg(1), call.arg(0)])),
        ),
        Function::scalar(
            "PARSE_DATETIME",
            vec![Param::required("format"), value()],
            |call| {
                Ok(cast(
                    f::to_timestamp(vec![call.arg(1), call.arg(0)]),
                    types::datetime(),
                ))
            },
        ),
        Function::scalar(
            "PARSE_TIMESTAMP",
            vec![Param::required("format"), value(), zone()],
            |call| {
                let zone = time_zone(call, 2)?;
                Ok(instant(
                    f::to_timestamp(vec![call.arg(1), call.arg(0)]),
                    &zone,
                ))
            },
        ),
        Function::scalar("UNIX_SECONDS", vec![value()], |call| {
            Ok(instant_micros(call.arg(0)) / lit_i64(1_000_000))
        })
        .safe(),
        Function::scalar("UNIX_MILLIS", vec![value()], |call| {
            Ok(instant_micros(call.arg(0)) / lit_i64(1_000))
        })
        .safe(),
        Function::scalar("UNIX_MICROS", vec![value()], |call| {
            Ok(instant_micros(call.arg(0)))
        })
        .safe(),
        Function::scalar("TIMESTAMP_SECONDS", vec![value()], |call| {
            Ok(instant_of_micros(
                cast(call.arg(0), DataType::Int64) * lit_i64(1_000_000),
            ))
        })
        .safe(),
        Function::scalar("TIMESTAMP_MILLIS", vec![value()], |call| {
            Ok(instant_of_micros(
                cast(call.arg(0), DataType::Int64) * lit_i64(1_000),
            ))
        })
        .safe(),
        Function::scalar("TIMESTAMP_MICROS", vec![value()], |call| {
            Ok(instant_of_micros(cast(call.arg(0), DataType::Int64)))
        })
        .safe(),
    ]
}

/// `DATE_ADD(value, interval)` and its kin, which add an `INTERVAL` to a date or a time.
fn shift(name: &str, op: Operator) -> Function {
    Function::scalar(
        name,
        vec![Param::required("value"), Param::required("interval_value")],
        move |call| {
            Ok(datafusion::logical_expr::binary_expr(
                call.arg(0),
                op,
                call.arg(1),
            ))
        },
    )
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
