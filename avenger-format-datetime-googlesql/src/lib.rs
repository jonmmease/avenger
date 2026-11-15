#![doc = include_str!("../README.md")]

mod element;
mod format;
mod parse;
mod provider;

pub use element::Kind;
pub use format::Format;
pub use parse::{parse_date, parse_datetime, parse_time, parse_timestamp};
pub use provider::GoogleSqlDateTimeFormatProvider;

/// An error of a format or of a value, with GoogleSQL's message.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct Error(pub String);
