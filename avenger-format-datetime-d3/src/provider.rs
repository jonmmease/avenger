use crate::{
    DateTimeFormatContext, DateTimeLocaleSpec, PreparedDateTimeFormat, ResolvedDateTimeLocale,
};
use avenger_format::{
    DateTimeFormatError, DateTimeFormatProvider, DateTimeInputKind, PreparedDateFormatter,
    PreparedNaiveDateTimeFormatter, PreparedZonedDateTimeFormatter,
};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

/// Prepare D3 datetime patterns with locale definitions and a display timezone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct D3DateTimeFormatProvider {
    /// Locale name. An omitted name selects `en-US`.
    pub locale: Option<String>,
    /// Custom D3 definitions, keyed by locale name.
    pub locales: BTreeMap<String, DateTimeLocaleSpec>,
    /// Resolved IANA display timezone for zoned datetimes. Defaults to UTC.
    pub timezone: Tz,
}

impl Default for D3DateTimeFormatProvider {
    fn default() -> Self {
        Self {
            locale: None,
            locales: BTreeMap::new(),
            timezone: Tz::UTC,
        }
    }
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
    pub fn with_timezone(mut self, timezone: Tz) -> Self {
        self.timezone = timezone;
        self
    }
}

impl DateTimeFormatProvider for D3DateTimeFormatProvider {
    fn prepare_date(
        &self,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedDateFormatter>, DateTimeFormatError> {
        let prepared = prepare(self, pattern, chrono_tz::UTC)?;
        prepared.validate_input(DateTimeInputKind::Date)?;
        Ok(Arc::new(prepared))
    }

    fn prepare_naive(
        &self,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedNaiveDateTimeFormatter>, DateTimeFormatError> {
        let prepared = prepare(self, pattern, chrono_tz::UTC)?;
        prepared.validate_input(DateTimeInputKind::Naive)?;
        Ok(Arc::new(prepared))
    }

    fn prepare_zoned(
        &self,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedZonedDateTimeFormatter>, DateTimeFormatError> {
        Ok(Arc::new(prepare(self, pattern, self.timezone)?))
    }
}

fn prepare(
    provider: &D3DateTimeFormatProvider,
    pattern: &str,
    timezone: Tz,
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
        ResolvedDateTimeLocale::new(id, definition.clone())?
    } else {
        crate::bundled::resolve(&normalized)?
    };
    PreparedDateTimeFormat::new(Some(pattern), DateTimeFormatContext::new(&locale, timezone))
}

impl PreparedDateFormatter for PreparedDateTimeFormat {
    fn format(&self, value: NaiveDate) -> Result<String, DateTimeFormatError> {
        self.format_date(value)
    }
}

impl PreparedNaiveDateTimeFormatter for PreparedDateTimeFormat {
    fn format(&self, value: NaiveDateTime) -> Result<String, DateTimeFormatError> {
        self.format_naive(value)
    }
}

impl PreparedZonedDateTimeFormatter for PreparedDateTimeFormat {
    fn format(&self, value: DateTime<Utc>) -> Result<String, DateTimeFormatError> {
        self.format_zoned(value)
    }
}
