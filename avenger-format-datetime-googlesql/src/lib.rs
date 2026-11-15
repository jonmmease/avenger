#![doc = include_str!("../README.md")]

mod element;
mod format;
mod provider;

pub use element::Kind;
pub use format::Format;
pub use provider::GoogleSqlDateTimeFormatProvider;

/// An error of a format or of a value, with GoogleSQL's message.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// GoogleSQL documents the element but does not support it, as `%J`.
    #[error("{0}")]
    Unsupported(String),
}
