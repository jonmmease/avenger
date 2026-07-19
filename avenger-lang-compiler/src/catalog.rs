//! DataFusion-backed catalog registration and execution-free schema analysis.

#![allow(clippy::result_large_err)]

use std::{collections::BTreeMap, ops::ControlFlow, sync::Arc};

use avenger_lang_core::{
    DataCapabilities, DeclarationId, Diagnostic, EnvironmentProvider, PhysicalType,
    ResolvedCatalogTable, ResolvedDeclaration, ResolvedProject, ResolvedValue, SourceLabel,
    SourceOrigin, project::normalize_path,
};
use datafusion::{
    catalog::{CatalogProvider, MemoryCatalogProvider, MemorySchemaProvider},
    common::TableReference,
    dataframe::DataFrame,
    datasource::{TableProvider, memory::MemTable},
    execution::options::ArrowReadOptions,
    prelude::{CsvReadOptions, JsonReadOptions, ParquetReadOptions, SessionContext},
};
use sqlparser::{
    ast::{
        Expr as SqlExpr, FunctionArg, FunctionArgExpr, FunctionArgOperator, Ident, ObjectName,
        Statement, TableFactor, VisitMut, VisitorMut,
    },
    dialect::GenericDialect,
    parser::Parser,
};

use crate::{
    AnalyzedColumn, AnalyzedDataset, CatalogFactoryRegistry, CompileEnvironment, DatasetLineage,
    DatasetLineageIndex, DatasetProvenance, DatasetSchemaIndex, DatasetStageId, DatasetStageKind,
    ProjectDatasetId, TableFactoryRegistry, lowering::physical_data_type,
};

pub(crate) struct CatalogAnalysis {
    pub datasets: DatasetSchemaIndex,
    pub lineage: DatasetLineageIndex,
    pub dependency_fingerprint: String,
}

pub(crate) struct CatalogOptions<'a> {
    pub project_root: &'a std::path::Path,
    pub capabilities: &'a DataCapabilities,
    pub environment_provider: &'a dyn EnvironmentProvider,
    pub catalog_factories: &'a CatalogFactoryRegistry,
    pub table_factories: &'a TableFactoryRegistry,
}

pub(crate) async fn register_and_analyze_catalog(
    project: &ResolvedProject,
    environment: &CompileEnvironment,
    options: CatalogOptions<'_>,
) -> Result<CatalogAnalysis, Diagnostic> {
    let context = environment.session_context();
    let mut provider_fingerprints =
        register_external_catalogs(project, environment, &options).await?;

    let declarations = declaration_index(project);
    let table_by_id = project
        .catalog_tables
        .values()
        .map(|table| (table.id.clone(), table))
        .collect::<BTreeMap<_, _>>();
    let mut providers = BTreeMap::<DeclarationId, Arc<dyn TableProvider>>::new();
    let mut stages = BTreeMap::<DeclarationId, DatasetStageId>::new();
    let mut datasets = DatasetSchemaIndex::default();
    let mut lineage = DatasetLineageIndex::default();

    analyze_external_catalogs(project, context, &mut datasets, &mut lineage).await?;

    for id in &project.table_order {
        let Some(table) = table_by_id.get(id).copied() else {
            continue;
        };
        let declaration = declarations.get(id).copied().ok_or_else(|| {
            catalog_diagnostic(table, "AVENGER-DATA-001", "catalog declaration is missing")
        })?;
        let (mut provider, logical_plan_fingerprint) =
            create_table_provider(project, declaration, table, environment, &options).await?;
        if matches!(
            declaration.properties.get("materialize"),
            Some(ResolvedValue::Atom(mode)) if mode == "session"
        ) {
            provider = Arc::new(SessionMaterializedTable::new(provider));
        }
        if !matches!(
            table.kind.as_str(),
            "inline" | "sql" | "csv" | "json" | "parquet" | "arrow" | "ipc"
        ) && let Some(factory) = options.table_factories.get(&table.kind)
        {
            let provider_options = declaration_options(
                &declaration.properties,
                options.capabilities,
                options.environment_provider,
                declaration,
            )?;
            if let Some(fingerprint) = factory
                .dependency_fingerprint(&provider_options, environment)
                .await
                .map_err(|error| {
                    declaration_diagnostic(declaration, "AVENGER-DATA-047", error.to_string())
                })?
            {
                provider_fingerprints.push(format!("table:{}:{fingerprint}", table.path.join(".")));
            }
        }
        register_table(context, &table.path, Arc::clone(&provider))
            .map_err(|message| catalog_diagnostic(table, "AVENGER-DATA-002", message))?;
        providers.insert(id.clone(), provider);

        let reference = table_reference(&table.path)
            .map_err(|message| catalog_diagnostic(table, "AVENGER-DATA-003", message))?;
        let dataframe = context
            .table(reference)
            .await
            .map_err(|error| catalog_diagnostic(table, "AVENGER-DATA-004", error.to_string()))?;
        let dataset_id = ProjectDatasetId::new(format!("catalog:{}", table.id.as_str()));
        let stage = DatasetStageId::new(dataset_id.clone(), 0);
        let columns = dataframe
            .schema()
            .iter()
            .map(|(qualifier, field)| AnalyzedColumn {
                name: field.name().clone(),
                qualifier: qualifier.map(ToString::to_string),
                data_type: field.data_type().clone(),
                nullable: field.is_nullable(),
            })
            .collect();
        let schema = Arc::new(dataframe.schema().as_arrow().clone());
        datasets
            .insert(AnalyzedDataset {
                id: dataset_id,
                stage: stage.clone(),
                provenance: DatasetProvenance {
                    declaration_span: table.span,
                    stage_span: table.span,
                    stage_kind: if table.kind == "sql" {
                        DatasetStageKind::SqlView
                    } else {
                        DatasetStageKind::CatalogTable
                    },
                },
                qualified_name: Some(table.path.join(".")),
                columns,
                schema,
                logical_plan_fingerprint,
            })
            .map_err(|error| catalog_diagnostic(table, "AVENGER-DATA-005", error.to_string()))?;
        let upstream_stages = table
            .dependencies
            .iter()
            .filter_map(|dependency| stages.get(dependency).cloned())
            .collect();
        lineage
            .insert(
                stage.clone(),
                DatasetLineage {
                    upstream_stages,
                    columns: Vec::new(),
                },
            )
            .map_err(|error| catalog_diagnostic(table, "AVENGER-DATA-005", error.to_string()))?;
        stages.insert(id.clone(), stage);
    }

    // Imported packs keep their declaration-local registrations for planning
    // internal chains, then gain chart-facing aliases by replacing the pack's
    // single schema/catalog root with the import binding.
    for (alias, table) in imported_table_aliases(project) {
        if project.catalog_tables.contains_key(&alias) {
            continue;
        }
        let Some(provider) = providers.get(&table.id).cloned() else {
            continue;
        };
        let path = alias.split('.').map(str::to_owned).collect::<Vec<_>>();
        register_table(context, &path, provider)
            .map_err(|message| catalog_diagnostic(table, "AVENGER-DATA-006", message))?;
        let reference = table_reference(&path)
            .map_err(|message| catalog_diagnostic(table, "AVENGER-DATA-003", message))?;
        let dataframe = context
            .table(reference)
            .await
            .map_err(|error| catalog_diagnostic(table, "AVENGER-DATA-004", error.to_string()))?;
        let dataset_id =
            ProjectDatasetId::new(format!("catalog:{}:alias:{}", table.id.as_str(), alias));
        let stage = DatasetStageId::new(dataset_id.clone(), 0);
        datasets
            .insert(AnalyzedDataset {
                id: dataset_id,
                stage: stage.clone(),
                provenance: DatasetProvenance {
                    declaration_span: table.span,
                    stage_span: table.span,
                    stage_kind: if table.kind == "sql" {
                        DatasetStageKind::SqlView
                    } else {
                        DatasetStageKind::CatalogTable
                    },
                },
                qualified_name: Some(alias),
                columns: dataframe
                    .schema()
                    .iter()
                    .map(|(qualifier, field)| AnalyzedColumn {
                        name: field.name().clone(),
                        qualifier: qualifier.map(ToString::to_string),
                        data_type: field.data_type().clone(),
                        nullable: field.is_nullable(),
                    })
                    .collect(),
                schema: Arc::new(dataframe.schema().as_arrow().clone()),
                logical_plan_fingerprint: None,
            })
            .map_err(|error| catalog_diagnostic(table, "AVENGER-DATA-005", error.to_string()))?;
        lineage
            .insert(stage, DatasetLineage::default())
            .map_err(|error| catalog_diagnostic(table, "AVENGER-DATA-005", error.to_string()))?;
    }

    provider_fingerprints.sort();
    Ok(CatalogAnalysis {
        datasets,
        lineage,
        dependency_fingerprint: catalog_dependency_fingerprint(
            &project.source_fingerprint,
            &provider_fingerprints,
        ),
    })
}

