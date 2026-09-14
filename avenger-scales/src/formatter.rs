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
use avenger_format_datetime::{
    DateTimeFormatContext, DateTimeLocaleRegistry, NaiveDateTimeInput, PreparedDateTimeFormat,
    PreparedTimeMultiFormat,
};
use avenger_format_number::{
    prepare_number_float_format, NumberFormatContext, NumberLocaleRegistry, PreparedNumberFormat,
};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use chrono_tz::Tz;
use std::{fmt::Debug, sync::Arc};

/// Formatting options that prepare one shared engine for each batch.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DefaultFormatter {
    pub format_str: Option<String>,
    pub local_tz: Option<Tz>,
    pub number_locale: Option<String>,
    pub number_locale_registry: Option<Arc<NumberLocaleRegistry>>,
}
impl DefaultFormatter {
    /// Resolve number configuration before formatting any values.
    pub fn prepare_number(&self) -> Result<PreparedNumberFormat, AvengerScaleError> {
        let registry = self
            .number_locale_registry
            .clone()
            .unwrap_or_else(|| Arc::new(NumberLocaleRegistry::with_builtins()));
        let locale = registry.resolve(self.number_locale.as_deref().unwrap_or("en-US"))?;
        let context = NumberFormatContext::new(&locale);
        if let Some(spec) = &self.format_str {
            PreparedNumberFormat::new(Some(spec), Default::default(), context)
        } else {
            prepare_number_float_format(None, context)
        }
        .map_err(AvengerScaleError::from)
    }
    fn prepare_datetime(&self, default: &str) -> Result<PreparedDateTimeFormat, AvengerScaleError> {
        let locale = DateTimeLocaleRegistry::with_builtins().resolve("en-US")?;
        Ok(PreparedDateTimeFormat::new(
            Some(self.format_str.as_deref().unwrap_or(default)),
            Default::default(),
            DateTimeFormatContext::new(&locale, self.local_tz.unwrap_or(Tz::UTC)),
        )?)
    }
}

