//! The files of a project together: a document for each file, the dependency graph of the data
//! scripts and the catalog of their tables, the project's diagnostics, and the requests that
//! cross files.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
};

use avenger_sql::{
    analysis::Options,
    catalog::{imported_modules, references, same_path, same_table, Catalog, FilesTable},
    data::{uri_kind, DefinitionKind, Files, UriKind},
    library::Library,
    project::{
        duplicate_message, table_paths, unavailable_read, Graph, GraphError, NameKind,
        ProjectCatalog, Unloaded,
    },
};
use avenger_sql_syntax::{ast, ast::AstNode, TextEdit, TextRange};

use crate::{
    data::{file_string, FileString},
    navigation::{check_name, TableName, TableOwner},
    quote, Diagnostic, Document, Offset, Reference, Role, Severity, Source,
};

/// The role of a file of a project, as its host finds it: a data script, a chart script, or a
/// module with the path that `IMPORT MODULE` names it by, which the host takes from the file's
/// path under the project's modules root, as `["a", "b"]` for `a/b.av.sqlm`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileRole {
    Data,
    Chart,
    Module { names: Vec<String> },
}

impl FileRole {
    /// The role that the file's text parses in.
    pub fn role(&self) -> Role {
        match self {
            FileRole::Data => Role::Data,
            FileRole::Chart => Role::Script,
            FileRole::Module { .. } => Role::Module,
        }
    }
}

/// A name of a file of a project and the range that defines it, in the file at `path`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectLink {
    /// The range of the name, which an editor underlines as the link.
    pub origin: TextRange,
    /// The path of the file that defines the name.
    pub path: String,
    /// The range that defines the name, in the text of the file at `path`.
    pub target: TextRange,
}

/// A use of a definition, or the name that defines it, in the file at `path`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectReference {
    pub path: String,
    pub range: TextRange,
    /// Whether the range is a name that defines the definition, such as the name of a data
    /// script's statement that creates or loads a table, rather than a use of it.
    pub definition: bool,
}

/// The files of a project: a [`Document`] for each file by its path and role, the [`Graph`] of
/// the data scripts, and the [`ProjectCatalog`] of the tables that they define, which the
/// requests across files read.
///
/// Each data script analyzes in the order of the graph, against the host's tables, the
/// definitions of the data scripts before it, the tables of the files that the host gives, and
/// the modules, and each chart script against the tables of every data script. A data script in
/// a cycle, which the graph can't order, analyzes after the others, by path. The host gives the
/// tables of the files that the data scripts read, from its loader, and a file set without one
/// has an unknown schema, which [`Document::diagnostics`] leaves out the errors of. A read of a
/// table of a data script that would not load, as the host's loader finds it, or that has no
/// table, since its statement has errors or its files have none, has an error of the project,
/// which names the data script, in place of the errors of the analysis of the read's statement
/// when the table has no schema. Names of tables compare with the base's default schema.
///
/// A project does no I/O: the host gives it the texts of the files, the tables of the files
/// that the data scripts read, the paths of the project's files, which completion offers in a
/// URI, and the errors of its loader. An edit of a file analyzes that file and its dependents
/// again: for a data script, the data scripts downstream of it in the graph before the edit or
/// after it, the scripts whose imported modules read its tables among them, the data scripts
/// that define a name that one of these also defines, and the chart
/// scripts that read a table that one of them defines, or that import
/// a module, whose functions can read tables that the chart's text doesn't name; for a module,
/// the scripts that import it and their dependents. A change of the tables of files analyzes
/// every data script again, and the chart scripts that read their tables. The modules analyze
/// again after a change of a data script or of a module, against the catalog after the change,
/// and the other files take the catalog after the change without analyzing again.
///
/// A `Project` is `Send` and `Sync`.
pub struct Project {
    base: Arc<dyn Catalog + Send + Sync>,
    library: Arc<Library>,
    options: Options,
    tables: HashMap<Files, FilesTable>,
    paths: Arc<[String]>,
    files: BTreeMap<String, File>,
    graph: Graph,
    catalog: Arc<ProjectCatalog>,
    /// Why the tables of each data script are unavailable, as a loader would find them, for the
    /// data scripts whose tables are, by path.
    unloaded: BTreeMap<String, Unloaded>,
    loader: BTreeMap<String, Vec<Diagnostic>>,
}

const _: fn() = || {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<Project>();
};

struct File {
    role: FileRole,
    document: Document,
}

/// A change of the files of a project, which [`Project::update`] applies.
#[derive(Default)]
struct Change {
    /// The new texts of files, with their roles.
    texts: BTreeMap<String, (FileRole, String)>,
    removed: BTreeSet<String>,
    /// Whether every data script analyzes again, after a change of the tables of files.
    all_data: bool,
}

impl Project {
    /// A project without files, whose tables come from `base`, the host's catalog, and whose
    /// functions come from `library`.
    pub fn new(
        base: Arc<dyn Catalog + Send + Sync>,
        library: Arc<Library>,
        options: Options,
    ) -> Project {
        let catalog = Arc::new(ProjectCatalog::new(base.clone()));
        let graph = Graph::new(Vec::<(String, _)>::new(), &|_| None, &base.default_schema());
        Project {
            base,
            library,
            options,
            tables: HashMap::new(),
            paths: Arc::from([]),
            files: BTreeMap::new(),
            graph,
            catalog,
            unloaded: BTreeMap::new(),
            loader: BTreeMap::new(),
        }
    }

