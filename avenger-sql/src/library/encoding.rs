//! The text of bytes in hexadecimal digits and in base64, as GoogleSQL's `TO_HEX`, `FROM_HEX`,
//! `TO_BASE64`, and `FROM_BASE64` write and read it, with GoogleSQL's errors for text outside the
//! encoding.

use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{Array, ArrayRef, AsArray, BinaryBuilder, StringBuilder},
        compute::cast,
        datatypes::DataType,
    },
    common::{exec_err, Result},
    logical_expr::{
        ColumnarValue, Expr, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Volatility,
    },
};

use super::columnar;

/// An encoding of bytes as text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Encoding {
    /// Two hexadecimal digits for each byte, in lower case.
    Hex,
    /// Base64, as RFC 4648 defines it.
    Base64,
}

/// The text of the bytes `value` in `encoding`, as `TO_HEX` and `TO_BASE64` give it.
pub(crate) fn encoded(value: Expr, encoding: Encoding) -> Expr {
    let udf = Encode {
        encoding,
        signature: Signature::any(1, Volatility::Immutable),
    };
    ScalarUDF::new_from_impl(udf).call(vec![value])
}

/// The bytes of the text `value` in `encoding`, as `FROM_HEX` and `FROM_BASE64` give them, with
/// GoogleSQL's error for text outside the encoding, or `NULL` where `safe`.
pub(crate) fn decoded(value: Expr, encoding: Encoding, safe: bool) -> Expr {
    let udf = Decode {
        encoding,
        safe,
        signature: Signature::any(1, Volatility::Immutable),
    };
    ScalarUDF::new_from_impl(udf).call(vec![value])
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// The text of `bytes` in `encoding`.
fn encode(bytes: &[u8], encoding: Encoding) -> String {
    match encoding {
        Encoding::Hex => bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
        Encoding::Base64 => {
            let mut out = String::new();
            for chunk in bytes.chunks(3) {
                let value = (chunk.iter().enumerate()).fold(0u32, |value, (i, byte)| {
                    value | u32::from(*byte) << (16 - 8 * i)
                });
                for index in 0..4 {
                    match index <= chunk.len() {
                        true => out.push(BASE64[(value >> (18 - 6 * index) & 63) as usize] as char),
                        false => out.push('='),
                    }
                }
            }
            out
        }
    }
}

/// The bytes of `text` in `encoding`, or GoogleSQL's message.
fn decode(text: &str, encoding: Encoding) -> Result<Vec<u8>, String> {
    match encoding {
        Encoding::Hex => {
            let mut digits = Vec::new();
            for (offset, c) in text.char_indices() {
                match c.to_digit(16) {
                    Some(digit) => digits.push(digit as u8),
                    None => {
                        return Err(format!(
                            "Failed to decode invalid hexadecimal string due to character at offset {offset}: {text}"
                        ))
                    }
                }
            }
            // An odd number of digits starts with a `0`.
            if digits.len() % 2 == 1 {
                digits.insert(0, 0);
            }
            Ok(digits
                .chunks(2)
                .map(|pair| pair[0] << 4 | pair[1])
                .collect())
        }
        Encoding::Base64 => {
            let invalid = || "Failed to decode invalid base64 string".to_string();
            let symbols: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
            let data_len = symbols
                .iter()
                .position(|b| *b == b'=')
                .unwrap_or(symbols.len());
            let padding = symbols.len() - data_len;
            // Padding is optional, and when present completes the last group.
            if symbols[data_len..].iter().any(|b| *b != b'=')
                || (padding > 0 && (padding > 2 || !symbols.len().is_multiple_of(4)))
                || data_len % 4 == 1
            {
                return Err(invalid());
            }
            let mut out = Vec::new();
            for chunk in symbols[..data_len].chunks(4) {
                let mut value = 0u32;
                for (i, symbol) in chunk.iter().enumerate() {
                    let index = BASE64
                        .iter()
                        .position(|b| b == symbol)
                        .ok_or_else(invalid)?;
                    value |= (index as u32) << (18 - 6 * i);
                }
                for i in 0..chunk.len() - 1 {
                    out.push((value >> (16 - 8 * i)) as u8);
                }
            }
            Ok(out)
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct Encode {
    encoding: Encoding,
    signature: Signature,
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct Decode {
    encoding: Encoding,
    safe: bool,
    signature: Signature,
}

impl ScalarUDFImpl for Encode {
    fn name(&self) -> &str {
        match self.encoding {
            Encoding::Hex => "$to_hex",
            Encoding::Base64 => "$to_base64",
        }
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::Utf8)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let bytes = cast(
            &args.args[0].clone().into_array(args.number_rows)?,
            &DataType::Binary,
        )?;
        let bytes = bytes.as_binary::<i32>();
        let mut out = StringBuilder::new();
        for row in 0..bytes.len() {
            match bytes.is_null(row) {
                true => out.append_null(),
                false => out.append_value(encode(bytes.value(row), self.encoding)),
            }
        }
        let out: ArrayRef = Arc::new(out.finish());
        columnar(out, &args.args)
    }
}

impl ScalarUDFImpl for Decode {
    fn name(&self) -> &str {
        match self.encoding {
            Encoding::Hex => "$from_hex",
            Encoding::Base64 => "$from_base64",
        }
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::Binary)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let text = cast(
            &args.args[0].clone().into_array(args.number_rows)?,
            &DataType::Utf8,
        )?;
        let text = text.as_string::<i32>();
        let mut out = BinaryBuilder::new();
        for row in 0..text.len() {
            if text.is_null(row) {
                out.append_null();
                continue;
            }
            match decode(text.value(row), self.encoding) {
                Ok(bytes) => out.append_value(bytes),
                Err(_) if self.safe => out.append_null(),
                Err(message) => return exec_err!("{message}"),
            }
        }
        let out: ArrayRef = Arc::new(out.finish());
        columnar(out, &args.args)
    }
}
