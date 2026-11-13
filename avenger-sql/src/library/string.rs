//! The string functions, and the regular expressions, which follow RE2's syntax as GoogleSQL's
//! do.

use std::sync::{Arc, LazyLock};

use datafusion::{
    arrow::{
        array::{Array, ArrayRef, AsArray, StringArray},
        datatypes::DataType,
    },
    common::{Result, ScalarValue},
    functions::expr_fn as f,
    functions_nested::expr_fn as nested,
    logical_expr::{
        binary_expr, when, ColumnarValue, Expr, Operator, ScalarFunctionArgs, ScalarUDF,
        ScalarUDFImpl, Signature, Volatility,
    },
};

use super::{cast, error::guard, lit_i64, lit_str, null_when, Call, CallError, Function};

pub(super) fn functions() -> Vec<Function> {
    vec![
        Function::scalar("CONCAT", "Joins strings, or bytes, into one.")
            .description("Returns `NULL` when any value is `NULL`.")
            .param("values", "The strings, or the bytes, in order.")
            .form("(values: T...) -> T where T: STRING | BYTES", |call| {
                // `||` returns NULL for a NULL value, as GoogleSQL's CONCAT does, where
                // DataFusion's `concat` skips it.
                let mut values = call.rest().into_iter();
                let first = values
                    .next()
                    .ok_or_else(|| CallError::call("CONCAT needs a value"))?;
                Ok(values.fold(first, |a, b| binary_expr(a, Operator::StringConcat, b)))
            })
            .safe()
            .example("CONCAT('a', 'b', 'c')", "'abc'")
            .example("CONCAT('a', NULL)", "NULL"),
        Function::scalar("LENGTH", "Returns the number of characters of a string.")
            .param("value", "The string.")
            .form("(value: STRING) -> INT64", |call| {
                Ok(cast(f::character_length(call.arg(0)), DataType::Int64))
            })
            .safe()
            .example("LENGTH('héllo')", "5"),
        Function::scalar("LOWER", "Returns a string with its letters in lowercase.")
            .param("value", "The string.")
            .form("(value: STRING) -> STRING", |call| Ok(f::lower(call.arg(0))))
            .safe()
            .example("LOWER('Hello')", "'hello'"),
        Function::scalar("UPPER", "Returns a string with its letters in uppercase.")
            .param("value", "The string.")
            .form("(value: STRING) -> STRING", |call| Ok(f::upper(call.arg(0))))
            .safe()
            .example("UPPER('Hello')", "'HELLO'"),
        substr("SUBSTR", "Returns a part of a string.")
            .example("SUBSTR('apple', 2)", "'pple'")
            .example("SUBSTR('apple', 2, 2)", "'pp'")
            .example("SUBSTR('apple', -3, 2)", "'pl'"),
        substr("SUBSTRING", "Returns a part of a string, as `SUBSTR` does.")
            .example("SUBSTRING('apple', 2, 3)", "'ppl'"),
        trim("TRIM", "Removes characters from the start and the end of a string.", f::btrim)
            .example("TRIM('  a  ')", "'a'")
            .example("TRIM('xxaxx', 'x')", "'a'"),
        trim("LTRIM", "Removes characters from the start of a string.", f::ltrim)
            .example("LTRIM('xxaxx', 'x')", "'axx'"),
        trim("RTRIM", "Removes characters from the end of a string.", f::rtrim)
            .example("RTRIM('xxaxx', 'x')", "'xxa'"),
        Function::scalar("REPLACE", "Replaces every occurrence of a string in another.")
            .param("value", "The string.")
            .param("from_pattern", "The string to replace. An empty string replaces nothing.")
            .param("to_pattern", "The string that takes its place.")
            .form(
                "(value: STRING, from_pattern: STRING, to_pattern: STRING) -> STRING",
                |call| {
                    let (value, from, to) = (call.arg(0), call.arg(1), call.arg(2));
                    // GoogleSQL leaves the value as it is for an empty pattern.
                    Ok(when(from.clone().eq(lit_str("")), value.clone())
                        .otherwise(f::replace(value, from, to))?)
                },
            )
            .safe()
            .example("REPLACE('apple', 'p', 'b')", "'abble'"),
        Function::scalar("STARTS_WITH", "Returns whether a string starts with another.")
            .param("value", "The string.")
            .param("prefix", "The string to look for at the start.")
            .form("(value: STRING, prefix: STRING) -> BOOL", |call| {
                Ok(f::starts_with(call.arg(0), call.arg(1)))
            })
            .safe()
            .example("STARTS_WITH('apple', 'ap')", "TRUE"),
        Function::scalar("ENDS_WITH", "Returns whether a string ends with another.")
            .param("value", "The string.")
            .param("suffix", "The string to look for at the end.")
            .form("(value: STRING, suffix: STRING) -> BOOL", |call| {
                Ok(f::ends_with(call.arg(0), call.arg(1)))
            })
            .safe()
            .example("ENDS_WITH('apple', 'le')", "TRUE"),
        Function::scalar("STRPOS", "Returns the position of the first occurrence of a string in another.")
            .description("Positions count from 1, and the result is 0 when the string doesn't occur.")
            .param("value", "The string to search.")
            .param("search", "The string to find.")
            .form("(value: STRING, search: STRING) -> INT64", |call| {
                Ok(cast(f::strpos(call.arg(0), call.arg(1)), DataType::Int64))
            })
            .safe()
            .example("STRPOS('banana', 'an')", "2")
            .example("STRPOS('apple', 'z')", "0"),
        Function::scalar("SPLIT", "Splits a string into an array at a delimiter.")
            .param("value", "The string.")
            .param("delimiter", "The string between the parts. When omitted, `','`.")
            .form("(value: STRING[, delimiter: STRING]) -> ARRAY<STRING>", |call| {
                let delimiter = call.value(1).cloned().unwrap_or_else(|| lit_str(","));
                let none = Expr::Literal(ScalarValue::Utf8(None), None);
                Ok(nested::string_to_array(call.arg(0), delimiter, none))
            })
            .safe()
            .example("SPLIT('a,b,c')", "['a', 'b', 'c']")
            .example("SPLIT('a b', ' ')", "['a', 'b']"),
        Function::scalar("REGEXP_CONTAINS", "Returns whether a string has a match of a regular expression.")
            .param("value", "The string.")
            .param("regexp", "The regular expression, in RE2's syntax. A match anywhere in the string counts.")
            .form("(value: STRING, regexp: STRING) -> BOOL", |call| {
                check_pattern(call, 1)?;
                Ok(f::regexp_like(call.arg(0), call.arg(1), None))
            })
            .example("REGEXP_CONTAINS('abc123', r'[0-9]')", "TRUE"),
        Function::scalar("REGEXP_EXTRACT", "Returns the first match of a regular expression in a string.")
            .description("Returns `NULL` when the string has no match.")
            .param("value", "The string.")
            .param("regexp", "The regular expression, in RE2's syntax, with at most one capturing group. With a group, the result is the group's match, and without one, the whole match.")
            .form("(value: STRING, regexp: STRING) -> STRING", |call| {
                if let Some(groups) = check_pattern(call, 1)? {
                    if groups > 1 {
                        return Err(call.error_at(
                            1,
                            "the pattern of REGEXP_EXTRACT has at most one capturing group",
                        ));
                    }
                }
                let matched = f::regexp_match(call.arg(0), call.arg(1), None);
                Ok(nested::array_element(matched, lit_i64(1)))
            })
            .example("REGEXP_EXTRACT('abc123', r'[0-9]+')", "'123'")
            .example("REGEXP_EXTRACT('user@example.com', r'@(.+)')", "'example.com'"),
        Function::scalar("REGEXP_REPLACE", "Replaces every match of a regular expression in a string.")
            .param("value", "The string.")
            .param("regexp", "The regular expression, in RE2's syntax.")
            .param("replacement", "The string that takes the place of each match, where `\\1` to `\\9` stand for the matches of the capturing groups.")
            .form("(value: STRING, regexp: STRING, replacement: STRING) -> STRING", |call| {
                check_pattern(call, 1)?;
                // GoogleSQL replaces every match.
                Ok(f::regexp_replace(
                    call.arg(0),
                    call.arg(1),
                    call.arg(2),
                    Some(lit_str("g")),
                ))
            })
            .example("REGEXP_REPLACE('a1b2', r'[0-9]', '#')", "'a#b#'")
            .example("REGEXP_REPLACE('abc', r'(b)', r'<\\1>')", "'a<b>c'"),
        Function::scalar("REPEAT", "Returns a string repeated a number of times.")
            .param("original_value", "The string.")
            .param("repetitions", "The number of times, which is not negative.")
            .form("(original_value: STRING, repetitions: INT64) -> STRING", |call| {
                let (value, count) = (call.arg(0), call.arg(1));
                let message = "Second argument (repeat count) for REPEAT cannot be negative";
                let invalid = count.clone().lt(lit_i64(0));
                Ok(guard(call, invalid, f::repeat(value, count), message, Vec::new()))
            })
            .safe()
            .example("REPEAT('ab', 3)", "'ababab'")
            .example("REPEAT('ab', 0)", "''"),
        pad("LPAD", "Pads a string at its start to a length, or cuts it to the length.", f::lpad)
            .example("LPAD('abc', 6)", "'   abc'")
            .example("LPAD('abc', 6, 'xy')", "'xyxabc'")
            .example("LPAD('abc', 2)", "'ab'"),
        pad("RPAD", "Pads a string at its end to a length, or cuts it to the length.", f::rpad)
            .example("RPAD('abc', 6, 'xy')", "'abcxyx'")
            .example("RPAD('abc', 1)", "'a'"),
        Function::scalar("REVERSE", "Returns the characters of a string in reverse order.")
            .param("value", "The string.")
            .form("(value: STRING) -> STRING", |call| Ok(f::reverse(call.arg(0))))
            .safe()
            .example("REVERSE('abcé')", "'écba'"),
        part("LEFT", "Returns the first characters of a string.", f::left)
            .example("LEFT('abcdef', 3)", "'abc'")
            .example("LEFT('abc', 10)", "'abc'"),
        part("RIGHT", "Returns the last characters of a string.", f::right)
            .example("RIGHT('abcdef', 2)", "'ef'"),
        Function::scalar("INITCAP", "Returns a string with the first character of each word in upper case and the others in lower case.")
            .description("A word starts at the start of the string and after each delimiter. The delimiters stay as they are, and a character that isn't a letter, such as a digit, starts a word too.")
            .param("value", "The string.")
            .param("delimiters", "The characters that separate the words, each on its own. When omitted, they are whitespace and ``!\"#$%&()*+,-./:;<>?@[\\]^_{|}~``.")
            .form("(value: STRING[, delimiters: STRING]) -> STRING", |call| {
                let mut args = vec![call.arg(0)];
                args.extend(call.value(1).cloned());
                Ok(INITCAP.call(args))
            })
            .safe()
            .example("INITCAP('hello wORLD')", "'Hello World'")
            .example("INITCAP(\"o'neil-smith 1st\")", "'O\\'neil-Smith 1st'")
            .example("INITCAP('a.b_c d', '.')", "'A.B_c d'"),
    ]
}

