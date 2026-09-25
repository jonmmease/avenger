use crate::{
    currency_data::CURRENCY_FRACTION_DIGITS, format::ResolvedNumberFormat, FormatError,
    NumberLocaleExtensions, ResolvedNumberLocale, SignPolicy,
};
use serde::{Deserialize, Serialize};

/// Affixes around a currency amount. `¤` inserts its symbol or code and `-` inserts the locale minus.
/// Each positive or negative prefix/suffix pair must contain exactly one `¤`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrencyPattern {
    pub positive_prefix: String,
    pub positive_suffix: String,
    pub negative_prefix: String,
    pub negative_suffix: String,
}

/// Standard and accounting affixes, independent of the D3 `$` affixes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrencyFormat {
    pub standard: CurrencyPattern,
    pub accounting: CurrencyPattern,
    /// Inserted between an adjacent alphabetic currency affix and numeric digit.
    #[serde(default = "currency_space")]
    pub alphabetic_spacing: String,
}
fn currency_space() -> String {
    "\u{a0}".into()
}
impl Default for CurrencyFormat {
    fn default() -> Self {
        Self {
            standard: CurrencyPattern {
                positive_prefix: "¤".into(),
                positive_suffix: String::new(),
                negative_prefix: "-¤".into(),
                negative_suffix: String::new(),
            },
            accounting: CurrencyPattern {
                positive_prefix: "¤".into(),
                positive_suffix: String::new(),
                negative_prefix: "(¤".into(),
                negative_suffix: ")".into(),
            },
            alphabetic_spacing: currency_space(),
        }
    }
}

/// Localized currency symbols. A missing symbol falls back to the other symbol, then the code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CurrencySymbols {
    pub symbol: Option<String>,
    /// An abbreviated symbol that may be ambiguous outside its locale.
    pub narrow_symbol: Option<String>,
}

/// Select the symbol or uppercase code inserted into currency affixes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CurrencyDisplay {
    #[default]
    Symbol,
    Code,
    /// Prefer the shorter symbol even if it is shared by multiple currencies.
    NarrowSymbol,
}

/// Default fraction digits from the pinned CLDR currency data.
pub(crate) fn fraction_digits(code: &str) -> Option<u8> {
    CURRENCY_FRACTION_DIGITS
        .binary_search_by_key(&code, |&(code, _)| code)
        .ok()
        .map(|i| CURRENCY_FRACTION_DIGITS[i].1)
}

pub(crate) fn validate_metadata(metadata: &NumberLocaleExtensions) -> Result<(), FormatError> {
    for pattern in [&metadata.currency.standard, &metadata.currency.accounting] {
        for (before, after) in [
            (&pattern.positive_prefix, &pattern.positive_suffix),
            (&pattern.negative_prefix, &pattern.negative_suffix),
        ] {
            if before.matches('¤').count() + after.matches('¤').count() != 1 {
                return Err(FormatError::InvalidLocaleData(
                    "currency affixes require one `¤` placeholder per sign".into(),
                ));
            }
        }
    }
    if metadata
        .currency_symbols
        .keys()
        .any(|code| fraction_digits(code).is_none())
    {
        return Err(FormatError::InvalidLocaleData(
            "currency symbols require a recognized uppercase currency code".into(),
        ));
    }
    Ok(())
}

fn display_text<'a>(
    locale: &'a ResolvedNumberLocale,
    code: &'a str,
    display: CurrencyDisplay,
) -> &'a str {
    let Some(names) = locale.extensions.currency_symbols.get(code) else {
        return code;
    };
    match display {
        CurrencyDisplay::Code => code,
        CurrencyDisplay::Symbol => names
            .symbol
            .as_deref()
            .or(names.narrow_symbol.as_deref())
            .unwrap_or(code),
        CurrencyDisplay::NarrowSymbol => names
            .narrow_symbol
            .as_deref()
            .or(names.symbol.as_deref())
            .unwrap_or(code),
    }
}

/// Expand tokens once so punctuation inside an inserted symbol remains literal.
fn expand(pattern: &str, currency: &str, minus: &str) -> String {
    let mut result = String::new();
    for ch in pattern.chars() {
        match ch {
            '¤' => result.push_str(currency),
            '-' => result.push_str(minus),
            _ => result.push(ch),
        }
    }
    result
}

pub(crate) fn affixes(
    format: &ResolvedNumberFormat,
    locale: &ResolvedNumberLocale,
    negative: bool,
    number: &str,
) -> (String, String) {
    let code = format
        .currency
        .as_deref()
        .expect("validated currency format");
    let currency = display_text(locale, code, format.currency_display);
    let data = &locale.extensions.currency;
    let pattern = if format.sign == SignPolicy::Parentheses {
        &data.accounting
    } else {
        &data.standard
    };
    let (before, after) = if negative {
        (&pattern.negative_prefix, &pattern.negative_suffix)
    } else {
        (&pattern.positive_prefix, &pattern.positive_suffix)
    };
    let mut prefix = expand(before, currency, &locale.minus);
    let mut suffix = expand(after, currency, &locale.minus);
    if prefix.chars().last().is_some_and(char::is_alphabetic)
        && number.starts_with(|c: char| c.is_ascii_digit())
    {
        prefix.push_str(&data.alphabetic_spacing);
    }
    if suffix.chars().next().is_some_and(char::is_alphabetic)
        && number.ends_with(|c: char| c.is_ascii_digit())
    {
        suffix.insert_str(0, &data.alphabetic_spacing);
    }
    if !negative {
        match format.sign {
            SignPolicy::Plus => prefix.insert(0, '+'),
            SignPolicy::Space => prefix.insert(0, ' '),
            _ => {}
        }
    }
    (prefix, suffix)
}
