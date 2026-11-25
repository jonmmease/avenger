//! Loading a project into a DataFusion session: its data scripts in the order of their
//! dependencies, its chart scripts against the tables that they register and with their own
//! tables of files, and the reloads of scripts that change.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use avenger_sql::{
    analysis::Options,
    analyze_data,
    catalog::{references, Catalog, FilesTable, SessionCatalog},
    data::{
        file_reads, file_table, file_urls, lower, DataAnalysis, Files, Registered, Registration,
        ScriptTables,
    },
    library::Library,
    project::{
        table_reads, unavailable_read, Edge, Graph, GraphError, NameKind, ProjectCatalog, Unloaded,
    },
};
use avenger_sql_syntax::{Offset, TextRange};
use datafusion::{
    common::{DataFusionError, ScalarValue, TableReference},
    dataframe::DataFrame,
    prelude::SessionContext,
};

use avenger_sql::analysis::Overrides;

use crate::project::{ProjectError, ProjectFiles};
use crate::query::ChartQuery;

/// A project loaded into a DataFusion session: the tables that its data scripts register, the
/// result of each data script, and the modules that its scripts import.
///
/// [`LoadedProject::load`] runs the data scripts in the order of their [`Graph`], each after the
/// scripts whose tables, views, and schemas it reads. Each script reads the schemas of its
/// files, is analyzed with a [`ProjectCatalog`] of the session's tables, the definitions of the
/// scripts that it reads, and the project's modules that it imports, and is lowered into the
/// session. A script that fails registers nothing, and a script that reads a name of a script
/// that did not load does not load either. [`LoadedProject::run_chart`] runs a chart script
/// against the session, with the chart's own tables of files, which it keeps by the chart's
/// path, and [`LoadedProject::reload`] loads a changed data script or module, and the scripts
/// downstream of it, again.
///
/// A `LoadedProject` is `Send` and `Sync`, and the futures of its loads, reloads, and chart runs
/// are not `Send`: they hold the syntax trees of scripts, which are not `Send`, across their
/// awaits, and [`lower`] takes a catalog as a `&dyn` reference that is not `Sync`. A host awaits
/// them on one thread, as a web build does.
pub struct LoadedProject {
    files: ProjectFiles,
    ctx: SessionContext,
    /// The text of each data script, by path.
    texts: BTreeMap<String, String>,
    /// The text of each module, by path, as the last load or reload of the module read it.
    modules: BTreeMap<String, String>,
    graph: Graph,
    /// The result of each data script, by path.
    scripts: BTreeMap<String, ScriptLoad>,
    /// The own tables of files of each chart script that ran, by path.
    charts: std::sync::Mutex<BTreeMap<String, ChartTables>>,
    /// The results of the chart queries' runs, which later runs reuse.
    store: avenger_query_graph::ResultStore,
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

/// The scripts that read a file or a directory, as [`LoadedProject::scripts_reading`] finds them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileReaders {
    /// The data scripts whose last loads read it, by path, in the order of the load.
    pub data_scripts: Vec<String>,
    /// The chart scripts whose own tables of files read it in their last runs, by path.
    pub charts: Vec<String>,
}

impl FileReaders {
    /// Whether no script reads it.
    pub fn is_empty(&self) -> bool {
        self.data_scripts.is_empty() && self.charts.is_empty()
    }
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
    /// of the run. Its message is the error of each read, and then the run's, which can have
    /// other errors of the chart.
    #[error("{}\n{}", (.reads.iter()).map(|read| read.message.as_str()).collect::<Vec<_>>().join("\n"), run_message(.error))]
    Unloaded {
        reads: Vec<UnloadedRead>,
        #[source]
        error: DataFusionError,
    },
    #[error(transparent)]
    Run(DataFusionError),
}