/// `LPAD` or `RPAD`, whose pattern, a space when omitted, repeats to the length.
fn pad(name: &str, summary: &str, function: fn(Vec<Expr>) -> Expr) -> Function {
    Function::scalar(name, summary)
        .param("original_value", "The string.")
        .param("return_length", "The length of the result, in characters, which is not negative. A shorter length cuts the string.")
        .param("pattern", "The string that pads, which is not empty. When omitted, a space.")
        .form("(original_value: STRING, return_length: INT64[, pattern: STRING]) -> STRING", move |call| {
            let length = call.arg(1);
            let mut args = vec![call.arg(0), length.clone()];
            args.extend(call.value(2).cloned());
            let mut value = function(args);
            if let Some(pattern) = call.value(2) {
                let empty = pattern.clone().eq(lit_str(""));
                let message = "Third argument (pad pattern) for LPAD/RPAD cannot be empty";
                value = guard(call, empty, value, message, Vec::new());
            }
            let message = "Second argument (output size) for LPAD/RPAD cannot be negative";
            Ok(guard(call, length.lt(lit_i64(0)), value, message, Vec::new()))
        })
        .safe()
}

/// `LEFT` or `RIGHT`, of a length that is not negative.
fn part(name: &'static str, summary: &str, function: fn(Expr, Expr) -> Expr) -> Function {
    Function::scalar(name, summary)
        .param("value", "The string.")
        .param("length", "The number of characters, which is not negative. A length beyond the string's gives the whole string.")
        .form("(value: STRING, length: INT64) -> STRING", move |call| {
            let length = call.arg(1);
            let message = format!("Second argument in {name}() cannot be negative");
            let value = function(call.arg(0), length.clone());
            Ok(guard(call, length.lt(lit_i64(0)), value, &message, Vec::new()))
        })
        .safe()
}

