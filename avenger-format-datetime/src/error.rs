use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DateTimeParseError {
    #[error("invalid datetime format specifier at byte {position}: {message}")]
    Invalid { position: usize, message: String },
}

impl DateTimeParseError {
    pub(crate) fn invalid(position: usize, message: impl Into<String>) -> Self {
        Self::Invalid {
            position,
            message: message.into(),
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DateTimeFormatError {
    #[error(transparent)]
    Parse(#[from] DateTimeParseError),

    #[error("invalid datetime format: {0}")]
    InvalidFormat(String),

    #[error("unsupported datetime field `{0}`")]
    UnsupportedField(String),

    #[error("locale `{0}` was not found")]
    LocaleNotFound(String),

    #[error("invalid locale data: {0}")]
    InvalidLocaleData(String),

    #[error("invalid timezone `{0}`")]
    InvalidTimezone(String),

    #[error("timezone override is invalid for naive datetime formatting")]
    TimezoneOverrideForNaive,

    #[error("datetime format field `{0}` requires timezone-aware input")]
    TimezoneFieldForNaive(String),
}
