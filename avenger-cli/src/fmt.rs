//! `avenger fmt`: the selected scripts and modules in the style of `avenger-sql/STYLE.md`, by the
//! roles that their projects give them. A statement with a syntax error stays as written.

use std::path::{Path, PathBuf};

use avenger_sql_project::{Role, MODULE_SUFFIX};
use avenger_sql_syntax::{format::format, Role as TextRole};

use crate::{files::select, report::Output, Status};

/// Formats the scripts and the modules of `paths` in place, or with `check`, changes nothing
/// and fails when one needs formatting.
pub async fn run(paths: &[PathBuf], check: bool, output: &Output) -> Result<Status, String> {
    let selection = select(paths).await?;
    let mut files: Vec<(PathBuf, TextRole)> = Vec::new();
    for project in &selection.projects {
        for (path, role) in &project.paths {
            let role = match role {
                Role::DataScript => TextRole::Data,
                Role::ChartScript => TextRole::Script,
                Role::Module(_) => TextRole::Module,
                Role::Stylesheet => continue,
            };
            files.push((project.root.join(path), role));
        }
    }
    for path in &selection.loose {
        let role = match path.to_string_lossy().ends_with(MODULE_SUFFIX) {
            true => TextRole::Module,
            false => TextRole::Script,
        };
        files.push((path.clone(), role));
    }
    let mut changed = Vec::new();
    for (path, role) in files {
        if format_file(&path, role, check)? {
            changed.push(path);
        }
    }
    output.formatted(&changed, check);
    Ok(match check && !changed.is_empty() {
        true => Status::Failed,
        false => Status::Ok,
    })
}

/// Formats the file at `path`, a text of `role`, and whether its text changed, or would change
/// with `check`, which writes nothing.
fn format_file(path: &Path, role: TextRole, check: bool) -> Result<bool, String> {
    let error = |e: std::io::Error| format!("{}: {e}", path.display());
    let text = std::fs::read_to_string(path).map_err(error)?;
    let formatted = format(&text, role);
    if formatted == text {
        return Ok(false);
    }
    if !check {
        std::fs::write(path, formatted).map_err(error)?;
    }
    Ok(true)
}
