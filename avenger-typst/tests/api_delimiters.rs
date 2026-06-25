use avenger_typst::{
    AvengerTypst, MathDelimiterOptions, MathDisplayHint, MathStringRun, MathTypesetError,
    TypstEngineConfig, UnmatchedDelimiterPolicy,
};

fn engine() -> AvengerTypst {
    AvengerTypst::new(TypstEngineConfig::default()).unwrap()
}

#[test]
fn parses_single_math_span() {
    let artifact = engine()
        .typeset_math_string("$x^2$", &Default::default())
        .unwrap();

    assert_eq!(artifact.runs.len(), 1);
    match &artifact.runs[0] {
        MathStringRun::Math(run) => {
            assert_eq!(run.source, "x^2");
            assert_eq!(run.byte_range, 1..4);
            assert_eq!(run.delimiter.full_range, 0..5);
        }
        other => panic!("expected math run, got {other:?}"),
    }
}

#[test]
fn parses_plain_then_math() {
    let artifact = engine()
        .typeset_math_string("area $x^2$", &Default::default())
        .unwrap();

    assert_eq!(artifact.runs.len(), 2);
    match &artifact.runs[0] {
        MathStringRun::Plain(run) => {
            assert_eq!(run.text, "area ");
            assert_eq!(run.byte_range, 0..5);
        }
        other => panic!("expected plain run, got {other:?}"),
    }
    match &artifact.runs[1] {
        MathStringRun::Math(run) => {
            assert_eq!(run.source, "x^2");
            assert_eq!(run.byte_range, 6..9);
        }
        other => panic!("expected math run, got {other:?}"),
    }
}

#[test]
fn parses_two_math_spans_with_plain_between() {
    let artifact = engine()
        .typeset_math_string("$x$ + $y$", &Default::default())
        .unwrap();

    assert_eq!(artifact.runs.len(), 3);
    assert!(matches!(artifact.runs[0], MathStringRun::Math(_)));
    match &artifact.runs[1] {
        MathStringRun::Plain(run) => assert_eq!(run.text, " + "),
        other => panic!("expected plain run, got {other:?}"),
    }
    assert!(matches!(artifact.runs[2], MathStringRun::Math(_)));
}

#[test]
fn escaped_dollar_is_literal_text() {
    let artifact = engine()
        .typeset_math_string("cost \\$5", &Default::default())
        .unwrap();

    assert_eq!(artifact.runs.len(), 1);
    match &artifact.runs[0] {
        MathStringRun::Plain(run) => assert_eq!(run.text, "cost $5"),
        other => panic!("expected plain run, got {other:?}"),
    }
}

#[test]
fn unmatched_dollar_can_be_literal() {
    let artifact = engine()
        .typeset_math_string("cost $5", &Default::default())
        .unwrap();

    assert_eq!(artifact.runs.len(), 1);
    match &artifact.runs[0] {
        MathStringRun::Plain(run) => assert_eq!(run.text, "cost $5"),
        other => panic!("expected plain run, got {other:?}"),
    }
}

#[test]
fn unmatched_dollar_can_error() {
    let mut options = avenger_typst::MathStringOptions::default();
    options.delimiters.unmatched = UnmatchedDelimiterPolicy::Error;
    let err = engine()
        .typeset_math_string("cost $5", &options)
        .unwrap_err();

    assert_eq!(err, MathTypesetError::UnmatchedDelimiter { position: 5 });
}

#[test]
fn display_delimiter_whitespace_is_recorded() {
    let mut options = avenger_typst::MathStringOptions::default();
    options.delimiters = MathDelimiterOptions {
        allow_display_style: true,
        ..MathDelimiterOptions::default()
    };
    let artifact = engine().typeset_math_string("$ x $", &options).unwrap();

    match &artifact.runs[0] {
        MathStringRun::Math(run) => {
            assert_eq!(run.delimiter.display_hint, MathDisplayHint::Display);
            assert_eq!(run.source, " x ");
        }
        other => panic!("expected math run, got {other:?}"),
    }
}
