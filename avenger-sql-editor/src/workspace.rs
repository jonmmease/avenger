//! A project open in the editor: the loader's project, which loads the data scripts and runs the
//! chart scripts in the background, the language project, which analyzes every script and module
//! for the editor, the project's stylesheets, which style its charts, the sidebar's model of the
//! files by role, the gallery's entries of the chart scripts, and the tabs of the open files. A
//! project is a directory on disk, or the sample project that the editor embeds, which is
//! read-only.
//!
//! The window owns a [`Workspace`], and the background work shares its [`Loader`]. Each load of
//! the loader sends back a [`LoadState`], which the workspace applies to the language project and
//! the sidebar, so that the window never waits for the loader.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow::datatypes::DataType;
use avenger_sql::{
    analysis::Options,
    catalog::{references, same_path, FilesTable, MemoryCatalog},
    data::{Files, Registration},
    library::Library,
};
use avenger_sql_language::{Diagnostic, FileRole, Project, Severity, Source};
use avenger_sql_project::{
    ChartError, LoadedProject, ModuleFile, ProjectFiles, ProjectStylesheet, ScriptStatus,
    TableChange,
};
use avenger_style::{Sheet, SourceId, Span};
use datafusion::prelude::SessionContext;
use tokio::sync::RwLock;

use crate::buffer::Buffer;
use crate::gallery::{self, Entry};
use crate::highlight::Chars;
use crate::query::{self, LiftedText, Outcome, QueryError};

/// The loader's project, which the background work shares. A chart run reads it and a load of
/// data scripts writes it, so that a chart never reads a project that is partly loaded.
///
/// The futures of the loader are not `Send`, so they run on one thread, as the editor's
/// [`Exec`](crate::exec::Exec) runs its work. A load or a reload removes the tables of its
/// scripts before it registers them again, so the editor never stops one before its end.
pub struct Loader {
    project: RwLock<LoadedProject>,
}

/// What the window shows of a load of the data scripts: the result of each data script, and the
/// table of each set of files that they read, which the language project analyzes with.
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
    /// The loader's errors in the text.
    pub diagnostics: Vec<Diagnostic>,
    /// The tables and views that the script registered, in the order of its text.
    pub tables: Vec<Table>,
}

/// Whether a data script loaded, as the sidebar shows it.
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
    /// Loads the data scripts of `files` into a new session.
    pub async fn load(files: ProjectFiles) -> Result<(Loader, LoadState), String> {
        let project = (LoadedProject::load(files, SessionContext::new()).await)
            .map_err(|error| error.to_string())?;
        let state = LoadState::of(&project);
        let project = RwLock::new(project);
        Ok((Loader { project }, state))
    }

    /// Loads the data script at `path` again with `text`, and the scripts downstream of it, and
    /// returns the tables that changed and the state of the load.
    pub async fn reload(
        &self,
        path: &str,
        text: String,
    ) -> Result<(Vec<TableChange>, LoadState), String> {
        let mut project = self.project.write().await;
        let changes = (project.reload(path, text).await).map_err(|error| error.to_string())?;
        Ok((changes, LoadState::of(&project)))
    }

    /// Opens the project again from its files, as they are on disk, and loads every data script
    /// again, as after a module changes, which the data scripts and the charts import.
    pub async fn reload_all(&self) -> Result<(Vec<TableChange>, LoadState), String> {
        let mut project = self.project.write().await;
        let changes = (project.reload_all().await).map_err(|error| error.to_string())?;
        Ok((changes, LoadState::of(&project)))
    }

    /// Runs the chart script at `path`, whose text is `text`, against the tables of the project
    /// and with its own tables of files, which the loader keeps by `path`. A chart that reads a
    /// table of a data script that did not load has the error of each such read, which names
    /// the script, and then the error of its run, at the first such read.
    pub async fn run_chart(&self, path: &str, text: &str) -> Result<Outcome, QueryError> {
        let project = self.project.read().await;
        let frame = match project.run_chart(path, text).await {
            Ok(frame) => frame,
            Err(ChartError::Run(error)) => return Err(QueryError::new(&error, text)),
            Err(error @ ChartError::Unloaded { .. }) => {
                let ChartError::Unloaded { reads, .. } = &error else {
                    unreachable!("an unloaded read");
                };
                let range = reads.first().map(|read| Chars::new(text).range(read.range));
                return Err(QueryError {
                    message: error.to_string(),
                    range,
                });
            }
        };
        query::outcome(frame, text).await
    }

    /// The lifted form of the chart script at `path`, whose text is `text`, over the tables of
    /// the project and with its modules, as [`Loader::run_chart`] runs it.
    pub async fn lift(&self, path: &str, text: &str) -> Result<LiftedText, QueryError> {
        let project = self.project.read().await;
        query::lifted(project.lift(path, text).await, text)
    }

    /// The table of each set of files that the data scripts and the charts' own tables of files
    /// read, in their last loads and runs, which the language project analyzes with.
    pub async fn file_tables(&self) -> Vec<(Files, FilesTable)> {
        self.project.read().await.file_tables()
    }
}

