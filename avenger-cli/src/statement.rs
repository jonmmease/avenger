//! A target at a line, `FILE:LINE`, and the statement that it names: the top-level statement of
//! a script, or of a cell, that holds the line, and the text that runs it, so a chart script's
//! earlier query shows its rows, a statement that makes a table shows the table, and a data
//! script's statement shows its table as the project's session holds it.

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
    /// The table that a chart script's statement makes, by its path as the statement writes it.
    Table(String),
    /// The table that a data script's statement makes, as the project's session holds it after
    /// the script loads, by its path as the statement writes it.
    Loaded(String),
}

/// The statement at a line: what it shows, its key, the line of its first token, from 1, and the
/// range of its tokens in the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Statement {
    pub(crate) shows: Shows,
    pub(crate) key: Key,
    pub(crate) line: usize,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

/// What finds a statement again after edits around it: the script's value, its last query, so it
/// stays the value when queries come before it, another query by its ordinal among the queries,
/// and a statement that makes a table by the table's name, as a name that compares, and its
/// occurrence among the statements that make it, from 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Key {
    Value,
    Query(usize),
    Table { name: String, occurrence: usize },
}

impl Shows {
    /// What the statement shows, as a window's title names it: the result, a query by its
    /// ordinal, or a table by its path.
    pub(crate) fn describe(&self) -> String {
        match self {
            Shows::Value => "result".to_string(),
            Shows::Query(ordinal) => format!("query {ordinal}"),
            Shows::Table(path) | Shows::Loaded(path) => format!("table {path}"),
        }
    }
}

