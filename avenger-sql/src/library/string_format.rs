//! GoogleSQL's `FORMAT`: a pattern of literal text and elements, such as `%d` and `%'.2f`, and a
//! value for each element, and for each `*` width or precision. The elements parse and print as
//! `avenger-format-number-googlesql` does for charts, which `FORMAT` extends with several
//! elements, `*` arguments, integers and `NUMERIC` values that print exactly, and `%s`, `%t`, and
//! `%T`, the text of a string, of any value, and of a literal of any value.
//!
//! A literal pattern is checked when the call lowers, with each error at its argument; a pattern
//! from a column is checked when it runs, once for each distinct pattern of a batch.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use avenger_format_datetime_googlesql::Zone;
use avenger_format_number_googlesql::{pieces, Count, Decimal, Element, Locale, Piece, Value};
use chrono::NaiveDate;
use datafusion::{
    arrow::{
        array::{Array, ArrayRef, AsArray, StringBuilder},
        compute::cast,
        datatypes::{
            DataType, Date32Type, Decimal128Type, Decimal256Type, Float64Type, Int64Type,
            Time64MicrosecondType, TimeUnit,
        },
    },
    common::{exec_err, Result},
    logical_expr::{
        ColumnarValue, Expr, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Volatility,
    },
};

use super::{cast_string, columnar, form::Ty, Call, CallError};

/// The longest pattern, width, precision, and result of `FORMAT`, in bytes, as GoogleSQL's.
const MAX_WIDTH: u64 = 1 << 20;

/// The largest precision of `%f` and `%e`, which prints every digit of any `FLOAT64`.
const MAX_FLOAT_PRECISION: u64 = 1074;

/// The symbols of `FORMAT`, which has no locale.
static LOCALE: LazyLock<Locale> = LazyLock::new(Locale::default);

static FORMAT: LazyLock<ScalarUDF> = LazyLock::new(|| {
    ScalarUDF::new_from_impl(StringFormat {
        signature: Signature::variadic_any(Volatility::Immutable),
    })
});

/// `FORMAT(format_string, values...)`. A literal pattern that fails is an error at the argument
/// that fails it, or at the pattern.
pub(crate) fn format(call: &Call) -> Result<Expr, CallError> {
    let values = call.rest();
    let mut types = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let ty = call.type_of(value)?;
        match Ty::of_arrow(&ty) {
            Some(ty) => types.push(ty),
            None => return Err(call.rest_error_at(index, format!("FORMAT takes no {ty}"))),
        }
    }
    if let Some(Expr::Literal(pattern, _)) = call.value(0) {
        if let Some(Some(pattern)) = pattern.try_as_str() {
            if let Err(failure) = plan(pattern, &types) {
                return Err(match failure.argument {
                    Some(index) => call.rest_error_at(index, failure.message),
                    None => call.error_at(0, failure.message),
                });
            }
        }
    }
    let mut args = vec![call.arg(0)];
    args.extend((values.into_iter().zip(&types)).map(|(value, ty)| canonical(value, ty)));
    Ok(FORMAT.call(args))
}

/// A value of a scalar type in the Arrow type that the printer reads: an integer of any width as
/// an `INT64`, a float as a `FLOAT64`, and a string as a `Utf8`.
fn canonical(value: Expr, ty: &Ty) -> Expr {
    match ty {
        Ty::Int64 | Ty::Float64 | Ty::String | Ty::Bytes | Ty::Datetime | Ty::Timestamp => {
            super::cast(value, ty.arrow(Default::default()))
        }
        _ => value,
    }
}

/// A failure of a pattern for the types of its values: its message, and the index of the value
/// that fails it, or none for the pattern itself.
#[derive(Debug)]
struct Failure {
    argument: Option<usize>,
    message: String,
}

impl Failure {
    fn pattern(message: String) -> Failure {
        Failure {
            argument: None,
            message,
        }
    }