/// The stylesheets of a project, as their last load read them: the text of each `.css` file, and
/// the entry stylesheet loaded with its imports, which styles every chart of the project.
#[derive(Clone, Debug)]
pub struct Styles {
    entry: String,
    texts: BTreeMap<String, String>,
    loaded: ProjectStylesheet,
}

impl Styles {
    /// Reads the `.css` files of `files`, and loads the entry stylesheet with its imports. A
    /// `.css` file that does not read has no text, and an import of it is an error of the load.
    pub async fn load(files: &ProjectFiles) -> Result<Styles, String> {
        let mut texts = BTreeMap::new();
        for path in files.stylesheets() {
            if let Ok(text) = files.source().read_text(path).await {
                texts.insert(path.clone(), text);
            }
        }
        let loaded = (files.load_stylesheet().await).map_err(|error| error.to_string())?;
        Ok(Styles {
            entry: files.manifest().style.stylesheet.clone(),
            texts,
            loaded,
        })
    }

    /// The entry stylesheet with its imports, or `None` when the project has none.
    pub fn sheet(&self) -> Option<&Sheet> {
        self.loaded.sheet.as_ref()
    }

    /// The text of the stylesheet at `path`, as the load read it.
    pub fn text(&self, path: &str) -> Option<&str> {
        self.texts.get(path).map(String::as_str)
    }

    /// The errors and warnings of the stylesheet at `path`, at byte spans of its text as the load
    /// read it: those of the load of the entry stylesheet, and a warning on the first line of a
    /// stylesheet that the entry stylesheet does not reach, which styles no chart.
    pub fn diagnostics(&self, path: &str) -> Vec<avenger_style::Diagnostic> {
        let Some(source) = (self.loaded.sources.iter()).position(|source| source == path) else {
            return Vec::new();
        };
        let source = SourceId(source as u32);
        let mut diagnostics: Vec<avenger_style::Diagnostic> = (self.loaded.diagnostics.iter())
            .filter(|diagnostic| diagnostic.span.source == source)
            .cloned()
            .collect();
        if self
            .loaded
            .unreached
            .iter()
            .any(|unreached| unreached == path)
        {
            let text = self.text(path).unwrap_or_default();
            let line = text.find('\n').unwrap_or(text.len());
            let message = match self.loaded.sheet {
                Some(_) => format!(
                    "this stylesheet is not imported by the entry stylesheet `{}`, so it styles \
                     no chart",
                    self.entry
                ),
                None => format!(
                    "the project has no entry stylesheet `{}` to import this stylesheet, so it \
                     styles no chart",
                    self.entry
                ),
            };
            let mut warning =
                avenger_style::Diagnostic::warning(Span::new(source, 0, line), message);
            warning.help = Some(format!(
                "Add `@import '{}';` to `{}`.",
                relative_import(&self.entry, path),
                self.entry
            ));
            diagnostics.insert(0, warning);
        }
        diagnostics
    }
}

