//! The tables that a script reads, and where they come from.
//!
//! The pass looks up tables through a [`Catalog`]. Before it analyzes a script, it collects the
//! paths of the tables and modules that the script names with [`references`], and a catalog
//! fetches those tables, which can be asynchronous, as [`SessionCatalog::load`] does from a
//! DataFusion session. The analysis then looks them up synchronously. [`MemoryCatalog`] holds
//! tables for tests.
//!
//! A data script's external tables and `LOAD DATA` statements read files, whose schemas a
//! loader reads before the analysis, as [`crate::data::file_reads`] collects them: the catalog
//! gives the table of each [`Files`] with [`Catalog::files`]. A catalog of a project also gives
//! the place that defines each table with [`Catalog::definition`].
//!
//! Names match without regard to case, as in GoogleSQL. A path of one name is a table of the
//! default schema, a path of two a schema and a table, and a path of three a catalog, a schema
//! and a table.

use std::sync::Arc;

use avenger_sql_syntax::{ast, ast::AstNode, SyntaxNode, TextRange};
use datafusion::{
    arrow::datatypes::SchemaRef, catalog::TableProvider, common::Result,
    datasource::empty::EmptyTable, prelude::SessionContext,
};

use crate::{analysis::StatementTarget, data::Files, types::TypeOptions};

/// A path that a script names, and the range of its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    /// The names of the path, without backticks.
    pub names: Vec<String>,
    pub range: TextRange,
}

/// The tables and modules that a script names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct References {
    /// The paths of the tables in `FROM`, joins, and `TABLE` arguments, in the order of the text,
    /// without the reads of a table that an earlier statement of the script defines, such as a
    /// temporary table or a chart script's own table of files. Some of them name a pipe `WITH`
    /// name, or a table that doesn't exist, which a catalog doesn't find.
    pub tables: Vec<Reference>,
    /// The paths of the modules of `IMPORT MODULE`.
    pub modules: Vec<Reference>,
}

/// The tables and modules that the tree `root` names. When `root` is a script, a read of a table
/// that an earlier statement defines reads that statement's table, and is not a reference.
pub fn references(root: &SyntaxNode) -> References {
    let mut references = References::default();
    let reference = |path: Option<ast::Path>| {
        let path = path?;
        Some(Reference {
            names: path.names()?,
            range: path.range(),
        })
    };
    let mut add = |node: &SyntaxNode, defined: &[Vec<String>]| {
        for node in node.descendants() {
            let table = if let Some(source) = ast::TableSource::cast(node.clone()) {
                reference(source.path())
            } else if let Some(arg) = ast::TableArg::cast(node.clone()) {
                reference(arg.path())
            } else if let Some(arg) = ast::NamedTableArg::cast(node.clone()) {
                reference(arg.path())
            } else {
                if let Some(import) = ast::ImportStmt::cast(node.clone()) {
                    references.modules.extend(reference(import.path()));
                }
                None
            };
            let table =
                table.filter(|table| !defined.iter().any(|names| same_path(names, &table.names)));
            references.tables.extend(table);
        }
    };
    match ast::Script::cast(root.clone()) {
        Some(_) => {
            let mut defined = Vec::new();
            for child in root.children() {
                add(&child, &defined);
                let statement = ast::Stmt::cast(child);
                match statement.as_ref().and_then(StatementTarget::of) {
                    Some(target) if !target.schema => defined.push(target.names),
                    _ => {}
                }
            }
        }
        None => add(root, &[]),
    }
    references
}

/// A table of a catalog: the names it has in the catalog, and its provider.
#[derive(Clone, Debug)]
pub struct Table {
    /// The names of the table in its catalog, which can differ in case from the names that a
    /// script writes.
    pub names: Vec<String>,
    pub provider: Arc<dyn TableProvider>,
}

impl Table {
    pub fn schema(&self) -> SchemaRef {
        self.provider.schema()
    }
}

/// The table that reads a set of files, or the error of reading them, such as a file that
/// doesn't exist.
pub type FilesTable = std::result::Result<Arc<dyn TableProvider>, String>;

/// The place that defines a table, a view, or a schema of a catalog, such as the statement of a
/// project's data script that creates it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableDefinition {
    /// The file that holds the definition, as its host names it, such as a path in a project.
    pub file: String,
    /// The range of the statement that defines it, in the text of `file`.
    pub range: TextRange,
    pub description: Option<String>,
}

/// Where the pass looks up the tables of a script. The catalog of a data script's analysis holds
/// the tables outside the script: those of the host and of the other data scripts of its
/// project.
pub trait Catalog {
    /// The table at the path `names`, matched without regard to case.
    fn table(&self, names: &[String]) -> Option<Table>;

