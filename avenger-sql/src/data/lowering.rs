//! Lowering a data script to DataFusion: the table, the view, or the schema of each of its
//! definitions, registered in a session, and the tables of a chart script's own tables of files,
//! which its plans read without a registration.

use std::{collections::HashMap, sync::Arc};

use datafusion::{
    arrow::{array::RecordBatch, datatypes::SchemaRef},
    catalog::{CatalogProvider, MemorySchemaProvider, SchemaProvider, TableProvider},
    common::{tree_node::Transformed, Column, DataFusionError, Result},
    datasource::{provider_as_source, source_as_provider, MemTable, ViewTable},
    logical_expr::{cast, Expr, LogicalPlan, TableScan},
    prelude::SessionContext,
};

use super::{DataAnalysis, Definition, DefinitionKind, Files, ScriptTable};
use crate::{
    analysis::{Analysis, StatementPlan},
    catalog::{find, Catalog},
};

/// A table, a view, or a schema that [`lower`] registers in a session, for a definition of the
/// script that the script exports.
#[derive(Clone, Debug)]
pub struct Registered {
    /// The index of the definition in [`DataAnalysis::definitions`], whose kind tells what it is
    /// and whose names name it.
    pub definition: usize,
    pub registration: Registration,
}

/// What [`lower`] registers for a definition.
#[derive(Clone, Debug)]
pub enum Registration {
    /// The table of an external table, a `LOAD DATA`, a view, or a `CREATE TABLE ... AS`.
    Table(Arc<dyn TableProvider>),
    /// The schema of a `CREATE SCHEMA`, in the default catalog of the session.
    Schema(Arc<dyn SchemaProvider>),
}

/// The error of lowering a definition, such as a file that fails to read or a query that fails
/// to run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoweringError {
    /// The index of the definition in [`DataAnalysis::definitions`].
    pub definition: usize,
    pub message: String,
}

/// Lowers the data script of `analysis`, an analysis without errors, into the session `ctx`,
/// and returns what it registers, in the order of the text, or the error of each definition that
/// fails.
///
/// Each definition becomes a table: an external table is the table of its files, which
/// `catalog` gives, as [`Catalog::files`] gave it to the analysis, and a view is a `ViewTable` of
/// its plan. A `LOAD DATA` runs a scan of the table of its files and keeps its rows in a
/// `MemTable`, and an append of `LOAD DATA INTO` keeps the rows of its table and those of its
/// files, by the names of the table's columns, in a new `MemTable`. A `CREATE TABLE ... AS` runs
/// its plan and keeps its rows. Before a plan runs or a view keeps it, each scan of a
/// [`ScriptTable`] reads the table of the definition whose rows it sees, so a view over a
/// `LOAD DATA` reads the rows in memory, as they were when the view's statement ran.
///
/// The definitions that the script exports, as [`DataAnalysis::exported`] gives them, then
/// register in `ctx`: a schema in the default catalog when it lacks it, and then each table in its
/// schema, in place of the tables of the schema of the same name without regard to case, such as
/// a table of the host. A `TEMP` definition, and one that a later definition replaces, registers
/// nothing. When a definition fails, the script registers nothing, and when a registration fails,
/// the session keeps the schemas and the tables that it had before. The plans run in `ctx`, so it
/// has avenger-sql's [`crate::register`].
pub async fn lower(
    ctx: &SessionContext,
    analysis: &DataAnalysis,
    catalog: &dyn Catalog,
) -> std::result::Result<Vec<Registered>, Vec<LoweringError>> {
    let lowering = Lowering::run(ctx, &analysis.definitions, catalog).await?;
    register(ctx, analysis, &lowering).map_err(|error| vec![error])
}

/// The tables of the definitions of a chart script's own tables of files, as
/// [`Analysis::file_tables`] gives them, which the script's plans read in place of their
/// placeholders. They register in no session, so no other script reads them.
#[derive(Clone, Debug, Default)]
pub struct ScriptTables {
    /// The table of each definition, as the reads after it see it.
    tables: Vec<Option<Arc<dyn TableProvider>>>,
    /// The schema of each definition, which the scans of its placeholder keep.
    schemas: Vec<Option<SchemaRef>>,
}

