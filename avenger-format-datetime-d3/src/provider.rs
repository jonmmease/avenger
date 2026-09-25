use crate::{
    parse_datetime_timezone, DateTimeFormatContext as D3Context, DateTimeFormatOverrides,
    DateTimeLocaleSpec, PreparedDateTimeFormat, PreparedTimeMultiFormat, ResolvedDateTimeLocale,
};
use avenger_format::{
    DateTimeFormatConfig, DateTimeFormatError, DateTimeFormatProvider, DateTimeFormatRequest,
    NaiveDateTimeInput, PreparedCivilDateTimeFormatter, PreparedInstantFormatter,
    ZonedDateTimeInput,
};
use std::sync::Arc;

/// D3 patterns, locale definitions, and Vega automatic labels through the shared interface.
#[derive(Debug, Default)]
pub struct D3DateTimeFormatProvider;
impl DateTimeFormatProvider for D3DateTimeFormatProvider {
    fn prepare_naive(
        &self,
        config: &DateTimeFormatConfig,
        request: &DateTimeFormatRequest,
    ) -> Result<Arc<dyn PreparedCivilDateTimeFormatter>, DateTimeFormatError> {
        if request.timezone.is_some() {
            return Err(error(crate::DateTimeFormatError::TimezoneOverrideForNaive));
        }
        let prepared = PreparedFormat::new(config, request)?;
        match &prepared {
            PreparedFormat::Scalar(format) => format.validate_naive(),
            PreparedFormat::Multi(format) => format.validate_naive(),
        }
        .map_err(error)?;
        Ok(Arc::new(prepared))
    }

    fn prepare_zoned(
        &self,
        config: &DateTimeFormatConfig,
        request: &DateTimeFormatRequest,
    ) -> Result<Arc<dyn PreparedInstantFormatter>, DateTimeFormatError> {
        Ok(Arc::new(PreparedFormat::new(config, request)?))
    }
}

#[derive(Debug)]
enum PreparedFormat {
    Scalar(PreparedDateTimeFormat),
    Multi(PreparedTimeMultiFormat),
}
impl PreparedFormat {
    fn new(
        config: &DateTimeFormatConfig,
        request: &DateTimeFormatRequest,
    ) -> Result<Self, DateTimeFormatError> {
        if let Some(name) = request.options.keys().next() {
            return Err(DateTimeFormatError(format!(
                "unsupported D3 datetime format option `{name}`"
            )));
        }
        let id = config.locale.as_deref().unwrap_or("en-US");
        let normalized = id.replace('_', "-");
        let data = config.locales.get(id).or_else(|| {
            config
                .locales
                .iter()
                .find_map(|(name, data)| (name.replace('_', "-") == normalized).then_some(data))
        });
        let locale = if let Some(data) = data {
            let definition: DateTimeLocaleSpec = serde_json::from_value(data.clone())
                .map_err(|err| DateTimeFormatError(format!("invalid D3 locale `{id}`: {err}")))?;
            ResolvedDateTimeLocale::new(id, definition).map_err(error)?
        } else if normalized == "en-US" {
            ResolvedDateTimeLocale::en_us()
        } else {
            return Err(DateTimeFormatError(format!("locale `{id}` was not found")));
        };
        let timezone =
            parse_datetime_timezone(config.timezone.as_deref().unwrap_or("UTC")).map_err(error)?;
        let context = D3Context::new(&locale, timezone);
        let multi_spec = match &request.spec {
            serde_json::Value::Object(_) => {
                Some(serde_json::from_value(request.spec.clone()).map_err(|err| {
                    DateTimeFormatError(format!("invalid D3 time multi-format: {err}"))
                })?)
            }
            serde_json::Value::String(_) => None,
            _ => {
                return Err(DateTimeFormatError(
                    "D3 datetime specifier must be a string or object".into(),
                ))
            }
        };
        if let Some(spec) = multi_spec {
            let timezone = request
                .timezone
                .as_deref()
                .map(parse_datetime_timezone)
                .transpose()
                .map_err(error)?
                .unwrap_or(timezone);
            let prepared = PreparedTimeMultiFormat::new(&spec, D3Context::new(&locale, timezone))
                .map_err(error)?;
            Ok(Self::Multi(prepared))
        } else {
            Ok(Self::Scalar(
                PreparedDateTimeFormat::new(
                    request.spec.as_str(),
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
impl PreparedCivilDateTimeFormatter for PreparedFormat {
    fn format(&self, value: NaiveDateTimeInput) -> Result<String, DateTimeFormatError> {
        match self {
            Self::Scalar(format) => format.format_naive(value),
            Self::Multi(format) => format.format_naive(value),
        }
        .map_err(error)
    }
}
impl PreparedInstantFormatter for PreparedFormat {
    fn format(&self, value: ZonedDateTimeInput) -> Result<String, DateTimeFormatError> {
        match self {
            Self::Scalar(format) => format.format_zoned(value),
            Self::Multi(format) => format.format_zoned(value),
        }
        .map_err(error)
    }
}