pub trait NumberFormatter: Debug + Send + Sync + 'static {
    fn format(&self, value: &[Option<f64>], default: Option<&str>) -> Vec<String>;
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
impl NumberFormatter for PreparedNumberFormat {
    fn format(&self, values: &[Option<f64>], default: Option<&str>) -> Vec<String> {
        values
            .iter()
            .map(|value| {
                value.map_or_else(
                    || default.unwrap_or("").to_owned(),
                    |value| self.format(value).text,
                )
            })
            .collect()
    }
}
impl DateFormatter for PreparedDateTimeFormat {
    fn format(
        &self,
        values: &[Option<NaiveDate>],
        default: Option<&str>,
    ) -> Result<Vec<String>, AvengerScaleError> {
        self.validate_naive()?;
        values
            .iter()
            .map(|value| {
                value.map_or_else(
                    || Ok(default.unwrap_or("").to_owned()),
                    |value| Ok(self.format_naive(NaiveDateTimeInput::Date(value))?.text),
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
        DateFormatter::format(&self.prepare_datetime("%Y-%m-%d")?, values, default)
    }
}
impl TimestampFormatter for PreparedDateTimeFormat {
    fn format(
        &self,
        values: &[Option<NaiveDateTime>],
        default: Option<&str>,
    ) -> Result<Vec<String>, AvengerScaleError> {
        self.validate_naive()?;
        values
            .iter()
            .map(|value| {
                value.map_or_else(
                    || Ok(default.unwrap_or("").to_owned()),
                    |value| Ok(self.format_naive(NaiveDateTimeInput::DateTime(value))?.text),
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
            &self.prepare_datetime("%Y-%m-%d %H:%M:%S")?,
            values,
            default,
        )
    }
}
impl TimestamptzFormatter for PreparedDateTimeFormat {
    fn format(
        &self,
        values: &[Option<DateTime<Utc>>],
        default: Option<&str>,
    ) -> Result<Vec<String>, AvengerScaleError> {
        Ok(values
            .iter()
            .map(|value| {
                value.map_or_else(
                    || default.unwrap_or("").to_owned(),
                    |value| self.format_zoned(value).text,
                )
            })
            .collect())
    }
}
impl TimestamptzFormatter for DefaultFormatter {
    fn format(
        &self,
        values: &[Option<DateTime<Utc>>],
        default: Option<&str>,
    ) -> Result<Vec<String>, AvengerScaleError> {
        TimestamptzFormatter::format(
            &self.prepare_datetime("%Y-%m-%d %H:%M:%S %Z")?,
            values,
            default,
        )
    }
}

impl DateFormatter for PreparedTimeMultiFormat {
    fn format(
        &self,
        values: &[Option<NaiveDate>],
        default: Option<&str>,
    ) -> Result<Vec<String>, AvengerScaleError> {
        self.validate_naive()?;
        values
            .iter()
            .map(|value| {
                value.map_or_else(
                    || Ok(default.unwrap_or("").to_owned()),
                    |value| Ok(self.format_naive(NaiveDateTimeInput::Date(value))?.text),
                )
            })
            .collect()
    }
}
impl TimestampFormatter for PreparedTimeMultiFormat {
    fn format(
        &self,
        values: &[Option<NaiveDateTime>],
        default: Option<&str>,
    ) -> Result<Vec<String>, AvengerScaleError> {
        self.validate_naive()?;
        values
            .iter()
            .map(|value| {
                value.map_or_else(
                    || Ok(default.unwrap_or("").to_owned()),
                    |value| Ok(self.format_naive(NaiveDateTimeInput::DateTime(value))?.text),
                )
            })
            .collect()
    }
}
impl TimestamptzFormatter for PreparedTimeMultiFormat {
    fn format(
        &self,
        values: &[Option<DateTime<Utc>>],
        default: Option<&str>,
    ) -> Result<Vec<String>, AvengerScaleError> {
        Ok(values
            .iter()
            .map(|value| {
                value.map_or_else(
                    || default.unwrap_or("").to_owned(),
                    |value| self.format_zoned(value).text,
                )
            })
            .collect())
    }
}

#[derive(Debug, Clone)]
pub struct Formatters {
    pub number: Arc<dyn NumberFormatter>,
    pub date: Arc<dyn DateFormatter>,
    pub timestamp: Arc<dyn TimestampFormatter>,
    pub timestamptz: Arc<dyn TimestamptzFormatter>,
}

impl Default for Formatters {
    fn default() -> Self {
        Self {
            number: Arc::new(
                DefaultFormatter::default()
                    .prepare_number()
                    .expect("default D3 number format"),
            ),
            date: Arc::new(DefaultFormatter::default()),
            timestamp: Arc::new(DefaultFormatter::default()),
            timestamptz: Arc::new(DefaultFormatter::default()),
        }
    }
}

impl Formatters {
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
                Ok(ScalarOrArray::new_array(self.date.format(&dates, default)?))
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
                Ok(ScalarOrArray::new_array(self.date.format(&dates, default)?))
            }
            DataType::Timestamp(unit, timezone) => {
                let timestamps = timestamp_values(values, unit);
                let labels = if timezone.is_some() {
                    self.timestamptz.format(
                        &timestamps
                            .iter()
                            .map(|value| value.map(|value| value.and_utc()))
                            .collect::<Vec<_>>(),
                        default,
                    )?
                } else {
                    self.timestamp.format(&timestamps, default)?
                };
                Ok(ScalarOrArray::new_array(labels))
            }
            _ if dtype.is_numeric() => {
                // Keep binary64 values intact until formatting.
                let values = cast(values, &DataType::Float64)?;
                let values = values.as_primitive::<Float64Type>();
                Ok(ScalarOrArray::new_array(
                    self.number
                        .format(&values.iter().collect::<Vec<_>>(), default),
                ))
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

/// Preserve the source resolution until the D3 formatter clips instants to milliseconds.
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
    use super::*;
    use arrow::array::{Date64Array, TimestampMicrosecondArray, TimestampNanosecondArray};

    #[test]
    fn arrow_temporal_formatting_preserves_instants_and_nulls() {
        let locale = avenger_format_datetime::ResolvedDateTimeLocale::en_us();
        let mut formatters = Formatters {
            timestamptz: Arc::new(
                PreparedDateTimeFormat::new(
                    Some("%Q %f"),
                    Default::default(),
                    DateTimeFormatContext::new(&locale, Tz::UTC),
                )
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
        let dates = Arc::new(Date64Array::from(vec![Some(-86_400_000), None])) as ArrayRef;
        assert_eq!(
            formatters
                .format(&dates, Some("missing"))
                .unwrap()
                .as_vec(2, None),
            ["1969-12-31", "missing"]
        );
        formatters.date = Arc::new(
            PreparedDateTimeFormat::new(
                Some("%Z"),
                Default::default(),
                DateTimeFormatContext::new(&locale, Tz::UTC),
            )
            .unwrap(),
        );
        assert!(formatters.format(&dates, None).is_err());
    }
}
