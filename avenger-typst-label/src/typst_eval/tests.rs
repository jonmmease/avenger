//! Evaluation against upstream: each manifest case's content repr, and its errors.

use super::{Eval, Vm, eval_label, math_nesting_depth, parse_label};
use crate::label::fixtures::{self, WithSource};
use crate::label::label_file;
use crate::label::oracle::{Manifest, Reference};
use crate::typst_library::Library;
use crate::typst_library::diag::SourceResult;
use crate::typst_library::engine::{Engine, Sink};
use crate::typst_library::foundations::{Content, Repr, Scopes, Value};
use typst_syntax::{DiagSpanKind, FileId, ast, parse_code};

/// Evaluates a label source.
fn eval(source: &str) -> SourceResult<Content> {
    let world = WithSource { world: fixtures::shared(), source };
    let mut sink = Sink::new();
    let mut engine = Engine { world: &world, sink: &mut sink };
    let root = parse_label(source);
    eval_label(&mut engine, &root)
}

/// Evaluates code, such as `calc.even(4)`, to its value, which a label might not display.
fn value(code: &str) -> SourceResult<Value> {
    let world = WithSource { world: fixtures::shared(), source: code };
    let mut sink = Sink::new();
    let engine = Engine { world: &world, sink: &mut sink };
    let mut vm = Vm::new(engine, Scopes::new(Some(Library::get())));
    parse_code(code)
        .cast::<ast::Code>()
        .expect("code parses as code")
        .eval(&mut vm)
}

/// Cases whose first error deliberately differs from upstream's, with the reason.
const DIVERGENT_ERRORS: &[(&str, &str)] = &[
    (
        "error-wrong-argument-type",
        "labels have no gradients or tilings, so a stroke's paint is a color",
    ),
    (
        "error-calc-abs-type",
        "labels have no decimals, so calc's functions don't take them",
    ),
];

