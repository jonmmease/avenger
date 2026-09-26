use std::{fmt::Debug, sync::Arc};

/// Civil calendar fields without an instant or display offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NaiveDateTimeInput {
    Date(chrono::NaiveDate),
    DateTime(chrono::NaiveDateTime),
}
impl NaiveDateTimeInput {
    /// Preserve datetime fields and interpret a date-only value as midnight.
    pub fn datetime(self) -> chrono::NaiveDateTime {
        match self {
            Self::Date(date) => date.and_time(chrono::NaiveTime::MIN),
            Self::DateTime(value) => value,
        }
    }
}
/// An instant represented in UTC and displayed in the configured timezone.
pub type ZonedDateTimeInput = chrono::DateTime<chrono::Utc>;

/// A provider configuration or input-value error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct DateTimeFormatError(pub String);

/// Resolve an explicit pattern and provider-specific configuration for a sequence of labels.
pub trait DateTimeFormatProvider: Debug + Send + Sync + 'static {
    /// Locale and preparation options accepted by this provider.
    type Config;

    /// Prepare for civil dates and datetimes, rejecting fields that require an instant.
    /// Timezone configuration is unused and is not validated.
    fn prepare_naive(
        &self,
        config: &Self::Config,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedCivilDateTimeFormatter>, DateTimeFormatError>;

    /// Prepare for instants, resolving and validating the configured display timezone.
    fn prepare_zoned(
        &self,
        config: &Self::Config,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedInstantFormatter>, DateTimeFormatError>;
}

/// Immutable, deterministic formatting of civil calendar fields.
/// Preparation validates the specification. Formatting can still reject unsupported values.
pub trait PreparedCivilDateTimeFormatter: Debug + Send + Sync + 'static {
    fn format(&self, value: NaiveDateTimeInput) -> Result<String, DateTimeFormatError>;
}

/// Immutable, deterministic formatting of instants in a resolved display timezone.
/// Formatting reports values whose display date is outside the supported range.
pub trait PreparedInstantFormatter: Debug + Send + Sync + 'static {
    fn format(&self, value: ZonedDateTimeInput) -> Result<String, DateTimeFormatError>;
}
