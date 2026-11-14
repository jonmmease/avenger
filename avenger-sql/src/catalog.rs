//! The tables that a script reads, and where they come from.
//!
//! The pass looks up tables through a [`Catalog`]. Before it analyzes a script, it collects the
//! paths of the tables and modules that the script names with [`references`], and a catalog
//! fetches those tables, which can be asynchronous, as [`SessionCatalog::load`] does from a
//! DataFusion session. The analysis then looks them up synchronously. [`MemoryCatalog`] holds
//! tables for tests.
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

use crate::types::TypeOptions;

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
    /// The paths of the tables in `FROM`, joins, and `TABLE` arguments, in the order of the text.
    /// Some of them name a temporary table, a pipe `WITH` name, or a table that doesn't exist,
    /// which a catalog doesn't find.
    pub tables: Vec<Reference>,
    /// The paths of the modules of `IMPORT MODULE`.
    pub modules: Vec<Reference>,
}

/// The tables and modules that the tree `root` names.
pub fn references(root: &SyntaxNode) -> References {
    let mut references = References::default();
    let reference = |path: Option<ast::Path>| {
        let path = path?;
        Some(Reference {
            names: path.names()?,
            range: path.range(),
        })
    };
    for node in root.descendants() {
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
        references.tables.extend(table);
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

/// Where the pass looks up the tables of a script.
pub trait Catalog {
    /// The table at the path `names`, matched without regard to case.
    fn table(&self, names: &[String]) -> Option<Table>;

    /// The text of the module at the path `names` of an `IMPORT MODULE`, a `.sqlm` file.
    fn module(&self, _names: &[String]) -> Option<String> {
        None
    }

    /// How GoogleSQL's types map to Arrow types.
    fn type_options(&self) -> TypeOptions {
        TypeOptions::default()
    }
}

/// Whether the names `a` and `b` are the same name in GoogleSQL, which ignores case.
fn same_name(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// A catalog of the tables that a script names, loaded from a DataFusion session.
#[derive(Clone, Debug, Default)]
pub struct SessionCatalog {
    tables: Vec<Table>,
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
}

/// The name in `names` that is `name` without regard to case, preferring the one with its case.
fn find(names: Vec<String>, name: &str) -> Option<String> {
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

    fn type_options(&self) -> TypeOptions {
        self.type_options
    }
}

fn lookup(tables: &[Table], names: &[String]) -> Option<Table> {
    tables
        .iter()
        .find(|table| {
            table.names.len() == names.len()
                && (table.names.iter())
                    .zip(names)
                    .all(|(a, b)| same_name(a, b))
        })
        .cloned()
}

/// A catalog of tables and modules that a test adds.
#[derive(Clone, Debug, Default)]
pub struct MemoryCatalog {
    tables: Vec<Table>,
    modules: Vec<(Vec<String>, String)>,
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
        let names = names.iter().map(|name| name.to_string()).collect();
        self.modules.push((names, text.to_string()));
        self
    }
}

impl Catalog for MemoryCatalog {
    fn table(&self, names: &[String]) -> Option<Table> {
        lookup(&self.tables, names)
    }

    fn module(&self, names: &[String]) -> Option<String> {
        (self.modules.iter())
            .find(|(path, _)| {
                path.len() == names.len()
                    && path
                        .iter()
                        .zip(names)
                        .all(|(a, b)| a.eq_ignore_ascii_case(b))
            })
            .map(|(_, text)| text.clone())
    }
}
