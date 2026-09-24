use crate::{FormatError, NumberLocaleSpec, ResolvedNumberLocale};
use std::collections::BTreeMap;

/// Named D3 number locales. [`Self::default`] creates an empty registry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NumberLocaleRegistry {
    locales: BTreeMap<String, ResolvedNumberLocale>,
}
impl NumberLocaleRegistry {
    /// Create a registry containing the bundled U.S. English locale.
    pub fn with_builtins() -> Self {
        Self {
            locales: BTreeMap::from([("en-US".into(), ResolvedNumberLocale::en_us())]),
        }
    }
    /// Replace a named definition after validation succeeds.
    /// Previously resolved locales retain their definitions.
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
    /// Parse and register D3 locale JSON, leaving any existing entry intact on error.
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
    /// Resolve an exact, case-sensitive name, sharing its validated definition.
    /// Unknown names return [`FormatError::LocaleNotFound`] without a fallback.
    pub fn resolve(&self, id: &str) -> Result<ResolvedNumberLocale, FormatError> {
        self.locales
            .get(id)
            .cloned()
            .ok_or_else(|| FormatError::LocaleNotFound(id.into()))
    }
}
