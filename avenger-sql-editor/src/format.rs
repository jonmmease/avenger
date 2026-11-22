//! The Format button and its shortcut, ⇧⌥F (Shift+Alt+F off macOS): the query in the style of
//! `avenger-sql/STYLE.md`, with the cursor on the same token.

use avenger_sql_language::{Document, Source};
use eframe::egui;

/// The shortcut that formats the query.
pub fn shortcut() -> egui::KeyboardShortcut {
    egui::KeyboardShortcut::new(egui::Modifiers::SHIFT | egui::Modifiers::ALT, egui::Key::F)
}

/// Why the query can't be formatted: it has an error of the syntax, and the formatter keeps a
/// statement with an error as written.
pub fn blocked(document: &Document) -> Option<&'static str> {
    (document.diagnostics().iter())
        .any(|diagnostic| diagnostic.source == Source::Syntax)
        .then_some("The query has a syntax error")
}

/// The text of `document` in the style, or `None` when it is formatted.
pub fn formatted(document: &Document) -> Option<String> {
    let mut text = document.text().to_string();
    let edits = document.formatting();
    if edits.is_empty() {
        return None;
    }
    for edit in edits.iter().rev() {
        text.replace_range(
            usize::from(edit.range.start())..usize::from(edit.range.end()),
            &edit.replacement,
        );
    }
    Some(text)
}

/// The character of `new`, the text `old` formatted, that the cursor before the character at
/// `index` of `old` moves to, by the characters that are not whitespace or commas, since
/// formatting moves whitespace and adds or removes commas only. A cursor right after such a
/// character stays right after it, and a cursor in whitespace goes right before the next one.
pub fn cursor(old: &str, new: &str, index: usize) -> usize {
    let solid = |c: char| !c.is_whitespace() && c != ',';
    let before = old.chars().take(index).filter(|&c| solid(c)).count();
    let attached = index > 0 && old.chars().nth(index - 1).is_some_and(solid);
    let mut seen = 0;
    for (position, c) in new.chars().enumerate() {
        if solid(c) {
            if !attached && seen == before {
                return position;
            }
            seen += 1;
            if attached && seen == before {
                return position + 1;
            }
        }
    }
    new.chars().count()
}
