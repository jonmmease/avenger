use std::{fmt, ops::Range};

use crate::{
    error::AvengerTextError,
    measurement::TextMeasurementConfig,
    text_edit::{
        byte_offset_for_x, next_grapheme, next_word_boundary, prev_grapheme, prev_word_boundary,
        word_range_at, Affinity, ShapedLine,
    },
    TextEngine,
};

use super::shaped_line::safe_grapheme_offset;

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
pub enum Granularity {
    #[default]
    Char,
    Word,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SelectionState {
    pub anchor: Cursor,
    pub head: Cursor,
    pub granularity: Granularity,
}

impl SelectionState {
    pub fn collapsed(cursor: Cursor) -> Self {
        Self {
            anchor: cursor,
            head: cursor,
            granularity: Granularity::Char,
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
    Backspace,
    Delete,
    DeleteWordBack,
    DeleteWordForward,
    DeleteToStart,
    DeleteToEnd,
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
    Escape,
    Preedit {
        text: String,
        cursor: Option<(usize, usize)>,
    },
    Commit(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommittedText<'a> {
    before: &'a str,
    after: &'a str,
}

impl<'a> CommittedText<'a> {
    pub fn before(self) -> &'a str {
        self.before
    }

    pub fn after(self) -> &'a str {
        self.after
    }

    pub fn into_string(self) -> String {
        let mut text = String::with_capacity(self.before.len() + self.after.len());
        text.push_str(self.before);
        text.push_str(self.after);
        text
    }
}

impl fmt::Display for CommittedText<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.before)?;
        f.write_str(self.after)
    }
}

#[derive(Clone, Debug)]
pub struct SingleLineEditor {
    buffer: String,
    compose: Option<Range<usize>>,
    selection: SelectionState,
    show_cursor: bool,
    generation: u32,
    shaped_line: Option<ShapedLine>,
    word_drag_anchor: Option<Range<usize>>,
    drag_enabled: bool,
}

