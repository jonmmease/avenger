//! Provider interfaces for preparing and rendering localized number and datetime labels.

mod datetime;
pub use datetime::{
    DateTimeFormatError, DateTimeFormatProvider, NaiveDateTimeInput,
    PreparedCivilDateTimeFormatter, PreparedInstantFormatter, ZonedDateTimeInput,
};

mod formatted_number;
pub use formatted_number::{FormattedNumber, NumberTypesetting};
use std::{fmt::Debug, sync::Arc};

/// A configuration error detected before rendering values.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct NumberFormatError(pub String);

/// Resolve a pattern and provider-specific configuration once for a sequence of labels.
pub trait NumberFormatProvider: Debug + Send + Sync + 'static {
    /// Locale and preparation options accepted by this provider.
    type Config;

    /// Prepare an explicit pattern, retaining the resolved state independently of configuration.
    fn prepare(
        &self,
        config: &Self::Config,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedNumberFormatter>, NumberFormatError>;
}

/// A reusable formatter. Implementations must be immutable and deterministic,
/// including their handling of NaN, infinity, and signed zero.
pub trait PreparedNumberFormatter: Debug + Send + Sync + 'static {
    fn format(&self, value: f64) -> FormattedNumber;
}
