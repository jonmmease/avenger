//! The syntax of avenger-sql: a subset of GoogleSQL's grammar, read into a lossless syntax
//! tree with typed nodes.
//!
//! A script is statements separated by `;`: pipe queries, `IMPORT MODULE`, `DECLARE` and `SET`
//! of variables, `CREATE TEMP TABLE`, and `CREATE FUNCTION` of scalar, aggregate and table
//! functions. A query is a `FROM` clause, or a query in parentheses, and pipe operators, and
//! every nested query has the same form. [`parse_module`] reads a module, a `.sqlm` file of
//! GoogleSQL, whose public functions, table functions and constants a script imports with
//! `IMPORT MODULE`. Within the subset, the crate accepts what GoogleSQL accepts, rejects what
//! it rejects, and reports its first error where GoogleSQL does.
//!
//! # The tree
//!
//! [`parse`] never fails, and every text gives a tree. The tree is a `rowan` tree of
//! [`SyntaxNode`]s, each of a [`SyntaxKind`], and holds every byte of the text, comments and
//! whitespace included, with a range on every node. The grammar file `sql.ungram` describes
//! the children of each kind of node, and a test checks that the parser builds those shapes.
//!
//! # Typed nodes
//!
//! [`ast`] has a typed node for each kind, generated from `sql.ungram`, with an accessor for
//! each child, such as [`ast::PipeWhere::condition`], and an enum for each choice of nodes, such
//! as [`ast::Expr`]. [`Parse::script`] gives the root of a script as an [`ast::Script`]. Since
//! the tree of a text with errors can lack any part, an accessor returns an `Option` or an
//! iterator. Accessors such as [`ast::Literal::string_value`] and [`ast::NameRef::text`] decode
//! values.
//!
//! ```
//! use avenger_sql_syntax::{ast, parse};
//!
//! let parse = parse("FROM orders |> WHERE total > 10 |> AGGREGATE SUM(total) GROUP BY region");
//! assert!(parse.ok());
//! let script = parse.script().unwrap();
//! let Some(ast::Stmt::Query(query)) = script.stmts().next() else { unreachable!() };
//! let Some(ast::PipeOperator::PipeWhere(filter)) = query.pipe_operators().next() else {
//!     unreachable!()
//! };
//! assert_eq!(filter.condition().unwrap().to_string(), "total > 10");
//! for operator in query.pipe_operators() {
//!     println!("{operator}");
//! }
//! ```
//!
//! # Diagnostics
//!
//! Each error is a [`Diagnostic`], with a code, a range, a message, help, and the edits that
//! fix it. The lexer reports malformed tokens. The parser reports the errors of the grammar and
//! puts what it cannot read into `ERROR` nodes, so it can go on. A validation pass, which
//! [`parse`] runs on every tree, reports the checks that GoogleSQL makes after its parser: the
//! rules of modules, the order of `DECLARE`, the clauses that each kind of function takes, and
//! the escapes of literals. [`Parse::diagnostics`] holds all of them in the order of their
//! ranges.
//!
//! # Completion and highlighting
//!
//! [`expected_at`] tells an editor what the grammar accepts at a cursor: keywords, tokens,
//! names in their roles, expressions, types, pipe operators, and statements. [`highlight`]
//! classifies the tokens of a tree for syntax highlighting: keywords where the parser read them
//! as keywords, literals, operators, and names in the same roles.

pub mod ast;
pub mod diagnostic;
mod escape;
mod highlight;
pub mod lexer;
mod parser;
mod syntax_kind;
mod validation;

use rowan::{GreenNode, TextSize};

pub use diagnostic::{Diagnostic, DiagnosticCode, Fix, TextEdit};
pub use highlight::{highlight, Highlight, HighlightRange};
pub use parser::is_reserved;
pub use rowan::{TextRange, TextSize as Offset};
pub use syntax_kind::{SqlLanguage, SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};

/// The result of a parse: the syntax tree and the diagnostics of the text.
#[derive(Clone, Debug)]
pub struct Parse {
    green: GreenNode,
    diagnostics: Vec<Diagnostic>,
}

impl Parse {
    /// A parse of the tree `green`, with the diagnostics of the lexer and the parser, to which
    /// it adds those of the validation pass.
    fn new(green: GreenNode, mut diagnostics: Vec<Diagnostic>) -> Self {
        diagnostics.extend(validation::validate(&SyntaxNode::new_root(green.clone())));
        diagnostics.sort_by_key(|diagnostic| diagnostic.range.start());
        Parse { green, diagnostics }
    }

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
    Parse::new(green, diagnostics)
}

/// The tree of a module, the text of a `.sqlm` file.
pub fn parse_module(source: &str) -> Parse {
    let (green, diagnostics, _) = parser::parse(source, parser::Entry::Module, None);
    Parse::new(green, diagnostics)
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
    /// A pattern variable of `MATCH_RECOGNIZE`, which `DEFINE` defines.
    PatternVariable,
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
