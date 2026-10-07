mod common;

use std::io::Read;
use std::sync::Arc;

use avenger_typst_label::{
    EngineOptions, LabelEngine, LabelError, LabelLimits, LabelOptions, LabelWarning,
    MissingFontPolicy, RegisteredFont, referenced_params,
};

fn engine() -> LabelEngine {
    LabelEngine::new(common::engine_options())
}

#[test]
fn errors_when_source_exceeds_limit() {
    let mut options = LabelOptions::default();
    options.limits.max_source_bytes = 2;

    let err = engine().compile("abc", &options).unwrap_err();
    assert_eq!(err, LabelError::SourceTooLarge { actual: 3, limit: 2 });
}

#[test]
fn errors_when_math_span_count_exceeds_limit() {
    let mut options = LabelOptions::default();
    options.limits.max_math_spans = 1;

    let err = engine().compile("$x$ $y$", &options).unwrap_err();
    assert_eq!(err, LabelError::TooManyMathSpans { actual: 2, limit: 1 });
}

#[test]
fn empty_math_is_an_empty_equation() {
    let label = engine().compile("before $$ after", &LabelOptions::default()).unwrap();
    assert!(label.flags.has_math);
}

#[test]
fn errors_are_upstreams_diagnostics() {
    // An embedded statement needs a semicolon before the closing dollar sign.
    let err = engine()
        .compile("before $#let x = 1$ after", &LabelOptions::default())
        .unwrap_err();
    assert_eq!(
        err,
        LabelError::Source {
            range: 18..18,
            message: "expected semicolon or line break".into(),
            hints: vec![],
        }
    );

    let err = engine().compile("a\n\nb", &LabelOptions::default()).unwrap_err();
    assert_eq!(
        err,
        LabelError::Source {
            range: 1..3,
            message: "paragraph breaks are not supported in labels".into(),
            hints: vec!["a label is one paragraph".into()],
        }
    );
}

#[test]
fn math_depth_limit_is_enforced() {
    let mut options = LabelOptions::default();
    options.limits.max_math_depth = 2;

    let err = engine().compile("$a + (((x)))$", &options).unwrap_err();
    assert_eq!(err, LabelError::MathDepthExceeded { actual: 3, limit: 2 });
}

#[test]
fn math_depth_counts_nested_constructs_not_brackets() {
    // A chain of fractions nests one level per slash, without any brackets.
    let source = format!("${}a$", "a/".repeat(40));
    let err = engine().compile(&source, &LabelOptions::default()).unwrap_err();
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
fn referenced_params_handle_deep_math() {
    let source = format!("${}a$", "a/".repeat(4000));
    assert_eq!(referenced_params(&source), Ok(vec![]));
}

#[test]
fn math_at_the_depth_limit_fits_a_wasm_sized_stack() {
    // Each construct nests to the default limit. Release builds must stay within 1 MiB, the
    // WebAssembly default stack; debug builds use far larger frames.
    let stack = if cfg!(debug_assertions) { 16 << 20 } else { 1 << 20 };
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

/// An engine with Lato as its only font, under a policy.
fn lato_engine(policy: MissingFontPolicy) -> LabelEngine {
    let mut options = EngineOptions::default();
    options.fonts.load_system_fonts = false;
    options.fonts.missing_font = policy;
    options.fonts.default_sans_serif_family = Some("Lato".to_string());
    let mut lato = Vec::new();
    brotli::Decompressor::new(avenger_fonts::LATO_REGULAR, 4096)
        .read_to_end(&mut lato)
        .unwrap();
    options.fonts.registered_fonts = vec![RegisteredFont::new(Arc::<[u8]>::from(lato))];
    LabelEngine::new(options)
}

#[test]
fn missing_fonts_follow_the_policy() {
    let mut options = LabelOptions::default();
    options.text.font_family = "Missing, Lato".into();

    // A list with an available family works under every policy; `Warn` reports the rest.
    let label = lato_engine(MissingFontPolicy::Error).compile("x", &options).unwrap();
    assert_eq!(label.warnings, []);
    let label = lato_engine(MissingFontPolicy::Warn).compile("x", &options).unwrap();
    assert_eq!(label.warnings, [LabelWarning::MissingFont { family: "Missing".into() }]);
    let label = lato_engine(MissingFontPolicy::Fallback)
        .compile("x", &options)
        .unwrap();
    assert_eq!(label.warnings, []);

    // `Error` fails when none of a list's families is available.
    options.text.font_family = "Missing".into();
    assert_eq!(
        lato_engine(MissingFontPolicy::Error)
            .compile_text("x", &options)
            .unwrap_err(),
        LabelError::MissingFont { family: "Missing".into() }
    );
    options.text.font_family = "Lato".into();
    options.math.font_family = "Lete Sans Math".into();
    assert_eq!(
        lato_engine(MissingFontPolicy::Error)
            .compile("$x$", &options)
            .unwrap_err(),
        LabelError::MissingFont { family: "Lete Sans Math".into() }
    );

    // Without a math font, math falls back to the text font, which upstream warns about.
    let label = lato_engine(MissingFontPolicy::Error)
        .compile("$x$", &LabelOptions::default())
        .unwrap();
    assert!(matches!(
        &label.warnings[..],
        [LabelWarning::Typst { message, .. }] if message == "current font is not designed for math"
    ));
}
