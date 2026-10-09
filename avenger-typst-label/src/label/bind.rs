//! Writing values into markup, for the libraries that compute what labels show.

use std::ops::Range;

use chrono::{Datelike, NaiveDate, NaiveTime, Timelike};
use typst_syntax::{LinkedNode, SyntaxKind};

use super::error::{LabelError, source_error};
use super::values::{LabelValue, LabelValues};
use crate::typst_eval::parse_label;
use crate::typst_library::foundations::Repr;

/// Writes values into a label's markup: the markup typesets as `source` would with each value
/// bound to its name before it, as `#let` would bind it, which labels don't have.
///
/// Each reference to a value's name, `#name` in markup and code or `name` in math, becomes the
/// value written as code, and values shadow the library's names, as bindings do. As in Typst, a
/// single letter in math displays as itself, so `$n$` stays the letter n, and `$#n$` refers to
/// `n`. Values the source doesn't refer to are ignored, and names without a value are left for
/// compilation to report. Errors in the bound markup point into the bound text.
///
/// A source that doesn't parse returns its syntax error, and so does math that calls a value's
/// name, as in `$rate(x)$`, which bind can't write: a space after the name, as in
/// `$rate (x)$`, sets the value before the parentheses.
pub fn bind(source: &str, values: &LabelValues) -> Result<String, LabelError> {
    let root = parse_label(source);
    let (errors, _) = root.errors_and_warnings();
    if let Some(error) = errors.into_iter().next() {
        return Err(source_error(source, &error.into()));
    }

    let mut splices: Vec<(Range<usize>, String)> = vec![];
    let mut stack = vec![LinkedNode::new(&root)];
    while let Some(node) = stack.pop() {
        match node.kind() {
            SyntaxKind::Ident => {
                let Some(value) = values.get(node.leaf_text().as_str()) else { continue };
                splices.push((node.range(), format!("({})", literal(value))));
                // Markup reads a `*` between letters as text, but after the `)` above it would
                // open strong emphasis.
                if let Some(next) = node.next_leaf()
                    && next.offset() == node.range().end
                    && next.kind() == SyntaxKind::Text
                    && next.leaf_text().starts_with('*')
                {
                    splices.push((next.offset()..next.offset(), "\\".into()));
                }
            }
            // A math name, alone or with fields, as in `$pt.x$`, becomes embedded code. Its `;`
            // ends the code, so that what follows keeps its meaning in math.
            SyntaxKind::MathIdent | SyntaxKind::MathFieldAccess => {
                let name = math_target(&node);
                let Some(value) = values.get(name.leaf_text().as_str()) else { continue };
                if is_math_callee(&node) {
                    return Err(LabelError::Source {
                        range: name.range(),
                        message: format!(
                            "cannot bind `{}`, which math calls",
                            name.leaf_text()
                        ),
                        hints: vec![format!("add a space after `{}`", name.leaf_text())],
                    });
                }
                let fields = &source[name.range().end..node.range().end];
                splices.push((node.range(), format!("#(({}){fields});", literal(value))));
            }
            // A field's name and a named argument's name aren't references.
            SyntaxKind::FieldAccess => stack.extend(node.children().take(1)),
            SyntaxKind::Named => stack.extend(node.children().skip(1)),
            _ => stack.extend(node.children()),
        }
    }

    let mut bound = source.to_owned();
    splices.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    for (range, text) in splices {
        bound.replace_range(range, &text);
    }
    Ok(bound)
}

/// The name a math name or field access refers to.
fn math_target<'a>(node: &LinkedNode<'a>) -> LinkedNode<'a> {
    let mut target = node.clone();
    while target.kind() == SyntaxKind::MathFieldAccess {
        let Some(first) = target.children().next() else { break };
        target = first;
    }
    target
}

/// Whether math calls a name or field access: as `rate(x)`, or, directly before delimiters,
/// as an implicit call that groups with them, as `rate{x}` does under a fraction.
fn is_math_callee(node: &LinkedNode) -> bool {
    let called = node.parent().is_some_and(|parent| {
        parent.kind() == SyntaxKind::MathCall && parent.offset() == node.offset()
    });
    let grouped = node.next_sibling().is_some_and(|next| {
        next.kind() == SyntaxKind::MathDelimited && next.offset() == node.range().end
    });
    called || grouped
}

