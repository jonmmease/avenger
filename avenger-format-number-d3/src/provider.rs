use crate::{
    adapters::apply_float_precision, prepare_number_step_format, NumberLocaleSpec,
    PreparedNumberFormat, ResolvedNumberLocale,
};
use avenger_format::{
    FormattedNumber, NumberFormatConfig, NumberFormatError, NumberFormatOptions,
    NumberFormatProvider, NumberFormatRequest, PreparedNumberFormatter,
};
use std::sync::Arc;

/// D3 specifiers and locale definitions exposed through the shared formatting interface.
#[derive(Debug, Default)]
pub struct D3NumberFormatProvider;

impl NumberFormatProvider for D3NumberFormatProvider {
    fn prepare(
        &self,
        config: &NumberFormatConfig,
        request: &NumberFormatRequest,
    ) -> Result<Arc<dyn PreparedNumberFormatter>, NumberFormatError> {
        let id = config.locale.as_deref().unwrap_or("en-US");
        let normalized = id.replace('_', "-");
        let data = config.locales.get(id).or_else(|| {
            config
                .locales
                .iter()
                .find_map(|(name, data)| (name.replace('_', "-") == normalized).then_some(data))
        });
        let locale = if let Some(data) = data {
            let spec: NumberLocaleSpec = serde_json::from_value(data.clone())
                .map_err(|err| NumberFormatError(format!("invalid D3 locale `{id}`: {err}")))?;
            ResolvedNumberLocale::new(id, spec)
        } else if normalized == "en-US" {
            Ok(ResolvedNumberLocale::en_us())
        } else {
            Err(crate::FormatError::LocaleNotFound(id.into()))
        }
        .map_err(|err| NumberFormatError(err.to_string()))?;
        let mut options = request.options.clone();
        let auto_precision = options
            .remove("auto_precision")
            .map(|value| {
                value
                    .as_bool()
                    .ok_or_else(|| invalid_option("auto_precision"))
            })
            .transpose()?
            .unwrap_or(false);
        let step = numeric_option(&mut options, "step")?;
        let reference = numeric_option(&mut options, "reference_value")?;
        if let Some(name) = options.keys().next() {
            return Err(NumberFormatError(format!(
                "unsupported D3 number format option `{name}`"
            )));
        }
        let spec = Some(request.spec.as_str());
        let prepared = match (step, reference, auto_precision) {
            (Some(step), Some(reference), false) =>
                prepare_number_step_format(step, reference, spec, &locale),
            (None, None, true) => PreparedNumberFormat::new(spec, &locale).map(|mut prepared| {
                apply_float_precision(&mut prepared);
                prepared
            }),
            (None, None, false) => PreparedNumberFormat::new(spec, &locale),
            _ => return Err(NumberFormatError(
                "D3 step formatting requires both `step` and `reference_value`, without `auto_precision`".into()
            )),
        }
        .map_err(|err| NumberFormatError(err.to_string()))?;
        Ok(Arc::new(prepared))
    }
}

impl PreparedNumberFormatter for PreparedNumberFormat {
    fn format(&self, value: f64) -> FormattedNumber {
        self.format(value)
    }
}

fn numeric_option(
    options: &mut NumberFormatOptions,
    name: &str,
) -> Result<Option<f64>, NumberFormatError> {
    options
        .remove(name)
        .map(|value| {
            value
                .as_f64()
                .filter(|v| v.is_finite())
                .ok_or_else(|| invalid_option(name))
        })
        .transpose()
}

fn invalid_option(name: &str) -> NumberFormatError {
    NumberFormatError(format!("invalid D3 number format option `{name}`"))
}
