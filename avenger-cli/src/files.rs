//! The files that a command reads: the files of the projects of the paths that it takes, by
//! their roles, and the loose scripts, modules, and Quarto documents, which are in no project or
//! have no role in theirs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use avenger_sql_project::{
    find_root, qmd::is_document, ProjectFiles, Role, DOCUMENT_SUFFIX, MODULE_SUFFIX, SCRIPT_SUFFIX,
};

/// The files of the paths that a command takes.
pub struct Selection {
    pub projects: Vec<ProjectSelection>,
    /// The scripts, modules, and documents outside every project, or without a role in theirs,
    /// by their paths on disk.
    pub loose: Vec<PathBuf>,
}

/// The files of one project that a command reads.
pub struct ProjectSelection {
    /// The root of the project on disk.
    pub root: PathBuf,
    pub files: ProjectFiles,
    /// The selected files, by their paths relative to the root, with their roles, in the order
    /// of their paths.
    pub paths: Vec<(String, Role)>,
}

/// What a path selects in its project: every file under a directory, by the directory's path
/// relative to the root, or one file.
enum Target {
    Directory(String),
    File(String),
}

/// The files of `paths`, or of the current directory without one: for a path in a project, the
/// files of the project under it that have roles, and for another path, the scripts, modules,
/// and documents under it. A path that does not exist, and a file that is not
/// a script, a module, a document, or a stylesheet, are errors.
pub async fn select(paths: &[PathBuf]) -> Result<Selection, String> {
    let paths = match paths.is_empty() {
        true => vec![std::env::current_dir().map_err(|error| error.to_string())?],
        false => paths.to_vec(),
    };
    let mut targets: BTreeMap<PathBuf, Vec<Target>> = BTreeMap::new();
    let mut loose = Vec::new();
    for path in &paths {
        let canonical =
            std::fs::canonicalize(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let directory = canonical.is_dir();
        match find_root(&canonical) {
            Some(root) => {
                let relative = relative(&root, &canonical).unwrap_or_default();
                let target = match directory {
                    true => Target::Directory(relative),
                    false => Target::File(relative),
                };
                targets.entry(root).or_default().push(target);
            }
            None if directory => loose.extend(scripts_under(&canonical)),
            None => loose.push(canonical),
        }
    }
    let mut projects = Vec::new();
    for (root, targets) in targets {
        let files = (ProjectFiles::find(&root).await)
            .map_err(|error| format!("{}: {error}", root.display()))?;
        let mut selected: BTreeMap<String, Role> = BTreeMap::new();
        for target in targets {
            match target {
                Target::Directory(directory) => {
                    for path in files.files() {
                        let under = directory.is_empty()
                            || path
                                .strip_prefix(&directory)
                                .is_some_and(|rest| rest.starts_with('/'));
                        if let (true, Ok(Some(role))) = (under, files.manifest().role(path)) {
                            selected.insert(path.clone(), role);
                        }
                    }
                }
                Target::File(path) => match files.manifest().role(&path) {
                    Ok(Some(role)) => {
                        selected.insert(path, role);
                    }
                    _ if is_script_or_module(&path) || is_document(&path) => {
                        loose.push(root.join(&path))
                    }
                    _ => {
                        return Err(format!(
                            "{}: not a script, a module, a document, or a stylesheet of the \
                             project at {}",
                            root.join(&path).display(),
                            root.display()
                        ))
                    }
                },
            }
        }
        projects.push(ProjectSelection {
            root,
            files,
            paths: selected.into_iter().collect(),
        });
    }
    for path in &loose {
        let name = path.to_string_lossy();
        if !is_script_or_module(&name) && !is_document(&name) {
            return Err(format!(
                "{}: not an avenger-sql script ({SCRIPT_SUFFIX}), module ({MODULE_SUFFIX}), or \
                 Quarto document ({DOCUMENT_SUFFIX})",
                path.display()
            ));
        }
    }
    loose.sort();
    loose.dedup();
    Ok(Selection { projects, loose })
}

/// Whether the file at `path` is named as a script or a module.
pub fn is_script_or_module(path: &str) -> bool {
    path.ends_with(SCRIPT_SUFFIX) || path.ends_with(MODULE_SUFFIX)
}

/// The scripts, modules, and documents under the directory `directory`, outside hidden
/// directories.
fn scripts_under(directory: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(directory) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        if path.is_dir() {
            found.extend(scripts_under(&path));
        } else if is_script_or_module(&name.to_string_lossy())
            || is_document(&name.to_string_lossy())
        {
            found.push(path);
        }
    }
    found
}

/// The path `path` relative to `root`, with its names separated by `/`.
fn relative(root: &Path, path: &Path) -> Option<String> {
    let names: Option<Vec<&str>> = (path.strip_prefix(root).ok()?.components())
        .map(|component| component.as_os_str().to_str())
        .collect();
    Some(names?.join("/"))
}
