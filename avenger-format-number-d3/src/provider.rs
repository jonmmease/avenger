use crate::{
    adapters::{apply_float_precision, step_format},
    prepare_number_float_format, NumberLocaleSpec, PreparedNumberFormat, ResolvedNumberLocale,
};
use avenger_format::{
    FormattedNumber, NumberFormatError, NumberFormatProvider, PreparedNumberFormatter, TickSpacing,
    TickStep,
};
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, collections::BTreeMap, sync::Arc};

/// Prepare D3 number patterns with locale definitions and a precision policy.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct D3NumberFormatProvider {
    /// Locale name. An omitted name selects `en-US`.
    pub locale: Option<String>,
    /// Custom D3 definitions, keyed by locale name.
    pub locales: BTreeMap<String, NumberLocaleSpec>,
    /// Single-value precision when the pattern omits it. Tick labels follow Vega's axis rules.
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

    /// Set single-value precision for patterns that omit it.
    pub fn with_precision(mut self, precision: D3NumberPrecision) -> Self {
        self.precision = precision;
        self
    }
}

/// Single-value precision when a pattern omits it. Explicit precision in the pattern takes
/// precedence. Tick formatting follows Vega's axis rules in either mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum D3NumberPrecision {
    /// Use the pattern's precision or its ordinary D3 default.
    #[default]
    FromSpecifier,
    /// Use Vega's automatic precision and trimming.
    Automatic,
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
        let prepared = PreparedNumberFormat::new(Some(pattern), &locale)?;
        let mut single = prepared.clone();
        if self.precision == D3NumberPrecision::Automatic {
            apply_float_precision(&mut single);
        }
        Ok(Arc::new(PreparedPattern {
            varying: prepare_number_float_format(Some(pattern), &locale)?,
            pattern: prepared,
            single,
        }))
    }
}

/// A pattern prepared by [`D3NumberFormatProvider`].
#[derive(Debug)]
struct PreparedPattern {
    /// The pattern as written, with omitted precision left automatic.
    pattern: PreparedNumberFormat,
    /// Single values, after the provider's precision policy.
    single: PreparedNumberFormat,
    /// Ticks spanning magnitudes, with Vega's `formatFloat` rules for log axes.
    varying: PreparedNumberFormat,
}

impl PreparedNumberFormatter for PreparedPattern {
    fn format(&self, value: f64) -> FormattedNumber {
        self.single.format(value)
    }

    /// Uniform ticks follow Vega's `formatSpan`, with the step and magnitude inferred from
    /// the values rather than read from a domain.
    fn format_ticks(&self, values: &[f64], spacing: TickSpacing) -> Vec<FormattedNumber> {
        let format = match spacing {
            TickSpacing::Uniform => match TickStep::infer(values) {
                Some(ticks) => Cow::Owned(step_format(&self.pattern, ticks.step, ticks.magnitude)),
                None => Cow::Borrowed(&self.pattern),
            },
            TickSpacing::Varying => Cow::Borrowed(&self.varying),
        };
        values.iter().map(|&value| format.format(value)).collect()
    }
}
