use std::{path::PathBuf, sync::Arc};

use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use async_trait::async_trait;
use avenger_lang_compiler::{
    CatalogFactory, CatalogFactoryError, CatalogFactoryRegistry, CompileEnvironment, Compiler,
    DatasetStageKind, TableFactory, TableFactoryError, TableFactoryRegistry,
};
use avenger_lang_core::{DataCapabilities, MapEnvironmentProvider};
use datafusion::{
    catalog::{
        CatalogProvider, MemoryCatalogProvider, MemorySchemaProvider, SchemaProvider, Session,
    },
    datasource::{TableProvider, TableType},
    error::Result as DataFusionResult,
    logical_expr::Expr,
    physical_plan::ExecutionPlan,
};

fn project_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/projects")
        .join(name)
}

#[derive(Debug)]
struct PanicOnScanTable {
    schema: SchemaRef,
}

#[async_trait]
impl TableProvider for PanicOnScanTable {
    fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }

    fn table_type(&self) -> TableType {
        TableType::Base
    }

    async fn scan(
        &self,
        _state: &dyn Session,
        _projection: Option<&Vec<usize>>,
        _filters: &[Expr],
        _limit: Option<usize>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        panic!("schema analysis must never scan a provider")
    }
}

fn schema_only_table() -> Arc<dyn TableProvider> {
    Arc::new(PanicOnScanTable {
        schema: Arc::new(Schema::new(vec![Field::new(
            "provider_value",
            DataType::Int64,
            false,
        )])),
    })
}

struct MockIcebergFactory;

#[async_trait]
impl CatalogFactory for MockIcebergFactory {
    async fn create(
        &self,
        options: &serde_json::Value,
        _environment: &CompileEnvironment,
    ) -> Result<Arc<dyn CatalogProvider>, CatalogFactoryError> {
        assert_eq!(options["token"], "fixture-token");
        assert_eq!(
            options["schemas"]["analytics"]["path"],
            serde_json::json!(["organization", "analytics"])
        );
        let catalog = MemoryCatalogProvider::new();
        let schema = MemorySchemaProvider::new();
        schema
            .register_table("remote_events".to_owned(), schema_only_table())
            .unwrap();
        catalog
            .register_schema("analytics", Arc::new(schema))
            .unwrap();
        Ok(Arc::new(catalog))
    }
}

struct MockDeltaFactory;

#[async_trait]
impl TableFactory for MockDeltaFactory {
    async fn create(
        &self,
        options: &serde_json::Value,
        _environment: &CompileEnvironment,
    ) -> Result<Arc<dyn TableProvider>, TableFactoryError> {
        assert_eq!(options["uri"], "s3://warehouse/events");
        Ok(schema_only_table())
    }
}

#[tokio::test]
async fn catalog_project_analyzes_qualified_tables_and_sql_views_without_execution() {
    let root = project_fixture("06_catalog_project");
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let analysis = compiler.analyze_project(&root).await.unwrap();
    let tables = analysis
        .datasets
        .iter()
        .map(|(_, dataset)| {
            (
                dataset.qualified_name.as_deref().unwrap().to_owned(),
                dataset,
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();

    assert_eq!(tables.len(), 4);
    assert!(tables.contains_key("regions"));
    assert!(tables.contains_key("vega.movies"));
    assert!(matches!(
        tables["vega.popular"].provenance.stage_kind,
        DatasetStageKind::SqlView
    ));
    assert_eq!(
        tables["vega.popular"]
            .columns
            .iter()
            .map(|column| (&column.name, &column.data_type, column.nullable))
            .collect::<Vec<_>>(),
        tables["vega.popular_from_first"]
            .columns
            .iter()
            .map(|column| (&column.name, &column.data_type, column.nullable))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        tables["vega.popular"].columns[1].data_type,
        DataType::Float64
    );
}

#[tokio::test]
async fn catalog_project_compiles_two_charts_against_one_registered_catalog() {
    let root = project_fixture("06_catalog_project");
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let project = compiler.compile_project(&root).await.unwrap();
    assert_eq!(project.charts.len(), 2);
}

#[tokio::test]
async fn catalog_parameterized_tables_bind_defaults_named_args_and_forwarding() {
    let root = project_fixture("phase9-parameterized");
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let analysis = compiler.analyze_project(&root).await.unwrap();
    let forwarded = analysis
        .datasets
        .iter()
        .map(|(_, dataset)| dataset)
        .find(|dataset| dataset.qualified_name.as_deref() == Some("local.forwarded"))
        .unwrap();
    assert_eq!(forwarded.columns[0].data_type, DataType::Utf8);
    assert_eq!(forwarded.columns[1].data_type, DataType::Int64);

    let artifact = compiler.compile_file("chart.avenger").await.unwrap();
    assert!(artifact.interface.params.contains_key("selected"));
}

#[tokio::test]
async fn catalog_pack_alias_retargets_public_paths_without_capturing_internal_chains() {
    let root = project_fixture("phase9-pack");
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let analysis = compiler.analyze_project(&root).await.unwrap();
    let names = analysis
        .datasets
        .iter()
        .filter_map(|(_, dataset)| dataset.qualified_name.as_deref())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(names.contains("samples.movies"));
    assert!(names.contains("samples.popular"));
    compiler.compile_file("chart.avenger").await.unwrap();
}

#[tokio::test]
async fn catalog_file_providers_expose_csv_json_parquet_directory_glob_and_ipc_schemas() {
    let root = project_fixture("phase9-files");
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let analysis = compiler.analyze_project(&root).await.unwrap();
    let names = analysis
        .datasets
        .iter()
        .filter_map(|(_, dataset)| dataset.qualified_name.as_deref())
        .collect::<std::collections::BTreeSet<_>>();
    for expected in [
        "files.rows",
        "files.events",
        "files.iris",
        "files.directory",
        "files.globbed",
        "files.taxi",
    ] {
        assert!(names.contains(expected), "missing schema for {expected}");
    }
    compiler.compile_file("chart.avenger").await.unwrap();
}

#[tokio::test]
async fn catalog_provider_factories_are_explicit_schema_only_and_environment_gated() {
    let root = project_fixture("phase9-providers");

    let denied = Compiler::builder().project_root(&root).build().unwrap();
    let failure = denied.analyze_project(&root).await.unwrap_err();
    assert_eq!(failure.diagnostics[0].code.as_str(), "AVENGER-DATA-020");

    let mut catalog_factories = CatalogFactoryRegistry::default();
    catalog_factories
        .register("iceberg", Arc::new(MockIcebergFactory))
        .unwrap();
    let mut table_factories = TableFactoryRegistry::default();
    table_factories
        .register("delta", Arc::new(MockDeltaFactory))
        .unwrap();
    let mut capabilities = DataCapabilities::project();
    capabilities.allow_environment = true;
    capabilities
        .environment_names
        .insert("ICEBERG_TOKEN".to_owned());
    let compiler = Compiler::builder()
        .project_root(&root)
        .catalog_factories(catalog_factories)
        .table_factories(table_factories)
        .data_capabilities(capabilities)
        .environment(Arc::new(MapEnvironmentProvider::new([(
            "ICEBERG_TOKEN".to_owned(),
            "fixture-token".to_owned(),
        )])))
        .build()
        .unwrap();
    let analysis = compiler.analyze_project(&root).await.unwrap();
    let names = analysis
        .datasets
        .iter()
        .filter_map(|(_, dataset)| dataset.qualified_name.as_deref())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(names.contains("warehouse.analytics.remote_events"));
    assert!(names.contains("local.events"));
}
