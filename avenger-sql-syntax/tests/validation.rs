//! The checks of the validation pass: the rules that GoogleSQL checks after its parser, with
//! the message and the range of each diagnostic.

mod common;

use avenger_sql_syntax::{parse, DiagnosticCode};
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
fn validation_leaves_the_tree_as_the_parser_builds_it() {
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
fn the_codes_of_the_checks() {
    let code = |text: &str| parse(text).diagnostics()[0].code;
    assert_eq!(code("FROM t |> SELECT `` "), DiagnosticCode::InvalidToken);
    assert_eq!(code("DECLARE x"), DiagnosticCode::InvalidStatement);
    assert_eq!(
        code("FROM t |> CROSS JOIN u ON true"),
        DiagnosticCode::Expected
    );
}

#[test]
fn a_bare_star_is_no_item_of_an_extend() {
    rejects("FROM t |> EXTEND a, *", "takes no `*`", "*");
    accepts("FROM t |> EXTEND a, t.*");
}
