//! The regular expressions of GoogleSQL's functions, which run RE2, over the `regex` crate. The
//! two engines share their model, leftmost-first matching without backtracking, and most of their
//! syntax, so a pattern runs as the `regex` pattern of the same meaning:
//!
//! - The Perl classes and the word boundaries are ASCII in RE2 and Unicode in `regex`. `\d` and
//!   `\w` become the ASCII classes `[[:digit:]]` and `[[:word:]]`, `\s` the class of RE2's white
//!   space, `[\t\n\f\r ]`, which holds no vertical tab, unlike `regex`'s ASCII `\s`, and `\b` and
//!   `\B` become `(?-u:\b)` and `(?-u:\B)`.
//! - The rewrite string of `REGEXP_REPLACE` is RE2's: `\0` to `\9` are the matches of the groups,
//!   `\\` is a backslash, and every other character is itself, `$` included. Any other escape, and
//!   a group that the pattern lacks, is an error.
//!
//! The functions compile each distinct pattern once, so a pattern from a column, which differs
//! from row to row, runs as a literal does.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use datafusion::{
    arrow::{
        array::{Array, ArrayRef, AsArray, BooleanArray, StringArray},
        compute::cast,
        datatypes::DataType,
    },
    common::{exec_err, Result},
    logical_expr::{
        ColumnarValue, Expr, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Volatility,
    },
};
use regex::Regex;

mod syntax;

/// A pattern of RE2, compiled: the `regex` of its meaning and its number of capturing groups.
#[derive(Clone, Debug)]
pub(crate) struct Re2 {
    pub regex: Regex,
    pub groups: usize,
}

/// The pattern `pattern` of RE2, compiled, or the error that keeps it from compiling.
pub(crate) fn compile(pattern: &str) -> std::result::Result<Re2, String> {
    let translated = syntax::translate(pattern)?;
    let regex = Regex::new(&translated).map_err(|error| error.to_string())?;
    Ok(Re2 {
        groups: regex.captures_len() - 1,
        regex,
    })
}

/// A piece of a rewrite string: text, or the match of a group.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Piece {
    Text(String),
    Group(usize),
}

/// The rewrite string `rewrite` of `REGEXP_REPLACE`, for a pattern of `groups` capturing groups,
/// or its error.
pub(crate) fn check_rewrite(rewrite: &str, groups: usize) -> std::result::Result<(), String> {
    pieces(rewrite, groups).map(drop)
}

fn pieces(rewrite: &str, groups: usize) -> std::result::Result<Vec<Piece>, String> {
    let mut pieces = Vec::new();
    let mut text = String::new();
    let mut chars = rewrite.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            text.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => text.push('\\'),
            Some(digit @ '0'..='9') => {
                let group = digit as usize - '0' as usize;
                if group > groups {
                    return Err(format!(
                        "the replacement reads group {group}, and the pattern has {groups}"
                    ));
                }
                if !text.is_empty() {
                    pieces.push(Piece::Text(std::mem::take(&mut text)));
                }
                pieces.push(Piece::Group(group));
            }
            Some(other) => {
                return Err(format!(
                    "the replacement has the escape `\\{other}`; its escapes are `\\0` to `\\9` and `\\\\`"
                ))
            }
            None => return Err("the replacement ends in a lone backslash".into()),
        }
    }
    if !text.is_empty() {
        pieces.push(Piece::Text(text));
    }
    Ok(pieces)
}

/// The compiled patterns of a call, by their text, each compiled once.
#[derive(Default)]
struct Patterns(HashMap<String, Arc<Re2>>);

impl Patterns {
    fn get(&mut self, pattern: &str) -> Result<Arc<Re2>> {
        if let Some(compiled) = self.0.get(pattern) {
            return Ok(compiled.clone());
        }
        match compile(pattern) {
            Ok(compiled) => {
                let compiled = Arc::new(compiled);
                self.0.insert(pattern.to_string(), compiled.clone());
                Ok(compiled)
            }
            Err(error) => exec_err!("the pattern `{pattern}` doesn't compile: {error}"),
        }
    }
}

/// The argument at `index` of `args`, as strings of each row.
fn strings(args: &ScalarFunctionArgs, index: usize) -> Result<ArrayRef> {
    let array = args.args[index].clone().into_array(args.number_rows)?;
    Ok(cast(&array, &DataType::Utf8)?)
}

/// Which of GoogleSQL's regex functions a function is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Kind {
    Contains,
    Extract,
    Replace,
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct Re2Function {
    kind: Kind,
    signature: Signature,
}

