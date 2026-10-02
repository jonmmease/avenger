mod common;

use avenger_typst_label::{CompiledLabel, LabelEngine, LabelError, LabelFrameItem, LabelOptions};

fn engine() -> LabelEngine {
    LabelEngine::new(common::engine_options()).unwrap()
}

fn has_shape(label: &CompiledLabel) -> bool {
    label.frame.items.iter().any(|(_, item)| match item {
        LabelFrameItem::Shape(_) => true,
        LabelFrameItem::Group(group) => group
            .items
            .iter()
            .any(|(_, item)| matches!(item, LabelFrameItem::Shape(_))),
        LabelFrameItem::Text(_) | LabelFrameItem::Image(_) => false,
    })
}

#[test]
fn rejects_hash_identifier() {
    let err = engine()
        .compile("$#x$", &LabelOptions::default())
        .unwrap_err();
    assert!(matches!(
        err,
        LabelError::UnsupportedSyntax { position: 1, .. }
    ));
}

#[test]
fn rejects_hash_content_block() {
    let err = engine()
        .compile("$#{x}$", &LabelOptions::default())
        .unwrap_err();
    assert!(matches!(
        err,
        LabelError::UnsupportedSyntax { position: 1, .. }
    ));
}

#[test]
fn rejects_hash_box_call() {
    let err = engine()
        .compile("$#box(x)$", &LabelOptions::default())
        .unwrap_err();
    assert!(matches!(
        err,
        LabelError::UnsupportedSyntax { position: 1, .. }
    ));
}

#[test]
fn rejects_import() {
    let err = engine()
        .compile("$#import \"foo.typ\"$", &LabelOptions::default())
        .unwrap_err();
    assert!(matches!(err, LabelError::Syntax { .. }));
}

#[test]
fn rejects_let_function() {
    let err = engine()
        .compile("$#let f(x) = x$", &LabelOptions::default())
        .unwrap_err();
    assert!(matches!(err, LabelError::Syntax { .. }));
}

#[test]
fn rejects_unretained_text_markup_functions() {
    let err = engine()
        .compile("#highlight[warning]", &LabelOptions::default())
        .unwrap_err();

    assert_eq!(
        err,
        LabelError::UnsupportedSyntax {
            position: 0,
            message: "unsupported static text command"
        }
    );
}

#[test]
fn allows_supported_typst_text_model_markup() {
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
    ];

    for sample in samples {
        let label = engine()
            .compile(sample, &LabelOptions::default())
            .unwrap_or_else(|err| panic!("{sample} should be accepted, got {err:?}"));
        assert!(label.flags.has_markup, "{sample}");
        assert!(!label.flags.has_math, "{sample}");
        assert!(label.metrics.width > 0.0, "{sample}");
        assert!(label.metrics.height > 0.0, "{sample}");
    }
}

#[test]
fn allows_supported_typst_decoration_options() {
    let samples = [
        "#underline(stroke: 1.5pt + red, offset: 2pt, extent: 3pt, evade: false, background: true)[care]",
        "#overline(stroke: 1.5pt + red, offset: -1.2em, extent: 2pt, evade: true, background: true)[top]",
        "#strike(stroke: 1.5pt + red, offset: -3.5pt, extent: 2pt, background: true)[gone]",
    ];

    for sample in samples {
        let label = engine()
            .compile(sample, &LabelOptions::default())
            .unwrap_or_else(|err| panic!("{sample} should be accepted, got {err:?}"));
        assert!(label.flags.has_markup, "{sample}");
        assert!(!label.flags.has_math, "{sample}");
        assert!(has_shape(&label), "{sample}");
    }
}

#[test]
fn rejects_unsupported_strike_evade_option() {
    let err = engine()
        .compile("#strike(evade: false)[old]", &LabelOptions::default())
        .unwrap_err();

    assert_eq!(
        err,
        LabelError::UnsupportedSyntax {
            position: 8,
            message: "strike does not support evade"
        }
    );
}

#[test]
fn allows_common_typst_math_fragments() {
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
        engine()
            .compile(sample, &LabelOptions::default())
            .unwrap_or_else(|err| panic!("{sample} should be accepted, got {err:?}"));
    }
}

#[test]
fn rejects_deferred_matrix_table_math() {
    for (source, feature) in [
        ("$mat(1, 2; 3, 4)$", "mat"),
        ("$vec(1, 2, 3)$", "vec"),
        ("$cases(x, y)$", "cases"),
    ] {
        let err = engine()
            .compile(source, &LabelOptions::default())
            .unwrap_err();

        assert_eq!(
            err,
            LabelError::UnsupportedFeature {
                position: 1,
                feature: feature.to_string(),
                message: "matrix/table math is not supported in Avenger Typst subset"
            },
            "{source}"
        );
    }
}

#[test]
fn rejects_invalid_under_over_arity() {
    let err = engine()
        .compile("$overbrace(x, y, z)$", &LabelOptions::default())
        .unwrap_err();

    assert_eq!(
        err,
        LabelError::UnsupportedSyntax {
            position: 1,
            message: "under/over math calls require a body and optional annotation"
        }
    );
}

#[test]
fn rejects_real_typst_parse_error() {
    let err = engine()
        .compile("before $x^$ after", &LabelOptions::default())
        .unwrap_err();

    assert!(matches!(err, LabelError::Syntax { position: 10, .. }));
}
