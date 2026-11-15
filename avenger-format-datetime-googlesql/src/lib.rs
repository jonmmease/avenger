#![doc = include_str!("../README.md")]

mod element;
mod format;
mod locale;
mod parse;
mod provider;

pub use element::Kind;
pub use format::Format;
pub use locale::Locale;
pub use parse::{parse_date, parse_datetime, parse_time, parse_timestamp};
pub use provider::GoogleSqlDateTimeFormatProvider;

/// An error of a format or of a value, with GoogleSQL's message.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct Error(pub String);

/// The week of the year of `date` for weeks that start on `start`, as `%U` (Sunday) and `%W`
/// (Monday) number them: the days before the first `start` of the year are week 0.
pub(crate) fn week_number(date: chrono::NaiveDate, start: chrono::Weekday) -> i64 {
    use chrono::Datelike;
    let since_start =
        (date.weekday().num_days_from_monday() + 7 - start.num_days_from_monday()) % 7;
    i64::from((date.ordinal0() + 7 - since_start) / 7)
}
