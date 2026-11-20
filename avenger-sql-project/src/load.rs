//! Loading a project into a DataFusion session: its data scripts in the order of their
//! dependencies, its chart scripts against the tables that they register and with their own
//! tables of files, and the reloads of scripts that change.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, OnceLock},
};

use avenger_sql::{
    analysis::Options,
    analyze_data,
    catalog::{references, Catalog, FilesTable, SessionCatalog},
    data::{
        file_reads, file_table, file_urls, lower, DataAnalysis, Files, Registered, Registration,
    },
    library::Library,
    project::{unavailable_read, Edge, Graph, GraphError, NameKind, ProjectCatalog},
};
use avenger_sql_syntax::{Offset, TextRange};
use datafusion::{
    common::{DataFusionError, TableReference},
    dataframe::DataFrame,
    datasource::listing::ListingTableUrl,
    prelude::SessionContext,
};

use crate::project::{ProjectError, ProjectFiles};

/// The library of every script, which holds no state of a session.
fn library() -> &'static Library {
    static LIBRARY: OnceLock<Library> = OnceLock::new();
    LIBRARY.get_or_init(Library::new)
}

/// A project loaded into a DataFusion session: the tables that its data scripts register, the
/// result of each data script, and the modules that its scripts import.
///
/// [`LoadedProject::load`] runs the data scripts in the order of their [`Graph`], each after the
/// scripts whose tables, views, and schemas it reads. Each script reads the schemas of its
/// files, is analyzed with a [`ProjectCatalog`] of the session's tables, the definitions of the
/// scripts that it reads, and the project's modules, and is lowered into the session. A script
/// that fails registers nothing, and a script that reads a name of a script that did not load
/// does not load either. [`LoadedProject::run_chart`] runs a chart script against the session,
/// with the chart's own tables of files, which it keeps by the chart's path, and
/// [`LoadedProject::reload`] loads a changed data script and the scripts downstream of it again.
///
/// A `LoadedProject` is `Send` and `Sync`, and the futures of its loads, reloads, and chart runs
/// are not `Send`: they hold the syntax trees of scripts, which are not `Send`, across their
/// awaits, and [`lower`] and [`avenger_sql::sql_with_modules`] take a catalog and the modules as
/// `&dyn` references that are not `Sync`. A host awaits them on one thread, as a web build
/// does.
pub struct LoadedProject {
    files: ProjectFiles,
    ctx: SessionContext,
    /// The text of each data script, by path.
    texts: BTreeMap<String, String>,
    /// The text of each module, by its module path.
    modules: Vec<(Vec<String>, String)>,
    graph: Graph,
    /// The result of each data script, by path.
    scripts: BTreeMap<String, ScriptLoad>,
    /// The own tables of files of each chart script that ran, by path.
    charts: std::sync::Mutex<BTreeMap<String, ChartTables>>,
}

/// The own tables of files of a chart script, as its last run read and built them: a run of the
/// chart reuses them while its statements of files stay the same.
#[derive(Clone, Debug, Default)]
struct ChartTables {
    /// The table of each set of files that the chart's statements read, in the order of its
    /// text, or the error of reading it.
    files: Vec<(Files, FilesTable)>,
    /// The texts of the chart's statements of files, and the tables that they built, which the
    /// chart's plans read.
    built: Option<(Vec<String>, avenger_sql::data::ScriptTables)>,
}

/// An error at a range of the text of a file of a project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectDiagnostic {
    /// The path of the file, relative to the project's root.
    pub path: String,
    pub range: TextRange,
    pub message: String,
    pub help: Option<String>,
}

/// The result of loading a data script.
#[derive(Clone, Debug)]
pub struct ScriptLoad {
    pub path: String,
    pub status: ScriptStatus,
    /// Every error in the text of the script, in the order of its ranges: the errors of the
    /// graph at the script, such as a name that another script also defines or a read in a
    /// cycle, and the errors that kept it from loading.
    pub diagnostics: Vec<ProjectDiagnostic>,
    /// The table of each set of files that the script read, in the order of its text, or the
    /// error of reading it. A script that did not run, or whose text has errors of its syntax,
    /// read none.
    pub files: Vec<(Files, FilesTable)>,
}

