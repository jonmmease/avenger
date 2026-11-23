//! A target at a line, `FILE:LINE`, and the statement that it names: the top-level statement of
//! a script, or of a cell, that holds the line, and the text that runs it, so a chart script's
//! earlier query shows its rows, and a statement that makes a table shows the table.

use avenger_sql_project::qmd::{Cell, Qmd};
use avenger_sql_syntax::{
    ast::{self, AstNode},
    SyntaxNode,
};

/// `target` without a line, and its line, from 1: the text after its last `:` when that text is
/// digits, so a Windows path keeps its drive letter.
pub fn split_line(target: &str) -> Result<(&str, Option<usize>), String> {
    let Some((path, line)) = target.rsplit_once(':') else {
        return Ok((target, None));
    };
    if line.is_empty() || !line.bytes().all(|byte| byte.is_ascii_digit()) {
        return Ok((target, None));
    }
    match line.parse::<usize>() {
        Ok(line) if line >= 1 => Ok((path, Some(line))),
        _ => Err(format!("{target}: lines count from 1")),
    }
}

/// What a statement shows when it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Shows {
    /// The script's value: its last query.
    Value,
    /// Another query's rows: its ordinal among the script's queries, from 1.
    Query(usize),
    /// The table that the statement makes, by its path as the statement writes it.
    Table(String),
}

/// The statement at a line: what it shows, the line of its first token, from 1, and the end of
/// its last token in the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Statement {
    pub(crate) shows: Shows,
    pub(crate) line: usize,
    pub(crate) end: usize,
}

impl Statement {
    /// The text that runs the statement in a chart script whose text is `text`: the whole text
    /// for its value, the text through the statement for another query, which becomes the
    /// script's last, and the text through the statement and a query of its table for a table.
    pub(crate) fn text(&self, text: &str) -> String {
        match &self.shows {
            Shows::Value => text.to_string(),
            Shows::Query(_) => text[..self.end].to_string(),
            Shows::Table(path) => format!("{};\nFROM {path}\n", &text[..self.end]),
        }
    }
}

/// The line, from 1, of the offset `offset` of `text`.
fn line_of(text: &str, offset: usize) -> usize {
    text[..offset].matches('\n').count() + 1
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

/// The statement of the chart script `text` at its line `line`, from 1: the top-level statement
/// whose first token and last token are on the line or around it, the first of two that start on
/// it. The errors name lines of the file, which has `before` lines before the text, as a cell's
/// document does.
pub(crate) fn at_line(text: &str, line: usize, before: usize) -> Result<Statement, String> {
    let place = |line: usize| line + before;
    let root = avenger_sql_syntax::parse(text).syntax();
    let Some(script) = ast::Script::cast(root) else {
        return Err("the text is no script".to_string());
    };
    let mut statements = Vec::new();
    for stmt in script.stmts() {
        if let Some((start, end)) = token_range(stmt.syntax()) {
            statements.push((stmt, start, end, line_of(text, start), line_of(text, end)));
        }
    }
    let Some(index) =
        (statements.iter()).position(|(_, _, _, first, last)| *first <= line && line <= *last)
    else {
        let before = (statements.iter()).rev().find(|(.., last)| *last < line);
        let after = (statements.iter()).find(|(_, _, _, first, _)| *first > line);
        let around = match (before, after) {
            (Some(before), Some(after)) => format!(
                "the statements around it start at lines {} and {}",
                place(before.3),
                place(after.3)
            ),
            (Some(before), None) => {
                format!("the last statement starts at line {}", place(before.3))
            }
            (None, Some(after)) => {
                format!("the first statement starts at line {}", place(after.3))
            }
            (None, None) => "the text has no statements".to_string(),
        };
        return Err(format!("no statement holds the line; {around}"));
    };
    let (stmt, _, end, first, _) = &statements[index];
    let queries: Vec<usize> = (statements.iter().enumerate())
        .filter(|(_, (stmt, ..))| matches!(stmt, ast::Stmt::Query(_)))
        .map(|(index, _)| index)
        .collect();
    let path = |path: Option<ast::Path>| match path {
        Some(path) => Ok(Shows::Table(path.syntax().text().to_string())),
        None => Err(format!(
            "the statement at line {} names no table",
            place(*first)
        )),
    };
    let shows = match stmt {
        ast::Stmt::Query(_) if queries.last() == Some(&index) => Shows::Value,
        ast::Stmt::Query(_) => {
            let ordinal = queries.iter().position(|&query| query == index);
            Shows::Query(ordinal.expect("a query of the script") + 1)
        }
        ast::Stmt::CreateTableStmt(create) => path(create.path())?,
        ast::Stmt::CreateExternalTableStmt(create) => path(create.path())?,
        ast::Stmt::CreateViewStmt(create) => path(create.path())?,
        ast::Stmt::LoadDataStmt(load) => path(load.path())?,
        _ => {
            return Err(format!(
                "the statement at line {} has no rows",
                place(*first)
            ))
        }
    };
    Ok(Statement {
        shows,
        line: *first,
        end: *end,
    })
}

/// The cell of `document` that holds its line `line`, from 1, with its index. The error of a line
/// outside every cell names the cells and their lines.
pub(crate) fn cell_at_line(document: &Qmd, line: usize) -> Result<(usize, &Cell), String> {
    let start = (document.text.split_inclusive('\n'))
        .take(line - 1)
        .map(str::len)
        .sum::<usize>();
    let inside =
        (document.cells.iter().enumerate()).find(|(_, cell)| cell.to_cell(start).is_some());
    if let Some(found) = inside {
        return Ok(found);
    }
    let cells: Vec<String> = (document.cells.iter().enumerate())
        .map(|(index, cell)| {
            let first = line_of(&document.text, cell.range.start);
            let last = line_of(&document.text, cell.range.end.max(cell.range.start + 1) - 1);
            format!("#{} at lines {first}-{last}", index + 1)
        })
        .collect();
    let cells = match cells.is_empty() {
        true => "the document has no `{avenger-sql}` cells".to_string(),
        false => format!("its cells are {}", cells.join(", ")),
    };
    Err(format!("the line is in no `{{avenger-sql}}` cell; {cells}"))
}