async fn register_external_catalogs(
    project: &ResolvedProject,
    environment: &CompileEnvironment,
    options: &CatalogOptions<'_>,
) -> Result<Vec<String>, Diagnostic> {
    let mut fingerprints = Vec::new();
    for declaration in project
        .files
        .values()
        .flat_map(|file| file.roots.iter())
        .filter(|declaration| declaration.keyword == "catalog")
    {
        let Some(name) = declaration.name.as_deref() else {
            continue;
        };
        let kind = declaration.kind.as_deref().unwrap_or("schemas");
        if matches!(kind, "schemas" | "memory") {
            ensure_catalog(environment.session_context(), name);
            continue;
        }
        let factory = options.catalog_factories.get(kind).ok_or_else(|| {
            declaration_diagnostic(
                declaration,
                "AVENGER-DATA-020",
                format!(
                    "catalog provider `{kind}` is unavailable; register a `{kind}` catalog factory"
                ),
            )
        })?;
        let mut provider_options = declaration_options(
            &declaration.properties,
            options.capabilities,
            options.environment_provider,
            declaration,
        )?;
        if let serde_json::Value::Object(object) = &mut provider_options {
            let mut projections = serde_json::Map::new();
            for schema in declaration
                .children
                .iter()
                .filter(|child| child.keyword == "schema")
            {
                let Some(alias) = schema.name.as_ref() else {
                    continue;
                };
                projections.insert(
                    alias.clone(),
                    declaration_options(
                        &schema.properties,
                        options.capabilities,
                        options.environment_provider,
                        schema,
                    )?,
                );
            }
            object.insert("schemas".to_owned(), serde_json::Value::Object(projections));
        }
        let provider = factory
            .create(&provider_options, environment)
            .await
            .map_err(|error| {
                declaration_diagnostic(declaration, "AVENGER-DATA-021", error.to_string())
            })?;
        if let Some(fingerprint) = factory
            .dependency_fingerprint(&provider_options, environment)
            .await
            .map_err(|error| {
                declaration_diagnostic(declaration, "AVENGER-DATA-027", error.to_string())
            })?
        {
            fingerprints.push(format!("catalog:{name}:{fingerprint}"));
        }
        if environment
            .session_context()
            .register_catalog(name, provider)
            .is_some()
        {
            return Err(declaration_diagnostic(
                declaration,
                "AVENGER-DATA-022",
                format!("catalog `{name}` is already registered in this generation"),
            ));
        }
    }
    Ok(fingerprints)
}

