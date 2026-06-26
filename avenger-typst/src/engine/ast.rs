use std::ops::Range;

use crate::delimiter::MathDelimiterInfo;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedLine {
    pub(crate) source: String,
    pub(crate) nodes: Vec<LineNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LineNode {
    Plain(PlainTextNode),
    Math(MathSpan),
    TextSpan(TextMarkupSpan),
    Emoji(EmojiAlias),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlainTextNode {
    pub(crate) text: String,
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MathSpan {
    pub(crate) source: String,
    pub(crate) source_range: Range<usize>,
    pub(crate) delimiter: MathDelimiterInfo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TextMarkupSpan {
    pub(crate) kind: TextMarkupKind,
    pub(crate) body: Vec<LineNode>,
    pub(crate) byte_range: Range<usize>,
    pub(crate) body_range: Range<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextMarkupKind {
    Underline,
    Strike,
    Overline,
    Subscript,
    Superscript,
    Highlight,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EmojiAlias {
    pub(crate) name: String,
    pub(crate) emoji: &'static str,
    pub(crate) byte_range: Range<usize>,
}
