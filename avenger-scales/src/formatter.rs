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
    NaiveDateTimeInput, PreparedCivilDateTimeFormatter, PreparedInstantFormatter,
    PreparedNumberFormatter,
};
mod prepare;
pub mod time;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
pub use prepare::{NumberFormatAdapter, NumberLabelContext, ScaleFormatting};
use std::{fmt::Debug, sync::Arc};
pub use time::DateTimeFormatAdapter;

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

    /// Format an Arrow array with the prepared formatters.
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
    use avenger_format::FormattedNumber;
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
        impl PreparedNumberFormatter for Custom {
            fn format(&self, value: f64) -> FormattedNumber {
                FormattedNumber::plain(format!("value={value}"))
            }
        }
        let formatters = Formatters {
            number: Some(Arc::new(Custom)),
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

    use avenger_format::DateTimeFormatProvider;
    use avenger_format_datetime_d3::{D3DateTimeFormatConfig, D3DateTimeFormatProvider};
    fn d3_config(timezone: &str) -> D3DateTimeFormatConfig {
        D3DateTimeFormatConfig::new().with_timezone(timezone)
    }

    #[test]
    fn zoned_batches_propagate_formatting_errors() {
        for spec in [Some("%c"), None] {
            let formatter = DateTimeFormatAdapter::d3(d3_config("Asia/Tokyo"), Default::default())
                .prepare_zoned(spec)
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
                D3DateTimeFormatProvider
                    .prepare_zoned(&d3_config("UTC"), "%Q %f")
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
            D3DateTimeFormatProvider
                .prepare_naive(&d3_config("UTC"), "%Y-%m-%d")
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
        assert!(D3DateTimeFormatProvider
            .prepare_naive(&d3_config("UTC"), "%Z")
            .is_err());
    }
}