    /// The project with `tables`, the table of each set of files that its data scripts read, or
    /// the error of reading it, which the host's loader reads.
    pub fn with_file_tables(
        mut self,
        tables: impl IntoIterator<Item = (Files, FilesTable)>,
    ) -> Self {
        self.set_file_tables(tables);
        self
    }

    /// The project with `paths`, the paths of its files, as [`Project::set_paths`] takes them.
    pub fn with_paths(mut self, paths: Arc<[String]>) -> Self {
        self.set_paths(paths);
        self
    }

    /// The project with the files `files`, each a path relative to the project's root, its
    /// role, and its text, which it analyzes together.
    pub fn with_files<P: Into<String>, T: Into<String>>(
        mut self,
        files: impl IntoIterator<Item = (P, FileRole, T)>,
    ) -> Self {
        let mut change = Change::default();
        for (path, role, text) in files {
            change.texts.insert(path.into(), (role, text.into()));
        }
        self.update(change);
        self
    }

    /// The document of the file at `path`.
    pub fn document(&self, path: &str) -> Option<&Document> {
        Some(&self.files.get(path)?.document)
    }

    /// The role of the file at `path`.
    pub fn role(&self, path: &str) -> Option<&FileRole> {
        Some(&self.files.get(path)?.role)
    }

