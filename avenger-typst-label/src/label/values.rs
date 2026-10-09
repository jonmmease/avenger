//! The values that `bind` writes into markup.

use indexmap::IndexMap;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Values by name, which `bind` writes into markup in place of references to their names.
pub type LabelValues = IndexMap<String, LabelValue>;

/// A value that markup can show, or pass to a function such as `#numfmt` or `#datetimefmt`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum LabelValue {
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Date(chrono::NaiveDate),
    /// A date and time without a timezone.
    NaiveDateTime(chrono::NaiveDateTime),
    /// An instant, which formatters show in their timezone.
    ZonedDateTime(chrono::DateTime<chrono::Utc>),
    Array(Vec<LabelValue>),
    Dict(IndexMap<String, LabelValue>),
}
