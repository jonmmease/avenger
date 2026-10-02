use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use std::{fmt::Debug, sync::Arc};

/// A provider configuration or input-value error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct DateTimeFormatError(pub String);

/// Resolve an explicit pattern and provider-specific configuration for a sequence of labels.
pub trait DateTimeFormatProvider: Debug + Send + Sync + 'static {
    /// Locale and preparation options accepted by this provider.
    type Config;

    /// Prepare for calendar dates, rejecting time, epoch, and timezone fields.
    /// Timezone configuration is unused and is not validated.
    fn prepare_date(
        &self,
        config: &Self::Config,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedDateFormatter>, DateTimeFormatError>;

    /// Prepare for naive datetimes, rejecting epoch and timezone fields.
    /// Timezone configuration is unused and is not validated.
    fn prepare_naive(
        &self,
        config: &Self::Config,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedNaiveDateTimeFormatter>, DateTimeFormatError>;

    /// Prepare for zoned datetimes, resolving and validating the configured display timezone.
    fn prepare_zoned(
        &self,
        config: &Self::Config,
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
