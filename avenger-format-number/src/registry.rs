use crate::{
    compact::CompactTier,
    locale::{CurrencyDisplayNames, CurrencyFormat, CurrencyPattern, NumberLocaleExtensions},
    FormatError, NumberLocaleSpec, ResolvedNumberLocale,
};
use std::{collections::BTreeMap, sync::Arc};

/// Named D3 locales and their separately registered Avenger metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NumberLocaleRegistry {
    locales: BTreeMap<String, ResolvedNumberLocale>,
}
impl NumberLocaleRegistry {
    /// Load bundled definitions and normalized compact/currency metadata.
    pub fn with_builtins() -> Self {
        let mut registry = Self::default();
        for (id, json, extensions) in [
            (
                "en-US",
                include_str!("../locales/en-US.json"),
                NumberLocaleExtensions::default(),
            ),
            (
                "de-DE",
                include_str!("../locales/de-DE.json"),
                builtin_de_de_spec(),
            ),
            (
                "fr-FR",
                include_str!("../locales/fr-FR.json"),
                builtin_fr_fr_spec(),
            ),
            (
                "ja-JP",
                include_str!("../locales/ja-JP.json"),
                builtin_ja_jp_spec(),
            ),
        ] {
            registry
                .register_custom_locale_json(id, json)
                .expect("bundled D3 number locale");
            registry
                .register_extensions(id, extensions)
                .expect("bundled Avenger number metadata");
        }
        registry
    }
    /// Register a D3 definition after validation.
    pub fn register_custom_locale(
        &mut self,
        id: impl Into<String>,
        spec: NumberLocaleSpec,
    ) -> Result<(), FormatError> {
        let id = id.into();
        let locale = ResolvedNumberLocale::new(id.clone(), spec)?;
        self.locales.insert(id, locale);
        Ok(())
    }
    /// Register an upstream D3 number locale JSON object.
    pub fn register_custom_locale_json(
        &mut self,
        id: impl Into<String>,
        json: &str,
    ) -> Result<(), FormatError> {
        self.register_custom_locale(
            id,
            serde_json::from_str(json)
                .map_err(|error| FormatError::InvalidLocaleData(error.to_string()))?,
        )
    }
    /// Register normalized extension metadata without changing standard D3 output.
    pub fn register_extensions(
        &mut self,
        id: &str,
        mut extensions: NumberLocaleExtensions,
    ) -> Result<(), FormatError> {
        for tiers in [&mut extensions.compact_short, &mut extensions.compact_long] {
            tiers.sort_by_key(|tier| tier.exponent);
            if tiers
                .windows(2)
                .any(|pair| pair[0].exponent == pair[1].exponent)
            {
                return Err(FormatError::InvalidLocaleData(
                    "duplicate compact tier exponent".into(),
                ));
            }
            for tier in tiers {
                for pattern in std::iter::once(&tier.other).chain(tier.one.iter()) {
                    if pattern.matches("{0}").count() != 1 {
                        return Err(FormatError::InvalidLocaleData(
                            "compact patterns require one `{0}` placeholder".into(),
                        ));
                    }
                }
            }
        }
        let locale = self
            .locales
            .get_mut(id)
            .ok_or_else(|| FormatError::LocaleNotFound(id.into()))?;
        locale.extensions = Arc::new(extensions);
        Ok(())
    }
    /// Resolve a name, sharing its validated definition and metadata.
    pub fn resolve(&self, id: &str) -> Result<ResolvedNumberLocale, FormatError> {
        self.locales
            .get(id)
            .cloned()
            .ok_or_else(|| FormatError::LocaleNotFound(id.into()))
    }
}

fn builtin_de_de_spec() -> NumberLocaleExtensions {
    NumberLocaleExtensions {
        currency: currency_suffix_format("\u{00a0}", false),
        currency_names: currency_names([
            ("USD", "$", "$", "US-Dollar"),
            ("EUR", "\u{20ac}", "\u{20ac}", "Euro"),
            ("JPY", "\u{00a5}", "\u{00a5}", "Japanischer Yen"),
        ]),
        compact_short: vec![
            compact_tier(6, "{0}\u{00a0}Mio."),
            compact_tier(9, "{0}\u{00a0}Mrd."),
            compact_tier(12, "{0}\u{00a0}Bio."),
        ],
        compact_long: vec![
            CompactTier {
                exponent: 3,
                one: Some("{0} Tausend".to_string()),
                other: "{0} Tausend".to_string(),
            },
            CompactTier {
                exponent: 6,
                one: Some("{0} Million".to_string()),
                other: "{0} Millionen".to_string(),
            },
            CompactTier {
                exponent: 9,
                one: Some("{0} Milliarde".to_string()),
                other: "{0} Milliarden".to_string(),
            },
            CompactTier {
                exponent: 12,
                one: Some("{0} Billion".to_string()),
                other: "{0} Billionen".to_string(),
            },
        ],
    }
}