/// Whether a data script loaded, and why it did not.
#[derive(Clone, Debug)]
pub enum ScriptStatus {
    Loaded(Box<LoadedScript>),
    /// The script ran and failed: its text, its analysis, or its lowering has errors, or it
    /// defines a name that another script also defines.
    Failed,
    /// The script did not run, since it reads names of the scripts `dependencies`, which did not
    /// load, by path.
    NotLoaded {
        dependencies: Vec<String>,
    },
    /// The script can't run, since it is in a cycle of scripts that read each other's names, or
    /// reads a script that is.
    Blocked,
}

/// A data script that loaded.
#[derive(Clone, Debug)]
pub struct LoadedScript {
    pub analysis: DataAnalysis,
    /// The tables, views, and schemas that the script registered in the session, in the order of
    /// its text.
    pub registered: Vec<Registered>,
    /// The schemas of `registered` that the session lacked before the script, which a reload
    /// of the script removes.
    created_schemas: Vec<String>,
}

/// A table of the session that a load changed, which the charts that read it show again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableChange {
    /// The path of the table, as its data script writes it.
    pub names: Vec<String>,
    pub change: Change,
}

/// What [`LoadedProject::reload_files`] changed after files changed on disk.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FilesReload {
    /// The tables of the session that changed, by path, which the charts that read them show
    /// again.
    pub tables: Vec<TableChange>,
    /// The chart scripts whose own tables of files read the files, by path, which show again
    /// with the tables that their next runs build.
    pub charts: Vec<String>,
}

/// How a table of the session changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// A table that the session lacked.
    Registered,
    /// A table that a data script registered again, with the rows and the files of the load.
    Replaced,
    /// A table that its data script no longer registers.
    Removed,
}

/// A read of a chart script of a table that a data script defines and that did not load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnloadedRead {
    /// The path of the table, as the chart writes it.
    pub names: Vec<String>,
    /// The range of the path in the text of the chart.
    pub range: TextRange,
    /// The path of the data script that defines the table, or the schema of the table.
    pub script: String,
    /// The error at the read, which names the script and says why it did not load.
    pub message: String,
}

/// Why a chart script fails.
#[derive(Debug, thiserror::Error)]
pub enum ChartError {
    /// The chart reads tables of data scripts that did not load: each such read, and the error
    /// of the run.
    #[error("{}", (.reads.iter()).map(|read| read.message.as_str()).collect::<Vec<_>>().join("\n"))]
    Unloaded {
        reads: Vec<UnloadedRead>,
        #[source]
        error: DataFusionError,
    },
    #[error(transparent)]
    Run(DataFusionError),
}

impl ScriptLoad {
    pub fn loaded(&self) -> Option<&LoadedScript> {
        match &self.status {
            ScriptStatus::Loaded(script) => Some(script),
            _ => None,
        }
    }
}

impl LoadedProject {
    /// Loads the project `files` into the session `ctx`, a new `SessionContext` or a session of
    /// the host with its own tables, which the scripts read as they read the project's: it
    /// registers avenger-sql's [`avenger_sql::register`] and the store of the project's file
    /// source in `ctx`, reads the data scripts and the modules, and loads every data script.
    ///
    /// A data script that fails is a result of the load, not an error: the error is a session
    /// whose query planner avenger-sql can't replace, or a file of the project that can't be
    /// read.
    pub async fn load(files: ProjectFiles, ctx: SessionContext) -> Result<Self, ProjectError> {
        avenger_sql::register(&ctx).map_err(ProjectError::Session)?;
        files.source().register(&ctx);
        let (texts, modules) = read(&files).await?;
        let graph = graph(&texts);
        let mut project = LoadedProject {
            files,
            ctx,
            texts,
            modules,
            graph,
            scripts: BTreeMap::new(),
            charts: Default::default(),
        };
        let all = project.texts.keys().cloned().collect();
        project.load_scripts(all).await;
        Ok(project)
    }

    pub fn files(&self) -> &ProjectFiles {
        &self.files
    }

    /// The session that holds the tables of the project.
    pub fn session(&self) -> &SessionContext {
        &self.ctx
    }

    /// The dependencies of the data scripts, and their errors.
    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    /// The result of each data script, in the order of the load: the scripts that can run in
    /// the order of [`Graph::order`], and then the blocked scripts, by path.
    pub fn scripts(&self) -> impl Iterator<Item = &ScriptLoad> {
        let order = self.graph.order().into_iter().chain(self.graph.blocked());
        order.filter_map(|path| self.scripts.get(path))
    }

    /// The result of the data script at `path`.
    pub fn script(&self, path: &str) -> Option<&ScriptLoad> {
        self.scripts.get(path)
    }

