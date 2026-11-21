//! Comments toggled on the lines of a selection.

use avenger_sql_editor::comment::toggle;

#[test]
fn a_cursor_comments_its_line_and_then_uncomments_it() {
    let text = "FROM penguins\n|> AGGREGATE mark_bar(x => species)";
    let (commented, cursor) = toggle(text, 20..20);
    assert_eq!(
        commented,
        "FROM penguins\n-- |> AGGREGATE mark_bar(x => species)"
    );
    assert_eq!(cursor, 23..23, "the cursor stays on its character");
    let (back, cursor) = toggle(&commented, cursor);
    assert_eq!(back, text);
    assert_eq!(cursor, 20..20);
}

#[test]
fn a_selection_comments_its_lines_at_their_smallest_indentation() {
    let text = "FROM t\n|> AGGREGATE mark_bar(\n     x => a,\n\n     y => b)\nafter";
    // From inside the second line to the start of the last line, which stays as it is.
    let start = text.find("AGGREGATE").unwrap();
    let end = text.find("after").unwrap();
    let (commented, selection) = toggle(text, start..end);
    assert_eq!(
        commented,
        "FROM t\n-- |> AGGREGATE mark_bar(\n--      x => a,\n\n--      y => b)\nafter"
    );
    assert_eq!(
        &commented[selection.clone()],
        &commented[start + 3..end + 9]
    );
    let (back, _) = toggle(&commented, selection);
    assert_eq!(back, text);
}

#[test]
fn lines_that_are_not_all_comments_take_a_comment() {
    let text = "-- a\nb";
    let (commented, _) = toggle(text, 0..text.len());
    assert_eq!(commented, "-- -- a\n-- b");
    // A comment without a space after `--` loses only the `--`.
    let (uncommented, _) = toggle("  --a\n  -- b", 0..12);
    assert_eq!(uncommented, "  a\n  b");
}

#[test]
fn an_empty_line_takes_a_comment() {
    let (commented, cursor) = toggle("a\n\nb", 2..2);
    assert_eq!(commented, "a\n-- \nb");
    assert_eq!(cursor, 5..5);
}