    /// The paths of the files, sorted.
    pub fn files(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    /// The dependency graph of the data scripts.
    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    /// Adds the file at `path` with `role` and `text`, or replaces the file at `path`, and
    /// returns the files that the change reaches, as [`Project::set_texts`] gives them.
    pub fn add(
        &mut self,
        path: impl Into<String>,
        role: FileRole,
        text: impl Into<String>,
    ) -> BTreeSet<String> {
        let mut change = Change::default();
        change.texts.insert(path.into(), (role, text.into()));
        self.update(change)
    }

    /// Removes the file at `path`, and returns the files that the change reaches, the removed
    /// file with them, or none when the project lacks it.
    pub fn remove(&mut self, path: &str) -> BTreeSet<String> {
        if !self.files.contains_key(path) {
            return BTreeSet::new();
        }
        let mut change = Change::default();
        change.removed.insert(path.to_string());
        self.update(change)
    }

    /// Sets the texts of files of the project, each a path and its text, together, as the edits
    /// of [`Project::rename`] change them, each with its role. A path that the project lacks
    /// changes nothing. The loader's errors of each file are left out, since their ranges are in
    /// the text before the edit, until the host sets them again.
    ///
    /// The result is the files that the change reaches: those that it analyzes again, whose
    /// diagnostics, symbols, and other requests can change, and those that it removes. Another
    /// file keeps its analysis, so its diagnostics stay as they were.
    pub fn set_texts<P: Into<String>, T: Into<String>>(
        &mut self,
        texts: impl IntoIterator<Item = (P, T)>,
    ) -> BTreeSet<String> {
        let mut change = Change::default();
        for (path, text) in texts {
            let path = path.into();
            if let Some(file) = self.files.get(&path) {
                (change.texts).insert(path, (file.role.clone(), text.into()));
            }
        }
        match change.texts.is_empty() {
            true => BTreeSet::new(),
            false => self.update(change),
        }
    }

    /// Sets the tables of the files that the data scripts and the chart scripts' own tables of
    /// files read, as [`Project::with_file_tables`] takes them, and returns the files that the
    /// change reaches, as [`Project::set_texts`] gives them.
    pub fn set_file_tables(
        &mut self,
        tables: impl IntoIterator<Item = (Files, FilesTable)>,
    ) -> BTreeSet<String> {
        self.tables = tables.into_iter().collect();
        self.update(Change {
            all_data: true,
            ..Change::default()
        })
    }

    /// Sets the paths of the project's files, relative to its root and separated by `/`, which
    /// completion offers in a URI of a data script, as [`Document::with_paths`] takes them.
    pub fn set_paths(&mut self, paths: Arc<[String]>) {
        for file in self.files.values_mut() {
            file.document.paths = paths.clone();
        }
        self.paths = paths;
    }

    /// Sets the errors of the host's loader in the file at `path`, such as the errors of
    /// lowering its statements and the reads of the tables of data scripts that did not load,
    /// whose source becomes [`Source::Loader`]. [`Project::diagnostics`] leaves out an error with
    /// the range and the message of one that it finds itself, since a loader that analyzes the
    /// scripts finds the same errors, and an error at a read of a table that has no table, where
    /// the project has its own error, which names the data script that defines the table.
    pub fn set_loader_diagnostics(&mut self, path: &str, mut diagnostics: Vec<Diagnostic>) {
        for diagnostic in &mut diagnostics {
            diagnostic.source = Source::Loader;
        }
        self.loader.insert(path.to_string(), diagnostics);
    }

    /// The diagnostics of the file at `path`, in the order of their ranges: its document's, and
    /// the project's in it: an error at each read of a table that a data script defines and that
    /// has no table, which names the script, and in a data script, the errors of the graph, at a
    /// name that another data script also defines and at a read of a cycle, a warning at each
    /// URI outside the project's root, which another machine would not have, and the errors of
    /// the loader.
    pub fn diagnostics(&self, path: &str) -> Vec<Diagnostic> {
        let Some(file) = self.files.get(path) else {
            return Vec::new();
        };
        let mut diagnostics = file.document.diagnostics();
        let project =
            |range: TextRange, message: String, help: Option<String>, severity| Diagnostic {
                range,
                message,
                help,
                severity,
                source: Source::Project,
                code: None,
                fixes: Vec::new(),
            };
        // A name that several data scripts define has one error at each definition, which names
        // every script that defines it.
        let mut duplicates: Vec<(TextRange, NameKind, Vec<String>, BTreeSet<String>)> = Vec::new();
        for error in self.graph.errors() {
            match error {
                GraphError::Duplicate {
                    kind,
                    names,
                    first,
                    second,
                } => {
                    for place in [first, second]
                        .into_iter()
                        .filter(|place| place.path == path)
                    {
                        let at = match (duplicates.iter()).position(|found| found.0 == place.range)
                        {
                            Some(at) => at,
                            None => {
                                duplicates.push((
                                    place.range,
                                    *kind,
                                    names.clone(),
                                    BTreeSet::new(),
                                ));
                                duplicates.len() - 1
                            }
                        };
                        (duplicates[at].3).extend([first.path.clone(), second.path.clone()]);
                    }
                }
                GraphError::Cycle { .. } => {
                    for place in (error.places().into_iter()).filter(|place| place.path == path) {
                        // A read of a cycle finds no table, since the script that defines it
                        // analyzes after the reader, and the cycle's error says why.
                        diagnostics.retain(|found| {
                            found.source != Source::Analysis || found.range != place.range
                        });
                        let diagnostic =
                            project(place.range, error.message(), None, Severity::Error);
                        diagnostics.push(diagnostic);
                    }
                }
            }
        }
        for (range, kind, names, scripts) in duplicates {
            let scripts: Vec<&str> = scripts.iter().map(String::as_str).collect();
            let message = duplicate_message(kind, &names, &scripts);
            diagnostics.push(project(range, message, None, Severity::Error));
        }
        let unavailable = self.unavailable_reads(path, &file.document);
        let reads: Vec<TextRange> = unavailable.iter().map(|read| read.range).collect();
        diagnostics.extend(unavailable);
        if matches!(file.role, FileRole::Data | FileRole::Chart) {
            for (range, uri) in uris(&file.document) {
                let outside = matches!(
                    uri_kind(&uri),
                    UriKind::AboveRoot | UriKind::Absolute | UriKind::FileUrl
                );
                if outside {
                    let message = format!(
                        "The URI '{uri}' is outside the project's root, so the project doesn't hold its files, which another machine would not have"
                    );
                    let help = "Keep the files in the project, and write the path relative to the project's root, as 'data/files/sales.csv'";
                    let warning =
                        project(range, message, Some(help.to_string()), Severity::Warning);
                    diagnostics.push(warning);
                }
            }
        }
        for diagnostic in self.loader.get(path).into_iter().flatten() {
            let found = reads.contains(&diagnostic.range)
                || (diagnostics.iter()).any(|found| {
                    found.range == diagnostic.range && found.message == diagnostic.message
                });
            if !found {
                diagnostics.push(diagnostic.clone());
            }
        }
        diagnostics.sort_by_key(|diagnostic| diagnostic.range.start());
        diagnostics
    }

    /// An error at each read in `document`, the file at `path`, of a table or a schema that
    /// another data script defines and that is unavailable, which names that script: since the
    /// script would not load, as a loader finds it, when it has errors, defines a name that
    /// another script defines too, is in a cycle, or reads a script that would not load, or,
    /// when it would load, since the table has no schema, as when its files have no table. The
    /// document leaves out the errors of the analysis in the statement of a read of a table
    /// without a schema. A read of a cycle has the cycle's error instead.
    fn unavailable_reads(&self, path: &str, document: &Document) -> Vec<Diagnostic> {
        let cycles: Vec<TextRange> = (self.graph.errors().iter())
            .filter(|error| matches!(error, GraphError::Cycle { .. }))
            .flat_map(|error| error.places())
            .filter(|place| place.path == path)
            .map(|place| place.range)
            .collect();
        let mut diagnostics = Vec::new();
        for (names, range) in document.table_reads() {
            if cycles.contains(&range) || document.own_table(&names, range.start()).is_some() {
                continue;
            }
            let Some((script, kind, names)) = self.definer(&names) else {
                continue;
            };
            // A data script reads none of its own definitions from the project: a read in the
            // statement that defines the same name reads the tables before the script.
            if script == path {
                continue;
            }
            // A table without a schema tells why it has none, as when its files could not be
            // read, and another table of a script that would not load tells why the script
            // would not.
            let why = match (self.missing(kind, &names), self.unloaded.get(&script)) {
                (Some(why), None | Some(Unloaded::Failed)) => why.to_string(),
                (_, Some(unloaded)) => unloaded.why(),
                (None, None) => continue,
            };
            diagnostics.push(Diagnostic {
                range,
                message: unavailable_read(kind, &names, &script, &why),
                help: None,
                severity: Severity::Error,
                source: Source::Project,
                code: None,
                fixes: Vec::new(),
            });
        }
        diagnostics
    }

    /// The data script that defines the table `names`, or else the schema of its path, with the
    /// kind and the path of what it defines, as a loader finds it: the first by path, when two
    /// scripts define it.
    fn definer(&self, names: &[String]) -> Option<(String, NameKind, Vec<String>)> {
        let default_schema = self.base.default_schema();
        let find = |kind: NameKind, names: &[String]| {
            (self.graph.paths()).find(|path| {
                (self.graph.names(path)).is_some_and(|script| {
                    (script.definitions.iter()).any(|name| {
                        name.kind == kind && kind.same(&name.names, names, &default_schema)
                    })
                })
            })
        };
        match find(NameKind::Table, names) {
            Some(path) => Some((path.to_string(), NameKind::Table, names.to_vec())),
            None => match names {
                [schema, _] => Some((
                    find(NameKind::Schema, &names[..1])?.to_string(),
                    NameKind::Schema,
                    vec![schema.clone()],
                )),
                _ => None,
            },
        }
    }

    /// Why the table `names` of a data script that would load has no schema, or `None` when it
    /// has one, or is a schema: its files have no table, or could not be read, or its statement
    /// has errors.
    fn missing(&self, kind: NameKind, names: &[String]) -> Option<&'static str> {
        if kind == NameKind::Schema || self.catalog.table(names).is_some() {
            return None;
        }
        let definition = self.catalog.definition(names)?;
        let analysis = (self.document(&definition.file)).and_then(Document::data_analysis)?;
        let files = (analysis.definitions.iter())
            .find(|defined| defined.range == definition.range)
            .and_then(|defined| match &defined.kind {
                DefinitionKind::ExternalTable { files }
                | DefinitionKind::LoadData { files, .. } => files.as_ref(),
                _ => None,
            });
        Some(match files.map(|files| self.tables.get(files)) {
            Some(None) => "whose files have no table, since the project has not read them",
            Some(Some(Err(_))) => "whose files could not be read",
            Some(Some(Ok(_))) | None => "which has errors",
        })
    }

