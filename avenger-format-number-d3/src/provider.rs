use crate::{
    adapters::prepare_number_float_format_with_overrides, prepare_number_step_format, Align,
    DigitSpec, FormatType, NumberFormatOverrides, NumberLocaleSpec, PreparedNumberFormat,
    ResolvedNumberLocale, SignPolicy, Symbol,
};
use avenger_format::{
    FormattedNumber, NumberFormatConfig, NumberFormatContext, NumberFormatError,
    NumberFormatOptions, NumberFormatProvider, NumberFormatRequest, PreparedNumberFormatter,
};
use serde_json::Value;
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
        let locale = if let Some(data) = config.locales.get(id) {
            let spec: NumberLocaleSpec = serde_json::from_value(data.clone())
                .map_err(|err| NumberFormatError(format!("invalid D3 locale `{id}`: {err}")))?;
            ResolvedNumberLocale::new(id, spec)
        } else if id == "en-US" {
            Ok(ResolvedNumberLocale::en_us())
        } else {
            Err(crate::FormatError::LocaleNotFound(id.into()))
        }
        .map_err(|err| NumberFormatError(err.to_string()))?;
        let overrides = parse_options(&request.options)?;
        let spec = request.spec.as_deref();
        let prepared = match request.context {
            NumberFormatContext::Scalar => PreparedNumberFormat::new(spec, overrides, &locale),
            NumberFormatContext::Continuous => {
                prepare_number_float_format_with_overrides(spec, overrides, &locale)
            }
            NumberFormatContext::Discrete => PreparedNumberFormat::new(
                Some(spec.filter(|s| !s.is_empty()).unwrap_or("c")),
                overrides,
                &locale,
            ),
            NumberFormatContext::Step {
                step,
                reference_value,
            } => prepare_number_step_format(step, reference_value, spec, overrides, &locale),
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

fn invalid_option(name: &str) -> NumberFormatError {
    NumberFormatError(format!("invalid D3 number format option `{name}`"))
}

fn character(value: &Value, name: &str) -> Result<char, NumberFormatError> {
    let mut chars = value.as_str().ok_or_else(|| invalid_option(name))?.chars();
    let ch = chars.next().ok_or_else(|| invalid_option(name))?;
    if chars.next().is_some() {
        return Err(invalid_option(name));
    }
    Ok(ch)
}

/// Named arguments share the specifier's field semantics. Null clears optional
/// padding fields and restores automatic precision for `precision`.
fn parse_options(
    options: &NumberFormatOptions,
) -> Result<NumberFormatOverrides, NumberFormatError> {
    let mut result = NumberFormatOverrides::default();
    for (name, value) in options {
        match name.as_str() {
            "type" | "style" => {
                if result.format_type.is_some() {
                    return Err(NumberFormatError(
                        "D3 number format accepts only one type option".into(),
                    ));
                }
                result.format_type = Some(
                    FormatType::from_char(character(value, name)?)
                        .ok_or_else(|| invalid_option(name))?,
                );
            }
            "precision" => {
                result.digit_spec = Some(if value.is_null() {
                    DigitSpec::Auto
                } else {
                    DigitSpec::Precision(
                        value
                            .as_u64()
                            .and_then(|v| u8::try_from(v).ok())
                            .ok_or_else(|| invalid_option(name))?,
                    )
                });
            }
            "group" => result.group = Some(value.as_bool().ok_or_else(|| invalid_option(name))?),
            "trim" => result.trim = Some(value.as_bool().ok_or_else(|| invalid_option(name))?),
            "zero" => result.zero = Some(value.as_bool().ok_or_else(|| invalid_option(name))?),
            "sign" => {
                result.sign = Some(
                    SignPolicy::from_char(character(value, name)?)
                        .ok_or_else(|| invalid_option(name))?,
                )
            }
            "symbol" => {
                result.symbol = Some(match value.as_str() {
                    Some("$") => Some(Symbol::CurrencyCompat),
                    Some("#") => Some(Symbol::Alternate),
                    Some("none") => None,
                    _ if value.is_null() => None,
                    _ => return Err(invalid_option(name)),
                })
            }
            "width" => {
                result.width = Some(if value.is_null() {
                    None
                } else {
                    Some(
                        value
                            .as_u64()
                            .and_then(|v| usize::try_from(v).ok())
                            .ok_or_else(|| invalid_option(name))?,
                    )
                })
            }
            "fill" => {
                result.fill = Some(if value.is_null() {
                    None
                } else {
                    Some(character(value, name)?)
                })
            }
            "align" => {
                result.align = Some(if value.is_null() {
                    None
                } else {
                    Some(
                        Align::from_char(character(value, name)?)
                            .ok_or_else(|| invalid_option(name))?,
                    )
                })
            }
            _ => {
                return Err(NumberFormatError(format!(
                    "unsupported D3 number format option `{name}`"
                )))
            }
        }
    }
    Ok(result)
}
