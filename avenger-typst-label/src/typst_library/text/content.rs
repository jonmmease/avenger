use std::ops::Range;

use crate::typst_eval::delimiter::DelimiterInfo;
use crate::typst_library::Color;
use crate::typst_library::text::smartquote::SmartQuote;
use crate::typst_svg::{DashPattern, LineCap, LineJoin};

/// Label-scoped content tree.
///
/// This mirrors the role of upstream Typst's `foundations::Content` for the
/// single-line label subset, without pulling in the full dynamic element
/// system.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LabelContent {
    pub(crate) source: String,
    pub(crate) nodes: Vec<LineNode>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum LineNode {
    Plain(PlainTextNode),
    Math(MathSpan),
    TextSpan(TextMarkupSpan),
    SmartQuote(SmartQuoteNode),
    Emoji(EmojiAlias),
    Symbol(SymbolAlias),
    Param(LabelParamRef),
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
    pub(crate) delimiter: DelimiterInfo,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TextMarkupSpan {
    pub(crate) kind: TextMarkupKind,
    pub(crate) options: TextMarkupOptions,
    pub(crate) body: Vec<LineNode>,
    pub(crate) byte_range: Range<usize>,
    pub(crate) body_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SmartQuoteNode {
    pub(crate) quote: SmartQuote,
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextMarkupKind {
    Underline,
    Strike,
    Overline,
    Subscript,
    Superscript,
    Lower,
    Upper,
    Smallcaps,
    Emph,
    Strong,
    Raw,
}

impl TextMarkupKind {
    pub(crate) fn is_line_decoration(self) -> bool {
        matches!(self, Self::Underline | Self::Strike | Self::Overline)
    }

    pub(crate) fn is_script(self) -> bool {
        matches!(self, Self::Subscript | Self::Superscript)
    }

    pub(crate) fn is_case_transform(self) -> bool {
        matches!(self, Self::Lower | Self::Upper)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct TextMarkupOptions {
    pub(crate) decoration: TextDecorationOptions,
    pub(crate) smallcaps: TextSmallcapsOptions,
    pub(crate) script: TextScriptOptions,
    pub(crate) strong: TextStrongOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct TextSmallcapsOptions {
    pub(crate) all: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TextStrongOptions {
    pub(crate) delta: i64,
}

impl Default for TextStrongOptions {
    fn default() -> Self {
        Self { delta: 300 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TextScriptOptions {
    pub(crate) typographic: bool,
    pub(crate) baseline: Option<DecorationLength>,
    pub(crate) size: Option<DecorationLength>,
}

impl Default for TextScriptOptions {
    fn default() -> Self {
        Self {
            typographic: true,
            baseline: None,
            size: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct TextDecorationOptions {
    pub(crate) stroke: DecorationStroke,
    pub(crate) offset: Option<DecorationLength>,
    pub(crate) extent: DecorationLength,
    pub(crate) background: bool,
    pub(crate) evade: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct DecorationStroke {
    pub(crate) paint: Option<Color>,
    pub(crate) thickness: Option<DecorationLength>,
    pub(crate) line_cap: Option<LineCap>,
    pub(crate) line_join: Option<LineJoin>,
    pub(crate) dash: Option<DecorationDash>,
    pub(crate) miter_limit: Option<f32>,
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

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct DecorationDash {
    pub(crate) array: Vec<DecorationDashLength>,
    pub(crate) phase: DecorationLength,
}

impl DecorationDash {
    pub(crate) fn resolve(&self, stroke_width: f32, font_size: f32) -> Option<DashPattern> {
        if self.array.is_empty() {
            return None;
        }
        Some(DashPattern {
            array: self
                .array
                .iter()
                .map(|length| length.resolve(stroke_width, font_size).max(0.0))
                .collect(),
            phase: self.phase.resolve(font_size),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum DecorationDashLength {
    Length(DecorationLength),
    LineWidth,
}

impl DecorationDashLength {
    fn resolve(self, stroke_width: f32, font_size: f32) -> f32 {
        match self {
            Self::Length(length) => length.resolve(font_size),
            Self::LineWidth => stroke_width,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EmojiAlias {
    pub(crate) name: String,
    pub(crate) emoji: &'static str,
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SymbolAlias {
    pub(crate) name: String,
    pub(crate) text: &'static str,
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LabelParamRef {
    pub(crate) name: String,
    pub(crate) byte_range: Range<usize>,
}