    /// A value of the type `ty` at `index` where the element expects `expected`, with GoogleSQL's
    /// message, which counts the pattern as argument 1.
    fn ty(index: usize, expected: &str, ty: &Ty) -> Failure {
        Failure {
            argument: Some(index),
            message: format!(
                "Invalid type for argument {} to FORMAT; Expected {expected}; Got {ty}",
                index + 2
            ),
        }
    }
}

/// How an element prints its value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Print {
    /// A number: an `INT64`, a `FLOAT64`, a `NUMERIC`, or a `BIGNUMERIC`.
    Number,
    /// `%s`: a string, up to a NUL character, as GoogleSQL's.
    String,
    /// `%t`: the text of any value.
    Text,
    /// `%T`: the text of a literal of any value.
    Literal,
}

/// A step of a pattern: literal text, or an element with the indices of its values.
#[derive(Clone, Debug)]
enum Step {
    Text(String),
    Element {
        element: Element,
        width: Option<usize>,
        precision: Option<usize>,
        value: usize,
        print: Print,
    },
}

/// The steps of the pattern `pattern` for values of the types `types`, or GoogleSQL's error,
/// the first in the pattern's order.
fn plan(pattern: &str, types: &[Ty]) -> Result<Vec<Step>, Failure> {
    let invalid = || Failure::pattern(format!("Invalid FORMAT string: \"{pattern}\""));
    if pattern.len() as u64 > MAX_WIDTH {
        return Err(Failure::pattern(too_long()));
    }
    let pieces = pieces(pattern).map_err(|_| invalid())?;
    let integer = |index: usize| match &types[index] {
        Ty::Int64 | Ty::Null => Ok(()),
        ty => Err(Failure::ty(index, "integer", ty)),
    };
    let mut steps = Vec::with_capacity(pieces.len());
    let mut next = 0;
    for piece in pieces {
        let element = match piece {
            Piece::Text(text) => {
                steps.push(Step::Text(text));
                continue;
            }
            Piece::Element(element) => element,
        };
        if element.specifier == '%' {
            return Err(invalid());
        }
        let mut width = None;
        match element.width {
            Some(Count::Argument) => {
                if next < types.len() {
                    integer(next)?;
                    width = Some(next);
                }
                next += 1;
            }
            Some(Count::Written(written)) if written > MAX_WIDTH => {
                return Err(Failure::pattern(format!(
                    "Width {written} exceeds the maximum supported output width of FORMAT. \
                     Limit: {MAX_WIDTH}"
                )));
            }
            _ => {}
        }
        let mut precision = None;
        match element.precision {
            Some(Count::Argument) => {
                if next < types.len() {
                    integer(next)?;
                    precision = Some(next);
                }
                next += 1;
            }
            Some(Count::Written(written)) => {
                check_precision(written, element.specifier).map_err(Failure::pattern)?
            }
            None => {}
        }
        if next < types.len() {
            let ty = &types[next];
            let typed = |expected: &str, takes: fn(&Ty) -> bool| match takes(ty) {
                true => Ok(()),
                false => Err(Failure::ty(next, expected, ty)),
            };
            let print = match element.specifier {
                'd' | 'i' | 'o' | 'x' | 'X' => {
                    typed("integer", |ty| matches!(ty, Ty::Int64 | Ty::Null))?;
                    Print::Number
                }
                'f' | 'F' | 'e' | 'E' | 'g' | 'G' => {
                    typed("FLOAT64", |ty| {
                        matches!(ty, Ty::Float64 | Ty::Numeric | Ty::BigNumeric | Ty::Null)
                    })?;
                    Print::Number
                }
                's' => {
                    typed("STRING", |ty| matches!(ty, Ty::String | Ty::Null))?;
                    Print::String
                }
                // `FORMAT` reads `%u` of an unsigned integer and `%p` and `%P` of a protocol
                // buffer or JSON, types that avenger-sql lacks.
                'u' => return Err(Failure::ty(next, "UINT", ty)),
                'p' | 'P' => return Err(Failure::ty(next, "PROTO or JSON", ty)),
                't' => Print::Text,
                'T' => Print::Literal,
                specifier => {
                    return Err(Failure::pattern(format!(
                        "Invalid format specifier character \"{specifier}\" in FORMAT string: \
                         \"{pattern}\""
                    )))
                }
            };
            steps.push(Step::Element {
                element,
                width,
                precision,
                value: next,
                print,
            });
        }
        next += 1;
    }
    if next != types.len() {
        let few = match next > types.len() {
            true => "few",
            false => "many",
        };
        return Err(Failure::pattern(format!(
            "Too {few} arguments to FORMAT for pattern \"{pattern}\"; Expected {}; Got {}",
            next + 1,
            types.len() + 1
        )));
    }
    Ok(steps)
}

