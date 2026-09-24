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

/// An upstream D3 number locale definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NumberLocaleSpec {
    pub decimal: String,
    pub thousands: String,
    pub grouping: Vec<usize>,
    pub currency: [String; 2],
    pub numerals: Option<[String; 10]>,
    pub percent: String,
    pub minus: String,
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
/// A validated D3 number locale definition.
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

    /// Validate grouping at construction so formatting cannot loop on invalid sizes.
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
