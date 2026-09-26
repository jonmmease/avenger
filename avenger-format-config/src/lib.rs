//! Serializable built-in formatter settings for scene and application data.
//! Generic text consumers receive bindings from `avenger-format`.

use avenger_format::{DateTimeFormatBinding, NumberFormatBinding};
pub use avenger_format_datetime_chrono::ChronoDateTimeFormatConfig;
use avenger_format_datetime_chrono::ChronoDateTimeFormatProvider;
pub use avenger_format_datetime_d3::D3DateTimeFormatConfig;
use avenger_format_datetime_d3::D3DateTimeFormatProvider;
use avenger_format_number_d3::D3NumberFormatProvider;
pub use avenger_format_number_d3::{D3NumberFormatConfig, D3NumberPrecision};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    hash::{Hash, Hasher},
    sync::{Mutex, OnceLock},
};

/// Saved number provider selection and its typed settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "kebab-case")]
pub enum NumberFormatConfig {
    D3(D3NumberFormatConfig),
}

/// Saved datetime provider selection and its typed settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "kebab-case")]
pub enum DateTimeFormatConfig {
    D3(D3DateTimeFormatConfig),
    Chrono(ChronoDateTimeFormatConfig),
}

impl NumberFormatConfig {
    /// Reuse a binding for recently resolved settings so text caches retain their identity.
    pub fn binding(&self) -> NumberFormatBinding {
        static CACHE: OnceLock<Mutex<VecDeque<(NumberFormatConfig, NumberFormatBinding)>>> =
            OnceLock::new();
        resolve(CACHE.get_or_init(Default::default), self, || match self {
            Self::D3(config) => NumberFormatBinding::new(D3NumberFormatProvider, config.clone()),
        })
    }
}

impl DateTimeFormatConfig {
    /// Reuse a binding for recently resolved settings so text caches retain their identity.
    pub fn binding(&self) -> DateTimeFormatBinding {
        static CACHE: OnceLock<Mutex<VecDeque<(DateTimeFormatConfig, DateTimeFormatBinding)>>> =
            OnceLock::new();
        resolve(CACHE.get_or_init(Default::default), self, || match self {
            Self::D3(config) => {
                DateTimeFormatBinding::new(D3DateTimeFormatProvider, config.clone())
            }
            Self::Chrono(config) => {
                DateTimeFormatBinding::new(ChronoDateTimeFormatProvider, config.clone())
            }
        })
    }
}

/// Bound retained definitions and refresh the position of an accessed binding.
fn resolve<C: PartialEq + Clone, B: Clone>(
    cache: &Mutex<VecDeque<(C, B)>>,
    config: &C,
    make: impl FnOnce() -> B,
) -> B {
    let mut entries = cache.lock().expect("formatter settings cache lock");
    let entry = if let Some(index) = entries.iter().position(|(key, _)| key == config) {
        entries.remove(index).unwrap()
    } else {
        (config.clone(), make())
    };
    let binding = entry.1.clone();
    entries.push_front(entry);
    entries.truncate(128);
    binding
}

impl Hash for NumberFormatConfig {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::D3(config) => {
                config.locale.hash(state);
                for (name, locale) in &config.locales {
                    name.hash(state);
                    locale.decimal.hash(state);
                    locale.thousands.hash(state);
                    locale.grouping.hash(state);
                    locale.currency.hash(state);
                    locale.numerals.hash(state);
                    locale.percent.hash(state);
                    locale.minus.hash(state);
                    locale.nan.hash(state);
                }
                std::mem::discriminant(&config.precision).hash(state);
                if let D3NumberPrecision::Step {
                    step,
                    reference_value,
                } = config.precision
                {
                    // Equal signed zeros must have the same hash.
                    for value in [step, reference_value] {
                        (if value == 0.0 { 0 } else { value.to_bits() }).hash(state);
                    }
                }
            }
        }
    }
}

impl Hash for DateTimeFormatConfig {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::D3(config) => {
                config.locale.hash(state);
                config.timezone.hash(state);
                for (name, locale) in &config.locales {
                    name.hash(state);
                    locale.date_time.hash(state);
                    locale.date.hash(state);
                    locale.time.hash(state);
                    locale.periods.hash(state);
                    locale.days.hash(state);
                    locale.short_days.hash(state);
                    locale.months.hash(state);
                    locale.short_months.hash(state);
                }
            }
            Self::Chrono(config) => {
                config.locale.hash(state);
                config.timezone.hash(state);
            }
        }
    }
}

impl From<D3NumberFormatConfig> for NumberFormatConfig {
    fn from(config: D3NumberFormatConfig) -> Self {
        Self::D3(config)
    }
}
impl From<D3DateTimeFormatConfig> for DateTimeFormatConfig {
    fn from(config: D3DateTimeFormatConfig) -> Self {
        Self::D3(config)
    }
}
impl From<ChronoDateTimeFormatConfig> for DateTimeFormatConfig {
    fn from(config: ChronoDateTimeFormatConfig) -> Self {
        Self::Chrono(config)
    }
}
