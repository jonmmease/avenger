//! Indentation as a user types, by `Document::on_type`, and the completion of a pipe operator with
//! its `|>`. Each text has the cursor `$0` just after the typed character.

mod common;

use avenger_sql_language::{CompletionKind, TextEdit};

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

#[test]
fn return_takes_the_depth_of_what_comes_next() {
    // After a pipe operator's keyword alone, its list goes on the lines after it.
    assert_eq!(
        typed("FROM t\n|> AGGREGATE\n$0", '\n'),
        "FROM t\n|> AGGREGATE\n     "
    );
    // After a comma of a list.
    assert_eq!(
        typed("FROM t\n|> AGGREGATE COUNT(*) AS n,\n$0", '\n'),
        "FROM t\n|> AGGREGATE COUNT(*) AS n,\n     "
    );
    // Inside the parentheses of a call.
    assert_eq!(
        typed("FROM t\n|> EXTEND f(\n$0", '\n'),
        "FROM t\n|> EXTEND f(\n     "
    );
    // After a query's line, the next pipe operator, at the query's depth.
    assert_eq!(edits("FROM t\n$0", '\n'), Vec::new());
}

#[test]
fn return_in_the_middle_of_a_line_indents_what_moved_down() {
    assert_eq!(
        typed("FROM t\n|> EXTEND f(a,\n$0b)", '\n'),
        "FROM t\n|> EXTEND f(a,\n     b)"
    );
    // A pipe operator that moves down takes its query's depth.
    assert_eq!(
        typed("FROM (\n  FROM t\n$0|> WHERE x)", '\n'),
        "FROM (\n  FROM t\n  |> WHERE x)"
    );
}

#[test]
fn a_line_s_first_token_moves_the_line() {
    // A closer goes to the depth of its bracket's line.
    assert_eq!(
        typed("FROM t\n|> EXTEND f(\n     a\n     )$0", ')'),
        "FROM t\n|> EXTEND f(\n     a\n   )"
    );
    // `|>` goes to the depth of its query.
    assert_eq!(
        typed("FROM (\n  FROM t\n      |>$0", '>'),
        "FROM (\n  FROM t\n  |>"
    );
    // A closer and a `>` in the middle of a line change nothing.
    assert_eq!(edits("FROM t\n  |> EXTEND f(a)$0", ')'), Vec::new());
    assert_eq!(edits("FROM t\n  |> WHERE a >$0", '>'), Vec::new());
}

#[test]
fn strings_comments_and_lines_at_their_depth_take_no_edit() {
    assert_eq!(edits("FROM t\n|> EXTEND 'a\n$0", '\n'), Vec::new());
    assert_eq!(edits("FROM t\n/* a\n$0", '\n'), Vec::new());
    assert_eq!(edits("FROM t\n|> AGGREGATE\n     $0", '\n'), Vec::new());
}

#[test]
fn return_empties_the_line_of_spaces_that_it_left() {
    assert_eq!(
        typed("FROM t\n|> AGGREGATE\n     \n$0", '\n'),
        "FROM t\n|> AGGREGATE\n\n     "
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
