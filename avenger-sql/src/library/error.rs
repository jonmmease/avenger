//! GoogleSQL's errors at run time: `$error_when(invalid, value, message, args...)` returns `value`,
//! and fails with `message` where `invalid` holds, as GoogleSQL's functions fail on an argument
//! out of their domain, such as `SQRT(-1)`. The message takes the text of each argument after it
//! at a `{}`, from the first row that fails, as GoogleSQL writes them.
//!
//! DataFusion's constant folding leaves a call that fails in place, so the error happens when the
//! query runs, and a branch of a `CASE` that no row takes doesn't fail.

use std::sync::{Arc, LazyLock};

use datafusion::{
    arrow::{
        array::{Array, ArrayRef, AsArray},
        datatypes::DataType,
    },
    common::{exec_err, Result, ScalarValue},
    logical_expr::{
        ColumnarValue, Expr, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Volatility,
    },
};

use super::{lit_str, null_when, Call};

/// `value`, and where `invalid` holds, the error `message` for a call, or `NULL` for its `SAFE.`
/// form, as GoogleSQL's functions fail on an argument out of their domain.
pub(crate) fn guard(
    call: &Call,
    invalid: Expr,
    value: Expr,
    message: &str,
    args: Vec<Expr>,
) -> Expr {
    match call.safe {
        true => null_when(invalid, value),
        false => error_when(invalid, value, message, args),
    }
}

/// `value`, or the error `message` where `invalid` holds, with the text of each of `args` at a
/// `{}` of `message`.
pub(crate) fn error_when(invalid: Expr, value: Expr, message: &str, args: Vec<Expr>) -> Expr {
    let mut call = vec![invalid, value, lit_str(message)];
    call.extend(args);
    ERROR_WHEN.call(call)
}

static ERROR_WHEN: LazyLock<Arc<ScalarUDF>> = LazyLock::new(|| {
    Arc::new(ScalarUDF::new_from_impl(ErrorWhen {
        signature: Signature::variadic_any(Volatility::Immutable),
    }))
});

#[derive(Debug, PartialEq, Eq, Hash)]
struct ErrorWhen {
    signature: Signature,
}

/// The text of the value at `row` of `array`, as GoogleSQL writes an argument in a message: a
/// float without a fraction as an integer, a decimal without trailing zeros, and a string as it
/// is.
fn text(array: &ArrayRef, row: usize) -> String {
    if array.is_null(row) {
        return "NULL".to_string();
    }
    match ScalarValue::try_from_array(array, row) {
        Ok(ScalarValue::Float64(Some(value))) if value.fract() == 0.0 && value.abs() < 1e15 => {
            format!("{}", value as i64)
        }
        Ok(ScalarValue::Float64(Some(value))) => format!("{value}"),
        Ok(ScalarValue::Utf8(Some(s)) | ScalarValue::Utf8View(Some(s)))
        | Ok(ScalarValue::LargeUtf8(Some(s))) => s,
        // A decimal without the zeros of its scale, as GoogleSQL writes a `NUMERIC`.
        Ok(value @ (ScalarValue::Decimal128(..) | ScalarValue::Decimal256(..))) => {
            let text = value.to_string();
            match text.contains('.') {
                true => text.trim_end_matches('0').trim_end_matches('.').to_string(),
                false => text,
            }
        }
        Ok(value) => value.to_string(),
        Err(_) => "?".to_string(),
    }
}

impl ScalarUDFImpl for ErrorWhen {
    fn name(&self) -> &str {
        "$error_when"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, arg_types: &[DataType]) -> Result<DataType> {
        Ok(arg_types[1].clone())
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let rows = args.number_rows;
        let invalid = args.args[0].clone().into_array(rows)?;
        let invalid = invalid.as_boolean();
        if let Some(row) =
            (0..invalid.len()).find(|&row| invalid.is_valid(row) && invalid.value(row))
        {
            let ColumnarValue::Scalar(message) = &args.args[2] else {
                return exec_err!("the message of $error_when is a literal");
            };
            let Some(Some(message)) = message.try_as_str() else {
                return exec_err!("the message of $error_when is a string");
            };
            let mut message = message.to_string();
            for arg in &args.args[3..] {
                let array = arg.clone().into_array(rows)?;
                let value = text(&array, if array.len() == 1 { 0 } else { row });
                message = message.replacen("{}", &value, 1);
            }
            return exec_err!("{message}");
        }
        Ok(args.args[1].clone())
    }
}
