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
}

/// The caller's purpose, without choosing datetime patterns or generating ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DateTimeFormatContext {
    /// Format an individual value using the provider's ordinary pattern.
    #[default]
    Scalar,
    /// Format data values with patterns appropriate to dates, civil datetimes, or instants.
    Data,
    /// Let the provider choose labels for calendar tick boundaries.
    Tick,
}

/// Preparation input. Pattern syntax and named options belong to the provider.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DateTimeFormatRequest {
    /// A provider-specific pattern or structured specification.
    pub spec: Option<serde_json::Value>,
    pub options: DateTimeFormatOptions,
    pub context: DateTimeFormatContext,
    /// Explicit per-call timezone override. Invalid for civil input.
    pub timezone: Option<String>,
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
    /// Validate the request and retain the configuration needed to format values.
    fn prepare(
        &self,
        config: &DateTimeFormatConfig,
        request: &DateTimeFormatRequest,
    ) -> Result<Arc<dyn PreparedDateTimeFormatter>, DateTimeFormatError>;
}

/// Immutable, deterministic formatting of civil fields and instants.
pub trait PreparedDateTimeFormatter: Debug + Send + Sync + 'static {
    /// Check pattern compatibility with civil inputs before formatting a batch.
    fn validate_naive(&self) -> Result<(), DateTimeFormatError>;
    /// Preserve civil calendar fields; reject explicit per-call timezone overrides.
    fn format_naive(&self, value: NaiveDateTimeInput) -> Result<String, DateTimeFormatError>;
    /// Display an instant in the configured timezone, reporting unrepresentable values.
    fn format_zoned(&self, value: ZonedDateTimeInput) -> Result<String, DateTimeFormatError>;
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
    /// Prepare through the explicitly selected provider.
    pub fn prepare(
        &self,
        config: &DateTimeFormatConfig,
        request: &DateTimeFormatRequest,
    ) -> Result<Arc<dyn PreparedDateTimeFormatter>, DateTimeFormatError> {
        self.providers
            .get(&config.provider)
            .ok_or_else(|| {
                DateTimeFormatError(format!(
                    "datetime format provider `{}` was not found",
                    config.provider
                ))
            })?
            .prepare(config, request)
    }
}