impl SingleLineEditor {
    pub fn new(text: impl Into<String>) -> Self {
        let buffer = sanitize_single_line(&text.into());
        let cursor = Cursor::new(buffer.len(), Affinity::Upstream);
        Self {
            buffer,
            compose: None,
            selection: SelectionState::collapsed(cursor),
            show_cursor: true,
            generation: 0,
            shaped_line: None,
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

    pub fn committed_text(&self) -> CommittedText<'_> {
        let range = self
            .compose
            .clone()
            .unwrap_or(self.buffer.len()..self.buffer.len());
        CommittedText {
            before: &self.buffer[..range.start],
            after: &self.buffer[range.end..],
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

    pub fn generation(&self) -> u32 {
        self.generation
    }

    pub fn cached_shaped_line(&self) -> Option<&ShapedLine> {
        self.shaped_line.as_ref()
    }

    pub fn mark_layout_dirty(&mut self) {
        self.shaped_line = None;
    }

    pub fn shape_line(
        &mut self,
        engine: &TextEngine,
        config: &TextMeasurementConfig,
    ) -> Result<&ShapedLine, AvengerTextError> {
        self.relayout(engine, config)?;
        Ok(self
            .shaped_line
            .as_ref()
            .expect("successful relayout installs a shaped line"))
    }

    pub fn apply(
        &mut self,
        action: Action,
        engine: &TextEngine,
        config: &TextMeasurementConfig,
    ) -> Result<bool, AvengerTextError> {
        let before = EditorSnapshot::from(&*self);
        let before_buffer = self.buffer.clone();
        if !matches!(&action, Action::Preedit { .. }) {
            self.show_cursor = true;
        }

        match action {
            Action::InsertText(text) => self.insert_text(&sanitize_single_line(&text)),
            Action::Backspace => self.delete_with_motion(Motion::Left),
            Action::Delete => self.delete_with_motion(Motion::Right),
            Action::DeleteWordBack => self.delete_with_motion(Motion::WordLeft),
            Action::DeleteWordForward => self.delete_with_motion(Motion::WordRight),
            Action::DeleteToStart => self.delete_with_motion(Motion::Start),
            Action::DeleteToEnd => self.delete_with_motion(Motion::End),
            Action::Motion { motion, extend } => self.move_cursor(motion, extend),
            Action::Click { x } => {
                self.relayout(engine, config)?;
                let hit =
                    byte_offset_for_x(self.shaped_line.as_ref().expect("relayout completed"), x);
                let cursor = Cursor::new(hit.0, hit.1);
                self.selection = SelectionState::collapsed(cursor);
                self.word_drag_anchor = None;
                self.drag_enabled = true;
            }
            Action::DoubleClick { x } => {
                self.relayout(engine, config)?;
                let (offset, _) =
                    byte_offset_for_x(self.shaped_line.as_ref().expect("relayout completed"), x);
                let range = word_range_at(&self.buffer, offset);
                self.selection = SelectionState {
                    anchor: Cursor::new(range.start, Affinity::Downstream),
                    head: Cursor::new(range.end, Affinity::Upstream),
                    granularity: Granularity::Word,
                };
                self.word_drag_anchor = Some(range);
                self.drag_enabled = true;
            }
            action @ (Action::TripleClick | Action::SelectAll) => {
                self.selection = SelectionState {
                    anchor: Cursor::new(0, Affinity::Downstream),
                    head: Cursor::new(self.buffer.len(), Affinity::Upstream),
                    granularity: Granularity::Char,
                };
                self.word_drag_anchor = None;
                self.drag_enabled = !matches!(action, Action::TripleClick);
            }
            Action::Drag { x } if self.drag_enabled => {
                self.relayout(engine, config)?;
                let (offset, affinity) =
                    byte_offset_for_x(self.shaped_line.as_ref().expect("relayout completed"), x);
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
                    self.selection.granularity = Granularity::Word;
                } else {
                    self.selection.head = Cursor::new(offset, affinity);
                    self.selection.granularity = Granularity::Char;
                }
            }
            Action::Drag { .. } => {}
            Action::Escape => {
                let head = self.safe_cursor(self.selection.head);
                self.selection = SelectionState::collapsed(head);
                self.word_drag_anchor = None;
                self.drag_enabled = true;
            }
            Action::Preedit { text, cursor } => self.apply_preedit(text, cursor),
            Action::Commit(text) => self.apply_commit(text),
        }

        let changed = before != EditorSnapshot::from(&*self);
        if changed {
            self.generation = self.generation.wrapping_add(1);
        }
        if self.buffer != before_buffer {
            self.relayout(engine, config)?;
        }
        Ok(changed)
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
        let text = sanitize_single_line(&text.into());
        let before_buffer = self.buffer.clone();
        let before_selection = self.selection;
        self.buffer = text;
        self.selection = clamp_selection(&self.buffer, selection);
        self.word_drag_anchor = None;
        self.drag_enabled = true;
        self.show_cursor = true;
        if self.buffer != before_buffer || self.selection != before_selection {
            self.shaped_line = None;
            self.generation = self.generation.wrapping_add(1);
        }
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
        self.generation = self.generation.wrapping_add(1);
        true
    }

    fn relayout(
        &mut self,
        engine: &TextEngine,
        config: &TextMeasurementConfig,
    ) -> Result<(), AvengerTextError> {
        let mut config = config.clone();
        config.text = &self.buffer;
        self.shaped_line = Some(engine.shape_line(&config)?);
        Ok(())
    }

    fn safe_cursor(&self, cursor: Cursor) -> Cursor {
        Cursor::new(
            safe_grapheme_offset(&self.buffer, cursor.index),
            cursor.affinity,
        )
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
        if !extend && !selection.is_empty() {
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
            granularity: Granularity::Char,
        };
        self.word_drag_anchor = None;
    }

    fn apply_commit(&mut self, text: String) {
        if matches!(text.as_str(), "\n" | "\r") || text.is_empty() && self.compose.is_none() {
            return;
        }
        let text = sanitize_single_line(&text);
        let range = self
            .compose
            .take()
            .unwrap_or_else(|| self.normalized_selection());
        self.buffer.replace_range(range.clone(), &text);
        let index = range.start + text.len();
        self.selection = SelectionState::collapsed(Cursor::new(index, Affinity::Upstream));
        self.show_cursor = true;
        self.word_drag_anchor = None;
    }
}

#[derive(PartialEq)]
struct EditorSnapshot {
    buffer: String,
    compose: Option<Range<usize>>,
    selection: SelectionState,
    show_cursor: bool,
    word_drag_anchor: Option<Range<usize>>,
    drag_enabled: bool,
}

impl From<&SingleLineEditor> for EditorSnapshot {
    fn from(editor: &SingleLineEditor) -> Self {
        Self {
            buffer: editor.buffer.clone(),
            compose: editor.compose.clone(),
            selection: editor.selection,
            show_cursor: editor.show_cursor,
            word_drag_anchor: editor.word_drag_anchor.clone(),
            drag_enabled: editor.drag_enabled,
        }
    }
}

fn sanitize_single_line(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_control()).collect()
}

fn clamp_selection(text: &str, mut selection: SelectionState) -> SelectionState {
    selection.anchor.index = safe_grapheme_offset(text, selection.anchor.index);
    selection.head.index = safe_grapheme_offset(text, selection.head.index);
    selection
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        empty_label_params,
        types::{FontStyle, FontWeight, TextSyntaxMode},
    };

