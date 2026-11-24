//! The cells of a Quarto document as one script: each cell's value, the statements that a value
//! reads, and the masked copy of the document's script that runs a value.
//!
//! A cell's value is its last query. It runs with the statements that it reads, in its own cell
//! and in the cells before it, and none of the others, so an error of a statement that it doesn't
//! read doesn't stop it. What a statement reads is found from the names in its text: a statement
//! reads the last statement before it that defines a temporary table, a temporary function, or a
//! module's alias that it names, and every `DECLARE` and `SET` before it of a variable that it
//! names, and what each of those reads in turn. A name that only matches, such as a column named
//! as a variable, reads the variable too, which runs a statement more and never one less.

use std::collections::{BTreeMap, BTreeSet};

use avenger_sql_project::qmd::Qmd;
use avenger_sql_syntax::{ast, ast::AstNode, SyntaxElement, SyntaxKind, SyntaxNode};

/// A statement of a document's script: the start of its tokens, the end of its `;`, its cell,
/// whether it is a query, the names that it defines, and the names in its text.
#[derive(Clone, Debug)]
pub(crate) struct CellStatement {
    pub(crate) start: usize,
    /// The end of the statement with its `;`, when one follows it.
    terminated: usize,
    pub(crate) cell: Option<usize>,
    pub(crate) query: bool,
    defines: Vec<(Kind, String)>,
    names: BTreeSet<String>,
}

/// What a statement defines a name as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// A temporary table, a table of files, a temporary function, or a module's alias, which a
    /// later definition replaces.
    Defined,
    /// A variable, whose `DECLARE` and each `SET` give it its value.
    Variable,
}

/// The statements of the script of `document`, in order.
pub(crate) fn statements(document: &Qmd) -> Vec<CellStatement> {
    let script = &document.script;
    let parse = avenger_sql_syntax::parse_cells(&script.text, &script.cell_starts());
    let Some(root) = ast::Script::cast(parse.syntax()) else {
        return Vec::new();
    };
    (root.stmts())
        .filter_map(|stmt| {
            let (start, end) = token_range(stmt.syntax())?;
            let terminated = semicolon_end(stmt.syntax()).unwrap_or(end);
            Some(CellStatement {
                start,
                terminated,
                cell: script.cell_at(start),
                query: matches!(stmt, ast::Stmt::Query(_)),
                defines: defines(&stmt),
                names: names(stmt.syntax()),
            })
        })
        .collect()
}

/// The index among `statements` of the value of the cell at `cell`: its last query, or `None` for
/// a cell without a query.
pub(crate) fn value(statements: &[CellStatement], cell: usize) -> Option<usize> {
    (statements.iter()).rposition(|statement| statement.query && statement.cell == Some(cell))
}

/// The statements that the statement at `at` runs with: the statements of its cell up to it, and
/// what each of those reads, by their indices among `statements`.
pub(crate) fn closure(statements: &[CellStatement], at: usize) -> BTreeSet<usize> {
    let cell = statements[at].cell;
    let mut keep: BTreeSet<usize> = (0..=at)
        .filter(|index| statements[*index].cell == cell)
        .collect();
    let mut queue: Vec<usize> = keep.iter().copied().collect();
    while let Some(index) = queue.pop() {
        for read in reads(statements, index) {
            if keep.insert(read) {
                queue.push(read);
            }
        }
    }
    keep
}

/// The statements before the statement at `at` that it reads.
fn reads(statements: &[CellStatement], at: usize) -> Vec<usize> {
    let mut latest: BTreeMap<&str, usize> = BTreeMap::new();
    let mut variables: Vec<usize> = Vec::new();
    let names = &statements[at].names;
    for (index, statement) in statements[..at].iter().enumerate() {
        for (kind, name) in &statement.defines {
            if !names.contains(name) {
                continue;
            }
            match kind {
                Kind::Defined => {
                    latest.insert(name, index);
                }
                Kind::Variable => variables.push(index),
            }
        }
    }
    latest.into_values().chain(variables).collect()
}

