//! Smart indentation: Enter starts a line at its depth by the style of `avenger-sql/STYLE.md`,
//! typing the first token of a line, such as `)`, `|>`, or `GROUP `, moves the line to its depth,
//! and Tab re-indents a line or the lines of a selection, moves a cursor in the indentation to the
//! first word, and otherwise opens the completion popup.
//!
//! Positions are character indices of the text, as the editor's cursor has them.

use std::ops::Range;

use avenger_sql_syntax::indent::depth;

/// What Tab does.
#[derive(Debug, PartialEq, Eq)]
pub enum Tab {
    /// Re-indents lines: the new text and the selection over the same characters.
    Edit(String, Range<usize>),
    /// Moves the cursor to the first word of its line.
    Move(usize),
    /// Opens the completion popup at the cursor.
    Complete,
    /// Leaves the text and the selection, as for a selection whose lines are at their depths.
    Stay,
}

fn byte(text: &str, index: usize) -> usize {
    text.char_indices()
        .nth(index)
        .map_or(text.len(), |(byte, _)| byte)
}

fn chars(text: &str, byte: usize) -> usize {
    text[..byte].chars().count()
}

/// The byte offset of the start of the line that holds the byte `offset`.
fn line_start(text: &str, offset: usize) -> usize {
    text[..offset].rfind('\n').map_or(0, |end| end + 1)
}

/// The length in bytes of the indentation of the line that starts at `start`.
fn indentation(text: &str, start: usize) -> usize {
    text[start..]
        .bytes()
        .take_while(|b| *b == b' ' || *b == b'\t')
        .count()
}

/// The text after Enter replaces `selection` with a line break, and the cursor: the new line
/// starts at its depth, without the spaces around the break, and between an opener and its closer,
/// as in `mark_bar(|)`, the cursor goes on a line of its own between them, at the depth of the
/// items. Inside a string or a comment, the line break goes in alone.
pub fn enter(text: &str, selection: Range<usize>) -> (String, usize) {
    let (start, end) = (byte(text, selection.start), byte(text, selection.end));
    let plain = || {
        let new = format!("{}\n{}", &text[..start], &text[end..]);
        (new, selection.start + 1)
    };
    let before = text[..start].trim_end_matches([' ', '\t']);
    let rest = &text[end..];
    let after = rest.trim_start_matches([' ', '\t']);
    let joined = format!("{before}\n{after}");
    let Some(level) = depth(&joined, before.len() + 1) else {
        return plain();
    };
    let pad = |n: usize| " ".repeat(n);
    let closes = |open: char, close: char| before.ends_with(open) && after.starts_with(close);
    if closes('(', ')') || closes('[', ']') {
        let open = format!("{before}\n");
        let inner = depth(&open, open.len()).unwrap_or(level + 2);
        let new = format!("{before}\n{}\n{}{after}", pad(inner), pad(level));
        return (new, chars(text, before.len()) + 1 + inner);
    }
    let new = format!("{before}\n{}{after}", pad(level));
    (new, chars(text, before.len()) + 1 + level)
}

/// The text with the lines that start at `starts`, byte offsets in order, at their depths, and
/// the selection `selection` over the same characters, where a position in a line's indentation
/// goes to its first word. Each line takes its depth after the lines above it moved, so that the
/// items under a line that moves follow it. A blank line stays blank unless `blank` is set.
/// `None` when no line moves.
fn reindent(
    text: &str,
    starts: &[usize],
    selection: Range<usize>,
    blank: bool,
) -> Option<(String, Range<usize>)> {
    let mut text = text.to_string();
    let (mut low, mut high) = (selection.start, selection.end);
    let mut delta: isize = 0;
    let mut moved = false;
    for &start in starts {
        let start = (start as isize + delta) as usize;
        let Some(level) = depth(&text, start) else {
            continue;
        };
        let current = indentation(&text, start);
        let empty = text[start + current..].starts_with('\n') || start + current == text.len();
        if (empty && !blank) || text[start..start + current] == " ".repeat(level) {
            continue;
        }
        moved = true;
        text.replace_range(start..start + current, &" ".repeat(level));
        delta += level as isize - current as isize;
        // A position after the indentation moves with it, and one in it goes to its end.
        let line = chars(&text, start);
        let shift = |index: usize| match index {
            index if index > line + current => index + level - current,
            index if index >= line => line + level,
            index => index,
        };
        (low, high) = (shift(low), shift(high));
    }
    moved.then_some((text, low..high))
}

/// What Tab does with `selection`, whose `end` is the cursor.
pub fn tab(text: &str, selection: Range<usize>) -> Tab {
    let (low, high) = (
        selection.start.min(selection.end),
        selection.start.max(selection.end),
    );
    let (first, last) = (
        line_start(text, byte(text, low)),
        line_start(text, byte(text, high)),
    );
    if first != last {
        let starts: Vec<usize> = std::iter::once(first)
            .chain((text[first..last].match_indices('\n')).map(|(offset, _)| first + offset + 1))
            .filter(|&start| start <= last)
            .collect();
        return match reindent(text, &starts, selection, false) {
            Some((new, selection)) => Tab::Edit(new, selection),
            None => Tab::Stay,
        };
    }
    let cursor = selection.end;
    if let Some((new, selection)) = reindent(text, &[first], selection, true) {
        return Tab::Edit(new, selection);
    }
    let word = chars(text, first + indentation(text, first));
    match cursor < word {
        true => Tab::Move(word),
        false => Tab::Complete,
    }
}

/// The text after typing `typed` before the cursor `cursor`, when it makes the line's first token
/// one that moves the line: a closer, `|>`, or a keyword or an operator followed by a space, such
/// as `GROUP ` or `AND `. `None` when the line stays.
pub fn typed(text: &str, cursor: usize, typed: char) -> Option<(String, usize)> {
    let at = byte(text, cursor);
    let start = line_start(text, at);
    let typed_so_far = text[start..at].trim_start_matches([' ', '\t']);
    let moves = match typed {
        ')' | ']' => typed_so_far.len() == 1,
        '>' => typed_so_far == "|>",
        ' ' => matches!(
            typed_so_far.trim_end().to_ascii_uppercase().as_str(),
            "GROUP"
                | "WINDOW"
                | "AND"
                | "OR"
                | "END"
                | "||"
                | "+"
                | "-"
                | "*"
                | "/"
                | "="
                | "<"
                | ">"
                | "<="
                | ">="
                | "!="
                | "<>"
        ),
        _ => false,
    };
    if !moves {
        return None;
    }
    reindent(text, &[start], cursor..cursor, false).map(|(new, selection)| (new, selection.end))
}
