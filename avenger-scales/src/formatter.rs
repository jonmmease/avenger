use crate::error::AvengerScaleError;
use arrow::{
    array::{Array, ArrayRef, AsArray},
    compute::kernels::cast,
    datatypes::{
        DataType, Date32Type, Date64Type, Float64Type, TimeUnit, TimestampMicrosecondType,
        TimestampMillisecondType, TimestampNanosecondType, TimestampSecondType,
    },
};
use avenger_common::value::ScalarOrArray;
use avenger_format::{
    DateTimeFormatConfig, DateTimeFormatRegistry, DateTimeFormatRequest, NaiveDateTimeInput,
    NumberFormatConfig, NumberFormatOptions, NumberFormatRegistry, NumberFormatRequest,
    PreparedCivilDateTimeFormatter, PreparedInstantFormatter, PreparedNumberFormatter,
};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use chrono_tz::Tz;
use std::{fmt::Debug, sync::Arc};

/// Formatting options that prepare one shared engine for each batch.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DefaultFormatter {
    pub format_str: Option<String>,
    pub local_tz: Option<Tz>,
    pub number_format: Option<NumberFormatConfig>,
    pub number_formatters: Arc<NumberFormatRegistry>,
    pub datetime_format: Option<DateTimeFormatConfig>,
    pub datetime_formatters: Arc<DateTimeFormatRegistry>,
}
impl DefaultFormatter {
    /// Resolve number configuration before formatting any values.
    pub fn prepare_number(&self) -> Result<Arc<dyn PreparedNumberFormatter>, AvengerScaleError> {
        let config = self.number_format.as_ref().ok_or_else(|| {
            avenger_format::NumberFormatError("number formatting is not configured".into())
        })?;
        Ok(self.number_formatters.prepare(
            config,
            &match self.format_str.as_deref() {
                Some(spec) => NumberFormatRequest::new(spec),
                None => d3_continuous_number_request(None, Default::default()),
            },
        )?)
    }
    /// Prepare a civil formatter using the supplied fallback when no pattern is configured.
    pub fn prepare_naive(
        &self,
        default_spec: &str,
    ) -> Result<Arc<dyn PreparedCivilDateTimeFormatter>, AvengerScaleError> {
        Ok(self.datetime_formatters.prepare_naive(
            &self.datetime_config()?,
            &DateTimeFormatRequest::new(self.format_str.as_deref().unwrap_or(default_spec)),
        )?)
    }

    /// Prepare an instant formatter using the supplied fallback when no pattern is configured.
    pub fn prepare_zoned(
        &self,
        default_spec: &str,
    ) -> Result<Arc<dyn PreparedInstantFormatter>, AvengerScaleError> {
        Ok(self.datetime_formatters.prepare_zoned(
            &self.datetime_config()?,
            &DateTimeFormatRequest::new(self.format_str.as_deref().unwrap_or(default_spec)),
        )?)
    }

    fn datetime_config(&self) -> Result<DateTimeFormatConfig, AvengerScaleError> {
        let mut config = self.datetime_format.clone().ok_or_else(|| {
            avenger_format::DateTimeFormatError("datetime formatting is not configured".into())
        })?;
        if let Some(timezone) = self.local_tz {
            config.timezone = Some(timezone.to_string());
        }
        Ok(config)
    }
}

/// Choose D3's default format for continuous labels and request automatic precision.
pub fn d3_continuous_number_request(
    spec: Option<&str>,
    mut options: NumberFormatOptions,
) -> NumberFormatRequest {
    options
        .entry("auto_precision".into())
        .or_insert(true.into());
    NumberFormatRequest {
        spec: spec.filter(|s| !s.is_empty()).unwrap_or(",").into(),
        options,
    }
}

/// Choose D3's default tick format and supply the selected spacing and reference magnitude.
pub fn d3_step_number_request(
    spec: Option<&str>,
    mut options: NumberFormatOptions,
    step: f64,
    reference_value: f64,
) -> NumberFormatRequest {
    // A degenerate tick set retains the format's ordinary precision.
    let step = if step.is_finite() { step } else { 0.0 };
    let reference_value = if reference_value.is_finite() {
        reference_value
    } else {
        0.0
    };
    options.entry("step".into()).or_insert(step.into());
    options
        .entry("reference_value".into())
        .or_insert(reference_value.into());
    NumberFormatRequest {
        spec: spec.unwrap_or(",f").into(),
        options,
    }
}

/// Select D3's calendar-sensitive patterns for time-axis labels.
pub fn d3_datetime_tick_request() -> DateTimeFormatRequest {
    DateTimeFormatRequest::new(serde_json::json!({}))
}