fn catalog_dependency_fingerprint(source: &str, providers: &[String]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"avenger-catalog-analysis-v1\0");
    hasher.update(source.as_bytes());
    for provider in providers {
        hasher.update(b"\0provider\0");
        hasher.update(provider.as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

async fn analyze_external_catalogs(
    project: &ResolvedProject,
    context: &SessionContext,
    datasets: &mut DatasetSchemaIndex,
    lineage: &mut DatasetLineageIndex,
) -> Result<(), Diagnostic> {
    for declaration in project
        .files
        .values()
        .flat_map(|file| file.roots.iter())
        .filter(|declaration| declaration.keyword == "catalog")
    {
        let kind = declaration.kind.as_deref().unwrap_or("schemas");
        if matches!(kind, "schemas" | "memory") {
            continue;
        }
        let Some(catalog_name) = declaration.name.as_deref() else {
            continue;
        };
        let catalog = context.catalog(catalog_name).ok_or_else(|| {
            declaration_diagnostic(
                declaration,
                "AVENGER-DATA-023",
                format!("catalog factory did not register `{catalog_name}`"),
            )
        })?;
        for projection in declaration
            .children
            .iter()
            .filter(|child| child.keyword == "schema")
        {
            let Some(schema_name) = projection.name.as_deref() else {
                continue;
            };
            let schema = catalog.schema(schema_name).ok_or_else(|| {
                declaration_diagnostic(
                    projection,
                    "AVENGER-DATA-024",
                    format!(
                        "catalog provider did not expose projected schema `{catalog_name}.{schema_name}`"
                    ),
                )
            })?;
            let mut table_names = schema.table_names();
            table_names.sort();
            for table_name in table_names {
                let provider = schema
                    .table(&table_name)
                    .await
                    .map_err(|error| {
                        declaration_diagnostic(projection, "AVENGER-DATA-025", error.to_string())
                    })?
                    .ok_or_else(|| {
                        declaration_diagnostic(
                            projection,
                            "AVENGER-DATA-025",
                            format!("provider listed missing table `{table_name}`"),
                        )
                    })?;
                let qualified = format!("{catalog_name}.{schema_name}.{table_name}");
                let dataset_id = ProjectDatasetId::new(format!("provider:{qualified}"));
                let stage = DatasetStageId::new(dataset_id.clone(), 0);
                datasets
                    .insert(AnalyzedDataset {
                        id: dataset_id,
                        stage: stage.clone(),
                        provenance: DatasetProvenance {
                            declaration_span: declaration.span,
                            stage_span: projection.span,
                            stage_kind: DatasetStageKind::CatalogTable,
                        },
                        qualified_name: Some(qualified.clone()),
                        columns: provider
                            .schema()
                            .fields()
                            .iter()
                            .map(|field| AnalyzedColumn {
                                name: field.name().clone(),
                                qualifier: Some(qualified.clone()),
                                data_type: field.data_type().clone(),
                                nullable: field.is_nullable(),
                            })
                            .collect(),
                        schema: provider.schema(),
                        logical_plan_fingerprint: None,
                    })
                    .map_err(|error| {
                        declaration_diagnostic(projection, "AVENGER-DATA-026", error.to_string())
                    })?;
                lineage
                    .insert(stage, DatasetLineage::default())
                    .map_err(|error| {
                        declaration_diagnostic(projection, "AVENGER-DATA-026", error.to_string())
                    })?;
            }
        }
    }
    Ok(())
}

async fn create_table_provider(
    project: &ResolvedProject,
    declaration: &ResolvedDeclaration,
    table: &ResolvedCatalogTable,
    environment: &CompileEnvironment,
    options: &CatalogOptions<'_>,
) -> Result<(Arc<dyn TableProvider>, Option<String>), Diagnostic> {
    let context = environment.session_context();
    match table.kind.as_str() {
        "inline" => inline_table(context, declaration).await,
        "sql" => {
            let query = declaration
                .properties
                .get("sql")
                .and_then(|value| match value {
                    ResolvedValue::Query(query) => Some(query),
                    _ => None,
                })
                .ok_or_else(|| {
                    declaration_diagnostic(
                        declaration,
                        "AVENGER-DATA-030",
                        "`table sql` requires a query-valued `sql:` property",
                    )
                })?;
            let sql =
                expand_catalog_sql(project, &query.sql, Some(table), true).map_err(|message| {
                    declaration_diagnostic(declaration, "AVENGER-DATA-050", message)
                })?;
            context
                .sql(&sql)
                .await
                .map(|dataframe| {
                    let fingerprint = logical_plan_fingerprint(&dataframe);
                    (dataframe.into_view(), Some(fingerprint))
                })
                .map_err(|error| {
                    declaration_diagnostic(declaration, "AVENGER-DATA-031", error.to_string())
                })
        }
        "csv" | "json" | "parquet" => {
            let path = table_path(project, declaration, options)?;
            let dataframe = match table.kind.as_str() {
                "csv" => {
                    let mut read = CsvReadOptions::new();
                    if let Some(file_options) = file_options(declaration, "csv")? {
                        for (name, value) in file_options {
                            match name.as_str() {
                                "has_header" => {
                                    let ResolvedValue::Boolean(value) = value else {
                                        return Err(file_option_type(declaration, name, "boolean"));
                                    };
                                    read = read.has_header(*value);
                                }
                                "delimiter" => {
                                    let ResolvedValue::String(value) = value else {
                                        return Err(file_option_type(
                                            declaration,
                                            name,
                                            "one ASCII character",
                                        ));
                                    };
                                    let bytes = value.as_bytes();
                                    if bytes.len() != 1 || !bytes[0].is_ascii() {
                                        return Err(file_option_type(
                                            declaration,
                                            name,
                                            "one ASCII character",
                                        ));
                                    }
                                    read = read.delimiter(bytes[0]);
                                }
                                "schema_infer_max_records" => {
                                    read = read.schema_infer_max_records(option_usize(
                                        declaration,
                                        name,
                                        value,
                                    )?);
                                }
                                "file_extension" => {
                                    let ResolvedValue::String(value) = value else {
                                        return Err(file_option_type(declaration, name, "string"));
                                    };
                                    read = read.file_extension(value);
                                }
                                _ => return Err(unknown_file_option(declaration, "csv", name)),
                            }
                        }
                    }
                    context.read_csv(path, read).await
                }
                "json" => {
                    let mut read = JsonReadOptions::default();
                    if let Some(file_options) = file_options(declaration, "json")? {
                        for (name, value) in file_options {
                            match name.as_str() {
                                "schema_infer_max_records" => {
                                    read = read.schema_infer_max_records(option_usize(
                                        declaration,
                                        name,
                                        value,
                                    )?);
                                }
                                "file_extension" => {
                                    let ResolvedValue::String(value) = value else {
                                        return Err(file_option_type(declaration, name, "string"));
                                    };
                                    read = read.file_extension(value);
                                }
                                _ => return Err(unknown_file_option(declaration, "json", name)),
                            }
                        }
                    }
                    context.read_json(path, read).await
                }
                "parquet" => {
                    let mut read = ParquetReadOptions::default();
                    if let Some(file_options) = file_options(declaration, "parquet")? {
                        for (name, value) in file_options {
                            match name.as_str() {
                                "file_extension" => {
                                    let ResolvedValue::String(value) = value else {
                                        return Err(file_option_type(declaration, name, "string"));
                                    };
                                    read = read.file_extension(value);
                                }
                                _ => return Err(unknown_file_option(declaration, "parquet", name)),
                            }
                        }
                    }
                    context.read_parquet(path, read).await
                }
                _ => unreachable!(),
            }
            .map_err(|error| {
                declaration_diagnostic(declaration, "AVENGER-DATA-032", error.to_string())
            })?;
            let fingerprint = logical_plan_fingerprint(&dataframe);
            Ok((dataframe.into_view(), Some(fingerprint)))
        }
        "arrow" | "ipc" => {
            let path = table_path(project, declaration, options)?;
            let mut read = ArrowReadOptions::default();
            if let Some(file_options) = file_options(declaration, "arrow")? {
                for (name, value) in file_options {
                    match name.as_str() {
                        "file_extension" => {
                            let ResolvedValue::String(value) = value else {
                                return Err(file_option_type(declaration, name, "string"));
                            };
                            read.file_extension = value;
                        }
                        _ => return Err(unknown_file_option(declaration, "arrow", name)),
                    }
                }
            }
            let dataframe = context.read_arrow(path, read).await.map_err(|error| {
                declaration_diagnostic(declaration, "AVENGER-DATA-039", error.to_string())
            })?;
            let fingerprint = logical_plan_fingerprint(&dataframe);
            Ok((dataframe.into_view(), Some(fingerprint)))
        }
        kind => {
            let factory = options.table_factories.get(kind).ok_or_else(|| {
                declaration_diagnostic(
                    declaration,
                    "AVENGER-DATA-033",
                    format!(
                        "table provider `{kind}` is unavailable; register a `{kind}` table factory"
                    ),
                )
            })?;
            let provider_options = declaration_options(
                &declaration.properties,
                options.capabilities,
                options.environment_provider,
                declaration,
            )?;
            factory
                .create(&provider_options, environment)
                .await
                .map(|provider| (provider, None))
                .map_err(|error| {
                    declaration_diagnostic(declaration, "AVENGER-DATA-034", error.to_string())
                })
        }
    }
}

fn file_options<'a>(
    declaration: &'a ResolvedDeclaration,
    kind: &str,
) -> Result<Option<&'a BTreeMap<String, ResolvedValue>>, Diagnostic> {
    match declaration.properties.get("options") {
        None => Ok(None),
        Some(ResolvedValue::Object { properties, .. }) => Ok(Some(properties)),
        Some(_) => Err(declaration_diagnostic(
            declaration,
            "AVENGER-DATA-044",
            format!("`table {kind}` options must be an object"),
        )),
    }
}