    /// The table of each set of files that the data scripts read in their last loads, or the
    /// error of reading it, by the scripts in the order of the load, as an editor's analysis
    /// of the scripts takes them.
    pub fn file_tables(&self) -> impl Iterator<Item = &(Files, FilesTable)> {
        self.scripts().flat_map(|script| &script.files)
    }

    /// The data scripts whose last loads read the file at `path`, relative to the project's
    /// root, in the order of the load, and then the chart scripts whose own tables of files read
    /// it in their last runs, by path: those with a URI that names the file, or a directory or a
    /// pattern that holds it, as [`file_urls`] resolves the URIs and their
    /// [`reads`](avenger_sql::data::FileUrl::reads) matches the files that the tables list. The
    /// match reads the URIs and not the store, so a file that a change removed
    /// matches as it did, and a new file matches a pattern or a directory before any load lists
    /// it. A script that failed since no file matched its URIs reads those files too, so a new
    /// file that matches loads it.
    pub fn scripts_reading(&self, path: &str) -> Vec<String> {
        let Ok(url) = self.files.source().url(path) else {
            return Vec::new();
        };
        let Ok(file) = ListingTableUrl::try_new(url, None) else {
            return Vec::new();
        };
        let base = self.files.source().base().as_str();
        let state = self.ctx.state();
        let ignore_subdirectory =
            (state.config_options().execution).listing_table_ignore_subdirectory;
        let reads = |files: &[(Files, FilesTable)]| {
            (files.iter()).any(|(files, _)| {
                (file_urls(files, base).into_iter().flatten()).any(|uri| {
                    uri.object_store() == file.object_store()
                        && uri.reads(file.prefix(), ignore_subdirectory)
                })
            })
        };
        let mut scripts: Vec<String> = (self.scripts())
            .filter(|script| reads(&script.files))
            .map(|script| script.path.clone())
            .collect();
        let charts = self.charts.lock().expect("the tables of the charts");
        scripts.extend(
            (charts.iter())
                .filter(|(_, chart)| reads(&chart.files))
                .map(|(path, _)| path.clone()),
        );
        scripts
    }

    /// Loads the data scripts that read the files at `paths`, relative to the project's root,
    /// again, as [`LoadedProject::scripts_reading`] finds them, and every script downstream of
    /// them, each once, after the files change on disk, and returns the tables of the session
    /// that changed, by path, and the charts whose own tables of files read them. The texts of
    /// the scripts are those of the last load. Each such chart reads its files again, and drops
    /// the tables that its last run built, which its next run builds again.
    pub async fn reload_files(&mut self, paths: &[&str]) -> FilesReload {
        let reading: BTreeSet<String> = (paths.iter())
            .flat_map(|path| self.scripts_reading(path))
            .collect();
        let (scripts, charts): (Vec<String>, Vec<String>) =
            (reading.into_iter()).partition(|path| self.scripts.contains_key(path));
        let state = self.ctx.state();
        let base = self.files.source().base().as_str();
        let tables = self.charts.get_mut().expect("the tables of the charts");
        for path in &charts {
            let Some(chart) = tables.get_mut(path) else {
                continue;
            };
            for (files, table) in &mut chart.files {
                *table = file_table(&state, files, base).await;
            }
            chart.built = None;
        }
        let mut set = BTreeSet::new();
        for script in &scripts {
            set.extend(affected(&self.graph, &self.graph, script));
        }
        FilesReload {
            tables: self.load_scripts(set).await,
            charts,
        }
    }

    /// The text of the data script at `path`, as the last load read it.
    pub fn text(&self, path: &str) -> Option<&str> {
        self.texts.get(path).map(String::as_str)
    }

    /// The text of the module at the module path `names`, matched without regard to case.
    pub fn module(&self, names: &[String]) -> Option<String> {
        (self.modules.iter())
            .find(|(path, _)| same_path(path, names))
            .map(|(_, text)| text.clone())
    }