/// Every case in the suite evaluates as upstream's does: to content with upstream's repr, or
/// to upstream's first error in the label, with its hints. (Upstream's references can start with errors in
/// the reference generator's wrapper, which have no range in the label.)
fn check_suite(suite: &str) {
    let manifest = Manifest::load(suite);
    let mut failures = vec![];
    for case in &manifest.cases {
        let reference = Reference::load(suite, &case.id).unwrap();
        let result = eval(&case.source);
        match (&reference.repr, result) {
            (Some(expected), Ok(content)) => {
                let actual = content.repr();
                if actual != expected.as_str() {
                    failures
                        .push(format!("{}:\n  {actual}\n  expected {expected}", case.id));
                }
            }
            (Some(_), Err(errors)) => {
                failures.push(format!("{}: fails with {}", case.id, errors[0].message));
            }
            (None, Ok(_)) => {
                // Upstream failed later, when laying out, or with an error upstream reports
                // only then.
                if !reference.errors.is_empty() {
                    let message = &reference.errors[0].message;
                    failures.push(format!("{}: evaluates, expected {message}", case.id));
                }
            }
            (None, Err(_)) => {
                let (message, range, hints) = error(&case.source);
                let Some(expected) =
                    reference.errors.iter().find(|error| error.range.is_some())
                else {
                    failures
                        .push(format!("{}: upstream has no error in the label", case.id));
                    continue;
                };
                let divergent = DIVERGENT_ERRORS.iter().any(|(id, _)| *id == case.id);
                if (message != expected.message
                    || range != expected.range
                    || hints != expected.hints)
                    && !divergent
                {
                    failures.push(format!(
                        "{}: {message} at {range:?} {hints:?}, expected {} at {:?} {:?}",
                        case.id, expected.message, expected.range, expected.hints
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn frame_cases_evaluate_like_upstream() {
    check_suite("upstream_frames");
}

#[test]
fn math_cases_evaluate_like_upstream() {
    check_suite("upstream_math");
}

#[test]
fn math_nesting_is_measured_inside_equations() {
    let depth = |source: &str| math_nesting_depth(&parse_label(source));
    assert_eq!(depth("a (b) c"), 0);
    assert_eq!(depth("$x$"), 0);
    assert_eq!(depth("$x^2$"), 1);
    assert_eq!(depth("$sqrt(frac(a, b^2))$"), 3);
}

/// The first error's message, range and hints.
fn error(source: &str) -> (String, Option<[usize; 2]>, Vec<String>) {
    let errors = eval(source).expect_err(source);
    let error = &errors[0];
    let range = match error.span.get() {
        DiagSpanKind::Range { id, range } if id == label_file() => {
            Some([range.start, range.end])
        }
        _ => None,
    };
    let hints = error.hints.iter().map(|hint| hint.v.to_string()).collect();
    (error.message.to_string(), range, hints)
}

#[test]
fn unsupported_constructs_are_errors() {
    for (source, message) in [
        ("#let x = 1", "let bindings are not supported in labels"),
        ("#set text(red)", "set rules are not supported in labels"),
        ("#show strong: emph", "show rules are not supported in labels"),
        ("#if true [a]", "conditionals are not supported in labels"),
        ("#for x in (1,) [a]", "loops are not supported in labels"),
        ("#import \"a.typ\"", "imports are not supported in labels"),
        ("#include \"a.typ\"", "includes are not supported in labels"),
        ("#context 1", "context expressions are not supported in labels"),
        ("#(x => x)", "functions defined in a label are not supported in labels"),
        ("#{ let x = 1 }", "let bindings are not supported in labels"),
        ("= Heading", "headings are not supported in labels"),
        ("- item", "lists are not supported in labels"),
        ("https://typst.app", "links are not supported in labels"),
        ("a <here>", "`<label>` markers are not supported in labels"),
        ("@here", "references are not supported in labels"),
        ("$a & b$", "alignment points are not supported in labels"),
        ("```rust fn```", "syntax highlighting is not supported in labels"),
        ("#(1 + (x = 2))", "assignments are not supported in labels"),
    ] {
        assert_eq!(error(source).0, message, "{source}");
    }
    // Content that would need more than one paragraph.
    let (message, range, hints) = error("a\n\nb");
    assert_eq!(message, "paragraph breaks are not supported in labels");
    assert_eq!(range, Some([1, 3]));
    assert_eq!(hints, ["a label is one paragraph"]);
    assert_eq!(error("```\na\nb\n```").0, "raw text in a label must be a single line");
    let (message, _, hints) = error("$ x $");
    assert_eq!(message, "block equations are not supported in labels");
    assert_eq!(
        hints,
        ["remove the spaces just inside the dollar signs to make the equation inline"]
    );
}

/// The float type's `inf` and `nan` are upstream's constants; the type has no other fields, and
/// no constructor.
#[test]
fn float_constants_are_upstreams() {
    let repr = |source: &str| eval(source).expect(source).repr();
    assert_eq!(repr("#float.inf"), repr("∞"));
    assert_eq!(repr("#(-float.inf)"), repr("−∞"));
    assert_eq!(repr("#float.nan"), repr("NaN"));
    assert_eq!(error("#float.pi").0, "type float does not contain field `pi`");
    assert_eq!(error("#float(1)").0, "expected function, found type");
    assert_eq!(error("#float").0, "cannot display type in a label");
}

/// The calc module has upstream's functions and constants, in upstream's order. The frame
/// cases check what they return, except for booleans, which a label can't display (D3).
#[test]
fn calc_is_upstreams() {
    let Ok(Value::Module(calc)) = value("calc") else { panic!("calc is a module") };
    let names: Vec<&str> = calc.scope().iter().map(|(name, _)| name.as_str()).collect();
    let upstream = "abs pow exp sqrt root sin cos tan asin acos atan atan2 sinh cosh tanh asinh \
                    acosh atanh log ln erf fact perm binom gcd lcm floor ceil trunc fract round \
                    clamp min max even odd rem div-euclid rem-euclid quo norm inf pi tau e";
    assert_eq!(names, upstream.split_whitespace().collect::<Vec<_>>());
    let repr = |code| value(code).expect(code).repr();
    assert_eq!(
        repr("(calc.even(4), calc.even(5), calc.odd(4), calc.odd(5))"),
        "(true, false, false, true)"
    );
    assert_eq!(repr("calc.even(-3)"), "false");
}

/// A NaN bound to `calc.clamp` is an error, where upstream panics.
#[test]
fn calc_clamp_rejects_nan_bounds() {
    assert_eq!(error("#calc.clamp(1, float.nan, 2)").0, "min and max may not be NaN");
    assert_eq!(error("#calc.clamp(1, 0, float.nan)").0, "min and max may not be NaN");
    // The clamped value can be NaN.
    assert_eq!(
        eval("#calc.clamp(float.nan, 0, 1)").unwrap().repr(),
        eval("NaN").unwrap().repr()
    );
}

mod values {
    use std::sync::Arc;

    use avenger_format::{DateTimeFormatProvider, NumberFormatProvider};

    use super::*;
    use crate::label::FormattingCache;
    use crate::typst_library::World;
    use crate::typst_library::text::{Font, FontBook, TextElem};

    /// A world with the fixture fonts, the source and the d3-format providers.
    struct FormatWorld<'a> {
        source: &'a str,
        number: Arc<dyn NumberFormatProvider>,
        datetime: Arc<dyn DateTimeFormatProvider>,
        cache: FormattingCache,
    }

    impl World for FormatWorld<'_> {
        fn book(&self) -> &FontBook {
            fixtures::shared().book()
        }

        fn source(&self, id: FileId) -> Option<&str> {
            (id == label_file()).then_some(self.source)
        }

        fn font(&self, index: usize) -> Option<Font> {
            fixtures::shared().font(index)
        }

        fn number_format(&self) -> Option<&Arc<dyn NumberFormatProvider>> {
            Some(&self.number)
        }

        fn datetime_format(&self) -> Option<&Arc<dyn DateTimeFormatProvider>> {
            Some(&self.datetime)
        }

        fn formatting_cache(&self) -> Option<&FormattingCache> {
            Some(&self.cache)
        }
    }

    fn eval_formatted(source: &str) -> SourceResult<Content> {
        let world = FormatWorld {
            source,
            number: Arc::new(avenger_format_number_d3::D3NumberFormatProvider::new()),
            datetime: Arc::new(
                avenger_format_datetime_d3::D3DateTimeFormatProvider::new(),
            ),
            cache: FormattingCache::default(),
        };
        let mut sink = Sink::new();
        let mut engine = Engine { world: &world, sink: &mut sink };
        eval_label(&mut engine, &parse_label(source))
    }

    #[test]
    fn values_display_like_upstream_values() {
        let text = |source| eval(source).unwrap();
        assert_eq!(text("#(-5)"), TextElem::packed("\u{2212}5"));
        assert_eq!(text("#2.5"), TextElem::packed("2.5"));
        // A run of line breaks in a string becomes a space (D5).
        assert_eq!(text("#\"a\\r\\nb\""), TextElem::packed("a b"));
        assert!(text("#none").is_empty());
        // Numbers are content in function arguments (D4).
        assert!(eval("$frac(#3, 2)$").is_ok());
        // Booleans, dates, arrays and dictionaries don't display (D3).
        let error = eval("#true").unwrap_err();
        assert_eq!(error[0].message, "cannot display boolean in a label");
        assert_eq!(error[0].hints[0].v, "use a string instead");
        let error = eval("#datetime(year: 2024, month: 3, day: 1)").unwrap_err();
        assert_eq!(error[0].hints[0].v, "format it with `#datetimefmt`");
    }

    #[test]
    fn numfmt_and_datetimefmt_format_with_the_label_formatters() {
        let repr = |source| eval_formatted(source).unwrap().repr();
        assert_eq!(repr("#numfmt(1234.5, \",.1f\")"), "[1,234.5]");
        assert_eq!(
            repr("#numfmt(-12345.0, \".2e\")"),
            "equation(\n  body: sequence([−], [1.23], [×], attach(base: [10], t: [4])),\n)"
        );
        assert_eq!(
            repr("#datetimefmt(datetime(year: 2024, month: 3, day: 1), \"%b %Y\")"),
            "[Mar 2024]"
        );

        let error = |source| eval_formatted(source).unwrap_err()[0].message.to_string();
        assert_eq!(error("#numfmt(\"a\")"), "expected float, found string");
        assert_eq!(error("#numfmt(1, \"\", y: 2)"), "unexpected argument: y");
        assert_eq!(error("#datetimefmt(1, \"%Y\")"), "expected datetime, found integer");
        // Without a formatter, formatting fails.
        assert_eq!(
            eval("#numfmt(1)").unwrap_err()[0].message,
            "number formatting is not configured"
        );
    }
}
