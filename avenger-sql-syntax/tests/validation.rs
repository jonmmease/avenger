//! The checks of the validation pass: the rules that GoogleSQL checks after its parser, with
//! the message and the range of each diagnostic.

mod common;

use avenger_sql_syntax::parse;
use common::*;

#[test]
fn rejects_what_googlesql_rejects_after_parsing() {
    for (text, message, at) in VALIDATED {
        rejects(text, message, at);
    }
    for (text, message, at) in VALIDATED_MODULES {
        rejects_module(text, message, at);
    }
}

#[test]
fn the_parser_accepts_what_only_validation_rejects() {
    // A text that only validation rejects has a tree without ERROR nodes.
    for (text, _, _) in VALIDATED {
        let root = parse(text).syntax();
        assert!(
            nodes(&root, avenger_sql_syntax::SyntaxKind::ERROR).is_empty(),
            "{text}: {root:#?}"
        );
    }
}

#[test]
fn an_unclosed_string_adds_no_errors_of_its_literal() {
    // The open quote ends at the line end, and the lexer reads `b' AS s` as the start of an
    // unclosed bytes literal. Each unclosed part has its lexer error, and the literal has no
    // others.
    let text = "FROM t |> SELECT 'a\nb' AS s";
    let messages: Vec<String> = parse(text)
        .diagnostics()
        .iter()
        .map(|diagnostic| diagnostic.message.clone())
        .collect();
    assert_eq!(
        messages,
        [
            "this quoted literal has no closing quote",
            "this quoted literal has no closing quote"
        ]
    );
}
