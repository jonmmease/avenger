use crate::{NumberFormatError, NumberLocaleSpec, ResolvedNumberLocale};
use std::collections::BTreeMap;

/// Named D3 number locales. [`Self::default`] creates an empty registry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NumberLocaleRegistry {
    locales: BTreeMap<String, ResolvedNumberLocale>,
}
impl NumberLocaleRegistry {
    /// Load U.S. English, or all D3 locales with the `all-locales` feature.
    pub fn with_builtins() -> Self {
        let mut registry = Self::default();
        for &(id, json) in avenger_format_locales_d3::BUNDLED_NUMBER {
            registry
                .register_custom_locale_json(id, json)
                .expect("bundled D3 number locale");
        }
        registry
    }
    /// Replace a named definition after validation succeeds.
    /// Previously resolved locales retain their definitions.
    pub fn register_custom_locale(
        &mut self,
        id: impl Into<String>,
        spec: NumberLocaleSpec,
    ) -> Result<(), NumberFormatError> {
        let id = id.into();
        let locale = ResolvedNumberLocale::new(id.clone(), spec)?;
        self.locales.insert(id, locale);
        Ok(())
    }
    /// Parse and register D3 locale JSON, leaving any existing entry intact on error.
    pub fn register_custom_locale_json(
        &mut self,
        id: impl Into<String>,
        json: &str,
    ) -> Result<(), NumberFormatError> {
        let id = id.into();
        let spec = NumberLocaleSpec::from_json(&id, json)?;
        self.register_custom_locale(id, spec)
    }
    /// Resolve an exact, case-sensitive name, sharing its validated definition.
    /// Unknown names return [`NumberFormatError::LocaleUnavailable`] without a fallback.
    pub fn resolve(&self, id: &str) -> Result<ResolvedNumberLocale, NumberFormatError> {
        self.locales
            .get(id)
            .cloned()
            .ok_or_else(|| NumberFormatError::LocaleUnavailable {
                locale: id.into(),
                message: "locale is not registered".into(),
            })
    }
}
