use crate::{
    compact::{CompactPluralRule, CompactTier},
    error::FormatError,
};
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
/// Compact metadata registered separately from the D3 number definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct NumberLocaleExtensions {
    pub compact_short: Vec<CompactTier>,
    pub compact_long: Vec<CompactTier>,
    /// Selects the `one` pattern from the rounded coefficient.
    pub compact_plural_rule: CompactPluralRule,
}
impl NumberLocaleExtensions {
    /// English compact suffixes for powers of a thousand.
    pub fn en_us() -> Self {
        fn tiers(suffixes: [&str; 4]) -> Vec<CompactTier> {
            [3, 6, 9, 12]
                .into_iter()
                .zip(suffixes)
                .map(|(exponent, suffix)| CompactTier {
                    exponent,
                    other: format!("{{0}}{suffix}"),
                    one: None,
                    exact_one: None,
                })
                .collect()
        }
        Self {
            compact_short: tiers(["K", "M", "B", "T"]),
            compact_long: tiers([" thousand", " million", " billion", " trillion"]),
            compact_plural_rule: CompactPluralRule::IntegerOne,
        }
    }
}

/// A validated locale with an immutable definition shared across clones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedNumberLocale {
    id: LocaleId,
    definition: Arc<NumberLocaleSpec>,
    pub(crate) extensions: Arc<NumberLocaleExtensions>,
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

    /// Borrow compact metadata associated with this definition.
    pub fn extensions(&self) -> &NumberLocaleExtensions {
        &self.extensions
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
            extensions: Arc::new(NumberLocaleExtensions::default()),
        })
    }
    /// Resolve the bundled U.S. English locale.
    pub fn en_us() -> Self {
        let mut locale =
            Self::new("en-US", NumberLocaleSpec::en_us()).expect("bundled D3 number locale");
        locale.extensions = Arc::new(NumberLocaleExtensions::en_us());
        locale
    }
}
