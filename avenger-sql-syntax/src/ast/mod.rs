//! Typed nodes over the syntax tree.
//!
//! Each node of the tree is a [`SyntaxNode`] of a [`SyntaxKind`]. A typed node, such as
//! [`PipeWhere`], wraps a node of one kind and has an accessor for each of its children, which
//! `sql.ungram` describes. An enum, such as [`Expr`], wraps a node of any of its kinds. A typed
//! node is a view: [`AstNode::cast`] makes one from a node of its kind, and
//! [`AstNode::syntax`] gives the node back, with its range and its tokens.
//!
//! The tree of a text with errors can lack any part, so every accessor returns an `Option`, or
//! an iterator that can be empty. A labeled child, such as [`BinExpr::rhs`], is the child of its
//! type at the label's place: in `a +` with no right operand, `rhs` is `None`, and in a tree
//! that lacks the left operand, the right one is `lhs`.
//!
//! The accessors are generated from `sql.ungram` by `build.rs`. The accessors that decode text,
//! such as [`Literal::string_value`] and [`NameRef::text`], are in `manual`.

mod manual;

use std::marker::PhantomData;

use crate::{SyntaxKind, SyntaxNode, SyntaxToken};

pub use manual::{BinaryOp, CreateStmt, FunctionKind, LiteralKind, PrefixOp, Scope};

/// A typed view of a node of the syntax tree.
pub trait AstNode: Sized {
    /// Whether a node of `kind` is a node of this type.
    fn can_cast(kind: SyntaxKind) -> bool;

    /// The node as this type, when its kind is one of this type's.
    fn cast(syntax: SyntaxNode) -> Option<Self>;

    fn syntax(&self) -> &SyntaxNode;

    /// The range of the node's text, without the trivia at its edges.
    fn range(&self) -> rowan::TextRange {
        self.syntax().text_range()
    }
}

/// The children of a node that are nodes of type `N`, in order.
#[derive(Clone, Debug)]
pub struct AstChildren<N> {
    inner: rowan::SyntaxNodeChildren<crate::SqlLanguage>,
    ty: PhantomData<N>,
}

impl<N: AstNode> Iterator for AstChildren<N> {
    type Item = N;

    fn next(&mut self) -> Option<N> {
        self.inner.find_map(N::cast)
    }
}

mod support {
    use super::{AstChildren, AstNode};
    use crate::{SyntaxKind, SyntaxNode, SyntaxToken};

    /// The `nth` child of `parent` of type `N`.
    pub(super) fn child<N: AstNode>(parent: &SyntaxNode, nth: usize) -> Option<N> {
        parent.children().filter_map(N::cast).nth(nth)
    }

    pub(super) fn children<N: AstNode>(parent: &SyntaxNode) -> AstChildren<N> {
        AstChildren {
            inner: parent.children(),
            ty: std::marker::PhantomData,
        }
    }

    /// The first token of `parent`, not of its children, whose kind is one of `kinds`.
    pub(super) fn token(parent: &SyntaxNode, kinds: &[SyntaxKind]) -> Option<SyntaxToken> {
        parent
            .children_with_tokens()
            .filter_map(|element| element.into_token())
            .find(|token| kinds.contains(&token.kind()))
    }
}

/// The tokens of `node`, not of its children, other than trivia.
pub(crate) fn tokens(node: &SyntaxNode) -> impl Iterator<Item = SyntaxToken> {
    node.children_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| !token.kind().is_trivia())
}

/// A typed node of one kind.
macro_rules! ast_node {
    ($(#[$meta:meta])* $name:ident, $kind:ident) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        pub struct $name {
            pub(crate) syntax: SyntaxNode,
        }

        impl AstNode for $name {
            fn can_cast(kind: SyntaxKind) -> bool {
                kind == SyntaxKind::$kind
            }

            fn cast(syntax: SyntaxNode) -> Option<Self> {
                Self::can_cast(syntax.kind()).then_some(Self { syntax })
            }

            fn syntax(&self) -> &SyntaxNode {
                &self.syntax
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                std::fmt::Display::fmt(&self.syntax, f)
            }
        }
    };
}

/// A typed node of any of the kinds of its variants.
macro_rules! ast_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        pub enum $name {
            $($variant($variant),)*
        }

        impl AstNode for $name {
            fn can_cast(kind: SyntaxKind) -> bool {
                $($variant::can_cast(kind))||*
            }

            fn cast(syntax: SyntaxNode) -> Option<Self> {
                let kind = syntax.kind();
                $(if $variant::can_cast(kind) {
                    return $variant::cast(syntax).map(Self::$variant);
                })*
                None
            }

            fn syntax(&self) -> &SyntaxNode {
                match self {
                    $(Self::$variant(node) => node.syntax(),)*
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                std::fmt::Display::fmt(self.syntax(), f)
            }
        }
    };
}

include!(concat!(env!("OUT_DIR"), "/nodes.rs"));
