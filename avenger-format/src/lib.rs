//! Provider interfaces for preparing and rendering localized number and datetime labels.

mod datetime;
pub use datetime::{
    DateTimeFormatError, DateTimeFormatProvider, DateTimeInputKind, PreparedDateFormatter,
    PreparedNaiveDateTimeFormatter, PreparedZonedDateTimeFormatter,
};

mod formatted_number;
pub use formatted_number::{FormattedNumber, NumberTypesetting};
use std::{fmt::Debug, sync::Arc};

/// A configuration error detected before rendering values.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct NumberFormatError(pub String);

/// Prepare explicit patterns using the provider's locale and formatting settings.
pub trait NumberFormatProvider: Debug + Send + Sync + 'static {
    /// Prepare an explicit pattern, retaining resolved settings independently of the provider.
    fn prepare(&self, pattern: &str)
        -> Result<Arc<dyn PreparedNumberFormatter>, NumberFormatError>;
}

/// A reusable formatter. Implementations must be immutable and deterministic,
/// including their handling of NaN, infinity, and signed zero.
pub trait PreparedNumberFormatter: Debug + Send + Sync + 'static {
    fn format(&self, value: f64) -> FormattedNumber;
}
