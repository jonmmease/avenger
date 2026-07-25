//! DataFusion-backed catalog registration and execution-free schema analysis.

#![allow(clippy::result_large_err)]

use std::{
    collections::{BTreeMap, BTreeSet},
    ops::ControlFlow,
    sync::Arc,
};

use avenger_lang_core::{
    DataCapabilities, DeclarationId, Diagnostic, EnvironmentProvider, ModuleItemId, PhysicalType,
    ResolvedCatalogTable, ResolvedDeclaration, ResolvedModuleGraph, ResolvedQuery,
    ResolvedRelationId, ResolvedRelationTarget, ResolvedValue, SourceLabel, SourceOrigin,
    module_graph::normalize_path,
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
    DependencyFingerprint, ModuleDatasetId, TableFactoryRegistry, lowering::physical_data_type,
};

pub(crate) struct CatalogAnalysis {
    pub datasets: DatasetSchemaIndex,
    pub lineage: DatasetLineageIndex,
    pub dataset_fingerprints: BTreeMap<DatasetStageId, DependencyFingerprint>,
    pub table_fingerprints: BTreeMap<DeclarationId, DependencyFingerprint>,
    pub dependency_fingerprint: String,
}

pub(crate) struct CatalogOptions<'a> {
    pub project_root: &'a std::path::Path,
    pub capabilities: &'a DataCapabilities,
    pub environment_provider: &'a dyn EnvironmentProvider,
    pub catalog_factories: &'a CatalogFactoryRegistry,
    pub table_factories: &'a TableFactoryRegistry,
    /// When present, initialize only providers owned by these reachable
    /// top-level module items. `None` is the eager editor/checking mode.
    pub reachable_items: Option<&'a BTreeSet<ModuleItemId>>,
}

struct ExternalCatalogRegistration {
    provider: Arc<dyn CatalogProvider>,
    fingerprint: String,
}

pub(crate) async fn register_and_analyze_catalog(
    project: &ResolvedModuleGraph,
    environment: &CompileEnvironment,
    options: CatalogOptions<'_>,
) -> Result<CatalogAnalysis, Diagnostic> {
    let context = environment.session_context();
    let external_fingerprints = register_external_catalogs(project, environment, &options).await?;

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
    let mut dataset_fingerprints = BTreeMap::new();
    let mut table_fingerprints = BTreeMap::new();
    let mut relation_stages = BTreeMap::new();
    let mut relation_fingerprints = BTreeMap::new();

    analyze_external_catalogs(
        project,
        context,
        &external_fingerprints,
        &options,
        ExternalCatalogAnalysisOutput {
            datasets: &mut datasets,
            lineage: &mut lineage,
            dataset_fingerprints: &mut dataset_fingerprints,
            relation_stages: &mut relation_stages,
            relation_fingerprints: &mut relation_fingerprints,
        },
    )
    .await?;

    for id in &project.table_order {
        let Some(table) = table_by_id.get(id).copied() else {
            continue;
        };
        if !item_is_reachable(&table.relation.defining_item, &options) {
            continue;
        }
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
        let mut provider_snapshot = None;
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
                provider_snapshot = Some(hash_parts(
                    "avenger-table-provider-v1",
                    [provider_options.to_string().as_str(), fingerprint.as_str()],
                ));
            } else {
                provider_snapshot = Some(hash_parts(
                    "avenger-table-provider-v1",
                    [provider_options.to_string().as_str()],
                ));
            }
        }
        let internal_path = vec![internal_relation_name(&table.relation)];
        register_table(context, &internal_path, Arc::clone(&provider))
            .map_err(|message| catalog_diagnostic(table, "AVENGER-DATA-002", message))?;
        providers.insert(id.clone(), provider);

        let reference = table_reference(&internal_path)
            .map_err(|message| catalog_diagnostic(table, "AVENGER-DATA-003", message))?;
        let dataframe = context
            .table(reference)
            .await
            .map_err(|error| catalog_diagnostic(table, "AVENGER-DATA-004", error.to_string()))?;
        let dataset_id = ModuleDatasetId::new(format!("catalog:{}", table.id.as_str()));
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
        let table_fingerprint = table_dependency_fingerprint(
            project,
            declaration,
            table,
            TableFingerprintInputs {
                dependencies: &table_fingerprints,
                relation_dependencies: &relation_fingerprints,
                logical_plan: logical_plan_fingerprint.as_deref(),
                provider_snapshot: provider_snapshot.as_deref(),
            },
            &options,
        )?;
        datasets
            .insert(AnalyzedDataset {
                id: dataset_id.clone(),
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
                qualified_path: Some(table.path.clone()),
                columns,
                schema,
                logical_plan_fingerprint,
            })
            .map_err(|error| catalog_diagnostic(table, "AVENGER-DATA-005", error.to_string()))?;
        dataset_fingerprints.insert(stage.clone(), table_fingerprint.clone());
        let mut upstream_stages = table
            .dependencies
            .iter()
            .filter_map(|dependency| stages.get(dependency).cloned())
            .collect::<Vec<_>>();
        if let Some(ResolvedValue::Query(query)) = declaration.properties.get("sql") {
            upstream_stages.extend(query.relations.iter().filter_map(|reference| {
                let ResolvedRelationTarget::Relation(relation) = &reference.target else {
                    return None;
                };
                relation_stages.get(relation).cloned()
            }));
        }
        upstream_stages.sort();
        upstream_stages.dedup();
        lineage
            .insert(
                stage.clone(),
                DatasetLineage {
                    upstream_stages,
                    columns: Vec::new(),
                },
            )
            .map_err(|error| catalog_diagnostic(table, "AVENGER-DATA-005", error.to_string()))?;
        stages.insert(id.clone(), stage.clone());
        relation_stages.insert(table.relation.clone(), stage);
        relation_fingerprints.insert(table.relation.clone(), table_fingerprint.clone());
        table_fingerprints.insert(id.clone(), table_fingerprint);
    }

    let dependency_fingerprint =
        catalog_dependency_fingerprint(environment.dependency_fingerprint(), &dataset_fingerprints);
    Ok(CatalogAnalysis {
        datasets,
        lineage,
        dataset_fingerprints,
        table_fingerprints,
        dependency_fingerprint,
    })
}

