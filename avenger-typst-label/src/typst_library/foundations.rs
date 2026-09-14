//! Retained Typst foundation values.
//!
//! This is the tiny read-only subset of upstream `typst-library` foundations
//! needed for external label parameters.

use indexmap::IndexMap;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub(crate) enum Value {
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Date(chrono::NaiveDate),
    DateTime(chrono::NaiveDateTime),
    UtcDateTime(chrono::DateTime<chrono::Utc>),
    Array(Vec<Value>),
    Dict(Dict),
}

pub(crate) type Dict = IndexMap<String, Value>;

#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub(crate) struct Scope {
    values: Dict,
}

impl Scope {
    pub(crate) fn new(values: Dict) -> Self {
        Self { values }
    }

    pub(crate) fn get(&self, name: &str) -> Option<&Value> {
        self.values.get(name)
    }
}