impl Statement {
    /// The text of a chart script that runs the statement of the script `text`: the whole text
    /// for its value, the text through the statement for another query, which becomes the
    /// script's last, the text through the statement and a query of its table for a chart's
    /// table, and a query of a data script's table in the project's session.
    pub(crate) fn text(&self, text: &str) -> String {
        match &self.shows {
            Shows::Value => text.to_string(),
            Shows::Query(_) => text[..self.end].to_string(),
            Shows::Table(path) => format!("{};\nFROM {path}\n", &text[..self.end]),
            Shows::Loaded(path) => format!("FROM {path}\n"),
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

/// The kind of a script, which parses with its own grammar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Script {
    Chart,
    Data,
}

/// A top-level statement of a script, with the range of its tokens and the lines of its first
/// token and its last.
struct Located {
    stmt: ast::Stmt,
    start: usize,
    end: usize,
    first: usize,
    last: usize,
}

/// The top-level statements of the script `text`, of the kind `script`, in order.
fn statements(text: &str, script: Script) -> Vec<Located> {
    let parse = match script {
        Script::Chart => avenger_sql_syntax::parse(text),
        Script::Data => avenger_sql_syntax::parse_data(text),
    };
    let Some(root) = ast::Script::cast(parse.syntax()) else {
        return Vec::new();
    };
    (root.stmts())
        .filter_map(|stmt| {
            let (start, end) = token_range(stmt.syntax())?;
            Some(Located {
                stmt,
                start,
                end,
                first: line_of(text, start),
                last: line_of(text, end),
            })
        })
        .collect()
}

/// The table that a statement makes: its path as the statement writes it, the path as a name
/// that compares, without quotes and in lowercase, and whether the table is temporary.
fn table(stmt: &ast::Stmt) -> Option<(String, String, bool)> {
    let (path, scope) = match stmt {
        ast::Stmt::CreateTableStmt(create) => (create.path(), create.scope()),
        ast::Stmt::CreateExternalTableStmt(create) => (create.path(), create.scope()),
        ast::Stmt::CreateViewStmt(create) => (create.path(), create.scope()),
        ast::Stmt::LoadDataStmt(load) => (load.path(), load.scope()),
        _ => return None,
    };
    let written = path?.syntax().text().to_string();
    let name: String = (written.chars())
        .filter(|c| *c != '`' && !c.is_whitespace())
        .collect::<String>()
        .to_lowercase();
    let temporary = scope.is_some_and(|scope| {
        matches!(
            scope.kind(),
            avenger_sql_syntax::SyntaxKind::TEMP_KW | avenger_sql_syntax::SyntaxKind::TEMPORARY_KW
        )
    });
    Some((written, name, temporary))
}

/// The tables of the data script `text` that a target can show, each by its path as its last
/// statement writes it and that statement's line, in order: a table that another script reads,
/// not a temporary one.
pub(crate) fn data_tables(text: &str) -> Vec<(String, usize)> {
    let statements = statements(text, Script::Data);
    let mut tables: Vec<(String, String, usize)> = Vec::new();
    for located in &statements {
        if let Some((written, name, false)) = table(&located.stmt) {
            tables.retain(|(_, other, _)| *other != name);
            tables.push((written, name, located.first));
        }
    }
    tables.sort_by_key(|(_, _, line)| *line);
    (tables.into_iter())
        .map(|(written, _, line)| (written, line))
        .collect()
}

/// The statement of the script `text`, of the kind `script`, at its line `line`, from 1: the
/// top-level statement whose first token and last token are on the line or around it, the first
/// of two that start on it. The errors name lines of the file, which has `before` lines before
/// the text, as a cell's document does.
///
/// A data script's statement shows its table as the project's session holds it, so a statement
/// of a temporary table, which no other script reads, and a statement of a table that a later
/// statement loads into again, show nothing, and are errors.
pub(crate) fn at_line(
    text: &str,
    line: usize,
    before: usize,
    script: Script,
) -> Result<Statement, String> {
    let place = |line: usize| line + before;
    let statements = statements(text, script);
    let Some(index) = (statements.iter()).position(|at| at.first <= line && line <= at.last) else {
        let before = (statements.iter()).rev().find(|at| at.last < line);
        let after = (statements.iter()).find(|at| at.first > line);
        let around = match (before, after) {
            (Some(before), Some(after)) => format!(
                "the statements around it start at lines {} and {}",
                place(before.first),
                place(after.first)
            ),
            (Some(before), None) => {
                format!("the last statement starts at line {}", place(before.first))
            }
            (None, Some(after)) => {
                format!("the first statement starts at line {}", place(after.first))
            }
            (None, None) => "the text has no statements".to_string(),
        };
        return Err(format!("no statement holds the line; {around}"));
    };
    describe(&statements, index, before, script)
}

/// The statement of the script `text`, of the kind `script`, that `key` finds after edits around
/// it, with the key. The errors name the file `file`, as "`sales.av.sql` has no table `sales` any
/// more" for a key that finds none.
pub(crate) fn by_key(
    text: &str,
    key: &Key,
    before: usize,
    script: Script,
    file: &str,
) -> Result<Statement, String> {
    let statements = statements(text, script);
    let queries: Vec<usize> = (statements.iter().enumerate())
        .filter(|(_, at)| matches!(at.stmt, ast::Stmt::Query(_)))
        .map(|(index, _)| index)
        .collect();
    let index = match key {
        Key::Value => {
            (queries.last().copied()).ok_or_else(|| format!("`{file}` has no query any more"))?
        }
        Key::Query(ordinal) => (queries.get(ordinal - 1).copied())
            .ok_or_else(|| format!("`{file}` has no query {ordinal} any more"))?,
        Key::Table { name, occurrence } => (statements.iter().enumerate())
            .filter(|(_, at)| table(&at.stmt).is_some_and(|(_, other, _)| other == *name))
            .nth(occurrence - 1)
            .map(|(index, _)| index)
            .ok_or_else(|| format!("`{file}` has no table `{name}` any more"))?,
    };
    let statement =
        describe(&statements, index, before, script).map_err(|error| format!("{file}: {error}"))?;
    Ok(Statement {
        key: key.clone(),
        ..statement
    })
}

/// The statement at `index` of `statements`, of a script of the kind `script` whose file has
/// `before` lines before it: what it shows and its key.
fn describe(
    statements: &[Located],
    index: usize,
    before: usize,
    script: Script,
) -> Result<Statement, String> {
    let place = |line: usize| line + before;
    let at = &statements[index];
    let queries: Vec<usize> = (statements.iter().enumerate())
        .filter(|(_, at)| matches!(at.stmt, ast::Stmt::Query(_)))
        .map(|(index, _)| index)
        .collect();
    let no_rows = || format!("the statement at line {} has no rows", place(at.first));
    let (shows, key) = match (&at.stmt, script) {
        (ast::Stmt::Query(_), _) if queries.last() == Some(&index) => (Shows::Value, Key::Value),
        (ast::Stmt::Query(_), _) => {
            let ordinal = queries.iter().position(|&query| query == index);
            let ordinal = ordinal.expect("a query of the script") + 1;
            (Shows::Query(ordinal), Key::Query(ordinal))
        }
        (stmt, script) => {
            let (written, name, temporary) = table(stmt).ok_or_else(no_rows)?;
            let occurrence = (statements[..index].iter())
                .filter(|earlier| table(&earlier.stmt).is_some_and(|(_, other, _)| other == name))
                .count()
                + 1;
            let shows = match script {
                Script::Chart => Shows::Table(written),
                Script::Data => {
                    if temporary {
                        return Err(format!(
                            "the statement at line {} makes the temporary table `{written}`, \
                             which only its script reads",
                            place(at.first)
                        ));
                    }
                    let later = (statements[index + 1..].iter()).find(|later| {
                        table(&later.stmt).is_some_and(|(_, other, _)| other == name)
                    });
                    if let Some(later) = later {
                        return Err(format!(
                            "the statement at line {} makes `{written}`, which the statement at \
                             line {} loads again; the table shows at line {}",
                            place(at.first),
                            place(later.first),
                            place(later.first)
                        ));
                    }
                    Shows::Loaded(written)
                }
            };
            (shows, Key::Table { name, occurrence })
        }
    };
    Ok(Statement {
        shows,
        key,
        line: at.first,
        start: at.start,
        end: at.end,
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
