//! Dates and datetimes, which labels format with `#datetimefmt`.
//!
//! avenger: in place of upstream's `foundations/datetime.rs`, which builds on the `time` crate
//! and has constructors, arithmetic and formatting. A label builds datetimes with the
//! `datetime` function (`label/datetime.rs`) to format them with `#datetimefmt`, so this is a
//! chrono value with upstream's repr.

use std::cmp::Ordering;

use chrono::{Datelike, Timelike};
use ecow::{EcoString, EcoVec, eco_format};

use crate::typst_library::foundations::{Repr, repr, ty};

/// A date, a naive datetime, or a zoned datetime.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum Datetime {
    /// A calendar date.
    Date(chrono::NaiveDate),
    /// A date and a time of day, without a timezone.
    Naive(chrono::NaiveDateTime),
    /// An instant, which formatters show in their timezone.
    Zoned(chrono::DateTime<chrono::Utc>),
}

ty!(Datetime, name = "datetime", title = "Datetime", long = "datetime");

impl Datetime {
    /// The fields upstream's repr shows: the date, and the time of day unless this is a date.
    fn fields(&self) -> (chrono::NaiveDate, Option<chrono::NaiveTime>) {
        match self {
            Self::Date(date) => (*date, None),
            Self::Naive(datetime) => (datetime.date(), Some(datetime.time())),
            Self::Zoned(instant) => (instant.date_naive(), Some(instant.time())),
        }
    }
}

impl Repr for Datetime {
    fn repr(&self) -> EcoString {
        let (date, time) = self.fields();
        let mut fields = EcoVec::new();
        fields.push(eco_format!("year: {}", (date.year() as i64).repr()));
        fields.push(eco_format!("month: {}", (date.month() as i64).repr()));
        fields.push(eco_format!("day: {}", (date.day() as i64).repr()));
        if let Some(time) = time {
            fields.push(eco_format!("hour: {}", (time.hour() as i64).repr()));
            fields.push(eco_format!("minute: {}", (time.minute() as i64).repr()));
            fields.push(eco_format!("second: {}", (time.second() as i64).repr()));
        }
        eco_format!("datetime{}", &repr::pretty_array_like(&fields, false))
    }
}

impl PartialOrd for Datetime {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match (self, other) {
            (Self::Date(a), Self::Date(b)) => a.partial_cmp(b),
            (Self::Naive(a), Self::Naive(b)) => a.partial_cmp(b),
            (Self::Zoned(a), Self::Zoned(b)) => a.partial_cmp(b),
            _ => None,
        }
    }
}
