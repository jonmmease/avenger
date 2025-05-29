use crate::error::AvengerScaleError;
use arrow::array::ArrayRef;
use arrow::array::{timezone::Tz as ArrowTz, AsArray};
use arrow::datatypes::Float32Type;
use arrow::{
    compute::kernels::cast,
    datatypes::{DataType, Date32Type, TimeUnit, TimestampMillisecondType},
};
use avenger_common::value::ScalarOrArray;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use chrono_tz::Tz;
use crate::format_num::NumberFormat;
use std::{fmt::Debug, str::FromStr, sync::Arc};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DefaultFormatter {
    pub format_str: Option<String>,
    pub local_tz: Option<Tz>,
}

pub trait NumberFormatter: Debug + Send + Sync + 'static {
    fn format(&self, value: &[Option<f32>], default: Option<&str>) -> Vec<String>;
}

pub trait DateFormatter: Debug + Send + Sync + 'static {
    fn format(&self, value: &[Option<NaiveDate>], default: Option<&str>) -> Vec<String>;
}

pub trait TimestampFormatter: Debug + Send + Sync + 'static {
    fn format(&self, value: &[Option<NaiveDateTime>], default: Option<&str>) -> Vec<String>;
}

pub trait TimestamptzFormatter: Debug + Send + Sync + 'static {
    fn format(&self, value: &[Option<DateTime<Utc>>], default: Option<&str>) -> Vec<String>;
}

impl NumberFormatter for DefaultFormatter {
    fn format(&self, value: &[Option<f32>], default: Option<&str>) -> Vec<String> {
        let default = default.unwrap_or("");
        if let Some(format_str) = &self.format_str {
            // Use format_num for d3-style formatting
            let formatter = NumberFormat::new();
            value
                .iter()
                .map(|&v| {
                    v.map(|v| {
                        // Use format_num for d3-style formatting
                        formatter.format(format_str, v)
                    })
                    .unwrap_or_else(|| default.to_string())
                })
                .collect()
        } else {
            value
                .iter()
                .map(|&v| {
                    v.map(|v| v.to_string())
                        .unwrap_or_else(|| default.to_string())
                })
                .collect()
        }
    }
}

impl DateFormatter for DefaultFormatter {
    fn format(&self, value: &[Option<NaiveDate>], default: Option<&str>) -> Vec<String> {
        let default = default.unwrap_or("");
        // If the format string is empty, just return the date as a string with default format
        if let Some(format_str) = &self.format_str {
            value
                .iter()
                .map(|v| {
                    v.map(|v| v.format(format_str).to_string())
                        .unwrap_or_else(|| default.to_string())
                })
                .collect()
        } else {
            value
                .iter()
                .map(|v| {
                    v.map(|v| v.to_string())
                        .unwrap_or_else(|| default.to_string())
                })
                .collect()
        }
    }
}

impl TimestampFormatter for DefaultFormatter {
    fn format(&self, value: &[Option<NaiveDateTime>], default: Option<&str>) -> Vec<String> {
        let default = default.unwrap_or("");
        // If the format string is empty, just return the date as a string with default format
        if let Some(format_str) = &self.format_str {
            value
                .iter()
                .map(|v| {
                    v.map(|v| v.format(format_str).to_string())
                        .unwrap_or_else(|| default.to_string())
                })
                .collect()
        } else {
            value
                .iter()
                .map(|v| {
                    v.map(|v| v.to_string())
                        .unwrap_or_else(|| default.to_string())
                })
                .collect()
        }
    }
}

impl TimestamptzFormatter for DefaultFormatter {
    fn format(&self, value: &[Option<DateTime<Utc>>], default: Option<&str>) -> Vec<String> {
        let default = default.unwrap_or("");
        if let Some(format_str) = &self.format_str {
            if let Some(local_tz) = &self.local_tz {
                value
                    .iter()
                    .map(|v| {
                        v.map(|v| v.with_timezone(local_tz).format(format_str).to_string())
                            .unwrap_or_else(|| default.to_string())
                    })
                    .collect()
            } else {
                value
                    .iter()
                    .map(|v| {
                        v.map(|v| v.format(format_str).to_string())
                            .unwrap_or_else(|| default.to_string())
                    })
                    .collect()
            }
        } else {
            // If the format string is empty, just return the date as a string with default format
            if let Some(local_tz) = &self.local_tz {
                value
                    .iter()
                    .map(|v| {
                        v.map(|v| v.with_timezone(local_tz).to_string())
                            .unwrap_or_else(|| default.to_string())
                    })
                    .collect()
            } else {
                value
                    .iter()
                    .map(|v| {
                        v.map(|v| v.to_string())
                            .unwrap_or_else(|| default.to_string())
                    })
                    .collect()
            }
        }
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
            number: Arc::new(DefaultFormatter::default()),
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
                // TODO: do we need to handle nulls here?
                let values = values.as_primitive::<Date32Type>();
                let dates: Vec<_> = (0..values.len()).map(|i| values.value_as_date(i)).collect();
                Ok(ScalarOrArray::new_array(self.date.format(&dates, default)))
            }

            DataType::Timestamp(_, None) => {
                let values = cast(values, &DataType::Timestamp(TimeUnit::Millisecond, None))?;
                let values = values.as_primitive::<TimestampMillisecondType>();

                let timestamps: Vec<_> = (0..values.len())
                    .map(|i| values.value_as_datetime(i))
                    .collect();
                Ok(ScalarOrArray::new_array(
                    self.timestamp.format(&timestamps, default),
                ))
            }
            DataType::Timestamp(_, Some(tz)) => {
                let values = cast(
                    values,
                    &DataType::Timestamp(TimeUnit::Millisecond, Some(tz.clone())),
                )?;
                let values = values.as_primitive::<TimestampMillisecondType>();

                // Parse timezone
                let tz = ArrowTz::from_str(tz.as_ref())?;

                // Convert to chrono timestamps with timezone
                let timestamps: Vec<_> = (0..values.len())
                    .map(|i| values.value_as_datetime_with_tz(i, tz))
                    .collect();

                // Convert to UTC
                let timestamps_utc: Vec<_> = timestamps
                    .iter()
                    .map(|t| t.map(|t| t.with_timezone(&Utc)))
                    .collect();
                Ok(ScalarOrArray::new_array(
                    self.timestamptz.format(&timestamps_utc, default),
                ))
            }
            _ if dtype.is_numeric() => {
                // Cast and downcast to f32
                let values = cast(values, &DataType::Float32)?;
                let values = values.as_primitive::<Float32Type>();
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
