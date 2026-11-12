//! The syntax of avenger-sql: a subset of GoogleSQL's grammar, read into a lossless syntax
//! tree.
//!
//! A script is statements separated by `;`: pipe queries, `IMPORT MODULE`, `DECLARE` and `SET`
//! of variables, `CREATE TEMP TABLE`, and `CREATE FUNCTION` of scalar, aggregate and table
//! functions. A query is a `FROM` clause, or a query in parentheses, and pipe operators, and
//! every nested query has the same form. [`parse_module`] reads a module, a `.sqlm` file of
//! GoogleSQL, whose public functions, table functions and constants a script imports with
//! `IMPORT MODULE`. Within the subset, the parser accepts what GoogleSQL accepts, rejects what
//! it rejects, and its first error on invalid text is at GoogleSQL's.
//!
//! [`parse`] never fails. Its tree, a `rowan` tree of [`SyntaxKind`]s, holds every byte of the
//! text, comments and whitespace included, and every text gives a tree: the parser reports a
//! [`Diagnostic`] for each error, skips what it cannot read into an `ERROR` node, and goes on.
//! [`expected_at`] tells an editor what the grammar accepts at a cursor.

pub mod diagnostic;
pub mod lexer;
mod parser;
mod syntax_kind;

use rowan::{GreenNode, TextSize};

pub use diagnostic::{Diagnostic, DiagnosticCode, Fix, TextEdit};
pub use rowan::{TextRange, TextSize as Offset};
pub use syntax_kind::{SqlLanguage, SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};

/// The result of a parse: the syntax tree and the diagnostics of the text.
#[derive(Clone, Debug)]
pub struct Parse {
    green: GreenNode,
    diagnostics: Vec<Diagnostic>,
}

impl Parse {
    /// The root of the tree: a [`SyntaxKind::SCRIPT`] or a [`SyntaxKind::MODULE`] node.
    pub fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }

    /// The diagnostics of the text, in the order of their ranges.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Whether the text has no diagnostics.
    pub fn ok(&self) -> bool {
        self.diagnostics.is_empty()
    }
}

/// The tree of a script, the text of statements separated by `;`.
pub fn parse(source: &str) -> Parse {
    let (green, diagnostics, _) = parser::parse(source, parser::Entry::Script, None);
    Parse { green, diagnostics }
}

/// The tree of a module, the text of a `.sqlm` file.
pub fn parse_module(source: &str) -> Parse {
    let (green, diagnostics, _) = parser::parse(source, parser::Entry::Module, None);
    Parse { green, diagnostics }
}

/// What the grammar accepts at `offset` of a script's text: the forms that a completion can
/// offer there. When a word ends at `offset`, as when a user is typing it, the forms are those
/// that can take the word's place.
pub fn expected_at(source: &str, offset: TextSize) -> Vec<Expected> {
    let offset = usize::from(offset).min(source.len());
    let start = source[..offset]
        .bytes()
        .rev()
        .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
        .count();
    let at = offset - start;
    let (_, _, expected) = parser::parse(
        &source[..at],
        parser::Entry::Script,
        Some(TextSize::from(at as u32)),
    );
    expected
}

/// A form that the grammar accepts at a cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Expected {
    /// A keyword, such as `BY` after `GROUP`.
    Keyword(SyntaxKind),
    /// A punctuation token, such as `(`.
    Token(SyntaxKind),
    /// A name in a role, such as a column or a table.
    Name(NameRole),
    Expression,
    Type,
    /// A pipe operator, after `|>`.
    PipeOperator,
    /// The start of a statement.
    Statement,
}

/// The role of a name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NameRole {
    /// A column, or the first name of a path in an expression.
    Column,
    /// A table, a pipe `WITH` name, or a temporary table.
    Table,
    Function,
    Type,
    /// A name that a node defines, such as an alias.
    Alias,
    /// A field after a `.`.
    Field,
    /// A named window.
    Window,
    /// A query that a pipe `WITH` names.
    Query,
    /// A parameter of a function, or the name of a named argument.
    Parameter,
    /// A variable of a script.
    Variable,
    Module,
    /// A date part, such as `MONTH` in `INTERVAL 1 MONTH`.
    DatePart,
    /// The name of an option.
    Option,
    /// Another name, such as a sampling method.
    Other,
}
