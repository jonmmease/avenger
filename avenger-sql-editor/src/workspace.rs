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
    ChartError, FilesReload, LoadedProject, ModuleFile, ProjectFiles, ProjectStylesheet,
    ScriptStatus, TableChange,
};
use avenger_style::{Sheet, SourceId, Span};
use datafusion::prelude::SessionContext;
use tokio::sync::RwLock;

#[cfg(not(target_arch = "wasm32"))]
use crate::add_file;
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

    /// Loads the data script or the module at `path` again with `text`, and the scripts
    /// downstream of it, and returns the tables that changed and the state of the load.
    pub async fn reload(
        &self,
        path: &str,
        text: String,
    ) -> Result<(Vec<TableChange>, LoadState), String> {
        let mut project = self.project.write().await;
        let changes = (project.reload(path, text).await).map_err(|error| error.to_string())?;
        Ok((changes, LoadState::of(&project)))
    }

    /// Loads the data scripts that read the files at `paths`, relative to the project's root,
    /// again, after the files changed on disk, and the scripts downstream of them, and returns
    /// what changed, the tables and the charts whose own tables of files read the files, which
    /// run again, and the state of the load, or `None` when no data script and no chart reads
    /// the files, so that nothing loads.
    pub async fn reload_files(&self, paths: &[String]) -> Option<(FilesReload, LoadState)> {
        let mut project = self.project.write().await;
        let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
        if (paths.iter()).all(|path| project.scripts_reading(path).is_empty()) {
            return None;
        }
        let reload = project.reload_files(&paths).await;
        Some((reload, LoadState::of(&project)))
    }

    /// Opens the project again from its files, as they are on disk, and loads every data script
    /// again, as after Add file writes a new data script.
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

/// What Add file did: the data script or the chart script that took the statements, the names of
/// the tables, the files that it copied into the project, relative to the root, the errors of the
/// files that it did not add, and what loads or runs next, or `None` while the script's tab holds
/// edits that are not saved.
#[derive(Debug, PartialEq, Eq)]
pub struct Added {
    pub script: String,
    pub tables: Vec<String>,
    pub copied: Vec<String>,
    pub errors: Vec<String>,
    pub load: Option<Load>,
    /// The chart script that runs, when the statements went into a chart that saved.
    pub run: Option<String>,
}

/// The data files of Add file before their copies: the data script that takes their statements,
/// and each file by its name, with its URI under the project's root, or its path outside the
/// root, which [`Adding::copy`] copies into the project in the background.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub struct Adding {
    root: PathBuf,
    script: String,
    files: Vec<(String, AddedFile)>,
    errors: Vec<String>,
}

/// Where a data file of Add file is.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
enum AddedFile {
    /// A file under the root, by its URI relative to the root.
    Under(String),
    /// A file outside the root, by its path on disk.
    Outside(PathBuf),
}

/// The data files of Add file after their copies, by name and URI, which [`Workspace::add`]
/// writes the statements of, with the paths of the copies, relative to the root, and the errors
/// of the files that are not added.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub struct Copied {
    script: String,
    files: Vec<(String, String)>,
    copied: Vec<String>,
    errors: Vec<String>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Adding {
    /// Copies each file from outside the project's root into [`add_file::FILES`], under a name
    /// that no file there has. The copies are I/O, which the editor makes in the background. A
    /// file that does not copy is not added, with its error.
    pub fn copy(self) -> Copied {
        let Adding {
            root,
            script,
            files,
            mut errors,
        } = self;
        let (mut added, mut copied) = (Vec::new(), Vec::new());
        for (name, file) in files {
            let uri = match file {
                AddedFile::Under(uri) => uri,
                AddedFile::Outside(path) => {
                    let data = add_file::DataFile::of(&name).expect("a data file of Add file");
                    match copy(&root, &path, &data) {
                        Ok(uri) => {
                            copied.push(uri.clone());
                            uri
                        }
                        Err(error) => {
                            errors.push(error);
                            continue;
                        }
                    }
                }
            };
            added.push((name, uri));
        }
        Copied {
            script,
            files: added,
            copied,
            errors,
        }
    }
}

/// A file under the root of the open project that changed on disk, by its path relative to the
/// root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Watched {
    /// A file of the project's own, a script, a module, or a stylesheet, which
    /// [`Workspace::reread`] reads again.
    Project(String),
    /// Another file, such as a data file, which can change a table: the editor loads the data
    /// scripts that read it again, with [`Loader::reload_files`].
    Data(String),
}

