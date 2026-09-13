use crate::{compact::CompactTier, error::FormatError};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

/// A locale name resolved within an Avenger formatting context.
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrencyPattern {
    pub positive_prefix: String,
    pub positive_suffix: String,
    pub negative_prefix: String,
    pub negative_suffix: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrencyFormat {
    pub standard: CurrencyPattern,
    pub accounting: CurrencyPattern,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CurrencyDisplayNames {
    pub symbol: Option<String>,
    pub narrow_symbol: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CurrencyDisplay {
    #[default]
    Symbol,
    Code,
    Name,
    NarrowSymbol,
}

/// Normalized metadata used only by the Avenger `S`, `L`, and `C` extensions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NumberLocaleExtensions {
    pub currency: CurrencyFormat,
    pub currency_names: BTreeMap<String, CurrencyDisplayNames>,
    pub compact_short: Vec<CompactTier>,
    pub compact_long: Vec<CompactTier>,
}
impl Default for NumberLocaleExtensions {
    fn default() -> Self {
        Self {
            currency: CurrencyFormat {
                standard: CurrencyPattern {
                    positive_prefix: "\u{00a4}".to_string(),
                    positive_suffix: String::new(),
                    negative_prefix: "-\u{00a4}".to_string(),
                    negative_suffix: String::new(),
                },
                accounting: CurrencyPattern {
                    positive_prefix: "\u{00a4}".to_string(),
                    positive_suffix: String::new(),
                    negative_prefix: "(\u{00a4}".to_string(),
                    negative_suffix: ")".to_string(),
                },
            },
            currency_names: BTreeMap::from([
                (
                    "USD".to_string(),
                    CurrencyDisplayNames {
                        symbol: Some("$".to_string()),
                        narrow_symbol: Some("$".to_string()),
                        name: Some("US dollars".to_string()),
                    },
                ),
                (
                    "EUR".to_string(),
                    CurrencyDisplayNames {
                        symbol: Some("\u{20ac}".to_string()),
                        narrow_symbol: Some("\u{20ac}".to_string()),
                        name: Some("euros".to_string()),
                    },
                ),
                (
                    "JPY".to_string(),
                    CurrencyDisplayNames {
                        symbol: Some("\u{00a5}".to_string()),
                        narrow_symbol: Some("\u{00a5}".to_string()),
                        name: Some("Japanese yen".to_string()),
                    },
                ),
            ]),
            compact_short: vec![
                CompactTier {
                    exponent: 3,
                    one: Some("{0}K".to_string()),
                    other: "{0}K".to_string(),
                },
                CompactTier {
                    exponent: 6,
                    one: Some("{0}M".to_string()),
                    other: "{0}M".to_string(),
                },
                CompactTier {
                    exponent: 9,
                    one: Some("{0}B".to_string()),
                    other: "{0}B".to_string(),
                },
                CompactTier {
                    exponent: 12,
                    one: Some("{0}T".to_string()),
                    other: "{0}T".to_string(),
                },
            ],
            compact_long: vec![
                CompactTier {
                    exponent: 3,
                    one: Some("{0} thousand".to_string()),
                    other: "{0} thousand".to_string(),
                },
                CompactTier {
                    exponent: 6,
                    one: Some("{0} million".to_string()),
                    other: "{0} million".to_string(),
                },
                CompactTier {
                    exponent: 9,
                    one: Some("{0} billion".to_string()),
                    other: "{0} billion".to_string(),
                },
                CompactTier {
                    exponent: 12,
                    one: Some("{0} trillion".to_string()),
                    other: "{0} trillion".to_string(),
                },
            ],
        }
    }
}

/// A D3 definition and independent Avenger extension metadata.
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

    /// Borrow the validated compact and currency metadata.
    pub fn extensions(&self) -> &NumberLocaleExtensions {
        &self.extensions
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
            extensions: Arc::new(NumberLocaleExtensions::default()),
        })
    }
    /// Resolve the bundled U.S. English locale.
    pub fn en_us() -> Self {
        Self::new("en-US", NumberLocaleSpec::en_us()).expect("bundled D3 number locale")
    }
}
