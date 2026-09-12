use std::{borrow::Cow, ops::Range};

use avenger_typst_label::{LabelEngine, LabelError, TextStyle};

use super::shaped_line::{
    Affinity, byte_offset_for_x, next_grapheme, next_word_boundary, prev_grapheme,
    prev_word_boundary, safe_grapheme_offset, shape_line, word_range_at,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Cursor {
    pub index: usize,
    pub affinity: Affinity,
}

impl Cursor {
    pub fn new(index: usize, affinity: Affinity) -> Self {
        Self { index, affinity }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SelectionState {
    pub anchor: Cursor,
    pub head: Cursor,
}

impl SelectionState {
    pub fn collapsed(cursor: Cursor) -> Self {
        Self {
            anchor: cursor,
            head: cursor,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Motion {
    Left,
    Right,
    WordLeft,
    WordRight,
    Start,
    End,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    InsertText(String),
    /// Deletes the selection or, without one, the text the motion moves the cursor over.
    Delete(Motion),
    Motion {
        motion: Motion,
        extend: bool,
    },
    Click {
        x: f32,
    },
    DoubleClick {
        x: f32,
    },
    TripleClick,
    Drag {
        x: f32,
    },
    SelectAll,
    Preedit {
        text: String,
        cursor: Option<(usize, usize)>,
    },
    Commit(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SingleLineEditor {
    buffer: String,
    compose: Option<Range<usize>>,
    selection: SelectionState,
    show_cursor: bool,
    word_drag_anchor: Option<Range<usize>>,
    drag_enabled: bool,
}

impl SingleLineEditor {
    pub fn new(text: impl Into<String>) -> Self {
        let buffer = normalize_single_line(&text.into());
        let cursor = Cursor::new(buffer.len(), Affinity::Upstream);
        Self {
            buffer,
            compose: None,
            selection: SelectionState::collapsed(cursor),
            show_cursor: true,
            word_drag_anchor: None,
            drag_enabled: true,
        }
    }

    pub fn text(&self) -> &str {
        &self.buffer
    }

    pub fn compose_range(&self) -> Option<Range<usize>> {
        self.compose.clone()
    }

    /// The text without the composition in progress.
    pub fn committed_text(&self) -> Cow<'_, str> {
        match &self.compose {
            None => Cow::Borrowed(&self.buffer),
            Some(range) => {
                Cow::Owned([&self.buffer[..range.start], &self.buffer[range.end..]].concat())
            }
        }
    }

    pub fn selection(&self) -> SelectionState {
        self.selection
    }

    pub fn normalized_selection(&self) -> Range<usize> {
        let anchor = safe_grapheme_offset(&self.buffer, self.selection.anchor.index);
        let head = safe_grapheme_offset(&self.buffer, self.selection.head.index);
        anchor.min(head)..anchor.max(head)
    }

    pub fn selected_text(&self) -> &str {
        let range = self.normalized_selection();
        &self.buffer[range]
    }

    pub fn show_cursor(&self) -> bool {
        self.show_cursor
    }

    /// Applies an action, and returns whether the editor changed. Clicks and drags hit-test the
    /// text as the engine shapes it in the style.
    pub fn apply(
        &mut self,
        action: Action,
        engine: &LabelEngine,
        style: &TextStyle,
    ) -> Result<bool, LabelError> {
        let before = self.clone();
        if !matches!(&action, Action::Preedit { .. }) {
            self.show_cursor = true;
        }

        match action {
            Action::InsertText(text) => self.insert_text(&normalize_single_line(&text)),
            Action::Delete(motion) => self.delete_with_motion(motion),
            Action::Motion { motion, extend } => self.move_cursor(motion, extend),
            Action::Click { x } => {
                let hit = byte_offset_for_x(&shape_line(engine, &self.buffer, style)?, x);
                self.selection = SelectionState::collapsed(Cursor::new(hit.0, hit.1));
                self.word_drag_anchor = None;
                self.drag_enabled = true;
            }
            Action::DoubleClick { x } => {
                let (offset, _) = byte_offset_for_x(&shape_line(engine, &self.buffer, style)?, x);
                let range = word_range_at(&self.buffer, offset);
                self.selection = SelectionState {
                    anchor: Cursor::new(range.start, Affinity::Downstream),
                    head: Cursor::new(range.end, Affinity::Upstream),
                };
                self.word_drag_anchor = Some(range);
                self.drag_enabled = true;
            }
            action @ (Action::TripleClick | Action::SelectAll) => {
                self.selection = SelectionState {
                    anchor: Cursor::new(0, Affinity::Downstream),
                    head: Cursor::new(self.buffer.len(), Affinity::Upstream),
                };
                self.word_drag_anchor = None;
                self.drag_enabled = !matches!(action, Action::TripleClick);
            }
            Action::Drag { x } if self.drag_enabled => {
                let (offset, affinity) =
                    byte_offset_for_x(&shape_line(engine, &self.buffer, style)?, x);
                if let Some(anchor_word) = self.word_drag_anchor.clone() {
                    let hit_word = word_range_at(&self.buffer, offset);
                    if hit_word.end <= anchor_word.start {
                        self.selection.anchor = Cursor::new(anchor_word.end, Affinity::Upstream);
                        self.selection.head = Cursor::new(hit_word.start, Affinity::Downstream);
                    } else {
                        self.selection.anchor =
                            Cursor::new(anchor_word.start, Affinity::Downstream);
                        self.selection.head = Cursor::new(hit_word.end, Affinity::Upstream);
                    }
                } else {
                    self.selection.head = Cursor::new(offset, affinity);
                }
            }
            Action::Drag { .. } => {}
            Action::Preedit { text, cursor } => self.apply_preedit(text, cursor),
            Action::Commit(text) => self.apply_commit(text),
        }

        Ok(*self != before)
    }

    pub fn replace_committed_text(&mut self, text: impl Into<String>) -> bool {
        self.restore_committed_state(text, self.selection)
    }

    /// Restore committed text and selection, as used by controlled state and undo.
    pub fn restore_committed_state(
        &mut self,
        text: impl Into<String>,
        selection: SelectionState,
    ) -> bool {
        if self.compose.is_some() {
            return false;
        }
        self.buffer = normalize_single_line(&text.into());
        self.selection = clamp_selection(&self.buffer, selection);
        self.word_drag_anchor = None;
        self.drag_enabled = true;
        self.show_cursor = true;
        true
    }

    pub fn set_selection(&mut self, selection: SelectionState) -> bool {
        let selection = clamp_selection(&self.buffer, selection);
        if self.selection == selection {
            return false;
        }
        self.selection = selection;
        self.word_drag_anchor = None;
        self.drag_enabled = true;
        self.show_cursor = true;
        true
    }

    fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let range = self.normalized_selection();
        self.buffer.replace_range(range.clone(), text);
        let index = range.start + text.len();
        self.compose = None;
        self.selection = SelectionState::collapsed(Cursor::new(index, Affinity::Upstream));
        self.word_drag_anchor = None;
    }

    fn delete_with_motion(&mut self, motion: Motion) {
        if self.normalized_selection().is_empty() {
            self.move_cursor(motion, true);
        }
        self.delete_selection();
    }

    fn delete_selection(&mut self) {
        let range = self.normalized_selection();
        if range.is_empty() {
            return;
        }
        self.buffer.replace_range(range.clone(), "");
        self.compose = None;
        self.selection = SelectionState::collapsed(Cursor::new(range.start, Affinity::Downstream));
        self.word_drag_anchor = None;
    }

    fn move_cursor(&mut self, motion: Motion, extend: bool) {
        let selection = self.normalized_selection();
        if !extend && !selection.is_empty() && !matches!(motion, Motion::Start | Motion::End) {
            let index = match motion {
                Motion::Left | Motion::WordLeft | Motion::Start => selection.start,
                Motion::Right | Motion::WordRight | Motion::End => selection.end,
            };
            self.selection = SelectionState::collapsed(Cursor::new(
                index,
                if index == selection.end {
                    Affinity::Upstream
                } else {
                    Affinity::Downstream
                },
            ));
            self.word_drag_anchor = None;
            return;
        }

        let current = safe_grapheme_offset(&self.buffer, self.selection.head.index);
        let next = match motion {
            Motion::Left => prev_grapheme(&self.buffer, current),
            Motion::Right => next_grapheme(&self.buffer, current),
            Motion::WordLeft => prev_word_boundary(&self.buffer, current),
            Motion::WordRight => next_word_boundary(&self.buffer, current),
            Motion::Start => 0,
            Motion::End => self.buffer.len(),
        };
        let cursor = Cursor::new(
            next,
            if matches!(motion, Motion::Left | Motion::WordLeft | Motion::Start) {
                Affinity::Downstream
            } else {
                Affinity::Upstream
            },
        );
        if extend {
            self.selection.head = cursor;
        } else {
            self.selection = SelectionState::collapsed(cursor);
        }
        self.word_drag_anchor = None;
    }

    fn apply_preedit(&mut self, text: String, cursor: Option<(usize, usize)>) {
        if matches!(text.as_str(), "\n" | "\r") || text.is_empty() && self.compose.is_none() {
            return;
        }
        let cursor = cursor.map(|(a, h)| {
            let offset = |byte| {
                text.char_indices()
                    .take_while(|(i, _)| *i < byte)
                    .map(|(_, c)| c)
                    .filter(|ch| !ch.is_control() && !matches!(ch, '\u{2028}' | '\u{2029}'))
                    .map(char::len_utf8)
                    .sum()
            };
            (offset(a), offset(h))
        });
        let text = normalize_single_line(&text);
        let range = self
            .compose
            .take()
            .unwrap_or_else(|| self.normalized_selection());
        self.buffer.replace_range(range.clone(), &text);
        let compose = range.start..range.start + text.len();
        self.compose = Some(compose.clone());
        self.show_cursor = cursor.is_some();
        let (anchor, head) = cursor.unwrap_or((text.len(), text.len()));
        let anchor = range.start + safe_grapheme_offset(&text, anchor);
        let head = range.start + safe_grapheme_offset(&text, head);
        self.selection = SelectionState {
            anchor: Cursor::new(anchor, Affinity::Downstream),
            head: Cursor::new(head, Affinity::Upstream),
        };
        self.word_drag_anchor = None;
    }

    fn apply_commit(&mut self, text: String) {
        if matches!(text.as_str(), "\n" | "\r") || text.is_empty() && self.compose.is_none() {
            return;
        }
        let text = normalize_single_line(&text);
        let range = self
            .compose
            .take()
            .unwrap_or_else(|| self.normalized_selection());
        self.buffer.replace_range(range.clone(), &text);
        let index = range.start + text.len();
        self.selection = SelectionState::collapsed(Cursor::new(index, Affinity::Upstream));
        self.word_drag_anchor = None;
    }
}

/// Remove control characters and Unicode line/paragraph separators for a single-line editor.
pub fn normalize_single_line(text: &str) -> String {
    text.chars()
        .filter(|ch| !ch.is_control() && !matches!(ch, '\u{2028}' | '\u{2029}'))
        .collect()
}

fn clamp_selection(text: &str, mut selection: SelectionState) -> SelectionState {
    selection.anchor.index = safe_grapheme_offset(text, selection.anchor.index);
    selection.head.index = safe_grapheme_offset(text, selection.head.index);
    selection
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_typst_label::bundled_label_engine;

    fn style() -> TextStyle {
        TextStyle {
            font_family: "Lato".into(),
            font_size: 16.0,
            ..Default::default()
        }
    }

    /// Applies an action that needs no hit test.
    fn apply(editor: &mut SingleLineEditor, action: Action) -> bool {
        editor
            .apply(action, &bundled_label_engine(), &style())
            .unwrap()
    }

    #[test]
    fn line_boundary_motions_reach_the_boundary_with_a_selection() {
        let text = "éabcdé";
        for (anchor, head) in [(2, 4), (4, 2)] {
            for (motion, boundary) in [(Motion::Start, 0), (Motion::End, text.len())] {
                for extend in [false, true] {
                    let mut editor = SingleLineEditor::new(text);
                    editor.set_selection(SelectionState {
                        anchor: Cursor::new(anchor, Affinity::Downstream),
                        head: Cursor::new(head, Affinity::Upstream),
                    });
                    apply(&mut editor, Action::Motion { motion, extend });
                    assert_eq!(editor.selection().head.index, boundary);
                    assert_eq!(
                        editor.selection().anchor.index,
                        if extend { anchor } else { boundary }
                    );
                }
            }
        }
    }

    #[test]
    fn grapheme_backspace_and_word_delete_preserve_boundaries() {
        let mut grapheme = SingleLineEditor::new("e\u{301}");
        assert!(apply(&mut grapheme, Action::Delete(Motion::Left)));
        assert_eq!(grapheme.text(), "");

        let mut words = SingleLineEditor::new("one two");
        apply(&mut words, Action::Delete(Motion::WordLeft));
        assert_eq!(words.text(), "one ");
    }

    #[test]
    fn preedit_is_spliced_but_excluded_from_committed_text() {
        let mut editor = SingleLineEditor::new("ab");
        apply(
            &mut editor,
            Action::Preedit {
                text: "中".to_string(),
                cursor: Some(("中".len(), "中".len())),
            },
        );
        assert_eq!(editor.text(), "ab中");
        assert_eq!(editor.committed_text(), "ab");

        apply(
            &mut editor,
            Action::Preedit {
                text: String::new(),
                cursor: None,
            },
        );
        assert_eq!(editor.text(), "ab");
        assert_eq!(editor.compose_range(), Some(2..2));
        apply(&mut editor, Action::Commit("中".to_string()));
        assert_eq!(editor.text(), "ab中");
        assert_eq!(editor.compose_range(), None);
    }

    #[test]
    fn unicode_separators_are_removed_from_values_and_preedit_offsets() {
        assert_eq!(normalize_single_line("a\u{2028}b\u{2029}c\n\t"), "abc");
        let mut editor = SingleLineEditor::new("a\u{2028}b");
        apply(
            &mut editor,
            Action::Preedit {
                text: "é\u{2028}x".into(),
                cursor: Some((5, 6)),
            },
        );
        assert_eq!(editor.text(), "abéx");
        assert_eq!(editor.selection().anchor.index, 4);
        assert_eq!(editor.selection().head.index, 5);
        apply(&mut editor, Action::Commit("é\u{2029}x".into()));
        assert_eq!(editor.text(), "abéx");
    }

    #[test]
    fn insert_sanitizes_single_line_control_characters() {
        let mut editor = SingleLineEditor::new("");
        apply(
            &mut editor,
            Action::InsertText("a\n\tb\r\u{7}c".to_string()),
        );
        assert_eq!(editor.text(), "abc");
    }

    #[test]
    fn controlled_text_rejects_writes_during_composition() {
        let mut editor = SingleLineEditor::new("old");
        apply(
            &mut editor,
            Action::Preedit {
                text: "x".to_string(),
                cursor: Some((1, 1)),
            },
        );
        assert!(!editor.replace_committed_text("external"));
        assert_eq!(editor.committed_text(), "old");
    }

    #[test]
    fn controlled_and_undo_restores_clamp_anchor_and_head_to_graphemes() {
        let mut editor = SingleLineEditor::new("abcdef");
        assert!(editor.replace_committed_text("x"));
        assert_eq!(editor.selection().anchor.index, 1);
        assert_eq!(editor.selection().head.index, 1);

        let stale = SelectionState {
            anchor: Cursor::new(2, Affinity::Downstream),
            head: Cursor::new(usize::MAX, Affinity::Upstream),
        };
        assert!(editor.restore_committed_state("e\u{301}x", stale));
        assert_eq!(editor.selection().anchor.index, 0);
        assert_eq!(editor.selection().head.index, "e\u{301}x".len());
    }

    #[test]
    fn word_drag_stays_snapped_and_triple_click_disables_extension() {
        let text = "one two three";
        let mut editor = SingleLineEditor::new(text);
        let line = shape_line(&bundled_label_engine(), text, &style()).unwrap();
        let two_x = crate::text_edit::cursor_rect_for_offset(&line, 5, Affinity::Downstream).x;
        apply(&mut editor, Action::DoubleClick { x: two_x });
        assert_eq!(editor.normalized_selection(), 4..7);

        let three_x = crate::text_edit::cursor_rect_for_offset(&line, 11, Affinity::Downstream).x;
        apply(&mut editor, Action::Drag { x: three_x });
        assert_eq!(editor.normalized_selection(), 4..text.len());

        apply(&mut editor, Action::TripleClick);
        assert!(!apply(&mut editor, Action::Drag { x: 0.0 }));
        assert_eq!(editor.normalized_selection(), 0..text.len());
    }
}