/// The message of the error of a chart's run, as [`ChartError::Unloaded`] shows it: the errors
/// of a script without DataFusion's prefix of an external error.
fn run_message(error: &DataFusionError) -> String {
    match error.find_root() {
        DataFusionError::External(external) => external.to_string(),
        error => error.to_string(),
    }
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
        let graph = graph(&files, &texts, &modules, &default_schema(&ctx));
        let mut project = LoadedProject {
            files,
            ctx,
            texts,
            modules,
            graph,
            scripts: BTreeMap::new(),
            charts: Default::default(),
            store: crate::query::store(),
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
    /// error of reading it, by the scripts in the order of the load, and then those that the
    /// chart scripts' own tables of files read in their last runs, by path, as an editor's
    /// analysis of the scripts takes them.
    pub fn file_tables(&self) -> Vec<(Files, FilesTable)> {
        let mut tables: Vec<(Files, FilesTable)> = (self.scripts())
            .flat_map(|script| script.files.iter().cloned())
            .collect();
        let charts = self.charts.lock().expect("the tables of the charts");
        tables.extend((charts.values()).flat_map(|chart| chart.files.iter().cloned()));
        tables
    }

    /// The data scripts whose last loads read the file or the directory at `path`, relative to
    /// the project's root, in the order of the load, and the chart scripts whose own tables of
    /// files read it in their last runs, by path, as [`changed_by`](avenger_sql::data::FileUrl::changed_by) matches it with the
    /// URIs that [`file_urls`] resolves: those with a URI that names the file, or a directory or
    /// a pattern that holds it, as the tables match the files that they list, and those with a
    /// URI under the directory, which a move or a removal of the directory changes. The match
    /// reads the URIs and not the store, so a file that a change removed matches as it did, and
    /// a new file matches a pattern or a directory before any load lists it. A script that
    /// failed since no file matched its URIs reads those files too, so a new file that matches
    /// loads it. The match compares the spellings of the paths: a URI that names a file with
    /// other cases of its letters, or through a symbolic link, does not match the file's path.
    pub fn scripts_reading(&self, path: &str) -> FileReaders {
        let Ok(changed) = self.files.source().url(path) else {
            return FileReaders::default();
        };
        let base = self.files.source().base().as_str();
        let state = self.ctx.state();
        let ignore_subdirectory =
            (state.config_options().execution).listing_table_ignore_subdirectory;
        let reads = |files: &[(Files, FilesTable)]| {
            (files.iter()).any(|(files, _)| {
                (file_urls(files, base).into_iter().flatten())
                    .any(|uri| uri.changed_by(&changed, ignore_subdirectory))
            })
        };
        let data_scripts = (self.scripts())
            .filter(|script| reads(&script.files))
            .map(|script| script.path.clone())
            .collect();
        let charts = self.charts.lock().expect("the tables of the charts");
        let charts = (charts.iter())
            .filter(|(_, chart)| reads(&chart.files))
            .map(|(path, _)| path.clone())
            .collect();
        FileReaders {
            data_scripts,
            charts,
        }
    }

    /// Loads the data scripts that read the files or the directories at `paths`, relative to
    /// the project's root, again, as [`LoadedProject::scripts_reading`] finds them, and every
    /// script downstream of them, each once, after they change on disk, and returns the tables
    /// of the session that changed, by path, and the charts whose own tables of files read
    /// them. The texts of the scripts are those of the last load. Each such chart drops the
    /// tables of files that its last run read and built, which its next run reads and builds
    /// again.
    pub async fn reload_files(&mut self, paths: &[&str]) -> FilesReload {
        let mut scripts = BTreeSet::new();
        let mut charts = BTreeSet::new();
        for path in paths {
            let readers = self.scripts_reading(path);
            scripts.extend(readers.data_scripts);
            charts.extend(readers.charts);
        }
        let tables = self.charts.get_mut().expect("the tables of the charts");
        for path in &charts {
            tables.remove(path);
        }
        let mut set = BTreeSet::new();
        for script in &scripts {
            set.extend(affected(&self.graph, &self.graph, script));
        }
        FilesReload {
            tables: self.load_scripts(set).await,
            charts: charts.into_iter().collect(),
        }
    }

    /// The text of the data script at `path`, as the last load read it.
    pub fn text(&self, path: &str) -> Option<&str> {
        self.texts.get(path).map(String::as_str)
    }

    /// The last query of the chart script at `path`, whose text is `text`, which reads the
    /// tables of the session and its own tables of files, and imports the project's modules: it
    /// analyzes the chart with a catalog of the tables that it reads, its tables of files, and
    /// the modules that it imports, plans its statements, and evaluates its variables. The
    /// [`ChartQuery`] gives the query's frame, unrun, and collects its rows through the project's
    /// store of results, which keeps them for later runs of the project's charts. An error of the script
    /// is an [`avenger_sql::ScriptError`], and a statement of files that fails, such as a
    /// `LOAD DATA` whose rows don't load, is an error at the statement.
    ///
    /// The chart's tables of files are its own: they register in no session. The project keeps
    /// them by `path`, and builds them again when the chart's statements of files change, or
    /// when [`LoadedProject::reload_files`] finds that a file that they read changed, so a run
    /// that changes only the chart's query reuses them, and a `LOAD DATA` reads its files once.
    ///
    /// When the run fails and the chart reads a table that a data script defines, or a table in
    /// a schema that a data script creates, and that script did not load, the error is
    /// [`ChartError::Unloaded`], with an error at each such read that names the script, and the
    /// error of the run. A read of a name of a pipe `WITH` reads the `WITH` query, and is not
    /// such a read.
    pub async fn run_chart(&self, path: &str, text: &str) -> Result<ChartQuery, ChartError> {
        self.run_chart_with(path, text, &Overrides::new()).await
    }

    /// The query of the chart script at `path`, as [`LoadedProject::run_chart`] plans it, with
    /// the values of `overrides` in place of the values of the variables that they name, as
    /// [`avenger_sql::analysis::Analysis::planned_query`] binds them.
    pub async fn run_chart_with(
        &self,
        path: &str,
        text: &str,
        overrides: &Overrides,
    ) -> Result<ChartQuery, ChartError> {
        let error = match self.chart_query(path, text, overrides).await {
            Ok(query) => return Ok(query),
            Err(error) => error,
        };
        let root = avenger_sql_syntax::parse(text).syntax();
        let reads: Vec<UnloadedRead> = (table_reads(&root).into_iter())
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
    /// as [`LoadedProject::run_chart`] analyzes it and [`avenger_sql::analysis::Analysis::lift`]
    /// prints it.
    pub async fn lift(
        &self,
        path: &str,
        text: &str,
    ) -> Result<avenger_sql::analysis::Lifted, DataFusionError> {
        let (analysis, _) = self.chart_analysis(path, text).await?;
        Ok(analysis.lift(text).expect("a script without errors lifts"))
    }

    /// The query of the chart script at `path`, as [`LoadedProject::run_chart_with`] plans it.
    async fn chart_query(
        &self,
        path: &str,
        text: &str,
        overrides: &Overrides,
    ) -> Result<ChartQuery, DataFusionError> {
        let analysis = self.ready_chart(path, text).await?;
        let query = analysis.planned_query(&self.ctx, overrides).await?;
        ChartQuery::new(query, &self.ctx, &self.store).await
    }

    /// The variables of the chart script at `path`, whose text is `text`, and their values as its
    /// last query reads them, as [`avenger_sql::analysis::Analysis::variables`] evaluates them,
    /// with the tables that [`LoadedProject::run_chart`] gives the script.
    pub async fn chart_variables(
        &self,
        path: &str,
        text: &str,
        overrides: &Overrides,
    ) -> Result<Vec<(String, ScalarValue)>, DataFusionError> {
        let analysis = self.ready_chart(path, text).await?;
        analysis.variables(&self.ctx, overrides).await
    }

    /// The analysis of the chart script at `path`, whose text is `text`, ready to run: with the
    /// tables of its statements of files, built again only when those statements change.
    async fn ready_chart(
        &self,
        path: &str,
        text: &str,
    ) -> Result<avenger_sql::analysis::Analysis, DataFusionError> {
        let (analysis, catalog) = self.chart_analysis(path, text).await?;
        if analysis.file_tables.is_empty() {
            return Ok(analysis);
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
                            code: None,
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
        analysis.with_tables(&tables)
    }

    /// The analysis of the chart script at `path`, whose text is `text`, without errors, and its
    /// catalog, as [`LoadedProject::script_catalog`] gives it, with the table of each set of
    /// files of its own tables that the chart keeps from its last run, or reads.
    async fn chart_analysis(
        &self,
        path: &str,
        text: &str,
    ) -> Result<(avenger_sql::analysis::Analysis, ProjectCatalog), DataFusionError> {
        let root = avenger_sql::script_tree(text)?;
        let kept = self.chart(path).files;
        let (catalog, files) = self.script_catalog(&root, &kept).await?;
        {
            let mut charts = self.charts.lock().expect("the tables of the charts");
            match files.is_empty() {
                true => {
                    charts.remove(path);
                }
                false => charts.entry(path.to_string()).or_default().files = files,
            }
        }
        let analysis = avenger_sql::script_analysis(text, &root, &catalog)?;
        Ok((analysis, catalog))
    }

    /// The catalog of the script `root`, a data script or a chart script, and the table of each
    /// set of files that it reads, in the order of its text: the tables of the session that it
    /// reads, the project's modules that it imports, directly or through other modules, and the
    /// tables of its files, each the table of `kept`, an earlier read of the same files, or a
    /// read of the files now. A read of `kept` that failed, such as of a file that did not exist
    /// yet, reads again.
    async fn script_catalog(
        &self,
        root: &avenger_sql_syntax::SyntaxNode,
        kept: &[(Files, FilesTable)],
    ) -> Result<(ProjectCatalog, Vec<(Files, FilesTable)>), DataFusionError> {
        let references = references(root);
        let module = |names: &[String]| module_text(&self.files, &self.modules, names);
        let base = (SessionCatalog::load(&self.ctx, &references).await?)
            .with_modules(&references, &module);
        let types = base.type_options();
        let mut catalog = ProjectCatalog::new(Arc::new(base));
        let state = self.ctx.state();
        let base_url = self.files.source().base().as_str();
        let mut files: Vec<(Files, FilesTable)> = Vec::new();
        for file_read in file_reads(root, types) {
            if (files.iter()).any(|(read, _)| *read == file_read.files) {
                continue;
            }
            let kept =
                (kept.iter()).find(|(read, table)| *read == file_read.files && table.is_ok());
            let table = match kept {
                Some((_, table)) => table.clone(),
                None => file_table(&state, &file_read.files, base_url).await,
            };
            files.push((file_read.files.clone(), table.clone()));
            catalog = catalog.with_files(file_read.files, table);
        }
        Ok((catalog, files))
    }

    /// The own tables of files of the chart script at `path`, as its last run left them.
    fn chart(&self, path: &str) -> ChartTables {
        let charts = self.charts.lock().expect("the tables of the charts");
        charts.get(path).cloned().unwrap_or_default()
    }

    /// Loads the data script or the module at `path` again with the text `text`, and every
    /// script downstream of it, and returns the tables of the session that changed, by path.
    ///
    /// The scripts that reload are the script, or the scripts that import the module, directly
    /// or through other modules, before the change and after it, the scripts that depend on
    /// them before the change and after it, and the scripts that define a name that one of them
    /// also defines, with their own downstream scripts in turn. The tables, views, and schemas
    /// that they registered before leave the session, including those that a script no longer
    /// defines, and the scripts load again in the order of the new graph. A table that a script
    /// registers again is [`Change::Replaced`], since its rows and its files are those of the
    /// reload.
    pub async fn reload(
        &mut self,
        path: &str,
        text: String,
    ) -> Result<Vec<TableChange>, ProjectError> {
        let module = (self.files.modules().iter())
            .find(|module| module.path == path)
            .map(|module| module.names.clone());
        let mut changed: Vec<String> = Vec::new();
        if let Some(script) = self.texts.get_mut(path) {
            *script = text;
            changed.push(path.to_string());
        } else if let Some(names) = &module {
            changed.extend(self.graph.importers(names).into_iter().map(str::to_string));
            self.modules.insert(path.to_string(), text);
        } else {
            return Err(ProjectError::NotScript(path.into()));
        }
        let new = graph(
            &self.files,
            &self.texts,
            &self.modules,
            &default_schema(&self.ctx),
        );
        let old = std::mem::replace(&mut self.graph, new);
        if let Some(names) = &module {
            changed.extend(self.graph.importers(names).into_iter().map(str::to_string));
        }
        let mut set = BTreeSet::new();
        for script in &changed {
            set.extend(affected(&old, &self.graph, script));
        }
        Ok(self.load_scripts(set).await)
    }

    /// Opens the project again from its file source, with its manifest, its files, and the
    /// texts of its data scripts and modules as they are now, and loads every data script
    /// again, in the same session. Every table, view, and schema of the project leaves the
    /// session before the load, and a table of the host that a script's table replaced comes
    /// back, so the tables of the host stay. The charts' own tables of files build again at
    /// their next runs.
    pub async fn reload_all(&mut self) -> Result<Vec<TableChange>, ProjectError> {
        let files = ProjectFiles::open(self.files.source().clone()).await?;
        (self.charts.get_mut().expect("the tables of the charts")).clear();
        let (texts, modules) = read(&files).await?;
        let mut set: BTreeSet<String> = self.texts.keys().cloned().collect();
        set.extend(texts.keys().cloned());
        self.graph = graph(&files, &texts, &modules, &default_schema(&self.ctx));
        self.files = files;
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
        changes(before, after, &default_schema(&self.ctx))
    }

    /// The number of rows of the table or the view of the session at the path `names`, as a data
    /// script writes it, which runs the view's query or reads the table's files.
    pub async fn row_count(&self, names: &[String]) -> Result<usize, DataFusionError> {
        let reference = match names {
            [name] => TableReference::bare(name.as_str()),
            [schema, name] => TableReference::partial(
                avenger_sql::data::schema_name(&self.ctx, schema),
                name.as_str(),
            ),
            _ => {
                return Err(DataFusionError::Plan(format!(
                    "no table `{}`",
                    names.join(".")
                )))
            }
        };
        self.ctx.table(reference).await?.count().await
    }

    /// Removes the tables and then the schemas that `script` registered from the session, and
    /// registers the tables that its tables replaced, such as tables of the host, again, and
    /// returns the paths of the script's tables.
    fn deregister(&self, script: &LoadedScript) -> Vec<Vec<String>> {
        let catalog = self.ctx.catalog(&self.default_catalog());
        let schema_name = |name: &str| avenger_sql::data::schema_name(&self.ctx, name);
        let names = table_names(script);
        for registered in &script.registered {
            let Registration::Table(_) = registered.registration else {
                continue;
            };
            let names = &script.analysis.definitions[registered.definition].names;
            let reference = |name: &str| match names.as_slice() {
                [_] => Some(TableReference::bare(name)),
                [schema, _] => Some(TableReference::partial(schema_name(schema), name)),
                _ => None,
            };
            let Some(table) = reference(names.last().expect("a table has a name")) else {
                continue;
            };
            // A table that the session no longer has, such as one that the host removed, leaves
            // nothing to remove.
            let _ = self.ctx.deregister_table(table);
            for (name, provider) in &registered.replaced {
                if let Some(table) = reference(name) {
                    let _ = self.ctx.register_table(table, provider.clone());
                }
            }
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
            let in_cycle = (self.graph.errors().iter()).any(|error| {
                matches!(error, GraphError::Cycle { .. })
                    && (error.places().iter()).any(|place| place.path == path)
            });
            if !in_cycle {
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
        let (analysis, catalog) = self.analyze_script(path, &self.texts[path], files).await?;
        let diagnostic = |range: TextRange, message: String| ProjectDiagnostic {
            path: path.to_string(),
            range,
            message,
            help: None,
        };
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
                    diagnostic(analysis.definitions[error.definition].range, error.message)
                })
                .collect()),
        }
    }

    /// The table of the data script at `path` after its statement that holds the offset `at` of
    /// its text: a step of the script, such as a temporary table, or a table that a later
    /// statement of the script loads into again, which the session doesn't hold as the statement
    /// left it. The script's statements through that one build their tables again, and register
    /// nothing, so the session keeps the script's final tables. The error is the script's own,
    /// at its places in the script.
    pub async fn step(&self, path: &str, at: usize) -> Result<DataFrame, Vec<ProjectDiagnostic>> {
        self.step_text(path, &self.texts[path], at).await
    }

    /// The table of the data script at `path`, as [`LoadedProject::step`] builds it, with the
    /// text `text` in place of the script's own, as a debugger runs a statement whose query it
    /// cut short.
    pub async fn step_text(
        &self,
        path: &str,
        text: &str,
        at: usize,
    ) -> Result<DataFrame, Vec<ProjectDiagnostic>> {
        let (analysis, catalog) = self.analyze_script(path, text, &mut Vec::new()).await?;
        let failure = |range: TextRange, message: String| {
            vec![ProjectDiagnostic {
                path: path.to_string(),
                range,
                message,
                help: None,
            }]
        };
        let start = TextRange::empty(Offset::from(0));
        let offset = Offset::from(at as u32);
        let Some(index) = (analysis.definitions.iter())
            .position(|definition| definition.range.contains_inclusive(offset))
        else {
            return Err(failure(
                start,
                "no statement of the script makes a table there".into(),
            ));
        };
        let definitions = &analysis.definitions[..=index];
        let tables = ScriptTables::build(&self.ctx, definitions, &catalog)
            .await
            .map_err(|errors| {
                (errors.into_iter())
                    .flat_map(|error| failure(definitions[error.definition].range, error.message))
                    .collect::<Vec<_>>()
            })?;
        let range = definitions[index].range;
        let Some(table) = tables.table(index) else {
            return Err(failure(range, "the statement makes no table".into()));
        };
        (self.ctx.read_table(table)).map_err(|error| failure(range, error.to_string()))
    }

    /// The analysis of the data script at `path`, without errors, with its catalog, which holds
    /// the definitions of the scripts that it reads. The table of each set of files that it
    /// reads goes to `files`, also when its analysis fails.
    async fn analyze_script(
        &self,
        path: &str,
        text: &str,
        files: &mut Vec<(Files, FilesTable)>,
    ) -> Result<(DataAnalysis, ProjectCatalog), Vec<ProjectDiagnostic>> {
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
        // The analysis places an error of reading files at the URIs of the read.
        let (mut catalog, read) = (self.script_catalog(&root, &[]).await).map_err(|error| {
            let message = format!("The tables that the script reads don't load: {error}");
            vec![diagnostic(start, message, None)]
        })?;
        *files = read;
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
        let analysis = analyze_data(&root, &catalog, Library::shared(), &Options::default());
        if !analysis.diagnostics.is_empty() {
            return Err((analysis.diagnostics.iter())
                .map(|error| diagnostic(error.range, error.message.clone(), None))
                .collect());
        }
        Ok((analysis, catalog))
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
    /// script did not load, with the kind and the path of the name that it defines. A table of
    /// one name is in the session's default schema.
    fn unloaded_definer(&self, names: &[String]) -> Option<(String, NameKind, Vec<String>)> {
        let default_schema = default_schema(&self.ctx);
        let definer = |kind: NameKind, names: &[String]| {
            (self.graph.paths()).find(|path| {
                (self.graph.names(path)).is_some_and(|script| {
                    (script.definitions.iter()).any(|name| {
                        name.kind == kind && kind.same(&name.names, names, &default_schema)
                    })
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
        let blocked = |path: &str| self.graph.blocked().contains(&path);
        let unloaded = match self.scripts.get(script).map(|load| &load.status) {
            _ if blocked(script) => Unloaded::Blocked,
            Some(ScriptStatus::NotLoaded { .. }) => Unloaded::NotLoaded {
                causes: (self.causes(script).into_iter())
                    .map(|cause| {
                        let blocked = blocked(&cause);
                        (cause, blocked)
                    })
                    .collect(),
            },
            _ => Unloaded::Failed,
        };
        unavailable_read(kind, names, script, &unloaded.why())
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

/// The texts of the data scripts of `files` and of its modules, each by path.
async fn read(
    files: &ProjectFiles,
) -> Result<(BTreeMap<String, String>, BTreeMap<String, String>), ProjectError> {
    let source = files.source();
    let mut texts = BTreeMap::new();
    for path in files.data_scripts() {
        texts.insert(path.clone(), source.read_text(path).await?);
    }
    let mut modules = BTreeMap::new();
    for module in files.modules() {
        modules.insert(module.path.clone(), source.read_text(&module.path).await?);
    }
    Ok((texts, modules))
}

/// The text of the module of `files` at the module path `names`, with the texts of the modules
/// `modules`, by path.
fn module_text(
    files: &ProjectFiles,
    modules: &BTreeMap<String, String>,
    names: &[String],
) -> Option<String> {
    modules.get(&files.module(names)?.path).cloned()
}

/// The graph of the data scripts of `texts`, which import the modules of `files`, with the
/// texts `modules`, by path, and a table of one name in the schema `default_schema`.
fn graph(
    files: &ProjectFiles,
    texts: &BTreeMap<String, String>,
    modules: &BTreeMap<String, String>,
    default_schema: &str,
) -> Graph {
    let scripts = (texts.iter())
        .map(|(path, text)| (path.clone(), avenger_sql_syntax::parse_data(text).syntax()));
    let module = |names: &[String]| module_text(files, modules, names);
    Graph::new(scripts, &module, default_schema)
}

/// The default schema of the session `ctx`, which holds a table of one name.
fn default_schema(ctx: &SessionContext) -> String {
    let state = ctx.state();
    state.config_options().catalog.default_schema.clone()
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

/// The changes from the tables `before` to the tables `after`, in the order of the paths, with a
/// table of one name in the schema `default_schema`.
fn changes(
    before: Vec<Vec<String>>,
    after: Vec<Vec<String>>,
    default_schema: &str,
) -> Vec<TableChange> {
    let same = |a: &[String], b: &[String]| avenger_sql::catalog::same_table(a, b, default_schema);
    let mut changes: Vec<TableChange> = Vec::new();
    for names in &after {
        let change = match before.iter().any(|old| same(old, names)) {
            true => Change::Replaced,
            false => Change::Registered,
        };
        if !(changes.iter()).any(|changed| same(&changed.names, names)) {
            changes.push(TableChange {
                names: names.clone(),
                change,
            });
        }
    }
    for names in before {
        if !(changes.iter()).any(|changed| same(&changed.names, &names)) {
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