/// A value as code that evaluates to it.
fn literal(value: &LabelValue) -> String {
    match value {
        LabelValue::None => "none".into(),
        LabelValue::Bool(value) => value.to_string(),
        // The lexer reads `9223372036854775808` as a float.
        LabelValue::Int(i64::MIN) => "(-9223372036854775807 - 1)".into(),
        LabelValue::Int(value) => value.to_string(),
        LabelValue::Float(value) if value.is_nan() => "float.nan".into(),
        LabelValue::Float(value) if value.is_infinite() => {
            if *value < 0.0 { "-float.inf" } else { "float.inf" }.into()
        }
        // `Debug` always writes a decimal point or an exponent, and the shortest digits that
        // read back as the same float.
        LabelValue::Float(value) => format!("{value:?}"),
        LabelValue::Str(value) => value.as_str().repr().into(),
        LabelValue::Date(date) => datetime(*date, None, false),
        LabelValue::NaiveDateTime(datetime_value) => {
            datetime(datetime_value.date(), Some(datetime_value.time()), false)
        }
        LabelValue::ZonedDateTime(instant) => {
            let utc = instant.naive_utc();
            datetime(utc.date(), Some(utc.time()), true)
        }
        LabelValue::Array(values) => match values.as_slice() {
            [] => "()".into(),
            [value] => format!("({},)", literal(value)),
            values => {
                let values: Vec<String> = values.iter().map(literal).collect();
                format!("({})", values.join(", "))
            }
        },
        // Every key is a string, since bare keys can't be keywords such as `none`.
        LabelValue::Dict(values) if values.is_empty() => "(:)".into(),
        LabelValue::Dict(values) => {
            let entries: Vec<String> = values
                .iter()
                .map(|(key, value)| {
                    format!("{}: {}", key.as_str().repr(), literal(value))
                })
                .collect();
            format!("({})", entries.join(", "))
        }
    }
}

