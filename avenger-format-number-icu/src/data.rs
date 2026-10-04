//! Locale resolution and compiled-data access shared by formatter components.

use crate::skeleton::unsupported;
use avenger_format::NumberFormatError;
use icu_decimal::{
    provider::{Baked, DecimalDigitsV1, DecimalSymbolsV1},
    DecimalFormatterPreferences,
};
use icu_locale::{LocaleCanonicalizer, LocaleFallbacker};
use icu_locale_core::Locale;
use icu_provider::prelude::*;

/// Supplemental CLDR number data that ICU4X does not compile.
#[derive(Debug)]
pub(crate) struct Symbols {
    pub nan: &'static str,
    pub infinity: &'static str,
}

include!("generated.rs");

pub(crate) fn data_error(error: DataError) -> NumberFormatError {
    NumberFormatError::InvalidLocaleData {
        message: error.to_string(),
    }
}

fn silent() -> DataRequestMetadata {
    let mut metadata = DataRequestMetadata::default();
    metadata.silent = true;
    metadata
}

/// Find the nearest supplemental row along a fallback chain.
fn find_symbols(chain: &[String], numbering_system: &str) -> Option<&'static Symbols> {
    chain.iter().find_map(|locale| {
        let key = format!("{locale}/{numbering_system}");
        SYMBOLS
            .binary_search_by_key(&key.as_str(), |(k, _)| *k)
            .ok()
            .map(|i| &SYMBOL_VALUES[SYMBOLS[i].1])
    })
}

/// CLDR locales whose number data ICU4X 2.3 lacks. ICU4X would format them with root data.
const ICU4X_MISSING: [&str; 3] = ["az-Cyrl", "pa-Arab", "uz-Arab"];

/// A requested locale resolved through ICU4X's fallback chain.
#[derive(Debug)]
pub(crate) struct Context {
    pub prefs: DecimalFormatterPreferences,
    pub symbols: &'static Symbols,
}

impl Context {
    /// Resolve a locale and optional numbering system, rejecting locales without CLDR number data.
    pub fn new(name: &str, numbering_system: Option<&str>) -> Result<Self, NumberFormatError> {
        let unavailable = |message: String| NumberFormatError::LocaleUnavailable {
            locale: name.into(),
            message,
        };
        let mut locale: Locale = name
            .replace('_', "-")
            .parse()
            .map_err(|e: icu_locale_core::ParseError| unavailable(e.to_string()))?;
        LocaleCanonicalizer::new_extended().canonicalize(&mut locale);
        let mut prefs = DecimalFormatterPreferences::from_locale_strict(&locale)
            .map_err(|_| unavailable("invalid Unicode locale preference".into()))?;
        if let Some(nu) = numbering_system {
            let override_locale = format!("und-u-nu-{nu}")
                .parse::<Locale>()
                .map_err(|_| unsupported(nu, "invalid numbering system"))?;
            prefs.numbering_system =
                DecimalFormatterPreferences::from(override_locale).numbering_system;
        }
        let data_locale = DecimalSymbolsV1::INFO.make_locale(prefs.locale_preferences);
        let defaults = DataProvider::<DecimalSymbolsV1>::load(
            &Baked,
            DataRequest {
                id: DataIdentifierBorrowed::for_locale(&data_locale),
                ..Default::default()
            },
        )
        .map_err(|e| unavailable(e.to_string()))?;
        let numbering_system = prefs
            .numbering_system
            .map(|n| n.to_string())
            .unwrap_or_else(|| defaults.payload.get().numsys().into());
        // Without digit data, ICU4X would silently substitute Latin digits.
        let digits = DataMarkerAttributes::try_from_str(&numbering_system)
            .ok()
            .and_then(|attributes| {
                DataProvider::<DecimalDigitsV1>::load(
                    &Baked,
                    DataRequest {
                        id: DataIdentifierBorrowed::for_marker_attributes_and_locale(
                            attributes,
                            &DataLocale::default(),
                        ),
                        metadata: silent(),
                    },
                )
                .ok()
            });
        if digits.is_none() {
            return Err(unsupported(
                &numbering_system,
                "ICU has no digits for this numbering system",
            ));
        }
        let fallbacker = LocaleFallbacker::new().for_config(DecimalSymbolsV1::INFO.fallback_config);
        let mut iter = fallbacker.fallback_for(data_locale);
        let mut chain = Vec::new();
        while !iter.get().is_unknown() {
            chain.push(iter.get().to_string());
            iter.step();
        }
        if locale.id.language.is_unknown() {
            chain.push("und".into());
        }
        if let Some(missing) = chain.iter().find(|l| ICU4X_MISSING.contains(&l.as_str())) {
            return Err(unavailable(format!(
                "ICU4X has no number data for `{missing}`"
            )));
        }
        // Locales lacking symbols for a requested numbering system use their ordinary symbols.
        let symbols = find_symbols(&chain, &numbering_system)
            .or_else(|| find_symbols(&chain, "latn"))
            .ok_or_else(|| unavailable("ICU has no localized number data".into()))?;
        Ok(Self { prefs, symbols })
    }

    /// Load numbering-system data when requested, then the locale's default data.
    pub fn load<M, P>(&self, provider: &P) -> Result<DataPayload<M>, DataError>
    where
        M: DataMarker,
        P: DataProvider<M>,
    {
        let locale = M::INFO.make_locale(self.prefs.locale_preferences);
        if let Some(id) = self.prefs.nu_id(&locale) {
            let request = DataRequest {
                id,
                metadata: silent(),
            };
            if let Some(response) = provider.load(request).allow_identifier_not_found()? {
                return Ok(response.payload);
            }
        }
        let request = DataRequest {
            id: DataIdentifierBorrowed::for_locale(&locale),
            ..Default::default()
        };
        Ok(provider.load(request)?.payload)
    }
}
