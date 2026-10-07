//! Serializable built-in formatter settings for scene and application data.
//! Generic text consumers receive the selected provider as an `Arc<dyn …>` from `provider()`.

use avenger_format::{DateTimeFormatProvider, NumberFormatProvider};
pub use avenger_format_datetime_chrono::ChronoDateTimeFormatProvider;
pub use avenger_format_datetime_d3::D3DateTimeFormatProvider;
#[cfg(feature = "icu")]
pub use avenger_format_datetime_icu::{
    IcuPatternDateTimeFormatProvider, IcuSemanticDateTimeFormatProvider,
};
pub use avenger_format_number_d3::{D3NumberFormatProvider, D3NumberPrecision};
#[cfg(feature = "icu")]
pub use avenger_format_number_icu::IcuNumberFormatProvider;
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock},
};

/// Recently resolved settings and their providers, most recent first.
type Interned<C, P> = Mutex<VecDeque<(C, Arc<P>)>>;

/// Saved number provider selection and its settings.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "kebab-case")]
pub enum NumberFormatConfig {
    D3(D3NumberFormatProvider),
    #[cfg(feature = "icu")]
    Icu(IcuNumberFormatProvider),
}

/// Saved datetime provider selection and its settings.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "kebab-case")]
pub enum DateTimeFormatConfig {
    D3(D3DateTimeFormatProvider),
    Chrono(ChronoDateTimeFormatProvider),
    #[cfg(feature = "icu")]
    IcuPattern(IcuPatternDateTimeFormatProvider),
    #[cfg(feature = "icu")]
    IcuSemantic(IcuSemanticDateTimeFormatProvider),
}

impl NumberFormatConfig {
    /// The selected provider. Recently resolved settings return the same `Arc`, so text caches
    /// that match providers by identity keep their entries when scenes are rebuilt.
    pub fn provider(&self) -> Arc<dyn NumberFormatProvider> {
        static CACHE: OnceLock<Interned<NumberFormatConfig, dyn NumberFormatProvider>> =
            OnceLock::new();
        resolve(CACHE.get_or_init(Default::default), self, || match self {
            Self::D3(provider) => Arc::new(provider.clone()),
            #[cfg(feature = "icu")]
            Self::Icu(provider) => Arc::new(provider.clone()),
        })
    }
}

impl DateTimeFormatConfig {
    /// The selected provider. Recently resolved settings return the same `Arc`, so text caches
    /// that match providers by identity keep their entries when scenes are rebuilt.
    pub fn provider(&self) -> Arc<dyn DateTimeFormatProvider> {
        static CACHE: OnceLock<Interned<DateTimeFormatConfig, dyn DateTimeFormatProvider>> =
            OnceLock::new();
        resolve(CACHE.get_or_init(Default::default), self, || match self {
            Self::D3(provider) => Arc::new(provider.clone()),
            Self::Chrono(provider) => Arc::new(provider.clone()),
            #[cfg(feature = "icu")]
            Self::IcuPattern(provider) => Arc::new(provider.clone()),
            #[cfg(feature = "icu")]
            Self::IcuSemantic(provider) => Arc::new(provider.clone()),
        })
    }
}

/// Bound retained definitions and refresh the position of an accessed provider.
fn resolve<C: PartialEq + Clone, P: ?Sized>(
    cache: &Interned<C, P>,
    config: &C,
    make: impl FnOnce() -> Arc<P>,
) -> Arc<P> {
    let mut entries = cache.lock().expect("formatter settings cache lock");
    let entry = if let Some(index) = entries.iter().position(|(key, _)| key == config) {
        entries.remove(index).unwrap()
    } else {
        (config.clone(), make())
    };
    let provider = entry.1.clone();
    entries.push_front(entry);
    entries.truncate(128);
    provider
}

impl From<D3NumberFormatProvider> for NumberFormatConfig {
    fn from(provider: D3NumberFormatProvider) -> Self {
        Self::D3(provider)
    }
}
#[cfg(feature = "icu")]
impl From<IcuNumberFormatProvider> for NumberFormatConfig {
    fn from(provider: IcuNumberFormatProvider) -> Self {
        Self::Icu(provider)
    }
}
impl From<D3DateTimeFormatProvider> for DateTimeFormatConfig {
    fn from(provider: D3DateTimeFormatProvider) -> Self {
        Self::D3(provider)
    }
}
impl From<ChronoDateTimeFormatProvider> for DateTimeFormatConfig {
    fn from(provider: ChronoDateTimeFormatProvider) -> Self {
        Self::Chrono(provider)
    }
}
#[cfg(feature = "icu")]
impl From<IcuPatternDateTimeFormatProvider> for DateTimeFormatConfig {
    fn from(provider: IcuPatternDateTimeFormatProvider) -> Self {
        Self::IcuPattern(provider)
    }
}
#[cfg(feature = "icu")]
impl From<IcuSemanticDateTimeFormatProvider> for DateTimeFormatConfig {
    fn from(provider: IcuSemanticDateTimeFormatProvider) -> Self {
        Self::IcuSemantic(provider)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_settings_share_one_provider() {
        let number = NumberFormatConfig::D3(D3NumberFormatProvider::new());
        assert!(Arc::ptr_eq(&number.provider(), &number.clone().provider()));
        let datetime =
            DateTimeFormatConfig::Chrono(ChronoDateTimeFormatProvider::new().with_locale("fr-FR"));
        assert!(Arc::ptr_eq(
            &datetime.provider(),
            &datetime.clone().provider()
        ));
    }

    #[test]
    fn unknown_providers_are_errors() {
        assert!(serde_json::from_str::<NumberFormatConfig>(r#"{"provider":"unknown"}"#).is_err());
        assert!(serde_json::from_str::<DateTimeFormatConfig>(r#"{"provider":"unknown"}"#).is_err());
    }
}
