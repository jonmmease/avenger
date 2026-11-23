//! Indentation as a user types, by the style of avenger-sql's `STYLE.md`: the edits that an editor
//! applies after a character that triggers them, as the Language Server Protocol's on-type
//! formatting sends them. Return gives the new line the depth of what most likely comes next, and
//! a line's first token, typed as `)`, `]`, or the `>` of `|>`, moves the line to the depth of its
//! bracket or its query. The depths are those of [`avenger_sql_syntax::indent::depth`].

use avenger_sql_syntax::{indent, TextEdit, TextRange};

use crate::{Document, Offset};

/// The characters after which [`Document::on_type`] can edit, the first of them first.
pub const ON_TYPE_TRIGGERS: [char; 4] = ['\n', ')', ']', '>'];

/// The range of the bytes `start..end`.
fn range(start: usize, end: usize) -> TextRange {
    TextRange::new(Offset::from(start as u32), Offset::from(end as u32))
}

/// The length of the spaces and tabs at the start of `line`.
fn leading(line: &str) -> usize {
    line.len() - line.trim_start_matches([' ', '\t']).len()
}

impl Document {
    /// The edits after the character `ch`, which the user typed just before `offset`: on Return,
    /// the new line's indentation, and the emptying of the line that Return left when it holds
    /// only spaces; on `)`, `]`, or the `>` of `|>` that is the first token of its line, the line's
    /// indentation. None when the line has its depth already, when the character is not its
    /// line's first token, or inside a string or a comment. The indentation is spaces, as the
    /// formatter writes it.
    pub fn on_type(&self, offset: Offset, ch: char) -> Vec<TextEdit> {
        let text = self.text();
        let offset = usize::from(self.clamp(offset));
        let line_start = text[..offset].rfind('\n').map_or(0, |end| end + 1);
        let typed = text[line_start..offset].trim_start_matches([' ', '\t']);
        let first = match ch {
            '\n' => true,
            ')' | ']' => typed.len() == 1 && typed.starts_with(ch),
            '>' => typed == "|>",
            _ => false,
        };
        if !first {
            return Vec::new();
        }
        let mut edits = Vec::new();
        // The line that Return left, when it holds only spaces outside a string or a comment.
        if ch == '\n' && line_start > 0 {
            let end = line_start - 1;
            let start = text[..end].rfind('\n').map_or(0, |end| end + 1);
            let left = text[start..end].trim_end_matches('\r');
            if !left.is_empty() && left.trim().is_empty() && indent::depth(text, start).is_some() {
                edits.push(TextEdit::new(range(start, start + left.len()), ""));
            }
        }
        if let Some(depth) = indent::depth(text, line_start) {
            let line_end = text[line_start..]
                .find('\n')
                .map_or(text.len(), |end| line_start + end);
            let spaces = leading(&text[line_start..line_end]);
            let indentation = " ".repeat(depth);
            if text[line_start..line_start + spaces] != indentation {
                edits.push(TextEdit::new(
                    range(line_start, line_start + spaces),
                    indentation,
                ));
            }
        }
        edits
    }
}
