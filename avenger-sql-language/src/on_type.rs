//! Indentation as a user types, by the style of avenger-sql's `STYLE.md`: the edits that an editor
//! applies after a character that triggers them, as the Language Server Protocol's on-type
//! formatting sends them. Return gives the new line the depth of what most likely comes next; a
//! line's first token, typed as `)`, `]`, or the `>` of `|>`, moves the line to the depth of its
//! bracket or its query; and the space after a clause's keyword at the start of a line, as
//! `GROUP BY` or `ON`, moves the line to the depth of the operator's keyword, and after an `AND` or
//! `OR`, to the depth of its chain. The depths are those of
//! [`avenger_sql_syntax::indent::depth`].
//!
//! An edit inserts or deletes spaces at the start of the line, before the cursor, so that the
//! cursor moves with the line's text, and never inserts at the cursor, since an editor applies an
//! insertion at the cursor after it. An editor's own indentation rules place the cursor on Return,
//! and these edits correct what the rules can't express, such as the closer of a bracket that a
//! pipe operator's line opens.

use avenger_sql_syntax::{indent, TextEdit, TextRange};

use crate::{Document, Offset};

/// The characters after which [`Document::on_type`] can edit, the first of them first.
pub const ON_TYPE_TRIGGERS: [char; 5] = ['\n', ')', ']', '>', ' '];

/// The range of the bytes `start..end`.
fn range(start: usize, end: usize) -> TextRange {
    TextRange::new(Offset::from(start as u32), Offset::from(end as u32))
}

/// The length of the spaces and tabs at the start of `line`.
fn leading(line: &str) -> usize {
    line.len() - line.trim_start_matches([' ', '\t']).len()
}

/// Whether `typed`, the text of a line before a typed space, is the keywords of a clause of a pipe
/// operator, as `GROUP`, `GROUP BY`, or `ON`, or an `AND` or `OR` that continues a chain.
fn clause_keywords(typed: &str) -> bool {
    let words: Vec<&str> = typed.split(' ').filter(|word| !word.is_empty()).collect();
    match words.as_slice() {
        [word] if word.eq_ignore_ascii_case("AND") || word.eq_ignore_ascii_case("OR") => true,
        [first, rest @ ..] => {
            avenger_sql_syntax::indent::starts_operator_clause(first)
                && typed.chars().all(|c| c.is_ascii_alphabetic() || c == ' ')
                && (rest.iter()).all(|word| avenger_sql_syntax::SyntaxKind::keyword(word).is_some())
        }
        [] => false,
    }
}

/// The edit that gives the line that starts at `line_start` of `text` its depth, as spaces
/// inserted or deleted at its start, or tabs replaced. None when the line has its depth, inside a
/// string or a comment, and for an insertion at `cursor`, which an editor applies after the
/// cursor, so that the cursor would stay behind the indentation.
fn indentation(text: &str, line_start: usize, cursor: usize) -> Option<TextEdit> {
    let depth = indent::depth(text, line_start)?;
    let line_end = text[line_start..]
        .find('\n')
        .map_or(text.len(), |end| line_start + end);
    let spaces = leading(&text[line_start..line_end]);
    let current = &text[line_start..line_start + spaces];
    if current.contains('\t') {
        // Tabs become the spaces of the style.
        return Some(TextEdit::new(
            range(line_start, line_start + spaces),
            " ".repeat(depth),
        ));
    }
    match depth.cmp(&spaces) {
        std::cmp::Ordering::Greater if line_start != cursor => Some(TextEdit::new(
            range(line_start, line_start),
            " ".repeat(depth - spaces),
        )),
        std::cmp::Ordering::Less => Some(TextEdit::new(
            range(line_start, line_start + spaces - depth),
            "",
        )),
        _ => None,
    }
}

impl Document {
    /// The edits after the character `ch`, which the user typed just before `offset`: on Return,
    /// the new line's indentation, the emptying of the line that Return left when it holds only
    /// spaces, and the indentation of the line after it when it starts with a closer, as Return
    /// between an editor's autoclosed brackets leaves it; on `)`, `]`, or the `>` of `|>` that is
    /// the first token of its line, and on the space after the keywords of an operator's clause or
    /// after an `AND` or `OR` that start the line, the line's indentation. None when the line has
    /// its depth already, when the character is not its line's first token, or inside a string or
    /// a comment. The indentation is spaces, as the formatter writes it, inserted or deleted at the
    /// start of the line, and never inserted at the cursor, where it would stay after the cursor.
    pub fn on_type(&self, offset: Offset, ch: char) -> Vec<TextEdit> {
        let text = self.text();
        let offset = usize::from(self.clamp(offset));
        let line_start = text[..offset].rfind('\n').map_or(0, |end| end + 1);
        let typed = text[line_start..offset].trim_start_matches([' ', '\t']);
        let first = match ch {
            '\n' => true,
            ')' | ']' => typed.len() == 1 && typed.starts_with(ch),
            '>' => typed == "|>",
            ' ' => clause_keywords(typed),
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
        edits.extend(indentation(text, line_start, offset));
        // The closer on the line after, which Return between autoclosed brackets moves down and
        // which the user never types.
        if ch == '\n' {
            if let Some(end) = text[offset..].find('\n') {
                let next = offset + end + 1;
                let rest = text[next..].trim_start_matches([' ', '\t']);
                if rest.starts_with([')', ']']) {
                    edits.extend(indentation(text, next, offset));
                }
            }
        }
        edits
    }
}
