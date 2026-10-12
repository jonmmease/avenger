pub mod band;
pub mod continuous;
mod marks;
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
use avenger_common::{types::TextSyntaxMode, value::ScalarOrArray};
use avenger_format::{FormatError, FormatValues, PreparedFormatter, TickSpacing};
use avenger_scales::{error::AvengerScaleError, scales::ConfiguredScale};
use avenger_typst_label::{bind, escape_text, LabelValue, LabelValues};
use chrono::NaiveDateTime;

/// The scale's range as mark coordinates.
fn pixel_range(scale: &ConfiguredScale) -> Result<(f32, f32), AvengerScaleError> {
    let (start, end) = scale.numeric_interval_range()?;
    Ok((start as f32, end as f32))
}

/// A tick as its formatter wrote it: text, or Typst markup, as for an exponent.
#[derive(Debug, PartialEq)]
pub(crate) enum TickText {
    Plain(String),
    Markup(String),
}

impl TickText {
    fn into_markup(self) -> String {
        match self {
            TickText::Plain(text) => escape_text(&text),
            TickText::Markup(markup) => markup,
        }
    }

    fn into_value(self) -> LabelValue {
        match self {
            TickText::Plain(text) => LabelValue::Str(text),
            TickText::Markup(markup) => LabelValue::Markup(markup),
        }
    }
}

/// Tick label text and how it reads: each tick's `tick_label` markup with `#tick` bound to the
/// tick, or else the ticks themselves, as plain text unless one of them is markup.
pub(crate) fn tick_label_text(
    ticks: Vec<TickText>,
    tick_label: Option<&str>,
) -> Result<(Vec<String>, TextSyntaxMode), AvengerGuidesError> {
    if let Some(template) = tick_label {
        let text = ticks
            .into_iter()
            .map(|tick| {
                bind(
                    template,
                    &LabelValues::from([("tick".into(), tick.into_value())]),
                )
            })
            .collect::<Result<_, _>>()?;
        return Ok((text, TextSyntaxMode::TypstMarkup));
    }
    if ticks.iter().any(|tick| matches!(tick, TickText::Markup(_))) {
        let text = ticks.into_iter().map(TickText::into_markup).collect();
        return Ok((text, TextSyntaxMode::TypstMarkup));
    }
    let text = ticks
        .into_iter()
        .map(|tick| match tick {
            TickText::Plain(text) | TickText::Markup(text) => text,
        })
        .collect();
    Ok((text, TextSyntaxMode::Plain))
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
