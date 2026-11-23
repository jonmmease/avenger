//! `CAST(x AS STRING)` of the types whose text GoogleSQL writes otherwise than Arrow's cast: a
//! `FLOAT64` with 15 significant digits, or 17 where 15 don't give the value back, a `NUMERIC` or
//! `BIGNUMERIC` without trailing zeros, a `DATETIME` with a space and its fraction in 3 or 6
//! digits, a `TIMESTAMP` as its civil time in the time zone, with its offset, and an `INTERVAL`
//! as `Y-M D H:M:S`.

use std::sync::Arc;

use avenger_format_datetime_googlesql::Zone;
use chrono::NaiveDateTime;
use datafusion::{
    arrow::{
        array::{Array, ArrayRef, AsArray, StringBuilder},
        datatypes::{
            DataType, Decimal128Type, Decimal256Type, DecimalType, Float64Type,
            IntervalMonthDayNanoType, TimeUnit, TimestampMicrosecondType,
        },
    },
    common::{exec_err, Result},
    logical_expr::{
        ColumnarValue, Expr, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Volatility,
    },
};

use super::{cast, columnar, form::Ty};
use crate::types;

const MICROS_PER_SECOND: i64 = 1_000_000;

/// `CAST(value AS STRING)` of a value of the type `from`, as GoogleSQL writes it, where Arrow's
/// cast writes it otherwise: a `FLOAT64`, a `NUMERIC` or `BIGNUMERIC`, a `DATETIME`, a
/// `TIMESTAMP` in the time zone `zone`, or an `INTERVAL`. `None` for another type, whose Arrow
/// cast is GoogleSQL's, and for a time zone that GoogleSQL doesn't read.
pub(crate) fn cast_to_string(value: Expr, from: &Ty, zone: &str) -> Option<Expr> {
    let (value, zone) = match from {
        Ty::Float64 => (cast(value, DataType::Float64), Zone::UTC),
        Ty::Numeric | Ty::BigNumeric | Ty::Interval => (value, Zone::UTC),
        Ty::Datetime => (cast(value, types::datetime()), Zone::UTC),
        Ty::Timestamp => (cast(value, types::timestamp()), Zone::parse(zone)?),
        _ => return None,
    };
    let udf = CastToString {
        zone,
        signature: Signature::any(1, Volatility::Immutable),
    };
    Some(ScalarUDF::new_from_impl(udf).call(vec![value]))
}

/// `value` as C's `printf` writes it with `%.{precision}g`.
fn printf_g(value: f64, precision: usize) -> String {
    let scientific = format!("{value:.*e}", precision - 1);
    let (mantissa, exponent) = scientific.split_once('e').expect("an exponent");
    let exponent: i32 = exponent.parse().expect("an integer exponent");
    let trim = |text: &str| -> String {
        match text.contains('.') {
            true => text.trim_end_matches('0').trim_end_matches('.').to_string(),
            false => text.to_string(),
        }
    };
    if exponent < -4 || exponent >= precision as i32 {
        let sign = if exponent < 0 { '-' } else { '+' };
        format!("{}e{sign}{:02}", trim(mantissa), exponent.abs())
    } else {
        let decimals = (precision as i32 - 1 - exponent).max(0) as usize;
        trim(&format!("{value:.decimals$}"))
    }
}

/// A `FLOAT64` as GoogleSQL writes it: with 15 significant digits, or 17 where 15 don't give
/// the value back, as `printf`'s `%g` does, and `nan`, `inf` and `-inf`. Negative zero is `0`.
pub(crate) fn float_text(value: f64) -> String {
    match value {
        _ if value.is_nan() => "nan".to_string(),
        f64::INFINITY => "inf".to_string(),
        f64::NEG_INFINITY => "-inf".to_string(),
        0.0 => "0".to_string(),
        _ => {
            let short = printf_g(value, 15);
            match short.parse::<f64>() == Ok(value) {
                true => short,
                false => printf_g(value, 17),
            }
        }
    }
}

/// The fraction of a second of `micros`, microseconds, as GoogleSQL writes it: in 3 digits, or 6
/// where 3 don't hold it, or none.
pub(crate) fn fraction_text(micros: i64) -> String {
    match micros {
        0 => String::new(),
        _ if micros % 1000 == 0 => format!(".{:03}", micros / 1000),
        _ => format!(".{micros:06}"),
    }
}