fn option_usize(
    declaration: &ResolvedDeclaration,
    name: &str,
    value: &ResolvedValue,
) -> Result<usize, Diagnostic> {
    let ResolvedValue::Number(value) = value else {
        return Err(file_option_type(
            declaration,
            name,
            "a non-negative integer",
        ));
    };
    value
        .parse::<usize>()
        .map_err(|_| file_option_type(declaration, name, "a non-negative integer"))
}

fn file_option_type(declaration: &ResolvedDeclaration, name: &str, expected: &str) -> Diagnostic {
    declaration_diagnostic(
        declaration,
        "AVENGER-DATA-045",
        format!("file option `{name}` requires {expected}"),
    )
}

fn unknown_file_option(declaration: &ResolvedDeclaration, kind: &str, name: &str) -> Diagnostic {
    declaration_diagnostic(
        declaration,
        "AVENGER-DATA-046",
        format!("`table {kind}` does not support file option `{name}`"),
    )
}

async fn inline_table(
    context: &SessionContext,
    declaration: &ResolvedDeclaration,
) -> Result<(Arc<dyn TableProvider>, Option<String>), Diagnostic> {
    let rows = declaration.properties.get("values").ok_or_else(|| {
        declaration_diagnostic(
            declaration,
            "AVENGER-DATA-035",
            "`table inline` requires `values:`",
        )
    })?;
    let sql = inline_values_sql(rows)
        .map_err(|message| declaration_diagnostic(declaration, "AVENGER-DATA-036", message))?;
    context
        .sql(&sql)
        .await
        .map(|dataframe| {
            let fingerprint = logical_plan_fingerprint(&dataframe);
            (dataframe.into_view(), Some(fingerprint))
        })
        .map_err(|error| declaration_diagnostic(declaration, "AVENGER-DATA-037", error.to_string()))
}

