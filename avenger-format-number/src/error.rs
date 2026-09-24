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

    #[error("locale `{0}` was not found")]
    LocaleNotFound(String),

    #[error("invalid locale data: {0}")]
    InvalidLocaleData(String),
}
