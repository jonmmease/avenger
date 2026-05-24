//! Channel descriptor types for defining mark channels

use datafusion::common::ScalarValue;
use serde::{Deserialize, Serialize};

/// Default value for a channel
#[derive(Debug, Clone)]
pub enum ChannelDefault {
    Scalar(ScalarValue),
    // Could extend with other types later, like theme based defaults,
    // expressions based on other channels, etc.
}

/// Descriptor for a channel that a mark supports
#[derive(Debug, Clone)]
pub struct ChannelDescriptor {
    pub name: &'static str,
    pub required: bool,
    pub default_value: Option<ChannelDefault>,
    pub allow_column_ref: bool,
}

/// Remove trailing numbers from a channel name to get the base scale name.
///
/// For example, `x1` becomes `x`, `color2` becomes `color`, and `x` stays `x`.
pub fn strip_trailing_numbers(name: &str) -> &str {
    name.trim_end_matches(char::is_numeric)
}

/// Channel name with trailing numbers stripped (`y2` -> `y`, `x10` -> `x`).
///
/// This newtype ensures that scales are stored and retrieved using consistent
/// base names, preventing runtime errors from mismatched lookups.
#[derive(Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct BaseChannelName(String);

impl BaseChannelName {
    /// Create a base channel name by stripping trailing numbers from the raw name.
    pub fn from_raw(name: &str) -> Self {
        Self(strip_trailing_numbers(name).to_string())
    }

    /// Return the base channel name as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for BaseChannelName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::fmt::Debug for BaseChannelName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BaseChannelName({})", self.0)
    }
}

impl From<&str> for BaseChannelName {
    fn from(name: &str) -> Self {
        Self::from_raw(name)
    }
}

impl From<String> for BaseChannelName {
    fn from(name: String) -> Self {
        Self::from_raw(&name)
    }
}

impl AsRef<str> for BaseChannelName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
