use crate::{
    DateTimeFormatError, PreparedDateFormatter, PreparedNaiveDateTimeFormatter,
    PreparedNumberFormatter, PreparedZonedDateTimeFormatter, TickSpacing,
};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use std::{fmt, sync::Arc};

/// A prepared formatter for one kind of value, for consumers such as axes that label whichever
/// values they are given.
#[derive(Debug, Clone)]
pub enum PreparedFormatter {
    Number(Arc<dyn PreparedNumberFormatter>),
    Date(Arc<dyn PreparedDateFormatter>),
    NaiveDateTime(Arc<dyn PreparedNaiveDateTimeFormatter>),
    ZonedDateTime(Arc<dyn PreparedZonedDateTimeFormatter>),
}

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

/// Errors from labeling values with a [`PreparedFormatter`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FormatError {
    #[error("a {formatter} formatter cannot label {values} values")]
    Mismatch {
        formatter: ValueKind,
        values: ValueKind,
    },
    #[error(transparent)]
    DateTime(#[from] DateTimeFormatError),
}

impl PreparedFormatter {
    pub fn kind(&self) -> ValueKind {
        match self {
            Self::Number(_) => ValueKind::Number,
            Self::Date(_) => ValueKind::Date,
            Self::NaiveDateTime(_) => ValueKind::NaiveDateTime,
            Self::ZonedDateTime(_) => ValueKind::ZonedDateTime,
        }
    }

    /// Label a set of values, such as an axis's ticks, returning one label per value. Numbers
    /// use the formatter's `format_ticks` with `spacing`. Datetimes format each value, and
    /// missing datetimes get empty labels.
    pub fn format_ticks(
        &self,
        values: FormatValues<'_>,
        spacing: TickSpacing,
    ) -> Result<Vec<String>, FormatError> {
        match (self, values) {
            (Self::Number(format), FormatValues::Numbers(values)) => Ok(format
                .format_ticks(values, spacing)
                .into_iter()
                .map(|label| label.text)
                .collect()),
            (Self::Date(format), FormatValues::Dates(values)) => each(values, |v| format.format(v)),
            (Self::NaiveDateTime(format), FormatValues::NaiveDateTimes(values)) => {
                each(values, |v| format.format(v))
            }
            (Self::ZonedDateTime(format), FormatValues::ZonedDateTimes(values)) => {
                each(values, |v| format.format(v))
            }
            (format, values) => Err(FormatError::Mismatch {
                formatter: format.kind(),
                values: values.kind(),
            }),
        }
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

fn each<T: Copy>(
    values: &[Option<T>],
    format: impl Fn(T) -> Result<String, DateTimeFormatError>,
) -> Result<Vec<String>, FormatError> {
    values
        .iter()
        .map(|value| Ok(value.map(&format).transpose()?.unwrap_or_default()))
        .collect()
}

impl From<Arc<dyn PreparedNumberFormatter>> for PreparedFormatter {
    fn from(format: Arc<dyn PreparedNumberFormatter>) -> Self {
        Self::Number(format)
    }
}

impl From<Arc<dyn PreparedDateFormatter>> for PreparedFormatter {
    fn from(format: Arc<dyn PreparedDateFormatter>) -> Self {
        Self::Date(format)
    }
}

impl From<Arc<dyn PreparedNaiveDateTimeFormatter>> for PreparedFormatter {
    fn from(format: Arc<dyn PreparedNaiveDateTimeFormatter>) -> Self {
        Self::NaiveDateTime(format)
    }
}

impl From<Arc<dyn PreparedZonedDateTimeFormatter>> for PreparedFormatter {
    fn from(format: Arc<dyn PreparedZonedDateTimeFormatter>) -> Self {
        Self::ZonedDateTime(format)
    }
}
