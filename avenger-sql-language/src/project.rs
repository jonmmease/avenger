//! The files of a project together: a document for each file, the dependency graph of the data
//! scripts and the catalog of their tables, the project's diagnostics, and the requests that
//! cross files.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
};

use avenger_sql::{
    analysis::Options,
    catalog::{references, same_path, Catalog, FilesTable},
    data::{uri_kind, DefinitionKind, Files, UriKind},
    library::Library,
    project::{unavailable_read, Graph, NameKind, ProjectCatalog},
};
use avenger_sql_syntax::{ast, ast::AstNode, TextEdit, TextRange};

use crate::{
    data::{file_string, FileString},
    navigation::{check_name, TableName, TableOwner},
    quote, Diagnostic, Document, Offset, Reference, Role, Severity, Source,
};

/// The role of a file of a project, as its host finds it: a data script, a chart script, or a
/// module with the path that `IMPORT MODULE` names it by, which the host takes from the file's
/// path under the project's modules root, as `["a", "b"]` for `a/b.sqlm`.
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
/// table that a data script defines and that has no table, since its statement has errors or
/// its files have none, has an error of the project, which names the data script, in place of
/// the errors of the analysis of the read's statement.
///
/// A project does no I/O: the host gives it the texts of the files, the tables of the files
/// that the data scripts read, the paths of the project's files, which completion offers in a
/// URI, and the errors of its loader. An edit of a file analyzes that file and its dependents
/// again: for a data script, the data scripts downstream of it in the graph before the edit or
/// after it, the data scripts that define a name that one of these also defines, and the chart
/// scripts that read a table that one of them defines, or that import
/// a module, whose functions can read tables that the chart's text doesn't name; for a module,
/// the scripts that import it and their dependents. A change of the tables of files analyzes
/// every data script again, and the chart scripts that read their tables. The modules analyze
/// again after a change of a data script or of a module, against the catalog after the change.
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
        Project {
            base,
            library,
            options,
            tables: HashMap::new(),
            paths: Arc::from([]),
            files: BTreeMap::new(),
            graph: Graph::new(Vec::<(String, _)>::new()),
            catalog,
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

    /// Adds the file at `path` with `role` and `text`, or replaces the file at `path`.
    pub fn add(&mut self, path: impl Into<String>, role: FileRole, text: impl Into<String>) {
        let mut change = Change::default();
        change.texts.insert(path.into(), (role, text.into()));
        self.update(change);
    }

    /// Removes the file at `path`, and whether the project had it.
    pub fn remove(&mut self, path: &str) -> bool {
        if !self.files.contains_key(path) {
            return false;
        }
        let mut change = Change::default();
        change.removed.insert(path.to_string());
        self.update(change);
        true
    }

    /// Sets the text of the file at `path`, and whether the project has it. The loader's errors
    /// of the file are left out, since their ranges are in the text before the edit, until the
    /// host sets them again.
    pub fn set_text(&mut self, path: &str, text: impl Into<String>) -> bool {
        let Some(file) = self.files.get(path) else {
            return false;
        };
        let mut change = Change::default();
        (change.texts).insert(path.to_string(), (file.role.clone(), text.into()));
        self.update(change);
        true
    }

    /// Sets the tables of the files that the data scripts and the chart scripts' own tables of
    /// files read, as [`Project::with_file_tables`] takes them.
    pub fn set_file_tables(&mut self, tables: impl IntoIterator<Item = (Files, FilesTable)>) {
        self.tables = tables.into_iter().collect();
        self.update(Change {
            all_data: true,
            ..Change::default()
        });
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
    /// with the source [`Source::Loader`]. [`Project::diagnostics`] leaves out an error with the
    /// range and the message of one that it finds itself, since a loader that analyzes the
    /// scripts finds the same errors, and an error at a read of a table that has no table, where
    /// the project has its own error, which names the data script that defines the table.
    pub fn set_loader_diagnostics(&mut self, path: &str, diagnostics: Vec<Diagnostic>) {
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
        for error in self.graph.errors() {
            for place in error
                .places()
                .into_iter()
                .filter(|place| place.path == path)
            {
                // A read of a cycle finds no table, since the script that defines it analyzes
                // after the reader, and the cycle's error says why.
                if matches!(error, avenger_sql::project::GraphError::Cycle { .. }) {
                    diagnostics.retain(|found| {
                        found.source != Source::Analysis || found.range != place.range
                    });
                }
                let diagnostic = project(place.range, error.message(), None, Severity::Error);
                diagnostics.push(diagnostic);
            }
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

    /// An error at each read in `document`, the file at `path`, of a table that a data script
    /// defines and whose table is unavailable, since its statement has errors or its files have
    /// no table, which names that script. The document leaves out the errors of the analysis in
    /// the statement of the read, which the analysis can't check without the table's schema. A
    /// read of a cycle has the cycle's error instead.
    fn unavailable_reads(&self, path: &str, document: &Document) -> Vec<Diagnostic> {
        let cycles: Vec<TextRange> = (self.graph.errors().iter())
            .filter(|error| matches!(error, avenger_sql::project::GraphError::Cycle { .. }))
            .flat_map(|error| error.places())
            .filter(|place| place.path == path)
            .map(|place| place.range)
            .collect();
        let mut diagnostics = Vec::new();
        for read in document.reads() {
            let names = &read.names;
            if cycles.contains(&read.range)
                || document.last_name(read.range).is_none()
                || self.catalog.table(names).is_some()
                || self.catalog.has_schema(names)
            {
                continue;
            }
            let Some(definition) = self.catalog.definition(names) else {
                continue;
            };
            // A data script reads none of its own definitions from the project: a read in the
            // statement that defines the same name reads the tables before the script.
            if definition.file == path {
                continue;
            }
            let Some(analysis) =
                (self.document(&definition.file)).and_then(Document::data_analysis)
            else {
                continue;
            };
            let files = (analysis.definitions.iter())
                .find(|defined| defined.range == definition.range)
                .and_then(|defined| match &defined.kind {
                    DefinitionKind::ExternalTable { files }
                    | DefinitionKind::LoadData { files, .. } => files.as_ref(),
                    _ => None,
                });
            let why = match files.map(|files| self.tables.get(files)) {
                Some(None) => "whose files have no table, since the project has not read them",
                Some(Some(Err(_))) => "whose files could not be read",
                Some(Some(Ok(_))) | None => "which has errors",
            };
            diagnostics.push(Diagnostic {
                range: read.range,
                message: unavailable_read(NameKind::Table, names, &definition.file, why),
                help: None,
                severity: Severity::Error,
                source: Source::Project,
                code: None,
                fixes: Vec::new(),
            });
        }
        diagnostics
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
            return Err("A table of the host has no rename".to_string());
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
        let errors = |project: &Project| -> Vec<Diagnostic> {
            (project.files())
                .flat_map(|path| project.diagnostics(path))
                .filter(|diagnostic| diagnostic.severity == Severity::Error)
                .filter(|diagnostic| diagnostic.source != Source::Loader)
                .collect()
        };
        let (before, after) = (errors(self), errors(&renamed));
        if after.len() > before.len() {
            let message = (after.iter())
                .find(|error| !before.contains(error))
                .or(after.first())
                .map(|error| error.message.clone())
                .unwrap_or_default();
            return Err(format!("The rename to `{name}` makes an error: {message}"));
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
        // A TEMP table's name is taken in its script, and another's in any data script.
        let taken = match temp_table(document, table) {
            true => defines(document, &renamed, true),
            false => (self.files.values()).any(|file| defines(&file.document, &renamed, true)),
        } || self.catalog.table(&renamed).is_some();
        if taken && !same_path(&renamed, &table.names) {
            return Err(format!(
                "The rename to `{name}` makes the name of another table, {}",
                renamed.join(".")
            ));
        }
        let replacement = quote(name);
        let edits = (self.table_references(path, table).into_iter())
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

    /// Applies `change`, and analyzes the files that it changes and their dependents again.
    fn update(&mut self, change: Change) {
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
        // The tables of the data scripts before the change, by path.
        let before: BTreeMap<String, Vec<Vec<String>>> = (self.files.iter())
            .filter_map(|(path, file)| {
                Some((path.clone(), exported(file.document.data_analysis()?)))
            })
            .collect();
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
        let graph = Graph::new(roots.iter().cloned());
        for (path, root) in &roots {
            if all_data || imports(root, &modules) {
                seeds.insert(path.clone());
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
        let unchanged = (self.files.iter())
            .filter(|(path, _)| !pending.contains_key(*path))
            .map(|(_, file)| (&file.role, file.document.text()));
        let changed = (pending.values()).map(|(role, text)| (role, text.as_str()));
        for (role, text) in unchanged.chain(changed) {
            if let FileRole::Module { names } = role {
                catalog = catalog.with_module(names.clone(), text.to_string());
            }
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
            }
            let analysis = self.files[path].document.data_analysis();
            catalog = catalog.with_analysis(path, analysis.expect("a data script"));
        }
        self.graph = graph;
        self.catalog = Arc::new(catalog);

        // The chart scripts that changed, that read a table that changed, or that import a
        // module that changed, or any module after a change of the data scripts, or that have
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
                (read.tables.iter())
                    .any(|read| tables.iter().any(|table| same_path(table, &read.names)))
                    || imports(&file.document.root(), &modules)
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
            self.files.insert(path, File { role, document });
        }
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
            && same_path(&definition.names, names)
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

/// Whether the script `root` imports one of the modules `modules`.
fn imports(root: &avenger_sql_syntax::SyntaxNode, modules: &[Vec<String>]) -> bool {
    (references(root).modules.iter()).any(|import| {
        modules
            .iter()
            .any(|module| same_path(module, &import.names))
    })
}

/// The URIs of the statements of the data script `document` that read files, each at the range
/// of its string.
fn uris(document: &Document) -> Vec<(TextRange, String)> {
    let root = document.root();
    (root.descendants_with_tokens())
        .filter_map(|element| element.into_token())
        .filter(|token| matches!(file_string(token), Some(FileString::Uri(_))))
        .filter_map(|token| {
            let literal = ast::Literal::cast(token.parent()?)?;
            Some((literal.range(), literal.string_value()?))
        })
        .collect()
}
