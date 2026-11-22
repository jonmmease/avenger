//! A project that stays loaded while its files change: the [`Loader`], which loads the data
//! scripts into a session and runs the chart scripts, and the [`Analysis`], the language project
//! that analyzes every script and module with the tables that the loader read, kept in step.
//!
//! The analysis also holds the project's Quarto documents, whose `{avenger-sql}` cells it
//! analyzes as chart scripts at the paths that [`qmd::cell_path`](crate::qmd::cell_path) gives.
//!
//! The futures of the loader are not `Send`, so a tool runs them on one thread, apart from the
//! analysis. Each load sends back a [`LoadState`], which [`Analysis::apply`] takes, so that the
//! analysis never waits for the loader.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use avenger_sql::{
    analysis::{Lifted, Options},
    catalog::{references, same_table, FilesTable, MemoryCatalog},
    data::{Files, Registration},
    library::Library,
};
use avenger_sql_language::{Diagnostic, FileRole, Project, Severity, Source};
use datafusion::{
    arrow::datatypes::DataType, common::DataFusionError, dataframe::DataFrame,
    prelude::SessionContext,
};
use tokio::sync::RwLock;

use crate::{
    load::{ChartError, FilesReload, LoadedProject, ScriptStatus, TableChange},
    project::{ProjectError, ProjectFiles},
    qmd::{cell_path, set_cells, Qmd},
};

/// The loaded project, which a tool's background work shares. A chart run reads it and a load
/// of data scripts writes it, so that a chart never reads a project that is partly loaded. A
/// load or a reload removes the tables of its scripts before it registers them again, so a tool
/// never stops one before its end.
pub struct Loader {
    project: RwLock<LoadedProject>,
}

/// What a load of the data scripts leaves: the result of each data script, and the table of each
/// set of files that they read, which the analysis reads.
#[derive(Clone, Debug)]
pub struct LoadState {
    /// The data scripts in the order of the load.
    pub scripts: Vec<ScriptState>,
    pub file_tables: Vec<(Files, FilesTable)>,
}

/// The result of the load of a data script.
#[derive(Clone, Debug)]
pub struct ScriptState {
    pub path: String,
    /// The text that loaded, which the ranges of the diagnostics are in.
    pub text: String,
    pub status: Status,
    /// The loader's errors in the text, with the source [`Source::Loader`].
    pub diagnostics: Vec<Diagnostic>,
    /// The tables and views that the script registered, in the order of its text.
    pub tables: Vec<Table>,
}

/// Whether a data script loaded, and why it did not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Loaded,
    /// The script has errors.
    Failed,
    /// The script reads tables of scripts that did not load.
    NotLoaded,
    /// The script is in a cycle of scripts, or reads one.
    Blocked,
}

/// A table of the session: its path and its columns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub names: Vec<String>,
    pub columns: Vec<(String, DataType)>,
}

impl LoadState {
    /// The state of the last load of each data script of `project`.
    pub fn of(project: &LoadedProject) -> Self {
        let scripts = (project.scripts())
            .map(|script| {
                let text = project.text(&script.path).unwrap_or_default().to_string();
                let status = match &script.status {
                    ScriptStatus::Loaded(_) => Status::Loaded,
                    ScriptStatus::Failed => Status::Failed,
                    ScriptStatus::NotLoaded { .. } => Status::NotLoaded,
                    ScriptStatus::Blocked => Status::Blocked,
                };
                let diagnostics = (script.diagnostics.iter())
                    .map(|diagnostic| Diagnostic {
                        range: diagnostic.range,
                        message: diagnostic.message.clone(),
                        help: diagnostic.help.clone(),
                        severity: Severity::Error,
                        source: Source::Loader,
                        code: None,
                        fixes: Vec::new(),
                    })
                    .collect();
                let tables = (script.loaded().into_iter())
                    .flat_map(|loaded| {
                        (loaded.registered.iter()).map(move |registered| (loaded, registered))
                    })
                    .filter_map(|(loaded, registered)| match &registered.registration {
                        Registration::Table(table) => Some(Table {
                            names: loaded.analysis.definitions[registered.definition]
                                .names
                                .clone(),
                            columns: (table.schema().fields().iter())
                                .map(|field| (field.name().clone(), field.data_type().clone()))
                                .collect(),
                        }),
                        Registration::Schema(_) => None,
                    })
                    .collect();
                ScriptState {
                    path: script.path.clone(),
                    text,
                    status,
                    diagnostics,
                    tables,
                }
            })
            .collect();
        Self {
            scripts,
            file_tables: project.file_tables(),
        }
    }
}

