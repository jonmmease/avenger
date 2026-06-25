use avenger_typst::{AvengerTypst, MathTypesetError, TypstEngineBackend, TypstEngineConfig};

#[cfg(not(feature = "vendor-typst"))]
use avenger_typst::TypstInitError;

fn engine() -> AvengerTypst {
    AvengerTypst::new(TypstEngineConfig::default()).unwrap()
}

#[test]
fn errors_when_source_exceeds_limit() {
    let mut options = avenger_typst::MathFragmentOptions::default();
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
    let mut options = avenger_typst::MathStringOptions::default();
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
    let mut options = avenger_typst::MathFragmentOptions::default();
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

#[cfg(not(feature = "vendor-typst"))]
#[test]
fn explicit_vendor_typst_backend_reports_unavailable_until_wired() {
    let err = AvengerTypst::new(TypstEngineConfig {
        backend: TypstEngineBackend::VendorTypst,
        ..Default::default()
    })
    .unwrap_err();

    assert_eq!(
        err,
        TypstInitError::BackendUnavailable("vendor-typst feature is not enabled")
    );
}

#[cfg(feature = "vendor-typst")]
#[test]
fn explicit_vendor_typst_backend_reaches_layout_boundary() {
    let engine = AvengerTypst::new(TypstEngineConfig {
        backend: TypstEngineBackend::VendorTypst,
        ..Default::default()
    })
    .unwrap();

    let err = engine
        .typeset_math_fragment("x^2 + y^2", &Default::default())
        .unwrap_err();

    assert_eq!(
        err,
        MathTypesetError::UnsupportedOutput("vendor-typst layout is not wired yet")
    );
}
