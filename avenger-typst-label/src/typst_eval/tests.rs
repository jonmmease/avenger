//! Evaluation against upstream: each manifest case's content repr, and its errors.

use super::{eval_label, math_nesting_depth, parse_label};
use crate::label::fixtures::{self, WithSource};
use crate::label::label_file;
use crate::label::oracle::{Manifest, Reference};
use crate::typst_library::diag::SourceResult;
use crate::typst_library::engine::{Engine, Sink};
use crate::typst_library::foundations::{Content, Repr, Scope};
use typst_syntax::{DiagSpanKind, FileId};

/// Evaluates a label source without parameters.
fn eval(source: &str) -> SourceResult<Content> {
    eval_with(source, Scope::new())
}

fn eval_with(source: &str, params: Scope) -> SourceResult<Content> {
    let world = WithSource { world: fixtures::shared(), source };
    let mut sink = Sink::new();
    let mut engine = Engine { world: &world, sink: &mut sink };
    let root = parse_label(source);
    eval_label(&mut engine, &root, params)
}

/// The first error's message and source range.
/// Cases whose first error deliberately differs from upstream's, with the reason.
const DIVERGENT_ERRORS: &[(&str, &str)] = &[(
    "error-wrong-argument-type",
    "labels have no gradients or tilings, so a stroke's paint is a color",
)];

/// Every case in the suite evaluates as upstream's does: to content with upstream's repr, or
/// to upstream's first error in the label, with its hints. (Upstream's references can start with errors in
/// the probe's wrapper, which have no range in the label.)
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
    // Content that would need more than one line.
    let (message, range, hints) = error("a \\ b");
    assert_eq!(message, "line breaks are not supported in labels");
    assert_eq!(range, Some([2, 3]));
    assert_eq!(hints, ["a label is a single line"]);
    assert_eq!(error("a\n\nb").0, "paragraph breaks are not supported in labels");
    assert_eq!(error("```\na\nb\n```").0, "raw text in a label must be a single line");
    let (message, _, hints) = error("$ x $");
    assert_eq!(message, "block equations are not supported in labels");
    assert_eq!(
        hints,
        ["remove the spaces just inside the dollar signs to make the equation inline"]
    );
}

/// The expectations of `tests/api_strict_syntax.rs`, as upstream reports them.
#[test]
fn strict_syntax_errors_are_upstream_errors() {
    assert_eq!(error("$#x$").0, "unknown variable: x");
    assert_eq!(error("$#x$").1, Some([2, 3]));
    assert_eq!(error("$#{x}$").1, Some([3, 4]));
    assert_eq!(error("$#box(x)$").0, "unknown variable: box");
    assert_eq!(error("$#box(x)$").1, Some([2, 5]));
    // An import statement needs a semicolon before the closing dollar sign.
    assert_eq!(error("$#import \"foo.typ\"$").0, "expected semicolon or line break");
    assert_eq!(error("$#import \"foo.typ\";$").0, "imports are not supported in labels");
    assert_eq!(error("$#let f(x) = x$").0, "expected semicolon or line break");
    assert_eq!(error("$#let f(x) = x;$").0, "let bindings are not supported in labels");
    assert_eq!(
        error("#strike(evade: false)[old]"),
        ("unexpected argument: evade".into(), Some([8, 20]), vec![])
    );
    assert_eq!(error("$overbrace(x, y, z)$").0, "unexpected argument");
    assert_eq!(error("$overbrace(x, y, z)$").1, Some([17, 18]));
    for (source, message) in [
        ("$mat(1, 2; 3, 4)$", "matrices are not supported in labels"),
        ("$vec(1, 2, 3)$", "vectors are not supported in labels"),
        ("$cases(x, y)$", "case distinctions are not supported in labels"),
    ] {
        assert_eq!(error(source).0, message, "{source}");
    }
    assert_eq!(error("before $x^$ after").0, "expected expression");
    assert_eq!(error("before $x^$ after").1, Some([10, 10]));
}

