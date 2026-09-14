use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ParseError {
    #[error("invalid number format specifier at byte {position}: {message}")]
    Invalid { position: usize, message: String },
}

impl ParseError {
    pub(crate) fn invalid(position: usize, message: impl Into<String>) -> Self {
        Self::Invalid {
            position,
            message: message.into(),
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum FormatError {
    #[error(transparent)]
    Parse(#[from] ParseError),

    #[error("invalid number format: {0}")]
    InvalidFormat(String),

    #[error("unsupported number format type `{0}`")]
    UnsupportedType(String),

    #[error("unsupported number format extension `{0}`")]
    UnsupportedExtension(String),

    #[error("invalid currency code `{0}`")]
    InvalidCurrencyCode(String),

    #[error("currency formatting requires `C[ISO]` or a structured currency override")]
    MissingCurrencyCode,

    #[error("locale `{0}` was not found")]
    LocaleNotFound(String),

    #[error("invalid locale data: {0}")]
    InvalidLocaleData(String),
}
