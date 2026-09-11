use std::{collections::BTreeMap, sync::Arc, sync::OnceLock};

use avenger_format_datetime::{DateTimeLocaleRegistry, DateTimeLocaleSpec};
use avenger_format_number::{NumberLocaleRegistry, NumberLocaleSpec};
use avenger_typst_label::{LabelLimits, MathStyle};

use crate::types::TextSyntaxMode;

pub(crate) const DEFAULT_MARKUP_LINE_LEADING_FACTOR: f32 = 0.65;

pub type NumberLocaleSpecs = BTreeMap<String, NumberLocaleSpec>;
pub type DateTimeLocaleSpecs = BTreeMap<String, DateTimeLocaleSpec>;

#[derive(Debug, Clone, PartialEq)]
pub struct TextMarkupConfig {
    pub math_style: MathStyle,
    pub syntax_mode: TextSyntaxMode,
    pub limits: LabelLimits,
}

impl Default for TextMarkupConfig {
    fn default() -> Self {
        Self {
            math_style: MathStyle::default(),
            syntax_mode: TextSyntaxMode::Plain,
            limits: LabelLimits::default(),
        }
    }
}

impl TextMarkupConfig {
    pub(crate) fn with_syntax_mode(&self, mode: TextSyntaxMode) -> Self {
        let mut config = self.clone();
        config.syntax_mode = mode;
        config
    }

    pub(crate) fn plain_text(&self) -> Self {
        self.with_syntax_mode(TextSyntaxMode::Plain)
    }
}

pub fn empty_label_params() -> &'static avenger_typst_label::LabelParams {
    static EMPTY: OnceLock<avenger_typst_label::LabelParams> = OnceLock::new();
    EMPTY.get_or_init(avenger_typst_label::LabelParams::default)
}

pub fn label_params_fingerprint(params: &avenger_typst_label::LabelParams) -> String {
    fn value_fingerprint(value: &avenger_typst_label::LabelParamValue, out: &mut String) {
        match value {
            avenger_typst_label::LabelParamValue::None => out.push_str("none"),
            avenger_typst_label::LabelParamValue::Bool(value) => {
                out.push_str("bool:");
                out.push_str(if *value { "true" } else { "false" });
            }
            avenger_typst_label::LabelParamValue::Int(value) => {
                out.push_str("int:");
                out.push_str(&value.to_string());
            }
            avenger_typst_label::LabelParamValue::Float(value) => {
                out.push_str("float:");
                out.push_str(&value.to_bits().to_string());
            }
            avenger_typst_label::LabelParamValue::Str(value) => {
                out.push_str("str:");
                out.push_str(&value.len().to_string());
                out.push(':');
                out.push_str(value);
            }
            avenger_typst_label::LabelParamValue::Date(value) => {
                out.push_str("date:");
                out.push_str(&value.to_string());
            }
            avenger_typst_label::LabelParamValue::DateTime(value) => {
                out.push_str("datetime:");
                out.push_str(
                    &value
                        .and_utc()
                        .timestamp_nanos_opt()
                        .unwrap_or_default()
                        .to_string(),
                );
            }
            avenger_typst_label::LabelParamValue::UtcDateTime(value) => {
                out.push_str("utc-datetime:");
                out.push_str(&value.timestamp_nanos_opt().unwrap_or_default().to_string());
            }
            avenger_typst_label::LabelParamValue::Array(values) => {
                out.push_str("array:[");
                for value in values {
                    value_fingerprint(value, out);
                    out.push(',');
                }
                out.push(']');
            }
            avenger_typst_label::LabelParamValue::Dict(values) => {
                out.push_str("dict:{");
                for (key, value) in values {
                    out.push_str(&key.len().to_string());
                    out.push(':');
                    out.push_str(key);
                    out.push('=');
                    value_fingerprint(value, out);
                    out.push(',');
                }
                out.push('}');
            }
        }
    }

    let mut out = String::new();
    for (key, value) in params {
        out.push_str(&key.len().to_string());
        out.push(':');
        out.push_str(key);
        out.push('=');
        value_fingerprint(value, &mut out);
        out.push(';');
    }
    out
}

pub fn number_locale_specs_fingerprint(specs: &NumberLocaleSpecs) -> String {
    if specs.is_empty() {
        String::new()
    } else {
        serde_json::to_string(specs).unwrap_or_else(|_| format!("{specs:?}"))
    }
}

pub fn number_locale_registry_from_specs(
    specs: Option<&NumberLocaleSpecs>,
) -> Result<Option<Arc<NumberLocaleRegistry>>, String> {
    let Some(specs) = specs else {
        return Ok(None);
    };
    if specs.is_empty() {
        return Ok(None);
    }

    let mut registry = NumberLocaleRegistry::with_builtins();
    for (id, spec) in specs {
        registry
            .register_custom_locale(id.clone(), spec.clone())
            .map_err(|err| err.to_string())?;
    }
    Ok(Some(Arc::new(registry)))
}

pub fn datetime_locale_specs_fingerprint(specs: &DateTimeLocaleSpecs) -> String {
    if specs.is_empty() {
        String::new()
    } else {
        serde_json::to_string(specs).unwrap_or_else(|_| format!("{specs:?}"))
    }
}

pub fn datetime_locale_registry_from_specs(
    specs: Option<&DateTimeLocaleSpecs>,
) -> Result<Option<Arc<DateTimeLocaleRegistry>>, String> {
    let Some(specs) = specs else {
        return Ok(None);
    };
    if specs.is_empty() {
        return Ok(None);
    }

    let mut registry = DateTimeLocaleRegistry::with_builtins();
    for (id, spec) in specs {
        registry
            .register_custom_locale(id.clone(), spec.clone())
            .map_err(|err| err.to_string())?;
    }
    Ok(Some(Arc::new(registry)))
}
