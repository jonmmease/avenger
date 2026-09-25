use crate::{
    parse_datetime_timezone, DateTimeFormatContext as D3Context, DateTimeFormatOverrides,
    DateTimeLocaleSpec, PreparedDateTimeFormat, PreparedTimeMultiFormat, ResolvedDateTimeLocale,
};
use avenger_format::{
    DateTimeFormatConfig, DateTimeFormatContext, DateTimeFormatError, DateTimeFormatProvider,
    DateTimeFormatRequest, NaiveDateTimeInput, PreparedDateTimeFormatter, ZonedDateTimeInput,
};
use std::sync::Arc;

/// D3 patterns, locale definitions, and Vega automatic labels through the shared interface.
#[derive(Debug, Default)]
pub struct D3DateTimeFormatProvider;
impl DateTimeFormatProvider for D3DateTimeFormatProvider {
    fn prepare(
        &self,
        config: &DateTimeFormatConfig,
        request: &DateTimeFormatRequest,
    ) -> Result<Arc<dyn PreparedDateTimeFormatter>, DateTimeFormatError> {
        if let Some(name) = request.options.keys().next() {
            return Err(DateTimeFormatError(format!(
                "unsupported D3 datetime format option `{name}`"
            )));
        }
        let id = config.locale.as_deref().unwrap_or("en-US");
        let locale = if let Some(data) = config.locales.get(id) {
            let definition: DateTimeLocaleSpec = serde_json::from_value(data.clone())
                .map_err(|err| DateTimeFormatError(format!("invalid D3 locale `{id}`: {err}")))?;
            ResolvedDateTimeLocale::new(id, definition).map_err(error)?
        } else if id == "en-US" {
            ResolvedDateTimeLocale::en_us()
        } else {
            return Err(DateTimeFormatError(format!("locale `{id}` was not found")));
        };
        let timezone =
            parse_datetime_timezone(config.timezone.as_deref().unwrap_or("UTC")).map_err(error)?;
        let context = D3Context::new(&locale, timezone);
        if request.spec.is_none() && request.context == DateTimeFormatContext::Data {
            let prepare = |spec| {
                PreparedDateTimeFormat::new(
                    Some(spec),
                    DateTimeFormatOverrides {
                        timezone: request.timezone.clone(),
                    },
                    context,
                )
                .map_err(error)
            };
            return Ok(Arc::new(PreparedData {
                date: prepare("%Y-%m-%d")?,
                datetime: prepare("%Y-%m-%d %H:%M:%S")?,
                instant: prepare("%Y-%m-%d %H:%M:%S %Z")?,
            }));
        }
        let multi_spec = match &request.spec {
            None if request.context == DateTimeFormatContext::Tick => Some(Default::default()),
            Some(value) if value.is_object() => {
                Some(serde_json::from_value(value.clone()).map_err(|err| {
                    DateTimeFormatError(format!("invalid D3 time multi-format: {err}"))
                })?)
            }
            None | Some(serde_json::Value::String(_)) => None,
            Some(_) => {
                return Err(DateTimeFormatError(
                    "D3 datetime specifier must be a string or object".into(),
                ))
            }
        };
        if let Some(spec) = multi_spec {
            // The per-call override remains explicit so civil preflight rejects it.
            let timezone = request
                .timezone
                .as_deref()
                .map(parse_datetime_timezone)
                .transpose()
                .map_err(error)?
                .unwrap_or(timezone);
            let prepared = PreparedTimeMultiFormat::new(&spec, D3Context::new(&locale, timezone))
                .map_err(error)?;
            Ok(Arc::new(PreparedMulti {
                prepared,
                timezone_override: request.timezone.is_some(),
            }))
        } else {
            Ok(Arc::new(
                PreparedDateTimeFormat::new(
                    request.spec.as_ref().and_then(serde_json::Value::as_str),
                    DateTimeFormatOverrides {
                        timezone: request.timezone.clone(),
                    },
                    context,
                )
                .map_err(error)?,
            ))
        }
    }
}

fn error(error: crate::DateTimeFormatError) -> DateTimeFormatError {
    DateTimeFormatError(error.to_string())
}
impl PreparedDateTimeFormatter for PreparedDateTimeFormat {
    fn validate_naive(&self) -> Result<(), DateTimeFormatError> {
        self.validate_naive().map_err(error)
    }
    fn format_naive(&self, value: NaiveDateTimeInput) -> Result<String, DateTimeFormatError> {
        self.format_naive(value).map_err(error)
    }
    fn format_zoned(&self, value: ZonedDateTimeInput) -> Result<String, DateTimeFormatError> {
        self.format_zoned(value).map_err(error)
    }
}

#[derive(Debug)]
struct PreparedMulti {
    prepared: PreparedTimeMultiFormat,
    timezone_override: bool,
}
impl PreparedDateTimeFormatter for PreparedMulti {
    fn validate_naive(&self) -> Result<(), DateTimeFormatError> {
        if self.timezone_override {
            return Err(error(crate::DateTimeFormatError::TimezoneOverrideForNaive));
        }
        self.prepared.validate_naive().map_err(error)
    }
    fn format_naive(&self, value: NaiveDateTimeInput) -> Result<String, DateTimeFormatError> {
        if self.timezone_override {
            return Err(error(crate::DateTimeFormatError::TimezoneOverrideForNaive));
        }
        self.prepared.format_naive(value).map_err(error)
    }
    fn format_zoned(&self, value: ZonedDateTimeInput) -> Result<String, DateTimeFormatError> {
        self.prepared.format_zoned(value).map_err(error)
    }
}

/// Data labels retain enough calendar fields to distinguish individual values.
#[derive(Debug)]
struct PreparedData {
    date: PreparedDateTimeFormat,
    datetime: PreparedDateTimeFormat,
    instant: PreparedDateTimeFormat,
}
impl PreparedDateTimeFormatter for PreparedData {
    fn validate_naive(&self) -> Result<(), DateTimeFormatError> {
        self.datetime.validate_naive().map_err(error)
    }
    fn format_naive(&self, value: NaiveDateTimeInput) -> Result<String, DateTimeFormatError> {
        match value {
            NaiveDateTimeInput::Date(_) => self.date.format_naive(value),
            NaiveDateTimeInput::DateTime(_) => self.datetime.format_naive(value),
        }
        .map_err(error)
    }
    fn format_zoned(&self, value: ZonedDateTimeInput) -> Result<String, DateTimeFormatError> {
        self.instant.format_zoned(value).map_err(error)
    }
}