    /// The rows of the last query of the chart script at `path`, whose text is `text`, which
    /// reads the tables of the session and its own tables of files, and imports the project's
    /// modules, as [`avenger_sql::sql_with_modules`] plans a script. An error of the script is an
    /// [`avenger_sql::ScriptError`], and a statement of files that fails, such as a `LOAD DATA`
    /// whose rows don't load, is an error at the statement.
    ///
    /// The chart's tables of files are its own: they register in no session. The project keeps
    /// them by `path`, and builds them again when the chart's statements of files change, or
    /// when [`LoadedProject::reload_files`] finds that a file that they read changed, so a run
    /// that changes only the chart's query reuses them, and a `LOAD DATA` reads its files once.
    ///
    /// When the run fails and the chart reads a table that a data script defines, or a table in
    /// a schema that a data script creates, and that script did not load, the error is
    /// [`ChartError::Unloaded`], with an error at each such read that names the script.
    pub async fn run_chart(&self, path: &str, text: &str) -> Result<DataFrame, ChartError> {
        let error = match self.chart_frame(path, text).await {
            Ok(frame) => return Ok(frame),
            Err(error) => error,
        };
        let root = avenger_sql_syntax::parse(text).syntax();
        let reads: Vec<UnloadedRead> = (references(&root).tables.into_iter())
            .filter_map(|reference| {
                let (script, kind, names) = self.unloaded_definer(&reference.names)?;
                Some(UnloadedRead {
                    message: self.unloaded_message(kind, &names, &script),
                    names: reference.names,
                    range: reference.range,
                    script,
                })
            })
            .collect();
        match reads.is_empty() {
            true => Err(ChartError::Run(error)),
            false => Err(ChartError::Unloaded { reads, error }),
        }
    }

    /// The chart script at `path`, whose text is `text`, as the script that its lifted calls
    /// plan, with the tables of the session, its own tables of files, and the project's modules,
    /// as [`avenger_sql::lift_with_modules`] prints a script.
    pub async fn lift(
        &self,
        path: &str,
        text: &str,
    ) -> Result<avenger_sql::analysis::Lifted, DataFusionError> {
        let (analysis, _) = self.chart_analysis(path, text).await?;
        Ok(analysis.lift(text).expect("a script without errors lifts"))
    }

    /// The rows of the chart script at `path`, as [`LoadedProject::run_chart`] runs it.
    async fn chart_frame(&self, path: &str, text: &str) -> Result<DataFrame, DataFusionError> {
        let (analysis, catalog) = self.chart_analysis(path, text).await?;
        if analysis.file_tables.is_empty() {
            return analysis.dataframe(&self.ctx).await;
        }
        let statements: Vec<String> = (analysis.file_tables.iter())
            .map(|definition| text[definition.range].to_string())
            .collect();
        let built = self
            .chart(path)
            .built
            .filter(|(texts, _)| *texts == statements);
        let tables = match built {
            Some((_, tables)) => tables,
            None => {
                let built = avenger_sql::data::ScriptTables::build(
                    &self.ctx,
                    &analysis.file_tables,
                    &catalog,
                )
                .await;
                let tables = built.map_err(|errors| {
                    let diagnostics = (errors.into_iter())
                        .map(|error| avenger_sql::analysis::Diagnostic {
                            range: analysis.file_tables[error.definition].range,
                            message: error.message,
                        })
                        .collect();
                    let error = avenger_sql::ScriptError::new(text, diagnostics);
                    DataFusionError::External(Box::new(error))
                })?;
                let mut charts = self.charts.lock().expect("the tables of the charts");
                let chart = charts.entry(path.to_string()).or_default();
                chart.built = Some((statements, tables.clone()));
                tables
            }
        };
        analysis.with_tables(&tables)?.dataframe(&self.ctx).await
    }

    /// The analysis of the chart script at `path`, whose text is `text`, without errors, and its
    /// catalog: the tables of the session that it reads, the table of each set of files of its
    /// own tables, which the chart keeps from its last run or reads, and the project's modules.
    async fn chart_analysis(
        &self,
        path: &str,
        text: &str,
    ) -> Result<(avenger_sql::analysis::Analysis, ProjectCatalog), DataFusionError> {
        let root = avenger_sql::script_tree(text)?;
        let base = SessionCatalog::load(&self.ctx, &references(&root)).await?;
        let types = base.type_options();
        let mut catalog = ProjectCatalog::new(Arc::new(base));
        let kept = self.chart(path).files;
        let state = self.ctx.state();
        let base_url = self.files.source().base().as_str();
        let mut files: Vec<(Files, FilesTable)> = Vec::new();
        for file_read in file_reads(&root, types) {
            if (files.iter()).any(|(read, _)| *read == file_read.files) {
                continue;
            }
            let table = match kept.iter().find(|(read, _)| *read == file_read.files) {
                Some((_, table)) => table.clone(),
                None => file_table(&state, &file_read.files, base_url).await,
            };
            files.push((file_read.files.clone(), table.clone()));
            catalog = catalog.with_files(file_read.files, table);
        }
        {
            let mut charts = self.charts.lock().expect("the tables of the charts");
            match files.is_empty() {
                true => {
                    charts.remove(path);
                }
                false => charts.entry(path.to_string()).or_default().files = files,
            }
        }
        for (names, text) in &self.modules {
            catalog = catalog.with_module(names.clone(), text.clone());
        }
        let analysis = avenger_sql::script_analysis(text, &root, &catalog)?;
        Ok((analysis, catalog))
    }

