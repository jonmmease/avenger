//! The syntax of avenger-sql: a subset of GoogleSQL's grammar, read into a lossless syntax
//! tree with typed nodes.
//!
//! A script is statements separated by `;`: pipe queries, `IMPORT MODULE`, `DECLARE` and `SET`
//! of variables, `CREATE TEMP TABLE`, and `CREATE FUNCTION` of scalar, aggregate and table
//! functions. A query is a `FROM` clause, or a query in parentheses, and pipe operators, and
//! every nested query has the same form. [`parse_module`] reads a module, a `.av.sqlm` file of
//! GoogleSQL, whose public functions, table functions and constants a script imports with
//! `IMPORT MODULE`. [`parse_data`] reads a data script, which registers the tables of a project
//! with GoogleSQL's `CREATE EXTERNAL TABLE`, `LOAD DATA`, `CREATE VIEW`, `CREATE TABLE ... AS`
//! and `CREATE SCHEMA`, and the scripts of [`parse`] read those tables. Within the subset, the
//! crate accepts what GoogleSQL accepts, rejects what it rejects, and reports its first error
//! where GoogleSQL does.
//!
//! The one exception reads reserved words as names where avenger's charts need them: a word
//! that `=>` follows is an argument's name, as in `scale(range => 'tableau10')`, and `SELECT`
//! and `INTERVAL` that `(` follows are calls, unless a date part follows the parentheses of
//! `INTERVAL`, as in the literal `INTERVAL (x) DAY`. GoogleSQL rejects these, and every text
//! that GoogleSQL accepts reads as it does there.
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
//! [`parse`] runs on every tree, reports the checks that GoogleSQL makes after its parser, and
//! the statements that the role of the text doesn't hold: the rules of modules and of data
//! scripts, the order of `DECLARE`, the clauses that each kind of function takes, and the
//! escapes of literals. [`Parse::diagnostics`] holds all of them in the order of their ranges.
//!
//! # Completion and highlighting
//!
//! [`expected_at`] tells an editor what the grammar accepts at a cursor of a text in its
//! [`Role`]: keywords, tokens, names in their roles, expressions, types, pipe operators, and the
//! statements that the role holds. [`highlight`]
//! classifies the tokens of a tree for syntax highlighting: keywords where the parser read them
//! as keywords, literals, operators, and names in the same roles.
//!
//! # Layout
//!
//! [`layout`] lays out text in lines of at most [`layout::WIDTH`] characters: a
//! [`layout::Doc`] is text with the places where it can break, and [`layout::list`] lays out a
//! call or an array on one line or with one item to a line.
//!
//! # Formatting
//!
//! [`format::format`] writes a text of a [`Role`] in the style of avenger-sql's `STYLE.md`, from
//! the syntax tree alone: it changes whitespace, the case of keywords and of the names of
//! functions, and the commas after the last items of lists, and keeps a statement with an error,
//! such as a statement that the role doesn't hold, as written.
//! [`indent::depth`] gives the depth of a line by the same style from the lexer's tokens, so
//! that an editor can indent unfinished text on Enter and Tab.

pub mod ast;
pub mod diagnostic;
mod escape;
pub mod format;
mod highlight;
pub mod indent;
pub mod layout;
pub mod lexer;
mod parser;
mod syntax_kind;
mod validation;

use rowan::{GreenNode, TextSize};

pub use diagnostic::{line_column, Diagnostic, DiagnosticCode, Fix, TextEdit};
pub use highlight::{highlight, Highlight, HighlightRange};
pub use parser::is_reserved;
pub use rowan::{TextRange, TextSize as Offset};
pub use syntax_kind::{SqlLanguage, SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};

