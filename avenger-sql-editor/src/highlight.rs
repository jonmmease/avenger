//! The highlighting of a query: the classes of avenger-sql-language's highlights, which the
//! analysis refines for the editor's text, or the syntax's alone for a text without an analysis,
//! such as the Lifted SQL tab, each with a class that the editor colors.

use std::ops::Range;

use avenger_sql_language::{Highlight, HighlightRange, TextRange};
use avenger_sql_syntax::{FunctionRole, NameRole};

/// The class of a token, which sets its color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// Punctuation, and a name without a class of its own, such as an alias or a field.
    Plain,
    Keyword,
    /// A function of GoogleSQL or a transform.
    Function,
    /// A mark, a composition, or a constructor of avenger-chart.
    Chart,
    /// The name of a parameter, as before `=>`.
    Argument,
    Column,
    Table,
    Type,
    /// A variable, a query parameter, or a range variable.
    Variable,
    Number,
    String,
    Comment,
    /// An operator, such as `|>`, `=>`, or `+`.
    Operator,
}

/// A token of the text: its range in characters and its class.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub range: Range<usize>,
    pub class: Class,
}

/// The class of `highlight`.
fn class(highlight: Highlight) -> Class {
    match highlight {
        Highlight::Keyword => Class::Keyword,
        Highlight::Comment => Class::Comment,
        Highlight::String | Highlight::EnumValue => Class::String,
        Highlight::Number => Class::Number,
        Highlight::QueryParameter => Class::Variable,
        Highlight::Operator => Class::Operator,
        Highlight::Punctuation => Class::Plain,
        Highlight::Name(role) => match role {
            NameRole::Function(
                FunctionRole::Mark | FunctionRole::Composition | FunctionRole::Constructor,
            ) => Class::Chart,
            NameRole::Function(_) => Class::Function,
            NameRole::Parameter => Class::Argument,
            NameRole::Column => Class::Column,
            NameRole::Table | NameRole::Query => Class::Table,
            NameRole::Type => Class::Type,
            NameRole::Variable | NameRole::RangeVariable => Class::Variable,
            _ => Class::Plain,
        },
    }
}

/// The highlights of the syntax of `text` alone.
pub fn syntax(text: &str) -> Vec<HighlightRange> {
    avenger_sql_syntax::highlight(&avenger_sql_syntax::parse(text).syntax())
}

/// The spans of `highlights` of `text`, in characters, in order.
pub fn spans(text: &str, highlights: &[HighlightRange]) -> Vec<Span> {
    let chars = Chars::new(text);
    (highlights.iter())
        .map(|highlight| Span {
            range: chars.range(highlight.range),
            class: class(highlight.highlight),
        })
        .collect()
}

/// The character index of each byte offset of a text, which converts the byte ranges of the
/// language crate to the character ranges of egui's cursor and back.
pub struct Chars<'t> {
    text: &'t str,
}

impl<'t> Chars<'t> {
    pub fn new(text: &'t str) -> Self {
        Self { text }
    }

    /// The character index of the byte offset `byte`, at most the length of the text.
    pub fn index(&self, byte: usize) -> usize {
        let byte = byte.min(self.text.len());
        self.text[..byte].chars().count()
    }

    /// The characters of the byte range `range`.
    pub fn range(&self, range: TextRange) -> Range<usize> {
        self.index(range.start().into())..self.index(range.end().into())
    }

    /// The byte offset of the character index `index`, at most the length of the text.
    pub fn offset(&self, index: usize) -> usize {
        (self.text.char_indices().nth(index)).map_or(self.text.len(), |(byte, _)| byte)
    }
}
