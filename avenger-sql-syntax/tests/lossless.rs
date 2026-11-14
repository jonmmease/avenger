//! Every text parses into a tree that holds it, without a panic, and a node starts and ends at
//! a significant token.

mod common;

use avenger_sql_syntax::{expected_at, parse, parse_module, SyntaxKind, SyntaxNode};
use common::*;

fn inputs() -> Vec<String> {
    let mut inputs: Vec<String> = [OPERATORS, STATEMENTS, SOURCES, MODULE]
        .iter()
        .map(|text| text.to_string())
        .collect();
    inputs.extend(QUERIES.iter().map(|text| text.to_string()));
    inputs.extend(
        EXPRESSIONS
            .iter()
            .map(|text| format!("FROM t |> WHERE {text}")),
    );
    inputs.extend(REJECTED.iter().map(|(text, _, _)| text.to_string()));
    inputs.extend(REJECTED_MODULES.iter().map(|(text, _, _)| text.to_string()));
    // Prefixes of the longer inputs, as an editor sees them while a user types.
    for text in [OPERATORS, STATEMENTS, SOURCES] {
        let mut end = 0;
        while end < text.len() {
            inputs.push(text[..end].to_string());
            end += 7;
            while !text.is_char_boundary(end.min(text.len())) {
                end += 1;
            }
        }
    }
    inputs
}

/// Checks that no node starts or ends with whitespace, and that only a node that holds its
/// comments starts with one.
fn check_edges(node: &SyntaxNode) {
    for child in node.descendants() {
        if child.kind() == SyntaxKind::SCRIPT || child.kind() == SyntaxKind::MODULE {
            continue;
        }
        let first = child.first_token();
        let last = child.last_token();
        if let Some(first) = first {
            assert_ne!(
                first.kind(),
                SyntaxKind::WHITESPACE,
                "{child:?} starts with whitespace"
            );
        }
        if let Some(last) = last {
            assert!(!last.kind().is_trivia(), "{child:?} ends with trivia");
        }
    }
}

#[test]
fn every_input_round_trips() {
    for text in inputs() {
        for parse in [parse(&text), parse_module(&text)] {
            let root = parse.syntax();
            assert_eq!(root.text().to_string(), text);
            check_edges(&root);
            for diagnostic in parse.diagnostics() {
                assert!(usize::from(diagnostic.range.end()) <= text.len());
            }
        }
    }
}

#[test]
fn every_cursor_has_expectations_without_a_panic() {
    for text in [OPERATORS, STATEMENTS, SOURCES] {
        for offset in (0..=text.len()).filter(|offset| text.is_char_boundary(*offset)) {
            expected_at(text, (offset as u32).into());
        }
    }
}

#[test]
fn garbage_round_trips() {
    // The pieces, separated by single spaces, the line end among them.
    let pieces: Vec<&str> = "FROM |> ( ) SELECT , ' ` WHERE a 1 . * JOIN ; AS NOT < > [ ] CREATE \
        TABLE /* -- \n @ => UNION ALL AGGREGATE GROUP BY \" \\ é \u{0}"
        .split(' ')
        .collect();
    assert_eq!(pieces.len(), 37);
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    for _ in 0..3000 {
        let mut text = String::new();
        for _ in 0..(state % 24) {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            text.push_str(pieces[(state % pieces.len() as u64) as usize]);
            if state.is_multiple_of(3) {
                text.push(' ');
            }
        }
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let parse = parse(&text);
        assert_eq!(parse.syntax().text().to_string(), text, "{text:?}");
        check_edges(&parse.syntax());
    }
}
