use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use std::fmt;

/// Values to label, in the form each kind of formatter accepts. `None` is a missing value.
#[derive(Debug, Clone, Copy)]
pub enum FormatValues<'a> {
    Numbers(&'a [f64]),
    Dates(&'a [Option<NaiveDate>]),
    NaiveDateTimes(&'a [Option<NaiveDateTime>]),
    ZonedDateTimes(&'a [Option<DateTime<Utc>>]),
}

/// The kind of value that a formatter accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    Number,
    Date,
    NaiveDateTime,
    ZonedDateTime,
}

impl fmt::Display for ValueKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Number => "number",
            Self::Date => "date",
            Self::NaiveDateTime => "naive datetime",
            Self::ZonedDateTime => "zoned datetime",
        })
    }
}

impl FormatValues<'_> {
    pub fn kind(&self) -> ValueKind {
        match self {
            Self::Numbers(_) => ValueKind::Number,
            Self::Dates(_) => ValueKind::Date,
            Self::NaiveDateTimes(_) => ValueKind::NaiveDateTime,
            Self::ZonedDateTimes(_) => ValueKind::ZonedDateTime,
        }
    }
}
