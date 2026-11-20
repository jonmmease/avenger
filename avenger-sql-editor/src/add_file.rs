//! Add file: the `CREATE EXTERNAL TABLE` statement of a data file, which the editor writes into a
//! data script of the open project, or the `CREATE TEMP EXTERNAL TABLE` statement that it writes
//! into the active chart script, as a table of the chart alone. The format of a file comes from
//! its extension, its table name from the rest of its name, and the statement goes at the end of
//! a data script, or before a chart's query, in the style of avenger-sql's formatter. [`Workspace::adding`](crate::workspace::Workspace) and
//! `Adding::copy` copy a file from outside the project's root into [`FILES`], and
//! `Workspace::add` writes the statements.

use std::collections::BTreeSet;

use avenger_sql::data::{file_option, Compression, Format};
use avenger_sql_syntax::{ast, ast::AstNode, Role, SyntaxKind};

/// The directory of the project's data files, by convention, which a file from outside the
/// project's root is copied into.
pub const FILES: &str = "data/files";

/// The data script that takes the statements of a project without one.
pub const NEW_SCRIPT: &str = "data/tables.sql";

/// A data file by its name: the name before its extensions, its extensions, and the format and
/// the compression that they give.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DataFile<'a> {
    /// The name before the extensions, as `sales` of `sales.csv.gz`.
    pub stem: &'a str,
    /// The extensions without their first `.`, as the name writes them, as `csv.gz`.
    pub extension: &'a str,
    pub format: Format,
    pub compression: Option<Compression>,
}