    /// The own tables of files of the chart script at `path`, as its last run left them.
    fn chart(&self, path: &str) -> ChartTables {
        let charts = self.charts.lock().expect("the tables of the charts");
        charts.get(path).cloned().unwrap_or_default()
    }

    /// Loads the data script at `path` again with the text `text`, and every script downstream
    /// of it, and returns the tables of the session that changed, by path.
    ///
    /// The scripts that reload are the script, the scripts that depend on it before the change
    /// and after it, and the scripts that define a name that one of them also defines, with
    /// their own downstream scripts in turn. The tables, views, and schemas that they registered
    /// before leave the session, including those that a script no longer defines, and the
    /// scripts load again in the order of the new graph. A table that a script registers again
    /// is [`Change::Replaced`], since its rows and its files are those of the reload.
    pub async fn reload(
        &mut self,
        path: &str,
        text: String,
    ) -> Result<Vec<TableChange>, ProjectError> {
        if !self.texts.contains_key(path) {
            return Err(ProjectError::NotDataScript(path.into()));
        }
        self.texts.insert(path.to_string(), text);
        let old = std::mem::replace(&mut self.graph, graph(&self.texts));
        let set = affected(&old, &self.graph, path);
        Ok(self.load_scripts(set).await)
    }

    /// Opens the project again from its file source, with its manifest, its files, and the
    /// texts of its data scripts and modules as they are now, and loads every data script
    /// again, in the same session. Every table, view, and schema of the project leaves the
    /// session before the load, the tables of the host stay, and the charts' own tables of files
    /// build again at their next runs.
    pub async fn reload_all(&mut self) -> Result<Vec<TableChange>, ProjectError> {
        let files = ProjectFiles::open(self.files.source().clone()).await?;
        (self.charts.get_mut().expect("the tables of the charts")).clear();
        let (texts, modules) = read(&files).await?;
        let mut set: BTreeSet<String> = self.texts.keys().cloned().collect();
        set.extend(texts.keys().cloned());
        self.files = files;
        self.graph = graph(&texts);
        self.texts = texts;
        self.modules = modules;
        Ok(self.load_scripts(set).await)
    }

    /// Removes from the session what the scripts of `set` registered, and loads those of them
    /// that the project still has in the order of the graph, and returns the tables that
    /// changed.
    async fn load_scripts(&mut self, set: BTreeSet<String>) -> Vec<TableChange> {
        let mut before: Vec<Vec<String>> = Vec::new();
        for path in &set {
            let Some(load) = self.scripts.remove(path) else {
                continue;
            };
            if let ScriptStatus::Loaded(script) = load.status {
                before.extend(self.deregister(&script));
            }
        }
        let order: Vec<String> = (self.graph.order().into_iter())
            .chain(self.graph.blocked())
            .filter(|path| set.contains(*path))
            .map(str::to_string)
            .collect();
        let mut after: Vec<Vec<String>> = Vec::new();
        for path in order {
            let load = self.load_script(&path).await;
            if let ScriptStatus::Loaded(script) = &load.status {
                after.extend(table_names(script));
            }
            self.scripts.insert(path, load);
        }
        changes(before, after)
    }

    /// Removes the tables and then the schemas that `script` registered from the session, and
    /// returns the paths of the tables.
    fn deregister(&self, script: &LoadedScript) -> Vec<Vec<String>> {
        let catalog = self.ctx.catalog(&self.default_catalog());
        let schema_name = |name: &str| avenger_sql::data::schema_name(&self.ctx, name);
        let names = table_names(script);
        for names in &names {
            let reference = match names.as_slice() {
                [name] => TableReference::bare(name.as_str()),
                [schema, name] => TableReference::partial(schema_name(schema), name.as_str()),
                _ => continue,
            };
            // A table that the session no longer has, such as one that the host removed, leaves
            // nothing to remove.
            let _ = self.ctx.deregister_table(reference);
        }
        if let Some(catalog) = &catalog {
            for schema in &script.created_schemas {
                let _ = catalog.deregister_schema(&schema_name(schema), true);
            }
        }
        names
    }