async fn register_external_catalogs(
    project: &ResolvedModuleGraph,
    environment: &CompileEnvironment,
    options: &CatalogOptions<'_>,
) -> Result<BTreeMap<ModuleItemId, ExternalCatalogRegistration>, Diagnostic> {
    let mut registrations = BTreeMap::new();
    for (item, declaration) in project
        .source_modules
        .values()
        .flat_map(|file| {
            file.item_order
                .iter()
                .zip(&file.roots)
                .filter(|(item, _)| item_is_reachable(item, options))
        })
        .filter(|(_, declaration)| declaration.keyword == "catalog")
    {
        let Some(_) = declaration.name.as_deref() else {
            continue;
        };
        let kind = declaration.kind.as_deref().unwrap_or("schemas");
        if matches!(kind, "schemas" | "memory") {
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
        let provider_fingerprint = factory
            .dependency_fingerprint(&provider_options, environment)
            .await
            .map_err(|error| {
                declaration_diagnostic(declaration, "AVENGER-DATA-027", error.to_string())
            })?;
        registrations.insert(
            item.clone(),
            ExternalCatalogRegistration {
                provider,
                fingerprint: external_catalog_fingerprint(
                    declaration,
                    &provider_options,
                    provider_fingerprint.as_deref(),
                    environment.dependency_fingerprint(),
                ),
            },
        );
    }
    Ok(registrations)
}

fn catalog_dependency_fingerprint(
    environment: &str,
    datasets: &BTreeMap<DatasetStageId, DependencyFingerprint>,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"avenger-catalog-analysis-v1\0");
    hasher.update(environment.as_bytes());
    for (stage, fingerprint) in datasets {
        hasher.update(b"\0dataset\0");
        hasher.update(stage.dataset.as_str().as_bytes());
        hasher.update(stage.ordinal.to_le_bytes());
        hasher.update(b"\0");
        hasher.update(fingerprint.as_str().as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn external_catalog_fingerprint(
    declaration: &ResolvedDeclaration,
    options: &serde_json::Value,
    provider_snapshot: Option<&str>,
    environment: &str,
) -> String {
    let declaration = serde_json::to_string(declaration).unwrap_or_default();
    let options = options.to_string();
    hash_parts(
        "avenger-external-catalog-v1",
        [
            declaration.as_str(),
            options.as_str(),
            provider_snapshot.unwrap_or(""),
            environment,
        ],
    )
}

struct TableFingerprintInputs<'a> {
    dependencies: &'a BTreeMap<DeclarationId, DependencyFingerprint>,
    relation_dependencies: &'a BTreeMap<ResolvedRelationId, DependencyFingerprint>,
    logical_plan: Option<&'a str>,
    provider_snapshot: Option<&'a str>,
}

fn table_dependency_fingerprint(
    project: &ResolvedModuleGraph,
    declaration: &ResolvedDeclaration,
    table: &ResolvedCatalogTable,
    inputs: TableFingerprintInputs<'_>,
    options: &CatalogOptions<'_>,
) -> Result<DependencyFingerprint, Diagnostic> {
    let declaration_json = serde_json::to_string(declaration).map_err(|error| {
        declaration_diagnostic(declaration, "AVENGER-DATA-048", error.to_string())
    })?;
    let mut parts = vec![
        table.path.join("."),
        table.kind.clone(),
        declaration_json,
        inputs.logical_plan.unwrap_or("").to_owned(),
        inputs.provider_snapshot.unwrap_or("").to_owned(),
        options
            .capabilities
            .object_store_schemes
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\0"),
    ];
    for dependency in &table.dependencies {
        if let Some(fingerprint) = inputs.dependencies.get(dependency) {
            parts.push(fingerprint.as_str().to_owned());
        }
    }
    if let Some(ResolvedValue::Query(query)) = declaration.properties.get("sql") {
        for reference in &query.relations {
            let ResolvedRelationTarget::Relation(relation) = &reference.target else {
                continue;
            };
            if let Some(fingerprint) = inputs.relation_dependencies.get(relation) {
                parts.push(fingerprint.as_str().to_owned());
            }
        }
    }
    if matches!(
        table.kind.as_str(),
        "csv" | "json" | "parquet" | "arrow" | "ipc"
    ) {
        let path = table_path(project, declaration, options)?;
        let resource = if path.contains("://") {
            format!("object:{path}")
        } else if path.contains(['*', '?', '[']) {
            crate::compiler::glob_content_version(std::path::Path::new(&path))
                .map_err(|error| declaration_diagnostic(declaration, "AVENGER-DATA-049", error))?
        } else {
            crate::compiler::resource_content_version(std::path::Path::new(&path)).map_err(
                |error| declaration_diagnostic(declaration, "AVENGER-DATA-049", error.to_string()),
            )?
        };
        parts.push(resource);
    }
    Ok(DependencyFingerprint::new(hash_parts(
        "avenger-catalog-table-v1",
        parts.iter().map(String::as_str),
    )))
}

fn hash_parts<'a>(domain: &str, parts: impl IntoIterator<Item = &'a str>) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    for part in parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

pub(crate) fn internal_relation_name(relation: &ResolvedRelationId) -> String {
    let nested = relation.nested_path.join(".");
    let digest = hash_parts(
        "avenger-internal-relation-v1",
        [
            relation.defining_item.module.as_str(),
            relation.defining_item.declaration.as_str(),
            nested.as_str(),
        ],
    );
    format!("__avenger_rel_{}", digest.trim_start_matches("sha256:"))
}

async fn analyze_external_catalogs(
    project: &ResolvedModuleGraph,
    context: &SessionContext,
    external_catalogs: &BTreeMap<ModuleItemId, ExternalCatalogRegistration>,
    options: &CatalogOptions<'_>,
    output: ExternalCatalogAnalysisOutput<'_>,
) -> Result<(), Diagnostic> {
    for (item, declaration) in project
        .source_modules
        .values()
        .flat_map(|file| {
            file.item_order
                .iter()
                .zip(&file.roots)
                .filter(|(item, _)| item_is_reachable(item, options))
        })
        .filter(|(_, declaration)| declaration.keyword == "catalog")
    {
        let kind = declaration.kind.as_deref().unwrap_or("schemas");
        if matches!(kind, "schemas" | "memory") {
            continue;
        }
        let Some(catalog_name) = declaration.name.as_deref() else {
            continue;
        };
        let registration = external_catalogs.get(item).ok_or_else(|| {
            declaration_diagnostic(
                declaration,
                "AVENGER-DATA-023",
                format!("catalog factory did not create `{catalog_name}`"),
            )
        })?;
        let catalog = &registration.provider;
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
                let relation = ResolvedRelationId {
                    defining_item: item.clone(),
                    nested_path: vec![schema_name.to_owned(), table_name.clone()],
                };
                register_table(
                    context,
                    &[internal_relation_name(&relation)],
                    Arc::clone(&provider),
                )
                .map_err(|error| declaration_diagnostic(projection, "AVENGER-DATA-022", error))?;
                let dataset_id = ModuleDatasetId::new(format!(
                    "provider:{}:{}:{}",
                    item.module.as_str(),
                    item.declaration.as_str(),
                    relation.nested_path.join(".")
                ));
                let stage = DatasetStageId::new(dataset_id.clone(), 0);
                output
                    .datasets
                    .insert(AnalyzedDataset {
                        id: dataset_id.clone(),
                        stage: stage.clone(),
                        provenance: DatasetProvenance {
                            declaration_span: declaration.span,
                            stage_span: projection.span,
                            stage_kind: DatasetStageKind::CatalogTable,
                        },
                        qualified_name: Some(qualified.clone()),
                        qualified_path: Some(vec![
                            catalog_name.to_owned(),
                            schema_name.to_owned(),
                            table_name.to_owned(),
                        ]),
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
                let fingerprint = DependencyFingerprint::new(hash_parts(
                    "avenger-external-dataset-v1",
                    [registration.fingerprint.as_str(), qualified.as_str()],
                ));
                output
                    .dataset_fingerprints
                    .insert(stage.clone(), fingerprint.clone());
                output
                    .relation_stages
                    .insert(relation.clone(), stage.clone());
                output.relation_fingerprints.insert(relation, fingerprint);
                output
                    .lineage
                    .insert(stage, DatasetLineage::default())
                    .map_err(|error| {
                        declaration_diagnostic(projection, "AVENGER-DATA-026", error.to_string())
                    })?;
            }
        }
    }
    Ok(())
}

struct ExternalCatalogAnalysisOutput<'a> {
    datasets: &'a mut DatasetSchemaIndex,
    lineage: &'a mut DatasetLineageIndex,
    dataset_fingerprints: &'a mut BTreeMap<DatasetStageId, DependencyFingerprint>,
    relation_stages: &'a mut BTreeMap<ResolvedRelationId, DatasetStageId>,
    relation_fingerprints: &'a mut BTreeMap<ResolvedRelationId, DependencyFingerprint>,
}