fn logical_plan_fingerprint(dataframe: &DataFrame) -> String {
    use sha2::{Digest, Sha256};
    let plan = dataframe.logical_plan().display_indent_schema().to_string();
    format!("sha256:{:x}", Sha256::digest(plan.as_bytes()))
}

/// A generation-local, lazy materialization. Logical planning consults only
/// `schema`; the first physical scan loads the source, and every later scan in
/// the same compilation session reuses the immutable Arrow batches.
struct SessionMaterializedTable {
    source: Arc<dyn TableProvider>,
    materialized: tokio::sync::OnceCell<Arc<MemTable>>,
}

impl SessionMaterializedTable {
    fn new(source: Arc<dyn TableProvider>) -> Self {
        Self {
            source,
            materialized: tokio::sync::OnceCell::new(),
        }
    }
}

impl std::fmt::Debug for SessionMaterializedTable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SessionMaterializedTable")
            .field("schema", &self.source.schema())
            .field("is_materialized", &self.materialized.initialized())
            .finish()
    }
}

#[async_trait::async_trait]
impl TableProvider for SessionMaterializedTable {
    fn schema(&self) -> arrow::datatypes::SchemaRef {
        self.source.schema()
    }

    fn table_type(&self) -> datafusion::datasource::TableType {
        self.source.table_type()
    }

    async fn scan(
        &self,
        state: &dyn datafusion::catalog::Session,
        projection: Option<&Vec<usize>>,
        filters: &[datafusion::logical_expr::Expr],
        limit: Option<usize>,
    ) -> datafusion::common::Result<Arc<dyn datafusion::physical_plan::ExecutionPlan>> {
        let materialized = self
            .materialized
            .get_or_try_init(|| async {
                MemTable::load(Arc::clone(&self.source), Some(1), state)
                    .await
                    .map(Arc::new)
            })
            .await?;
        materialized.scan(state, projection, filters, limit).await
    }
}

fn table_path(
    project: &ResolvedProject,
    declaration: &ResolvedDeclaration,
    options: &CatalogOptions<'_>,
) -> Result<String, Diagnostic> {
    let ResolvedValue::String(authored) = declaration.properties.get("path").ok_or_else(|| {
        declaration_diagnostic(
            declaration,
            "AVENGER-DATA-040",
            "file table requires `path:`",
        )
    })?
    else {
        return Err(declaration_diagnostic(
            declaration,
            "AVENGER-DATA-040",
            "file table `path:` must be a string",
        ));
    };
    if let Some((scheme, _)) = authored.split_once("://") {
        let allowed = match scheme {
            "http" | "https" => options.capabilities.allow_http,
            "file" => options.capabilities.allow_filesystem,
            other => options.capabilities.allows_object_store_scheme(other),
        };
        return allowed.then(|| authored.clone()).ok_or_else(|| {
            declaration_diagnostic(
                declaration,
                "AVENGER-DATA-041",
                format!("data capability denies `{scheme}` object-store access"),
            )
        });
    }
    if !options.capabilities.allow_filesystem {
        return Err(declaration_diagnostic(
            declaration,
            "AVENGER-DATA-041",
            "data capability denies filesystem access",
        ));
    }
    let source = project.sources.get(declaration.source).ok_or_else(|| {
        declaration_diagnostic(
            declaration,
            "AVENGER-DATA-042",
            "source file is unavailable",
        )
    })?;
    let base = match &source.origin {
        SourceOrigin::File(path) => path.parent().unwrap_or(options.project_root),
        _ => options.project_root,
    };
    let path = normalize_path(&base.join(authored));
    let root = normalize_path(options.project_root);
    if !path.starts_with(&root) {
        return Err(declaration_diagnostic(
            declaration,
            "AVENGER-DATA-043",
            format!(
                "data path `{}` escapes project root `{}`",
                path.display(),
                root.display()
            ),
        ));
    }
    Ok(path.to_string_lossy().into_owned())
}