impl ScriptTables {
    /// Builds the table of each of `definitions`, in the order of the text, as [`lower`] builds
    /// a data script's: the table of the files of an external table, which `catalog` gives as
    /// [`Catalog::files`] gave it to the analysis, and a `MemTable` of the rows of a `LOAD DATA`,
    /// which it reads now. The error is that of each definition that fails.
    pub async fn build(
        ctx: &SessionContext,
        definitions: &[Definition],
        catalog: &dyn Catalog,
    ) -> std::result::Result<ScriptTables, Vec<LoweringError>> {
        Ok(Lowering::run(ctx, definitions, catalog).await?.tables)
    }

    /// `plan` with each scan of a [`ScriptTable`] reading the table of its definition, including
    /// the scans of its subqueries, or the error of a scan whose definition has no table or a
    /// table of other columns.
    pub fn replace(&self, plan: &LogicalPlan) -> std::result::Result<LogicalPlan, String> {
        let mut failed = None;
        let plan = (plan.clone().transform_down_with_subqueries(|node| {
            let LogicalPlan::TableScan(scan) = &node else {
                return Ok(Transformed::no(node));
            };
            let Ok(provider) = source_as_provider(&scan.source) else {
                return Ok(Transformed::no(node));
            };
            let Some(script) = provider.downcast_ref::<ScriptTable>() else {
                return Ok(Transformed::no(node));
            };
            let name = script.names().join(".");
            let index = script.definition();
            let Some(table) = self.tables.get(index).cloned().flatten() else {
                failed.get_or_insert_with(|| {
                    format!("{name} is a table whose earlier statement of this script failed")
                });
                return Ok(Transformed::no(node));
            };
            // The parents of the scan keep the schema of the placeholder.
            if (self.schemas.get(index).cloned().flatten())
                .is_some_and(|schema| schema != table.schema())
            {
                failed.get_or_insert_with(|| {
                    format!("The table of {name} has columns other than those of its statement")
                });
                return Ok(Transformed::no(node));
            }
            let LogicalPlan::TableScan(scan) = node else {
                unreachable!("the node is a scan")
            };
            Ok(Transformed::yes(LogicalPlan::TableScan(TableScan {
                source: provider_as_source(table),
                ..scan
            })))
        }))
        .map_err(|error| error.to_string())?
        .data;
        match failed {
            Some(error) => Err(error),
            None => Ok(plan),
        }
    }
}

impl Analysis {
    /// The analysis with each plan of its statements reading `tables`, the tables of its own
    /// tables of files, in place of their placeholders, as [`ScriptTables::replace`] replaces
    /// them, so that [`Analysis::dataframe`] runs it.
    pub fn with_tables(mut self, tables: &ScriptTables) -> Result<Analysis> {
        let replace = |plan: &mut LogicalPlan| -> Result<()> {
            *plan = tables.replace(plan).map_err(DataFusionError::Plan)?;
            Ok(())
        };
        for statement in &mut self.statements {
            match &mut statement.plan {
                StatementPlan::Query(plan) | StatementPlan::Variable { plan, .. } => replace(plan)?,
                StatementPlan::Definition => {}
            }
        }
        if let Some(plan) = &mut self.plan {
            replace(plan)?;
        }
        Ok(self)
    }
}

/// The state of the lowering of a script, by the index of each definition.
struct Lowering {
    /// The table and the schema of each definition, as the reads after it see it.
    tables: ScriptTables,
    /// The rows of each definition that keeps rows, a `LOAD DATA` or a `CREATE TABLE ... AS`,
    /// with the rows of the loads before it for an append.
    rows: Vec<Option<Vec<Vec<RecordBatch>>>>,
    /// The last append of `LOAD DATA INTO` to each table that has one, by its definition.
    last_load: HashMap<usize, usize>,
}

