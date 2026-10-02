use crate::{
    adapters::apply_float_precision, prepare_number_step_format, NumberLocaleSpec,
    PreparedNumberFormat, ResolvedNumberLocale,
};
use avenger_format::{
    FormattedNumber, NumberFormatError, NumberFormatProvider, PreparedNumberFormatter,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

/// Prepare D3 number patterns with locale definitions and a precision policy.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct D3NumberFormatProvider {
    /// Locale name. An omitted name selects `en-US`.
    pub locale: Option<String>,
    /// Custom D3 definitions, keyed by locale name.
    pub locales: BTreeMap<String, NumberLocaleSpec>,
    /// Precision selection when the pattern omits an explicit precision.
    pub precision: D3NumberPrecision,
}

impl D3NumberFormatProvider {
    /// Use U.S. English and the pattern's ordinary D3 precision.
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
        definition: NumberLocaleSpec,
    ) -> Self {
        self.locales.insert(name.into(), definition);
        self
    }

    /// Set precision selection for patterns that omit precision.
    pub fn with_precision(mut self, precision: D3NumberPrecision) -> Self {
        self.precision = precision;
        self
    }
}

/// D3 precision selection. Explicit precision in the pattern takes precedence.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum D3NumberPrecision {
    /// Use the pattern's precision or its ordinary D3 default.
    #[default]
    FromSpecifier,
    /// Use Vega's automatic precision and trimming.
    Automatic,
    /// Infer precision and a shared SI unit from numeric spacing and magnitude.
    Step {
        /// Finite spacing between values. Its magnitude determines precision.
        step: f64,
        /// Finite reference magnitude, typically the largest absolute value.
        reference_value: f64,
    },
}

impl NumberFormatProvider for D3NumberFormatProvider {
    fn prepare(
        &self,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedNumberFormatter>, NumberFormatError> {
        let id = self.locale.as_deref().unwrap_or("en-US");
        let normalized = id.replace('_', "-");
        let data = self.locales.get(id).or_else(|| {
            self.locales
                .iter()
                .find_map(|(name, data)| (name.replace('_', "-") == normalized).then_some(data))
        });
        let locale = if let Some(definition) = data {
            ResolvedNumberLocale::new(id, definition.clone())
        } else {
            crate::bundled::resolve(&normalized)
        }?;
        let prepared = match self.precision {
            D3NumberPrecision::FromSpecifier => PreparedNumberFormat::new(Some(pattern), &locale),
            D3NumberPrecision::Automatic => {
                PreparedNumberFormat::new(Some(pattern), &locale).map(|mut prepared| {
                    apply_float_precision(&mut prepared);
                    prepared
                })
            }
            D3NumberPrecision::Step {
                step,
                reference_value,
            } => {
                for (option, value) in [("step", step), ("reference_value", reference_value)] {
                    if !value.is_finite() {
                        return Err(NumberFormatError::InvalidOption {
                            option: option.into(),
                            message: "must be finite".into(),
                        });
                    }
                }
                prepare_number_step_format(step, reference_value, Some(pattern), &locale)
            }
        }?;
        Ok(Arc::new(prepared))
    }
}

impl PreparedNumberFormatter for PreparedNumberFormat {
    fn format(&self, value: f64) -> FormattedNumber {
        self.format(value)
    }
}
