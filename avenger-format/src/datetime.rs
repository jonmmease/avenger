use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use std::{fmt, fmt::Debug, sync::Arc};

/// The calendar fields and timezone information supplied to a formatter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateTimeInputKind {
    /// A calendar date without a time of day or timezone.
    Date,
    /// Calendar date and time fields without a timezone.
    Naive,
    /// A UTC datetime displayed in the provider's timezone.
    Zoned,
}

impl fmt::Display for DateTimeInputKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Date => "date",
            Self::Naive => "naive datetime",
            Self::Zoned => "zoned datetime",
        })
    }
}

/// Shared error categories for datetime providers, with backend diagnostic details.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DateTimeFormatError {
    /// The format specification has invalid syntax.
    #[error("invalid datetime format specification{}: {message}", .position.map(|p| format!(" at byte {p}")).unwrap_or_default())]
    InvalidPattern {
        message: String,
        /// Zero-based byte offset reported by the backend, or `None` when unavailable.
        position: Option<usize>,
    },
    /// The format specification cannot format the requested input type or field combination.
    #[error("unsupported format specification for {input} input: {message}")]
    UnsupportedPattern {
        input: DateTimeInputKind,
        message: String,
    },
    /// A provider or format option has an unsupported value.
    #[error("invalid datetime format option `{option}`: {message}")]
    InvalidOption { option: String, message: String },
    /// The locale is unknown, unregistered, or disabled by a build feature.
    #[error("locale `{locale}` is unavailable: {message}")]
    LocaleUnavailable { locale: String, message: String },
    /// A custom locale definition is malformed or inconsistent.
    #[error("invalid locale data: {message}")]
    InvalidLocaleData { message: String },
    /// The provider cannot format a particular input value.
    #[error("unsupported {input} value: {message}")]
    UnsupportedValue {
        input: DateTimeInputKind,
        message: String,
    },
    /// A datetime or its timezone conversion exceeds the supported calendar range.
    #[error("datetime exceeds the supported calendar range")]
    OutOfRange,
    /// Rendering failed without a more specific diagnosis from the backend.
    #[error("datetime cannot be rendered with the prepared format specification")]
    FormattingFailed,
}

/// Prepare explicit format specifications using the provider's locale and formatting settings.
/// Prepared formatters retain resolved settings independently of the provider.
pub trait DateTimeFormatProvider: Debug + Send + Sync + 'static {
    /// Prepare for calendar dates, rejecting time, epoch, and timezone fields.
    /// The provider's display timezone does not affect the date.
    fn prepare_date(
        &self,
        spec: &str,
    ) -> Result<Arc<dyn PreparedDateFormatter>, DateTimeFormatError>;

    /// Prepare for naive datetimes, rejecting epoch and timezone fields.
    /// The provider's display timezone does not affect the calendar fields.
    fn prepare_naive(
        &self,
        spec: &str,
    ) -> Result<Arc<dyn PreparedNaiveDateTimeFormatter>, DateTimeFormatError>;

    /// Prepare for UTC datetimes displayed in the provider's timezone.
    fn prepare_zoned(
        &self,
        spec: &str,
    ) -> Result<Arc<dyn PreparedZonedDateTimeFormatter>, DateTimeFormatError>;

    /// Check that dates display in a calendar whose months start on the same days as Gregorian
    /// months, which [`CalendarPatterns`](crate::CalendarPatterns) needs to label boundaries.
    /// Providers that only use the Gregorian calendar keep this default.
    fn check_gregorian_months(&self) -> Result<(), DateTimeFormatError> {
        Ok(())
    }
}

/// Immutable, deterministic formatting of calendar dates without a time or timezone.
/// Preparation validates the specification. Formatting can still reject unsupported values.
pub trait PreparedDateFormatter: Debug + Send + Sync + 'static {
    fn format(&self, value: NaiveDate) -> Result<String, DateTimeFormatError>;
}

/// Immutable, deterministic formatting of naive calendar fields.
/// Preparation validates the specification. Formatting can still reject unsupported values.
pub trait PreparedNaiveDateTimeFormatter: Debug + Send + Sync + 'static {
    fn format(&self, value: NaiveDateTime) -> Result<String, DateTimeFormatError>;
}

/// Immutable, deterministic formatting of UTC datetimes in a resolved display timezone.
/// Formatting reports values whose display date is outside the supported range.
pub trait PreparedZonedDateTimeFormatter: Debug + Send + Sync + 'static {
    fn format(&self, value: DateTime<Utc>) -> Result<String, DateTimeFormatError>;

    /// The timezone that values are displayed in.
    fn timezone(&self) -> chrono_tz::Tz;
}
