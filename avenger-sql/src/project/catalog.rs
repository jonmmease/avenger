//! The catalog of a project: the tables that its data scripts define, over a base catalog.

use std::{fmt, sync::Arc};

use datafusion::{catalog::TableProvider, datasource::empty::EmptyTable};

use super::NameKind;
use crate::{
    catalog::{same_path, Catalog, FilesTable, Table, TableDefinition},
    data::{DataAnalysis, Definition, DefinitionKind, Files},
    types::TypeOptions,
};

/// The catalog of a project, which the analysis of its chart scripts and of its data scripts
/// reads: the tables, views, and schemas that the data scripts define without `TEMP`, each with
/// the place that defines it, the tables of the file sets that the data scripts read, and the
/// modules, over a base catalog of the host's tables, or of a session's tables when a loader
/// loads the project.
///
/// A lookup finds the names of the data scripts first, and then the base's, so a script's table
/// hides a table of the base at the same path, and so does a script's definition that has no
/// table, after an error. A name that two data scripts define, which the project's
/// [`super::Graph`] reports, finds the one added first. A path of one name that is both a table
/// and a schema finds the table's definition.
#[derive(Clone)]
pub struct ProjectCatalog {
    base: Arc<dyn Catalog + Send + Sync>,
    /// Each table, view, and schema of the data scripts, in the order that they were added.
    definitions: Vec<ScriptName>,
    files: Vec<(Files, FilesTable)>,
    modules: Vec<(Vec<String>, String)>,
}

impl ProjectCatalog {
    /// The catalog of the tables of `base` alone.
    pub fn new(base: Arc<dyn Catalog + Send + Sync>) -> ProjectCatalog {
        ProjectCatalog {
            base,
            definitions: Vec::new(),
            files: Vec::new(),
            modules: Vec::new(),
        }
    }

    /// Adds the definitions that the analysis `analysis` of the data script at `path` exports,
    /// each table as an empty table of its schema.
    pub fn with_analysis(self, path: &str, analysis: &DataAnalysis) -> ProjectCatalog {
        (analysis.exported()).fold(self, |catalog, definition| {
            catalog.with_definition(path, definition, None)
        })
    }

    /// Adds `definition`, a definition of the data script at `path`, with `provider` as its
    /// table, or an empty table of its schema without one. A table whose schema is unknown, after
    /// an error of its statement, and that has no `provider`, has a definition and no table, and
    /// hides a table of the base at its path.
    pub fn with_definition(
        mut self,
        path: &str,
        definition: &Definition,
        provider: Option<Arc<dyn TableProvider>>,
    ) -> ProjectCatalog {
        let place = TableDefinition {
            file: path.to_string(),
            range: definition.range,
            description: definition.description.clone(),
        };
        let kind = match definition.kind {
            DefinitionKind::Schema => NameKind::Schema,
            _ => NameKind::Table,
        };
        let table = match kind {
            NameKind::Table => provider
                .or_else(|| {
                    (definition.schema.clone())
                        .map(|schema| Arc::new(EmptyTable::new(schema)) as Arc<dyn TableProvider>)
                })
                .map(|provider| Table {
                    names: definition.names.clone(),
                    provider,
                }),
            NameKind::Schema => None,
        };
        self.definitions.push(ScriptName {
            names: definition.names.clone(),
            kind,
            place,
            table,
        });
        self
    }

    /// Adds `table` as the table that reads the files `files`, or the error of reading them.
    pub fn with_files(mut self, files: Files, table: FilesTable) -> ProjectCatalog {
        self.files.push((files, table));
        self
    }

    /// Adds the module of the text `text` at the path `names`.
    pub fn with_module(mut self, names: Vec<String>, text: String) -> ProjectCatalog {
        self.modules.push((names, text));
        self
    }

    /// The first definition of the kind `kind` at the path `names`, with a path of a table of
    /// one name in the base's default schema.
    fn script_name(&self, kind: NameKind, names: &[String]) -> Option<&ScriptName> {
        let default_schema = self.base.default_schema();
        (self.definitions.iter())
            .find(|name| name.kind == kind && kind.same(&name.names, names, &default_schema))
    }

    /// The tables of the data scripts.
    fn script_tables(&self) -> impl Iterator<Item = &Table> {
        self.definitions
            .iter()
            .filter_map(|name| name.table.as_ref())
    }
}

/// A table, a view, or a schema of a data script: its path, its kind, the place that defines it,
/// and its table, which a schema, and a table after an error of its statement, lack.
#[derive(Clone, Debug)]
struct ScriptName {
    names: Vec<String>,
    kind: NameKind,
    place: TableDefinition,
    table: Option<Table>,
}

impl fmt::Debug for ProjectCatalog {
    /// The catalog without its base, whose catalog need not be `Debug`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProjectCatalog")
            .field("definitions", &self.definitions)
            .field("files", &self.files)
            .field("modules", &self.modules)
            .finish_non_exhaustive()
    }
}

impl Catalog for ProjectCatalog {
    /// The table of the first definition of a table or a view of the data scripts at `names`,
    /// which has none after an error, or else the base's.
    fn table(&self, names: &[String]) -> Option<Table> {
        match self.script_name(NameKind::Table, names) {
            Some(name) => name.table.clone(),
            None => self.base.table(names),
        }
    }

    /// The tables of the data scripts, and then the tables of the base that their definitions do
    /// not hide.
    fn tables(&self) -> Vec<Table> {
        let mut tables: Vec<Table> = self.script_tables().cloned().collect();
        tables.extend(
            (self.base.tables().into_iter())
                .filter(|table| (self.script_name(NameKind::Table, &table.names)).is_none()),
        );
        tables
    }

    fn module(&self, names: &[String]) -> Option<String> {
        (self.modules.iter())
            .find(|(path, _)| same_path(path, names))
            .map(|(_, text)| text.clone())
            .or_else(|| self.base.module(names))
    }

    fn type_options(&self) -> TypeOptions {
        self.base.type_options()
    }

    fn default_schema(&self) -> String {
        self.base.default_schema()
    }

    fn files(&self, files: &Files) -> Option<FilesTable> {
        (self.files.iter())
            .find(|(held, _)| held == files)
            .map(|(_, table)| table.clone())
            .or_else(|| self.base.files(files))
    }

    /// Whether a data script creates the schema `names`, or holds a table in it, or the base
    /// holds it.
    fn has_schema(&self, names: &[String]) -> bool {
        self.script_name(NameKind::Schema, names).is_some()
            || self.script_tables().any(|table| {
                table.names.len() == names.len() + 1
                    && same_path(&table.names[..names.len()], names)
            })
            || self.base.has_schema(names)
    }

    /// The definition of a table or a view of the data scripts, then of a schema of the data
    /// scripts, then of the base.
    fn definition(&self, names: &[String]) -> Option<TableDefinition> {
        (self.script_name(NameKind::Table, names))
            .or_else(|| self.script_name(NameKind::Schema, names))
            .map(|name| name.place.clone())
            .or_else(|| self.base.definition(names))
    }
}
