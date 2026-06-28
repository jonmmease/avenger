use std::ops::Range;

use crate::delimiter::MathDelimiterInfo;
use crate::style::Color;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ParsedLine {
    pub(crate) source: String,
    pub(crate) nodes: Vec<LineNode>,
}

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TextMarkupSpan {
    pub(crate) kind: TextMarkupKind,
    pub(crate) options: TextMarkupOptions,
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

impl TextMarkupKind {
    pub(crate) fn is_line_decoration(self) -> bool {
        matches!(self, Self::Underline | Self::Strike | Self::Overline)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct TextMarkupOptions {
    pub(crate) decoration: TextDecorationOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct TextDecorationOptions {
    pub(crate) stroke: DecorationStroke,
    pub(crate) offset: Option<DecorationLength>,
    pub(crate) extent: DecorationLength,
    pub(crate) background: bool,
    pub(crate) evade: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct DecorationStroke {
    pub(crate) paint: Option<Color>,
    pub(crate) thickness: Option<DecorationLength>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum DecorationLength {
    Pt(f32),
    Em(f32),
}

impl DecorationLength {
    pub(crate) fn resolve(self, font_size: f32) -> f32 {
        match self {
            Self::Pt(value) => value,
            Self::Em(value) => value * font_size,
        }
    }
}

impl Default for DecorationLength {
    fn default() -> Self {
        Self::Pt(0.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EmojiAlias {
    pub(crate) name: String,
    pub(crate) emoji: &'static str,
    pub(crate) byte_range: Range<usize>,
}
