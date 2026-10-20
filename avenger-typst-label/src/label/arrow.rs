//! Labels for the values of Arrow arrays, with the `arrow` feature.

use arrow_array::cast::AsArray;
use arrow_array::types::{
    Date32Type, Date64Type, Float64Type, TimestampMicrosecondType,
    TimestampMillisecondType, TimestampNanosecondType, TimestampSecondType,
};
use arrow_array::{Array, ArrayRef, ArrowPrimitiveType, PrimitiveArray};
use arrow_cast::cast;
use arrow_schema::{DataType, TimeUnit};
use avenger_format::{FormatValues, TickSpacing};
use chrono::NaiveDateTime;

use super::{LabelEngine, LabelError, LabelValue};

impl LabelEngine {
    /// Labels the values of an Arrow array with the engine's providers through `pattern`, as
    /// [`format`](Self::format) labels them: numbers of any numeric type, dates, naive datetimes
    /// for timestamps without a timezone, and zoned datetimes for timestamps with one. Other
    /// values, such as strings, are their text, without formatting. A null is a missing value:
    /// NaN for a number, and empty text otherwise.
    pub fn format_array(
        &self,
        values: &ArrayRef,
        pattern: &str,
        spacing: TickSpacing,
    ) -> Result<Vec<LabelValue>, LabelError> {
        let format = |values| self.format(values, pattern, spacing);
        match values.data_type() {
            data_type if data_type.is_numeric() => {
                let numbers = cast(values, &DataType::Float64)
                    .map_err(|err| LabelError::ArrowText(err.to_string()))?;
                let numbers: Vec<f64> = numbers
                    .as_primitive::<Float64Type>()
                    .iter()
                    .map(|value| value.unwrap_or(f64::NAN))
                    .collect();
                format(FormatValues::Numbers(&numbers))
            }
            DataType::Date32 => format(FormatValues::Dates(&each(
                values.as_primitive::<Date32Type>(),
                |array, i| array.value_as_date(i),
            ))),
            DataType::Date64 => format(FormatValues::Dates(&each(
                values.as_primitive::<Date64Type>(),
                |array, i| array.value_as_date(i),
            ))),
            DataType::Timestamp(unit, timezone) => {
                let datetimes = timestamps(values, *unit);
                if timezone.is_some() {
                    let instants: Vec<_> = datetimes
                        .iter()
                        .map(|value| value.map(|value| value.and_utc()))
                        .collect();
                    format(FormatValues::ZonedDateTimes(&instants))
                } else {
                    format(FormatValues::NaiveDateTimes(&datetimes))
                }
            }
            _ => {
                let text = cast(values, &DataType::Utf8)
                    .map_err(|err| LabelError::ArrowText(err.to_string()))?;
                Ok(text
                    .as_string::<i32>()
                    .iter()
                    .map(|value| LabelValue::Str(value.unwrap_or_default().to_string()))
                    .collect())
            }
        }
    }
}

/// The UTC calendar fields of timestamps, where nulls and values outside Chrono's range are
/// missing.
fn timestamps(values: &ArrayRef, unit: TimeUnit) -> Vec<Option<NaiveDateTime>> {
    match unit {
        TimeUnit::Second => {
            each(values.as_primitive::<TimestampSecondType>(), |array, i| {
                array.value_as_datetime(i)
            })
        }
        TimeUnit::Millisecond => {
            each(values.as_primitive::<TimestampMillisecondType>(), |array, i| {
                array.value_as_datetime(i)
            })
        }
        TimeUnit::Microsecond => {
            each(values.as_primitive::<TimestampMicrosecondType>(), |array, i| {
                array.value_as_datetime(i)
            })
        }
        TimeUnit::Nanosecond => {
            each(values.as_primitive::<TimestampNanosecondType>(), |array, i| {
                array.value_as_datetime(i)
            })
        }
    }
}

/// Each of `array`'s values, or `None` for a null.
fn each<T: ArrowPrimitiveType, V>(
    array: &PrimitiveArray<T>,
    value: impl Fn(&PrimitiveArray<T>, usize) -> Option<V>,
) -> Vec<Option<V>> {
    (0..array.len())
        .map(|i| if array.is_null(i) { None } else { value(array, i) })
        .collect()
}
