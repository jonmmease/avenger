//! Provider interfaces for preparing and rendering localized number and datetime labels.

mod datetime;
pub use datetime::{
    DateTimeFormatConfig, DateTimeFormatError, DateTimeFormatProvider, DateTimeFormatRegistry,
    DateTimeLocaleData, NaiveDateTimeInput, PreparedCivilDateTimeFormatter,
    PreparedInstantFormatter, ZonedDateTimeInput,
};

mod formatted_number;
pub use formatted_number::{FormattedNumber, NumberTypesetting};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fmt::Debug,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

/// Named options interpreted and validated by the selected provider.
pub type NumberFormatOptions = BTreeMap<String, serde_json::Value>;
/// Locale definitions in the selected provider's data format, keyed by locale name.
pub type NumberLocaleData = BTreeMap<String, serde_json::Value>;

/// Serializable provider selection and locale data shared by measurement and rendering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NumberFormatConfig {
    /// Registered provider name.
    pub provider: String,
    /// Locale name understood by the provider. `None` uses its default locale.
    pub locale: Option<String>,
    /// Custom locale definitions in the selected provider’s data format.
    #[serde(default)]
    pub locales: NumberLocaleData,
}
impl NumberFormatConfig {
    /// Select a provider with its ordinary locale and no custom locale definitions.
    pub fn new(provider: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            locale: None,
            locales: BTreeMap::new(),
        }
    }

    /// Select a locale understood by the provider.
    pub fn with_locale(mut self, locale: impl Into<String>) -> Self {
        self.locale = Some(locale.into());
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

/// Input to preparation. Specifier syntax and named options belong to the provider.
#[derive(Debug, Clone, PartialEq)]
pub struct NumberFormatRequest {
    /// Explicit provider-specific specifier.
    pub spec: String,
    pub options: NumberFormatOptions,
}

impl NumberFormatRequest {
    /// Supply a format specification with no additional options.
    pub fn new(spec: impl Into<String>) -> Self {
        Self {
            spec: spec.into(),
            options: BTreeMap::new(),
        }
    }
}

impl From<&str> for NumberFormatRequest {
    fn from(spec: &str) -> Self {
        Self::new(spec)
    }
}

impl From<String> for NumberFormatRequest {
    fn from(spec: String) -> Self {
        Self::new(spec)
    }
}

impl From<&NumberFormatRequest> for NumberFormatRequest {
    fn from(request: &NumberFormatRequest) -> Self {
        request.clone()
    }
}

/// A configuration error detected before rendering values.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct NumberFormatError(pub String);

/// Parse options and resolve locale data once for a sequence of numeric labels.
/// The same request must preserve formatting behavior throughout a registry snapshot.
pub trait NumberFormatProvider: Debug + Send + Sync + 'static {
    fn prepare(
        &self,
        config: &NumberFormatConfig,
        request: &NumberFormatRequest,
    ) -> Result<Arc<dyn PreparedNumberFormatter>, NumberFormatError>;
}

/// A reusable formatter. Implementations must be immutable and deterministic,
/// including their handling of NaN, infinity, and signed zero.
pub trait PreparedNumberFormatter: Debug + Send + Sync + 'static {
    fn format(&self, value: f64) -> FormattedNumber;
}

/// An immutable snapshot when shared through `Arc`. Cloning and registering a
/// provider creates a new cache identity while existing snapshots retain theirs.
#[derive(Debug, Clone)]
pub struct NumberFormatRegistry {
    providers: BTreeMap<String, Arc<dyn NumberFormatProvider>>,
    cache_id: usize,
}
fn next_cache_id() -> usize {
    static NEXT: AtomicUsize = AtomicUsize::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}
impl Default for NumberFormatRegistry {
    fn default() -> Self {
        Self {
            providers: BTreeMap::new(),
            cache_id: next_cache_id(),
        }
    }
}
impl PartialEq for NumberFormatRegistry {
    fn eq(&self, other: &Self) -> bool {
        self.cache_id == other.cache_id
    }
}
impl Eq for NumberFormatRegistry {}
impl NumberFormatRegistry {
    /// Add or replace a provider and invalidate cache keys derived from this registry.
    pub fn register(&mut self, name: impl Into<String>, provider: Arc<dyn NumberFormatProvider>) {
        self.providers.insert(name.into(), provider);
        self.cache_id = next_cache_id();
    }
    /// Process-local identity for label caches. Do not persist it in scene data.
    pub fn cache_id(&self) -> usize {
        self.cache_id
    }
    /// Prepare from a format string or a request with provider-specific options.
    pub fn prepare(
        &self,
        config: &NumberFormatConfig,
        request: impl Into<NumberFormatRequest>,
    ) -> Result<Arc<dyn PreparedNumberFormatter>, NumberFormatError> {
        self.providers
            .get(&config.provider)
            .ok_or_else(|| {
                NumberFormatError(format!(
                    "number format provider `{}` was not found",
                    config.provider
                ))
            })?
            .prepare(config, &request.into())
    }
}