/// The role of a text in a project, which sets the statements that it holds: a chart script
/// reads tables and runs queries, a module defines the functions and constants that scripts
/// import, and a data script registers the tables that the scripts read. The parser reads the
/// statements of every role in every text, and the validation pass reports a statement that the
/// role of its text doesn't hold, with help that names its place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    /// A chart script, which [`parse`] reads: pipe queries, `IMPORT MODULE`, `DECLARE`, `SET`,
    /// `CREATE TEMP TABLE`, `CREATE TEMP FUNCTION`, and its own tables of files, `CREATE TEMP
    /// EXTERNAL TABLE` and `LOAD DATA ... TEMP TABLE`.
    Script,
    /// A module, a `.av.sqlm` file, which [`parse_module`] reads.
    Module,
    /// A data script, which [`parse_data`] reads.
    Data,
}

/// The result of a parse: the syntax tree and the diagnostics of the text.
#[derive(Clone, Debug)]
pub struct Parse {
    green: GreenNode,
    diagnostics: Vec<Diagnostic>,
}

impl Parse {
    /// A parse of the tree `green` of a text of `role`, with the diagnostics of the lexer and
    /// the parser, to which it adds those of the validation pass.
    fn new(green: GreenNode, mut diagnostics: Vec<Diagnostic>, role: Role) -> Self {
        diagnostics.extend(validation::validate(
            &SyntaxNode::new_root(green.clone()),
            role,
        ));
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
    parse_role(source, Role::Script)
}

/// The tree of a module, the text of a `.av.sqlm` file.
pub fn parse_module(source: &str) -> Parse {
    parse_role(source, Role::Module)
}

/// The tree of a data script, whose statements register the tables of a project: `CREATE
/// EXTERNAL TABLE`, `LOAD DATA`, `CREATE VIEW`, `CREATE TABLE ... AS` and `CREATE SCHEMA`, with
/// `CREATE TEMP FUNCTION` and `IMPORT MODULE` for its own statements. Its root is a
/// [`SyntaxKind::SCRIPT`] node, as a script's is, and [`Parse::script`] gives it.
///
/// ```
/// use avenger_sql_syntax::{parse, parse_data};
///
/// let text = "CREATE EXTERNAL TABLE sales OPTIONS (format = 'CSV', uris = ['sales.csv'])";
/// assert!(parse_data(text).ok());
/// // A script reads the tables of the data scripts, and its own tables of files are TEMP.
/// assert!(!parse(text).ok());
/// assert!(parse(&text.replace("CREATE", "CREATE TEMP")).ok());
/// // A data script registers tables, and runs no queries.
/// assert!(!parse_data("FROM sales").ok());
/// ```
pub fn parse_data(source: &str) -> Parse {
    parse_role(source, Role::Data)
}

/// The tree of a text of `role`, as [`parse`], [`parse_module`] or [`parse_data`] reads it.
pub(crate) fn parse_role(source: &str, role: Role) -> Parse {
    let (green, diagnostics, _) = parser::parse(source, role, None);
    Parse::new(green, diagnostics, role)
}

/// What the grammar accepts at `offset` of a text of `role`: the forms that a completion can offer
/// there. When a word ends at `offset`, as when a user is typing it, the forms are those that can
/// take the word's place. A text offers the statements that its role holds, so that `CREATE` in
/// a data script offers `EXTERNAL TABLE`, `VIEW` and `SCHEMA`, and a chart script offers only
/// `EXTERNAL TABLE`, after `CREATE TEMP`.
pub fn expected_at(source: &str, offset: TextSize, role: Role) -> Vec<Expected> {
    let offset = usize::from(offset).min(source.len());
    let start = source[..offset]
        .bytes()
        .rev()
        .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
        .count();
    let at = offset - start;
    let (_, _, expected) = parser::parse(&source[..at], role, Some(TextSize::from(at as u32)));
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
    /// One of a fixed set of words or phrases, such as a date part, a sampling method, or a pipe
    /// operator such as `ORDER BY`. Some words, such as `TRUE` or `DATE`, are not keywords of the
    /// lexer.
    Words(&'static [&'static str]),
    /// Words in a sequence, such as `BY NAME`, where the grammar takes the first only with the
    /// rest. When the text ends after the first words, the phrase holds the words after them.
    Phrase(&'static [&'static str]),
}

