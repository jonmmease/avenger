//! Provider interfaces for preparing and rendering localized number and datetime labels.

mod calendar;
pub use calendar::CalendarPatterns;

mod datetime;
pub use datetime::{
    DateTimeFormatError, DateTimeFormatProvider, DateTimeInputKind, PreparedDateFormatter,
    PreparedNaiveDateTimeFormatter, PreparedZonedDateTimeFormatter,
};

mod formatted_number;
pub use formatted_number::{FormattedNumber, NumberTypesetting};

mod values;
pub use values::{FormatValues, ValueKind};

mod precision;
pub use precision::{PrecisionSharing, TickStep};

use std::{fmt::Debug, sync::Arc};

/// Shared preparation errors for number providers, with backend diagnostic details.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NumberFormatError {
    /// The pattern parser rejected the syntax.
    #[error("invalid number pattern{}: {message}", .position.map(|p| format!(" at byte {p}")).unwrap_or_default())]
    InvalidPattern {
        message: String,
        /// Zero-based byte offset reported by the backend, or `None` when unavailable.
        position: Option<usize>,
    },
    /// A provider option has an unsupported value.
    #[error("invalid number format option `{option}`: {message}")]
    InvalidOption { option: String, message: String },
    /// The locale is unknown, unregistered, or disabled by a build feature.
    #[error("locale `{locale}` is unavailable: {message}")]
    LocaleUnavailable { locale: String, message: String },
    /// A custom locale definition is malformed or inconsistent.
    #[error("invalid locale data: {message}")]
    InvalidLocaleData { message: String },
}

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

    /// Format values as one set, such as an axis's ticks, returning one label per value.
    /// When the pattern leaves precision open, `sharing` says whether the values share one
    /// precision and unit or each keep their own. Explicit pattern settings take precedence.
    /// The default formats each value independently.
    fn format_all(&self, values: &[f64], _sharing: PrecisionSharing) -> Vec<FormattedNumber> {
        values.iter().map(|&value| self.format(value)).collect()
    }
}