impl Re2Function {
    fn udf(kind: Kind, arguments: usize) -> Arc<ScalarUDF> {
        Arc::new(ScalarUDF::new_from_impl(Re2Function {
            kind,
            signature: Signature::any(arguments, Volatility::Immutable),
        }))
    }
}

pub(in crate::library) static CONTAINS: LazyLock<Arc<ScalarUDF>> =
    LazyLock::new(|| Re2Function::udf(Kind::Contains, 2));
pub(in crate::library) static EXTRACT: LazyLock<Arc<ScalarUDF>> =
    LazyLock::new(|| Re2Function::udf(Kind::Extract, 2));
pub(in crate::library) static REPLACE: LazyLock<Arc<ScalarUDF>> =
    LazyLock::new(|| Re2Function::udf(Kind::Replace, 3));

/// `REGEXP_CONTAINS(value, regexp)`: whether `value` has a match of `regexp`.
pub(crate) fn contains(value: Expr, regexp: Expr) -> Expr {
    CONTAINS.call(vec![value, regexp])
}

/// `REGEXP_EXTRACT(value, regexp)`: the first match of `regexp` in `value`, or its group.
pub(crate) fn extract(value: Expr, regexp: Expr) -> Expr {
    EXTRACT.call(vec![value, regexp])
}

/// `REGEXP_REPLACE(value, regexp, replacement)`: `value` with each match of `regexp` replaced.
pub(crate) fn replace(value: Expr, regexp: Expr, replacement: Expr) -> Expr {
    REPLACE.call(vec![value, regexp, replacement])
}

impl ScalarUDFImpl for Re2Function {
    fn name(&self) -> &str {
        match self.kind {
            Kind::Contains => "$re2_contains",
            Kind::Extract => "$re2_extract",
            Kind::Replace => "$re2_replace",
        }
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(match self.kind {
            Kind::Contains => DataType::Boolean,
            Kind::Extract | Kind::Replace => DataType::Utf8,
        })
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let values = strings(&args, 0)?;
        let values = values.as_string::<i32>();
        let regexps = strings(&args, 1)?;
        let regexps = regexps.as_string::<i32>();
        let mut patterns = Patterns::default();
        let array: ArrayRef = match self.kind {
            Kind::Contains => {
                let mut out = Vec::with_capacity(values.len());
                for row in 0..values.len() {
                    out.push(match values.is_null(row) || regexps.is_null(row) {
                        true => None,
                        false => {
                            let re2 = patterns.get(regexps.value(row))?;
                            Some(re2.regex.is_match(values.value(row)))
                        }
                    });
                }
                Arc::new(BooleanArray::from(out))
            }
            Kind::Extract => {
                let mut out: Vec<Option<String>> = Vec::with_capacity(values.len());
                for row in 0..values.len() {
                    if values.is_null(row) || regexps.is_null(row) {
                        out.push(None);
                        continue;
                    }
                    let re2 = patterns.get(regexps.value(row))?;
                    if re2.groups > 1 {
                        return exec_err!(
                            "the pattern of REGEXP_EXTRACT has at most one capturing group"
                        );
                    }
                    let found = (re2.regex.captures(values.value(row)))
                        .and_then(|captures| captures.get(re2.groups))
                        .map(|matched| matched.as_str().to_string());
                    out.push(found);
                }
                Arc::new(StringArray::from(out))
            }
            Kind::Replace => {
                let rewrites = strings(&args, 2)?;
                let rewrites = rewrites.as_string::<i32>();
                let mut out: Vec<Option<String>> = Vec::with_capacity(values.len());
                for row in 0..values.len() {
                    if values.is_null(row) || regexps.is_null(row) || rewrites.is_null(row) {
                        out.push(None);
                        continue;
                    }
                    let re2 = patterns.get(regexps.value(row))?;
                    let pieces = match pieces(rewrites.value(row), re2.groups) {
                        Ok(pieces) => pieces,
                        Err(error) => return exec_err!("{error}"),
                    };
                    let replaced =
                        re2.regex
                            .replace_all(values.value(row), |captures: &regex::Captures| {
                                let mut text = String::new();
                                for piece in &pieces {
                                    match piece {
                                        Piece::Text(piece) => text.push_str(piece),
                                        Piece::Group(group) => {
                                            if let Some(matched) = captures.get(*group) {
                                                text.push_str(matched.as_str());
                                            }
                                        }
                                    }
                                }
                                text
                            });
                    out.push(Some(replaced.into_owned()));
                }
                Arc::new(StringArray::from(out))
            }
        };
        Ok(ColumnarValue::Array(array))
    }
}