fn item_is_reachable(item: &ModuleItemId, options: &CatalogOptions<'_>) -> bool {
    options
        .reachable_items
        .is_none_or(|reachable| reachable.contains(item))
}

async fn create_table_provider(
    project: &ResolvedModuleGraph,
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
            ensure_query_relations_registered(context, query).map_err(|message| {
                declaration_diagnostic(declaration, "AVENGER-DATA-108", message)
            })?;
            let sql = expand_catalog_sql(project, query, Some(table), true).map_err(|message| {
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
    project: &ResolvedModuleGraph,
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
    let source = project.authored_source(declaration.span).ok_or_else(|| {
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
pub(crate) fn expand_chart_sql(
    project: &ResolvedModuleGraph,
    query: &ResolvedQuery,
    sql: &str,
) -> Result<String, String> {
    expand_catalog_sql_with_sql(project, query, sql, None, false)
}

pub(crate) fn ensure_query_relations_registered(
    context: &SessionContext,
    query: &ResolvedQuery,
) -> Result<(), String> {
    for reference in &query.relations {
        let ResolvedRelationTarget::Relation(relation) = &reference.target else {
            continue;
        };
        ensure_relation_registered(context, relation, &reference.authored_path)?;
    }
    Ok(())
}

pub(crate) fn ensure_relation_registered(
    context: &SessionContext,
    relation: &ResolvedRelationId,
    authored_path: &[String],
) -> Result<(), String> {
    let internal = internal_relation_name(relation);
    if context
        .table_exist(&internal)
        .map_err(|error| error.to_string())?
    {
        Ok(())
    } else {
        Err(format!(
            "dataset `{}` was not exposed by its provider",
            authored_path.join(".")
        ))
    }
}

fn expand_catalog_sql(
    project: &ResolvedModuleGraph,
    query: &ResolvedQuery,
    caller: Option<&ResolvedCatalogTable>,
    bind_caller_defaults: bool,
) -> Result<String, String> {
    expand_catalog_sql_with_sql(project, query, &query.sql, caller, bind_caller_defaults)
}

fn expand_catalog_sql_with_sql(
    project: &ResolvedModuleGraph,
    query: &ResolvedQuery,
    sql: &str,
    caller: Option<&ResolvedCatalogTable>,
    bind_caller_defaults: bool,
) -> Result<String, String> {
    let mut statement = parse_query_statement(sql)?;
    if bind_caller_defaults && let Some(caller) = caller {
        let bindings = default_param_expressions(project, caller)?;
        substitute_query_params(&mut statement, &bindings)?;
    }
    let relation_map = query_relation_map(query);
    let mut expander = TableFunctionExpander {
        project,
        relations: &relation_map,
        error: None,
    };
    if let ControlFlow::Break(()) = statement.visit(&mut expander) {
        return Err(expander
            .error
            .unwrap_or_else(|| "table-function expansion stopped".to_owned()));
    }
    rewrite_query_relations(&mut statement, &relation_map);
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

fn query_relation_map(query: &ResolvedQuery) -> BTreeMap<Vec<String>, ResolvedRelationId> {
    query
        .relations
        .iter()
        .filter_map(|reference| match &reference.target {
            ResolvedRelationTarget::Relation(relation) => {
                Some((reference.authored_path.clone(), relation.clone()))
            }
            ResolvedRelationTarget::Input => None,
        })
        .collect()
}

fn object_name_path(name: &ObjectName) -> Vec<String> {
    name.0
        .iter()
        .filter_map(|part| part.as_ident())
        .map(|identifier| identifier.value.clone())
        .collect()
}

fn rewrite_query_relations(
    statement: &mut Statement,
    relations: &BTreeMap<Vec<String>, ResolvedRelationId>,
) {
    struct Rewriter<'a>(&'a BTreeMap<Vec<String>, ResolvedRelationId>);

    impl VisitorMut for Rewriter<'_> {
        type Break = ();

        fn pre_visit_relation(&mut self, relation: &mut ObjectName) -> ControlFlow<Self::Break> {
            if let Some(target) = self.0.get(&object_name_path(relation)) {
                *relation =
                    ObjectName::from(vec![Ident::with_quote('"', internal_relation_name(target))]);
            }
            ControlFlow::Continue(())
        }
    }

    let _ = statement.visit(&mut Rewriter(relations));
}

struct TableFunctionExpander<'a> {
    project: &'a ResolvedModuleGraph,
    relations: &'a BTreeMap<Vec<String>, ResolvedRelationId>,
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
        let authored_path = object_name_path(name);
        let Some(relation) = self.relations.get(&authored_path) else {
            // DataFusion may own ordinary table functions; only Avenger table
            // declarations are rewritten here.
            return ControlFlow::Continue(());
        };
        let Some(arguments) = args else {
            return ControlFlow::Continue(());
        };
        let Some(table) = self.project.catalog_tables.get(relation) else {
            self.error = Some(format!(
                "resolved relation `{}` does not support table arguments",
                authored_path.join(".")
            ));
            return ControlFlow::Break(());
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
    project: &ResolvedModuleGraph,
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
    let relation_map = query_relation_map(query);
    let mut expander = TableFunctionExpander {
        project,
        relations: &relation_map,
        error: None,
    };
    if let ControlFlow::Break(()) = statement.visit(&mut expander) {
        return Err(expander
            .error
            .unwrap_or_else(|| "nested table-function expansion stopped".to_owned()));
    }
    rewrite_query_relations(&mut statement, &relation_map);
    match statement {
        Statement::Query(query) => Ok(*query),
        _ => unreachable!(),
    }
}

fn default_param_expressions(
    project: &ResolvedModuleGraph,
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

fn declaration_index(
    project: &ResolvedModuleGraph,
) -> BTreeMap<DeclarationId, &ResolvedDeclaration> {
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
    for root in project.source_modules.values().flat_map(|file| &file.roots) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_lang_core::{DeclarationKey, ModuleItemId, SourceModuleId};

    #[test]
    fn semantic_relations_rewrite_to_one_quoted_internal_name() {
        let relation = ResolvedRelationId {
            defining_item: ModuleItemId {
                module: SourceModuleId::new("sha256:module"),
                declaration: DeclarationKey::new("item"),
            },
            nested_path: vec!["schema".to_owned(), "movies".to_owned()],
        };
        let mut statement = parse_query_statement("SELECT m.title FROM data.movies AS m").unwrap();
        rewrite_query_relations(
            &mut statement,
            &BTreeMap::from([(
                vec!["data".to_owned(), "movies".to_owned()],
                relation.clone(),
            )]),
        );
        let sql = statement.to_string();
        assert!(sql.contains(&format!("\"{}\"", internal_relation_name(&relation))));
        assert!(sql.contains("AS m"));
        assert!(sql.contains("m.title"));
        assert!(!sql.contains("data.movies"));
    }
}
