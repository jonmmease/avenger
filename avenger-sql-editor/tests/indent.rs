//! Smart indentation: Enter, the first token of a line, and Tab.

use avenger_sql_editor::indent::{enter, tab, typed, Tab};

/// `text` without its `‸`, and the character index of the `‸`.
fn at(text: &str) -> (String, usize) {
    let index = text.find('‸').expect("a cursor");
    (text.replacen('‸', "", 1), text[..index].chars().count())
}

/// `text` with a `‸` at the character index `cursor`.
fn mark(text: &str, cursor: usize) -> String {
    let byte = text
        .char_indices()
        .nth(cursor)
        .map_or(text.len(), |(byte, _)| byte);
    format!("{}‸{}", &text[..byte], &text[byte..])
}

fn press_enter(text: &str) -> String {
    let (text, cursor) = at(text);
    let (new, cursor) = enter(&text, cursor..cursor);
    mark(&new, cursor)
}

#[test]
fn enter_starts_a_line_at_its_depth() {
    // The next pipe operator, and an item after a comma.
    assert_eq!(press_enter("FROM t‸"), "FROM t\n‸");
    assert_eq!(
        press_enter("FROM t\n|> AGGREGATE mark_bar(\n     x => species,‸"),
        "FROM t\n|> AGGREGATE mark_bar(\n     x => species,\n     ‸"
    );
    // The spaces around the break go.
    assert_eq!(press_enter("FROM t   ‸  |> WHERE a"), "FROM t\n‸|> WHERE a");
    // Between an opener and its closer, the cursor goes on a line of its own.
    assert_eq!(
        press_enter("FROM t\n|> AGGREGATE mark_bar(‸)"),
        "FROM t\n|> AGGREGATE mark_bar(\n     ‸\n   )"
    );
    // After the last argument, the new line takes the depth of the items.
    assert_eq!(
        press_enter("FROM t\n|> AGGREGATE mark_bar(\n     angle => 23,‸\n   )"),
        "FROM t\n|> AGGREGATE mark_bar(\n     angle => 23,\n     ‸\n   )"
    );
    // Inside a string, the break goes in alone.
    assert_eq!(
        press_enter("FROM t |> SELECT '''a‸b'''"),
        "FROM t |> SELECT '''a\n‸b'''"
    );
}

#[test]
fn the_first_token_of_a_line_moves_the_line_to_its_depth() {
    let text = "FROM t\n|> AGGREGATE mark_bar(\n     x => species,\n     )";
    let cursor = text.chars().count();
    assert_eq!(
        typed(text, cursor, ')'),
        Some((
            "FROM t\n|> AGGREGATE mark_bar(\n     x => species,\n   )".to_string(),
            53
        ))
    );
    let text = "FROM t\n|> AGGREGATE mark_bar(x => species) AS chart\n     GROUP ";
    let (new, _) = typed(text, text.chars().count(), ' ').unwrap();
    assert!(new.ends_with("\n   GROUP "), "{new:?}");
    let text = "FROM t\n|> WHERE a\n     |>";
    let (new, _) = typed(text, text.chars().count(), '>').unwrap();
    assert!(new.ends_with("\n|>"), "{new:?}");
    // A token after the first one, and a line at its depth, stay.
    assert_eq!(typed("FROM t\n|> WHERE f(a)", 19, ')'), None);
    assert_eq!(typed("FROM t\n|>", 9, '>'), None);
}

fn press_tab(text: &str) -> Tab {
    let (text, cursor) = at(text);
    tab(&text, cursor..cursor)
}

#[test]
fn tab_re_indents_moves_to_the_first_word_or_completes() {
    // A line off its depth moves, and the cursor in its indentation goes to its first word.
    assert_eq!(
        press_tab("FROM t\n|> AGGREGATE mark_bar(\n‸ x => species,"),
        Tab::Edit(
            "FROM t\n|> AGGREGATE mark_bar(\n     x => species,".to_string(),
            35..35
        )
    );
    // In the indentation of a line at its depth, the cursor goes to the first word.
    assert_eq!(
        press_tab("FROM t\n|> AGGREGATE mark_bar(\n  ‸   x => species,"),
        Tab::Move(35)
    );
    // Otherwise, Tab completes, on an empty line at its depth too.
    assert_eq!(press_tab("FROM t\n|> WHERE spe‸"), Tab::Complete);
    assert_eq!(press_tab("FROM t\n‸"), Tab::Complete);
    // A selection over lines re-indents each, and stays when every line is at its depth.
    let text = "FROM t\n  |> WHERE a\n |> LIMIT 3";
    let Tab::Edit(new, selection) = tab(text, 0..text.len()) else {
        panic!("an edit")
    };
    assert_eq!(new, "FROM t\n|> WHERE a\n|> LIMIT 3");
    assert_eq!(selection, 0..new.len());
    assert_eq!(tab(&new, 0..new.len()), Tab::Stay);
    // The items under a line that moves follow it, so that a second Tab leaves the text.
    let text = "FROM t\n  |> AGGREGATE mark_bar(\n       x => species,\n     )";
    let Tab::Edit(new, selection) = tab(text, 0..text.len()) else {
        panic!("an edit")
    };
    assert_eq!(
        new,
        "FROM t\n|> AGGREGATE mark_bar(\n     x => species,\n   )"
    );
    assert_eq!(tab(&new, selection), Tab::Stay);
}