#[test]
fn supported_markup_evaluates() {
    for source in [
        "#lower[LOUD]",
        "#upper[quiet]",
        "#smallcaps[Small Caps]",
        "H#sub[2]O",
        "x#super[2]",
        "#emph[call]",
        "#strong(delta: 150)[mild]",
        "_emph syntax_",
        "*strong syntax*",
        "`x # y`",
        "#raw(\"z * w\")",
        "#highlight[warning]",
        "#underline(stroke: 1.5pt + red, offset: 2pt, extent: 3pt, evade: false, background: true)[care]",
        "#overline(stroke: 1.5pt + red, offset: -1.2em, extent: 2pt, evade: true, background: true)[top]",
        "#strike(stroke: 1.5pt + red, offset: -3.5pt, extent: 2pt, background: true)[gone]",
        "#text(fill: tomato, size: 1.2em, font: \"Lato\")[styled]",
        "#{ [a] + [b] }",
        "$alpha + beta$",
        "$sqrt(x^2 + y^2)$",
        "$root(3, x)$",
        "$sum_(i=1)^n x_i$",
        "$binom(n, k)$",
        "$cancel(x)$",
        "$a class(\"relation\", !) b$",
        "$lr(| A mid(|) integral |)$",
        "$script(a / b, cramped: #true) + sscript(c / d)$",
        "$hat(i) + accent(v, <-)$",
        "$stretch(->, size: #200%)$",
        "$overline(underline(x + y))$",
        "$overbrace(x + y) + underbrace(a + b)$",
        "$overbracket(x) + underparen(y) + overshell(z)$",
        "$overbrace(x + y, \"sum\") + underparen(z, alpha)$",
        "$attach(Pi, t: alpha, b: beta, tl: 1, tr: 2+3, bl: 4+5, br: 6)$",
        "$a'''_b$",
        "$bold(x) + italic(y) + upright(z) + bb(N) + cal(P) + frak(g)$",
        "$floor(x) + paren.l x paren.r$",
    ] {
        eval(source).unwrap_or_else(|err| panic!("{source}: {err:?}"));
    }
}

mod params {
    use std::sync::Arc;

    use avenger_format::{DateTimeFormatProvider, NumberFormatProvider};

    use super::*;
    use crate::label::FormattingCache;
    use crate::typst_library::World;
    use crate::typst_library::foundations::{Datetime, IntoValue, Value};
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

    fn eval_formatted(
        source: &str,
        params: Vec<(&'static str, Value)>,
    ) -> SourceResult<Content> {
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
        eval_label(&mut engine, &parse_label(source), scope(params))
    }

    fn scope(params: Vec<(&'static str, Value)>) -> Scope {
        let mut scope = Scope::new();
        for (name, value) in params {
            scope.define(name, value);
        }
        scope
    }

    #[test]
    fn params_display_like_upstream_values() {
        let text =
            |source, value: Value| eval_with(source, scope(vec![("p", value)])).unwrap();
        assert_eq!(text("#p", Value::Int(-5)), TextElem::packed("\u{2212}5"));
        assert_eq!(text("#p", Value::Float(2.5)), TextElem::packed("2.5"));
        // A run of line breaks in a parameter becomes a space (D5).
        assert_eq!(text("#p", "a\r\nb".into_value()), TextElem::packed("a b"));
        assert!(text("#p", Value::None).is_empty());
        // Numbers are content in function arguments (D4).
        assert!(eval_with("$frac(#p, 2)$", scope(vec![("p", Value::Int(3))])).is_ok());
        // Booleans, dates, arrays and dictionaries don't display (D3).
        let error = eval_with("#p", scope(vec![("p", Value::Bool(true))])).unwrap_err();
        assert_eq!(error[0].message, "cannot display boolean in a label");
        assert_eq!(error[0].hints[0].v, "use a string instead");
        let date = Datetime::Date(chrono::NaiveDate::from_ymd_opt(2024, 3, 1).unwrap());
        let error =
            eval_with("#p", scope(vec![("p", Value::Datetime(date))])).unwrap_err();
        assert_eq!(error[0].hints[0].v, "format it with `#datetimefmt`");
        // Parameters shadow math definitions, as `let` bindings do.
        let shadowed =
            eval_with("$alpha$", scope(vec![("alpha", "a".into_value())])).unwrap();
        assert!(shadowed.repr().contains("[a]"), "{}", shadowed.repr());
    }

    #[test]
    fn numfmt_and_datetimefmt_format_with_the_label_formatters() {
        let repr = |source, params| eval_formatted(source, params).unwrap().repr();
        assert_eq!(repr("#numfmt(1234.5, \",.1f\")", vec![]), "[1,234.5]");
        assert_eq!(
            repr("#numfmt(n, \".2e\")", vec![("n", Value::Float(-12345.0))]),
            "equation(\n  body: sequence([−], [1.23], [×], attach(base: [10], t: [4])),\n)"
        );
        let date = Datetime::Date(chrono::NaiveDate::from_ymd_opt(2024, 3, 1).unwrap());
        assert_eq!(
            repr("#datetimefmt(d, \"%b %Y\")", vec![("d", Value::Datetime(date))]),
            "[Mar 2024]"
        );

        let error =
            |source| eval_formatted(source, vec![]).unwrap_err()[0].message.to_string();
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
