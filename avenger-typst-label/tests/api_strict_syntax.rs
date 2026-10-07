//! The label syntax: upstream's, where everything a label can't use is an error in upstream's
//! wording.

mod common;

use avenger_typst_label::{LabelEngine, LabelError, LabelOptions};

fn engine() -> LabelEngine {
    LabelEngine::new(common::engine_options())
}

/// The message and range of a source's error.
fn error(source: &str) -> (String, std::ops::Range<usize>) {
    match engine().compile(source, &LabelOptions::default()).unwrap_err() {
        LabelError::Source { range, message, .. } => (message, range),
        other => panic!("{source}: {other:?}"),
    }
}

#[test]
fn embedded_code_follows_upstream() {
    assert_eq!(error("$#x$"), ("unknown variable: x".into(), 2..3));
    assert_eq!(error("$#{x}$"), ("unknown variable: x".into(), 3..4));
    assert_eq!(error("$#box(x)$"), ("unknown variable: box".into(), 2..5));
}

#[test]
fn statements_are_errors() {
    assert_eq!(
        error("#let x = 1"),
        ("let bindings are not supported in labels".into(), 1..10)
    );
    // In math, a statement needs a semicolon before the closing dollar sign.
    assert_eq!(error("$#import \"foo.typ\"$").0, "expected semicolon or line break");
    assert_eq!(error("$#import \"foo.typ\";$").0, "imports are not supported in labels");
    assert_eq!(error("$#let f(x) = x$").0, "expected semicolon or line break");
    assert_eq!(error("$#let f(x) = x;$").0, "let bindings are not supported in labels");
}

#[test]
fn unknown_arguments_are_errors() {
    assert_eq!(
        error("#strike(evade: false)[old]"),
        ("unexpected argument: evade".into(), 8..20)
    );
    assert_eq!(error("#super(foo: true)[x]"), ("unexpected argument: foo".into(), 7..16));
}

#[test]
fn strokes_are_lengths_colors_and_dictionaries() {
    // Labels have no gradients or tilings.
    assert_eq!(
        error("#underline(stroke: 1pt + gradient.linear(red, blue))[group]"),
        ("unknown variable: gradient".into(), 25..33)
    );
    assert_eq!(
        error("#underline(stroke: pattern())[group]"),
        ("unknown variable: pattern".into(), 19..26)
    );
    // Upstream's message also lists gradients and tilings.
    assert_eq!(
        error("#underline(stroke: \"x\")[a]"),
        (
            "expected length, color, dictionary, stroke, or auto, found string".into(),
            19..22
        )
    );
    // A miter limit is a number.
    assert_eq!(
        error("#underline(stroke: (miter-limit: 2pt))[group]"),
        ("expected float or auto, found length".into(), 19..37)
    );
}

#[test]
fn unknown_symbols_are_errors() {
    assert_eq!(
        error("#emoji.not.real"),
        ("module `emoji` does not contain `not`".into(), 7..10)
    );
    assert_eq!(error("#sym.not.real"), ("unknown symbol modifier".into(), 9..13));
    assert_eq!(error("#sym.arrow.diagonal"), ("unknown symbol modifier".into(), 11..19));
}

#[test]
fn raw_text_is_one_line_without_highlighting() {
    for source in ["```typ\nlet x = 1\n```", "```typ let x = 1```"] {
        match engine().compile(source, &LabelOptions::default()).unwrap_err() {
            LabelError::Source { range, message, hints } => {
                assert_eq!(
                    (message.as_str(), range),
                    ("syntax highlighting is not supported in labels", 3..6),
                    "{source:?}"
                );
                assert_eq!(hints, ["remove the language tag"]);
            }
            other => panic!("{source:?}: {other:?}"),
        }
    }
    assert_eq!(
        error("```\na\nb\n```"),
        ("raw text in a label must be a single line".into(), 0..11)
    );
    // The same rules hold for raw text from a call.
    match engine().compile("#raw(\"a\", lang: \"rust\")", &LabelOptions::default()) {
        Err(LabelError::Source { range, message, hints }) => {
            assert_eq!(
                (message.as_str(), range),
                ("syntax highlighting is not supported in labels", 1..23)
            );
            assert_eq!(hints, ["remove the `lang` argument"]);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        error("#raw(\"a\\nb\")"),
        ("raw text in a label must be a single line".into(), 1..12)
    );
}

/// Sources that no upstream case covers; the cases check the rest of the syntax.
#[test]
fn code_block_sums_and_delimiter_symbols_compile() {
    for source in ["#{ [a] + [b] }", "$floor(x) + paren.l x paren.r$"] {
        engine()
            .compile(source, &LabelOptions::default())
            .unwrap_or_else(|err| panic!("{source}: {err:?}"));
    }
}

#[test]
fn multiline_math_is_an_error() {
    for (source, message, range) in [
        ("$mat(1, 2; 3, 4)$", "matrices are not supported in labels", 1..16),
        ("$vec(1, 2, 3)$", "vectors are not supported in labels", 1..13),
        ("$cases(x, y)$", "case distinctions are not supported in labels", 1..12),
        ("before $mat(1, 2; 3, 4)$ after", "matrices are not supported in labels", 8..23),
        ("$x &= y$", "alignment points are not supported in labels", 3..4),
        ("$x \\ y$", "line breaks are not supported in equations in labels", 3..4),
    ] {
        assert_eq!(error(source), (message.into(), range), "{source}");
    }
}

#[test]
fn stroke_paints_in_math_are_colors() {
    // Labels have no gradients or tilings.
    assert_eq!(
        error("$cancel(x, stroke: #(paint: gradient.linear(red, blue)))$"),
        ("unknown variable: gradient".into(), 28..36)
    );
    // Upstream's message also lists gradients and tilings.
    assert_eq!(
        error("$cancel(x, stroke: #auto)$"),
        ("expected length, color, dictionary, or stroke, found auto".into(), 20..24)
    );
}

#[test]
fn surplus_arguments_are_errors() {
    assert_eq!(error("$overbrace(x, y, z)$"), ("unexpected argument".into(), 17..18));
}

#[test]
fn syntax_errors_are_upstreams() {
    assert_eq!(error("before $x^$ after"), ("expected expression".into(), 10..10));
}
