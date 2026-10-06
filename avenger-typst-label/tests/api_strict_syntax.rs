//! The label syntax: upstream's, where everything a label can't use is an error in upstream's
//! wording.

mod common;

use avenger_typst_label::{
    CompiledLabel, FrameItem, LabelEngine, LabelError, LabelOptions,
};

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

fn has_shape(label: &CompiledLabel) -> bool {
    let mut shape = false;
    label.frame.visit(Default::default(), &mut |_, item| {
        shape |= matches!(item, FrameItem::Shape(_));
    });
    shape
}

#[test]
fn embedded_code_follows_upstream() {
    assert_eq!(error("$#x$"), ("unknown variable: x".into(), 2..3));
    assert_eq!(error("$#{x}$"), ("unknown variable: x".into(), 3..4));
    assert_eq!(error("$#box(x)$"), ("unknown variable: box".into(), 2..5));
}

#[test]
fn statements_are_errors() {
    // In math, a statement needs a semicolon before the closing dollar sign.
    assert_eq!(error("$#import \"foo.typ\"$").0, "expected semicolon or line break");
    assert_eq!(error("$#import \"foo.typ\";$").0, "imports are not supported in labels");
    assert_eq!(error("$#let f(x) = x$").0, "expected semicolon or line break");
    assert_eq!(error("$#let f(x) = x;$").0, "let bindings are not supported in labels");
}

#[test]
fn supported_text_markup_compiles() {
    let samples = [
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
    ];

    for sample in samples {
        let label = engine()
            .compile(sample, &LabelOptions::default())
            .unwrap_or_else(|err| panic!("{sample} should be accepted, got {err:?}"));
        assert!(!label.flags.has_math, "{sample}");
        assert!(label.metrics.width > 0.0, "{sample}");
        assert!(label.metrics.height > 0.0, "{sample}");
    }
}

#[test]
fn decoration_options_compile() {
    let samples = [
        "#underline(stroke: 1.5pt + red, offset: 2pt, extent: 3pt, evade: false, background: true)[care]",
        "#overline(stroke: 1.5pt + red, offset: -1.2em, extent: 2pt, evade: true, background: true)[top]",
        "#strike(stroke: 1.5pt + red, offset: -3.5pt, extent: 2pt, background: true)[gone]",
    ];

    for sample in samples {
        let label = engine()
            .compile(sample, &LabelOptions::default())
            .unwrap_or_else(|err| panic!("{sample} should be accepted, got {err:?}"));
        assert!(has_shape(&label), "{sample}");
    }
}

#[test]
fn unknown_arguments_are_errors() {
    assert_eq!(
        error("#strike(evade: false)[old]"),
        ("unexpected argument: evade".into(), 8..20)
    );
}

#[test]
fn common_math_compiles() {
    let samples = [
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
    ];

    for sample in samples {
        let label = engine()
            .compile(sample, &LabelOptions::default())
            .unwrap_or_else(|err| panic!("{sample} should be accepted, got {err:?}"));
        assert!(label.flags.has_math, "{sample}");
    }
}

#[test]
fn multiline_math_is_an_error() {
    for (source, message) in [
        ("$mat(1, 2; 3, 4)$", "matrices are not supported in labels"),
        ("$vec(1, 2, 3)$", "vectors are not supported in labels"),
        ("$cases(x, y)$", "case distinctions are not supported in labels"),
    ] {
        assert_eq!(error(source).0, message, "{source}");
    }
}

#[test]
fn surplus_arguments_are_errors() {
    assert_eq!(error("$overbrace(x, y, z)$"), ("unexpected argument".into(), 17..18));
}

#[test]
fn syntax_errors_are_upstreams() {
    assert_eq!(error("before $x^$ after"), ("expected expression".into(), 10..10));
}