/// GoogleSQL's delimiters of the words of `INITCAP`, when a call gives none.
const DELIMITERS: &str = " \t\n\u{b}\u{c}\r!\"#$%&()*+,-./:;<>?@[\\]^_{|}~\u{85}\u{a0}";

/// `value` with the first character of each word in upper case and the others in lower case,
/// where a word starts at the start and after each of `delimiters`.
fn initcap(value: &str, delimiters: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut start = true;
    for c in value.chars() {
        if delimiters.contains(c) {
            out.push(c);
            start = true;
            continue;
        }
        match start {
            true => out.extend(c.to_uppercase()),
            false => out.extend(c.to_lowercase()),
        }
        start = false;
    }
    out
}

static INITCAP: LazyLock<Arc<ScalarUDF>> = LazyLock::new(|| {
    Arc::new(ScalarUDF::new_from_impl(Initcap {
        signature: Signature::variadic_any(Volatility::Immutable),
    }))
});

/// GoogleSQL's `INITCAP`, whose words differ from DataFusion's `initcap`, which also capitalizes
/// after `'` and takes no delimiters.
#[derive(Debug, PartialEq, Eq, Hash)]
struct Initcap {
    signature: Signature,
}

impl ScalarUDFImpl for Initcap {
    fn name(&self) -> &str {
        "$initcap"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::Utf8)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let rows = args.number_rows;
        let scalar = args
            .args
            .iter()
            .all(|arg| matches!(arg, ColumnarValue::Scalar(_)));
        let strings = |arg: &ColumnarValue| -> Result<ArrayRef> {
            datafusion::arrow::compute::cast(&arg.clone().into_array(rows)?, &DataType::Utf8)
                .map_err(Into::into)
        };
        let values = strings(&args.args[0])?;
        let values = values.as_string::<i32>();
        let delimiters = args.args.get(1).map(strings).transpose()?;
        let delimiters = delimiters.as_ref().map(|d| d.as_string::<i32>());
        let out: StringArray = (0..values.len())
            .map(|row| {
                let delimiters = match delimiters {
                    Some(d) if d.is_null(if d.len() == 1 { 0 } else { row }) => return None,
                    Some(d) => d.value(if d.len() == 1 { 0 } else { row }),
                    None => DELIMITERS,
                };
                (!values.is_null(row)).then(|| initcap(values.value(row), delimiters))
            })
            .collect();
        let out: ArrayRef = Arc::new(out);
        match scalar {
            true => Ok(ColumnarValue::Scalar(ScalarValue::try_from_array(&out, 0)?)),
            false => Ok(ColumnarValue::Array(out)),
        }
    }
}