/// The text of the date and time `datetime` as GoogleSQL writes a `DATETIME`.
fn datetime_text(datetime: NaiveDateTime) -> String {
    let micros = i64::from(datetime.and_utc().timestamp_subsec_micros());
    format!(
        "{}{}",
        datetime.format("%Y-%m-%d %H:%M:%S"),
        fraction_text(micros)
    )
}

/// The text of an interval of `months`, `days` and `nanoseconds` as GoogleSQL writes it:
/// `Y-M D H:M:S`, with the fraction of its seconds.
fn interval_text(months: i32, days: i32, nanoseconds: i64) -> String {
    let sign = if months < 0 { "-" } else { "" };
    let (years, months) = (months.abs() / 12, months.abs() % 12);
    let micros = nanoseconds / 1_000;
    let time_sign = if micros < 0 { "-" } else { "" };
    let micros = micros.abs();
    let (hours, minutes, seconds) = (
        micros / 3_600_000_000,
        micros / 60_000_000 % 60,
        micros / MICROS_PER_SECOND % 60,
    );
    let fraction = fraction_text(micros % MICROS_PER_SECOND);
    format!("{sign}{years}-{months} {days} {time_sign}{hours}:{minutes}:{seconds}{fraction}")
}

/// The text of the value at `row` of `array` as GoogleSQL's `CAST(x AS STRING)` writes it, with
/// a timestamp's civil time and offset in the time zone `zone`, as `-08` or `+05:30`.
pub(crate) fn text(array: &ArrayRef, row: usize, zone: Zone) -> Result<String> {
    let trimmed = |text: String| match text.contains('.') {
        true => text.trim_end_matches('0').trim_end_matches('.').to_string(),
        false => text,
    };
    let out_of_range = || exec_err!("a date and time out of range");
    Ok(match array.data_type() {
        DataType::Float64 => float_text(array.as_primitive::<Float64Type>().value(row)),
        DataType::Decimal128(precision, scale) => trimmed(Decimal128Type::format_decimal(
            array.as_primitive::<Decimal128Type>().value(row),
            *precision,
            *scale,
        )),
        DataType::Decimal256(precision, scale) => trimmed(Decimal256Type::format_decimal(
            array.as_primitive::<Decimal256Type>().value(row),
            *precision,
            *scale,
        )),
        DataType::Timestamp(TimeUnit::Microsecond, None) => {
            let values = array.as_primitive::<TimestampMicrosecondType>();
            match values.value_as_datetime(row) {
                Some(datetime) => datetime_text(datetime),
                None => return out_of_range(),
            }
        }
        DataType::Timestamp(TimeUnit::Microsecond, Some(_)) => {
            let values = array.as_primitive::<TimestampMicrosecondType>();
            let Some(instant) = values.value_as_datetime(row).map(|utc| utc.and_utc()) else {
                return out_of_range();
            };
            let local = zone.local(instant);
            let offset = (local - instant.naive_utc()).num_seconds();
            let sign = if offset < 0 { '-' } else { '+' };
            let (hours, minutes) = (offset.abs() / 3600, offset.abs() / 60 % 60);
            match minutes {
                0 => format!("{}{sign}{hours:02}", datetime_text(local)),
                _ => format!("{}{sign}{hours:02}:{minutes:02}", datetime_text(local)),
            }
        }
        DataType::Interval(_) => {
            let value = array.as_primitive::<IntervalMonthDayNanoType>().value(row);
            interval_text(value.months, value.days, value.nanoseconds)
        }
        other => return exec_err!("$cast_to_string takes no {other}"),
    })
}

/// The text of a value as GoogleSQL's `CAST(x AS STRING)` writes it, with a timestamp in the time
/// zone `zone`.
#[derive(Debug, PartialEq, Eq, Hash)]
struct CastToString {
    zone: Zone,
    signature: Signature,
}

impl ScalarUDFImpl for CastToString {
    fn name(&self) -> &str {
        "$cast_to_string"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::Utf8)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let values = args.args[0].clone().into_array(args.number_rows)?;
        let mut out = StringBuilder::new();
        for row in 0..values.len() {
            match values.is_null(row) {
                true => out.append_null(),
                false => out.append_value(text(&values, row, self.zone)?),
            }
        }
        columnar(Arc::new(out.finish()), &args.args)
    }
}