/// The words that start a constant: `NULL`, `TRUE`, `FALSE`, and the keywords of typed literals
/// that avenger-sql reads, as `DATE '2024-01-01'`.
pub const LITERAL_WORDS: &[&str] = &[
    "NULL",
    "TRUE",
    "FALSE",
    "DATE",
    "TIME",
    "DATETIME",
    "TIMESTAMP",
    "NUMERIC",
    "DECIMAL",
    "BIGNUMERIC",
    "BIGDECIMAL",
];

/// The date parts of an `INTERVAL`, as in `INTERVAL 3 DAY` or `INTERVAL '1-2' YEAR TO MONTH`.
pub const INTERVAL_PARTS: &[&str] = &[
    "YEAR",
    "QUARTER",
    "MONTH",
    "WEEK",
    "DAY",
    "HOUR",
    "MINUTE",
    "SECOND",
    "MILLISECOND",
    "MICROSECOND",
];

/// GoogleSQL's date parts, which `EXTRACT` and the functions of dates and times take. `WEEK`
/// also takes a day, as in `WEEK(MONDAY)`.
pub const DATE_PARTS: &[&str] = &[
    "YEAR",
    "ISOYEAR",
    "QUARTER",
    "MONTH",
    "WEEK",
    "ISOWEEK",
    "DAY",
    "DAYOFYEAR",
    "DAYOFWEEK",
    "HOUR",
    "MINUTE",
    "SECOND",
    "MILLISECOND",
    "MICROSECOND",
    "DATE",
    "TIME",
    "DATETIME",
];

/// The role of a name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NameRole {
    /// A column, or the first name of a path in an expression.
    Column,
    /// A table, a pipe `WITH` name, a temporary table, or a table or a view of a data script.
    Table,
    /// A schema, which groups tables under its name, as `finance` in `finance.orders`, and which
    /// a data script's `CREATE SCHEMA` defines.
    Schema,
    /// A function, by what a call of it computes.
    Function(FunctionRole),
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
    /// A name for the row of a source, as `p` in `p.x` after `FROM penguins AS p`, or a pattern
    /// variable of `MATCH_RECOGNIZE` that qualifies a column, as `dn` in `dn.x`. Only a semantic
    /// model gives this role: the syntax takes the first name of a path for a column.
    RangeVariable,
    Module,
    /// A date part, such as `MONTH` in `INTERVAL 1 MONTH`.
    DatePart,
    /// The name of an option.
    Option,
    /// Another name, such as a sampling method.
    Other,
}

/// What a call of a function computes. The syntax tells a table function by its place, after
/// `FROM` or a pipe `CALL`, and the kind of a function that a `CREATE` defines. A semantic model
/// tells the rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FunctionRole {
    /// A call whose function the syntax alone does not tell.
    Unresolved,
    /// A value of each row.
    Scalar,
    /// A value of the rows of a group, or of a window with `OVER`.
    Aggregate,
    /// A value of each row over a window.
    Window,
    /// The rows of a table, after `FROM` or a pipe `CALL`.
    Table,
    /// A mark of avenger-chart, which builds a chart of the rows of its group.
    Mark,
    /// A constructor of avenger-chart, which returns a fragment of a chart's message, a channel
    /// function, such as `x`, or `datum` or `value`.
    Constructor,
    /// A transform: `bin`, `stack`, `timeunit`, `lump`, or `impute`.
    Transform,
    /// A composition of avenger-chart, which composes charts: a layer, a concat, or a facet.
    Composition,
}

impl From<ast::FunctionKind> for FunctionRole {
    /// The role of a function that a `CREATE` defines, by its kind.
    fn from(kind: ast::FunctionKind) -> FunctionRole {
        match kind {
            ast::FunctionKind::Scalar => FunctionRole::Scalar,
            ast::FunctionKind::Aggregate => FunctionRole::Aggregate,
            ast::FunctionKind::Table => FunctionRole::Table,
        }
    }
}