    /// Loads the data script at `path`, after the scripts that it depends on.
    async fn load_script(&self, path: &str) -> ScriptLoad {
        let mut diagnostics = self.graph_diagnostics(path);
        let mut files = Vec::new();
        let duplicate = (self.graph.errors().iter()).any(|error| match error {
            GraphError::Duplicate { first, second, .. } => {
                first.path == path || second.path == path
            }
            GraphError::Cycle { .. } => false,
        });
        let status = if self.graph.blocked().contains(&path) {
            // A script in a cycle has its error at its read of the next script, and a script
            // that reads a script of a cycle has its errors at its reads of blocked scripts.
            if diagnostics.is_empty() {
                diagnostics.extend(self.dependency_diagnostics(path));
            }
            ScriptStatus::Blocked
        } else if duplicate {
            ScriptStatus::Failed
        } else {
            let unloaded: Vec<String> = (self.graph.dependencies(path).into_iter())
                .filter(|dependency| {
                    (self.scripts.get(*dependency)).is_none_or(|load| load.loaded().is_none())
                })
                .map(str::to_string)
                .collect();
            if unloaded.is_empty() {
                match self.run_script(path, &mut files).await {
                    Ok(script) => ScriptStatus::Loaded(Box::new(script)),
                    Err(errors) => {
                        diagnostics.extend(errors);
                        ScriptStatus::Failed
                    }
                }
            } else {
                diagnostics.extend(self.dependency_diagnostics(path));
                ScriptStatus::NotLoaded {
                    dependencies: unloaded,
                }
            }
        };
        diagnostics.sort_by_key(|diagnostic| diagnostic.range.start());
        ScriptLoad {
            path: path.to_string(),
            status,
            diagnostics,
            files,
        }
    }

    /// Parses, analyzes, and lowers the data script at `path`, whose dependencies loaded, and
    /// adds the table of each set of files that it reads to `files`.
    async fn run_script(
        &self,
        path: &str,
        files: &mut Vec<(Files, FilesTable)>,
    ) -> Result<LoadedScript, Vec<ProjectDiagnostic>> {
        let text = &self.texts[path];
        let diagnostic =
            |range: TextRange, message: String, help: Option<String>| ProjectDiagnostic {
                path: path.to_string(),
                range,
                message,
                help,
            };
        let parse = avenger_sql_syntax::parse_data(text);
        if !parse.ok() {
            return Err((parse.diagnostics().iter())
                .map(|error| diagnostic(error.range, error.message.clone(), error.help.clone()))
                .collect());
        }
        let root = parse.syntax();
        let start = TextRange::empty(Offset::from(0));
        let base =
            (SessionCatalog::load(&self.ctx, &references(&root)).await).map_err(|error| {
                let message = format!("The tables that the script reads don't load: {error}");
                vec![diagnostic(start, message, None)]
            })?;
        let types = base.type_options();
        let mut catalog = ProjectCatalog::new(Arc::new(base));
        // The definitions of the scripts that it reads, so that it finds their schemas and the
        // places of their names.
        for dependency in self.graph.dependencies(path) {
            let Some(loaded) = self.scripts.get(dependency).and_then(ScriptLoad::loaded) else {
                continue;
            };
            for registered in &loaded.registered {
                let definition = &loaded.analysis.definitions[registered.definition];
                let table = match &registered.registration {
                    Registration::Table(table) => Some(table.clone()),
                    Registration::Schema(_) => None,
                };
                catalog = catalog.with_definition(dependency, definition, table);
            }
        }
        // The analysis places an error of reading files at the URIs of the read.
        let state = self.ctx.state();
        let base_url = self.files.source().base().as_str();
        for file_read in file_reads(&root, types) {
            if !(files.iter()).any(|(read, _)| *read == file_read.files) {
                let table = file_table(&state, &file_read.files, base_url).await;
                files.push((file_read.files.clone(), table.clone()));
                catalog = catalog.with_files(file_read.files, table);
            }
        }
        for (names, text) in &self.modules {
            catalog = catalog.with_module(names.clone(), text.clone());
        }
        let analysis = analyze_data(&root, &catalog, library(), &Options::default());
        if !analysis.diagnostics.is_empty() {
            return Err((analysis.diagnostics.iter())
                .map(|error| diagnostic(error.range, error.message.clone(), None))
                .collect());
        }
        let schemas_before = self.schema_names();
        match lower(&self.ctx, &analysis, &catalog).await {
            Ok(registered) => {
                let created_schemas = (registered.iter())
                    .filter(|registered| matches!(registered.registration, Registration::Schema(_)))
                    .map(|registered| analysis.definitions[registered.definition].names[0].clone())
                    .filter(|name| {
                        avenger_sql::catalog::find(schemas_before.clone(), name).is_none()
                    })
                    .collect();
                Ok(LoadedScript {
                    analysis,
                    registered,
                    created_schemas,
                })
            }
            Err(errors) => Err((errors.into_iter())
                .map(|error| {
                    let range = analysis.definitions[error.definition].range;
                    diagnostic(range, error.message, None)
                })
                .collect()),
        }
    }

