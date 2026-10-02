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

    #[error("datetime exceeds the supported calendar range")]
    DateTimeOutOfRange,

    #[error("leap seconds are unsupported for naive datetime formatting")]
    LeapSecondForNaive,

    #[error("locale `{0}` was not found")]
    LocaleNotFound(String),

    #[error("invalid locale data: {0}")]
    InvalidLocaleData(String),

    #[error("datetime format field `{0}` requires zoned datetime input")]
    TimezoneFieldForNaive(String),

    #[error("datetime format field `{0}` requires a time of day")]
    TimeFieldForDate(String),
}