impl Lowering {
    /// Builds the table of each of `definitions` in `ctx`, in the order of the text, or gives the
    /// error of each that fails.
    async fn run(
        ctx: &SessionContext,
        definitions: &[Definition],
        catalog: &dyn Catalog,
    ) -> std::result::Result<Lowering, Vec<LoweringError>> {
        let mut lowering = Lowering {
            tables: ScriptTables {
                tables: vec![None; definitions.len()],
                schemas: (definitions.iter())
                    .map(|definition| definition.schema.clone())
                    .collect(),
            },
            rows: vec![None; definitions.len()],
            last_load: HashMap::new(),
        };
        let mut errors = Vec::new();
        for index in 0..definitions.len() {
            match lowering.definition(ctx, catalog, definitions, index).await {
                Ok(table) => lowering.tables.tables[index] = table,
                Err(message) => errors.push(LoweringError {
                    definition: index,
                    message,
                }),
            }
        }
        match errors.is_empty() {
            true => Ok(lowering),
            false => Err(errors),
        }
    }

    /// The table of the definition at `index` of `definitions`, or `None` for a schema.
    async fn definition(
        &mut self,
        ctx: &SessionContext,
        catalog: &dyn Catalog,
        definitions: &[Definition],
        index: usize,
    ) -> std::result::Result<Option<Arc<dyn TableProvider>>, String> {
        let definition = &definitions[index];
        let name = definition.names.join(".");
        let table: Arc<dyn TableProvider> = match &definition.kind {
            DefinitionKind::Schema => return Ok(None),
            DefinitionKind::ExternalTable { files } => files_table(catalog, files)?,
            DefinitionKind::View { plan } => {
                let plan = plan
                    .as_ref()
                    .ok_or_else(|| format!("The query of the view {name} has errors"))?;
                Arc::new(ViewTable::new(self.tables.replace(plan)?, None))
            }
            DefinitionKind::TableAs { plan } => {
                let (Some(plan), Some(schema)) = (plan, &definition.schema) else {
                    return Err(format!("The query of the table {name} has errors"));
                };
                let rows = run(ctx, self.tables.replace(plan)?, schema).await?;
                self.keep(index, schema, rows)?
            }
            DefinitionKind::LoadData { files, appends, .. } => {
                let schema = (definition.schema.as_ref())
                    .ok_or_else(|| format!("The files of {name} have no schema"))?;
                let mut rows = match appends {
                    None => Vec::new(),
                    Some(root) => {
                        // The last load of the table before this one holds its rows.
                        let last = self.last_load.insert(*root, index).unwrap_or(*root);
                        self.rows[last].clone().ok_or_else(|| {
                            format!(
                                "LOAD DATA INTO {name} appends to a table whose earlier statement failed"
                            )
                        })?
                    }
                };
                rows.extend(load(ctx, files_table(catalog, files)?, schema).await?);
                self.keep(index, schema, rows)?
            }
        };
        Ok(Some(table))
    }

    /// A `MemTable` of the rows `rows`, of the schema `schema`, which the definition at `index`
    /// keeps.
    fn keep(
        &mut self,
        index: usize,
        schema: &SchemaRef,
        rows: Vec<Vec<RecordBatch>>,
    ) -> std::result::Result<Arc<dyn TableProvider>, String> {
        let mut partitions = rows.clone();
        if partitions.is_empty() {
            partitions.push(Vec::new());
        }
        let table =
            MemTable::try_new(schema.clone(), partitions).map_err(|error| error.to_string())?;
        self.rows[index] = Some(rows);
        Ok(Arc::new(table))
    }

    /// The table of the table, view, or `LOAD DATA` at `index` that the session registers: for
    /// a table with appends, that of the last.
    fn registered(&self, index: usize) -> Option<Arc<dyn TableProvider>> {
        let last = self.last_load.get(&index).copied().unwrap_or(index);
        self.tables.tables[last].clone()
    }
}

/// The table of the files `files` that `catalog` gives.
fn files_table(
    catalog: &dyn Catalog,
    files: &Option<Files>,
) -> std::result::Result<Arc<dyn TableProvider>, String> {
    let files = files
        .as_ref()
        .ok_or_else(|| "The options of the files have errors".to_string())?;
    catalog
        .files(files)
        .unwrap_or_else(|| Err("The catalog of the loader reads no files".to_string()))
}

/// The rows of `plan` in the partitions of its run in `ctx`, of the schema `schema`.
async fn run(
    ctx: &SessionContext,
    plan: LogicalPlan,
    schema: &SchemaRef,
) -> std::result::Result<Vec<Vec<RecordBatch>>, String> {
    let rows = async {
        let partitions = ctx
            .execute_logical_plan(plan)
            .await?
            .collect_partitioned()
            .await?;
        with_schema(schema, partitions)
    };
    rows.await.map_err(|error| error.to_string())
}

