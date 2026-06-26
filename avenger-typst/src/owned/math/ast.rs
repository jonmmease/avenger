use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnedMath {
    pub(crate) source: String,
    pub(crate) nodes: Vec<OwnedMathNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OwnedMathNode {
    Space(OwnedMathSpace),
    Text(OwnedMathText),
    Identifier(OwnedMathIdentifier),
    Operator(OwnedMathOperator),
    Shorthand(OwnedMathShorthand),
    StringLiteral(OwnedMathStringLiteral),
    Group(OwnedMathGroup),
    Attach(OwnedMathAttach),
    Fraction(OwnedMathFraction),
    Call(OwnedMathCall),
}

impl OwnedMathNode {
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
pub(crate) struct OwnedMathSpace {
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnedMathText {
    pub(crate) text: String,
    pub(crate) kind: OwnedMathTextKind,
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnedMathTextKind {
    Grapheme,
    Number,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnedMathIdentifier {
    pub(crate) name: String,
    pub(crate) symbol: Option<&'static str>,
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnedMathOperator {
    pub(crate) operator: String,
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnedMathShorthand {
    pub(crate) source: String,
    pub(crate) replacement: &'static str,
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnedMathStringLiteral {
    pub(crate) text: String,
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnedMathGroup {
    pub(crate) left: char,
    pub(crate) right: char,
    pub(crate) body: Vec<OwnedMathNode>,
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnedMathAttach {
    pub(crate) base: Box<OwnedMathNode>,
    pub(crate) top: Option<Box<OwnedMathNode>>,
    pub(crate) bottom: Option<Box<OwnedMathNode>>,
    pub(crate) primes: usize,
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnedMathFraction {
    pub(crate) numerator: Box<OwnedMathNode>,
    pub(crate) denominator: Box<OwnedMathNode>,
    pub(crate) slash_range: Range<usize>,
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnedMathCall {
    pub(crate) name: String,
    pub(crate) args: Vec<OwnedMathArg>,
    pub(crate) byte_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnedMathArg {
    pub(crate) nodes: Vec<OwnedMathNode>,
    pub(crate) byte_range: Range<usize>,
}
