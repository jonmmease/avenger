//! Provider interfaces for preparing and rendering localized number labels.

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
}

/// Numeric context supplied by the caller, without selecting or generating ticks.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum NumberFormatContext {
    /// Format an individual value using the provider's ordinary defaults.
    #[default]
    Scalar,
    /// Choose automatic precision for unrelated or unevenly spaced numeric values.
    Continuous,
    /// Preserve numeric category identities unless an explicit format is supplied.
    Discrete,
    /// Coordinate precision and units for a set of labels.
    Step {
        /// Selected spacing between ticks. Zero or non-finite spacing uses default precision.
        step: f64,
        /// Typically the label with the largest absolute magnitude.
        reference_value: f64,
    },
}

/// Input to preparation. Specifier syntax and named options belong to the provider.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NumberFormatRequest {
    /// Provider-specific specifier; `None` selects its default.
    pub spec: Option<String>,
    pub options: NumberFormatOptions,
    pub context: NumberFormatContext,
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
    pub fn prepare(
        &self,
        config: &NumberFormatConfig,
        request: &NumberFormatRequest,
    ) -> Result<Arc<dyn PreparedNumberFormatter>, NumberFormatError> {
        self.providers
            .get(&config.provider)
            .ok_or_else(|| {
                NumberFormatError(format!(
                    "number format provider `{}` was not found",
                    config.provider
                ))
            })?
            .prepare(config, request)
    }
}
