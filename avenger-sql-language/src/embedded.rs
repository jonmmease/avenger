//! The strings in a language of their own: the string literals of Typst markup, as the analysis
//! marks them, and the values of typed literals, such as `'2024-01-01'` in `DATE '2024-01-01'`. An
//! editor's Tree-sitter grammar highlights them with grammars of their languages, by injection,
//! so a language server leaves them to it, while [`Document::highlights`] keeps their classes for
//! the tools that highlight without injections.

use avenger_sql_syntax::{SyntaxKind, TextRange};

use crate::Document;

/// The language of a string.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Embedded {
    /// Typst markup, in a string whose parameter takes markup.
    Typst,
    /// The value of a typed literal: a date, a time, a datetime, a timestamp, a range, a number of
    /// a decimal type, or JSON.
    TypedValue,
}

/// A string literal in a language of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmbeddedString {
    /// The range of the literal, its quotes included. A literal of several adjacent strings, as
    /// `'a' 'b'`, is one range of Typst markup and a range for each string of a typed literal.
    pub range: TextRange,
    pub language: Embedded,
}

impl Document {
    /// The strings in a language of their own, in the order of the text.
    pub fn embedded_strings(&self) -> Vec<EmbeddedString> {
        let markup = (self.model().markup.iter()).map(|&range| EmbeddedString {
            range,
            language: Embedded::Typst,
        });
        let typed = (self.root().descendants())
            .filter(|node| node.kind() == SyntaxKind::TYPED_LITERAL)
            .flat_map(|node| node.children_with_tokens())
            .filter_map(|element| element.into_token())
            .filter(|token| token.kind() == SyntaxKind::STRING)
            .map(|token| EmbeddedString {
                range: token.text_range(),
                language: Embedded::TypedValue,
            });
        let mut strings: Vec<EmbeddedString> = markup.chain(typed).collect();
        strings.sort_by_key(|string| string.range.start());
        strings
    }
}