/// A call of the label library's `datetime` that builds this date or datetime.
fn datetime(date: NaiveDate, time: Option<NaiveTime>, utc: bool) -> String {
    let mut fields = vec![
        format!("year: {}", date.year()),
        format!("month: {}", date.month()),
        format!("day: {}", date.day()),
    ];
    if let Some(time) = time {
        fields.push(format!("hour: {}", time.hour()));
        fields.push(format!("minute: {}", time.minute()));
        fields.push(format!("second: {}", time.second()));
        if time.nanosecond() != 0 {
            fields.push(format!("nanosecond: {}", time.nanosecond()));
        }
    }
    if utc {
        fields.push("utc: true".into());
    }
    format!("datetime({})", fields.join(", "))
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;
    use typst_syntax::ast;

    use super::*;
    use crate::label::fixtures::{self, WithSource};
    use crate::typst_eval::{Eval, Vm};
    use crate::typst_library::Library;
    use crate::typst_library::engine::{Engine, Sink};
    use crate::typst_library::foundations::{Datetime, Scopes, Str, Value};

    /// A value as the evaluator has it.
    fn evaluated(value: &LabelValue) -> Value {
        match value {
            LabelValue::None => Value::None,
            LabelValue::Bool(value) => Value::Bool(*value),
            LabelValue::Int(value) => Value::Int(*value),
            LabelValue::Float(value) => Value::Float(*value),
            LabelValue::Str(value) => Value::Str(value.as_str().into()),
            LabelValue::Date(value) => Value::Datetime(Datetime::Date(*value)),
            LabelValue::NaiveDateTime(value) => Value::Datetime(Datetime::Naive(*value)),
            LabelValue::ZonedDateTime(value) => Value::Datetime(Datetime::Zoned(*value)),
            LabelValue::Array(values) => {
                Value::Array(values.iter().map(evaluated).collect())
            }
            LabelValue::Dict(values) => Value::Dict(
                values
                    .iter()
                    .map(|(name, value)| (Str::from(name.as_str()), evaluated(value)))
                    .collect(),
            ),
        }
    }

    /// Evaluates code with the label library in scope.
    fn eval(code: &str) -> Value {
        let world = WithSource { world: fixtures::shared(), source: code };
        let mut sink = Sink::new();
        let engine = Engine { world: &world, sink: &mut sink };
        let mut vm = Vm::new(engine, Scopes::new(Some(Library::get())));
        let root = typst_syntax::parse_code(code);
        let (errors, _) = root.errors_and_warnings();
        assert!(errors.is_empty(), "{code}: {errors:?}");
        let expr = root
            .cast::<ast::Code>()
            .and_then(|code| code.exprs().next())
            .unwrap_or_else(|| panic!("{code} has no expression"));
        expr.eval(&mut vm).unwrap_or_else(|error| panic!("{code}: {error:?}"))
    }

    /// Every kind of value reads back from its literal as the same value, at the extremes of
    /// its range.
    #[test]
    fn literals_read_back_as_their_values() {
        let date = NaiveDate::from_ymd_opt(-262_143, 1, 1).unwrap();
        let leap = NaiveDate::from_ymd_opt(2016, 12, 31)
            .unwrap()
            .and_hms_nano_opt(23, 59, 59, 1_999_999_999)
            .unwrap();
        let many: Vec<_> = (0..45).map(LabelValue::Int).collect();
        let keys: IndexMap<String, LabelValue> = ["none", "in", "_", "two words", "a-b"]
            .into_iter()
            .map(|key| (key.to_string(), LabelValue::Str(key.to_string())))
            .collect();
        let values = [
            LabelValue::None,
            LabelValue::Bool(false),
            LabelValue::Int(i64::MIN),
            LabelValue::Int(i64::MAX),
            LabelValue::Int(-5),
            LabelValue::Float(-0.0),
            LabelValue::Float(5e-324),
            LabelValue::Float(1e300),
            LabelValue::Float(-2.5),
            LabelValue::Float(1.0),
            LabelValue::Float(f64::MAX),
            LabelValue::Float(f64::INFINITY),
            LabelValue::Float(f64::NEG_INFINITY),
            LabelValue::Str(
                "quote \" backslash \\ tab \t line\nnull \0 é\u{301} 😀".into(),
            ),
            LabelValue::Str(String::new()),
            LabelValue::Date(date),
            LabelValue::NaiveDateTime(leap),
            LabelValue::ZonedDateTime(leap.and_utc()),
            LabelValue::Array(vec![]),
            LabelValue::Array(vec![LabelValue::Float(0.5)]),
            LabelValue::Array(many),
            LabelValue::Dict(IndexMap::new()),
            LabelValue::Dict(keys.clone()),
            LabelValue::Array(vec![
                LabelValue::Dict(keys),
                LabelValue::Array(vec![LabelValue::Int(i64::MIN)]),
            ]),
        ];
        for value in values {
            let code = literal(&value);
            let (actual, expected) = (eval(&code), evaluated(&value));
            assert_eq!(actual.ty(), expected.ty(), "{code}");
            assert_eq!(actual, expected, "{code}");
        }
        let negative_zero = eval(&literal(&LabelValue::Float(-0.0)));
        assert!(matches!(negative_zero, Value::Float(value) if value.is_sign_negative()));
        let nan = eval(&literal(&LabelValue::Float(f64::NAN)));
        assert!(matches!(nan, Value::Float(value) if value.is_nan()), "{nan:?}");
    }

    /// A value takes the place of each reference to its name, and nothing else.
    #[test]
    fn bind_replaces_references_only() {
        let values = LabelValues::from([
            ("n".to_string(), LabelValue::Int(2)),
            ("rate".to_string(), LabelValue::Float(0.5)),
            ("pt".to_string(), LabelValue::Dict(IndexMap::new())),
        ]);
        let cases = [
            // A `*` between letters stays text.
            ("#n*2", "#(2)\\*2"),
            ("#(n*2)", "#((2)*2)"),
            ("#n; #n.x #n(1)", "#(2); #(2).x #(2)(1)"),
            ("#text(fill: n, size: n)[n]", "#text(fill: (2), size: (2))[n]"),
            ("#pt.n", "#((:)).n"),
            // Single letters in math are letters, not names.
            (
                "$n rate_1 rate' rate; pt.x_1$",
                "$n #((0.5));_1 #((0.5));' #((0.5));; #(((:)).x);_1$",
            ),
            (
                "$frac(rate, 2) rate (x) #rate$",
                "$frac(#((0.5));, 2) #((0.5)); (x) #(0.5)$",
            ),
            ("#unknown and `#n` <n> @n", "#unknown and `#n` <n> @n"),
        ];
        for (source, bound) in cases {
            assert_eq!(bind(source, &values).unwrap(), bound, "{source}");
        }
    }
}
