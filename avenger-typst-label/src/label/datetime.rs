//! The `datetime` function, which builds the dates and datetimes that `#datetimefmt` formats.
//!
//! Upstream builds a datetime by calling its `datetime` type. Labels have no type constructors,
//! so this function takes the type's name and its arguments, validated with chrono, plus
//! `nanosecond`, for chrono's precision, and `utc`, for an instant.

#![allow(
    clippy::too_many_arguments,
    reason = "Each of `datetime`'s named arguments is a parameter of the function."
)]

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use ecow::{EcoString, eco_format};

use crate::typst_library::diag::{HintedStrResult, bail};
use crate::typst_library::foundations::{Datetime, func};

func! {
/// Builds a date from `year`, `month` and `day`, and a naive datetime with `hour`,
/// `minute` and `second` as well, or an instant with `utc`.
///
/// ```example
/// #datetimefmt(datetime(year: 2024, month: 1, day: 5), "%B %-d, %Y")
/// ```
#[func]
pub fn datetime(
    /// The year.
    #[named]
    year: Option<i32>,
    /// The month, from 1 to 12.
    #[named]
    month: Option<u8>,
    /// The day of the month.
    #[named]
    day: Option<u8>,
    /// The hour.
    #[named]
    hour: Option<u8>,
    /// The minute.
    #[named]
    minute: Option<u8>,
    /// The second.
    #[named]
    second: Option<u8>,
    /// The nanosecond within the second. A leap second is the 59th second with a
    /// nanosecond from 1,000,000,000.
    #[named]
    nanosecond: Option<u32>,
    /// Whether the date and time are in UTC, which makes the datetime an instant that
    /// formatters show in their timezone.
    #[named]
    #[default(false)]
    utc: bool,
) -> HintedStrResult<Datetime> {
    let time = match (hour, minute, second) {
        (Some(hour), Some(minute), Some(second)) => match NaiveTime::from_hms_nano_opt(
            hour.into(),
            minute.into(),
            second.into(),
            nanosecond.unwrap_or(0),
        ) {
            Some(time) => Some(time),
            None => bail!("time is invalid"),
        },
        (None, None, None) => None,
        (hour, minute, second) => {
            let missing = [
                hour.is_none().then_some("`hour`"),
                minute.is_none().then_some("`minute`"),
                second.is_none().then_some("`second`"),
            ];
            bail!(
                "time is incomplete";
                hint: "add {} to get a valid time", missing_args(&missing);
            )
        }
    };

    let date = match (year, month, day) {
        (Some(year), Some(month), Some(day)) => {
            match NaiveDate::from_ymd_opt(year, month.into(), day.into()) {
                Some(date) => Some(date),
                None => bail!("date is invalid"),
            }
        }
        (None, None, None) => None,
        (year, month, day) => {
            let missing = [
                year.is_none().then_some("`year`"),
                month.is_none().then_some("`month`"),
                day.is_none().then_some("`day`"),
            ];
            bail!(
                "date is incomplete";
                hint: "add {} to get a valid date", missing_args(&missing);
            )
        }
    };

    Ok(match (date, time) {
        (Some(date), Some(time)) => {
            let datetime = NaiveDateTime::new(date, time);
            if utc {
                Datetime::Zoned(datetime.and_utc())
            } else {
                Datetime::Naive(datetime)
            }
        }
        (Some(date), None) => {
            if nanosecond.is_some() {
                bail!(
                    "`nanosecond` needs a time";
                    hint: "add the `hour`, `minute`, and `second` arguments to get a valid time";
                )
            }
            if utc {
                bail!(
                    "`utc` needs a time";
                    hint: "add the `hour`, `minute`, and `second` arguments to get a valid time";
                )
            }
            Datetime::Date(date)
        }
        // avenger: a label has no times without dates.
        (None, Some(_)) => bail!(
            "times without dates are not supported in labels";
            hint: "add the `year`, `month`, and `day` arguments to get a valid date";
        ),
        (None, None) => bail!(
            "at least one of date or time must be fully specified";
            hint: "add the `hour`, `minute`, and `second` arguments to get a valid time";
            hint: "add the `year`, `month`, and `day` arguments to get a valid date";
        ),
    })
}
}

/// Upstream's list of the arguments a date or time lacks, as in "the `year` and `day`
/// arguments".
fn missing_args(missing: &[Option<&str>]) -> EcoString {
    let args: Vec<&str> = missing.iter().flatten().copied().collect();
    match args.as_slice() {
        [] => unreachable!("an incomplete date or time lacks an argument"),
        [arg] => eco_format!("the {arg} argument"),
        [arg1, arg2] => eco_format!("the {arg1} and {arg2} arguments"),
        [args @ .., tail] => eco_format!("the {}, and {tail} arguments", args.join(", ")),
    }
}