    /// The name at `offset` in the file at `path` and the range that defines it, in its file: a
    /// table to the name of the data script's statement that creates or first loads it, an
    /// `IMPORT MODULE` to the `MODULE` statement of the module's file, or the start of the file
    /// without one, and another name as [`Document::definition`] defines it.
    pub fn definition(&self, path: &str, offset: Offset) -> Option<ProjectLink> {
        let document = self.document(path)?;
        if let Some(link) = self.module_definition(document, offset) {
            return Some(link);
        }
        if let Some(table) = document.table_at(offset) {
            let (path, target) = match table.own {
                Some(index) => (path.to_string(), document.definitions()[index].name_range),
                None => {
                    let definition = self.catalog.definition(&table.names)?;
                    let analysis = self.document(&definition.file)?.data_analysis()?;
                    let defined = (analysis.definitions.iter())
                        .find(|defined| defined.range == definition.range)?;
                    (definition.file, defined.name_range)
                }
            };
            return Some(ProjectLink {
                origin: table.range,
                path,
                target,
            });
        }
        let link = document.definition(offset)?;
        Some(ProjectLink {
            origin: link.origin,
            path: path.to_string(),
            target: link.target,
        })
    }

    /// The link of the path of an `IMPORT MODULE` at `offset` in `document` to its module file.
    fn module_definition(&self, document: &Document, offset: Offset) -> Option<ProjectLink> {
        let offset = document.clamp(offset);
        let import = (document.root().descendants())
            .filter_map(ast::ImportStmt::cast)
            .find_map(|import| {
                import
                    .path()
                    .filter(|path| path.range().contains_inclusive(offset))
            })?;
        let names = import.names()?;
        let (path, file) = self.files.iter().find(|(_, file)| {
            matches!(&file.role, FileRole::Module { names: module } if same_path(module, &names))
        })?;
        let module = ast::Module::cast(file.document.root())
            .and_then(|module| module.module_stmt())
            .map(|stmt| crate::trimmed(stmt.syntax()));
        Some(ProjectLink {
            origin: import.range(),
            path: path.clone(),
            target: module.unwrap_or_default(),
        })
    }

    /// The uses of the definition of the name at `offset` in the file at `path`, and the names
    /// that define it, by the files in the order of their paths and in each in the order of its
    /// text. A table's are its reads in every script and the names of the statements that create
    /// or load it, of a `TEMP` table in its data script alone, and another name's are those of
    /// [`Document::references`].
    pub fn references(&self, path: &str, offset: Offset) -> Vec<ProjectReference> {
        let Some(document) = self.document(path) else {
            return Vec::new();
        };
        let reference = |path: &str, reference: Reference| ProjectReference {
            path: path.to_string(),
            range: reference.range,
            definition: reference.definition,
        };
        match document.table_at(offset) {
            Some(table) => (self.table_references(path, &table).into_iter())
                .flat_map(|(path, references)| {
                    (references.into_iter()).map(move |found| reference(path, found))
                })
                .collect(),
            None => (document.references(offset).into_iter())
                .map(|found| reference(path, found))
                .collect(),
        }
    }