fn register_table(
    context: &SessionContext,
    path: &[String],
    provider: Arc<dyn TableProvider>,
) -> Result<(), String> {
    let reference = table_reference(path)?;
    match path {
        [_] => {}
        [schema, _] => ensure_schema(context, None, schema)?,
        [catalog, schema, _] => ensure_schema(context, Some(catalog), schema)?,
        _ => unreachable!(),
    }
    context
        .register_table(reference, provider)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn table_reference(path: &[String]) -> Result<TableReference, String> {
    match path {
        [table] => Ok(TableReference::bare(table.clone())),
        [schema, table] => Ok(TableReference::partial(schema.clone(), table.clone())),
        [catalog, schema, table] => Ok(TableReference::full(
            catalog.clone(),
            schema.clone(),
            table.clone(),
        )),
        _ => Err(format!(
            "catalog table path `{}` must have one, two, or three components",
            path.join(".")
        )),
    }
}

fn ensure_catalog(context: &SessionContext, name: &str) -> Arc<dyn CatalogProvider> {
    if let Some(catalog) = context.catalog(name) {
        return catalog;
    }
    let catalog: Arc<dyn CatalogProvider> = Arc::new(MemoryCatalogProvider::new());
    context.register_catalog(name, Arc::clone(&catalog));
    catalog
}

fn ensure_schema(
    context: &SessionContext,
    catalog: Option<&str>,
    schema: &str,
) -> Result<(), String> {
    let catalog_name = catalog.map(str::to_owned).unwrap_or_else(|| {
        context
            .state()
            .config_options()
            .catalog
            .default_catalog
            .clone()
    });
    let catalog = ensure_catalog(context, &catalog_name);
    if catalog.schema(schema).is_none() {
        catalog
            .register_schema(schema, Arc::new(MemorySchemaProvider::new()))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Expand Avenger parameterized table invocations into ordinary derived SQL
/// tables before handing the canonical query to DataFusion's parser.
pub(crate) fn expand_chart_sql(project: &ResolvedProject, sql: &str) -> Result<String, String> {
    expand_catalog_sql(project, sql, None, false)
}

fn expand_catalog_sql(
    project: &ResolvedProject,
    sql: &str,
    caller: Option<&ResolvedCatalogTable>,
    bind_caller_defaults: bool,
) -> Result<String, String> {
    let mut statement = parse_query_statement(sql)?;
    if bind_caller_defaults && let Some(caller) = caller {
        let bindings = default_param_expressions(project, caller)?;
        substitute_query_params(&mut statement, &bindings)?;
    }
    let prefix = caller
        .map(|table| &table.path[..table.path.len().saturating_sub(1)])
        .unwrap_or(&[]);
    let mut expander = TableFunctionExpander {
        project,
        prefix,
        error: None,
    };
    if let ControlFlow::Break(()) = statement.visit(&mut expander) {
        return Err(expander
            .error
            .unwrap_or_else(|| "table-function expansion stopped".to_owned()));
    }
    Ok(statement.to_string())
}

fn parse_query_statement(sql: &str) -> Result<Statement, String> {
    let statements = Parser::parse_sql(&GenericDialect, sql).map_err(|error| error.to_string())?;
    let [statement] = statements.as_slice() else {
        return Err("SQL source must contain exactly one query".to_owned());
    };
    if !matches!(statement, Statement::Query(_)) {
        return Err("SQL source must be a query".to_owned());
    }
    Ok(statement.clone())
}

struct TableFunctionExpander<'a> {
    project: &'a ResolvedProject,
    prefix: &'a [String],
    error: Option<String>,
}

impl VisitorMut for TableFunctionExpander<'_> {
    type Break = ();

    fn pre_visit_table_factor(
        &mut self,
        table_factor: &mut TableFactor,
    ) -> ControlFlow<Self::Break> {
        let TableFactor::Table {
            name, alias, args, ..
        } = table_factor
        else {
            return ControlFlow::Continue(());
        };
        let authored_name = name.to_string();
        let Some(table) = resolve_table_name(self.project, self.prefix, &authored_name) else {
            // DataFusion may own ordinary table functions; only Avenger table
            // declarations are rewritten here.
            return ControlFlow::Continue(());
        };
        let Some(arguments) = args else {
            if !authored_name.contains('.') && table.path.len() > 1 {
                *name = ObjectName::from(table.path.iter().map(Ident::new).collect::<Vec<_>>());
            }
            return ControlFlow::Continue(());
        };
        let alias = alias.clone();
        let arguments = arguments.clone();
        match instantiate_table_query(self.project, table, &arguments.args) {
            Ok(subquery) => {
                *table_factor = TableFactor::Derived {
                    lateral: false,
                    subquery: Box::new(subquery),
                    alias,
                    sample: None,
                };
                ControlFlow::Continue(())
            }
            Err(error) => {
                self.error = Some(error);
                ControlFlow::Break(())
            }
        }
    }
}

fn instantiate_table_query(
    project: &ResolvedProject,
    table: &ResolvedCatalogTable,
    arguments: &[FunctionArg],
) -> Result<sqlparser::ast::Query, String> {
    if table.kind != "sql" {
        return Err(format!(
            "table `{}` is not parameterizable because it is `{}` rather than `sql`",
            table.path.join("."),
            table.kind
        ));
    }
    let declarations = declaration_index(project);
    let declaration = declarations
        .get(&table.id)
        .copied()
        .ok_or_else(|| format!("table `{}` declaration is missing", table.path.join(".")))?;
    let query = match declaration.properties.get("sql") {
        Some(ResolvedValue::Query(query)) => query,
        _ => return Err(format!("table `{}` has no SQL query", table.path.join("."))),
    };
    let mut bindings = default_param_expressions(project, table)?;
    let mut supplied = std::collections::BTreeSet::new();
    for argument in arguments {
        let FunctionArg::Named {
            name,
            arg: FunctionArgExpr::Expr(expression),
            operator: FunctionArgOperator::RightArrow,
        } = argument
        else {
            return Err(format!(
                "table `{}` arguments must use named `name => value` syntax",
                table.path.join(".")
            ));
        };
        let name = name.value.clone();
        if !supplied.insert(name.clone()) {
            return Err(format!(
                "table argument `{name}` is supplied more than once"
            ));
        }
        let param = table
            .params
            .iter()
            .filter_map(|id| project.params.get(id))
            .find(|param| param.source_name == name)
            .ok_or_else(|| {
                format!(
                    "table `{}` has no parameter named `{name}`",
                    table.path.join(".")
                )
            })?;
        if !scalar_argument(expression) {
            return Err(format!(
                "table argument `{name}` must be a scalar literal or visible `$param`"
            ));
        }
        bindings.insert(
            name,
            typed_expression(expression.clone(), &param.data_type)?,
        );
    }

    let mut statement = parse_query_statement(&query.sql)?;
    substitute_query_params(&mut statement, &bindings)?;
    let prefix = &table.path[..table.path.len().saturating_sub(1)];
    let mut expander = TableFunctionExpander {
        project,
        prefix,
        error: None,
    };
    if let ControlFlow::Break(()) = statement.visit(&mut expander) {
        return Err(expander
            .error
            .unwrap_or_else(|| "nested table-function expansion stopped".to_owned()));
    }
    match statement {
        Statement::Query(query) => Ok(*query),
        _ => unreachable!(),
    }
}

fn resolve_table_name<'a>(
    project: &'a ResolvedProject,
    prefix: &[String],
    name: &str,
) -> Option<&'a ResolvedCatalogTable> {
    let normalized = name
        .split('.')
        .map(|component| component.trim_matches('"'))
        .collect::<Vec<_>>()
        .join(".");
    if !normalized.contains('.') && !prefix.is_empty() {
        let qualified = format!("{}.{}", prefix.join("."), normalized);
        if let Some(table) = project.catalog_tables.get(&qualified) {
            return Some(table);
        }
    }
    project
        .catalog_tables
        .get(&normalized)
        .or_else(|| imported_table_aliases(project).get(&normalized).copied())
}