/// The error of a precision over its specifier's limit: 1074 for `%f` and `%e`, none for `%g`,
/// which prints no trailing zeros, and the longest result for the others.
fn check_precision(precision: u64, specifier: char) -> Result<(), String> {
    let limit = match specifier {
        'f' | 'F' | 'e' | 'E' => MAX_FLOAT_PRECISION,
        'g' | 'G' => return Ok(()),
        _ => MAX_WIDTH,
    };
    match precision > limit {
        true => Err(format!(
            "Precision value {precision} is too big; limit {limit}"
        )),
        false => Ok(()),
    }
}

fn too_long() -> String {
    format!("Output string too long while evaluating FORMAT; limit {MAX_WIDTH}")
}

/// The result of `steps` for the values at `row` of `values`, or `None` for `NULL`: a `NULL`
/// value of a number or of `%s`, or of a `*` argument, makes the result `NULL`, and `%t` and `%T`
/// print `NULL`.
fn print(steps: &[Step], values: &[ArrayRef], row: usize) -> Result<Option<String>> {
    let mut out = String::new();
    for step in steps {
        let (element, width, precision, value, print) = match step {
            Step::Text(text) => {
                out.push_str(text);
                continue;
            }
            Step::Element {
                element,
                width,
                precision,
                value,
                print,
            } => (element, width, precision, &values[*value], *print),
        };
        let argument = |index: &Option<usize>| -> Option<Option<i64>> {
            match index {
                None => Some(None),
                Some(index) => {
                    let values = &values[*index];
                    match values.is_null(row) || values.data_type() == &DataType::Null {
                        true => None,
                        false => Some(Some(values.as_primitive::<Int64Type>().value(row))),
                    }
                }
            }
        };
        let (Some(width), Some(mut precision)) = (argument(width), argument(precision)) else {
            return Ok(None);
        };
        if width.is_some_and(|width| width.unsigned_abs() > MAX_WIDTH) {
            return exec_err!("{}", too_long());
        }
        if let Some(written) = precision.filter(|&precision| precision >= 0) {
            if let Err(message) = check_precision(written as u64, element.specifier) {
                return exec_err!("{message}");
            }
        }
        // A larger precision of `%g` prints as 1074 does.
        if matches!(element.specifier, 'g' | 'G') {
            precision = precision.map(|precision| precision.min(MAX_FLOAT_PRECISION as i64));
        }
        let null = value.is_null(row) || value.data_type() == &DataType::Null;
        let text = match print {
            Print::Number if null => return Ok(None),
            Print::Number => {
                let digits;
                let number = match value.data_type() {
                    DataType::Int64 => Value::Integer(value.as_primitive::<Int64Type>().value(row)),
                    DataType::Float64 => {
                        Value::Float(value.as_primitive::<Float64Type>().value(row))
                    }
                    DataType::Decimal128(_, scale) => {
                        let decimal = value.as_primitive::<Decimal128Type>().value(row);
                        digits = decimal.unsigned_abs().to_string();
                        decimal_value(decimal < 0, &digits, *scale)
                    }
                    DataType::Decimal256(_, scale) => {
                        let decimal = value.as_primitive::<Decimal256Type>().value(row);
                        digits = decimal.wrapping_abs().to_string();
                        decimal_value(decimal.is_negative(), &digits, *scale)
                    }
                    other => return exec_err!("FORMAT prints no number of the type {other}"),
                };
                element.format_number(&LOCALE, width, precision, number)
            }
            Print::String if null => return Ok(None),
            Print::String => {
                let string = value.as_string::<i32>().value(row);
                let string = string.split('\0').next().unwrap_or_default();
                element.format_text(width, precision, string)
            }
            Print::Text => element.format_text(width, precision, &text(value, row)?),
            Print::Literal => element.format_text(width, precision, &literal(value, row)?),
        };
        out.push_str(&text);
        if out.len() as u64 > MAX_WIDTH {
            return exec_err!("{}", too_long());
        }
    }
    Ok(Some(out))
}