    /// The errors of the graph at the script at `path`, one at each of their places in it.
    fn graph_diagnostics(&self, path: &str) -> Vec<ProjectDiagnostic> {
        let mut diagnostics = Vec::new();
        for error in self.graph.errors() {
            let message = error.message();
            for place in error.places() {
                if place.path == path {
                    diagnostics.push(ProjectDiagnostic {
                        path: place.path,
                        range: place.range,
                        message: message.clone(),
                        help: None,
                    });
                }
            }
        }
        diagnostics
    }

    /// An error at the first read of the script at `path` of a name of each script that it
    /// depends on and that did not load, which names that script, by the path of the script.
    fn dependency_diagnostics(&self, path: &str) -> Vec<ProjectDiagnostic> {
        let mut first: BTreeMap<&str, &Edge> = BTreeMap::new();
        for edge in self.graph.edges().iter().filter(|edge| edge.script == path) {
            let loaded =
                (self.scripts.get(&edge.dependency)).is_some_and(|load| load.loaded().is_some());
            if loaded {
                continue;
            }
            // The edges of the script are in the order of its reads.
            first.entry(&edge.dependency).or_insert(edge);
        }
        (first.into_values())
            .map(|edge| ProjectDiagnostic {
                path: path.to_string(),
                range: edge.read.range,
                message: self.unloaded_message(edge.read.kind, &edge.read.names, &edge.dependency),
                help: None,
            })
            .collect()
    }

    /// The data script that defines the table at `names`, or creates its schema, when that
    /// script did not load, with the kind and the path of the name that it defines.
    fn unloaded_definer(&self, names: &[String]) -> Option<(String, NameKind, Vec<String>)> {
        let definer = |kind: NameKind, names: &[String]| {
            (self.graph.paths()).find(|path| {
                (self.graph.names(path)).is_some_and(|script| {
                    (script.definitions.iter())
                        .any(|name| name.kind == kind && same_path(&name.names, names))
                })
            })
        };
        let (path, kind, names) = match definer(NameKind::Table, names) {
            Some(path) => (path, NameKind::Table, names),
            None => match names {
                [schema, _] => (
                    definer(NameKind::Schema, &names[..1])?,
                    NameKind::Schema,
                    std::slice::from_ref(schema),
                ),
                _ => return None,
            },
        };
        let loaded = (self.scripts.get(path)).is_some_and(|load| load.loaded().is_some());
        (!loaded).then(|| (path.to_string(), kind, names.to_vec()))
    }

    /// The error of a read of the name `names` of the kind `kind`, which the data script at
    /// `script` defines and which did not load.
    fn unloaded_message(&self, kind: NameKind, names: &[String], script: &str) -> String {
        // A blocked script can load after its readers, which come first by path.
        let blocked = self.graph.blocked().contains(&script);
        let why = match self.scripts.get(script).map(|load| &load.status) {
            _ if blocked => {
                "which can't run, since it is in a cycle of data scripts that read each other's tables, or reads one".to_string()
            }
            Some(ScriptStatus::NotLoaded { .. }) => {
                let causes: Vec<String> = (self.causes(script).into_iter())
                    .map(|cause| match self.graph.blocked().contains(&cause.as_str()) {
                        true => format!("{cause} is in a cycle"),
                        false => format!("{cause} failed to load"),
                    })
                    .collect();
                format!("which did not load, since {}", causes.join(" and "))
            }
            _ => "which failed to load".to_string(),
        };
        unavailable_read(kind, names, script, &why)
    }

