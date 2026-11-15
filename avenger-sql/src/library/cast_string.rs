//! `CAST(x AS STRING)` of the types whose text GoogleSQL writes otherwise than Arrow's cast: a
//! `FLOAT64` with 15 significant digits, or 17 where 15 don't give the value back, a `NUMERIC` or
//! `BIGNUMERIC` without trailing zeros, a `DATETIME` with a space and its fraction in 3 or 6
//! digits, and a `TIMESTAMP` as its civil time in the time zone, with its offset.

use std::sync::{Arc, LazyLock};

use datafusion::{
    arrow::{
        array::{Array, ArrayRef, AsArray, StringBuilder},
        datatypes::{
            DataType, Decimal128Type, Decimal256Type, DecimalType, Float64Type, Int64Type,
        },
    },
    common::{exec_err, Result},
    functions::datetime::expr_fn::to_local_time,
    logical_expr::{
        ColumnarValue, Expr, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Volatility,
    },
};

use super::{cast, columnar, form::Ty, lit_i64};
use crate::types;

const MICROS_PER_SECOND: i64 = 1_000_000;

/// `CAST(value AS STRING)` of a value of the type `from`, as GoogleSQL writes it, where Arrow's
/// cast writes it otherwise: a `FLOAT64`, a `NUMERIC` or `BIGNUMERIC`, a `DATETIME`, or a
/// `TIMESTAMP` in the time zone `zone`. `None` for another type, whose Arrow cast is GoogleSQL's.
pub(crate) fn cast_to_string(value: Expr, from: &Ty, zone: &str) -> Option<Expr> {
    Some(match from {
        Ty::Float64 => TO_STRING.call(vec![cast(value, DataType::Float64)]),
        Ty::Numeric | Ty::BigNumeric => TO_STRING.call(vec![value]),
        Ty::Datetime => TO_STRING.call(vec![cast(value, DataType::Int64)]),
        Ty::Timestamp => {
            let zoned = cast(value.clone(), types::zoned(zone));
            let local = cast(
                cast(to_local_time(vec![zoned]), types::datetime()),
                DataType::Int64,
            );
            let utc = cast(cast(value, types::timestamp()), DataType::Int64);
            let offset = (local.clone() - utc) / lit_i64(MICROS_PER_SECOND);
            TO_STRING.call(vec![local, offset])
        }
        _ => return None,
    })
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
fn float_text(value: f64) -> String {
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

/// The text of the date and time `micros`, civil microseconds, as GoogleSQL writes a `DATETIME`:
/// its fraction in 3 digits, or 6 where 3 don't hold it, or none.
fn datetime_text(micros: i64) -> Result<String> {
    let Some(datetime) = chrono::DateTime::from_timestamp_micros(micros) else {
        return exec_err!("a DATETIME out of range");
    };
    let fraction = micros.rem_euclid(MICROS_PER_SECOND);
    let fraction = match fraction {
        0 => String::new(),
        _ if fraction % 1000 == 0 => format!(".{:03}", fraction / 1000),
        _ => format!(".{fraction:06}"),
    };
    Ok(format!(
        "{}{fraction}",
        datetime.naive_utc().format("%Y-%m-%d %H:%M:%S")
    ))
}

/// The text of the value at `row` of `array` as GoogleSQL's `CAST(x AS STRING)` writes it: a
/// float as [`float_text`], a decimal without trailing zeros, and a date and time with a space.
/// A timestamp's array holds its civil microseconds, and `offset` its time zone's offset in
/// seconds, which follows as `-08` or `+05:30`.
fn text(array: &ArrayRef, row: usize, offset: Option<i64>) -> Result<String> {
    let trimmed = |text: String| match text.contains('.') {
        true => text.trim_end_matches('0').trim_end_matches('.').to_string(),
        false => text,
    };
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
        DataType::Int64 => {
            let text = datetime_text(array.as_primitive::<Int64Type>().value(row))?;
            match offset {
                None => text,
                Some(offset) => {
                    let sign = if offset < 0 { '-' } else { '+' };
                    let (hours, minutes) = (offset.abs() / 3600, offset.abs() / 60 % 60);
                    match minutes {
                        0 => format!("{text}{sign}{hours:02}"),
                        _ => format!("{text}{sign}{hours:02}:{minutes:02}"),
                    }
                }
            }
        }
        other => return exec_err!("$to_string takes no {other}"),
    })
}

static TO_STRING: LazyLock<Arc<ScalarUDF>> = LazyLock::new(|| {
    Arc::new(ScalarUDF::new_from_impl(ToString {
        signature: Signature::variadic_any(Volatility::Immutable),
    }))
});

/// The text of a value, and of the offset of a timestamp's time zone, as GoogleSQL's
/// `CAST(x AS STRING)` writes it.
#[derive(Debug, PartialEq, Eq, Hash)]
struct ToString {
    signature: Signature,
}

impl ScalarUDFImpl for ToString {
    fn name(&self) -> &str {
        "$to_string"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::Utf8)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let rows = args.number_rows;
        let values = args.args[0].clone().into_array(rows)?;
        let offsets = (args.args.get(1))
            .map(|offset| offset.clone().into_array(rows))
            .transpose()?;
        let offset_at = |row: usize| -> Option<i64> {
            let offsets = offsets.as_ref()?.as_primitive::<Int64Type>();
            let at = if offsets.len() == 1 { 0 } else { row };
            (!offsets.is_null(at)).then(|| offsets.value(at))
        };
        let mut out = StringBuilder::new();
        for row in 0..values.len() {
            match values.is_null(row) {
                true => out.append_null(),
                false => out.append_value(text(&values, row, offset_at(row))?),
            }
        }
        columnar(Arc::new(out.finish()), &args.args)
    }
}