    /// The tables of the catalog, which an editor offers where a script names a table.
    fn tables(&self) -> Vec<Table>;

    /// The text of the module at the path `names` of an `IMPORT MODULE`, a `.sqlm` file.
    fn module(&self, _names: &[String]) -> Option<String> {
        None
    }

    /// How GoogleSQL's types map to Arrow types.
    fn type_options(&self) -> TypeOptions {
        TypeOptions::default()
    }

    /// The table that reads the files `files`, of an external table or a `LOAD DATA`: a table
    /// of the files' schema, or the error of reading it. `None` when the catalog reads no
    /// files.
    fn files(&self, _files: &Files) -> Option<FilesTable> {
        None
    }

    /// Whether the catalog holds the schema at the path `names`, which a data script's tables
    /// can be in. By default, whether a table of the catalog is in it.
    fn has_schema(&self, names: &[String]) -> bool {
        in_schema(&self.tables(), names)
    }

    /// The place that defines the table, the view, or the schema at the path `names`, and its
    /// description.
    fn definition(&self, _names: &[String]) -> Option<TableDefinition> {
        None
    }
}

/// Whether the names `a` and `b` are the same name in GoogleSQL, which ignores case.
fn same_name(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Whether the paths `a` and `b` name the same table or module, without regard to case.
pub fn same_path(a: &[String], b: &[String]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_name(a, b))
}

/// A catalog of the tables that a script names, loaded from a DataFusion session.
#[derive(Clone, Debug, Default)]
pub struct SessionCatalog {
    tables: Vec<Table>,
    modules: Vec<(Vec<String>, String)>,
    type_options: TypeOptions,
}

impl SessionCatalog {
    /// The tables of `ctx` that `references` names. A path that names no table of `ctx` has no
    /// table in the catalog.
    pub async fn load(ctx: &SessionContext, references: &References) -> Result<Self> {
        let state = ctx.state();
        let options = state.config().options();
        let default_catalog = options.catalog.default_catalog.clone();
        let default_schema = options.catalog.default_schema.clone();
        let type_options = TypeOptions {
            utf8_view: options.sql_parser.map_string_types_to_utf8view,
        };
        let mut catalog = SessionCatalog {
            tables: Vec::new(),
            modules: Vec::new(),
            type_options,
        };
        for reference in &references.tables {
            let (catalog_name, schema_name, table_name) = match reference.names.as_slice() {
                [table] => (default_catalog.as_str(), default_schema.as_str(), table),
                [schema, table] => (default_catalog.as_str(), schema.as_str(), table),
                [catalog, schema, table] => (catalog.as_str(), schema.as_str(), table),
                _ => continue,
            };
            if catalog.table(&reference.names).is_some() {
                continue;
            }
            let Some(catalog_name) = find(ctx.catalog_names(), catalog_name) else {
                continue;
            };
            let Some(catalog_provider) = ctx.catalog(&catalog_name) else {
                continue;
            };
            let Some(schema_name) = find(catalog_provider.schema_names(), schema_name) else {
                continue;
            };
            let Some(schema) = catalog_provider.schema(&schema_name) else {
                continue;
            };
            let Some(table_name) = find(schema.table_names(), table_name) else {
                continue;
            };
            let Some(provider) = schema.table(&table_name).await? else {
                continue;
            };
            // The names that a script writes, so a lookup by them finds the table.
            catalog.tables.push(Table {
                names: reference.names.clone(),
                provider,
            });
        }
        Ok(catalog)
    }

    /// The catalog with the modules that `modules` gives for the paths that `references` imports,
    /// and for the paths that those modules import in turn. `modules` returns the text of the
    /// module at a path, or `None` when it has none.
    pub fn with_modules(
        mut self,
        references: &References,
        modules: &dyn Fn(&[String]) -> Option<String>,
    ) -> Self {
        let mut pending: Vec<Vec<String>> = (references.modules.iter())
            .map(|reference| reference.names.clone())
            .collect();
        while let Some(names) = pending.pop() {
            if self.module(&names).is_some() {
                continue;
            }
            let Some(text) = modules(&names) else {
                continue;
            };
            let parse = avenger_sql_syntax::parse_module(&text);
            let imports = self::references(&parse.syntax()).modules;
            pending.extend(imports.into_iter().map(|reference| reference.names));
            self.modules.push((names, text));
        }
        self
    }
}

/// The name in `names` that is `name` without regard to case, preferring the one with its case.
pub fn find(names: Vec<String>, name: &str) -> Option<String> {
    if names.iter().any(|candidate| candidate == name) {
        return Some(name.to_string());
    }
    names
        .into_iter()
        .find(|candidate| same_name(candidate, name))
}