/// The rows of the table `files`, a table of files, with the columns of `schema`, which the
/// table's columns of the same names, without regard to case, cast to.
async fn load(
    ctx: &SessionContext,
    files: Arc<dyn TableProvider>,
    schema: &SchemaRef,
) -> std::result::Result<Vec<Vec<RecordBatch>>, String> {
    let rows = async {
        let source = files.schema();
        let mut frame = ctx.read_table(files)?;
        if source != *schema {
            let columns = (schema.fields().iter())
                .map(|field| {
                    let name = (source.fields().iter())
                        .find(|column| column.name().eq_ignore_ascii_case(field.name()))
                        .map_or(field.name(), |column| column.name());
                    cast(
                        Expr::Column(Column::new_unqualified(name)),
                        field.data_type().clone(),
                    )
                    .alias(field.name())
                })
                .collect::<Vec<_>>();
            frame = frame.select(columns)?;
        }
        with_schema(schema, frame.collect_partitioned().await?)
    };
    rows.await.map_err(|error| error.to_string())
}

/// `partitions` with the schema `schema`, whose columns they have in its order.
fn with_schema(
    schema: &SchemaRef,
    partitions: Vec<Vec<RecordBatch>>,
) -> Result<Vec<Vec<RecordBatch>>> {
    (partitions.into_iter())
        .map(|batches| {
            (batches.into_iter())
                .map(|batch| {
                    Ok(RecordBatch::try_new(
                        schema.clone(),
                        batch.columns().to_vec(),
                    )?)
                })
                .collect()
        })
        .collect()
}

/// The name of the schema of the default catalog of `ctx` that is `name` without regard to case,
/// preferring the one with its case, or `name` when the catalog has none, as a reference to a
/// table in the schema writes it.
pub fn schema_name(ctx: &SessionContext, name: &str) -> String {
    let catalog_name = ctx.state().config_options().catalog.default_catalog.clone();
    let schemas = ctx
        .catalog(&catalog_name)
        .map(|catalog| catalog.schema_names());
    (schemas.and_then(|schemas| find(schemas, name))).unwrap_or_else(|| name.to_string())
}

/// Registers in `ctx` the definitions that `analysis` exports, with the tables of `lowering`, all
/// or none: it finds the schema of each table, in the session or among the schemas that the
/// script creates, before it changes the session, and when a registration fails, it restores the
/// schemas and the tables that the session had.
fn register(
    ctx: &SessionContext,
    analysis: &DataAnalysis,
    lowering: &Lowering,
) -> std::result::Result<Vec<Registered>, LoweringError> {
    let definitions = &analysis.definitions;
    let options = ctx.state().config_options().catalog.clone();
    let exported: Vec<usize> = (analysis.exported())
        .map(|exported| {
            (definitions.iter())
                .position(|definition| std::ptr::eq(definition, exported))
                .expect("an exported definition is a definition of the analysis")
        })
        .collect();
    let Some(&first) = exported.first() else {
        return Ok(Vec::new());
    };
    let catalog = ctx
        .catalog(&options.default_catalog)
        .ok_or_else(|| LoweringError {
            definition: first,
            message: format!("The session has no catalog {}", options.default_catalog),
        })?;
    // The schema of each schema that the script exports, and those that the session lacks,
    // which the script creates, by name.
    let mut schemas: Vec<(usize, Arc<dyn SchemaProvider>)> = Vec::new();
    let mut created: Vec<(usize, String, Arc<dyn SchemaProvider>)> = Vec::new();
    for &index in &exported {
        let definition = &definitions[index];
        if !matches!(definition.kind, DefinitionKind::Schema) {
            continue;
        }
        let name = &definition.names[0];
        let existing = (find(catalog.schema_names(), name)).and_then(|name| catalog.schema(&name));
        let schema = existing.unwrap_or_else(|| {
            let schema: Arc<dyn SchemaProvider> = Arc::new(MemorySchemaProvider::new());
            created.push((index, name.clone(), schema.clone()));
            schema
        });
        schemas.push((index, schema));
    }
    // The schema of each table and the name of the table in it.
    let mut tables: Vec<(usize, Arc<dyn SchemaProvider>, String)> = Vec::new();
    for &index in &exported {
        let definition = &definitions[index];
        if matches!(definition.kind, DefinitionKind::Schema) {
            continue;
        }
        let (schema, name) = match definition.names.as_slice() {
            [name] => (options.default_schema.as_str(), name),
            [schema, name] => (schema.as_str(), name),
            _ => unreachable!("a data script names a table by one or two names"),
        };
        let provider = (find(catalog.schema_names(), schema))
            .and_then(|existing| catalog.schema(&existing))
            .or_else(|| {
                (created.iter())
                    .find(|(_, created, _)| created.eq_ignore_ascii_case(schema))
                    .map(|(_, _, provider)| provider.clone())
            })
            .ok_or_else(|| LoweringError {
                definition: index,
                message: format!("The session has no schema {schema}"),
            })?;
        tables.push((index, provider, name.clone()));
    }
    let mut changes = Changes::default();
    let applied = changes.apply(&*catalog, &created, &tables, lowering);
    if let Err(error) = applied {
        changes.undo(&*catalog);
        return Err(error);
    }
    let registered = (exported.iter())
        .map(|&index| {
            let registration = match (schemas.iter()).find(|(schema, _)| *schema == index) {
                Some((_, schema)) => Registration::Schema(schema.clone()),
                None => Registration::Table(
                    (lowering.registered(index))
                        .expect("a definition of a table has a table after lowering"),
                ),
            };
            Registered {
                definition: index,
                registration,
            }
        })
        .collect();
    Ok(registered)
}

