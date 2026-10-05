mod common;

use avenger_typst_label::{LabelEngine, LabelError, LabelLimits, LabelOptions, referenced_params};

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
fn math_depth_counts_nested_constructs_not_brackets() {
    // A chain of fractions nests one level per slash, without any brackets.
    let source = format!("${}a$", "a/".repeat(40));
    let err = engine()
        .compile(&source, &LabelOptions::default())
        .unwrap_err();
    assert_eq!(
        err,
        LabelError::MathDepthExceeded {
            actual: 40,
            limit: LabelLimits::default().max_math_depth,
        }
    );

    // Brackets inside a string do not nest.
    let mut options = LabelOptions::default();
    options.limits.max_math_depth = 2;
    engine().compile("$\"(((\" x$", &options).unwrap();
}

#[test]
fn referenced_params_rejects_excessive_math_nesting() {
    let source = format!("${}a$", "a/".repeat(4000));
    assert!(matches!(
        referenced_params(&source),
        Err(LabelError::MathDepthExceeded { .. })
    ));
}

#[test]
fn math_at_the_depth_limit_fits_a_wasm_sized_stack() {
    // Each construct nests to the default limit. Release builds must stay within 1 MiB, the
    // WebAssembly default stack; debug builds use far larger frames.
    let stack = if cfg!(debug_assertions) {
        16 << 20
    } else {
        1 << 20
    };
    let depth = LabelLimits::default().max_math_depth;
    let nest = |open: &str, inner: &str, close: &str| {
        format!("${}{inner}{}$", open.repeat(depth), close.repeat(depth))
    };
    let sources = [
        nest("(", "x", ")"),
        nest("[", "x", "]"),
        nest("sqrt(", "x", ")"),
        nest("frac(", "x", ", 1)"),
        nest("x^(", "x", ")"),
        nest("abs(", "x", ")"),
        nest("hat(", "x", ")"),
        nest("cancel(", "x", ")"),
        nest("bold(", "x", ")"),
        nest("√", "x", ""),
        format!("${}a$", "a/".repeat(depth)),
    ];
    std::thread::Builder::new()
        .stack_size(stack)
        .spawn(move || {
            let engine = engine();
            for source in &sources {
                if let Err(err) = engine.compile(source, &LabelOptions::default()) {
                    panic!("{source}: {err}");
                }
            }
        })
        .unwrap()
        .join()
        .expect("labels at the depth limit should not overflow the stack");
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
