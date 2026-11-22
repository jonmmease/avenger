//! `avenger fmt`: the selected scripts and modules in the style of `avenger-sql/STYLE.md`, by the
//! roles that their projects give them, and the `{avenger-sql}` cells of the selected Quarto
//! documents, as chart scripts, with the rest of each document as it is. A statement with a
//! syntax error stays as written.

use std::path::{Path, PathBuf};

use avenger_sql_project::{
    qmd::{is_document, Qmd},
    Role, MODULE_SUFFIX,
};
use avenger_sql_syntax::{format::format, Role as TextRole};

use crate::{files::select, report::Output, Status};

/// How a file formats: as a text of a role, or as a document of cells.
#[derive(Clone, Copy)]
enum Kind {
    Text(TextRole),
    Document,
}

/// Formats the scripts, the modules, and the cells of the documents of `paths` in place, or with
/// `check`, changes nothing and fails when one needs formatting.
pub async fn run(paths: &[PathBuf], check: bool, output: &Output) -> Result<Status, String> {
    let selection = select(paths).await?;
    let mut files: Vec<(PathBuf, Kind)> = Vec::new();
    for project in &selection.projects {
        for (path, role) in &project.paths {
            let kind = match role {
                Role::DataScript => Kind::Text(TextRole::Data),
                Role::ChartScript => Kind::Text(TextRole::Script),
                Role::Module(_) => Kind::Text(TextRole::Module),
                Role::Document => Kind::Document,
                Role::Stylesheet => continue,
            };
            files.push((project.root.join(path), kind));
        }
    }
    for path in &selection.loose {
        let name = path.to_string_lossy();
        let kind = if is_document(&name) {
            Kind::Document
        } else if name.ends_with(MODULE_SUFFIX) {
            Kind::Text(TextRole::Module)
        } else {
            Kind::Text(TextRole::Script)
        };
        files.push((path.clone(), kind));
    }
    let mut changed = Vec::new();
    for (path, kind) in files {
        if format_file(&path, kind, check)? {
            changed.push(path);
        }
    }
    output.formatted(&changed, check);
    Ok(match check && !changed.is_empty() {
        true => Status::Failed,
        false => Status::Ok,
    })
}

/// Formats the file at `path`, and whether its text changed, or would change with `check`, which
/// writes nothing.
fn format_file(path: &Path, kind: Kind, check: bool) -> Result<bool, String> {
    let error = |e: std::io::Error| format!("{}: {e}", path.display());
    let text = std::fs::read_to_string(path).map_err(error)?;
    let formatted = match kind {
        Kind::Text(role) => format(&text, role),
        Kind::Document => Qmd::parse(text.as_str()).format(|cell| format(cell, TextRole::Script)),
    };
    if formatted == text {
        return Ok(false);
    }
    if !check {
        std::fs::write(path, formatted).map_err(error)?;
    }
    Ok(true)
}
