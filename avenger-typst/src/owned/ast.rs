use std::ops::Range;

use crate::delimiter::MathDelimiterInfo;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnedLine {
    pub(crate) source: String,
    pub(crate) nodes: Vec<OwnedLineNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OwnedLineNode {
    Plain(OwnedPlainText),
    Math(OwnedMathSpan),
    TextSpan(OwnedTextSpan),
    Emoji(OwnedEmojiAlias),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnedPlainText {
    pub(crate) text: String,
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnedMathSpan {
    pub(crate) source: String,
    pub(crate) source_range: Range<usize>,
    pub(crate) delimiter: MathDelimiterInfo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnedTextSpan {
    pub(crate) kind: OwnedTextSpanKind,
    pub(crate) body: Vec<OwnedLineNode>,
    pub(crate) byte_range: Range<usize>,
    pub(crate) body_range: Range<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnedTextSpanKind {
    Underline,
    Strike,
    Overline,
    Subscript,
    Superscript,
    Highlight,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnedEmojiAlias {
    pub(crate) name: String,
    pub(crate) emoji: &'static str,
    pub(crate) byte_range: Range<usize>,
}