impl Loader {
    /// Loads the data scripts of `files` into the session `ctx`, as [`LoadedProject::load`] does.
    pub async fn load(
        files: ProjectFiles,
        ctx: SessionContext,
    ) -> Result<(Loader, LoadState), ProjectError> {
        let project = LoadedProject::load(files, ctx).await?;
        let state = LoadState::of(&project);
        let project = RwLock::new(project);
        Ok((Loader { project }, state))
    }

    /// Loads the data script or the module at `path` again with `text`, and the scripts
    /// downstream of it, and returns the tables that changed and the state of the load.
    pub async fn reload(
        &self,
        path: &str,
        text: String,
    ) -> Result<(Vec<TableChange>, LoadState), ProjectError> {
        let mut project = self.project.write().await;
        let changes = project.reload(path, text).await?;
        Ok((changes, LoadState::of(&project)))
    }

    /// Loads the data scripts that read the files or the directories at `paths`, relative to the
    /// project's root, again, after they changed on disk, and the scripts downstream of them, and
    /// returns what changed, the tables and the charts whose own tables of files read them,
    /// which run again, and the state of the load, or `None` when no data script and no chart
    /// reads them, so that nothing loads.
    pub async fn reload_files(&self, paths: &[String]) -> Option<(FilesReload, LoadState)> {
        let mut project = self.project.write().await;
        let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
        if (paths.iter()).all(|path| project.scripts_reading(path).is_empty()) {
            return None;
        }
        let reload = project.reload_files(&paths).await;
        Some((reload, LoadState::of(&project)))
    }

    /// Opens the project again from its file source and loads every data script again, as after
    /// a new data script appears.
    pub async fn reload_all(&self) -> Result<(Vec<TableChange>, LoadState), ProjectError> {
        let mut project = self.project.write().await;
        let changes = project.reload_all().await?;
        Ok((changes, LoadState::of(&project)))
    }

    /// The rows of the chart script at `path`, whose text is `text`, as
    /// [`LoadedProject::run_chart`] runs it.
    pub async fn run_chart(&self, path: &str, text: &str) -> Result<DataFrame, ChartError> {
        self.project.read().await.run_chart(path, text).await
    }

    /// The lifted form of the chart script at `path`, whose text is `text`, as
    /// [`LoadedProject::lift`] gives it.
    pub async fn lift(&self, path: &str, text: &str) -> Result<Lifted, DataFusionError> {
        self.project.read().await.lift(path, text).await
    }

    /// The table of each set of files that the data scripts and the charts' own tables of files
    /// read, in their last loads and runs, which the analysis reads.
    pub async fn file_tables(&self) -> Vec<(Files, FilesTable)> {
        self.project.read().await.file_tables()
    }

    /// The schema of the session that holds a table of a path of one name.
    pub async fn default_schema(&self) -> String {
        let project = self.project.read().await;
        let state = project.session().state();
        state.config_options().catalog.default_schema.clone()
    }
}

/// The language project of a loaded project, which analyzes every data script, chart script,
/// module, and cell of a document with the tables that the loader read, and the loader's errors
/// of each data script.
pub struct Analysis {
    project: Project,
    /// The documents, by path, with the texts whose cells the language project holds.
    documents: BTreeMap<String, Qmd>,
    /// The table of each set of files that the loader read, as the language project has them.
    file_tables: Vec<(Files, FilesTable)>,
}

impl Analysis {
    /// Analyzes the files of `files`, the data scripts, the chart scripts, the modules, and the
    /// cells of the documents, with their texts from its file source, before any load: a set of
    /// files has no table, so it has an unknown schema until [`Analysis::apply`] takes a
    /// [`LoadState`].
    pub async fn new(files: &ProjectFiles) -> Result<Analysis, ProjectError> {
        let source = files.source();
        let roles = (files.data_scripts().iter())
            .map(|path| (path, FileRole::Data))
            .chain((files.chart_scripts().iter()).map(|path| (path, FileRole::Chart)))
            .chain((files.modules().iter()).map(|module| {
                let role = FileRole::Module {
                    names: module.names.clone(),
                };
                (&module.path, role)
            }));
        let mut texts = Vec::new();
        for (path, role) in roles {
            texts.push((path.clone(), role, source.read_text(path).await?));
        }
        let mut documents = BTreeMap::new();
        for path in files.documents() {
            let document = Qmd::parse(source.read_text(path).await?);
            for (index, cell) in document.cells.iter().enumerate() {
                texts.push((
                    cell_path(path, index + 1),
                    FileRole::Chart,
                    cell.text.clone(),
                ));
            }
            documents.insert(path.clone(), document);
        }
        let project = Project::new(
            Arc::new(MemoryCatalog::new()),
            Arc::new(Library::new()),
            Options::default(),
        )
        .with_paths(files.files().into())
        .with_files(texts);
        Ok(Analysis {
            project,
            documents,
            file_tables: Vec::new(),
        })
    }