/// `SUBSTR(value, position [, length])`, whose position counts from 1, from the end when it is
/// negative, and is 1 when it is 0.
fn substr(name: &str, summary: &str) -> Function {
    Function::scalar(name, summary)
        .param("value", "The string.")
        .param("position", "Where the part starts, from 1. A negative position counts from the end, with -1 for the last character, and 0 or a position before the start is 1.")
        .param("length", "The most characters that the part holds. A negative length is an error. When omitted, the part runs to the end of the string.")
        .form("(value: STRING, position: INT64[, length: INT64]) -> STRING", |call| {
            let value = call.arg(0);
            let position = call.arg(1);
            let size = cast(f::character_length(value.clone()), DataType::Int64);
            let start = when(position.clone().gt(lit_i64(0)), position.clone())
                .when(position.clone().eq(lit_i64(0)), lit_i64(1))
                .otherwise(f::greatest(vec![size + position + lit_i64(1), lit_i64(1)]))?;
            let Some(length) = call.value(2).cloned() else {
                return Ok(f::substr(value, start));
            };
            // DataFusion's substring fails on a negative length, so it reads `NULL` there.
            let negative = length.clone().lt(lit_i64(0));
            let result = f::substring(value, start, null_when(negative, length.clone()));
            // GoogleSQL names SUBSTRING `SUBSTR` in its message.
            let message = "Third argument in SUBSTR() cannot be negative";
            Ok(guard(call, length.lt(lit_i64(0)), result, message, Vec::new()))
        })
        .safe()
}

fn trim(name: &str, summary: &str, function: fn(Vec<Expr>) -> Expr) -> Function {
    Function::scalar(name, summary)
        .param("value", "The string.")
        .param(
            "characters",
            "The characters to remove, each on its own. When omitted, whitespace.",
        )
        .form(
            "(value: STRING[, characters: STRING]) -> STRING",
            move |call| {
                let mut args = vec![call.arg(0)];
                args.extend(call.value(1).cloned());
                Ok(function(args))
            },
        )
        .safe()
}

/// The number of capturing groups of the pattern at `index`, when it is a literal, or the error
/// of a pattern that doesn't compile.
fn check_pattern(call: &Call, index: usize) -> Result<Option<usize>, CallError> {
    let Some(Expr::Literal(value, _)) = call.value(index) else {
        return Ok(None);
    };
    let Some(Some(pattern)) = value.try_as_str() else {
        return Ok(None);
    };
    match regex::Regex::new(pattern) {
        Ok(regex) => Ok(Some(regex.captures_len() - 1)),
        Err(error) => Err(call.error_at(index, format!("the pattern doesn't compile: {error}"))),
    }
}
