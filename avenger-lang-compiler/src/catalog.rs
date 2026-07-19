//! DataFusion-backed catalog registration and execution-free schema analysis.

use std::{collections::BTreeMap, sync::Arc};

use avenger_lang_core::{
    DataCapabilities, DeclarationId, Diagnostic, EnvironmentProvider, ResolvedCatalogTable,
    ResolvedDeclaration, ResolvedProject, ResolvedValue, SourceLabel, SourceOrigin,
    project::normalize_path,
};
use datafusion::{
    catalog::{CatalogProvider, MemoryCatalogProvider, MemorySchemaProvider},
    common::TableReference,
    datasource::TableProvider,
    prelude::{CsvReadOptions, JsonReadOptions, ParquetReadOptions, SessionContext},
};

use crate::{
    AnalyzedColumn, AnalyzedDataset, CatalogFactoryRegistry, CompileEnvironment, DatasetLineage,
    DatasetLineageIndex, DatasetProvenance, DatasetSchemaIndex, DatasetStageId, DatasetStageKind,
    ProjectDatasetId, TableFactoryRegistry,
};

pub(crate) struct CatalogAnalysis {
    pub datasets: DatasetSchemaIndex,
    pub lineage: DatasetLineageIndex,
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

    for id in &project.table_order {
        let Some(table) = table_by_id.get(id).copied() else {
            continue;
        };
        let declaration = declarations.get(id).copied().ok_or_else(|| {
            catalog_diagnostic(table, "AVENGER-DATA-001", "catalog declaration is missing")
        })?;
        let provider =
            create_table_provider(project, declaration, table, environment, &options).await?;
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

    Ok(CatalogAnalysis { datasets, lineage })
}

async fn register_external_catalogs(
    project: &ResolvedProject,
    environment: &CompileEnvironment,
    options: &CatalogOptions<'_>,
) -> Result<(), Diagnostic> {
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
        let provider_options = declaration_options(
            &declaration.properties,
            options.capabilities,
            options.environment_provider,
            declaration,
        )?;
        let provider = factory
            .create(&provider_options, environment)
            .await
            .map_err(|error| {
                declaration_diagnostic(declaration, "AVENGER-DATA-021", error.to_string())
            })?;
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
    Ok(())
}

async fn create_table_provider(
    project: &ResolvedProject,
    declaration: &ResolvedDeclaration,
    table: &ResolvedCatalogTable,
    environment: &CompileEnvironment,
    options: &CatalogOptions<'_>,
) -> Result<Arc<dyn TableProvider>, Diagnostic> {
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
            let sql = bind_table_defaults(project, table, &query.sql)?;
            context
                .sql(&sql)
                .await
                .map(|dataframe| dataframe.into_view())
                .map_err(|error| {
                    declaration_diagnostic(declaration, "AVENGER-DATA-031", error.to_string())
                })
        }
        "csv" | "json" | "parquet" => {
            let path = table_path(project, declaration, options)?;
            let dataframe = match table.kind.as_str() {
                "csv" => context.read_csv(path, CsvReadOptions::new()).await,
                "json" => context.read_json(path, JsonReadOptions::default()).await,
                "parquet" => {
                    context
                        .read_parquet(path, ParquetReadOptions::default())
                        .await
                }
                _ => unreachable!(),
            }
            .map_err(|error| {
                declaration_diagnostic(declaration, "AVENGER-DATA-032", error.to_string())
            })?;
            Ok(dataframe.into_view())
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
                .map_err(|error| {
                    declaration_diagnostic(declaration, "AVENGER-DATA-034", error.to_string())
                })
        }
    }
}

async fn inline_table(
    context: &SessionContext,
    declaration: &ResolvedDeclaration,
) -> Result<Arc<dyn TableProvider>, Diagnostic> {
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
        .map(|dataframe| dataframe.into_view())
        .map_err(|error| declaration_diagnostic(declaration, "AVENGER-DATA-037", error.to_string()))
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

fn bind_table_defaults(
    project: &ResolvedProject,
    table: &ResolvedCatalogTable,
    sql: &str,
) -> Result<String, Diagnostic> {
    let mut bound = sql.to_owned();
    for param_id in &table.params {
        let param = &project.params[param_id];
        let literal = scalar_literal_sql(&param.default)
            .map_err(|message| catalog_diagnostic(table, "AVENGER-DATA-050", message))?;
        bound = replace_placeholder(&bound, &param.source_name, &literal);
    }
    Ok(bound)
}

fn replace_placeholder(sql: &str, name: &str, replacement: &str) -> String {
    let needle = format!("${name}");
    let mut result = String::with_capacity(sql.len());
    let mut rest = sql;
    while let Some(offset) = rest.find(&needle) {
        result.push_str(&rest[..offset]);
        let after = &rest[offset + needle.len()..];
        if after
            .chars()
            .next()
            .is_some_and(|character| character == '_' || character.is_alphanumeric())
        {
            result.push_str(&needle);
        } else {
            result.push_str(replacement);
        }
        rest = after;
    }
    result.push_str(rest);
    result
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