    /// The scripts that failed or are blocked, upstream of the script at `path` that did not
    /// load, by path.
    fn causes(&self, path: &str) -> BTreeSet<String> {
        let mut causes = BTreeSet::new();
        let mut pending = vec![path.to_string()];
        let mut seen = BTreeSet::new();
        while let Some(script) = pending.pop() {
            if !seen.insert(script.clone()) {
                continue;
            }
            match self.scripts.get(&script).map(|load| &load.status) {
                Some(ScriptStatus::NotLoaded { dependencies }) => {
                    pending.extend(dependencies.iter().cloned());
                }
                Some(ScriptStatus::Loaded(_)) => {}
                _ => {
                    causes.insert(script);
                }
            }
        }
        causes
    }

    fn default_catalog(&self) -> String {
        let state = self.ctx.state();
        state.config_options().catalog.default_catalog.clone()
    }

    /// The schemas of the session's default catalog.
    fn schema_names(&self) -> Vec<String> {
        (self.ctx.catalog(&self.default_catalog()))
            .map_or_else(Vec::new, |catalog| catalog.schema_names())
    }
}

/// The texts of the data scripts of `files`, by path, and of its modules, by module path.
async fn read(
    files: &ProjectFiles,
) -> Result<(BTreeMap<String, String>, Vec<(Vec<String>, String)>), ProjectError> {
    let source = files.source();
    let mut texts = BTreeMap::new();
    for path in files.data_scripts() {
        texts.insert(path.clone(), source.read_text(path).await?);
    }
    let mut modules = Vec::new();
    for module in files.modules() {
        modules.push((module.names.clone(), source.read_text(&module.path).await?));
    }
    Ok((texts, modules))
}

/// The graph of the data scripts of `texts`.
fn graph(texts: &BTreeMap<String, String>) -> Graph {
    Graph::new(
        (texts.iter())
            .map(|(path, text)| (path.clone(), avenger_sql_syntax::parse_data(text).syntax())),
    )
}

/// The scripts that a change of the script at `path` reloads: the script, and in turn the
/// scripts downstream of each, and the scripts that define a name that each also defines, in the
/// graph before the change, `old`, and after it, `new`.
fn affected(old: &Graph, new: &Graph, path: &str) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    let mut pending = vec![path.to_string()];
    while let Some(script) = pending.pop() {
        if !set.insert(script.clone()) {
            continue;
        }
        for graph in [old, new] {
            pending.extend(graph.downstream(&script).into_iter().map(str::to_string));
            for error in graph.errors() {
                if let GraphError::Duplicate { first, second, .. } = error {
                    if first.path == script {
                        pending.push(second.path.clone());
                    } else if second.path == script {
                        pending.push(first.path.clone());
                    }
                }
            }
        }
    }
    set
}

/// The paths of the tables, not the schemas, that `script` registered.
fn table_names(script: &LoadedScript) -> Vec<Vec<String>> {
    (script.registered.iter())
        .filter(|registered| matches!(registered.registration, Registration::Table(_)))
        .map(|registered| {
            script.analysis.definitions[registered.definition]
                .names
                .clone()
        })
        .collect()
}

/// The changes from the tables `before` to the tables `after`, in the order of the paths.
fn changes(before: Vec<Vec<String>>, after: Vec<Vec<String>>) -> Vec<TableChange> {
    let mut changes: Vec<TableChange> = Vec::new();
    for names in &after {
        let change = match before.iter().any(|old| same_path(old, names)) {
            true => Change::Replaced,
            false => Change::Registered,
        };
        if !(changes.iter()).any(|changed| same_path(&changed.names, names)) {
            changes.push(TableChange {
                names: names.clone(),
                change,
            });
        }
    }
    for names in before {
        if !(changes.iter()).any(|changed| same_path(&changed.names, &names)) {
            changes.push(TableChange {
                names,
                change: Change::Removed,
            });
        }
    }
    changes.sort_by_key(|changed| lowercase(&changed.names));
    changes
}

fn lowercase(names: &[String]) -> Vec<String> {
    names.iter().map(|name| name.to_ascii_lowercase()).collect()
}

/// Whether the paths `a` and `b` name the same table or module, without regard to case, as in
/// GoogleSQL.
fn same_path(a: &[String], b: &[String]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.eq_ignore_ascii_case(b))
}
