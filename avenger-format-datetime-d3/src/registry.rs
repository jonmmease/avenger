use crate::{DateTimeFormatError, DateTimeLocaleSpec, ResolvedDateTimeLocale};
use std::collections::BTreeMap;

/// Named, validated D3 time locales owned by a formatting context.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DateTimeLocaleRegistry {
    locales: BTreeMap<String, ResolvedDateTimeLocale>,
}
impl DateTimeLocaleRegistry {
    /// Load U.S. English, or all D3 locales with the `all-locales` feature.
    pub fn with_builtins() -> Self {
        let mut registry = Self::default();
        for &(id, json) in crate::bundled::LOCALES {
            registry
                .register_custom_locale_json(id, json)
                .expect("bundled D3 time locale");
        }
        registry
    }
    /// Register a definition after validating its locale expansions.
    pub fn register_custom_locale(
        &mut self,
        id: impl Into<String>,
        spec: DateTimeLocaleSpec,
    ) -> Result<(), DateTimeFormatError> {
        let id = id.into();
        let locale = ResolvedDateTimeLocale::new(id.clone(), spec)?;
        self.locales.insert(id, locale);
        Ok(())
    }
    /// Register an upstream D3 time locale JSON object.
    pub fn register_custom_locale_json(
        &mut self,
        id: impl Into<String>,
        json: &str,
    ) -> Result<(), DateTimeFormatError> {
        let id = id.into();
        let spec =
            serde_json::from_str(json).map_err(|error| DateTimeFormatError::InvalidLocaleData {
                message: format!("locale `{id}`: {error}"),
            })?;
        self.register_custom_locale(id, spec)
    }
    /// Resolve a name, sharing the validated definition with the caller.
    pub fn resolve(&self, id: &str) -> Result<ResolvedDateTimeLocale, DateTimeFormatError> {
        self.locales
            .get(id)
            .cloned()
            .ok_or_else(|| DateTimeFormatError::LocaleUnavailable {
                locale: id.into(),
                message: "locale is not registered".into(),
            })
    }
}