pub trait DateFormatter: Debug + Send + Sync + 'static {
    fn format(
        &self,
        value: &[Option<NaiveDate>],
        default: Option<&str>,
    ) -> Result<Vec<String>, AvengerScaleError>;
}
pub trait TimestampFormatter: Debug + Send + Sync + 'static {
    fn format(
        &self,
        value: &[Option<NaiveDateTime>],
        default: Option<&str>,
    ) -> Result<Vec<String>, AvengerScaleError>;
}
pub trait TimestamptzFormatter: Debug + Send + Sync + 'static {
    fn format(
        &self,
        value: &[Option<DateTime<Utc>>],
        default: Option<&str>,
    ) -> Result<Vec<String>, AvengerScaleError>;
}
/// Format a numeric batch, substituting `default` for null values.
pub fn format_numbers(
    formatter: &dyn PreparedNumberFormatter,
    values: &[Option<f64>],
    default: Option<&str>,
) -> Vec<String> {
    values
        .iter()
        .map(|value| {
            value.map_or_else(
                || default.unwrap_or("").to_owned(),
                |value| formatter.format(value).text,
            )
        })
        .collect()
}
impl<T: PreparedCivilDateTimeFormatter + ?Sized> DateFormatter for T {
    fn format(
        &self,
        values: &[Option<NaiveDate>],
        default: Option<&str>,
    ) -> Result<Vec<String>, AvengerScaleError> {
        values
            .iter()
            .map(|value| {
                value.map_or_else(
                    || Ok(default.unwrap_or("").to_owned()),
                    |value| {
                        Ok(PreparedCivilDateTimeFormatter::format(
                            self,
                            NaiveDateTimeInput::Date(value),
                        )?)
                    },
                )
            })
            .collect()
    }
}
impl DateFormatter for DefaultFormatter {
    fn format(
        &self,
        values: &[Option<NaiveDate>],
        default: Option<&str>,
    ) -> Result<Vec<String>, AvengerScaleError> {
        DateFormatter::format(self.prepare_naive("%Y-%m-%d")?.as_ref(), values, default)
    }
}
impl<T: PreparedCivilDateTimeFormatter + ?Sized> TimestampFormatter for T {
    fn format(
        &self,
        values: &[Option<NaiveDateTime>],
        default: Option<&str>,
    ) -> Result<Vec<String>, AvengerScaleError> {
        values
            .iter()
            .map(|value| {
                value.map_or_else(
                    || Ok(default.unwrap_or("").to_owned()),
                    |value| {
                        Ok(PreparedCivilDateTimeFormatter::format(
                            self,
                            NaiveDateTimeInput::DateTime(value),
                        )?)
                    },
                )
            })
            .collect()
    }
}
impl TimestampFormatter for DefaultFormatter {
    fn format(
        &self,
        values: &[Option<NaiveDateTime>],
        default: Option<&str>,
    ) -> Result<Vec<String>, AvengerScaleError> {
        TimestampFormatter::format(
            self.prepare_naive("%Y-%m-%d %H:%M:%S")?.as_ref(),
            values,
            default,
        )
    }
}
impl<T: PreparedInstantFormatter + ?Sized> TimestamptzFormatter for T {
    fn format(
        &self,
        values: &[Option<DateTime<Utc>>],
        default: Option<&str>,
    ) -> Result<Vec<String>, AvengerScaleError> {
        values
            .iter()
            .map(|value| {
                value.map_or_else(
                    || Ok(default.unwrap_or("").to_owned()),
                    |value| Ok(PreparedInstantFormatter::format(self, value)?),
                )
            })
            .collect()
    }
}
impl TimestamptzFormatter for DefaultFormatter {
    fn format(
        &self,
        values: &[Option<DateTime<Utc>>],
        default: Option<&str>,
    ) -> Result<Vec<String>, AvengerScaleError> {
        TimestamptzFormatter::format(
            self.prepare_zoned("%Y-%m-%d %H:%M:%S %Z")?.as_ref(),
            values,
            default,
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct Formatters {
    pub number: Option<Arc<dyn PreparedNumberFormatter>>,
    pub civil_datetime: Option<Arc<dyn PreparedCivilDateTimeFormatter>>,
    pub instant: Option<Arc<dyn PreparedInstantFormatter>>,
}

impl Formatters {
    /// Require a prepared formatter before producing numeric labels.
    pub fn number(&self) -> Result<&dyn PreparedNumberFormatter, AvengerScaleError> {
        self.number.as_deref().ok_or_else(|| {
            avenger_format::NumberFormatError("number formatting is not configured".into()).into()
        })
    }

    /// Require a prepared formatter before producing civil date or datetime labels.
    pub fn civil_datetime(&self) -> Result<&dyn PreparedCivilDateTimeFormatter, AvengerScaleError> {
        self.civil_datetime.as_deref().ok_or_else(|| {
            avenger_format::DateTimeFormatError(
                "civil datetime formatting is not configured".into(),
            )
            .into()
        })
    }

    /// Require a prepared formatter before producing labels for instants.
    pub fn instant(&self) -> Result<&dyn PreparedInstantFormatter, AvengerScaleError> {
        self.instant.as_deref().ok_or_else(|| {
            avenger_format::DateTimeFormatError("instant formatting is not configured".into())
                .into()
        })
    }

    /// Format an arrow array according to the registered formatters.
    /// Types other than numbers, dates, and timestamps are cast to string using the
    /// cast arrow kernel.
    pub fn format(
        &self,
        values: &ArrayRef,
        default: Option<&str>,
    ) -> Result<ScalarOrArray<String>, AvengerScaleError> {
        let dtype = values.data_type();

        match dtype {
            DataType::Date32 => {
                let values = values.as_primitive::<Date32Type>();
                let dates: Vec<_> = (0..values.len())
                    .map(|i| {
                        (!values.is_null(i))
                            .then(|| values.value_as_date(i))
                            .flatten()
                    })
                    .collect();
                Ok(ScalarOrArray::new_array(DateFormatter::format(
                    self.civil_datetime()?,
                    &dates,
                    default,
                )?))
            }

            DataType::Date64 => {
                let values = values.as_primitive::<Date64Type>();
                let dates = values
                    .iter()
                    .map(|value| {
                        value
                            .and_then(DateTime::from_timestamp_millis)
                            .map(|value| value.date_naive())
                    })
                    .collect::<Vec<_>>();
                Ok(ScalarOrArray::new_array(DateFormatter::format(
                    self.civil_datetime()?,
                    &dates,
                    default,
                )?))
            }
            DataType::Timestamp(unit, timezone) => {
                let timestamps = timestamp_values(values, unit);
                let labels = if timezone.is_some() {
                    TimestamptzFormatter::format(
                        self.instant()?,
                        &timestamps
                            .iter()
                            .map(|value| value.map(|value| value.and_utc()))
                            .collect::<Vec<_>>(),
                        default,
                    )?
                } else {
                    TimestampFormatter::format(self.civil_datetime()?, &timestamps, default)?
                };
                Ok(ScalarOrArray::new_array(labels))
            }
            _ if dtype.is_numeric() => {
                // Keep binary64 values intact until formatting.
                let values = cast(values, &DataType::Float64)?;
                let values = values.as_primitive::<Float64Type>();
                Ok(ScalarOrArray::new_array(format_numbers(
                    self.number()?,
                    &values.iter().collect::<Vec<_>>(),
                    default,
                )))
            }
            _ => {
                // Cast to string
                let default = default.unwrap_or("");
                let values = cast(values, &DataType::Utf8)?;
                Ok(ScalarOrArray::new_array(
                    values
                        .as_string::<i32>()
                        .iter()
                        .map(|s| {
                            s.map(|s| s.to_string())
                                .unwrap_or_else(|| default.to_string())
                        })
                        .collect(),
                ))
            }
        }
    }
}

/// Preserve the source resolution for the selected formatter.
pub(crate) fn timestamp_values(values: &ArrayRef, unit: &TimeUnit) -> Vec<Option<NaiveDateTime>> {
    macro_rules! timestamps {
        ($type:ty) => {{
            let values = values.as_primitive::<$type>();
            (0..values.len())
                .map(|index| {
                    (!values.is_null(index))
                        .then(|| values.value_as_datetime(index))
                        .flatten()
                })
                .collect()
        }};
    }
    match unit {
        TimeUnit::Second => timestamps!(TimestampSecondType),
        TimeUnit::Millisecond => timestamps!(TimestampMillisecondType),
        TimeUnit::Microsecond => timestamps!(TimestampMicrosecondType),
        TimeUnit::Nanosecond => timestamps!(TimestampNanosecondType),
    }
}

#[cfg(test)]
mod tests {
    use avenger_format::{FormattedNumber, NumberFormatError, NumberFormatProvider};
    #[test]
    fn numeric_labels_require_a_formatter_but_string_labels_do_not() {
        let formatters = super::Formatters::default();
        let strings = std::sync::Arc::new(arrow::array::StringArray::from(vec!["a"]))
            as arrow::array::ArrayRef;
        assert_eq!(
            formatters.format(&strings, None).unwrap().as_vec(1, None),
            ["a"]
        );
        let numbers = std::sync::Arc::new(arrow::array::Float64Array::from(vec![1.0]))
            as arrow::array::ArrayRef;
        assert!(formatters
            .format(&numbers, None)
            .unwrap_err()
            .to_string()
            .contains("number formatting is not configured"));
    }

    use super::*;
    use arrow::array::{Date64Array, TimestampMicrosecondArray, TimestampNanosecondArray};

    #[test]
    fn arrow_numbers_use_the_selected_provider_and_preserve_nulls() {
        #[derive(Debug)]
        struct Custom;
        impl NumberFormatProvider for Custom {
            fn prepare(
                &self,
                _: &NumberFormatConfig,
                request: &NumberFormatRequest,
            ) -> Result<Arc<dyn PreparedNumberFormatter>, NumberFormatError> {
                assert_eq!(request.spec, "custom");
                Ok(Arc::new(Custom))
            }
        }
        impl PreparedNumberFormatter for Custom {
            fn format(&self, value: f64) -> FormattedNumber {
                FormattedNumber::plain(format!("value={value}"))
            }
        }
        let mut registry = NumberFormatRegistry::default();
        registry.register("custom", Arc::new(Custom));
        let formatter = DefaultFormatter {
            format_str: Some("custom".into()),
            number_format: Some(NumberFormatConfig::new("custom")),
            number_formatters: Arc::new(registry),
            ..Default::default()
        };
        let formatters = Formatters {
            number: Some(formatter.prepare_number().unwrap()),
            ..Default::default()
        };
        let values = Arc::new(arrow::array::Float64Array::from(vec![Some(1.25), None])) as ArrayRef;
        assert_eq!(
            formatters
                .format(&values, Some("missing"))
                .unwrap()
                .as_vec(2, None),
            ["value=1.25", "missing"]
        );
    }

    fn d3_registry() -> DateTimeFormatRegistry {
        let mut registry = DateTimeFormatRegistry::default();
        registry.register(
            "d3",
            Arc::new(avenger_format_datetime_d3::D3DateTimeFormatProvider),
        );
        registry
    }
    fn d3_config(timezone: &str) -> DateTimeFormatConfig {
        DateTimeFormatConfig {
            timezone: Some(timezone.into()),
            ..DateTimeFormatConfig::new("d3")
        }
    }

    #[test]
    fn zoned_batches_propagate_formatting_errors() {
        for spec in [serde_json::json!("%c"), serde_json::json!({})] {
            let formatter = d3_registry()
                .prepare_zoned(&d3_config("Asia/Tokyo"), &DateTimeFormatRequest::new(spec))
                .unwrap();
            assert!(TimestamptzFormatter::format(
                formatter.as_ref(),
                &[None, Some(DateTime::<Utc>::MAX_UTC)],
                None
            )
            .unwrap_err()
            .to_string()
            .contains("calendar range"));
        }
    }

    #[test]
    fn arrow_temporal_formatting_preserves_instants_and_nulls() {
        let mut formatters = Formatters {
            instant: Some(
                d3_registry()
                    .prepare_zoned(&d3_config("UTC"), &DateTimeFormatRequest::new("%Q %f"))
                    .unwrap(),
            ),
            ..Default::default()
        };
        for values in [
            Arc::new(
                TimestampMicrosecondArray::from(vec![Some(-500), Some(-1500), None])
                    .with_timezone("UTC"),
            ) as ArrayRef,
            Arc::new(
                TimestampNanosecondArray::from(vec![Some(-500_000), Some(-1_500_000), None])
                    .with_timezone("UTC"),
            ) as ArrayRef,
        ] {
            assert_eq!(
                formatters
                    .format(&values, Some("missing"))
                    .unwrap()
                    .as_vec(3, None),
                ["0 000000", "-1 999000", "missing"]
            );
        }
        formatters.civil_datetime = Some(
            d3_registry()
                .prepare_naive(&d3_config("UTC"), &DateTimeFormatRequest::new("%Y-%m-%d"))
                .unwrap(),
        );
        let dates = Arc::new(Date64Array::from(vec![Some(-86_400_000), None])) as ArrayRef;
        assert_eq!(
            formatters
                .format(&dates, Some("missing"))
                .unwrap()
                .as_vec(2, None),
            ["1969-12-31", "missing"]
        );
        assert!(d3_registry()
            .prepare_naive(&d3_config("UTC"), &DateTimeFormatRequest::new("%Z"))
            .is_err());
    }
}
