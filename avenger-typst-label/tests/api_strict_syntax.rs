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
    for source in ["$mat(1, 2; 3, 4)$", "$vec(1, 2, 3)$", "$cases(x, y)$"] {
        let err = engine()
            .compile(source, &LabelOptions::default())
            .unwrap_err();

        assert_eq!(
            err,
            LabelError::UnsupportedSyntax {
                position: 1,
                message: "matrix/table math is not supported in Avenger Typst subset"
            },
            "{source}"
        );
    }
}

#[test]
fn rejects_real_typst_parse_error() {
    let err = engine()
        .compile("before $x^$ after", &LabelOptions::default())
        .unwrap_err();

    assert!(matches!(err, LabelError::Syntax { position: 10, .. }));
}
