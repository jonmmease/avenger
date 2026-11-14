//! The arithmetic of `INTERVAL`s that DataFusion lacks: an interval of a count of a part, the
//! product of an interval and an integer, and its quotient.
//!
//! An `INTERVAL` is an Arrow `Interval(MonthDayNano)`, whose months, days and nanoseconds are
//! apart, as GoogleSQL's months, days and microseconds are. GoogleSQL keeps microseconds, so a
//! quotient truncates its nanoseconds to microseconds.

use std::sync::{Arc, LazyLock};

use datafusion::{
    arrow::{
        array::{Array, ArrayRef, AsArray, Int64Array, IntervalMonthDayNanoArray},
        datatypes::{
            DataType, Int64Type, IntervalMonthDayNano, IntervalMonthDayNanoType, IntervalUnit,
        },
    },
    common::{exec_err, Result, ScalarValue},
    logical_expr::{
        ColumnarValue, Expr, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Volatility,
    },
};

use super::DatePart;

const NANOS_PER_MICRO: i64 = 1_000;
const NANOS_PER_DAY: i64 = 86_400_000_000_000;

/// Whether `INTERVAL count part` takes the part `part`.
pub(crate) fn takes(part: DatePart) -> bool {
    unit(part).is_some()
}

/// The unit of an interval of the part `part`, and the count of the unit in one part.
fn unit(part: DatePart) -> Option<(Unit, i64)> {
    Some(match part {
        DatePart::Year => (Unit::Months, 12),
        DatePart::Quarter => (Unit::Months, 3),
        DatePart::Month => (Unit::Months, 1),
        DatePart::Week(super::Weekday::Sunday) => (Unit::Days, 7),
        DatePart::Day => (Unit::Days, 1),
        other => (Unit::Nanos, other.micros()? * NANOS_PER_MICRO),
    })
}

/// The interval of `count` of `part`, as `INTERVAL count part` writes it, or `None` for a part
/// that an interval doesn't take.
pub(crate) fn of_part(count: Expr, part: DatePart) -> Option<Expr> {
    let (unit, months) = unit(part)?;
    Some(MAKE.call(vec![count, lit(unit as i64), lit(months)]))
}

/// The interval of `INTERVAL 'text' from TO to`, a literal, or the error of its text.
pub(crate) fn of_text(text: &str, from: DatePart, to: DatePart) -> Result<Expr, String> {
    let (months, days, nanos) = parts_of_text(text, from, to)?;
    Ok(Expr::Literal(
        ScalarValue::IntervalMonthDayNano(Some(IntervalMonthDayNano::new(months, days, nanos))),
        None,
    ))
}

