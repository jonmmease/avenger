//! The project that a command opens: its files, its analysis, and its loader, with the data
//! scripts loaded, and what the commands of a project's data read from it: its tables, its
//! chart scripts and cells, and the errors of the data scripts that did not load.

use std::path::{Path, PathBuf};

use avenger_sql::library::search::closest_name;
use avenger_sql_project::{
    find_root, qmd::cell_path, Analysis, LoadState, Loader, ProjectFiles, Status, Table,
};
use datafusion::prelude::SessionContext;

use crate::{
    check::{check_text, same, File},
    report::{Report, Severity},
};

/// A project, loaded.
pub(crate) struct Opened {
    /// The root of the project on disk.
    pub root: PathBuf,
    pub files: ProjectFiles,
    pub analysis: Analysis,
    pub loader: Loader,
    pub state: LoadState,
}

/// The analysis of `files` and their loader, with the data scripts loaded and the analysis in
/// step with the load.
pub(crate) async fn load_files(
    files: &ProjectFiles,
) -> Result<(Analysis, Loader, LoadState), String> {
    let mut analysis = Analysis::new(files).await.map_err(|e| e.to_string())?;
    let (loader, state) =
        (Loader::load(files.clone(), SessionContext::new()).await).map_err(|e| e.to_string())?;
    analysis.apply(&state);
    Ok((analysis, loader, state))
}

/// Opens the project of `path`, a file or a directory in it, or of the current directory
/// without one, and loads it.
pub(crate) async fn open(path: Option<&Path>) -> Result<Opened, String> {
    let path = match path {
        Some(path) => path.to_path_buf(),
        None => std::env::current_dir().map_err(|error| error.to_string())?,
    };
    let canonical =
        std::fs::canonicalize(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let root = find_root(&canonical)
        .ok_or_else(|| format!("{}: not in an avenger project", path.display()))?;
    let files =
        (ProjectFiles::find(&root).await).map_err(|e| format!("{}: {e}", root.display()))?;
    let (analysis, loader, state) = load_files(&files)
        .await
        .map_err(|e| format!("{}: {e}", root.display()))?;
    Ok(Opened {
        root,
        files,
        analysis,
        loader,
        state,
    })
}

impl Opened {
    /// The tables and the views that the data scripts registered, each with the path of its
    /// script, in the order of the load and of each script's text.
    pub fn tables(&self) -> impl Iterator<Item = (&str, &Table)> {
        (self.state.scripts.iter())
            .flat_map(|script| (script.tables.iter()).map(|table| (script.path.as_str(), table)))
    }

    /// The table at the path `name`, as a data script writes it or by its name alone in the
    /// default schema, or the error that suggests the table that it means.
    pub async fn table(&self, name: &str) -> Result<(&str, &Table), String> {
        let default = self.loader.default_schema().await;
        let path: Vec<&str> = name.split('.').collect();
        let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
        let found = self.tables().find(|(_, table)| {
            let names: Vec<&str> = table.names.iter().map(String::as_str).collect();
            match (names.as_slice(), path.as_slice()) {
                ([schema, a], [b]) => same(schema, &default) && same(a, b),
                ([a], [schema, b]) => same(schema, &default) && same(a, b),
                (names, path) => {
                    names.len() == path.len() && names.iter().zip(path).all(|(a, b)| same(a, b))
                }
            }
        });
        found.ok_or_else(|| {
            let names: Vec<String> = self.tables().map(|(_, t)| t.names.join(".")).collect();
            if names.is_empty() {
                return format!("no table `{name}`: the project's data scripts register none");
            }
            // A table of another schema by its name alone, or a name a few edits away.
            let last = path.last().copied().unwrap_or_default();
            let suggestion = (self.tables())
                .find(|(_, table)| table.names.last().is_some_and(|n| same(n, last)))
                .map(|(_, table)| table.names.join("."))
                .or_else(|| {
                    let names = names.iter().map(String::as_str);
                    closest_name(name, names).map(str::to_string)
                });
            match suggestion {
                Some(suggestion) => format!("no table `{name}`. Did you mean {suggestion}?"),
                None => format!("no table `{name}`: `avenger tables` lists the tables"),
            }
        })
    }

    /// The errors of the data scripts that did not load, as `check` reports them.
    pub async fn load_errors(&self) -> Vec<Report> {
        let mut reports = Vec::new();
        for script in &self.state.scripts {
            if script.status == Status::Loaded {
                continue;
            }
            let path = self.root.join(&script.path);
            let file = File {
                path: &path,
                text: &script.text,
                place: &same,
                first_line: None,
            };
            let (analysis, loader) = (&self.analysis, &self.loader);
            let text = &script.text;
            let errors = (check_text(analysis, loader, &script.path, text, false, &file).await)
                .into_iter()
                .filter(|report| report.severity == Severity::Error);
            reports.extend(errors);
        }
        reports
    }
}

/// The chart scripts of `files` and the cells of the documents of `analysis`, as
/// [`Opened::targets`] gives them.
pub(crate) fn targets(files: &ProjectFiles, analysis: &Analysis) -> Vec<String> {
    let mut targets: Vec<String> = files.chart_scripts().to_vec();
    for (document, qmd) in analysis.documents() {
        targets.extend((1..=qmd.cells.len()).map(|number| cell_path(document, number)));
    }
    targets
}
