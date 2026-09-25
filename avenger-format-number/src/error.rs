use thiserror::Error;

/// A rejected D3 format specifier, with its location in the original input.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// Invalid syntax, an unsupported type, or an unrepresentable field width.
    #[error("invalid number format specifier at byte {position}: {message}")]
    Invalid {
        /// Zero-based UTF-8 byte offset in the specifier.
        position: usize,
        message: String,
    },
}

impl ParseError {
    pub(crate) fn invalid(position: usize, message: impl Into<String>) -> Self {
        Self::Invalid {
            position,
            message: message.into(),
        }
    }
}

/// Failure to prepare a formatter or register or resolve a locale.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum FormatError {
    #[error(transparent)]
    Parse(#[from] ParseError),

    /// Parsed fields and overrides form an unsupported combination.
    #[error("invalid number format: {0}")]
    InvalidFormat(String),

    /// The requested locale name has no registry entry.
    #[error("locale `{0}` was not found")]
    LocaleNotFound(String),

    /// Locale data contains invalid grouping, compact exponents, or patterns.
    #[error("invalid locale data: {0}")]
    InvalidLocaleData(String),
}
