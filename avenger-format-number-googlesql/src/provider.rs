//! The number format provider of GoogleSQL's `FORMAT` patterns, for the labels of charts.

use std::collections::BTreeMap;
use std::sync::Arc;

use avenger_format::{
    FormattedNumber, NumberFormatError, NumberFormatProvider, NumberTypesetting, PrecisionSharing,
    PreparedNumberFormatter, TickStep,
};
use avenger_format_locales_d3::NumberLocaleSpec;
use serde::{Deserialize, Serialize};

use crate::format::{fixed, general};
use crate::{Format, Locale};

/// Magnitudes at and above this print in scientific notation without a pattern.
const LARGE: f64 = 1e21;

/// Nonzero magnitudes below this print in scientific notation without a pattern.
const SMALL: f64 = 1e-6;

/// Prepares GoogleSQL's `FORMAT` patterns, with the syntax name `googlesql`, with GoogleSQL's
/// symbols or those of a d3 number locale.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GoogleSqlNumberFormatProvider {
    /// Locale name. An omitted name selects GoogleSQL's symbols, which differ from those of the
    /// d3 locale `en-US` in the minus sign and NaN.
    pub locale: Option<String>,
    /// Custom d3 definitions, keyed by locale name.
    pub locales: BTreeMap<String, NumberLocaleSpec>,
}

impl GoogleSqlNumberFormatProvider {
    /// Use GoogleSQL's symbols.
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

    /// The selected locale: a custom definition, by its name with hyphens or underscores, or a
    /// bundled one.
    fn resolve_locale(&self) -> Result<Locale, NumberFormatError> {
        let Some(id) = self.locale.as_deref() else {
            return Ok(Locale::default());
        };
        let normalized = id.replace('_', "-");
        let custom = self.locales.get(id).or_else(|| {
            (self.locales.iter())
                .find_map(|(name, spec)| (name.replace('_', "-") == normalized).then_some(spec))
        });
        let locale = match custom {
            Some(spec) => Locale::new(spec),
            None => Locale::bundled(&normalized),
        };
        locale.map_err(|error| match error {
            NumberFormatError::InvalidLocaleData { message } => {
                NumberFormatError::InvalidLocaleData {
                    message: format!("locale `{id}`: {message}"),
                }
            }
            other => other,
        })
    }
}

impl NumberFormatProvider for GoogleSqlNumberFormatProvider {
    fn syntax(&self) -> &str {
        "googlesql"
    }

    /// Prepares a whole `FORMAT` pattern with exactly one numeric conversion, or the empty
    /// pattern, which stands for no pattern, with the symbols of the selected locale.
    fn prepare(
        &self,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedNumberFormatter>, NumberFormatError> {
        let locale = self.resolve_locale()?;
        Ok(match pattern.is_empty() {
            true => Arc::new(Prepared::Default(locale)),
            false => Arc::new(Prepared::Written(Format::new(pattern, &locale)?)),
        })
    }
}

/// A pattern prepared by [`GoogleSqlNumberFormatProvider`].
#[derive(Clone, Debug)]
enum Prepared {
    /// A written pattern, which formats as written in every mode.
    Written(Format),
    /// The empty pattern: the shortest decimals of each value, or the decimals of ticks' step.
    Default(Locale),
}

/// Whether a magnitude is too large or too small for decimals without a pattern.
fn is_extreme(magnitude: f64) -> bool {
    magnitude >= LARGE || (magnitude > 0.0 && magnitude < SMALL)
}

/// A value without a pattern: grouped, with the fewest decimals that read back as the value, or
/// as `%g` in scientific notation when its magnitude is extreme.
fn format_default(value: f64, locale: &Locale) -> FormattedNumber {
    let sign = if value < 0.0 {
        locale.minus.as_str()
    } else {
        ""
    };
    if value.is_nan() {
        return FormattedNumber::plain(locale.nan.as_deref().unwrap_or("nan"));
    }
    if value.is_infinite() {
        return FormattedNumber::plain(format!("{sign}inf"));
    }
    let magnitude = value.abs();
    if !is_extreme(magnitude) {
        // Rust's display of an `f64` is the shortest decimal that reads back as the value.
        let text = locale.decimal_text(&magnitude.to_string(), true);
        return FormattedNumber::plain(format!("{sign}{text}"));
    }
    let (scientific, _) = general(magnitude, 6, false, false);
    let (mantissa, exponent) = scientific.split_once('e').expect("scientific notation");
    let mantissa = format!("{sign}{}", locale.decimal_text(mantissa, false));
    FormattedNumber {
        text: format!("{sign}{}", locale.scientific_text(&scientific)),
        typesetting: NumberTypesetting::Exponent {
            mantissa,
            exponent: exponent.parse().expect("an integer exponent"),
        },
    }
}

/// A shared tick without a pattern: grouped with `decimals` decimals. A tick that rounds to
/// zero, such as -1e-17 for 0, prints as zero.
fn format_tick(value: f64, decimals: usize, locale: &Locale) -> FormattedNumber {
    if !value.is_finite() {
        return format_default(value, locale);
    }
    let text = fixed(value.abs(), decimals, false);
    let zero = !text.bytes().any(|byte| (b'1'..=b'9').contains(&byte));
    let sign = if value < 0.0 && !zero {
        locale.minus.as_str()
    } else {
        ""
    };
    FormattedNumber::plain(format!("{sign}{}", locale.decimal_text(&text, true)))
}

impl PreparedNumberFormatter for Prepared {
    fn format(&self, value: f64) -> FormattedNumber {
        match self {
            Prepared::Written(format) => FormattedNumber::plain(format.format(value)),
            Prepared::Default(locale) => format_default(value, locale),
        }
    }

    /// A written pattern formats each value as written. Without a pattern, shared ticks print
    /// grouped with the decimals of their step, as `%'.Nf`, and values per value as `format`
    /// prints them. Ticks whose largest magnitude is extreme print per value, and a tick that
    /// rounds to zero prints without a sign.
    fn format_all(&self, values: &[f64], sharing: PrecisionSharing) -> Vec<FormattedNumber> {
        let shared = match (self, sharing) {
            (Prepared::Default(locale), PrecisionSharing::Shared) => TickStep::infer(values)
                .filter(|ticks| !is_extreme(ticks.magnitude))
                .map(|ticks| ((-ticks.resolution).max(0) as usize, locale)),
            _ => None,
        };
        match shared {
            Some((decimals, locale)) => (values.iter())
                .map(|&value| format_tick(value, decimals, locale))
                .collect(),
            None => values.iter().map(|&value| self.format(value)).collect(),
        }
    }
}
