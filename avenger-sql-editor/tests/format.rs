//! The Format button: the cursor stays on its token when the text moves.

use avenger_sql_editor::format::cursor;

#[test]
fn the_cursor_stays_after_the_same_characters() {
    let old = "FROM t |> AGGREGATE mark_bar(x => a, y => b, )";
    let new = "FROM t\n|> AGGREGATE mark_bar(\n     x => a,\n     y => b,\n   )";
    // Right after `mark_bar`, and right before the `y` in the whitespace before it.
    let after = old.find("mark_bar").unwrap() + "mark_bar".len();
    assert_eq!(cursor(old, new, after), new.find("(").unwrap());
    let before_y = old.find("y =>").unwrap();
    assert_eq!(cursor(old, new, before_y), new.find("y =>").unwrap());
    // In the whitespace before `|>`, the cursor goes before it, after the line break.
    assert_eq!(cursor(old, new, 7), new.find("|>").unwrap());
    assert_eq!(cursor(old, new, 0), 0);
    assert_eq!(cursor(old, new, old.len()), new.len());
}