impl Catalog for SessionCatalog {
    fn table(&self, names: &[String]) -> Option<Table> {
        lookup(&self.tables, names)
    }

    /// The tables that the catalog loaded, which are those that the script names.
    fn tables(&self) -> Vec<Table> {
        self.tables.clone()
    }

    fn module(&self, names: &[String]) -> Option<String> {
        (self.modules.iter())
            .find(|(path, _)| same_path(path, names))
            .map(|(_, text)| text.clone())
    }

    fn type_options(&self) -> TypeOptions {
        self.type_options
    }
}

fn lookup(tables: &[Table], names: &[String]) -> Option<Table> {
    (tables.iter())
        .find(|table| same_path(&table.names, names))
        .cloned()
}

/// Whether a table of `tables` is in the schema at the path `names`.
fn in_schema(tables: &[Table], names: &[String]) -> bool {
    (tables.iter()).any(|table| {
        table.names.len() == names.len() + 1 && same_path(&table.names[..names.len()], names)
    })
}

/// A catalog of tables, modules, schemas, tables of files, and definitions that a test or a
/// host adds.
#[derive(Clone, Debug, Default)]
pub struct MemoryCatalog {
    tables: Vec<Table>,
    modules: Vec<(Vec<String>, String)>,
    schemas: Vec<Vec<String>>,
    files: Vec<(Files, FilesTable)>,
    definitions: Vec<(Vec<String>, TableDefinition)>,
}

fn path(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

impl MemoryCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the table `provider` at the path `names`, which is one name, or a schema and a table,
    /// or a catalog, a schema and a table.
    pub fn with_table(mut self, names: &[&str], provider: Arc<dyn TableProvider>) -> Self {
        self.tables.push(Table {
            names: names.iter().map(|name| name.to_string()).collect(),
            provider,
        });
        self
    }

    /// Adds an empty table of `schema` at the path `names`.
    pub fn with_schema(self, names: &[&str], schema: SchemaRef) -> Self {
        self.with_table(names, Arc::new(EmptyTable::new(schema)))
    }

    /// Adds the module of the text `text` at the path `names`.
    pub fn with_module(mut self, names: &[&str], text: &str) -> Self {
        self.modules.push((path(names), text.to_string()));
        self
    }

    /// Adds the schema at the path `names`, a namespace of tables, without tables.
    pub fn with_namespace(mut self, names: &[&str]) -> Self {
        self.schemas.push(path(names));
        self
    }

    /// Adds `provider` as the table that reads the files `files`.
    pub fn with_files(mut self, files: Files, provider: Arc<dyn TableProvider>) -> Self {
        self.files.push((files, Ok(provider)));
        self
    }

    /// Adds an empty table of `schema` as the table that reads the files `files`.
    pub fn with_files_schema(self, files: Files, schema: SchemaRef) -> Self {
        self.with_files(files, Arc::new(EmptyTable::new(schema)))
    }

    /// Adds `error` as the error of reading the files `files`.
    pub fn with_files_error(mut self, files: Files, error: &str) -> Self {
        self.files.push((files, Err(error.to_string())));
        self
    }

    /// Adds `definition` as the place that defines the table, the view, or the schema at the
    /// path `names`.
    pub fn with_definition(mut self, names: &[&str], definition: TableDefinition) -> Self {
        self.definitions.push((path(names), definition));
        self
    }
}

impl Catalog for MemoryCatalog {
    fn table(&self, names: &[String]) -> Option<Table> {
        lookup(&self.tables, names)
    }

    fn tables(&self) -> Vec<Table> {
        self.tables.clone()
    }

    fn module(&self, names: &[String]) -> Option<String> {
        (self.modules.iter())
            .find(|(path, _)| same_path(path, names))
            .map(|(_, text)| text.clone())
    }

    /// The table of `files` that the catalog holds. A catalog that holds no tables of files
    /// reads no files.
    fn files(&self, files: &Files) -> Option<FilesTable> {
        (self.files.iter())
            .find(|(held, _)| held == files)
            .map(|(_, provider)| provider.clone())
    }

    fn has_schema(&self, names: &[String]) -> bool {
        (self.schemas.iter()).any(|schema| same_path(schema, names))
            || in_schema(&self.tables, names)
    }

    fn definition(&self, names: &[String]) -> Option<TableDefinition> {
        (self.definitions.iter())
            .find(|(path, _)| same_path(path, names))
            .map(|(_, definition)| definition.clone())
    }
}
