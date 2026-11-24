//! Indentation as a user types, by `Document::on_type`, and the completion of a pipe operator with
//! its `|>`. Each text has the cursor `$0` just after the typed character.

mod common;

use avenger_sql_language::{CompletionKind, TextEdit};
use avenger_sql_syntax::TextRange;

/// The text after the edits of typing `ch` before the cursor of `text`.
fn typed(text: &str, ch: char) -> String {
    let (document, offset) = common::at_cursor(text);
    let mut edits: Vec<TextEdit> = document.on_type(offset, ch);
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start()));
    let mut out = document.text().to_string();
    for edit in edits {
        out.replace_range(
            std::ops::Range::<usize>::from(edit.range),
            &edit.replacement,
        );
    }
    out
}

/// The edits of typing `ch` before the cursor of `text`.
fn edits(text: &str, ch: char) -> Vec<TextEdit> {
    let (document, offset) = common::at_cursor(text);
    document.on_type(offset, ch)
}

/// The edit at `start..end` of the text, with `replacement`.
fn edit(start: u32, end: u32, replacement: &str) -> TextEdit {
    TextEdit::new(TextRange::new(start.into(), end.into()), replacement)
}

#[test]
fn return_corrects_the_editor_s_indentation_to_the_depth_of_what_comes_next() {
    // The editor's own indentation put the cursor at 2. After a pipe operator's keyword alone,
    // its list goes on the lines after it, 4 deeper than the `|>`.
    assert_eq!(
        typed("FROM t\n|> AGGREGATE\n  $0", '\n'),
        "FROM t\n|> AGGREGATE\n    "
    );
    // After a comma of a list.
    assert_eq!(
        typed("FROM t\n|> AGGREGATE COUNT(*) AS n,\n  $0", '\n'),
        "FROM t\n|> AGGREGATE COUNT(*) AS n,\n    "
    );
    // Inside the parentheses of a call.
    assert_eq!(
        typed("FROM t\n|> EXTEND f(\n  $0", '\n'),
        "FROM t\n|> EXTEND f(\n    "
    );
    // After a query's line, the next pipe operator, at the query's depth.
    assert_eq!(edits("FROM t\n$0", '\n'), Vec::new());
    assert_eq!(typed("FROM t\n    $0", '\n'), "FROM t\n");
}

#[test]
fn return_inserts_no_indentation_at_the_cursor() {
    // An editor applies an insertion at the cursor after it, so the cursor would stay at column
    // 0, behind the indentation.
    assert_eq!(edits("FROM t\n|> AGGREGATE\n$0", '\n'), Vec::new());
    assert_eq!(
        edits(
            "FROM t\n|> AGGREGATE mark_symbol(\n    x => a, y => b\n$0) AS chart",
            '\n'
        ),
        Vec::new()
    );
}

#[test]
fn return_in_the_middle_of_a_line_indents_what_moved_down() {
    assert_eq!(
        typed("FROM t\n|> EXTEND f(a,\n  $0b)", '\n'),
        "FROM t\n|> EXTEND f(a,\n    b)"
    );
    // A pipe operator that moves down takes its query's depth.
    assert_eq!(
        typed("FROM (\n    FROM t\n        $0|> WHERE x)", '\n'),
        "FROM (\n    FROM t\n    |> WHERE x)"
    );
}

#[test]
fn return_between_autoclosed_brackets_indents_the_closer_below() {
    assert_eq!(
        typed("FROM t\n|> WHERE f(\n    $0\n)", '\n'),
        "FROM t\n|> WHERE f(\n    \n   )"
    );
    // With the cursor's line at column 0, the closer alone.
    assert_eq!(
        typed("FROM t\n|> WHERE f(\n$0\n)", '\n'),
        "FROM t\n|> WHERE f(\n\n   )"
    );
}