/// The changes of the session that [`register`] made, which it undoes when a registration fails.
#[derive(Default)]
struct Changes {
    /// The schemas that it created, by name.
    schemas: Vec<String>,
    /// Each table that it registered or tried to.
    tables: Vec<TableChange>,
}

/// A table that [`register`] registered or tried to.
struct TableChange {
    schema: Arc<dyn SchemaProvider>,
    name: String,
    registered: bool,
    /// The tables of the schema of the same name that it replaced, by their names.
    replaced: Vec<(String, Arc<dyn TableProvider>)>,
}

impl Changes {
    /// Creates the schemas `created` in `catalog`, and registers the table of each of `tables`
    /// in its schema, in place of the tables of the same name without regard to case, or gives
    /// the error of the first that fails.
    fn apply(
        &mut self,
        catalog: &dyn CatalogProvider,
        created: &[(usize, String, Arc<dyn SchemaProvider>)],
        tables: &[(usize, Arc<dyn SchemaProvider>, String)],
        lowering: &Lowering,
    ) -> std::result::Result<(), LoweringError> {
        for (index, name, schema) in created {
            catalog
                .register_schema(name, schema.clone())
                .map_err(|failure| LoweringError {
                    definition: *index,
                    message: failure.to_string(),
                })?;
            self.schemas.push(name.clone());
        }
        for (index, schema, name) in tables {
            let error = |failure: DataFusionError| LoweringError {
                definition: *index,
                message: failure.to_string(),
            };
            let table = (lowering.registered(*index))
                .expect("a definition of a table has a table after lowering");
            self.tables.push(TableChange {
                schema: schema.clone(),
                name: name.clone(),
                registered: false,
                replaced: Vec::new(),
            });
            let change = self.tables.last_mut().expect("the change of the table");
            for existing in schema.table_names() {
                if existing.eq_ignore_ascii_case(name) {
                    if let Some(previous) = schema.deregister_table(&existing).map_err(error)? {
                        change.replaced.push((existing, previous));
                    }
                }
            }
            schema.register_table(name.clone(), table).map_err(error)?;
            change.registered = true;
        }
        Ok(())
    }

    /// Restores the tables that [`Changes::apply`] replaced, and removes the schemas that it
    /// created from `catalog`.
    fn undo(self, catalog: &dyn CatalogProvider) {
        for change in self.tables.into_iter().rev() {
            if change.registered {
                let _ = change.schema.deregister_table(&change.name);
            }
            for (name, table) in change.replaced {
                let _ = change.schema.register_table(name, table);
            }
        }
        for name in self.schemas.iter().rev() {
            let _ = catalog.deregister_schema(name, true);
        }
    }
}
