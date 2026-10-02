use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use std::{fmt::Debug, sync::Arc};

/// A provider configuration or input-value error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct DateTimeFormatError(pub String);

/// Prepare explicit patterns using the provider's locale and formatting settings.
/// Prepared formatters retain resolved settings independently of the provider.
pub trait DateTimeFormatProvider: Debug + Send + Sync + 'static {
    /// Prepare for calendar dates, rejecting time, epoch, and timezone fields.
    /// The provider's display timezone does not affect the date.
    fn prepare_date(
        &self,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedDateFormatter>, DateTimeFormatError>;

    /// Prepare for naive datetimes, rejecting epoch and timezone fields.
    /// The provider's display timezone does not affect the calendar fields.
    fn prepare_naive(
        &self,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedNaiveDateTimeFormatter>, DateTimeFormatError>;

    /// Prepare for UTC datetimes displayed in the provider's timezone.
    fn prepare_zoned(
        &self,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedZonedDateTimeFormatter>, DateTimeFormatError>;
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
}
