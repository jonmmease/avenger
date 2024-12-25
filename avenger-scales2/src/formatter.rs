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
use numfmt::Formatter;
use std::{fmt::Debug, str::FromStr, sync::Arc};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DefaultFormatter {
    pub format_str: Option<String>,
    pub local_tz: Option<Tz>,
}

pub trait NumberFormatter: Debug + Send + Sync + 'static {
    fn format(&self, value: &[f32]) -> Vec<String>;
}

pub trait DateFormatter: Debug + Send + Sync + 'static {
    fn format(&self, value: &[NaiveDate]) -> Vec<String>;
}

pub trait TimestampFormatter: Debug + Send + Sync + 'static {
    fn format(&self, value: &[NaiveDateTime]) -> Vec<String>;
}

pub trait TimestamptzFormatter: Debug + Send + Sync + 'static {
    fn format(&self, value: &[DateTime<Utc>]) -> Vec<String>;
}

impl NumberFormatter for DefaultFormatter {
    fn format(&self, value: &[f32]) -> Vec<String> {
        if let Some(format_str) = &self.format_str {
            let mut f: Formatter = format_str.parse().unwrap(); // Fix panic
            value.iter().map(|&v| f.fmt2(v).to_string()).collect()
        } else {
            value.iter().map(|&v| v.to_string()).collect()
        }
    }
}

impl DateFormatter for DefaultFormatter {
    fn format(&self, value: &[NaiveDate]) -> Vec<String> {
        // If the format string is empty, just return the date as a string with default format
        if let Some(format_str) = &self.format_str {
            value
                .iter()
                .map(|v| v.format(format_str).to_string())
                .collect()
        } else {
            value.iter().map(|v| v.to_string()).collect()
        }
    }
}

impl TimestampFormatter for DefaultFormatter {
    fn format(&self, value: &[NaiveDateTime]) -> Vec<String> {
        // If the format string is empty, just return the date as a string with default format
        if let Some(format_str) = &self.format_str {
            value
                .iter()
                .map(|v| v.format(format_str).to_string())
                .collect()
        } else {
            value.iter().map(|v| v.to_string()).collect()
        }
    }
}

impl TimestamptzFormatter for DefaultFormatter {
    fn format(&self, value: &[DateTime<Utc>]) -> Vec<String> {
        if let Some(format_str) = &self.format_str {
            if let Some(local_tz) = &self.local_tz {
                value
                    .iter()
                    .map(|v| v.with_timezone(local_tz).format(format_str).to_string())
                    .collect()
            } else {
                value
                    .iter()
                    .map(|v| v.format(format_str).to_string())
                    .collect()
            }
        } else {
            // If the format string is empty, just return the date as a string with default format
            if let Some(local_tz) = &self.local_tz {
                value
                    .iter()
                    .map(|v| v.with_timezone(local_tz).to_string())
                    .collect()
            } else {
                value.iter().map(|v| v.to_string()).collect()
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
    pub fn format(&self, values: &ArrayRef) -> Result<ScalarOrArray<String>, AvengerScaleError> {
        let dtype = values.data_type();

        match dtype {
            DataType::Date32 => {
                // TODO: do we need to handle nulls here?
                let values = values.as_primitive::<Date32Type>();
                let dates: Vec<_> = (0..values.len())
                    .map(|i| values.value_as_date(i).unwrap())
                    .collect();
                Ok(ScalarOrArray::new_array(self.date.format(&dates)))
            }
            DataType::Timestamp(_, None) => {
                let values = cast(values, &DataType::Timestamp(TimeUnit::Millisecond, None))?;
                let values = values.as_primitive::<TimestampMillisecondType>();

                let timestamps: Vec<_> = (0..values.len())
                    .map(|i| values.value_as_datetime(i).unwrap())
                    .collect();
                Ok(ScalarOrArray::new_array(self.timestamp.format(&timestamps)))
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
                    .map(|i| values.value_as_datetime_with_tz(i, tz).unwrap())
                    .collect();

                // Convert to UTC
                let timestamps_utc: Vec<_> =
                    timestamps.iter().map(|t| t.with_timezone(&Utc)).collect();
                Ok(ScalarOrArray::new_array(self.timestamptz.format(&timestamps_utc)))
            }
            _ if dtype.is_numeric() => {
                // Cast and downcast to f32
                let values = cast(values, &DataType::Float32)?;
                let values = values.as_primitive::<Float32Type>();
                Ok(ScalarOrArray::new_array(self.number.format(values.values())))
            }
            _ => {
                // Cast to string
                let values = cast(values, &DataType::Utf8)?;
                Ok(ScalarOrArray::new_array(values
                    .as_string::<i32>()
                    .iter()
                    .map(|s| s.unwrap_or("").to_string())
                    .collect()))
            }
        }
    }
}
