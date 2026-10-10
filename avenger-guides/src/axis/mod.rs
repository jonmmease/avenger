pub mod band;
pub mod continuous;
pub mod nested_band;
pub mod opts;
pub mod point;

use crate::error::AvengerGuidesError;
use arrow::{
    array::{Array, ArrayRef, AsArray, PrimitiveArray},
    compute::cast,
    datatypes::{
        ArrowPrimitiveType, DataType, Date32Type, Date64Type, Float64Type, TimeUnit,
        TimestampMicrosecondType, TimestampMillisecondType, TimestampNanosecondType,
        TimestampSecondType,
    },
};
use avenger_common::value::ScalarOrArray;
use avenger_format::{
    DateTimeFormatError, DateTimeFormatProvider, FormatError, FormatValues, NumberFormatProvider,
    PreparedFormatter, TickSpacing,
};
use avenger_scales::{error::AvengerScaleError, scales::to_text};
use chrono::NaiveDateTime;

/// The formatter for a guide over values of `data_type`, which is often known only once data
/// arrives. Dates and timestamps use `datetime`'s default calendar patterns. Other values use
/// `number_pattern` with `number`, since categories that are neither numbers nor times show as
/// text.
pub fn guide_format(
    data_type: &DataType,
    number: &dyn NumberFormatProvider,
    number_pattern: &str,
    datetime: &dyn DateTimeFormatProvider,
) -> Result<PreparedFormatter, AvengerGuidesError> {
    let patterns = || datetime.default_calendar_patterns();
    let datetime_error = |error: DateTimeFormatError| AvengerGuidesError::Format(error.into());
    Ok(match data_type {
        DataType::Date32 | DataType::Date64 => patterns()
            .prepare_date(datetime)
            .map_err(datetime_error)?
            .into(),
        DataType::Timestamp(_, None) => patterns()
            .prepare_naive(datetime)
            .map_err(datetime_error)?
            .into(),
        DataType::Timestamp(_, Some(_)) => patterns()
            .prepare_zoned(datetime)
            .map_err(datetime_error)?
            .into(),
        _ => number.prepare(number_pattern)?.into(),
    })
}

/// Labels for values as axes show them: numbers, dates, and times with `format`, which labels
/// them together when `spacing` is uniform, and other values, such as strings, as text.
pub fn label_values(
    values: &ArrayRef,
    format: &PreparedFormatter,
    spacing: TickSpacing,
) -> Result<Vec<String>, AvengerGuidesError> {
    let data_type = values.data_type();
    let labels = if data_type.is_numeric() || data_type.is_temporal() {
        tick_labels(values, format, spacing)?
    } else {
        to_text(values, "")?
    };
    Ok(labels.as_vec(values.len(), None))
}

/// Label values with the axis formatter, whose kind must match them: numbers, dates, naive
/// datetimes for timestamps without a timezone, and zoned datetimes for timestamps with one.
fn tick_labels(
    values: &ArrayRef,
    format: &PreparedFormatter,
    spacing: TickSpacing,
) -> Result<ScalarOrArray<String>, AvengerGuidesError> {
    let data_type = values.data_type();
    let labels = match data_type {
        data_type if data_type.is_numeric() => {
            format.format_ticks(FormatValues::Numbers(&tick_numbers(values)?), spacing)
        }
        DataType::Date32 => format.format_ticks(
            FormatValues::Dates(&each(values.as_primitive::<Date32Type>(), |a, i| {
                a.value_as_date(i)
            })),
            spacing,
        ),
        DataType::Date64 => format.format_ticks(
            FormatValues::Dates(&each(values.as_primitive::<Date64Type>(), |a, i| {
                a.value_as_date(i)
            })),
            spacing,
        ),
        DataType::Timestamp(unit, timezone) => {
            let datetimes = timestamps(values, *unit);
            if timezone.is_some() {
                let instants: Vec<_> = datetimes
                    .iter()
                    .map(|value| value.map(|value| value.and_utc()))
                    .collect();
                format.format_ticks(FormatValues::ZonedDateTimes(&instants), spacing)
            } else {
                format.format_ticks(FormatValues::NaiveDateTimes(&datetimes), spacing)
            }
        }
        data_type => return Err(AvengerGuidesError::UnsupportedTicks(data_type.clone())),
    };
    labels
        .map(ScalarOrArray::new_array)
        .map_err(|error| match error {
            FormatError::Mismatch { formatter, .. } => AvengerGuidesError::FormatMismatch {
                formatter,
                ticks: data_type.clone(),
            },
            error => AvengerGuidesError::Format(error),
        })
}

/// Numeric ticks as `f64`, with NaN for nulls.
fn tick_numbers(values: &ArrayRef) -> Result<Vec<f64>, AvengerGuidesError> {
    let values = cast(values, &DataType::Float64).map_err(AvengerScaleError::from)?;
    Ok(values
        .as_primitive::<Float64Type>()
        .iter()
        .map(|value| value.unwrap_or(f64::NAN))
        .collect())
}

/// UTC calendar fields of timestamps; nulls and values outside Chrono's range are missing.
fn timestamps(values: &ArrayRef, unit: TimeUnit) -> Vec<Option<NaiveDateTime>> {
    match unit {
        TimeUnit::Second => each(values.as_primitive::<TimestampSecondType>(), |a, i| {
            a.value_as_datetime(i)
        }),
        TimeUnit::Millisecond => each(values.as_primitive::<TimestampMillisecondType>(), |a, i| {
            a.value_as_datetime(i)
        }),
        TimeUnit::Microsecond => each(values.as_primitive::<TimestampMicrosecondType>(), |a, i| {
            a.value_as_datetime(i)
        }),
        TimeUnit::Nanosecond => each(values.as_primitive::<TimestampNanosecondType>(), |a, i| {
            a.value_as_datetime(i)
        }),
    }
}

fn each<T: ArrowPrimitiveType, V>(
    array: &PrimitiveArray<T>,
    value: impl Fn(&PrimitiveArray<T>, usize) -> Option<V>,
) -> Vec<Option<V>> {
    (0..array.len())
        .map(|i| {
            if array.is_null(i) {
                None
            } else {
                value(array, i)
            }
        })
        .collect()
}
