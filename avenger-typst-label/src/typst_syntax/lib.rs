//! Parser and syntax tree for Typst label markup.
//!
//! The files in `upstream/` are generated from upstream `typst-syntax` by `tools/typst-sync`:
//! the pinned upstream source minus the whole items listed in `tools/typst-sync/manifest.toml`,
//! with crate paths rewritten. Do not edit them by hand. This module root and `path.rs` are
//! Avenger's.

#[rustfmt::skip]
#[path = "upstream/ast.rs"]
#[allow(dead_code, reason = "upstream AST surface; the evaluator uses a subset")]
#[expect(unused_imports, reason = "grouped upstream imports keep names of removed items")]
pub mod ast;
#[rustfmt::skip]
#[path = "upstream/kind.rs"]
#[allow(dead_code, reason = "upstream typst-syntax subset")]
mod kind;
#[rustfmt::skip]
#[path = "upstream/lexer.rs"]
#[allow(dead_code, reason = "upstream typst-syntax subset")]
mod lexer;
#[rustfmt::skip]
#[path = "upstream/node.rs"]
#[allow(dead_code, reason = "upstream typst-syntax subset")]
#[expect(unused_imports, reason = "grouped upstream imports keep names of removed items")]
mod node;
#[rustfmt::skip]
#[path = "upstream/parser.rs"]
#[allow(dead_code, reason = "upstream typst-syntax subset")]
mod parser;
mod path;
#[rustfmt::skip]
#[path = "upstream/set.rs"]
mod set;
#[rustfmt::skip]
#[path = "upstream/span.rs"]
#[allow(dead_code, reason = "upstream typst-syntax subset")]
mod span;

pub use self::kind::SyntaxKind;
pub use self::lexer::{is_ident, is_newline, split_newlines};
pub use self::node::{SyntaxDiagnostic, SyntaxNode};
pub use self::parser::{parse, parse_math};
pub use self::path::FileId;
pub use self::span::{
    DiagSpan, RangeMapper, Span, SpanKind, SpanNumber, Spanned, SubRange,
};
// avenger: only tests resolve diagnostic spans so far.
#[cfg(test)]
pub use self::span::DiagSpanKind;

use self::lexer::Lexer;

/// The syntax mode of a portion of Typst code.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum SyntaxMode {
    /// Text and markup, as in the top level.
    Markup,
    /// Math atoms, operators, etc., as in equations.
    Math,
    /// Keywords, literals and operators, as after hashes.
    Code,
}