/// A decimal of the digits of its magnitude and its scale. A negative scale has no fraction.
fn decimal_value(negative: bool, digits: &str, scale: i8) -> Value<'_> {
    Value::Decimal(Decimal {
        negative,
        digits,
        scale: usize::try_from(scale).unwrap_or(0),
    })
}

/// The value at `row` of `array` as an `INT64` or a `FLOAT64`, from an integer or a float of
/// another width, as an element of an array or a field of a struct holds it.
fn widened(array: &ArrayRef, row: usize, to: &DataType) -> Result<ArrayRef> {
    Ok(cast(&array.slice(row, 1), to)?)
}

/// The text of the value at `row` of `array` as `%t` prints it: as GoogleSQL's `CAST(x AS
/// STRING)` writes most values, a timestamp in UTC, a `FLOAT64` with `.0` where it would read
/// as an integer, bytes with escapes, an array in brackets, and a struct in parentheses.
fn text(array: &ArrayRef, row: usize) -> Result<String> {
    if array.is_null(row) {
        return Ok("NULL".to_string());
    }
    Ok(match array.data_type() {
        DataType::Null => "NULL".to_string(),
        DataType::Boolean => array.as_boolean().value(row).to_string(),
        DataType::Int64 => array.as_primitive::<Int64Type>().value(row).to_string(),
        ty if ty.is_integer() => text(&widened(array, row, &DataType::Int64)?, 0)?,
        DataType::Float64 => float_text(array.as_primitive::<Float64Type>().value(row)),
        ty if ty.is_floating() => text(&widened(array, row, &DataType::Float64)?, 0)?,
        DataType::Utf8 => array.as_string::<i32>().value(row).to_string(),
        DataType::LargeUtf8 => array.as_string::<i64>().value(row).to_string(),
        DataType::Utf8View => array.as_string_view().value(row).to_string(),
        DataType::Binary | DataType::LargeBinary | DataType::BinaryView => {
            escape_bytes(binary(array, row)?)
        }
        DataType::Date32 => date_text(array.as_primitive::<Date32Type>().value(row))?,
        DataType::Time64(TimeUnit::Microsecond) => {
            time_text(array.as_primitive::<Time64MicrosecondType>().value(row))
        }
        DataType::Decimal128(..)
        | DataType::Decimal256(..)
        | DataType::Timestamp(..)
        | DataType::Interval(_) => cast_string::text(array, row, Zone::UTC)?,
        DataType::List(_) | DataType::LargeList(_) => {
            let elements = list(array, row);
            let texts: Result<Vec<String>> = (0..elements.len())
                .map(|index| text(&elements, index))
                .collect();
            format!("[{}]", texts?.join(", "))
        }
        DataType::Struct(_) => {
            let fields = array.as_struct().columns().to_vec();
            let texts: Result<Vec<String>> = fields.iter().map(|field| text(field, row)).collect();
            format!("({})", texts?.join(", "))
        }
        other => return exec_err!("FORMAT prints no value of the type {other}"),
    })
}