/// The URL of an `@import` in the stylesheet at `importer` of the stylesheet at `path`, both
/// relative to the project root.
fn relative_import(importer: &str, path: &str) -> String {
    let folder: Vec<&str> = importer.split('/').collect();
    let folder = &folder[..folder.len() - 1];
    let names: Vec<&str> = path.split('/').collect();
    let shared = (folder.iter().zip(&names))
        .take_while(|(a, b)| a == b)
        .count();
    let mut url: Vec<&str> = vec![".."; folder.len() - shared];
    url.extend(&names[shared..]);
    url.join("/")
}

/// What the file of a tab is: a script or a module, which the language project analyzes, or a
/// stylesheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TabKind {
    Sql(FileRole),
    Stylesheet,
}

/// A file of an open project in a tab: its path, its kind, the text that its file holds on disk,
/// and its buffer, whose text the editor edits.
pub struct Tab {
    pub path: String,
    pub kind: TabKind,
    /// The text of the file on disk, as the last open or save left it.
    saved: String,
    pub buffer: Buffer,
}

impl Tab {
    /// Whether the text differs from the file on disk.
    pub fn modified(&self) -> bool {
        self.buffer.text != self.saved
    }

    /// The name of the tab: the name of its file.
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

/// What a save asks the editor to do next.
#[derive(Debug, PartialEq, Eq)]
pub enum Saved {
    /// Load the data script at `path` again with `text`.
    Data { path: String, text: String },
    /// Run the chart script at `path`.
    Chart { path: String },
    /// Load the project again, since a module that its scripts import changed.
    Module,
    /// Load the stylesheets again, and restyle the charts.
    Stylesheet,
}

/// The files of a project by role, as the sidebar lists them.
#[derive(Clone, Debug, Default)]
pub struct Sidebar {
    /// The data scripts by path, each with the result of its last load, `None` before one.
    pub data: Vec<(String, Option<ScriptState>)>,
    /// The chart scripts by folder, each folder by path and its scripts by path.
    pub charts: Vec<(String, Vec<String>)>,
    /// The own tables of files of each chart script that has them, by path, with their columns
    /// when the language project knows them.
    pub chart_tables: std::collections::BTreeMap<String, Vec<Table>>,
    pub stylesheets: Vec<String>,
    pub modules: Vec<ModuleFile>,
}

/// A project open in the editor: its root on disk, its files by role, the loader, the language
/// project, the stylesheets, and the tabs of its open files.
pub struct Workspace {
    /// The root of the project on disk, or `None` for the sample project, whose files the editor
    /// embeds and which can't be saved.
    root: Option<PathBuf>,
    files: ProjectFiles,
    loader: Arc<Loader>,
    language: Project,
    styles: Styles,
    /// The result of the last load of each data script, in the order of the load.
    scripts: Vec<ScriptState>,
    /// The table of each set of files that the loader read, as the language project has them.
    file_tables: Vec<(Files, FilesTable)>,
    tabs: Vec<Tab>,
    active: Option<usize>,
    /// The path of the script whose result the result pane shows: the active tab's, or while a
    /// stylesheet's tab is active, that of the script tab that was active before it.
    shown: Option<String>,
    /// The data scripts that load again in the background.
    pub loading: BTreeSet<String>,
    /// Whether the stylesheets load again in the background.
    pub styling: bool,
}

/// The project on disk that holds `path`, a directory or a file under the project's root, loaded,
/// and the path of the file of `path` relative to the root, when it is a file.
#[cfg(not(target_arch = "wasm32"))]
pub async fn open(path: &Path) -> Result<(Workspace, Option<String>), String> {
    let root = avenger_sql_project::find_root(path).ok_or_else(|| {
        format!(
            "{} is not in a project: no {} in it or the directories above it",
            path.display(),
            avenger_sql_project::MANIFEST
        )
    })?;
    let files = (ProjectFiles::find(&root).await).map_err(|error| error.to_string())?;
    let file = std::fs::canonicalize(path)
        .ok()
        .filter(|path| path.is_file())
        .and_then(|path| relative(&root, &path));
    Ok((Workspace::load(Some(root), files).await?, file))
}

/// The sample project, which the editor embeds, loaded.
pub async fn sample() -> Result<Workspace, String> {
    let source = (crate::samples::source().await).map_err(|error| error.to_string())?;
    let files = (ProjectFiles::open(source).await).map_err(|error| error.to_string())?;
    Workspace::load(None, files).await
}

/// Makes a project in the directory `directory`: writes the smallest manifest, named after the
/// directory, unless the directory holds a manifest, and returns the directory.
#[cfg(not(target_arch = "wasm32"))]
pub fn create(directory: &Path) -> Result<PathBuf, String> {
    use avenger_sql_project::{Manifest, MANIFEST};
    let directory = std::fs::canonicalize(directory)
        .map_err(|error| format!("cannot open {}: {error}", directory.display()))?;
    let manifest = directory.join(MANIFEST);
    if !manifest.is_file() {
        let name = (directory.file_name()).map_or_else(
            || "project".to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        std::fs::write(&manifest, Manifest::smallest(&name))
            .map_err(|error| format!("cannot write {}: {error}", manifest.display()))?;
    }
    Ok(directory)
}

/// The path of `path` relative to `root`, with `/` between its names.
#[cfg(not(target_arch = "wasm32"))]
fn relative(root: &Path, path: &Path) -> Option<String> {
    let names: Option<Vec<&str>> = (path.strip_prefix(root).ok()?.components())
        .map(|component| component.as_os_str().to_str())
        .collect();
    Some(names?.join("/"))
}

/// The error of a write of the file `file`, the file at `path` of the project, when it no longer
/// holds `saved`, the text that the editor read from it or wrote to it last: another program
/// changed it, and the write would lose that change.
fn unchanged_on_disk(file: &Path, path: &str, saved: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(file)
        .map_err(|error| format!("cannot read {}: {error}", file.display()))?;
    match text == saved {
        true => Ok(()),
        false => Err(format!(
            "{path} changed on disk since the editor read it, and a save would lose that change: \
             close its tab and open it again to edit the text on disk"
        )),
    }
}

impl Workspace {
    /// Loads the project `files`, whose root on disk is `root`, or `None` for files that can't be
    /// saved: loads its data scripts and its stylesheets, and analyzes every data script, chart
    /// script, and module with the tables that the load read.
    pub async fn load(root: Option<PathBuf>, files: ProjectFiles) -> Result<Workspace, String> {
        let source = files.source().clone();
        let mut texts = Vec::new();
        let roles = (files.data_scripts().iter())
            .map(|path| (path, FileRole::Data))
            .chain((files.chart_scripts().iter()).map(|path| (path, FileRole::Chart)))
            .chain((files.modules().iter()).map(|module| {
                let role = FileRole::Module {
                    names: module.names.clone(),
                };
                (&module.path, role)
            }));
        for (path, role) in roles {
            let text = (source.read_text(path).await).map_err(|error| error.to_string())?;
            texts.push((path.clone(), role, text));
        }
        let paths: Arc<[String]> = files.files().into();
        let styles = Styles::load(&files).await?;
        let (loader, state) = Loader::load(files.clone()).await?;
        let language = Project::new(
            Arc::new(MemoryCatalog::new()),
            Arc::new(Library::new()),
            Options::default(),
        )
        .with_paths(paths)
        .with_files(texts);
        let mut workspace = Workspace {
            root,
            files,
            loader: Arc::new(loader),
            language,
            styles,
            scripts: Vec::new(),
            tabs: Vec::new(),
            file_tables: Vec::new(),
            active: None,
            shown: None,
            loading: BTreeSet::new(),
            styling: false,
        };
        workspace.apply(state);
        Ok(workspace)
    }

    /// The name of the project, from its manifest.
    pub fn name(&self) -> &str {
        &self.files.manifest().project.name
    }

    /// The root of the project on disk, or `None` for the sample project.
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// Whether the project can't be saved, as the sample project, whose edits run in memory.
    pub fn read_only(&self) -> bool {
        self.root.is_none()
    }

    pub fn loader(&self) -> &Arc<Loader> {
        &self.loader
    }

    pub fn files(&self) -> &ProjectFiles {
        &self.files
    }

    /// The language project, whose documents the editor analyzes the tabs with.
    pub fn language(&self) -> &Project {
        &self.language
    }

    /// The stylesheets, as their last load read them.
    pub fn styles(&self) -> &Styles {
        &self.styles
    }

    /// Takes the stylesheets of a new load.
    pub fn set_styles(&mut self, styles: Styles) {
        self.styles = styles;
    }

    /// Takes the state of a load: the tables of the files for the language project, the
    /// loader's errors of each data script whose text is the text that loaded, and the tables of
    /// each data script for the sidebar.
    pub fn apply(&mut self, state: LoadState) {
        self.set_file_tables(state.file_tables);
        for script in &state.scripts {
            // The ranges of the loader's errors are in the text that loaded, which an edit since
            // has moved.
            let current = (self.language.document(&script.path))
                .is_some_and(|document| document.text() == script.text);
            let diagnostics = match current {
                true => script.diagnostics.clone(),
                false => Vec::new(),
            };
            self.language
                .set_loader_diagnostics(&script.path, diagnostics);
        }
        self.scripts = state.scripts;
    }

    /// Takes the table of each set of files that the loader read, which a load or a chart's run
    /// can change, and analyzes the language project with them when they changed.
    pub fn set_file_tables(&mut self, tables: Vec<(Files, FilesTable)>) {
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
        if changed {
            self.language.set_file_tables(tables.clone());
            self.file_tables = tables;
        }
    }

    /// The files of the project by role, with the tables of the data scripts and the charts' own
    /// tables of files.
    pub fn sidebar(&self) -> Sidebar {
        let data = (self.files.data_scripts().iter())
            .map(|path| {
                let state = self.scripts.iter().find(|script| script.path == *path);
                (path.clone(), state.cloned())
            })
            .collect();
        let mut charts: Vec<(String, Vec<String>)> = Vec::new();
        for path in self.files.chart_scripts() {
            let folder = path.rsplit_once('/').map_or("", |(folder, _)| folder);
            match charts.iter_mut().find(|(name, _)| name == folder) {
                Some((_, paths)) => paths.push(path.clone()),
                None => charts.push((folder.to_string(), vec![path.clone()])),
            }
        }
        charts.sort_by(|a, b| a.0.cmp(&b.0));
        let chart_tables = (self.files.chart_scripts().iter())
            .filter_map(|path| {
                let analysis = self.language.document(path)?.analysis()?;
                let tables: Vec<Table> = (analysis.file_tables.iter())
                    .filter(|definition| definition.appends().is_none())
                    .map(|definition| Table {
                        names: definition.names.clone(),
                        columns: (definition.schema.iter())
                            .flat_map(|schema| schema.fields().iter())
                            .map(|field| (field.name().clone(), field.data_type().clone()))
                            .collect(),
                    })
                    .collect();
                (!tables.is_empty()).then(|| (path.clone(), tables))
            })
            .collect();
        Sidebar {
            data,
            charts,
            chart_tables,
            stylesheets: self.files.stylesheets().to_vec(),
            modules: self.files.modules().to_vec(),
        }
    }

    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    pub fn tab(&self, path: &str) -> Option<&Tab> {
        self.tabs.iter().find(|tab| tab.path == path)
    }

    pub fn tab_mut(&mut self, path: &str) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|tab| tab.path == path)
    }

