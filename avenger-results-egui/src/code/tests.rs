//! The layout of a query's lines with its open stages.

use super::{layout, Code, CodeOperator, Item, Row};

/// The code of `text`, whose top-level operators are the `|>`s that start its lines, or that
/// follow `FROM t ` on its first line.
fn code(text: &str) -> Code {
    let operators = (text.match_indices("|>").enumerate())
        .map(|(index, (pipe, _))| CodeOperator {
            ordinal: index + 1,
            pipe,
            name: "|> WHERE".into(),
        })
        .collect();
    Code {
        text: text.to_string(),
        operators,
        error: None,
    }
}

#[test]
fn closed_stages_keep_their_lines() {
    let code = code("FROM t\n|> WHERE x\n");
    assert_eq!(
        layout(&code, |_| false),
        vec![
            Row::Line(vec![Item::Text(0..6)]),
            Row::Line(vec![Item::Pipe(1), Item::Text(9..17)]),
        ]
    );
}

#[test]
fn an_open_stage_takes_a_row_and_moves_its_operator_below() {
    let code = code("FROM t\n    |> WHERE x\n");
    assert_eq!(
        layout(&code, |_| true),
        vec![
            Row::Line(vec![Item::Text(0..6)]),
            // The `|>` keeps its column after the spaces before it.
            Row::Stage {
                indent: 4,
                ordinal: 1
            },
            Row::Line(vec![Item::Space(6), Item::Text(13..21)]),
        ]
    );
    // After other text, the text keeps its line, and the `|>` starts the stage's row.
    let code = super::Code {
        text: "FROM t |> WHERE x".into(),
        ..code
    };
    let code = Code {
        operators: vec![CodeOperator {
            ordinal: 1,
            pipe: 7,
            name: "|> WHERE".into(),
        }],
        ..code
    };
    assert_eq!(
        layout(&code, |_| true),
        vec![
            Row::Line(vec![Item::Text(0..7)]),
            Row::Stage {
                indent: 0,
                ordinal: 1
            },
            Row::Line(vec![Item::Space(2), Item::Text(9..17)]),
        ]
    );
}