/// The text of the value at `row` of `array` as `%T` prints it: a literal that reads back as the
/// value, such as `"abc"`, `DATE "2024-01-02"`, and `[1, 2]`.
fn literal(array: &ArrayRef, row: usize) -> Result<String> {
    if array.is_null(row) {
        return Ok("NULL".to_string());
    }
    let typed = |name: &str| -> Result<String> { Ok(format!("{name} \"{}\"", text(array, row)?)) };
    Ok(match array.data_type() {
        DataType::Float64 => {
            let value = array.as_primitive::<Float64Type>().value(row);
            match value.is_finite() {
                true => float_text(value),
                false => format!("CAST(\"{}\" AS FLOAT64)", float_text(value)),
            }
        }
        ty if ty.is_floating() => literal(&widened(array, row, &DataType::Float64)?, 0)?,
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => {
            let string = text(array, row)?;
            quoted(string.as_bytes(), Escape::String)
        }
        DataType::Binary | DataType::LargeBinary | DataType::BinaryView => {
            format!("b{}", quoted(binary(array, row)?, Escape::Bytes))
        }
        DataType::Decimal128(..) => typed("NUMERIC")?,
        DataType::Decimal256(..) => typed("BIGNUMERIC")?,
        DataType::Date32 => typed("DATE")?,
        DataType::Time64(_) => typed("TIME")?,
        DataType::Timestamp(_, None) => typed("DATETIME")?,
        DataType::Timestamp(_, Some(_)) => typed("TIMESTAMP")?,
        DataType::Interval(_) => format!("{} YEAR TO SECOND", typed("INTERVAL")?),
        DataType::List(_) | DataType::LargeList(_) => {
            let elements = list(array, row);
            let literals: Result<Vec<String>> = (0..elements.len())
                .map(|index| literal(&elements, index))
                .collect();
            format!("[{}]", literals?.join(", "))
        }
        DataType::Struct(_) => {
            let fields = array.as_struct().columns().to_vec();
            let literals: Result<Vec<String>> =
                fields.iter().map(|field| literal(field, row)).collect();
            // A struct of one field writes `STRUCT`, since `(x)` reads as `x`.
            match literals?.as_slice() {
                [field] => format!("STRUCT({field})"),
                fields => format!("({})", fields.join(", ")),
            }
        }
        _ => text(array, row)?,
    })
}

/// The elements of the array at `row` of the list array `array`.
fn list(array: &ArrayRef, row: usize) -> ArrayRef {
    match array.data_type() {
        DataType::LargeList(_) => array.as_list::<i64>().value(row),
        _ => array.as_list::<i32>().value(row),
    }
}

/// The bytes at `row` of the binary array `array`.
fn binary(array: &ArrayRef, row: usize) -> Result<&[u8]> {
    Ok(match array.data_type() {
        DataType::Binary => array.as_binary::<i32>().value(row),
        DataType::LargeBinary => array.as_binary::<i64>().value(row),
        DataType::BinaryView => array.as_binary_view().value(row),
        other => return exec_err!("{other} is no binary type"),
    })
}

/// A `FLOAT64` as `%t` and `%T` print it: as `CAST(x AS STRING)` writes it, with `.0` where it
/// would read as an integer, and a sign on negative zero.
fn float_text(value: f64) -> String {
    let text = match value == 0.0 && value.is_sign_negative() {
        true => "-0".to_string(),
        false => cast_string::float_text(value),
    };
    match value.is_finite() && !text.contains(['.', 'e']) {
        true => format!("{text}.0"),
        false => text,
    }
}

/// A `DATE` of `days` since 1970-01-01 as GoogleSQL writes it.
fn date_text(days: i32) -> Result<String> {
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).expect("the epoch");
    match epoch.checked_add_signed(chrono::Duration::days(i64::from(days))) {
        Some(date) => Ok(date.format("%Y-%m-%d").to_string()),
        None => exec_err!("a date out of range"),
    }
}

