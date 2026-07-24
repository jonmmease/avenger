use avenger_lang_core::{
    SourceFile, SourceId, SourceOrigin,
    sql::LosslessTokenKind,
    syntax::{
        SqlIslandContext, SyntaxLimits, TolerantParsedFile, TolerantSyntaxNodeKind,
        parse_file_tolerant, parse_file_tolerant_with_limits,
    },
};

const VALID: &str = r#"avenger 1;

chart cartesian as chart {
    param float64 as width {
        value: 640.0;
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

#[test]
fn configured_limits_collapse_work_without_losing_source_ownership() {
    let parsed = parse_file_tolerant_with_limits(
        &SourceFile::new(
            SourceId::new(9),
            SourceOrigin::Memory("limited".into()),
            VALID.to_owned(),
        ),
        SyntaxLimits {
            max_tokens: 12,
            max_nesting_depth: 1,
            max_declarations: 1,
            ..SyntaxLimits::default()
        },
    );
    assert_lossless(VALID, &parsed);
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "AVENGER-TOKEN-003")
    );
}

#[test]
fn module_tokens_and_qualified_sql_relations_keep_independent_boundaries() {
    let text = r#"avenger 1;
import { movies as films } from './data.avenger';
import * as acme from 'native:acme';

chart acme.cartesian as chart {
  table sql as summary {
    sql: FROM samples.movies AS m
         SELECT m.genre, count(*) AS total
         GROUP BY m.genre;
  }
}
"#;
    let parsed = parse(text);
    assert_lossless(text, &parsed);
    assert_eq!(parsed.module_syntax.imports.len(), 2);
    assert_eq!(parsed.module_syntax.items.len(), 1);
    assert_eq!(
        parsed.module_syntax.items[0]
            .kind_segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<Vec<_>>(),
        ["acme", "cartesian"]
    );
    let query = parsed
        .nodes
        .iter()
        .find(|node| {
            matches!(
                node.kind,
                TolerantSyntaxNodeKind::SqlIsland {
                    context: SqlIslandContext::QueryProperty
                }
            )
        })
        .expect("query island");
    let query_text = &text[query.span.range.as_range()];
    assert!(query_text.contains("samples.movies"));
    assert!(query_text.contains("GROUP BY"));
}
