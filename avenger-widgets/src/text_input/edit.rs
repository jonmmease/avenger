//! A line of text with a selection, and the edits and caret movements that keys make.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

/// A line of text with a selection from an anchor to a caret. Both are byte offsets on grapheme
/// boundaries, and the selection is empty when they are equal.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Edit {
    pub(crate) text: String,
    pub(crate) anchor: usize,
    pub(crate) caret: usize,
}

/// How far a movement or a deletion reaches from the caret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reach {
    Grapheme,
    /// The start of a word backward, or the end of a word forward.
    Word,
    /// The start or the end of the text.
    Line,
}

impl Edit {
    /// The selected range, from its lower to its higher offset.
    pub(crate) fn selection(&self) -> Range<usize> {
        self.anchor.min(self.caret)..self.anchor.max(self.caret)
    }

    /// Take `text`, which app state can change between edits. If it differs, move the anchor
    /// and the caret to the nearest grapheme boundaries at or before them.
    pub(crate) fn sync(&mut self, text: &str) {
        if self.text != text {
            self.text = text.to_string();
            self.anchor = boundary(text, self.anchor);
            self.caret = boundary(text, self.caret);
        }
    }

    /// Place the caret at `offset`, and extend the selection to it or collapse the selection.
    pub(crate) fn place(&mut self, offset: usize, extend: bool) {
        self.caret = boundary(&self.text, offset);
        if !extend {
            self.anchor = self.caret;
        }
    }

    /// Select `range`, with the caret at its end.
    pub(crate) fn select(&mut self, range: Range<usize>) {
        self.anchor = boundary(&self.text, range.start);
        self.caret = boundary(&self.text, range.end);
    }

    /// Select from the range `from` through the range `to`, both whole, with the caret at the end
    /// toward `to`, as a drag from a selected word selects whole words.
    pub(crate) fn span(&mut self, from: Range<usize>, to: Range<usize>) {
        let (anchor, caret) = match to.start < from.start {
            true => (from.end, to.start),
            false => (from.start, to.end.max(from.end)),
        };
        self.anchor = boundary(&self.text, anchor);
        self.caret = boundary(&self.text, caret);
    }

    pub(crate) fn select_all(&mut self) {
        self.select(0..self.text.len());
    }

    /// Move the caret by `reach`, forward or backward, and extend the selection or collapse it.
    /// A collapsing movement starts from the selection's edge in its direction, and a grapheme
    /// movement stops there.
    pub(crate) fn move_caret(&mut self, reach: Reach, forward: bool, extend: bool) {
        let selection = self.selection();
        let from = match (extend, forward) {
            (true, _) => self.caret,
            (false, true) => selection.end,
            (false, false) => selection.start,
        };
        let to = match reach {
            Reach::Grapheme if !extend && !selection.is_empty() => from,
            reach => self.reach(from, reach, forward),
        };
        self.place(to, extend);
    }

    /// Replace the selection with `text`, and place the caret after it, or after the grapheme
    /// that it joins, as an emoji with a joiner joins the emoji after it.
    pub(crate) fn insert(&mut self, text: &str) {
        let selection = self.selection();
        self.text.replace_range(selection.clone(), text);
        self.caret = next_boundary(&self.text, selection.start + text.len());
        self.anchor = self.caret;
    }

    /// Delete the selection, or without a selection, the text from the caret to `reach`,
    /// forward or backward.
    pub(crate) fn delete(&mut self, reach: Reach, forward: bool) {
        if self.anchor == self.caret {
            self.caret = self.reach(self.caret, reach, forward);
        }
        self.insert("");
    }

    /// The offset `reach` away from `from`, forward or backward.
    fn reach(&self, from: usize, reach: Reach, forward: bool) -> usize {
        let text = self.text.as_str();
        match (reach, forward) {
            (Reach::Grapheme, true) => {
                (text[from..].graphemes(true).next()).map_or(from, |grapheme| from + grapheme.len())
            }
            (Reach::Grapheme, false) => {
                (text[..from].grapheme_indices(true).next_back()).map_or(0, |(start, _)| start)
            }
            (Reach::Word, true) => (text.unicode_word_indices())
                .map(|(start, word)| start + word.len())
                .find(|&end| end > from)
                .unwrap_or(text.len()),
            (Reach::Word, false) => (text.unicode_word_indices())
                .map(|(start, _)| start)
                .take_while(|&start| start < from)
                .last()
                .unwrap_or(0),
            (Reach::Line, true) => text.len(),
            (Reach::Line, false) => 0,
        }
    }
}

/// The last grapheme boundary of `text` at or before `offset`.
pub(crate) fn boundary(text: &str, offset: usize) -> usize {
    if offset >= text.len() {
        return text.len();
    }
    (text.grapheme_indices(true))
        .map(|(start, _)| start)
        .take_while(|&start| start <= offset)
        .last()
        .unwrap_or(0)
}

/// The first grapheme boundary of `text` at or after `offset`.
fn next_boundary(text: &str, offset: usize) -> usize {
    (text.grapheme_indices(true))
        .map(|(start, _)| start)
        .find(|&start| start >= offset)
        .unwrap_or(text.len())
}

#[cfg(test)]
mod tests;
