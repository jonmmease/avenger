use avenger_typst_label::{AvengerTypst, MathTypesetError, TypstEngineConfig};

fn engine() -> AvengerTypst {
    AvengerTypst::new(TypstEngineConfig::default()).unwrap()
}

#[test]
fn errors_when_source_exceeds_limit() {
    let mut options = avenger_typst_label::MathFragmentOptions::default();
    options.limits.max_source_bytes = 2;

    let err = engine().typeset_math_fragment("abc", &options).unwrap_err();
    assert_eq!(
        err,
        MathTypesetError::SourceTooLarge {
            actual: 3,
            limit: 2
        }
    );
}

#[test]
fn errors_when_math_span_count_exceeds_limit() {
    let mut options = avenger_typst_label::MathStringOptions::default();
    options.limits.max_math_spans = 1;

    let err = engine()
        .typeset_math_string("$x$ $y$", &options)
        .unwrap_err();
    assert_eq!(
        err,
        MathTypesetError::TooManyMathSpans {
            actual: 2,
            limit: 1
        }
    );
}

#[test]
fn empty_math_span_errors_with_source_range() {
    let err = engine()
        .typeset_math_string("before $$ after", &Default::default())
        .unwrap_err();
    assert_eq!(
        err,
        MathTypesetError::EmptyMathFragment { start: 8, end: 8 }
    );
}

#[test]
fn engine_error_for_one_span_reports_source_range() {
    let err = engine()
        .typeset_math_string("before $#let x = 1$ after", &Default::default())
        .unwrap_err();

    assert_eq!(
        err,
        MathTypesetError::UnsupportedSyntax {
            position: 8,
            message: "embedded Typst code is not allowed in math fragments"
        }
    );
}

#[test]
fn math_depth_limit_is_enforced() {
    let mut options = avenger_typst_label::MathFragmentOptions::default();
    options.limits.max_math_depth = 2;

    let err = engine()
        .typeset_math_fragment("a + (((x)))", &options)
        .unwrap_err();
    assert_eq!(
        err,
        MathTypesetError::MathDepthExceeded {
            actual: 3,
            limit: 2
        }
    );
}

#[test]
fn default_engine_produces_paths() {
    let engine = AvengerTypst::new(TypstEngineConfig::default()).unwrap();

    let artifact = engine
        .typeset_math_fragment("x^2 + y^2", &Default::default())
        .unwrap();

    assert!(
        artifact
            .paths
            .as_ref()
            .is_some_and(|paths| !paths.items.is_empty())
    );
}