    /// The tab that the editor shows.
    pub fn active(&self) -> Option<&Tab> {
        self.tabs.get(self.active?)
    }

    pub fn active_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active?)
    }

    /// The tab that the editor shows, the language project that analyzes it, and the
    /// stylesheets.
    pub fn active_with_context(&mut self) -> Option<(&mut Tab, &Project, &Styles)> {
        let tab = self.tabs.get_mut(self.active?)?;
        Some((tab, &self.language, &self.styles))
    }

    /// The tab whose result the result pane shows: the active tab, or while a stylesheet's tab
    /// is active, the script tab that was active before it, so that its chart shows the styles
    /// as they are saved.
    pub fn shown(&self) -> Option<&Tab> {
        self.tab(self.shown.as_deref()?)
    }

    pub fn shown_mut(&mut self) -> Option<&mut Tab> {
        let path = self.shown.clone()?;
        self.tab_mut(&path)
    }

    /// Shows the tab of the file at `path`, and opens it first when it is not open. Returns
    /// whether the tab opened now, or `None` for a path that is not a script, a module, or a
    /// stylesheet of the project.
    pub fn open_tab(&mut self, path: &str) -> Option<bool> {
        if let Some(index) = self.tabs.iter().position(|tab| tab.path == path) {
            self.activate(index);
            return Some(false);
        }
        let (kind, text) = match self.styles.text(path) {
            Some(text) => (TabKind::Stylesheet, text.to_string()),
            None => (
                TabKind::Sql(self.language.role(path)?.clone()),
                self.language.document(path)?.text().to_string(),
            ),
        };
        let id = (
            "tab",
            self.root.clone(),
            self.name().to_string(),
            path.to_string(),
        );
        self.tabs.push(Tab {
            path: path.to_string(),
            kind,
            saved: text.clone(),
            buffer: Buffer::new(text, id),
        });
        self.activate(self.tabs.len() - 1);
        Some(true)
    }

    /// Makes the tab at `index` the active tab, and the tab whose result shows unless it is a
    /// stylesheet's.
    fn activate(&mut self, index: usize) {
        self.active = Some(index);
        let tab = &self.tabs[index];
        if tab.kind != TabKind::Stylesheet {
            self.shown = Some(tab.path.clone());
        }
    }

    /// Closes the tab of the file at `path`, and shows the tab after it, or else the one before.
    /// Returns the closed tab, whose unsaved text is lost.
    pub fn close_tab(&mut self, path: &str) -> Option<Tab> {
        let index = self.tabs.iter().position(|tab| tab.path == path)?;
        let tab = self.tabs.remove(index);
        if self.shown.as_deref() == Some(path) {
            self.shown = None;
        }
        match self.active {
            _ if self.tabs.is_empty() => self.active = None,
            Some(active) if active > index => self.active = Some(active - 1),
            Some(active) => self.activate(active.min(self.tabs.len() - 1)),
            None => {}
        }
        // The language project goes back to the text on disk, which the loader read.
        if tab.modified() && tab.kind != TabKind::Stylesheet {
            self.language.set_texts([(path, tab.saved.clone())]);
        }
        Some(tab)
    }

    /// Analyzes the text of the tab of `path` again, when it changed since its last analysis.
    pub fn sync(&mut self, path: &str) {
        let Some(tab) = self.tabs.iter().find(|tab| tab.path == path) else {
            return;
        };
        let stale = (self.language.document(path))
            .is_some_and(|document| document.text() != tab.buffer.text);
        if stale {
            self.language.set_texts([(path, tab.buffer.text.clone())]);
        }
    }

    /// Whether a tab holds edits that are not saved.
    pub fn modified(&self) -> bool {
        self.tabs.iter().any(Tab::modified)
    }

    /// Drops the edits of the tabs that are not saved: each tab, and the language project, go
    /// back to the text on disk.
    pub fn discard(&mut self) {
        let modified: Vec<(String, String)> = (self.tabs.iter())
            .filter(|tab| tab.modified())
            .map(|tab| (tab.path.clone(), tab.saved.clone()))
            .collect();
        for (path, saved) in modified {
            self.edit(&path, &saved);
        }
    }

    /// Sets the text of the tab of `path`, as the editor's edits do, and analyzes it.
    pub fn edit(&mut self, path: &str, text: &str) {
        if let Some(tab) = self.tab_mut(path) {
            tab.buffer.text = text.to_string();
            self.sync(path);
        }
    }

    /// Writes the text of the tab of `path` to its file, and returns what the editor does next:
    /// a data script loads again, a chart runs, the project loads again after a module changes,
    /// and the stylesheets load again after a stylesheet changes.
    pub fn save(&mut self, path: &str) -> Result<Saved, String> {
        let file = (self.root.as_ref())
            .ok_or_else(|| format!("{path} is a file of the sample project, which can't be saved"))?
            .join(path);
        let tab = (self.tabs.iter_mut().find(|tab| tab.path == path))
            .ok_or_else(|| format!("{path} is not open"))?;
        unchanged_on_disk(&file, path, &tab.saved)?;
        std::fs::write(&file, &tab.buffer.text)
            .map_err(|error| format!("cannot save {}: {error}", file.display()))?;
        tab.saved = tab.buffer.text.clone();
        Ok(match &tab.kind {
            TabKind::Sql(FileRole::Data) => Saved::Data {
                path: path.to_string(),
                text: tab.saved.clone(),
            },
            TabKind::Sql(FileRole::Chart) => Saved::Chart {
                path: path.to_string(),
            },
            TabKind::Sql(FileRole::Module { .. }) => Saved::Module,
            TabKind::Stylesheet => Saved::Stylesheet,
        })
    }

    /// The entries of the gallery: a chart script of the project for each, with its text on
    /// disk, or as the editor's tab last saved it.
    pub fn gallery(&self) -> Vec<Entry> {
        let charts = (self.files.chart_scripts().iter()).filter_map(|path| {
            let text = match self.tab(path) {
                Some(tab) => tab.saved.clone(),
                None => self.language.document(path)?.text().to_string(),
            };
            Some((path.clone(), text))
        });
        gallery::entries(self.files.manifest(), charts)
    }

    /// The gallery's entry of the chart script at `path`, with its saved text.
    pub fn entry(&self, path: &str) -> Option<Entry> {
        let tab = self.tab(path)?;
        Some(Entry::new(self.files.manifest(), path, tab.saved.clone()))
    }

    /// The open chart tabs that a load that changed the tables `changes` runs again: those whose
    /// charts read a changed table, and those that import a module, whose functions can read
    /// tables that the chart's text doesn't name.
    pub fn reruns(&self, changes: &[TableChange]) -> Vec<String> {
        (self.tabs.iter())
            .filter(|tab| tab.kind == TabKind::Sql(FileRole::Chart))
            .filter(|tab| reads(&tab.buffer.text, changes))
            .map(|tab| tab.path.clone())
            .collect()
    }
}

/// Whether the chart script `text` reads a table of `changes`, or imports a module, whose
/// functions can read tables that the chart's text doesn't name.
pub fn reads(text: &str, changes: &[TableChange]) -> bool {
    let root = avenger_sql_syntax::parse(text).syntax();
    let read = references(&root);
    !read.modules.is_empty()
        || (read.tables.iter())
            .any(|table| (changes.iter()).any(|change| same_path(&change.names, &table.names)))
}
