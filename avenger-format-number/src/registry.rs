use crate::{FormatError, NumberLocaleSpec, ResolvedNumberLocale};
use std::collections::BTreeMap;

/// Named D3 number locales.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NumberLocaleRegistry {
    locales: BTreeMap<String, ResolvedNumberLocale>,
}
impl NumberLocaleRegistry {
    /// Load the bundled D3 number definitions.
    pub fn with_builtins() -> Self {
        let mut registry = Self::default();
        for (id, json) in [
            ("en-US", include_str!("../locales/en-US.json")),
            ("de-DE", include_str!("../locales/de-DE.json")),
            ("fr-FR", include_str!("../locales/fr-FR.json")),
            ("ja-JP", include_str!("../locales/ja-JP.json")),
        ] {
            registry
                .register_custom_locale_json(id, json)
                .expect("bundled D3 number locale");
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
    /// Resolve a name, sharing its validated definition.
    pub fn resolve(&self, id: &str) -> Result<ResolvedNumberLocale, FormatError> {
        self.locales
            .get(id)
            .cloned()
            .ok_or_else(|| FormatError::LocaleNotFound(id.into()))
    }
}