    /// The names of the table `table`, which a name of the file at `path` names, by file: in
    /// the file alone for a `TEMP` table, and otherwise in each file, as a table of a data
    /// script that defines it, or a table of the catalog in the other files.
    fn table_references<'a>(
        &'a self,
        path: &'a str,
        table: &TableName,
    ) -> Vec<(&'a str, Vec<Reference>)> {
        let document = &self.files[path].document;
        if temp_table(document, table) {
            let owner = TableOwner::Own { temp: true };
            return vec![(path, document.table_names(&table.names, owner))];
        }
        (self.files.iter())
            .map(|(path, file)| {
                let owner = match defines(&file.document, &table.names, false) {
                    true => TableOwner::Own { temp: false },
                    false => TableOwner::Catalog,
                };
                (
                    path.as_str(),
                    file.document.table_names(&table.names, owner),
                )
            })
            .filter(|(_, references)| !references.is_empty())
            .collect()
    }

    /// Whether the table `table`, which a name of a file names, is a table of the host, which no
    /// data script defines.
    fn host_table(&self, table: &TableName) -> bool {
        table.own.is_none()
            && !(self.files.values()).any(|file| defines(&file.document, &table.names, false))
    }

    /// The range of the name at `offset` in the file at `path` when a rename can change it: the
    /// last name of a table's path, of a table that a data script defines, or a name that
    /// [`Document::prepare_rename`] renames. Otherwise the reason that it can't.
    pub fn prepare_rename(&self, path: &str, offset: Offset) -> Result<TextRange, String> {
        let document = self
            .document(path)
            .ok_or_else(|| format!("No file {path} in the project"))?;
        let Some(table) = document.table_at(offset) else {
            return document.prepare_rename(offset);
        };
        if self.host_table(&table) {
            return Err(match self.catalog.table(&table.names) {
                Some(_) => "A table of the host has no rename".to_string(),
                None => format!("No table {} to rename", table.names.join(".")),
            });
        }
        let last = document
            .last_name(table.range)
            .ok_or("The table's name has no rename")?;
        match last.contains_inclusive(document.clamp(offset)) {
            true => Ok(last),
            false => Err("A schema has no rename".to_string()),
        }
    }

    /// The edits that rename the definition at `offset` in the file at `path` and each use of it
    /// to `name`, by the files that they edit: for a table that a data script defines, the last
    /// name of its path in the statements that create or load it and in each read of it, in
    /// every script, or in its data script alone for a `TEMP` table, and for another name, the
    /// edits of [`Document::rename`]. A rename of a table to the name of another table, a rename
    /// that adds an error to a file of the project, such as a rename of a column that a chart
    /// reads, and a rename that changes the table that a read reads, are errors.
    pub fn rename(
        &self,
        path: &str,
        offset: Offset,
        name: &str,
    ) -> Result<BTreeMap<String, Vec<TextEdit>>, String> {
        self.prepare_rename(path, offset)?;
        let document = &self.files[path].document;
        let table = document.table_at(offset);
        let edits = match &table {
            Some(table) => self.table_edits(path, table, name)?,
            None => BTreeMap::from([(path.to_string(), document.rename(offset, name)?)]),
        };

        // The renamed project must mean what the project means: no new error in any file,
        // besides the loader's, which the renamed project lacks, and the renamed names of a
        // table name the renamed table.
        let renamed = self.renamed(&edits);
        let errors = |project: &Project, path: &str| -> Vec<Diagnostic> {
            (project.diagnostics(path).into_iter())
                .filter(|diagnostic| diagnostic.severity == Severity::Error)
                .filter(|diagnostic| diagnostic.source != Source::Loader)
                .collect()
        };
        // An error of a file that the rename edits matches by its message, since the edits
        // move the ranges, and an error of another file by its range too.
        for file in renamed.files() {
            let mut before = errors(self, file);
            let edited = edits.contains_key(file);
            for error in errors(&renamed, file) {
                let found = (before.iter()).position(|old| {
                    old.message == error.message && (edited || old.range == error.range)
                });
                match found {
                    Some(at) => {
                        before.remove(at);
                    }
                    None => {
                        let message = error.message;
                        return Err(format!("The rename to `{name}` makes an error: {message}"));
                    }
                }
            }
        }
        if let Some(table) = table {
            let count = |found: Vec<(&str, Vec<Reference>)>| -> Vec<(String, usize)> {
                (found.into_iter())
                    .map(|(path, references)| (path.to_string(), references.len()))
                    .collect()
            };
            let before = count(self.table_references(path, &table));
            let mut names = table.names.clone();
            *names.last_mut().expect("a path of names") = name.to_string();
            let renamed_table = TableName { names, ..table };
            if count(renamed.table_references(path, &renamed_table)) != before {
                return Err(format!(
                    "The rename to `{name}` changes the table that a name refers to"
                ));
            }
        }
        Ok(edits)
    }

    /// The edits that rename the table `table`, which a name of the file at `path` names, to
    /// `name`, by file, or the error of a name that another table has.
    fn table_edits(
        &self,
        path: &str,
        table: &TableName,
        name: &str,
    ) -> Result<BTreeMap<String, Vec<TextEdit>>, String> {
        check_name(name)?;
        let document = &self.files[path].document;
        let mut renamed = table.names.clone();
        *renamed.last_mut().expect("a path of names") = name.to_string();
        let references = self.table_references(path, table);
        // A TEMP table's name is taken in its script. Another's is taken by a table of any data
        // script, and by a TEMP table of a script that reads the table, whose reads it would
        // read.
        let taken = match temp_table(document, table) {
            true => defines(document, &renamed, true),
            false => (self.files.iter()).any(|(path, file)| {
                let reads = (references.iter()).any(|(found, _)| found == path);
                defines(&file.document, &renamed, reads)
            }),
        } || self.catalog.table(&renamed).is_some();
        if taken && !same_path(&renamed, &table.names) {
            return Err(format!(
                "The rename to `{name}` makes the name of another table, {}",
                renamed.join(".")
            ));
        }
        // The functions of a module read the table where a script calls them, and a rename
        // changes no module.
        if !temp_table(document, table) {
            for (module, file) in &self.files {
                let reads = (table_paths(&file.document.root()).into_iter())
                    .any(|read| document.same_table(&read.names, &table.names));
                if matches!(file.role, FileRole::Module { .. }) && reads {
                    return Err(format!(
                        "The module {module} reads {}, and a rename changes no module",
                        table.names.join(".")
                    ));
                }
            }
        }
        let replacement = quote(name);
        let edits = (references.into_iter())
            .map(|(path, references)| {
                let document = &self.files[path].document;
                let edits = (references.iter())
                    .filter_map(|reference| document.last_name(reference.range))
                    .map(|range| TextEdit::new(range, &replacement))
                    .collect();
                (path.to_string(), edits)
            })
            .collect();
        Ok(edits)
    }

    /// The project with `edits` applied to the texts of its files.
    fn renamed(&self, edits: &BTreeMap<String, Vec<TextEdit>>) -> Project {
        let files = (self.files.iter()).map(|(path, file)| {
            let text = match edits.get(path) {
                Some(edits) => crate::navigation::apply(file.document.text(), edits),
                None => file.document.text().to_string(),
            };
            (path.clone(), file.role.clone(), text)
        });
        Project::new(
            self.base.clone(),
            self.library.clone(),
            self.options.clone(),
        )
        .with_file_tables(self.tables.clone())
        .with_paths(self.paths.clone())
        .with_files(files)
    }

    /// Applies `change`, analyzes the files that it changes and their dependents again, and
    /// returns the paths of the files that it analyzed again or removed.
    fn update(&mut self, change: Change) -> BTreeSet<String> {
        let Change {
            texts,
            removed,
            all_data,
        } = change;
        // The data scripts and the modules that change, by their roles before the change and
        // after it.
        let mut seeds = BTreeSet::new();
        let mut modules: Vec<Vec<String>> = Vec::new();
        for path in texts.keys().chain(&removed) {
            let before = self.files.get(path).map(|file| &file.role);
            let after = texts.get(path).map(|(role, _)| role);
            for role in before.into_iter().chain(after) {
                match role {
                    FileRole::Data => {
                        seeds.insert(path.clone());
                    }
                    FileRole::Module { names } => modules.push(names.clone()),
                    FileRole::Chart => {}
                }
            }
        }
        // The texts of the modules before the change, by module path.
        let modules_before =
            module_texts((self.files.values()).map(|file| (&file.role, file.document.text())));
        // The tables of the data scripts before the change, by path.
        let before: BTreeMap<String, Vec<Vec<String>>> = (self.files.iter())
            .filter_map(|(path, file)| {
                Some((path.clone(), exported(file.document.data_analysis()?)))
            })
            .collect();
        // The files that the change analyzes again or removes, which it returns.
        let mut reached: BTreeSet<String> = removed.clone();
        for path in &removed {
            self.files.remove(path);
            self.loader.remove(path);
        }
        // The texts of the files to analyze, by path. Until it analyzes, a file of `pending` has
        // the document of its text before the change, if it had one.
        let mut pending: BTreeMap<String, (FileRole, String)> = BTreeMap::new();
        for (path, (role, text)) in texts {
            self.loader.remove(&path);
            pending.insert(path, (role, text));
        }

        // The graph of the data scripts after the change.
        let is_data = |path: &String| {
            let role = (pending.get(path).map(|(role, _)| role))
                .or_else(|| self.files.get(path).map(|file| &file.role));
            role == Some(&FileRole::Data)
        };
        let data_paths: BTreeSet<String> = (self.files.keys().chain(pending.keys()))
            .filter(|path| is_data(path))
            .cloned()
            .collect();
        let roots: Vec<(String, avenger_sql_syntax::SyntaxNode)> = (data_paths.iter())
            .map(|path| {
                let root = match pending.get(path) {
                    Some((_, text)) => avenger_sql_syntax::parse_data(text).syntax(),
                    None => self.files[path].document.root(),
                };
                (path.clone(), root)
            })
            .collect();
        // The texts of the modules as the change leaves them, by module path.
        let unchanged = (self.files.iter())
            .filter(|(path, _)| !pending.contains_key(*path))
            .map(|(_, file)| (&file.role, file.document.text()));
        let changed = (pending.values()).map(|(role, text)| (role, text.as_str()));
        let modules_after = module_texts(unchanged.chain(changed));
        let default_schema = self.base.default_schema();
        let graph = Graph::new(
            roots.iter().cloned(),
            &|names| module_text(&modules_after, names),
            &default_schema,
        );
        // The data scripts that import a changed module, directly or through other modules,
        // before the change or after it.
        match all_data {
            true => seeds.extend(roots.iter().map(|(path, _)| path.clone())),
            false => {
                for module in &modules {
                    for graph in [&self.graph, &graph] {
                        seeds.extend(graph.importers(module).into_iter().map(str::to_string));
                    }
                }
            }
        }
        // The changed scripts, and the scripts downstream of each before the change or after it,
        // and the scripts that define a name that one of them defines too, whose analyses read
        // the catalog of that name, as an append of `LOAD DATA INTO` does.
        let mut affected = BTreeSet::new();
        let mut queue: Vec<String> = seeds.into_iter().collect();
        while let Some(path) = queue.pop() {
            if affected.insert(path.clone()) {
                for graph in [&self.graph, &graph] {
                    queue.extend(graph.downstream(&path).into_iter().map(str::to_string));
                    queue.extend(duplicates(graph, &path));
                }
            }
        }
        let mut tables: Vec<Vec<String>> = (affected.iter())
            .filter_map(|path| before.get(path))
            .flatten()
            .cloned()
            .collect();

        // The data scripts in the order of the graph, and then those that it can't order, each
        // against the tables of the files, the modules, and the scripts before it.
        let mut catalog = ProjectCatalog::new(self.base.clone());
        for (files, table) in &self.tables {
            catalog = catalog.with_files(files.clone(), table.clone());
        }
        for (names, text) in &modules_after {
            catalog = catalog.with_module(names.clone(), text.clone());
        }
        let order: Vec<String> = (graph.order().into_iter().chain(graph.blocked()))
            .map(str::to_string)
            .collect();
        for path in &order {
            if affected.contains(path) {
                let text = match pending.remove(path) {
                    Some((_, text)) => text,
                    None => self.files[path].document.text().to_string(),
                };
                let document = self.document_of(&text, Role::Data, Arc::new(catalog.clone()));
                tables.extend(exported(document.data_analysis().expect("a data script")));
                let role = FileRole::Data;
                self.files.insert(path.clone(), File { role, document });
                reached.insert(path.clone());
            } else {
                // A script that doesn't analyze again keeps its analysis, which no change reaches,
                // and completes the names of the catalog before it as the change leaves it.
                let file = self
                    .files
                    .get_mut(path)
                    .expect("a data script of the graph");
                file.document.catalog = Arc::new(catalog.clone());
            }
            let analysis = self.files[path].document.data_analysis();
            catalog = catalog.with_analysis(path, analysis.expect("a data script"));
        }
        self.graph = graph;
        self.catalog = Arc::new(catalog);

        // The chart scripts that changed, that read a table that changed, or that import a
        // module that changed, directly or through other modules, before the change or after it,
        // or any module after a change of the data scripts, or that have
        // their own tables of files after a change of the tables of files, and the modules, whose
        // functions read the tables, after a change of the data scripts or of a module.
        let data_changed = !affected.is_empty();
        let again: Vec<(String, FileRole, String)> = (self.files.iter())
            .filter(|(path, file)| {
                matches!(file.role, FileRole::Module { .. }) && !pending.contains_key(*path)
            })
            .filter(|_| data_changed || !modules.is_empty())
            .map(|(path, file)| {
                (
                    path.clone(),
                    file.role.clone(),
                    file.document.text().to_string(),
                )
            })
            .collect();
        for (path, role, text) in again {
            pending.insert(path, (role, text));
        }
        let charts: Vec<(String, String)> = (self.files.iter())
            .filter(|(path, file)| file.role == FileRole::Chart && !pending.contains_key(*path))
            .filter(|(_, file)| {
                let read = references(&file.document.root());
                (read.tables.iter()).any(|read| {
                    (tables.iter()).any(|table| same_table(table, &read.names, &default_schema))
                }) || [&modules_before, &modules_after]
                    .into_iter()
                    .any(|texts| imports(&file.document.root(), &modules, texts))
                    || (data_changed && !read.modules.is_empty())
                    || (all_data && !file.document.definitions().is_empty())
            })
            .map(|(path, file)| (path.clone(), file.document.text().to_string()))
            .collect();
        for (path, text) in charts {
            pending.insert(path, (FileRole::Chart, text));
        }
        for (path, (role, text)) in pending {
            let document = self.document_of(&text, role.role(), self.catalog.clone());
            reached.insert(path.clone());
            self.files.insert(path, File { role, document });
        }
        // The other chart scripts and modules complete the names of the catalog after the change.
        for file in self.files.values_mut() {
            if file.role != FileRole::Data {
                file.document.catalog = self.catalog.clone();
            }
        }
        self.unloaded = self.unloaded();
        reached
    }

    /// Why the tables of each data script that would not load are unavailable, as a loader finds
    /// it, by path: a script in a cycle, or that reads one, is blocked, a script that reads a
    /// script that would not load does not load, and another script fails when it has errors, or
    /// defines a name that another script defines too.
    fn unloaded(&self) -> BTreeMap<String, Unloaded> {
        let duplicated: BTreeSet<&str> = (self.graph.errors().iter())
            .filter_map(|error| match error {
                GraphError::Duplicate { first, second, .. } => {
                    Some([first.path.as_str(), second.path.as_str()])
                }
                GraphError::Cycle { .. } => None,
            })
            .flatten()
            .collect();
        let mut unloaded: BTreeMap<String, Unloaded> = (self.graph.blocked().into_iter())
            .map(|path| (path.to_string(), Unloaded::Blocked))
            .collect();
        for path in self.graph.order() {
            let mut causes: BTreeMap<String, bool> = BTreeMap::new();
            for dependency in self.graph.dependencies(path) {
                match unloaded.get(dependency) {
                    Some(Unloaded::Failed) => {
                        causes.insert(dependency.to_string(), false);
                    }
                    Some(Unloaded::Blocked) => {
                        causes.insert(dependency.to_string(), true);
                    }
                    Some(Unloaded::NotLoaded { causes: upstream }) => {
                        causes.extend(upstream.iter().cloned());
                    }
                    None => {}
                }
            }
            let errors = || {
                (self.document(path).into_iter())
                    .flat_map(Document::diagnostics)
                    .any(|diagnostic| diagnostic.severity == Severity::Error)
            };
            if !causes.is_empty() {
                let causes = causes.into_iter().collect();
                unloaded.insert(path.to_string(), Unloaded::NotLoaded { causes });
            } else if duplicated.contains(path) || errors() {
                unloaded.insert(path.to_string(), Unloaded::Failed);
            }
        }
        unloaded
    }

    /// The document of `text`, of the role `role`, with the catalog `catalog`.
    fn document_of(
        &self,
        text: &str,
        role: Role,
        catalog: Arc<dyn Catalog + Send + Sync>,
    ) -> Document {
        Document::new(
            text,
            role,
            catalog,
            self.library.clone(),
            self.options.clone(),
        )
        .with_paths(self.paths.clone())
    }
}

