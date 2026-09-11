mod common;

use avenger_typst_label::{LabelEngine, LabelError, LabelOptions};

fn engine() -> LabelEngine {
    LabelEngine::new(common::engine_options()).unwrap()
}

#[test]
fn errors_when_source_exceeds_limit() {
    let mut options = LabelOptions::default();
    options.limits.max_source_bytes = 2;

    let err = engine().compile("abc", &options).unwrap_err();
    assert_eq!(
        err,
        LabelError::SourceTooLarge {
            actual: 3,
            limit: 2
        }
    );
}

#[test]
fn errors_when_math_span_count_exceeds_limit() {
    let mut options = LabelOptions::default();
    options.limits.max_math_spans = 1;

    let err = engine().compile("$x$ $y$", &options).unwrap_err();
    assert_eq!(
        err,
        LabelError::TooManyMathSpans {
            actual: 2,
            limit: 1
        }
    );
}

#[test]
fn empty_math_span_errors_with_source_range() {
    let err = engine()
        .compile("before $$ after", &LabelOptions::default())
        .unwrap_err();
    assert_eq!(err, LabelError::EmptyMathFragment { start: 8, end: 8 });
}

#[test]
fn embedded_code_in_math_uses_canonical_typst_error() {
    let err = engine()
        .compile("before $#let x = 1$ after", &LabelOptions::default())
        .unwrap_err();

    assert!(matches!(err, LabelError::Syntax { .. }));
}

#[test]
fn math_depth_limit_is_enforced() {
    let mut options = LabelOptions::default();
    options.limits.max_math_depth = 2;

    let err = engine().compile("$a + (((x)))$", &options).unwrap_err();
    assert_eq!(
        err,
        LabelError::MathDepthExceeded {
            actual: 3,
            limit: 2
        }
    );
}

#[test]
fn default_engine_produces_paths() {
    let label = engine()
        .compile("$x^2 + y^2$", &LabelOptions::default())
        .unwrap();
    let svg = avenger_typst_label::svg_items(&label, &Default::default()).unwrap();

    assert!(
        svg.items
            .iter()
            .any(|(_, item)| matches!(item, avenger_typst_label::LabelFrameItem::Shape(_)))
    );
}
