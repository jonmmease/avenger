use avenger_typst::{AvengerTypst, MathTypesetError, TypstEngineConfig};

fn engine() -> AvengerTypst {
    AvengerTypst::new(TypstEngineConfig::default()).unwrap()
}

#[test]
fn rejects_hash_identifier() {
    let err = engine()
        .typeset_math_string("$#x$", &Default::default())
        .unwrap_err();
    assert!(matches!(
        err,
        MathTypesetError::UnsupportedSyntax { position: 1, .. }
    ));
}

#[test]
fn rejects_hash_content_block() {
    let err = engine()
        .typeset_math_string("$#{x}$", &Default::default())
        .unwrap_err();
    assert!(matches!(
        err,
        MathTypesetError::UnsupportedSyntax { position: 1, .. }
    ));
}

#[test]
fn rejects_hash_box_call() {
    let err = engine()
        .typeset_math_string("$#box(x)$", &Default::default())
        .unwrap_err();
    assert!(matches!(
        err,
        MathTypesetError::UnsupportedSyntax { position: 1, .. }
    ));
}

#[test]
fn rejects_import() {
    let err = engine()
        .typeset_math_string("$#import \"foo.typ\"$", &Default::default())
        .unwrap_err();
    assert!(matches!(
        err,
        MathTypesetError::UnsupportedSyntax { position: 1, .. }
    ));
}

#[test]
fn rejects_let_function() {
    let err = engine()
        .typeset_math_string("$#let f(x) = x$", &Default::default())
        .unwrap_err();
    assert!(matches!(
        err,
        MathTypesetError::UnsupportedSyntax { position: 1, .. }
    ));
}

#[test]
fn allows_common_typst_math_fragments() {
    let samples = [
        "$alpha + beta$",
        "$sqrt(x^2 + y^2)$",
        "$sum_(i=1)^n x_i$",
        "$binom(n, k)$",
        "$cancel(x)$",
        "$mat(1, 2; 3, 4)$",
    ];

    for sample in samples {
        engine()
            .typeset_math_string(sample, &Default::default())
            .unwrap_or_else(|err| panic!("{sample} should be accepted, got {err:?}"));
    }
}

#[cfg(feature = "vendor-typst")]
#[test]
fn rejects_real_typst_parse_error() {
    let err = engine()
        .typeset_math_string("before $x^$ after", &Default::default())
        .unwrap_err();

    assert_eq!(
        err,
        MathTypesetError::UnsupportedSyntax {
            position: 10,
            message: "invalid Typst math syntax"
        }
    );
}