impl<'a> DataFile<'a> {
    /// The data file named `name`, by its extensions without regard to case: the extension of a
    /// format of [`Format`], such as `.csv`, and after it, for a format that the option
    /// `compression` takes, the extension of a compression, such as `.gz`. The error is a
    /// message for the user: a name without such extensions, a format or a compression that
    /// this build does not read, or a name with `*`, which a URI reads as a pattern.
    pub fn of(name: &'a str) -> Result<DataFile<'a>, String> {
        if name.contains('*') {
            return Err(format!(
                "{name} has `*` in its name, which a URI reads as a pattern of names"
            ));
        }
        let lower = name.to_ascii_lowercase();
        let mut rest = lower.as_str();
        let compression = (Compression::ALL.into_iter())
            .find(|compression| rest.ends_with(&format!(".{}", compression.extension())));
        if let Some(compression) = compression {
            rest = &rest[..rest.len() - compression.extension().len() - 1];
        }
        let format = (Format::ALL.into_iter())
            .find(|format| rest.ends_with(&format!(".{}", format.extension())))
            .ok_or_else(|| unknown(name))?;
        let compressed =
            file_option("compression").is_some_and(|option| option.formats.contains(&format));
        if compression.is_some() && !compressed {
            return Err(unknown(name));
        }
        if !format.available() || compression.is_some_and(|compression| !compression.available()) {
            return Err(format!("This build reads no files such as {name}"));
        }
        let stem = rest.len() - format.extension().len() - 1;
        Ok(DataFile {
            stem: &name[..stem],
            extension: &name[stem + 1..],
            format,
            compression,
        })
    }

    /// The name of a copy of the file in a directory where `taken` tells the names of files:
    /// its own name, or else the stem with `-2`, `-3`, and so on, and the extensions, the first
    /// that `taken` does not hold.
    pub fn free_name(&self, taken: impl Fn(&str) -> bool) -> String {
        let name = format!("{}.{}", self.stem, self.extension);
        if !taken(&name) {
            return name;
        }
        (2..)
            .map(|n| format!("{}-{n}.{}", self.stem, self.extension))
            .find(|name| !taken(name))
            .expect("a free name")
    }
}

/// The error of a file without the extensions of a format that a data script reads.
fn unknown(name: &str) -> String {
    let formats: Vec<String> = (Format::ALL.into_iter())
        .filter(|format| format.available())
        .map(|format| format!(".{}", format.extension()))
        .collect();
    let compressed: Vec<String> = (file_option("compression").into_iter())
        .flat_map(|option| option.formats)
        .map(|format| format!(".{}", format.extension()))
        .collect();
    let compressions: Vec<String> = (Compression::ALL.into_iter())
        .filter(|compression| compression.available())
        .map(|compression| format!(".{}", compression.extension()))
        .collect();
    let mut message = format!(
        "{name} has no format that a data script reads: the extensions are {}",
        list(&formats, "and")
    );
    if !compressions.is_empty() {
        message.push_str(&format!(
            ", with {} after {}",
            list(&compressions, "or"),
            list(&compressed, "or")
        ));
    }
    message
}

/// `items` as a list in a sentence, with `and` or `or` before the last.
fn list(items: &[String], last: &str) -> String {
    match items {
        [] => String::new(),
        [item] => item.clone(),
        [first, second] => format!("{first} {last} {second}"),
        [rest @ .., end] => format!("{}, {last} {end}", rest.join(", ")),
    }
}

/// The table name of a file whose name before its extensions is `stem`: the stem in lowercase,
/// with `_` for each character that is not an ASCII letter, an ASCII digit, or `_`, `_` before a
/// leading digit, and `_` after a reserved keyword, so that the name needs no backticks. When
/// `taken`, the names of the tables of the project in lowercase, holds the name, it ends with
/// `_2`, `_3`, and so on, the first that `taken` does not hold.
pub fn table_name(stem: &str, taken: &BTreeSet<String>) -> String {
    let mut base: String = (stem.chars())
        .map(|c| match c.is_ascii_alphanumeric() {
            true => c.to_ascii_lowercase(),
            false => '_',
        })
        .collect();
    if base.is_empty() {
        base = "table".to_string();
    }
    if base.starts_with(|c: char| c.is_ascii_digit()) {
        base.insert(0, '_');
    }
    if avenger_sql_syntax::is_reserved(&base) {
        base.push('_');
    }
    if !taken.contains(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}_{n}"))
        .find(|name| !taken.contains(name))
        .expect("a free name")
}

/// The statement of a script of `role` that reads the file `file` at `uri`, relative to the
/// project's root, as the table `name`, in the style of avenger-sql's formatter: `CREATE
/// EXTERNAL TABLE` in a data script, and `CREATE TEMP EXTERNAL TABLE` in a chart script, whose
/// tables of files are its own, with the options `format`, `uris`, and `compression` for a
/// compressed file.
pub fn statement(name: &str, file: &DataFile<'_>, uri: &str, role: Role) -> String {
    let compression = (file.compression)
        .map(|compression| format!(", compression = '{}'", compression.name()))
        .unwrap_or_default();
    let temp = match role {
        Role::Script => "TEMP ",
        Role::Data | Role::Module => "",
    };
    let text = format!(
        "CREATE {temp}EXTERNAL TABLE {name} OPTIONS (format = '{}', uris = [{}]{compression});",
        file.format.name(),
        string_literal(uri)
    );
    avenger_sql_syntax::format::format(&text, role)
}

/// `text` as a string literal in single quotes, with `\` before a backslash or a quote.
fn string_literal(text: &str) -> String {
    format!("'{}'", text.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// The script that takes the statements: the active tab's file when it is a data script or a
/// chart script of `chart_scripts`, else the first of `data_scripts` by path, else
/// [`NEW_SCRIPT`].
pub fn script<'a>(
    active: Option<&'a str>,
    data_scripts: &'a [String],
    chart_scripts: &[String],
) -> &'a str {
    let script = |path: &str| (data_scripts.iter().chain(chart_scripts)).any(|p| p == path);
    match active {
        Some(active) if script(active) => active,
        _ => (data_scripts.iter().min()).map_or(NEW_SCRIPT, String::as_str),
    }
}

/// `text`, a chart script, with `statements` before its query, after a blank line: after its
/// last statement of files, `CREATE TEMP EXTERNAL TABLE` or `LOAD DATA`, and the comment at the
/// end of its line, or without one, before its first statement after its `DECLARE`s, after the
/// comments before that statement. A chart without such a statement takes them at its end.
pub fn insert(text: &str, statements: &[String]) -> String {
    let joined = statements.join("\n\n");
    let root = avenger_sql_syntax::parse(text).syntax();
    let Some(script) = ast::Script::cast(root) else {
        return append(text, statements);
    };
    let stmts: Vec<ast::Stmt> = script.stmts().collect();
    let files = (stmts.iter()).rposition(|stmt| {
        matches!(
            stmt,
            ast::Stmt::CreateExternalTableStmt(_) | ast::Stmt::LoadDataStmt(_)
        )
    });
    if let Some(index) = files {
        // After the `;` that ends the statement.
        let node = stmts[index].syntax();
        let mut end = node.text_range().end();
        let mut token = node.last_token();
        while let Some(next) = token.and_then(|token| token.next_token()) {
            if next.kind() == SyntaxKind::SEMICOLON {
                end = next.text_range().end();
                break;
            }
            if !next.kind().is_trivia() {
                break;
            }
            token = Some(next);
        }
        // And after the comment at the end of its line, which belongs to it.
        let end = usize::from(end);
        let line = text[end..].find('\n').map_or(text.len(), |at| end + at);
        let rest = text[end..line].trim_start();
        let end = match rest.is_empty() || rest.starts_with("--") {
            true => line,
            false => end,
        };
        return format!("{}\n\n{joined}{}", &text[..end], &text[end..]);
    }
    let first = (stmts.iter())
        .find(|stmt| !matches!(stmt, ast::Stmt::DeclareStmt(_)))
        .and_then(|stmt| {
            (stmt.syntax().descendants_with_tokens())
                .filter_map(|element| element.into_token())
                .find(|token| !token.kind().is_trivia())
        });
    match first {
        Some(token) => {
            let start = usize::from(token.text_range().start());
            format!("{}{joined}\n\n{}", &text[..start], &text[start..])
        }
        None => append(text, statements),
    }
}

/// The names of the tables that the chart script `text` defines, in lowercase: its temporary
/// tables and its own tables of files, whose names a table that Add file writes into the chart
/// does not take.
pub fn chart_tables(text: &str) -> BTreeSet<String> {
    let root = avenger_sql_syntax::parse(text).syntax();
    let Some(script) = ast::Script::cast(root) else {
        return BTreeSet::new();
    };
    (script.stmts())
        .filter_map(|stmt| match stmt {
            ast::Stmt::CreateTableStmt(create) => create.path(),
            ast::Stmt::CreateExternalTableStmt(create) => create.path(),
            ast::Stmt::LoadDataStmt(load) => load.path(),
            _ => None,
        })
        .filter_map(|path| match path.names()?.as_slice() {
            [name] => Some(name.to_ascii_lowercase()),
            _ => None,
        })
        .collect()
}

/// `text` with `statements` at its end, after a blank line, and a line break after the last.
pub fn append(text: &str, statements: &[String]) -> String {
    let statements = statements.join("\n\n");
    match text.trim_end() {
        "" => format!("{statements}\n"),
        text => format!("{text}\n\n{statements}\n"),
    }
}
