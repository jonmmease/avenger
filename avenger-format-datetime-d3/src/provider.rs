use crate::{
    parse_datetime_timezone, DateTimeFormatContext, DateTimeLocaleSpec, PreparedDateTimeFormat,
    ResolvedDateTimeLocale,
};
use avenger_format::{
    DateTimeFormatError, DateTimeFormatProvider, PreparedDateFormatter,
    PreparedNaiveDateTimeFormatter, PreparedZonedDateTimeFormatter,
};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

/// Prepare D3 datetime patterns with locale definitions and a display timezone.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct D3DateTimeFormatProvider {
    /// Locale name. An omitted name selects `en-US`.
    pub locale: Option<String>,
    /// Custom D3 definitions, keyed by locale name.
    pub locales: BTreeMap<String, DateTimeLocaleSpec>,
    /// IANA display timezone for zoned datetimes. An omitted name selects UTC.
    pub timezone: Option<String>,
}

impl D3DateTimeFormatProvider {
    /// Use U.S. English and UTC for zoned formatting.
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

    /// Set the IANA display timezone for zoned datetimes. Naive fields are unchanged.
    pub fn with_timezone(mut self, timezone: impl Into<String>) -> Self {
        self.timezone = Some(timezone.into());
        self
    }
}

impl DateTimeFormatProvider for D3DateTimeFormatProvider {
    fn prepare_date(
        &self,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedDateFormatter>, DateTimeFormatError> {
        let prepared = prepare(self, pattern, chrono_tz::UTC)?;
        prepared.validate_date().map_err(error)?;
        Ok(Arc::new(prepared))
    }

    fn prepare_naive(
        &self,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedNaiveDateTimeFormatter>, DateTimeFormatError> {
        let prepared = prepare(self, pattern, chrono_tz::UTC)?;
        prepared.validate_naive().map_err(error)?;
        Ok(Arc::new(prepared))
    }

    fn prepare_zoned(
        &self,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedZonedDateTimeFormatter>, DateTimeFormatError> {
        let timezone =
            parse_datetime_timezone(self.timezone.as_deref().unwrap_or("UTC")).map_err(error)?;
        Ok(Arc::new(prepare(self, pattern, timezone)?))
    }
}

fn prepare(
    provider: &D3DateTimeFormatProvider,
    pattern: &str,
    timezone: chrono_tz::Tz,
) -> Result<PreparedDateTimeFormat, DateTimeFormatError> {
    let id = provider.locale.as_deref().unwrap_or("en-US");
    let normalized = id.replace('_', "-");
    let data = provider.locales.get(id).or_else(|| {
        provider
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
    PreparedDateTimeFormat::new(Some(pattern), DateTimeFormatContext::new(&locale, timezone))
        .map_err(error)
}

fn error(error: crate::DateTimeFormatError) -> DateTimeFormatError {
    DateTimeFormatError(error.to_string())
}

impl PreparedDateFormatter for PreparedDateTimeFormat {
    fn format(&self, value: NaiveDate) -> Result<String, DateTimeFormatError> {
        self.format_date(value).map_err(error)
    }
}

impl PreparedNaiveDateTimeFormatter for PreparedDateTimeFormat {
    fn format(&self, value: NaiveDateTime) -> Result<String, DateTimeFormatError> {
        self.format_naive(value).map_err(error)
    }
}

impl PreparedZonedDateTimeFormatter for PreparedDateTimeFormat {
    fn format(&self, value: DateTime<Utc>) -> Result<String, DateTimeFormatError> {
        self.format_zoned(value).map_err(error)
    }
}
