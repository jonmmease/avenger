//! GoogleSQL's bitwise operators on `BYTES`: `~` of each byte; `&`, `|` and `^` of the bytes of
//! two values of equal length, which fail on lengths that differ; and `<<` and `>>` of the bits of
//! the bytes as one big-endian number, which keep the length and fail on a negative offset.

use std::sync::{Arc, LazyLock};

use datafusion::{
    arrow::{
        array::{Array, ArrayRef, AsArray, BinaryArray},
        compute::cast,
        datatypes::{DataType, Int64Type},
    },
    common::{exec_err, Result, ScalarValue},
    logical_expr::{
        ColumnarValue, Expr, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Volatility,
    },
};

use super::lit_str;

/// The bitwise operator `op` (`~`, `&`, `|`, `^`, `<<` or `>>`) of the bytes `x` and, but for
/// `~`, `y`: bytes, or an `INT64` offset for a shift.
pub(crate) fn bytes_bitwise(op: &str, x: Expr, y: Option<Expr>) -> Expr {
    let mut args = vec![lit_str(op), x];
    args.extend(y);
    BYTES_BITWISE.call(args)
}

static BYTES_BITWISE: LazyLock<Arc<ScalarUDF>> = LazyLock::new(|| {
    Arc::new(ScalarUDF::new_from_impl(BytesBitwise {
        signature: Signature::variadic_any(Volatility::Immutable),
    }))
});

#[derive(Debug, PartialEq, Eq, Hash)]
struct BytesBitwise {
    signature: Signature,
}

/// `bytes` shifted by `offset` bits, to the left when `left`, as one big-endian number of their
/// length.
fn shifted(bytes: &[u8], offset: usize, left: bool) -> Vec<u8> {
    let bits = bytes.len() * 8;
    let mut out = vec![0u8; bytes.len()];
    for to in 0..bits {
        let from = match left {
            true => to.checked_add(offset).filter(|&from| from < bits),
            false => to.checked_sub(offset),
        };
        if let Some(from) = from {
            if bytes[from / 8] & (0x80 >> (from % 8)) != 0 {
                out[to / 8] |= 0x80 >> (to % 8);
            }
        }
    }
    out
}

impl ScalarUDFImpl for BytesBitwise {
    fn name(&self) -> &str {
        "$bytes_bitwise"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::Binary)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let rows = args.number_rows;
        let ColumnarValue::Scalar(op) = &args.args[0] else {
            return exec_err!("the operator of $bytes_bitwise is a literal");
        };
        let op = op.try_as_str().flatten().unwrap_or_default().to_string();
        let scalar = (args.args[1..].iter()).all(|arg| matches!(arg, ColumnarValue::Scalar(_)));
        let x = cast(&args.args[1].clone().into_array(rows)?, &DataType::Binary)?;
        let x = x.as_binary::<i32>();
        let y = args
            .args
            .get(2)
            .map(|y| y.clone().into_array(rows))
            .transpose()?;
        let mut out: Vec<Option<Vec<u8>>> = Vec::with_capacity(x.len());
        for row in 0..x.len() {
            let pick = |array: &ArrayRef| if array.len() == 1 { 0 } else { row };
            if x.is_null(row) || y.as_ref().is_some_and(|y| y.is_null(pick(y))) {
                out.push(None);
                continue;
            }
            let a = x.value(row);
            let value = match op.as_str() {
                "~" => a.iter().map(|byte| !byte).collect(),
                "<<" | ">>" => {
                    let y = y.as_ref().expect("an offset");
                    let y = cast(y, &DataType::Int64)?;
                    let offset = y.as_primitive::<Int64Type>().value(pick(&y));
                    if offset < 0 {
                        return exec_err!("Bitwise shift by negative offset.");
                    }
                    shifted(a, offset as usize, op == "<<")
                }
                _ => {
                    let y = cast(y.as_ref().expect("a value"), &DataType::Binary)?;
                    let b = y.as_binary::<i32>().value(pick(&y));
                    if a.len() != b.len() {
                        return exec_err!(
                            "Bitwise binary operator for BYTES requires equal length of the inputs. Got {} bytes on the left hand side and {} bytes on the right hand side.",
                            a.len(),
                            b.len()
                        );
                    }
                    let combine: fn(u8, u8) -> u8 = match op.as_str() {
                        "&" => |a, b| a & b,
                        "|" => |a, b| a | b,
                        _ => |a, b| a ^ b,
                    };
                    a.iter().zip(b).map(|(a, b)| combine(*a, *b)).collect()
                }
            };
            out.push(Some(value));
        }
        let out: ArrayRef = Arc::new(BinaryArray::from_iter(out));
        match scalar {
            true => Ok(ColumnarValue::Scalar(ScalarValue::try_from_array(&out, 0)?)),
            false => Ok(ColumnarValue::Array(out)),
        }
    }
}