/// A load of data scripts that the editor starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Load {
    /// The data script or the module at `path`, with `text`, and the scripts downstream of it.
    Script { path: String, text: String },
    /// The data scripts that read the files at these paths, relative to the root, after they
    /// changed on disk.
    Files(Vec<String>),
    /// Every data script, from the project's files as they are on disk.
    All,
}

/// What a save asks the editor to do next.
#[derive(Debug, PartialEq, Eq)]
pub enum Saved {
    /// Load the data script at `path` again with `text`.
    Data { path: String, text: String },
    /// Run the chart script at `path`.
    Chart { path: String },
    /// Load the module at `path` again with `text`, and the data scripts that import it.
    Module { path: String, text: String },
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

/// Copies the data file `file` at `path`, outside the project's root, into the directory of the
/// project's data files under a name that no file there has, and returns its path relative to
/// the root.
#[cfg(not(target_arch = "wasm32"))]
fn copy(root: &Path, path: &Path, file: &add_file::DataFile<'_>) -> Result<String, String> {
    let directory = root.join(add_file::FILES);
    let cannot = |error: std::io::Error| format!("cannot copy {}: {error}", path.display());
    let mut from = std::fs::File::open(path).map_err(cannot)?;
    std::fs::create_dir_all(&directory).map_err(cannot)?;
    loop {
        let name = file.free_name(|name| directory.join(name).exists());
        let target = directory.join(&name);
        let mut to = match std::fs::File::create_new(&target) {
            Ok(to) => to,
            // A file of the name that appeared since the name was free.
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(cannot(error)),
        };
        if let Err(error) = std::io::copy(&mut from, &mut to) {
            let _ = std::fs::remove_file(&target);
            return Err(cannot(error));
        }
        return Ok(format!("{}/{name}", add_file::FILES));
    }
}

/// The error of Add file when it adds no file: the errors of the files, or else that it has none.
#[cfg(not(target_arch = "wasm32"))]
fn no_file(errors: &[String]) -> String {
    match errors.is_empty() {
        true => "No file to add".to_string(),
        false => errors.join("\n"),
    }
}

/// Writes `bytes` to a new file at `path`, and never over a file that is there.
#[cfg(not(target_arch = "wasm32"))]
fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::File::create_new(path)?.write_all(bytes)
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
    /// a data script or a module loads again, with the data scripts downstream of it, a chart
    /// runs, and the stylesheets load again after a stylesheet changes.
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
            TabKind::Sql(FileRole::Module { .. }) => Saved::Module {
                path: path.to_string(),
                text: tab.saved.clone(),
            },
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

    /// The gallery's entry of the chart script at `path`, with its saved text, or its text on
    /// disk without a tab.
    pub fn entry(&self, path: &str) -> Option<Entry> {
        let text = match self.tab(path) {
            Some(tab) => tab.saved.clone(),
            None => self.language.document(path)?.text().to_string(),
        };
        Some(Entry::new(self.files.manifest(), path, text))
    }

