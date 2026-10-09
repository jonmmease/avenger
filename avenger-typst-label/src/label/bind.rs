//! Writing values into markup, for the libraries that compute what labels show.

use std::ops::Range;

use chrono::{Datelike, NaiveDate, NaiveTime, Timelike};
use typst_syntax::{LinkedNode, SyntaxKind};

use super::error::{LabelError, source_error};
use super::params::{LabelParamValue, LabelParams};
use crate::typst_eval::parse_label;
use crate::typst_library::foundations::Repr;

/// Writes values into a label's markup: the markup typesets as `source` would with each value
/// bound to its name before it, as `#let` would bind it, which labels don't have.
///
/// Each reference to a value's name, `#name` in markup and code or `name` in math, becomes the
/// value written as code, and values shadow the library's names, as bindings do. Values the
/// source doesn't refer to are ignored, and names without a value are left for compilation to
/// report. Errors in the bound markup point into the bound text.
///
/// A source that doesn't parse returns its syntax error, and so does math that calls a value's
/// name, as in `$rate(x)$`, which bind can't write: a space after the name, as in
/// `$rate (x)$`, sets the value before the parentheses.
pub fn bind(source: &str, values: &LabelParams) -> Result<String, LabelError> {
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
fn literal(value: &LabelParamValue) -> String {
    match value {
        LabelParamValue::None => "none".into(),
        LabelParamValue::Bool(value) => value.to_string(),
        // The lexer reads `9223372036854775808` as a float.
        LabelParamValue::Int(i64::MIN) => "(-9223372036854775807 - 1)".into(),
        LabelParamValue::Int(value) => value.to_string(),
        LabelParamValue::Float(value) if value.is_nan() => "float.nan".into(),
        LabelParamValue::Float(value) if value.is_infinite() => {
            if *value < 0.0 { "-float.inf" } else { "float.inf" }.into()
        }
        // `Debug` always writes a decimal point or an exponent, and the shortest digits that
        // read back as the same float.
        LabelParamValue::Float(value) => format!("{value:?}"),
        LabelParamValue::Str(value) => value.as_str().repr().into(),
        LabelParamValue::Date(date) => datetime(*date, None, false),
        LabelParamValue::NaiveDateTime(datetime_value) => {
            datetime(datetime_value.date(), Some(datetime_value.time()), false)
        }
        LabelParamValue::ZonedDateTime(instant) => {
            let utc = instant.naive_utc();
            datetime(utc.date(), Some(utc.time()), true)
        }
        LabelParamValue::Array(values) => match values.as_slice() {
            [] => "()".into(),
            [value] => format!("({},)", literal(value)),
            values => {
                let values: Vec<String> = values.iter().map(literal).collect();
                format!("({})", values.join(", "))
            }
        },
        // Every key is a string, since bare keys can't be keywords such as `none`.
        LabelParamValue::Dict(values) if values.is_empty() => "(:)".into(),
        LabelParamValue::Dict(values) => {
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
    use crate::typst_library::foundations::{Scopes, Value};

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
        let many: Vec<_> = (0..45).map(LabelParamValue::Int).collect();
        let keys: IndexMap<String, LabelParamValue> =
            ["none", "in", "_", "two words", "a-b"]
                .into_iter()
                .map(|key| (key.to_string(), LabelParamValue::Str(key.to_string())))
                .collect();
        let values = [
            LabelParamValue::None,
            LabelParamValue::Bool(false),
            LabelParamValue::Int(i64::MIN),
            LabelParamValue::Int(i64::MAX),
            LabelParamValue::Int(-5),
            LabelParamValue::Float(-0.0),
            LabelParamValue::Float(5e-324),
            LabelParamValue::Float(1e300),
            LabelParamValue::Float(-2.5),
            LabelParamValue::Float(1.0),
            LabelParamValue::Float(f64::MAX),
            LabelParamValue::Float(f64::INFINITY),
            LabelParamValue::Float(f64::NEG_INFINITY),
            LabelParamValue::Str(
                "quote \" backslash \\ tab \t line\nnull \0 é\u{301} 😀".into(),
            ),
            LabelParamValue::Str(String::new()),
            LabelParamValue::Date(date),
            LabelParamValue::NaiveDateTime(leap),
            LabelParamValue::ZonedDateTime(leap.and_utc()),
            LabelParamValue::Array(vec![]),
            LabelParamValue::Array(vec![LabelParamValue::Float(0.5)]),
            LabelParamValue::Array(many),
            LabelParamValue::Dict(IndexMap::new()),
            LabelParamValue::Dict(keys.clone()),
            LabelParamValue::Array(vec![
                LabelParamValue::Dict(keys),
                LabelParamValue::Array(vec![LabelParamValue::Int(i64::MIN)]),
            ]),
        ];
        for value in values {
            let code = literal(&value);
            let (actual, expected) = (eval(&code), value.to_value());
            assert_eq!(actual.ty(), expected.ty(), "{code}");
            assert_eq!(actual, expected, "{code}");
        }
        let negative_zero = eval(&literal(&LabelParamValue::Float(-0.0)));
        assert!(matches!(negative_zero, Value::Float(value) if value.is_sign_negative()));
        let nan = eval(&literal(&LabelParamValue::Float(f64::NAN)));
        assert!(matches!(nan, Value::Float(value) if value.is_nan()), "{nan:?}");
    }

    /// A value takes the place of each reference to its name, and nothing else.
    #[test]
    fn bind_replaces_references_only() {
        let values = LabelParams::from([
            ("n".to_string(), LabelParamValue::Int(2)),
            ("rate".to_string(), LabelParamValue::Float(0.5)),
            ("pt".to_string(), LabelParamValue::Dict(IndexMap::new())),
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