fn imported_table_aliases(project: &ResolvedProject) -> BTreeMap<String, &ResolvedCatalogTable> {
    let mut result = BTreeMap::new();
    for importer in project.files.values() {
        for (binding, imported_file) in &importer.imports {
            let Some(file) = project.files.get(imported_file) else {
                continue;
            };
            if !matches!(file.kind, avenger_lang_core::ProjectFileKind::Data) {
                continue;
            }
            let root_name = file
                .roots
                .iter()
                .find(|root| matches!(root.keyword.as_str(), "schema" | "catalog"))
                .and_then(|root| root.name.as_deref());
            let Some(root_name) = root_name else {
                continue;
            };
            for table in project
                .catalog_tables
                .values()
                .filter(|table| &table.file == imported_file)
            {
                if table.path.first().is_some_and(|root| root == root_name) {
                    let mut path = table.path.clone();
                    path[0] = binding.clone();
                    result.insert(path.join("."), table);
                }
            }
        }
    }
    result
}

fn default_param_expressions(
    project: &ResolvedProject,
    table: &ResolvedCatalogTable,
) -> Result<BTreeMap<String, SqlExpr>, String> {
    table
        .params
        .iter()
        .map(|id| {
            let param = &project.params[id];
            let literal = scalar_literal_sql(&param.default)?;
            let expression = parse_sql_expression(&literal)?;
            Ok((
                param.source_name.clone(),
                typed_expression(expression, &param.data_type)?,
            ))
        })
        .collect()
}

fn typed_expression(expression: SqlExpr, data_type: &PhysicalType) -> Result<SqlExpr, String> {
    let arrow_type = physical_data_type(data_type).to_string();
    parse_sql_expression(&format!(
        "arrow_cast(({expression}), '{}')",
        arrow_type.replace('\'', "''")
    ))
}

fn parse_sql_expression(sql: &str) -> Result<SqlExpr, String> {
    Parser::new(&GenericDialect)
        .try_with_sql(sql)
        .map_err(|error| error.to_string())?
        .parse_expr()
        .map_err(|error| error.to_string())
}

fn scalar_argument(expression: &SqlExpr) -> bool {
    matches!(
        expression,
        SqlExpr::Value(_)
            | SqlExpr::Array(_)
            | SqlExpr::Struct { .. }
            | SqlExpr::Dictionary(_)
            | SqlExpr::Tuple(_)
            | SqlExpr::UnaryOp { .. }
            | SqlExpr::Function(_)
            | SqlExpr::Cast { .. }
            | SqlExpr::Nested(_)
    )
}

fn substitute_query_params(
    statement: &mut Statement,
    bindings: &BTreeMap<String, SqlExpr>,
) -> Result<(), String> {
    struct Substituter<'a> {
        bindings: &'a BTreeMap<String, SqlExpr>,
    }
    impl VisitorMut for Substituter<'_> {
        type Break = ();

        fn pre_visit_expr(&mut self, expression: &mut SqlExpr) -> ControlFlow<Self::Break> {
            let SqlExpr::Value(value) = expression else {
                return ControlFlow::Continue(());
            };
            let sqlparser::ast::Value::Placeholder(placeholder) = &value.value else {
                return ControlFlow::Continue(());
            };
            if let Some(replacement) = placeholder
                .strip_prefix('$')
                .and_then(|name| self.bindings.get(name))
            {
                *expression = replacement.clone();
            }
            ControlFlow::Continue(())
        }
    }
    let mut substituter = Substituter { bindings };
    let _ = statement.visit(&mut substituter);
    Ok(())
}