/// The script of `document` with only the statements of `keep`, cut after the statement at `at`:
/// every other statement, with its `;`, becomes spaces, and the text keeps its lines and its
/// offsets.
pub(crate) fn masked(
    document: &Qmd,
    statements: &[CellStatement],
    keep: &BTreeSet<usize>,
    at: usize,
) -> String {
    let mut bytes = document.script.text.as_bytes()[..statements[at].terminated].to_vec();
    for (index, statement) in statements.iter().enumerate().take(at) {
        if keep.contains(&index) {
            continue;
        }
        for byte in &mut bytes[statement.start..statement.terminated] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
    }
    String::from_utf8(bytes).expect("spaces in place of whole tokens")
}

/// The range of `node` from its first token to its last, without the comments and the spaces
/// around it.
fn token_range(node: &SyntaxNode) -> Option<(usize, usize)> {
    let mut tokens = (node.descendants_with_tokens())
        .filter_map(|element| element.into_token())
        .filter(|token| !token.kind().is_trivia());
    let first = tokens.next()?;
    let last = tokens.last().unwrap_or_else(|| first.clone());
    Some((
        usize::from(first.text_range().start()),
        usize::from(last.text_range().end()),
    ))
}

/// The end of the `;` after the statement `node`, when the next token that is not a space or a
/// comment is one.
fn semicolon_end(node: &SyntaxNode) -> Option<usize> {
    let mut next = node.next_sibling_or_token();
    while let Some(element) = next {
        match &element {
            SyntaxElement::Token(token) if token.kind().is_trivia() => {}
            SyntaxElement::Token(token) if token.kind() == SyntaxKind::SEMICOLON => {
                return Some(usize::from(token.text_range().end()));
            }
            _ => return None,
        }
        next = element.next_sibling_or_token();
    }
    None
}

/// The names that the statement `stmt` defines, in lowercase.
fn defines(stmt: &ast::Stmt) -> Vec<(Kind, String)> {
    let last = |path: Option<ast::Path>| {
        let names = path?.names()?;
        Some(names.last()?.to_lowercase())
    };
    let defined = |name: Option<String>| (name.into_iter()).map(|name| (Kind::Defined, name));
    match stmt {
        ast::Stmt::CreateTableStmt(create) => defined(last(create.path())).collect(),
        ast::Stmt::CreateExternalTableStmt(create) => defined(last(create.path())).collect(),
        ast::Stmt::CreateViewStmt(create) => defined(last(create.path())).collect(),
        ast::Stmt::LoadDataStmt(load) => defined(last(load.path())).collect(),
        ast::Stmt::CreateFunctionStmt(create) => defined(last(create.path())).collect(),
        ast::Stmt::ImportStmt(import) => {
            let alias = (import.alias().and_then(|alias| alias.name()))
                .and_then(|name| name.text())
                .map(|name| name.to_lowercase());
            defined(alias.or_else(|| last(import.path()))).collect()
        }
        ast::Stmt::DeclareStmt(declare) => (declare.syntax().children())
            .filter_map(ast::Name::cast)
            .filter_map(|name| name.text())
            .map(|name| (Kind::Variable, name.to_lowercase()))
            .collect(),
        ast::Stmt::SetStmt(set) => {
            let targets = (set.syntax().children()).flat_map(|child| {
                match ast::NameRef::cast(child.clone()) {
                    Some(name) => vec![name],
                    None => child.children().filter_map(ast::NameRef::cast).collect(),
                }
            });
            // The value after `=` is an expression, whose names are inside it, so the names that
            // are children of the `SET`, or of its list, are the names that it sets.
            targets
                .filter_map(|name| name.text())
                .map(|name| (Kind::Variable, name.to_lowercase()))
                .collect()
        }
        _ => Vec::new(),
    }
}

/// The names in the text of `node`, in lowercase: each part of a path, each variable, and each
/// called function.
fn names(node: &SyntaxNode) -> BTreeSet<String> {
    (node.descendants())
        .filter_map(ast::NameRef::cast)
        .filter_map(|name| name.text())
        .map(|name| name.to_lowercase())
        .collect()
}
