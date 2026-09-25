use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt::Debug, sync::Arc};

/// Named options interpreted and validated by the selected datetime provider.
pub type DateTimeFormatOptions = BTreeMap<String, serde_json::Value>;
/// Locale definitions in the selected provider's format, keyed by locale name.
pub type DateTimeLocaleData = BTreeMap<String, serde_json::Value>;

/// Serializable provider selection, locale data, and display timezone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DateTimeFormatConfig {
    /// Registered provider name.
    pub provider: String,
    /// Locale name understood by the provider. `None` uses its ordinary locale.
    pub locale: Option<String>,
    /// Custom locale definitions in the selected provider’s data format.
    #[serde(default)]
    pub locales: DateTimeLocaleData,
    /// IANA display timezone for instants. `None` uses UTC. Civil fields are unchanged.
    pub timezone: Option<String>,
}
impl DateTimeFormatConfig {
    /// Select a provider with its ordinary locale and UTC display timezone.
    pub fn new(provider: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            locale: None,
            locales: BTreeMap::new(),
            timezone: None,
        }
    }

    /// Select a locale understood by the provider.
    pub fn with_locale(mut self, locale: impl Into<String>) -> Self {
        self.locale = Some(locale.into());
        self
    }

    /// Set the IANA display timezone for instants. Civil fields are unchanged.
    pub fn with_timezone(mut self, timezone: impl Into<String>) -> Self {
        self.timezone = Some(timezone.into());
        self
    }

    /// Add or replace a provider-specific locale definition without selecting it.
    pub fn with_custom_locale(
        mut self,
        name: impl Into<String>,
        definition: serde_json::Value,
    ) -> Self {
        self.locales.insert(name.into(), definition);
        self
    }
}

/// Preparation input. Pattern syntax and named options belong to the provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateTimeFormatRequest {
    /// A provider-specific pattern or structured specification.
    pub spec: serde_json::Value,
    pub options: DateTimeFormatOptions,
    /// Explicit per-call timezone override. Invalid for civil input.
    pub timezone: Option<String>,
}

impl DateTimeFormatRequest {
    /// Supply a format specification with no options or timezone override.
    pub fn new(spec: impl Into<serde_json::Value>) -> Self {
        Self {
            spec: spec.into(),
            options: BTreeMap::new(),
            timezone: None,
        }
    }
}

/// Civil calendar fields without an instant or display offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NaiveDateTimeInput {
    Date(chrono::NaiveDate),
    DateTime(chrono::NaiveDateTime),
}
impl NaiveDateTimeInput {
    /// Preserve datetime fields and interpret a date-only value as midnight.
    pub fn datetime(self) -> chrono::NaiveDateTime {
        match self {
            Self::Date(date) => date.and_time(chrono::NaiveTime::MIN),
            Self::DateTime(value) => value,
        }
    }
}
/// An instant represented in UTC and displayed in the configured timezone.
pub type ZonedDateTimeInput = chrono::DateTime<chrono::Utc>;

/// A provider configuration or input-value error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct DateTimeFormatError(pub String);

/// Resolve syntax, options, and locale data once for a sequence of labels.
/// Registered providers must preserve their behavior throughout a registry snapshot.
pub trait DateTimeFormatProvider: Debug + Send + Sync + 'static {
    /// Prepare for civil dates and datetimes, rejecting options or fields that require an instant.
    fn prepare_naive(
        &self,
        config: &DateTimeFormatConfig,
        request: &DateTimeFormatRequest,
    ) -> Result<Arc<dyn PreparedCivilDateTimeFormatter>, DateTimeFormatError>;

    /// Prepare for instants displayed in the configured timezone.
    fn prepare_zoned(
        &self,
        config: &DateTimeFormatConfig,
        request: &DateTimeFormatRequest,
    ) -> Result<Arc<dyn PreparedInstantFormatter>, DateTimeFormatError>;
}

/// Immutable, deterministic formatting of civil calendar fields.
/// Preparation validates the specification; formatting can still reject unsupported values.
pub trait PreparedCivilDateTimeFormatter: Debug + Send + Sync + 'static {
    fn format(&self, value: NaiveDateTimeInput) -> Result<String, DateTimeFormatError>;
}

/// Immutable, deterministic formatting of instants in a resolved display timezone.
/// Formatting reports values whose display date is outside the supported range.
pub trait PreparedInstantFormatter: Debug + Send + Sync + 'static {
    fn format(&self, value: ZonedDateTimeInput) -> Result<String, DateTimeFormatError>;
}

/// An immutable snapshot when shared through `Arc`. Provider replacement gives a new cache identity.
#[derive(Debug, Clone)]
pub struct DateTimeFormatRegistry {
    providers: BTreeMap<String, Arc<dyn DateTimeFormatProvider>>,
    cache_id: usize,
}
impl Default for DateTimeFormatRegistry {
    fn default() -> Self {
        Self {
            providers: BTreeMap::new(),
            cache_id: crate::next_cache_id(),
        }
    }
}
impl PartialEq for DateTimeFormatRegistry {
    fn eq(&self, other: &Self) -> bool {
        self.cache_id == other.cache_id
    }
}
impl Eq for DateTimeFormatRegistry {}
impl DateTimeFormatRegistry {
    /// Add or replace a provider and invalidate cache keys derived from this registry.
    pub fn register(&mut self, name: impl Into<String>, provider: Arc<dyn DateTimeFormatProvider>) {
        self.providers.insert(name.into(), provider);
        self.cache_id = crate::next_cache_id();
    }
    /// Process-local identity for label caches. Do not persist it in scene data.
    pub fn cache_id(&self) -> usize {
        self.cache_id
    }
    /// Prepare a civil formatter through the explicitly selected provider.
    pub fn prepare_naive(
        &self,
        config: &DateTimeFormatConfig,
        request: &DateTimeFormatRequest,
    ) -> Result<Arc<dyn PreparedCivilDateTimeFormatter>, DateTimeFormatError> {
        self.provider(config)?.prepare_naive(config, request)
    }

    /// Prepare an instant formatter through the explicitly selected provider.
    pub fn prepare_zoned(
        &self,
        config: &DateTimeFormatConfig,
        request: &DateTimeFormatRequest,
    ) -> Result<Arc<dyn PreparedInstantFormatter>, DateTimeFormatError> {
        self.provider(config)?.prepare_zoned(config, request)
    }

    fn provider(
        &self,
        config: &DateTimeFormatConfig,
    ) -> Result<&dyn DateTimeFormatProvider, DateTimeFormatError> {
        self.providers
            .get(&config.provider)
            .map(Arc::as_ref)
            .ok_or_else(|| {
                DateTimeFormatError(format!(
                    "datetime format provider `{}` was not found",
                    config.provider
                ))
            })
    }
}
