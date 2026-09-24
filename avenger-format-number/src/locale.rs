use crate::error::FormatError;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// A name used to register and resolve a number locale.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LocaleId(pub String);
impl LocaleId {
    /// Construct a locale identifier without resolving it.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}
impl AsRef<str> for LocaleId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
impl std::fmt::Display for LocaleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// A D3 number locale definition. Missing JSON fields use [`Self::default`].
/// The default has no grouping or currency affixes. Use [`Self::en_us`] for U.S. conventions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NumberLocaleSpec {
    /// Decimal separator, defaulting to `.`.
    pub decimal: String,
    /// Separator between digit groups, defaulting to an empty string.
    pub thousands: String,
    /// Positive group sizes, cycled from right to left. An empty list disables grouping.
    pub grouping: Vec<usize>,
    /// Prefix and suffix for `$`, independent of digit precision.
    pub currency: [String; 2],
    /// Replacement strings for ASCII digits 0 through 9, applied to the entire label.
    pub numerals: Option<[String; 10]>,
    /// Suffix for percent formats, defaulting to `%`.
    pub percent: String,
    /// Negative sign, defaulting to Unicode `−`.
    pub minus: String,
    /// Label for NaN, defaulting to `NaN`.
    pub nan: String,
}
impl Default for NumberLocaleSpec {
    fn default() -> Self {
        Self {
            decimal: ".".into(),
            thousands: String::new(),
            grouping: Vec::new(),
            currency: [String::new(), String::new()],
            numerals: None,
            percent: "%".into(),
            minus: "−".into(),
            nan: "NaN".into(),
        }
    }
}
impl NumberLocaleSpec {
    /// Load the bundled U.S. English D3 definition.
    pub fn en_us() -> Self {
        serde_json::from_str(include_str!("../locales/en-US.json"))
            .expect("bundled D3 number locale")
    }
}
/// A validated locale with an immutable definition shared across clones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedNumberLocale {
    id: LocaleId,
    definition: Arc<NumberLocaleSpec>,
}
impl std::ops::Deref for ResolvedNumberLocale {
    type Target = NumberLocaleSpec;
    fn deref(&self) -> &Self::Target {
        &self.definition
    }
}
impl ResolvedNumberLocale {
    /// Return the name assigned when this locale was resolved.
    pub fn id(&self) -> &LocaleId {
        &self.id
    }

    /// Borrow the validated D3 number definition.
    pub fn definition(&self) -> &NumberLocaleSpec {
        &self.definition
    }

    /// Retain a named locale definition, rejecting zero-sized digit groups.
    pub fn new(id: impl Into<String>, definition: NumberLocaleSpec) -> Result<Self, FormatError> {
        if definition.grouping.contains(&0) {
            return Err(FormatError::InvalidLocaleData(
                "grouping sizes must be positive".into(),
            ));
        }
        Ok(Self {
            id: LocaleId::new(id),
            definition: Arc::new(definition),
        })
    }
    /// Resolve the bundled U.S. English locale.
    pub fn en_us() -> Self {
        Self::new("en-US", NumberLocaleSpec::en_us()).expect("bundled D3 number locale")
    }
}