    /// The document at `path`, with the text that the analysis has of it.
    pub fn qmd(&self, path: &str) -> Option<&Qmd> {
        self.documents.get(path)
    }

    /// The documents, by path, in the order of their paths.
    pub fn documents(&self) -> impl Iterator<Item = (&str, &Qmd)> {
        (self.documents.iter()).map(|(path, document)| (path.as_str(), document))
    }

    /// Sets the text of the document at `path`, as an editor or a change on disk gives it, which
    /// adds the document when the analysis lacks it, or removes the document without a text.
    /// The result is the files that the change reaches, as [`Project::set_texts`] gives them,
    /// with the document's own path, since its cells move when its other lines change.
    pub fn set_document(&mut self, path: &str, text: Option<String>) -> BTreeSet<String> {
        let old = self.documents.remove(path).unwrap_or_default();
        let new = text.map(Qmd::parse);
        let cells = new.as_ref().map_or(&[][..], |new| &new.cells[..]);
        let mut reached = set_cells(&mut self.project, path, &old.cells, cells);
        reached.insert(path.to_string());
        if let Some(new) = new {
            self.documents.insert(path.to_string(), new);
        }
        reached
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    /// The language project, which a tool edits as its files change.
    pub fn project_mut(&mut self) -> &mut Project {
        &mut self.project
    }

    /// Takes the state of a load: the tables of the files, and the loader's errors of each data
    /// script whose text in the analysis is the text that loaded, since an edit since the load
    /// moves the ranges of its errors. The result is the files whose diagnostics can change: those
    /// that the tables of the files analyze again, and the data scripts of the load.
    pub fn apply(&mut self, state: &LoadState) -> BTreeSet<String> {
        let mut reached = self.set_file_tables(state.file_tables.clone());
        for script in &state.scripts {
            reached.insert(script.path.clone());
            let current = (self.project.document(&script.path))
                .is_some_and(|document| document.text() == script.text);
            let diagnostics = match current {
                true => script.diagnostics.clone(),
                false => Vec::new(),
            };
            (self.project).set_loader_diagnostics(&script.path, diagnostics);
        }
        reached
    }

    /// Takes the table of each set of files that the loader read, which a load or a chart's run
    /// can change, and analyzes the language project with them when they changed: another set of
    /// files, another table, or another error. The result is the files that the change analyzes
    /// again, as `Project::set_file_tables` gives them, or none when the tables did not change.
    pub fn set_file_tables(&mut self, tables: Vec<(Files, FilesTable)>) -> BTreeSet<String> {
        let same = |(a, ta): &(Files, FilesTable), (b, tb): &(Files, FilesTable)| {
            a == b
                && match (ta, tb) {
                    (Ok(ta), Ok(tb)) => Arc::ptr_eq(ta, tb),
                    (Err(ta), Err(tb)) => ta == tb,
                    _ => false,
                }
        };
        let changed = tables.len() != self.file_tables.len()
            || !(tables.iter().zip(&self.file_tables)).all(|(a, b)| same(a, b));
        if !changed {
            return BTreeSet::new();
        }
        self.file_tables = tables.clone();
        self.project.set_file_tables(tables)
    }
}

/// The chart scripts of `charts`, each a path and its text, that a load that changed the tables
/// `changes` runs again, by path: those that read a changed table, with a path of one name in
/// the schema `default_schema`, those that import a module, whose functions can read tables
/// that the chart's text doesn't name, and those of `reading`, whose own tables of files read
/// files that changed, as [`FilesReload::charts`] gives them.
pub fn charts_to_run<'a>(
    charts: impl IntoIterator<Item = (&'a str, &'a str)>,
    changes: &[TableChange],
    reading: &[String],
    default_schema: &str,
) -> Vec<String> {
    (charts.into_iter())
        .filter(|(path, text)| {
            reads(text, changes, default_schema) || reading.iter().any(|chart| chart == path)
        })
        .map(|(path, _)| path.to_string())
        .collect()
}

/// Whether the chart script `text` reads a table of `changes`, or imports a module.
fn reads(text: &str, changes: &[TableChange], default_schema: &str) -> bool {
    let root = avenger_sql_syntax::parse(text).syntax();
    let read = references(&root);
    !read.modules.is_empty()
        || (read.tables.iter()).any(|table| {
            (changes.iter()).any(|change| same_table(&change.names, &table.names, default_schema))
        })
}