fn inline_values_sql(value: &ResolvedValue) -> Result<String, String> {
    let ResolvedValue::Array(rows) = value else {
        return Err("inline values must be an array".to_owned());
    };
    let Some(ResolvedValue::Object {
        properties: first, ..
    }) = rows.first()
    else {
        return Err("inline values require at least one object row".to_owned());
    };
    let columns = first.keys().cloned().collect::<Vec<_>>();
    if columns.is_empty() {
        return Err("inline rows cannot be empty".to_owned());
    }
    let mut sql_rows = Vec::new();
    for row in rows {
        let ResolvedValue::Object { properties, .. } = row else {
            return Err("inline rows must be objects".to_owned());
        };
        if properties.keys().ne(columns.iter()) {
            return Err("every inline row must contain the same ordered fields".to_owned());
        }
        sql_rows.push(format!(
            "({})",
            columns
                .iter()
                .map(|column| scalar_literal_sql(&properties[column]))
                .collect::<Result<Vec<_>, _>>()?
                .join(", ")
        ));
    }
    let aliases = columns
        .iter()
        .map(|column| format!("\"{}\"", column.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(", ");
    Ok(format!(
        "SELECT * FROM (VALUES {}) AS __avenger_inline({aliases})",
        sql_rows.join(", ")
    ))
}

fn scalar_literal_sql(value: &ResolvedValue) -> Result<String, String> {
    match value {
        ResolvedValue::String(value) => Ok(format!("'{}'", value.replace('\'', "''"))),
        ResolvedValue::Number(value) => Ok(value.clone()),
        ResolvedValue::Boolean(value) => Ok(if *value { "TRUE" } else { "FALSE" }.to_owned()),
        ResolvedValue::Null => Ok("NULL".to_owned()),
        ResolvedValue::Array(values) => Ok(format!(
            "[{}]",
            values
                .iter()
                .map(scalar_literal_sql)
                .collect::<Result<Vec<_>, _>>()?
                .join(", ")
        )),
        ResolvedValue::Object { properties, .. } => {
            let mut arguments = Vec::with_capacity(properties.len() * 2);
            for (name, value) in properties {
                arguments.push(format!("'{}'", name.replace('\'', "''")));
                arguments.push(scalar_literal_sql(value)?);
            }
            Ok(format!("named_struct({})", arguments.join(", ")))
        }
        _ => Err("table parameter and inline values must be scalar SQL literals".to_owned()),
    }
}

fn declaration_options(
    properties: &BTreeMap<String, ResolvedValue>,
    capabilities: &DataCapabilities,
    environment: &dyn EnvironmentProvider,
    declaration: &ResolvedDeclaration,
) -> Result<serde_json::Value, Diagnostic> {
    let mut values = serde_json::Map::new();
    for (name, value) in properties {
        values.insert(
            name.clone(),
            resolved_json(value, capabilities, environment, declaration)?,
        );
    }
    Ok(serde_json::Value::Object(values))
}

fn resolved_json(
    value: &ResolvedValue,
    capabilities: &DataCapabilities,
    environment: &dyn EnvironmentProvider,
    declaration: &ResolvedDeclaration,
) -> Result<serde_json::Value, Diagnostic> {
    Ok(match value {
        ResolvedValue::String(value) | ResolvedValue::Atom(value) => value.clone().into(),
        ResolvedValue::Number(value) => {
            serde_json::from_str(value).unwrap_or_else(|_| value.clone().into())
        }
        ResolvedValue::Boolean(value) => (*value).into(),
        ResolvedValue::Null | ResolvedValue::None => serde_json::Value::Null,
        ResolvedValue::Environment(name) => {
            if !capabilities.allows_environment(name) {
                return Err(declaration_diagnostic(
                    declaration,
                    "AVENGER-DATA-060",
                    format!("data capability denies environment variable `{name}`"),
                ));
            }
            environment
                .get(name)
                .ok_or_else(|| {
                    declaration_diagnostic(
                        declaration,
                        "AVENGER-DATA-061",
                        format!("environment provider has no value for `{name}`"),
                    )
                })?
                .into()
        }
        ResolvedValue::Array(values) => serde_json::Value::Array(
            values
                .iter()
                .map(|value| resolved_json(value, capabilities, environment, declaration))
                .collect::<Result<Vec<_>, _>>()?,
        ),
        ResolvedValue::Object { properties, .. } => {
            let mut object = serde_json::Map::new();
            for (name, value) in properties {
                object.insert(
                    name.clone(),
                    resolved_json(value, capabilities, environment, declaration)?,
                );
            }
            serde_json::Value::Object(object)
        }
        _ => {
            return Err(declaration_diagnostic(
                declaration,
                "AVENGER-DATA-062",
                "provider options must be literal values, arrays, objects, or explicit `env` reads",
            ));
        }
    })
}

fn declaration_index(project: &ResolvedProject) -> BTreeMap<DeclarationId, &ResolvedDeclaration> {
    fn visit<'a>(
        declaration: &'a ResolvedDeclaration,
        result: &mut BTreeMap<DeclarationId, &'a ResolvedDeclaration>,
    ) {
        result.insert(declaration.id.clone(), declaration);
        for child in &declaration.children {
            visit(child, result);
        }
    }
    let mut result = BTreeMap::new();
    for root in project.files.values().flat_map(|file| &file.roots) {
        visit(root, &mut result);
    }
    result
}

fn catalog_diagnostic(
    table: &ResolvedCatalogTable,
    code: &'static str,
    message: impl Into<String>,
) -> Diagnostic {
    Diagnostic::error(
        code,
        "catalog data planning failed",
        SourceLabel::new(table.span, message),
    )
}

fn declaration_diagnostic(
    declaration: &ResolvedDeclaration,
    code: &'static str,
    message: impl Into<String>,
) -> Diagnostic {
    Diagnostic::error(
        code,
        "catalog provider configuration failed",
        SourceLabel::new(declaration.span, message),
    )
}