/// The months, days and nanoseconds of the text of `INTERVAL 'text' from TO to`, as GoogleSQL reads
/// it: `Y-M` for years and months, then days, then `H:M:S` for the parts of the time, each group
/// with a sign of its own, for the parts from `from` to `to`.
fn parts_of_text(text: &str, from: DatePart, to: DatePart) -> Result<(i32, i32, i64), String> {
    let invalid = || format!("Invalid INTERVAL value '{text}'");
    let rank = |part: DatePart| match part {
        DatePart::Year => Some(0),
        DatePart::Month => Some(1),
        DatePart::Day => Some(2),
        DatePart::Hour => Some(3),
        DatePart::Minute => Some(4),
        DatePart::Second => Some(5),
        _ => None,
    };
    let (Some(start), Some(end)) = (rank(from), rank(to)) else {
        return Err(format!(
            "Unsupported INTERVAL datetime fields {} TO {}",
            from.name(),
            to.name()
        ));
    };
    if start >= end {
        return Err(format!(
            "Unsupported INTERVAL datetime fields {} TO {}",
            from.name(),
            to.name()
        ));
    }
    let signed = |token: &str| -> (i64, String) {
        match token.strip_prefix('-') {
            Some(rest) => (-1, rest.to_string()),
            None => (1, token.trim_start_matches('+').to_string()),
        }
    };
    let number = |text: &str| text.parse::<i64>().map_err(|_| invalid());
    let mut tokens = text.split_whitespace();
    let (mut months, mut days, mut nanos) = (0i64, 0i64, 0i128);
    if start <= 1 {
        let (sign, token) = signed(tokens.next().ok_or_else(invalid)?);
        let value = match (start, end) {
            (0, _) => {
                let (years, rest) = token.split_once('-').ok_or_else(invalid)?;
                number(years)? * 12 + number(rest)?
            }
            _ => number(&token)?,
        };
        months = sign * value;
    }
    if start <= 2 && end >= 2 {
        let (sign, token) = signed(tokens.next().ok_or_else(invalid)?);
        days = sign * number(&token)?;
    }
    if end >= 3 {
        let (sign, token) = signed(tokens.next().ok_or_else(invalid)?);
        let first = start.max(3);
        let fields: Vec<&str> = token.split(':').collect();
        if fields.len() != end - first + 1 {
            return Err(invalid());
        }
        let mut total: i128 = 0;
        for (offset, field) in fields.iter().enumerate() {
            let unit: i128 = match first + offset {
                3 => 3_600_000_000_000,
                4 => 60_000_000_000,
                _ => 1_000_000_000,
            };
            let value: i128 = match (first + offset, field.split_once('.')) {
                (5, Some((whole, fraction))) => {
                    if fraction.len() > 9 || !fraction.bytes().all(|b| b.is_ascii_digit()) {
                        return Err(invalid());
                    }
                    let digits = format!("{fraction:0<9}");
                    i128::from(number(whole)?) * unit
                        + digits.parse::<i128>().map_err(|_| invalid())?
                }
                _ => i128::from(number(field)?) * unit,
            };
            total += value;
        }
        nanos = i128::from(sign) * total;
    }
    if tokens.next().is_some() {
        return Err(invalid());
    }
    let months = i32::try_from(months).map_err(|_| invalid())?;
    let days = i32::try_from(days).map_err(|_| invalid())?;
    let nanos = i64::try_from(nanos).map_err(|_| invalid())?;
    Ok((months, days, nanos))
}

/// The interval of `days`, an `INT64`, such as the difference of two dates.
pub(crate) fn of_days(days: Expr) -> Expr {
    MAKE.call(vec![days, lit(Unit::Days as i64), lit(1)])
}

/// `interval * factor`, for an `INT64` factor: each part of the interval times the factor.
pub(crate) fn multiply(interval: Expr, factor: Expr) -> Expr {
    MULTIPLY.call(vec![interval, factor])
}

/// `interval / divisor`, for an `INT64` divisor.
pub(crate) fn divide(interval: Expr, divisor: Expr) -> Expr {
    DIVIDE.call(vec![interval, divisor])
}

fn lit(value: i64) -> Expr {
    Expr::Literal(ScalarValue::Int64(Some(value)), None)
}

/// The part of an interval that [`MAKE`] fills.
#[derive(Clone, Copy)]
enum Unit {
    Months = 0,
    Days = 1,
    Nanos = 2,
}

static MAKE: LazyLock<Arc<ScalarUDF>> = LazyLock::new(|| udf(Op::Make, 3));
static MULTIPLY: LazyLock<Arc<ScalarUDF>> = LazyLock::new(|| udf(Op::Multiply, 2));
static DIVIDE: LazyLock<Arc<ScalarUDF>> = LazyLock::new(|| udf(Op::Divide, 2));