fn builtin_fr_fr_spec() -> NumberLocaleExtensions {
    NumberLocaleExtensions {
        currency: currency_suffix_format("\u{00a0}", true),
        currency_names: currency_names([
            ("USD", "$US", "$", "dollars des \u{00c9}tats-Unis"),
            ("EUR", "\u{20ac}", "\u{20ac}", "euros"),
            ("JPY", "JPY", "\u{00a5}", "yens japonais"),
        ]),
        compact_short: vec![
            compact_tier(3, "{0}\u{00a0}k"),
            compact_tier(6, "{0}\u{00a0}M"),
            compact_tier(9, "{0}\u{00a0}Md"),
            compact_tier(12, "{0}\u{00a0}Bn"),
        ],
        compact_long: vec![
            CompactTier {
                exponent: 3,
                one: Some("{0} millier".to_string()),
                other: "{0} mille".to_string(),
            },
            CompactTier {
                exponent: 6,
                one: Some("{0} million".to_string()),
                other: "{0} millions".to_string(),
            },
            CompactTier {
                exponent: 9,
                one: Some("{0} milliard".to_string()),
                other: "{0} milliards".to_string(),
            },
            CompactTier {
                exponent: 12,
                one: Some("{0} billion".to_string()),
                other: "{0} billions".to_string(),
            },
        ],
    }
}

fn builtin_ja_jp_spec() -> NumberLocaleExtensions {
    NumberLocaleExtensions {
        currency: currency_prefix_format(),
        currency_names: currency_names([
            ("USD", "$", "$", "\u{7c73}\u{30c9}\u{30eb}"),
            ("EUR", "\u{20ac}", "\u{20ac}", "\u{30e6}\u{30fc}\u{30ed}"),
            ("JPY", "\u{ffe5}", "\u{ffe5}", "\u{65e5}\u{672c}\u{5186}"),
        ]),
        compact_short: vec![
            compact_tier(4, "{0}\u{4e07}"),
            compact_tier(8, "{0}\u{5104}"),
            compact_tier(12, "{0}\u{5146}"),
            compact_tier(16, "{0}\u{4eac}"),
        ],
        compact_long: vec![
            compact_tier(4, "{0}\u{4e07}"),
            compact_tier(8, "{0}\u{5104}"),
            compact_tier(12, "{0}\u{5146}"),
            compact_tier(16, "{0}\u{4eac}"),
        ],
    }
}

fn currency_prefix_format() -> CurrencyFormat {
    CurrencyFormat {
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
    }
}

fn currency_suffix_format(space: &str, accounting_parentheses: bool) -> CurrencyFormat {
    CurrencyFormat {
        standard: CurrencyPattern {
            positive_prefix: String::new(),
            positive_suffix: format!("{space}\u{00a4}"),
            negative_prefix: "-".to_string(),
            negative_suffix: format!("{space}\u{00a4}"),
        },
        accounting: CurrencyPattern {
            positive_prefix: String::new(),
            positive_suffix: format!("{space}\u{00a4}"),
            negative_prefix: if accounting_parentheses {
                "(".to_string()
            } else {
                "-".to_string()
            },
            negative_suffix: if accounting_parentheses {
                format!("{space}\u{00a4})")
            } else {
                format!("{space}\u{00a4}")
            },
        },
    }
}

fn currency_names<const N: usize>(
    names: [(&'static str, &'static str, &'static str, &'static str); N],
) -> BTreeMap<String, CurrencyDisplayNames> {
    names
        .into_iter()
        .map(|(code, symbol, narrow_symbol, name)| {
            (
                code.to_string(),
                CurrencyDisplayNames {
                    symbol: Some(symbol.to_string()),
                    narrow_symbol: Some(narrow_symbol.to_string()),
                    name: Some(name.to_string()),
                },
            )
        })
        .collect()
}

fn compact_tier(exponent: i32, pattern: &str) -> CompactTier {
    CompactTier {
        exponent,
        one: Some(pattern.to_string()),
        other: pattern.to_string(),
    }
}
