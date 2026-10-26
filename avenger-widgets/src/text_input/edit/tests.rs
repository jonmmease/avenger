//! Edits and caret movements over words, multi-byte characters and emoji.

use super::*;

/// An edit of `text` with the caret, and the anchor, at `caret`.
fn at(text: &str, caret: usize) -> Edit {
    Edit {
        text: text.to_string(),
        anchor: caret,
        caret,
    }
}

#[test]
fn inserts_at_the_caret_and_in_place_of_the_selection() {
    let mut edit = at("star wars", 4);
    edit.insert("ry");
    assert_eq!((edit.text.as_str(), edit.caret), ("starry wars", 6));

    edit.select(7..11);
    edit.insert("night");
    assert_eq!(
        (edit.text.as_str(), edit.selection()),
        ("starry night", 12..12)
    );
}

#[test]
fn an_insertion_that_joins_the_next_grapheme_places_the_caret_after_it() {
    let mut edit = at("💻", 0);
    edit.insert("👩\u{200d}");
    assert_eq!(
        (edit.text.as_str(), edit.caret),
        ("👩\u{200d}💻", edit.text.len())
    );
    edit.delete(Reach::Grapheme, false);
    assert_eq!(edit.text, "");
}

#[test]
fn moves_and_deletes_by_grapheme_over_multi_byte_characters_and_emoji() {
    // "é" is 2 bytes, and the family emoji is 18 bytes in one grapheme.
    let text = "café 👨‍👩‍👧!";
    let mut edit = at(text, text.len());
    edit.move_caret(Reach::Grapheme, false, false);
    assert_eq!(edit.caret, text.len() - 1);
    edit.move_caret(Reach::Grapheme, false, false);
    assert_eq!(edit.caret, 6);

    edit.delete(Reach::Grapheme, false);
    assert_eq!((edit.text.as_str(), edit.caret), ("café👨‍👩‍👧!", 5));
    edit.delete(Reach::Grapheme, false);
    assert_eq!((edit.text.as_str(), edit.caret), ("caf👨‍👩‍👧!", 3));
    edit.delete(Reach::Grapheme, true);
    assert_eq!((edit.text.as_str(), edit.caret), ("caf!", 3));
}

#[test]
fn moves_by_word_to_word_starts_backward_and_word_ends_forward() {
    let mut edit = at("the  dark knight", 0);
    edit.move_caret(Reach::Word, true, false);
    assert_eq!(edit.caret, 3);
    edit.move_caret(Reach::Word, true, false);
    assert_eq!(edit.caret, 9);
    edit.move_caret(Reach::Word, false, false);
    assert_eq!(edit.caret, 5);
    edit.move_caret(Reach::Word, false, false);
    assert_eq!(edit.caret, 0);
}

#[test]
fn deletes_by_word_and_to_the_line_ends() {
    let mut edit = at("the dark knight", 8);
    edit.delete(Reach::Word, false);
    assert_eq!((edit.text.as_str(), edit.caret), ("the  knight", 4));
    edit.delete(Reach::Word, true);
    assert_eq!((edit.text.as_str(), edit.caret), ("the ", 4));
    edit.delete(Reach::Line, false);
    assert_eq!((edit.text.as_str(), edit.caret), ("", 0));
}

#[test]
fn a_collapsing_movement_starts_from_the_selections_edge() {
    let mut edit = at("one two three", 0);
    edit.select(4..7);
    edit.move_caret(Reach::Grapheme, false, false);
    assert_eq!(edit.selection(), 4..4);

    edit.select(4..7);
    edit.move_caret(Reach::Word, true, false);
    assert_eq!(edit.selection(), 13..13);
}

#[test]
fn shift_movements_extend_the_selection_from_the_anchor() {
    let mut edit = at("one two", 3);
    edit.move_caret(Reach::Word, true, true);
    edit.move_caret(Reach::Grapheme, false, true);
    assert_eq!(
        (&edit.text[edit.selection()], edit.anchor, edit.caret),
        (" tw", 3, 6)
    );
    edit.move_caret(Reach::Line, false, true);
    assert_eq!(
        (&edit.text[edit.selection()], edit.anchor, edit.caret),
        ("one", 3, 0)
    );
}

#[test]
fn a_span_selects_both_ranges_whole_with_the_caret_toward_the_second() {
    let mut edit = at("alpha bravo charlie", 0);
    edit.span(6..11, 12..19);
    assert_eq!((edit.anchor, edit.caret), (6, 19));
    edit.span(6..11, 0..5);
    assert_eq!((edit.anchor, edit.caret), (11, 0));
    // A range inside the first keeps the first whole.
    edit.span(6..11, 8..8);
    assert_eq!((edit.anchor, edit.caret), (6, 11));
}

#[test]
fn a_deletion_removes_the_selection_whatever_its_reach() {
    let mut edit = at("one two", 0);
    edit.select(1..5);
    edit.delete(Reach::Line, true);
    assert_eq!((edit.text.as_str(), edit.caret), ("owo", 1));
}

#[test]
fn a_changed_text_moves_the_offsets_to_grapheme_boundaries_within_it() {
    let mut edit = Edit {
        text: "noel!!!".to_string(),
        anchor: 3,
        caret: 7,
    };
    // Offset 3 falls inside "e" with its combining accent, and 7 past the end.
    edit.sync("noe\u{301}l");
    assert_eq!((edit.anchor, edit.caret), (2, 6));
}
