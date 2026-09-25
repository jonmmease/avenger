use crate::{
    parse_datetime_timezone, DateTimeFormatContext, DateTimeLocaleSpec, PreparedDateTimeFormat,
    ResolvedDateTimeLocale,
};
use avenger_format::{
    DateTimeFormatError, DateTimeFormatProvider, NaiveDateTimeInput,
    PreparedCivilDateTimeFormatter, PreparedInstantFormatter, ZonedDateTimeInput,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

/// Locale definitions and display timezone for preparing D3 datetime patterns.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct D3DateTimeFormatConfig {
    /// Locale name. An omitted name selects `en-US`.
    pub locale: Option<String>,
    /// Custom D3 definitions, keyed by locale name.
    pub locales: BTreeMap<String, DateTimeLocaleSpec>,
    /// IANA display timezone for instants. An omitted name selects UTC.
    pub timezone: Option<String>,
}

impl D3DateTimeFormatConfig {
    /// Use U.S. English and UTC for instant display.
    pub fn new() -> Self {
        Self::default()
    }

    /// Select a locale, accepting hyphens or underscores in its name.
    pub fn with_locale(mut self, locale: impl Into<String>) -> Self {
        self.locale = Some(locale.into());
        self
    }

    /// Add or replace a custom definition without selecting it.
    pub fn with_custom_locale(
        mut self,
        name: impl Into<String>,
        definition: DateTimeLocaleSpec,
    ) -> Self {
        self.locales.insert(name.into(), definition);
        self
    }

    /// Set the IANA display timezone for instants. Civil fields are unchanged.
    pub fn with_timezone(mut self, timezone: impl Into<String>) -> Self {
        self.timezone = Some(timezone.into());
        self
    }
}

/// D3 datetime patterns and locale definitions through the shared interface.
#[derive(Debug, Default)]
pub struct D3DateTimeFormatProvider;

impl DateTimeFormatProvider for D3DateTimeFormatProvider {
    type Config = D3DateTimeFormatConfig;

    fn prepare_naive(
        &self,
        config: &Self::Config,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedCivilDateTimeFormatter>, DateTimeFormatError> {
        let prepared = prepare(config, pattern)?;
        prepared.validate_naive().map_err(error)?;
        Ok(Arc::new(prepared))
    }

    fn prepare_zoned(
        &self,
        config: &Self::Config,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedInstantFormatter>, DateTimeFormatError> {
        Ok(Arc::new(prepare(config, pattern)?))
    }
}

fn prepare(
    config: &D3DateTimeFormatConfig,
    pattern: &str,
) -> Result<PreparedDateTimeFormat, DateTimeFormatError> {
    let id = config.locale.as_deref().unwrap_or("en-US");
    let normalized = id.replace('_', "-");
    let data = config.locales.get(id).or_else(|| {
        config
            .locales
            .iter()
            .find_map(|(name, data)| (name.replace('_', "-") == normalized).then_some(data))
    });
    let locale = if let Some(definition) = data {
        ResolvedDateTimeLocale::new(id, definition.clone()).map_err(error)?
    } else if normalized == "en-US" {
        ResolvedDateTimeLocale::en_us()
    } else {
        return Err(DateTimeFormatError(format!("locale `{id}` was not found")));
    };
    let timezone =
        parse_datetime_timezone(config.timezone.as_deref().unwrap_or("UTC")).map_err(error)?;
    PreparedDateTimeFormat::new(Some(pattern), DateTimeFormatContext::new(&locale, timezone))
        .map_err(error)
}

fn error(error: crate::DateTimeFormatError) -> DateTimeFormatError {
    DateTimeFormatError(error.to_string())
}

impl PreparedCivilDateTimeFormatter for PreparedDateTimeFormat {
    fn format(&self, value: NaiveDateTimeInput) -> Result<String, DateTimeFormatError> {
        self.format_naive(value).map_err(error)
    }
}

impl PreparedInstantFormatter for PreparedDateTimeFormat {
    fn format(&self, value: ZonedDateTimeInput) -> Result<String, DateTimeFormatError> {
        self.format_zoned(value).map_err(error)
    }
}