/// A `TIME` of `micros` since midnight as GoogleSQL writes it, with its fraction in 3 or 6
/// digits.
fn time_text(micros: i64) -> String {
    let seconds = micros / 1_000_000;
    format!(
        "{:02}:{:02}:{:02}{}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60,
        cast_string::fraction_text(micros % 1_000_000)
    )
}

/// How a literal escapes its characters.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Escape {
    /// A string: `\n`, `\r`, `\t`, a control character as `\xHH`, and the rest as written.
    String,
    /// Bytes: printable ASCII as written, and every other byte as `\xHH`.
    Bytes,
}

/// A literal of `bytes` in quotes: double quotes, or single quotes where the text holds a double
/// quote and no single quote, with the quote and `\` escaped.
fn quoted(bytes: &[u8], escape: Escape) -> String {
    let quote = match bytes.contains(&b'"') && !bytes.contains(&b'\'') {
        true => '\'',
        false => '"',
    };
    let mut out = String::with_capacity(bytes.len() + 2);
    out.push(quote);
    let text = String::from_utf8_lossy(bytes);
    let chars: Box<dyn Iterator<Item = u32>> = match escape {
        Escape::String => Box::new(text.chars().map(u32::from)),
        Escape::Bytes => Box::new(bytes.iter().map(|&byte| u32::from(byte))),
    };
    for code in chars {
        let c = char::from_u32(code).unwrap_or(char::REPLACEMENT_CHARACTER);
        match c {
            '\\' => out.push_str("\\\\"),
            _ if c == quote => {
                out.push('\\');
                out.push(c);
            }
            '\n' if escape == Escape::String => out.push_str("\\n"),
            '\r' if escape == Escape::String => out.push_str("\\r"),
            '\t' if escape == Escape::String => out.push_str("\\t"),
            ' '..='~' => out.push(c),
            _ if escape == Escape::Bytes || code < 0x20 || code == 0x7f => {
                out.push_str(&format!("\\x{code:02x}"));
            }
            _ => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// Bytes as `%t` prints them: printable ASCII as written, with `\`, `'`, and `"` escaped, and
/// every other byte as `\xHH`.
fn escape_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &byte in bytes {
        match byte {
            b'\\' | b'\'' | b'"' => {
                out.push('\\');
                out.push(char::from(byte));
            }
            b' '..=b'~' => out.push(char::from(byte)),
            _ => out.push_str(&format!("\\x{byte:02x}")),
        }
    }
    out
}

/// `FORMAT` of a pattern and its values, each an argument, which plans each distinct pattern of a
/// batch once.
#[derive(Debug, PartialEq, Eq, Hash)]
struct StringFormat {
    signature: Signature,
}

impl ScalarUDFImpl for StringFormat {
    fn name(&self) -> &str {
        "$format"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::Utf8)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let rows = args.number_rows;
        let arrays: Vec<ArrayRef> = (args.args.iter())
            .map(|arg| arg.clone().into_array(rows))
            .collect::<Result<_>>()?;
        let patterns = cast(&arrays[0], &DataType::Utf8)?;
        let patterns = patterns.as_string::<i32>();
        let values = &arrays[1..];
        let types: Vec<Ty> = (values.iter())
            .map(|value| Ty::of_arrow(value.data_type()).unwrap_or(Ty::Any))
            .collect();
        let mut plans: HashMap<&str, Arc<Vec<Step>>> = HashMap::new();
        let mut out = StringBuilder::new();
        for row in 0..rows {
            if patterns.is_null(row) {
                out.append_null();
                continue;
            }
            let pattern = patterns.value(row);
            let steps = match plans.get(pattern) {
                Some(steps) => steps.clone(),
                None => match plan(pattern, &types) {
                    Ok(steps) => plans.entry(pattern).or_insert(Arc::new(steps)).clone(),
                    Err(failure) => return exec_err!("{}", failure.message),
                },
            };
            match print(&steps, values, row)? {
                Some(text) => out.append_value(text),
                None => out.append_null(),
            }
        }
        columnar(Arc::new(out.finish()), &args.args)
    }
}