#[test]
fn a_line_s_first_token_moves_the_line() {
    // The closer of a bracket that a pipe operator's line opens goes to the operator's keyword.
    assert_eq!(
        typed("FROM t\n|> EXTEND f(\n    a\n    )$0", ')'),
        "FROM t\n|> EXTEND f(\n    a\n   )"
    );
    // `|>` goes to the depth of its query.
    assert_eq!(
        typed("FROM (\n    FROM t\n      |>$0", '>'),
        "FROM (\n    FROM t\n    |>"
    );
    // A closer and a `>` in the middle of a line change nothing.
    assert_eq!(edits("FROM t\n|> EXTEND f(a)$0", ')'), Vec::new());
    assert_eq!(edits("FROM t\n|> WHERE a >$0", '>'), Vec::new());
}

#[test]
fn the_space_after_a_clause_s_keywords_moves_the_line() {
    // `GROUP BY` after the items, at the items' depth, goes to the operator's keyword.
    let text = "FROM t\n|> AGGREGATE\n    count(*) AS n,\n    GROUP BY $0";
    assert_eq!(
        typed(text, ' '),
        "FROM t\n|> AGGREGATE\n    count(*) AS n,\n   GROUP BY "
    );
    // On the space after `GROUP` too, and after a join's `ON`.
    assert_eq!(
        typed("FROM t\n|> AGGREGATE count(*) AS n\nGROUP $0", ' '),
        "FROM t\n|> AGGREGATE count(*) AS n\n   GROUP "
    );
    assert_eq!(
        typed("FROM t\n|> JOIN u\non $0", ' '),
        "FROM t\n|> JOIN u\n   on "
    );
    // After an `AND` or an `OR` that starts a line, the chain continues.
    assert_eq!(
        typed("FROM t\n|> WHERE x > 0\nAND $0", ' '),
        "FROM t\n|> WHERE x > 0\n    AND "
    );
    // A space after other words, or in the middle of a clause, changes nothing.
    assert_eq!(edits("FROM t\n|> AGGREGATE\n    count $0", ' '), Vec::new());
    assert_eq!(
        edits("FROM t\n|> AGGREGATE n\nGROUP BY a $0", ' '),
        Vec::new()
    );
}

#[test]
fn an_edit_inserts_or_deletes_spaces_at_the_start_of_the_line() {
    // The cursor, after the line's text, moves with it.
    assert_eq!(
        edits("FROM t\n|> EXTEND f(\n    a\n)$0", ')'),
        vec![edit(26, 26, "   ")]
    );
    assert_eq!(
        edits("FROM t\n|> EXTEND f(\n    a\n    )$0", ')'),
        vec![edit(26, 27, "")]
    );
    // Tabs become spaces.
    assert_eq!(
        edits("FROM t\n|> EXTEND f(\n    a\n\t)$0", ')'),
        vec![edit(26, 27, "   ")]
    );
}

#[test]
fn strings_comments_and_lines_at_their_depth_take_no_edit() {
    assert_eq!(edits("FROM t\n|> EXTEND 'a\n$0", '\n'), Vec::new());
    assert_eq!(edits("FROM t\n/* a\n$0", '\n'), Vec::new());
    assert_eq!(edits("FROM t\n|> AGGREGATE\n    $0", '\n'), Vec::new());
}

#[test]
fn return_empties_the_line_of_spaces_that_it_left() {
    assert_eq!(
        typed("FROM t\n|> AGGREGATE\n    \n  $0", '\n'),
        "FROM t\n|> AGGREGATE\n\n    "
    );
}

#[test]
fn a_pipe_operator_completes_with_its_pipe_where_one_can_come() {
    let (document, offset) = common::at_cursor("FROM t\nwh$0");
    let completions = document.completions(offset);
    let item = (completions.items.iter())
        .find(|item| item.label == "|> WHERE")
        .expect("the item `|> WHERE`");
    assert_eq!(item.insert, "|> WHERE ");
    assert_eq!(item.filter.as_deref(), Some("WHERE"));
    assert_eq!(item.kind, CompletionKind::Keyword);
    // After a `|>`, the operators' words alone, and in an expression, no pipe.
    for text in ["FROM t\n|> $0", "FROM t\n|> WHERE f($0"] {
        let (document, offset) = common::at_cursor(text);
        let labels: Vec<String> = (document.completions(offset).items.into_iter())
            .map(|item| item.label)
            .collect();
        assert!(
            !labels.iter().any(|label| label.starts_with("|>")),
            "{text}: {labels:?}"
        );
    }
}
