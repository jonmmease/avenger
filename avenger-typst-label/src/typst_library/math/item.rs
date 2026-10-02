use std::ops::Range;

use crate::typst_library::text::content::DecorationStroke;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MathAst {
    pub(crate) source: String,
    pub(crate) nodes: Vec<MathNode>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MathNode {
    Space(MathSpace),
    Spacing(MathSpacing),
    Text(MathText),
    Identifier(MathIdentifier),
    Operator(MathOperator),
    Shorthand(MathShorthand),
    StringLiteral(MathStringLiteral),
    Group(MathGroup),
    Attach(MathAttach),
    Fraction(MathFraction),
    Cancel(MathCancel),
    Accent(MathAccent),
    Call(MathCall),
}

impl MathNode {
    pub(crate) fn byte_range(&self) -> Range<usize> {
        match self {
            Self::Space(node) => node.byte_range.clone(),
            Self::Spacing(node) => node.byte_range.clone(),
            Self::Text(node) => node.byte_range.clone(),
            Self::Identifier(node) => node.byte_range.clone(),
            Self::Operator(node) => node.byte_range.clone(),
            Self::Shorthand(node) => node.byte_range.clone(),
            Self::StringLiteral(node) => node.byte_range.clone(),
            Self::Group(node) => node.byte_range.clone(),
            Self::Attach(node) => node.byte_range.clone(),
            Self::Fraction(node) => node.byte_range.clone(),
            Self::Cancel(node) => node.byte_range.clone(),
            Self::Accent(node) => node.byte_range.clone(),
            Self::Call(node) => node.byte_range.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MathSpace {
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MathSpacing {
    pub(crate) kind: MathSpacingKind,
    pub(crate) weak: bool,
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum MathSpacingKind {
    Thin,
    Medium,
    Thick,
    Quad,
    Wide,
}

impl MathSpacingKind {
    pub(crate) fn em_width(self) -> f32 {
        match self {
            Self::Thin => 1.0 / 6.0,
            Self::Medium => 2.0 / 9.0,
            Self::Thick => 5.0 / 18.0,
            Self::Quad => 1.0,
            Self::Wide => 2.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MathText {
    pub(crate) text: String,
    pub(crate) kind: MathTextKind,
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MathTextKind {
    Grapheme,
    Number,
    Upright,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MathIdentifier {
    pub(crate) name: String,
    pub(crate) symbol: Option<&'static str>,
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MathOperator {
    pub(crate) operator: String,
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MathShorthand {
    pub(crate) source: String,
    pub(crate) replacement: &'static str,
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MathStringLiteral {
    pub(crate) text: String,
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MathGroup {
    pub(crate) left: char,
    pub(crate) right: char,
    pub(crate) body: Vec<MathNode>,
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MathAttach {
    pub(crate) base: Box<MathNode>,
    pub(crate) top: Option<Vec<MathNode>>,
    pub(crate) bottom: Option<Vec<MathNode>>,
    pub(crate) top_left: Option<Vec<MathNode>>,
    pub(crate) top_right: Option<Vec<MathNode>>,
    pub(crate) bottom_left: Option<Vec<MathNode>>,
    pub(crate) bottom_right: Option<Vec<MathNode>>,
    pub(crate) primes: usize,
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MathFraction {
    pub(crate) numerator: Vec<MathNode>,
    pub(crate) denominator: Vec<MathNode>,
    pub(crate) style: MathFractionStyle,
    pub(crate) slash_range: Range<usize>,
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MathFractionStyle {
    Vertical,
    Skewed,
    Horizontal,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MathCancel {
    pub(crate) body: Vec<MathNode>,
    pub(crate) options: MathCancelOptions,
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MathCancelOptions {
    pub(crate) length: MathCancelLength,
    pub(crate) inverted: bool,
    pub(crate) cross: bool,
    pub(crate) angle: MathCancelAngle,
    pub(crate) stroke: DecorationStroke,
}

impl Default for MathCancelOptions {
    fn default() -> Self {
        Self {
            length: MathCancelLength::default(),
            inverted: false,
            cross: false,
            angle: MathCancelAngle::Auto,
            stroke: DecorationStroke::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MathCancelLength {
    pub(crate) relative: f32,
    pub(crate) absolute_em: f32,
}

impl Default for MathCancelLength {
    fn default() -> Self {
        Self {
            relative: 1.0,
            absolute_em: 0.3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum MathCancelAngle {
    Auto,
    Degrees(f32),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MathAccent {
    pub(crate) base: Vec<MathNode>,
    pub(crate) accent: char,
    pub(crate) size: MathAccentSize,
    pub(crate) dotless: bool,
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MathCall {
    pub(crate) name: String,
    pub(crate) args: Vec<MathArg>,
    pub(crate) options: MathCallOptions,
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct MathCallOptions {
    pub(crate) delimiter_size: Option<MathDelimitedSize>,
    pub(crate) stretch_size: Option<MathStretchSize>,
}

pub(crate) type MathDelimitedSize = MathRelativeSize;
pub(crate) type MathStretchSize = MathRelativeSize;
pub(crate) type MathAccentSize = MathRelativeSize;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MathRelativeSize {
    pub(crate) relative: f32,
    pub(crate) absolute_em: f32,
    pub(crate) absolute_pt: f32,
}

impl Default for MathRelativeSize {
    fn default() -> Self {
        Self {
            relative: 1.0,
            absolute_em: 0.0,
            absolute_pt: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MathArg {
    pub(crate) nodes: Vec<MathNode>,
    pub(crate) byte_range: Range<usize>,
}
