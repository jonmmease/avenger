//! Axes prepared once and drawn at any length, as whole axes or as parts that a layout places.

mod band;
mod continuous;
mod marks;
mod nested_band;
mod opts;
mod prepared;

pub use nested_band::NestedBandAxisLevelConfig;
pub use opts::{AxisConfig, AxisStyle, AxisTickInterval};
pub use prepared::{Axis, AxisPart};

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
use avenger_common::types::TextSyntaxMode;
use avenger_format::{FormatValues, TickSpacing};
use avenger_scales::{error::AvengerScaleError, scales::to_text};
use avenger_typst_label::{bind, escape_text, LabelEngine, LabelValue, LabelValues};
use chrono::NaiveDateTime;

/// Tick label text and how it reads: each tick's `tick_label` markup with `#tick` bound to the
/// tick, or else the ticks themselves, which are text or markup, as plain text unless one of
/// them is markup.
pub(crate) fn tick_label_text(
    ticks: Vec<LabelValue>,
    tick_label: Option<&str>,
) -> Result<(Vec<String>, TextSyntaxMode), AvengerGuidesError> {
    if let Some(template) = tick_label {
        let text = ticks
            .into_iter()
            .map(|tick| bind(template, &LabelValues::from([("tick".into(), tick)])))
            .collect::<Result<_, _>>()?;
        return Ok((text, TextSyntaxMode::TypstMarkup));
    }
    let markup = ticks
        .iter()
        .any(|tick| matches!(tick, LabelValue::Markup(_)));
    let text = ticks
        .into_iter()
        .map(|tick| match tick {
            LabelValue::Markup(markup) => markup,
            LabelValue::Str(text) if markup => escape_text(&text),
            LabelValue::Str(text) => text,
            tick => unreachable!("formatted ticks are text or markup, not {tick:?}"),
        })
        .collect();
    let syntax = if markup {
        TextSyntaxMode::TypstMarkup
    } else {
        TextSyntaxMode::Plain
    };
    Ok((text, syntax))
}

/// Values labeled with the text engine's providers through `pattern`, as
/// [`LabelEngine::format`] labels them: numbers, dates, naive datetimes for timestamps without a
/// timezone, and zoned datetimes for timestamps with one. Other values, such as strings, are
/// their text.
pub(crate) fn format_values(
    values: &ArrayRef,
    pattern: &str,
    spacing: TickSpacing,
    text_engine: &LabelEngine,
) -> Result<Vec<LabelValue>, AvengerGuidesError> {
    let format = |values| text_engine.format(values, pattern, spacing);
    let labels = match values.data_type() {
        data_type if data_type.is_numeric() => {
            format(FormatValues::Numbers(&tick_numbers(values)?))
        }
        DataType::Date32 => format(FormatValues::Dates(&each(
            values.as_primitive::<Date32Type>(),
            |a, i| a.value_as_date(i),
        ))),
        DataType::Date64 => format(FormatValues::Dates(&each(
            values.as_primitive::<Date64Type>(),
            |a, i| a.value_as_date(i),
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
            let text = to_text(values, "")?.as_vec(values.len(), None);
            return Ok(text.into_iter().map(LabelValue::Str).collect());
        }
    };
    Ok(labels?)
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
