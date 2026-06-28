use avenger_typst_label::{LabelEngine, LabelError, LabelOptions};

fn engine() -> LabelEngine {
    LabelEngine::new(Default::default()).unwrap()
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
fn allows_common_typst_math_fragments() {
    let samples = [
        "$alpha + beta$",
        "$sqrt(x^2 + y^2)$",
        "$sum_(i=1)^n x_i$",
        "$binom(n, k)$",
        "$cancel(x)$",
        "$a class(\"relation\", !) b$",
        "$script(a / b, cramped: #true) + sscript(c / d)$",
        "$overline(underline(x + y))$",
        "$overbrace(x + y) + underbrace(a + b)$",
        "$overbracket(x) + underparen(y) + overshell(z)$",
        "$overbrace(x + y, \"sum\") + underparen(z, alpha)$",
        "$attach(Pi, t: alpha, b: beta, tl: 1, tr: 2+3, bl: 4+5, br: 6)$",
        "$a'''_b$",
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