    /// The data files at `paths`, on disk, that Add file and a drop on the window add to the
    /// project, before their copies: a file under the project's root keeps its place, and
    /// [`Adding::copy`] copies a file outside it into [`add_file::FILES`] under a name that no
    /// file there has. The statements go into the script of [`add_file::script`]: the active
    /// chart script, or a data script. A file of no format that a data script reads is not
    /// added, with its error in [`Added::errors`].
    ///
    /// Adding fails for the sample project, which is read-only, when it adds no file, and when
    /// the script changed on disk since the editor read it, and its tab holds no edits that are
    /// not saved, since the save of the statements would lose that change.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn adding(&self, paths: &[PathBuf]) -> Result<Adding, String> {
        let root = (self.root.clone()).ok_or_else(|| {
            "The sample project is read-only: open or make a project to add files".to_string()
        })?;
        let active = (self.active()).map(|tab| tab.path.clone());
        let (data, charts) = (self.files.data_scripts(), self.files.chart_scripts());
        let script = add_file::script(active.as_deref(), data, charts).to_string();
        let new = !data.contains(&script) && !charts.contains(&script);
        let role = self.files.manifest().role(&script);
        if new && !matches!(role, Ok(Some(avenger_sql_project::Role::DataScript))) {
            return Err(format!(
                "The project has no data script, and its manifest gives {script} no role of a \
                 data script: make a data script to add files to"
            ));
        }
        if !new {
            let saved = match self.tab(&script) {
                Some(tab) => (!tab.modified()).then(|| tab.saved.clone()),
                None => (self.language.document(&script)).map(|document| document.text().into()),
            };
            if let Some(saved) = saved {
                unchanged_on_disk(&root.join(&script), &script, &saved)?;
            }
        }
        let (mut files, mut errors) = (Vec::new(), Vec::new());
        for path in paths {
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                errors.push(format!("{} has a name that is not UTF-8", path.display()));
                continue;
            };
            if let Err(error) = add_file::DataFile::of(name) {
                errors.push(error);
                continue;
            }
            let under = (std::fs::canonicalize(path).ok())
                .and_then(|canonical| relative(&root, &canonical));
            let file = match under {
                Some(uri) if uri.contains('*') => {
                    errors.push(format!(
                        "{uri} has `*` in its path, which a URI reads as a pattern of names"
                    ));
                    continue;
                }
                Some(uri) => AddedFile::Under(uri),
                None => AddedFile::Outside(path.clone()),
            };
            files.push((name.to_string(), file));
        }
        if files.is_empty() {
            return Err(no_file(&errors));
        }
        Ok(Adding {
            root,
            script,
            files,
            errors,
        })
    }

    /// Writes the `CREATE EXTERNAL TABLE` statement of each file of `copied`, with a table name
    /// that no table of the project has, at the end of its data script, whose tab opens, and adds
    /// the copies to the files of the project. A chart script takes `CREATE TEMP EXTERNAL TABLE`
    /// statements before its query, after its other statements of files, with names that no
    /// table of the project and no table of the chart has.
    ///
    /// A script whose tab holds edits that are not saved takes the statements in its tab, which
    /// stays modified, and loads or runs when it is saved. Otherwise the script is saved, and
    /// [`Added::load`] tells what loads: the saved data script, or, for a new script, the whole
    /// project, whose loader lists its files again, and [`Added::run`] the saved chart, which
    /// runs. Adding fails when no file copied.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn add(&mut self, copied: Copied) -> Result<Added, String> {
        let Copied {
            script,
            files,
            copied,
            errors,
        } = copied;
        let root = (self.root.clone()).ok_or("The sample project is read-only")?;
        // An earlier Add file can have made the script since the copies began.
        let chart = self.files.chart_scripts().contains(&script);
        let new = !chart && !self.files.data_scripts().contains(&script);
        let mut taken = self.table_names();
        let role = match chart {
            true => {
                let text = self.tab(&script).map(|tab| tab.buffer.text.clone());
                let text = text.or_else(|| {
                    (self.language.document(&script)).map(|document| document.text().to_string())
                });
                taken.extend(add_file::chart_tables(&text.unwrap_or_default()));
                avenger_sql_language::Role::Script
            }
            false => avenger_sql_language::Role::Data,
        };
        let (mut statements, mut tables) = (Vec::new(), Vec::new());
        for (name, uri) in &files {
            let file = add_file::DataFile::of(name).expect("a data file of Add file");
            let table = add_file::table_name(file.stem, &taken);
            taken.insert(table.clone());
            statements.push(add_file::statement(&table, &file, uri, role));
            tables.push(table);
        }
        if statements.is_empty() {
            return Err(no_file(&errors));
        }
        let mut listed = copied.clone();
        if new {
            let text = add_file::append("", &statements);
            let file = root.join(&script);
            let written = std::fs::create_dir_all(file.parent().unwrap_or(&root))
                .and_then(|()| write_new(&file, text.as_bytes()));
            if let Err(error) = written {
                self.list(listed)?;
                return Err(format!("cannot write {}: {error}", file.display()));
            }
            listed.push(script.clone());
            self.list(listed)?;
            self.language.add(&script, FileRole::Data, text);
            self.open_tab(&script);
            return Ok(Added {
                script,
                tables,
                copied,
                errors,
                load: Some(Load::All),
                run: None,
            });
        }
        self.list(listed)?;
        self.open_tab(&script);
        let tab = (self.tab(&script)).ok_or_else(|| format!("{script} does not open"))?;
        let modified = tab.modified();
        let text = match chart {
            true => add_file::insert(&tab.buffer.text, &statements),
            false => add_file::append(&tab.buffer.text, &statements),
        };
        self.edit(&script, &text);
        let (load, run) = match modified {
            true => (None, None),
            false => match self.save(&script)? {
                Saved::Data { path, text } => (Some(Load::Script { path, text }), None),
                Saved::Chart { path } => (None, Some(path)),
                saved => unreachable!("a script of Add file saves as one, not {saved:?}"),
            },
        };
        Ok(Added {
            script,
            tables,
            copied,
            errors,
            load,
            run,
        })
    }

    /// The names of the tables, views, and schemas that the data scripts define, in lowercase,
    /// as the language project analyzes their texts, with the edits of their tabs.
    #[cfg(not(target_arch = "wasm32"))]
    fn table_names(&self) -> BTreeSet<String> {
        let graph = self.language.graph();
        (graph.paths())
            .filter_map(|path| graph.names(path))
            .flat_map(|names| &names.definitions)
            .filter_map(|name| match name.names.as_slice() {
                [name] => Some(name.to_ascii_lowercase()),
                _ => None,
            })
            .collect()
    }

    /// Adds the files at `paths`, relative to the root, to the files of the project, whose
    /// paths completion offers in a URI.
    #[cfg(not(target_arch = "wasm32"))]
    fn list(&mut self, paths: Vec<String>) -> Result<(), String> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut listing = avenger_sql_project::Listing {
            paths: self.files.files().to_vec(),
            skipped: self.files.skipped().to_vec(),
        };
        listing.paths.extend(paths);
        let (manifest, source) = (self.files.manifest().clone(), self.files.source().clone());
        self.files =
            ProjectFiles::new(manifest, source, listing).map_err(|error| error.to_string())?;
        self.language.set_paths(self.files.files().into());
        Ok(())
    }

    /// The file at `path` on disk that changed, when it is a file under the root that is not
    /// hidden, nor under a hidden directory: a file of the project's own, or another file, such
    /// as a data file. The manifest and a file that two roles match are neither.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn watched(&self, path: &Path) -> Option<Watched> {
        let root = self.root.as_deref()?;
        // The root is a canonical path, and a removed file's directory still is one.
        let canonical = || match std::fs::canonicalize(path) {
            Ok(path) => Some(path),
            Err(_) => Some(
                std::fs::canonicalize(path.parent()?)
                    .ok()?
                    .join(path.file_name()?),
            ),
        };
        let path = match path.strip_prefix(root) {
            Ok(_) => path.to_path_buf(),
            Err(_) => canonical()?,
        };
        let relative = relative(root, &path)?;
        let hidden = relative.split('/').any(|name| name.starts_with('.'));
        if relative.is_empty() || hidden || relative == avenger_sql_project::MANIFEST {
            return None;
        }
        match self.files.manifest().role(&relative) {
            Ok(None) => Some(Watched::Data(relative)),
            Ok(Some(_)) => Some(Watched::Project(relative)),
            Err(_) => None,
        }
    }

    /// Reads the file at `path`, relative to the root, a script, a module, or a stylesheet,
    /// again after it changed on disk, and returns what the editor does next, as after a save:
    /// the language project analyzes the text on disk, and a tab of the file shows it. A tab
    /// with edits that are not saved keeps them, and its save fails, since it would lose the
    /// change. A file whose text is the one that the editor holds, as after the editor's own
    /// save, and a file that is not a file of the project, do nothing.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn reread(&mut self, path: &str) -> Option<Saved> {
        let file = self.root.as_deref()?.join(path);
        let text = std::fs::read_to_string(file).ok()?;
        let kind = match self.styles.text(path) {
            Some(_) => TabKind::Stylesheet,
            None => TabKind::Sql(self.language.role(path)?.clone()),
        };
        let held = match self.tab(path) {
            Some(tab) if tab.modified() => return None,
            Some(tab) => Some(tab.saved.as_str()),
            None if kind == TabKind::Stylesheet => self.styles.text(path),
            None => (self.language.document(path)).map(|document| document.text()),
        };
        if held == Some(text.as_str()) {
            return None;
        }
        if let Some(tab) = self.tab_mut(path) {
            tab.saved = text.clone();
            tab.buffer.text = text.clone();
        }
        if kind != TabKind::Stylesheet {
            self.language.set_texts([(path, text.clone())]);
        }
        Some(match kind {
            TabKind::Sql(FileRole::Data) => Saved::Data {
                path: path.to_string(),
                text,
            },
            TabKind::Sql(FileRole::Chart) => Saved::Chart {
                path: path.to_string(),
            },
            TabKind::Sql(FileRole::Module { .. }) => Saved::Module {
                path: path.to_string(),
                text,
            },
            TabKind::Stylesheet => Saved::Stylesheet,
        })
    }

    /// The open chart tabs that a load that changed the tables `changes` runs again: those whose
    /// charts read a changed table, those that import a module, whose functions can read
    /// tables that the chart's text doesn't name, and those of `charts`, whose own tables of
    /// files read files that changed.
    pub fn reruns(&self, changes: &[TableChange], charts: &[String]) -> Vec<String> {
        (self.tabs.iter())
            .filter(|tab| tab.kind == TabKind::Sql(FileRole::Chart))
            .filter(|tab| reads(&tab.buffer.text, changes) || charts.contains(&tab.path))
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
