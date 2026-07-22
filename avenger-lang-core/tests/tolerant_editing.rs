use avenger_lang_core::{
    SourceFile, SourceId, SourceOrigin,
    sql::LosslessTokenKind,
    syntax::{TolerantParsedFile, parse_file_tolerant},
};

const VALID: &str = r#"avenger 1;

chart cartesian as chart {
    param as width {
        type: float64;
        default: 640.0;
    }
    table sql as movies {
        sql: FROM vega.movies AS m SELECT m.*;
    }
    mark symbol as points {
        x: "Horsepower";
        size: [$width, 2.0];
    }
}
"#;

fn parse(text: &str) -> TolerantParsedFile {
    parse_file_tolerant(&SourceFile::new(
        SourceId::new(9),
        SourceOrigin::Memory("editing-corpus".into()),
        text.to_owned(),
    ))
}

fn assert_lossless(text: &str, parsed: &TolerantParsedFile) {
    let reconstructed = parsed
        .tokens
        .tokens()
        .iter()
        .filter(|token| token.kind() != LosslessTokenKind::Eof)
        .map(|token| parsed.tokens.raw(token))
        .collect::<String>();
    assert_eq!(reconstructed, text);
    assert!(parsed.nodes.len() <= parsed.tokens.tokens().len() * 4 + 32);
}

#[test]
fn deleting_each_delimiter_recovers_deterministically() {
    for (offset, character) in VALID.char_indices() {
        if !"{}[]();:".contains(character) {
            continue;
        }
        let mut edited = VALID.to_owned();
        edited.replace_range(offset..offset + character.len_utf8(), "");
        let first = parse(&edited);
        let second = parse(&edited);
        assert_lossless(&edited, &first);
        let first_diagnostics = first
            .diagnostics
            .iter()
            .map(|diagnostic| {
                (
                    &diagnostic.code,
                    &diagnostic.message,
                    &diagnostic.primary,
                    &diagnostic.secondary,
                )
            })
            .collect::<Vec<_>>();
        let second_diagnostics = second
            .diagnostics
            .iter()
            .map(|diagnostic| {
                (
                    &diagnostic.code,
                    &diagnostic.message,
                    &diagnostic.primary,
                    &diagnostic.secondary,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(first_diagnostics, second_diagnostics);
    }
}

#[test]
fn every_typed_prefix_terminates_and_is_lossless() {
    for end in VALID
        .char_indices()
        .map(|(offset, _)| offset)
        .chain(std::iter::once(VALID.len()))
    {
        let prefix = &VALID[..end];
        let parsed = parse(prefix);
        assert_lossless(prefix, &parsed);
    }
}

#[test]
fn malformed_lexical_forms_keep_the_valid_prefix() {
    for text in [
        "avenger 1; chart cartesian as c { x: 'open",
        "avenger 1; chart cartesian as c { x: /* open",
        "avenger 1; chart cartesian as c { x: $$open",
        "avenger 1; chart cartesian as c { x: 1; § y: 2; }",
    ] {
        let parsed = parse(text);
        assert_lossless(text, &parsed);
        assert!(parsed.strict.is_none());
    }
}

#[test]
fn arbitrary_unicode_inputs_terminate_without_panicking() {
    let alphabet = [
        'a', ' ', '{', '}', '[', ']', ';', ':', '\'', '😀', '\n', '$', '§',
    ];
    let mut state = 0x5eed_u64;
    for _ in 0..512 {
        let mut text = String::new();
        for _ in 0..64 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            text.push(alphabet[(state as usize) % alphabet.len()]);
        }
        let parsed = parse(&text);
        assert_lossless(&text, &parsed);
    }
}