/// The data scripts of `graph` that define a name that the data script at `path` also defines.
fn duplicates(graph: &Graph, path: &str) -> Vec<String> {
    (graph.errors().iter())
        .filter_map(|error| match error {
            avenger_sql::project::GraphError::Duplicate { first, second, .. } => {
                match (first.path == path, second.path == path) {
                    (true, _) => Some(second.path.clone()),
                    (_, true) => Some(first.path.clone()),
                    _ => None,
                }
            }
            avenger_sql::project::GraphError::Cycle { .. } => None,
        })
        .collect()
}

/// Whether the script of `document` defines the table `names`: without `TEMP`, or also with it
/// when `temp`. A chart script's own tables of files are `TEMP`.
fn defines(document: &Document, names: &[String], temp: bool) -> bool {
    (document.definitions().iter()).any(|definition| {
        (temp || !definition.temp)
            && !matches!(definition.kind, DefinitionKind::Schema)
            && document.same_table(&definition.names, names)
    })
}

/// Whether `table`, which a name of `document` names, is a `TEMP` table of its script: a data
/// script's, or a chart script's own table of files.
fn temp_table(document: &Document, table: &TableName) -> bool {
    table
        .own
        .is_some_and(|index| document.definitions()[index].temp)
}

/// The paths of the tables, views, and schemas that the analysis `analysis` of a data script
/// defines for the other scripts.
fn exported(analysis: &avenger_sql::data::DataAnalysis) -> Vec<Vec<String>> {
    (analysis.exported())
        .map(|definition| definition.names.clone())
        .collect()
}