    fn config<'a>(text: &'a str) -> TextMeasurementConfig<'a> {
        TextMeasurementConfig {
            text,
            font: "Lato",
            font_size: 16.0,
            font_weight: FontWeight::default(),
            font_style: FontStyle::default(),
            syntax_mode: TextSyntaxMode::Plain,
            params: empty_label_params(),
            number_locale: None,
            number_locale_specs: None,
            datetime_locale: None,
            datetime_timezone: None,
            datetime_locale_specs: None,
        }
    }

    #[test]
    fn grapheme_backspace_and_word_delete_preserve_boundaries() {
        let engine = TextEngine::with_default_config().unwrap();
        let mut grapheme = SingleLineEditor::new("e\u{301}");
        assert!(grapheme
            .apply(Action::Backspace, &engine, &config(""))
            .unwrap());
        assert_eq!(grapheme.text(), "");

        let mut words = SingleLineEditor::new("one two");
        words
            .apply(Action::DeleteWordBack, &engine, &config(""))
            .unwrap();
        assert_eq!(words.text(), "one ");
    }

    #[test]
    fn preedit_is_spliced_but_excluded_from_committed_text() {
        let engine = TextEngine::with_default_config().unwrap();
        let mut editor = SingleLineEditor::new("ab");
        editor
            .apply(
                Action::Preedit {
                    text: "中".to_string(),
                    cursor: Some(("中".len(), "中".len())),
                },
                &engine,
                &config(""),
            )
            .unwrap();
        assert_eq!(editor.text(), "ab中");
        assert_eq!(editor.committed_text().to_string(), "ab");

        editor
            .apply(
                Action::Preedit {
                    text: String::new(),
                    cursor: None,
                },
                &engine,
                &config(""),
            )
            .unwrap();
        assert_eq!(editor.text(), "ab");
        assert_eq!(editor.compose_range(), Some(2..2));
        editor
            .apply(Action::Commit("中".to_string()), &engine, &config(""))
            .unwrap();
        assert_eq!(editor.text(), "ab中");
        assert_eq!(editor.compose_range(), None);
    }

    #[test]
    fn insert_sanitizes_single_line_control_characters() {
        let engine = TextEngine::with_default_config().unwrap();
        let mut editor = SingleLineEditor::new("");
        editor
            .apply(
                Action::InsertText("a\n\tb\r\u{7}c".to_string()),
                &engine,
                &config(""),
            )
            .unwrap();
        assert_eq!(editor.text(), "abc");
    }

    #[test]
    fn controlled_text_rejects_writes_during_composition() {
        let engine = TextEngine::with_default_config().unwrap();
        let mut editor = SingleLineEditor::new("old");
        editor
            .apply(
                Action::Preedit {
                    text: "x".to_string(),
                    cursor: Some((1, 1)),
                },
                &engine,
                &config(""),
            )
            .unwrap();
        assert!(!editor.replace_committed_text("external"));
        assert_eq!(editor.committed_text().to_string(), "old");
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
            granularity: Granularity::Word,
        };
        assert!(editor.restore_committed_state("e\u{301}x", stale));
        assert_eq!(editor.selection().anchor.index, 0);
        assert_eq!(editor.selection().head.index, "e\u{301}x".len());
        assert_eq!(editor.selection().granularity, Granularity::Word);
    }

    #[test]
    fn word_drag_stays_snapped_and_triple_click_disables_extension() {
        let engine = TextEngine::with_default_config().unwrap();
        let text = "one two three";
        let mut editor = SingleLineEditor::new(text);
        let line = engine.shape_line(&config(text)).unwrap();
        let two_x = crate::text_edit::cursor_rect_for_offset(&line, 5, Affinity::Downstream).x;
        editor
            .apply(Action::DoubleClick { x: two_x }, &engine, &config(""))
            .unwrap();
        assert_eq!(editor.normalized_selection(), 4..7);
        assert_eq!(editor.selection().granularity, Granularity::Word);

        let three_x = crate::text_edit::cursor_rect_for_offset(&line, 11, Affinity::Downstream).x;
        editor
            .apply(Action::Drag { x: three_x }, &engine, &config(""))
            .unwrap();
        assert_eq!(editor.normalized_selection(), 4..text.len());

        editor
            .apply(Action::TripleClick, &engine, &config(""))
            .unwrap();
        let generation = editor.generation();
        editor
            .apply(Action::Drag { x: 0.0 }, &engine, &config(""))
            .unwrap();
        assert_eq!(editor.normalized_selection(), 0..text.len());
        assert_eq!(editor.generation(), generation);
    }
}