fn udf(op: Op, args: usize) -> Arc<ScalarUDF> {
    Arc::new(ScalarUDF::new_from_impl(IntervalFunction {
        op,
        signature: Signature::any(args, Volatility::Immutable),
    }))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Op {
    /// `(count, unit, scale)`: an interval of `count * scale` of the unit.
    Make,
    Multiply,
    Divide,
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct IntervalFunction {
    op: Op,
    signature: Signature,
}

impl ScalarUDFImpl for IntervalFunction {
    fn name(&self) -> &str {
        match self.op {
            Op::Make => "$interval",
            Op::Multiply => "$interval_multiply",
            Op::Divide => "$interval_divide",
        }
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::Interval(IntervalUnit::MonthDayNano))
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let rows = args.number_rows;
        let scalar = args
            .args
            .iter()
            .all(|arg| matches!(arg, ColumnarValue::Scalar(_)));
        let arrays = ColumnarValue::values_to_arrays(&args.args)?;
        let result: IntervalMonthDayNanoArray = match self.op {
            Op::Make => {
                let count = ints(&arrays[0])?;
                let unit = ints(&arrays[1])?;
                let scale = ints(&arrays[2])?;
                (0..count.len())
                    .map(|row| {
                        if count.is_null(row) {
                            return Ok(None);
                        }
                        let Some(value) = count.value(row).checked_mul(scale.value(row)) else {
                            return exec_err!("the interval is out of range");
                        };
                        make(unit.value(row), value).map(Some)
                    })
                    .collect::<Result<_>>()?
            }
            Op::Multiply | Op::Divide => {
                let interval = arrays[0].as_primitive::<IntervalMonthDayNanoType>();
                let factor = ints(&arrays[1])?;
                (0..interval.len())
                    .map(|row| {
                        if interval.is_null(row) || factor.is_null(row) {
                            return Ok(None);
                        }
                        let (value, n) = (interval.value(row), factor.value(row));
                        match self.op {
                            Op::Multiply => multiply_parts(value, n),
                            _ => divide_parts(value, n),
                        }
                        .map(Some)
                    })
                    .collect::<Result<_>>()?
            }
        };
        let result: ArrayRef = Arc::new(result);
        match scalar && rows == 1 {
            true => Ok(ColumnarValue::Scalar(ScalarValue::try_from_array(
                &result, 0,
            )?)),
            false => Ok(ColumnarValue::Array(result)),
        }
    }
}

/// The array `array` as `INT64`s.
fn ints(array: &ArrayRef) -> Result<Int64Array> {
    let cast = datafusion::arrow::compute::cast(array, &DataType::Int64)?;
    Ok(cast.as_primitive::<Int64Type>().clone())
}

fn make(unit: i64, value: i64) -> Result<IntervalMonthDayNano> {
    let narrow = |value: i64| -> Result<i32> {
        match i32::try_from(value) {
            Ok(value) => Ok(value),
            Err(_) => exec_err!("the interval is out of range"),
        }
    };
    Ok(match unit {
        0 => IntervalMonthDayNano::new(narrow(value)?, 0, 0),
        1 => IntervalMonthDayNano::new(0, narrow(value)?, 0),
        _ => IntervalMonthDayNano::new(0, 0, value),
    })
}

fn multiply_parts(value: IntervalMonthDayNano, n: i64) -> Result<IntervalMonthDayNano> {
    let months = i64::from(value.months).checked_mul(n);
    let days = i64::from(value.days).checked_mul(n);
    let nanos = value.nanoseconds.checked_mul(n);
    match (months, days, nanos) {
        (Some(months), Some(days), Some(nanos)) => {
            match (i32::try_from(months), i32::try_from(days)) {
                (Ok(months), Ok(days)) => Ok(IntervalMonthDayNano::new(months, days, nanos)),
                _ => exec_err!("the interval is out of range"),
            }
        }
        _ => exec_err!("the interval is out of range"),
    }
}

/// The quotient of an interval, as GoogleSQL divides one: the months, then the days with the
/// remainder of the months as 30 days each, then the time with the remainder of the days as 24
/// hours each, in microseconds.
fn divide_parts(value: IntervalMonthDayNano, n: i64) -> Result<IntervalMonthDayNano> {
    if n == 0 {
        return exec_err!("division by zero: an interval divided by 0");
    }
    let months = i64::from(value.months);
    let days = i64::from(value.days) + months % n * 30;
    let nanos = i128::from(value.nanoseconds) + i128::from(days % n) * i128::from(NANOS_PER_DAY);
    let micros = nanos / i128::from(n) / i128::from(NANOS_PER_MICRO);
    Ok(IntervalMonthDayNano::new(
        (months / n) as i32,
        (days / n) as i32,
        (micros * i128::from(NANOS_PER_MICRO)) as i64,
    ))
}