/// The texts of the modules among `files`, each a role and a text, by module path.
fn module_texts<'a>(
    files: impl Iterator<Item = (&'a FileRole, &'a str)>,
) -> Vec<(Vec<String>, String)> {
    (files.filter_map(|(role, text)| match role {
        FileRole::Module { names } => Some((names.clone(), text.to_string())),
        _ => None,
    }))
    .collect()
}

/// The text of the module at the path `names` among `texts`, by module path.
fn module_text(texts: &[(Vec<String>, String)], names: &[String]) -> Option<String> {
    (texts.iter())
        .find(|(module, _)| same_path(module, names))
        .map(|(_, text)| text.clone())
}

/// Whether the script `root` imports one of the modules `modules`, directly or through other
/// modules, with the texts of the modules `texts`, as [`imported_modules`] reads the imports.
fn imports(
    root: &avenger_sql_syntax::SyntaxNode,
    modules: &[Vec<String>],
    texts: &[(Vec<String>, String)],
) -> bool {
    let imported = imported_modules(&references(root), &|names| module_text(texts, names));
    (imported.iter()).any(|(names, _)| (modules.iter()).any(|module| same_path(module, names)))
}

/// The URIs of the statements of the data script `document` that read files, each at the range
/// of its string.
fn uris(document: &Document) -> Vec<(TextRange, String)> {
    let root = document.root();
    (root.descendants_with_tokens())
        .filter_map(|element| element.into_token())
        .filter(|token| {
            matches!(
                file_string(token, document.role()),
                Some(FileString::Uri(..))
            )
        })
        .filter_map(|token| {
            let literal = ast::Literal::cast(token.parent()?)?;
            Some((literal.range(), literal.string_value()?))
        })
        .collect()
}
