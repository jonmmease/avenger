mod common;

use avenger_typst_label::{
    EngineOptions, LabelEngine, LabelError, LabelLimits, LabelLineHeight, LabelOptions,
    LabelWarning, LabelWidth, MissingFontPolicy, RegisteredFont, referenced_params,
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
fn errors_when_width_is_negative_or_not_finite() {
    let engine = engine();
    let mut options = LabelOptions::default();
    for width in [-1.0, f32::NAN, f32::INFINITY] {
        for width in [LabelWidth::Max(width), LabelWidth::Fixed(width)] {
            options.width = width;
            let results = [
                engine.compile("a", &options).map(|label| label.metrics),
                engine.compile_text("a", &options).map(|label| label.metrics),
                engine.measure("a", &options),
            ];
            for result in results {
                let err = result.unwrap_err();
                assert!(
                    matches!(err, LabelError::InvalidWidth { .. }),
                    "{width:?}: {err}"
                );
            }
        }
    }
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
    LabelEngine::new(lato_options(policy))
}

/// The options of an engine with Lato as its only font and its default sans-serif family,
/// under a policy.
fn lato_options(policy: MissingFontPolicy) -> EngineOptions {
    let mut options = EngineOptions::default();
    options.fonts.load_system_fonts = false;
    options.fonts.missing_font = policy;
    options.fonts.default_sans_serif_family = Some("Lato".to_string());
    let lato = avenger_fonts::decompress(avenger_fonts::LATO_REGULAR);
    options.fonts.registered_fonts = vec![RegisteredFont::new(lato)];
    options
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

#[test]
fn text_falls_back_without_its_default_family() {
    // Without a default sans-serif family, `sans-serif` names fontdb's default, Arial, which
    // the engine lacks. Text falls back to Lato, the only face, so a label lays out as with
    // Lato named, its plain line pitch included.
    let options = LabelOptions {
        line_height: LabelLineHeight::Relative(1.0),
        ..LabelOptions::default()
    };
    let source = "Fallback \\ faces";
    let unnamed = |policy| {
        let mut engine_options = lato_options(policy);
        engine_options.fonts.default_sans_serif_family = None;
        LabelEngine::new(engine_options)
    };
    for policy in [MissingFontPolicy::Fallback, MissingFontPolicy::Warn] {
        let (engine, named) = (unnamed(policy), lato_engine(policy));
        let label = engine.compile(source, &options).unwrap();
        assert_eq!(label.metrics, named.compile(source, &options).unwrap().metrics);
        assert_eq!(
            engine.font_metrics(&options.text).unwrap(),
            named.font_metrics(&options.text).unwrap()
        );
        let warnings = match policy {
            MissingFontPolicy::Warn => {
                vec![LabelWarning::MissingFont { family: "Arial".into() }]
            }
            _ => vec![],
        };
        assert_eq!(label.warnings, warnings);
    }

    // `Error` still fails when none of the text's families is available.
    assert_eq!(
        unnamed(MissingFontPolicy::Error)
            .compile(source, &options)
            .unwrap_err(),
        LabelError::MissingFont { family: "sans-serif".into() }
    );

    // An engine without faces still lays labels out, with the leading as their line pitch.
    let mut faceless = EngineOptions::default();
    faceless.fonts.load_system_fonts = false;
    let label = LabelEngine::new(faceless).compile(source, &options).unwrap();
    assert!((label.metrics.line_pitch - 0.65 * options.text.font_size).abs() < 1e-4);
}
