use crate::{error::FormatError, locale::ResolvedNumberLocale};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CurrencyMetadata {
    pub code: &'static str,
    pub default_fraction_digits: u8,
}

pub fn currency_metadata(code: &str) -> Option<CurrencyMetadata> {
    match code {
        "USD" => Some(CurrencyMetadata {
            code: "USD",
            default_fraction_digits: 2,
        }),
        "EUR" => Some(CurrencyMetadata {
            code: "EUR",
            default_fraction_digits: 2,
        }),
        "JPY" => Some(CurrencyMetadata {
            code: "JPY",
            default_fraction_digits: 0,
        }),
        _ => None,
    }
}

pub fn validate_currency_code(code: &str) -> Result<(), FormatError> {
    if currency_metadata(code).is_some() {
        Ok(())
    } else {
        Err(FormatError::InvalidCurrencyCode(code.to_string()))
    }
}

pub(crate) fn currency_display_text(
    locale: &ResolvedNumberLocale,
    code: &str,
    display: crate::locale::CurrencyDisplay,
) -> String {
    let Some(names) = locale.currency_names.get(code) else {
        return code.to_string();
    };

    match display {
        crate::locale::CurrencyDisplay::Symbol => names
            .symbol
            .as_ref()
            .or(names.narrow_symbol.as_ref())
            .cloned()
            .unwrap_or_else(|| code.to_string()),
        crate::locale::CurrencyDisplay::Code => code.to_string(),
        crate::locale::CurrencyDisplay::Name => {
            names.name.clone().unwrap_or_else(|| code.to_string())
        }
        crate::locale::CurrencyDisplay::NarrowSymbol => names
            .narrow_symbol
            .as_ref()
            .or(names.symbol.as_ref())
            .cloned()
            .unwrap_or_else(|| code.to_string()),
    }
}
