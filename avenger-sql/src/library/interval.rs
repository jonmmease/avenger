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

/// The interval of `count` of `part`, as `INTERVAL count part` writes it, or `None` for a part
/// that an interval doesn't take.
pub(crate) fn of_part(count: Expr, part: DatePart) -> Option<Expr> {
    let (unit, months) = match part {
        DatePart::Year => (Unit::Months, 12),
        DatePart::Quarter => (Unit::Months, 3),
        DatePart::Month => (Unit::Months, 1),
        DatePart::Week(super::Weekday::Sunday) => (Unit::Days, 7),
        DatePart::Day => (Unit::Days, 1),
        other => (Unit::Nanos, other.micros()? * NANOS_PER_MICRO),
    };
    Some(MAKE.call(vec![count, lit(unit as i64), lit(months)]))
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
