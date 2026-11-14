//! Syntax highlighting from the tree.
//!
//! [`highlight`] classifies the tokens of a tree. Its classes follow the tree, not the token
//! stream: a word is a [`Highlight::Keyword`] only where the parser read it as a keyword, so a
//! column named `name` is a name in `|> SELECT name` and `NAME` is a keyword in `BY NAME`. A
//! name's class is its role, which the node around it gives: the path of a `FROM` item is a
//! table, the path of a call is a function, and the first name of a path in an expression is a
//! column or a variable, which only scope tells apart.
//!
//! The classes map onto the semantic tokens of the Language Server Protocol: `Keyword` to
//! `keyword`, `Comment` to `comment`, `String` to `string`, `Number` to `number`, `Operator` to
//! `operator`, and the roles of names to `class` (tables), `function`, `type`, `parameter`,
//! `variable`, `property` (fields) and `namespace` (modules).

use rowan::TextRange;

use crate::{ast::AstNode, NameRole, SyntaxKind::*, SyntaxNode, SyntaxToken};

/// The class of a range of text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Highlight {
    Keyword,
    Comment,
    /// A string or a bytes literal.
    String,
    /// An integer or a float literal.
    Number,
    /// A query parameter: `@name` or `?`.
    QueryParameter,
    /// An operator, such as `+`, `=`, `||`, `=>` or `|>`.
    Operator,
    /// `(`, `)`, `[`, `]`, `,`, `.` or `;`.
    Punctuation,
    /// A name in a role.
    Name(NameRole),
}

/// A range of text and its class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HighlightRange {
    pub range: TextRange,
    pub highlight: Highlight,
    /// Whether the range is a name that its node defines, such as an alias, a parameter, or the
    /// name of a `CREATE`, rather than one that refers to a definition.
    pub definition: bool,
}

/// The classes of the tokens of the tree `root`, in the order of the text, without overlaps.
/// Whitespace has no class, nor do the words that the parser skipped into `ERROR` nodes. A name
/// is one range, though a dashed table name, such as `my-project`, holds several tokens.
pub fn highlight(root: &SyntaxNode) -> Vec<HighlightRange> {
    let mut ranges = Vec::new();
    for token in root
        .descendants_with_tokens()
        .filter_map(|element| element.into_token())
    {
        let parent = token.parent();
        if let Some(name) = parent.filter(|node| matches!(node.kind(), NAME | NAME_REF)) {
            // A name is one range, at its first token.
            if name.first_token().as_ref() == Some(&token) {
                let (role, definition) = name_role(&name);
                ranges.push(HighlightRange {
                    range: name.text_range(),
                    highlight: Highlight::Name(role),
                    definition,
                });
            }
            continue;
        }
        if let Some(highlight) = token_class(&token) {
            ranges.push(HighlightRange {
                range: token.text_range(),
                highlight,
                definition: false,
            });
        }
    }
    ranges
}

/// The class of a token that isn't part of a name.
fn token_class(token: &SyntaxToken) -> Option<Highlight> {
    let kind = token.kind();
    let class = match kind {
        COMMENT => Highlight::Comment,
        STRING | BYTES => Highlight::String,
        INTEGER | FLOAT => Highlight::Number,
        PARAM | POSITIONAL => Highlight::QueryParameter,
        EQ | NEQ | LT | LT_EQ | GT | GT_EQ | PLUS | MINUS | STAR | SLASH | CONCAT | BIT_OR
        | AMP | CARET | TILDE | SHL | FAT_ARROW | PIPE => Highlight::Operator,
        L_PAREN | R_PAREN | L_BRACK | R_BRACK | COMMA | DOT | SEMICOLON => Highlight::Punctuation,
        _ if kind.is_keyword() => Highlight::Keyword,
        _ => return None,
    };
    Some(class)
}

/// The role of the `NAME` or `NAME_REF` node `name`, and whether the name is a definition.
fn name_role(name: &SyntaxNode) -> (NameRole, bool) {
    let Some(parent) = name.parent() else {
        return (NameRole::Other, false);
    };
    if name.kind() == NAME {
        let role = match parent.kind() {
            ALIAS | PIPE_AS | RENAME_ITEM => NameRole::Alias,
            REPLACE_ITEM | COLUMN_DEF => NameRole::Column,
            STRUCT_FIELD => match parent.parent().map(|node| node.kind()) {
                Some(TABLE_TYPE) => NameRole::Column,
                _ => NameRole::Field,
            },
            NAMED_QUERY => NameRole::Query,
            NAMED_WINDOW => NameRole::Window,
            PARAMETER => NameRole::Parameter,
            DECLARE_STMT => NameRole::Variable,
            OPTION_ENTRY => NameRole::Option,
            _ => NameRole::Other,
        };
        return (role, true);
    }
    let role = match parent.kind() {
        PATH => return path_role(name, &parent),
        NAMED_TYPE => NameRole::Type,
        FIELD_EXPR => NameRole::Field,
        NAME_LIST => match parent.parent().map(|node| node.kind()) {
            Some(SET_STMT) => NameRole::Variable,
            _ => NameRole::Column,
        },
        SET_STMT => NameRole::Variable,
        PIPE_DROP | ASSIGNMENT | RENAME_ITEM => NameRole::Column,
        NAMED_ARG | NAMED_TABLE_ARG => NameRole::Parameter,
        OVER_CLAUSE | WINDOW_SPEC | NAMED_WINDOW => NameRole::Window,
        INTERVAL_EXPR => NameRole::DatePart,
        _ => NameRole::Other,
    };
    (role, false)
}

/// The role of a name of the path `path`, by the node that holds the path.
fn path_role(name: &SyntaxNode, path: &SyntaxNode) -> (NameRole, bool) {
    let first = path.children().next().as_ref() == Some(name);
    let Some(holder) = path.parent() else {
        return (NameRole::Other, false);
    };
    let role = match holder.kind() {
        TABLE_SOURCE | TABLE_ARG | NAMED_TABLE_ARG | WILDCARD => NameRole::Table,
        TVF_SOURCE | CALL_EXPR | PIPE_CALL => NameRole::Function,
        IMPORT_STMT | MODULE_STMT => NameRole::Module,
        PATH_LIST | UNPIVOT_CLAUSE => NameRole::Column,
        CREATE_TABLE_STMT => return (NameRole::Table, true),
        CREATE_FUNCTION_STMT => return (NameRole::Function, true),
        CREATE_CONSTANT_STMT => return (NameRole::Variable, true),
        PATH_EXPR if !first => NameRole::Field,
        PATH_EXPR if is_date_part(&holder) => NameRole::DatePart,
        PATH_EXPR => NameRole::Column,
        _ => NameRole::Other,
    };
    (role, false)
}

/// Whether the path expression `expr` is the date part of an `EXTRACT`, as `MONTH` is in
/// `EXTRACT(MONTH FROM d)`.
fn is_date_part(expr: &SyntaxNode) -> bool {
    let Some(parent) = expr.parent().filter(|node| node.kind() == EXTRACT_EXPR) else {
        return false;
    };
    let first_expr = parent
        .children()
        .find(|child| crate::ast::Expr::can_cast(child.kind()));
    first_expr.as_ref() == Some(expr)
}
