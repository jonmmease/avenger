use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MathAst {
    pub(crate) source: String,
    pub(crate) nodes: Vec<MathNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MathNode {
    Space(MathSpace),
    Text(MathText),
    Identifier(MathIdentifier),
    Operator(MathOperator),
    Shorthand(MathShorthand),
    StringLiteral(MathStringLiteral),
    Group(MathGroup),
    Attach(MathAttach),
    Fraction(MathFraction),
    Call(MathCall),
}

impl MathNode {
    pub(crate) fn byte_range(&self) -> Range<usize> {
        match self {
            Self::Space(node) => node.byte_range.clone(),
            Self::Text(node) => node.byte_range.clone(),
            Self::Identifier(node) => node.byte_range.clone(),
            Self::Operator(node) => node.byte_range.clone(),
            Self::Shorthand(node) => node.byte_range.clone(),
            Self::StringLiteral(node) => node.byte_range.clone(),
            Self::Group(node) => node.byte_range.clone(),
            Self::Attach(node) => node.byte_range.clone(),
            Self::Fraction(node) => node.byte_range.clone(),
            Self::Call(node) => node.byte_range.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MathSpace {
    pub(crate) byte_range: Range<usize>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MathGroup {
    pub(crate) left: char,
    pub(crate) right: char,
    pub(crate) body: Vec<MathNode>,
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MathFraction {
    pub(crate) numerator: Box<MathNode>,
    pub(crate) denominator: Box<MathNode>,
    pub(crate) slash_range: Range<usize>,
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MathCall {
    pub(crate) name: String,
    pub(crate) args: Vec<MathArg>,
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MathArg {
    pub(crate) nodes: Vec<MathNode>,
    pub(crate) byte_range: Range<usize>,
}
