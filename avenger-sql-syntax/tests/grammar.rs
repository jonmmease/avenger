//! The trees of the corpora have the shapes that `sql.ungram` describes.

mod common;

use avenger_sql_syntax::{ast::grammar_violations, parse, parse_module, SyntaxNode};
use common::*;

fn conforms(root: &SyntaxNode) -> Vec<String> {
    grammar_violations(root)
}

#[test]
fn the_trees_of_the_corpora_follow_the_grammar() {
    let mut violations = Vec::new();
    let mut scripts: Vec<String> = [OPERATORS, STATEMENTS, SOURCES]
        .iter()
        .map(|text| text.to_string())
        .collect();
    scripts.extend(QUERIES.iter().map(|text| text.to_string()));
    scripts.extend(
        EXPRESSIONS
            .iter()
            .map(|text| format!("FROM t |> WHERE {text}")),
    );
    for text in &scripts {
        for violation in conforms(&parse(text).syntax()) {
            violations.push(format!("{violation} in {text}"));
        }
    }
    for violation in conforms(&parse_module(MODULE).syntax()) {
        violations.push(format!("{violation} in the module"));
    }
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

#[test]
fn the_trees_of_rejected_texts_follow_the_grammar_outside_their_errors() {
    let mut violations = Vec::new();
    for (text, _, _) in REJECTED.iter().chain(VALIDATED) {
        for violation in conforms(&parse(text).syntax()) {
            violations.push(format!("{violation} in {text}"));
        }
    }
    for (text, _, _) in REJECTED_MODULES.iter().chain(VALIDATED_MODULES) {
        for violation in conforms(&parse_module(text).syntax()) {
            violations.push(format!("{violation} in {text}"));
        }
    }
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}
