use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use arrow::{
    array::Int64Array,
    datatypes::{DataType, Field, Schema, SchemaRef},
    record_batch::RecordBatch,
};
use async_trait::async_trait;
use avenger_lang_compiler::{
    CatalogFactory, CatalogFactoryError, CatalogFactoryRegistry, CompileEnvironment, Compiler,
    DatasetStageKind, TableFactory, TableFactoryError, TableFactoryRegistry,
};
use avenger_lang_core::{
    ContentVersion, DataCapabilities, InMemorySourceLoader, LoadedSource, MapEnvironmentProvider,
    SourceLoader, SourceOrigin,
};
use datafusion::{
    catalog::{
        CatalogProvider, MemoryCatalogProvider, MemorySchemaProvider, SchemaProvider, Session,
    },
    datasource::{TableProvider, TableType, memory::MemTable},
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

struct MockIcebergFactory(&'static str);

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

    async fn dependency_fingerprint(
        &self,
        _options: &serde_json::Value,
        _environment: &CompileEnvironment,
    ) -> Result<Option<String>, CatalogFactoryError> {
        Ok(Some(self.0.to_owned()))
    }
}

struct MockDeltaFactory(&'static str);

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

    async fn dependency_fingerprint(
        &self,
        _options: &serde_json::Value,
        _environment: &CompileEnvironment,
    ) -> Result<Option<String>, TableFactoryError> {
        Ok(Some(self.0.to_owned()))
    }
}

#[derive(Debug)]
struct CountingTable {
    scans: Arc<AtomicUsize>,
    table: MemTable,
}

#[async_trait]
impl TableProvider for CountingTable {
    fn schema(&self) -> SchemaRef {
        self.table.schema()
    }

    fn table_type(&self) -> TableType {
        TableType::Base
    }

    async fn scan(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        self.scans.fetch_add(1, Ordering::SeqCst);
        self.table.scan(state, projection, filters, limit).await
    }
}

struct CountingFactory(Arc<AtomicUsize>);

#[async_trait]
impl TableFactory for CountingFactory {
    async fn create(
        &self,
        _options: &serde_json::Value,
        _environment: &CompileEnvironment,
    ) -> Result<Arc<dyn TableProvider>, TableFactoryError> {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "value",
            DataType::Int64,
            false,
        )]));
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![Arc::new(Int64Array::from(vec![1_i64, 2]))],
        )
        .unwrap();
        Ok(Arc::new(CountingTable {
            scans: Arc::clone(&self.0),
            table: MemTable::try_new(schema, vec![vec![batch]]).unwrap(),
        }))
    }
}

fn provider_registries(
    iceberg_snapshot: &'static str,
    delta_snapshot: &'static str,
) -> (CatalogFactoryRegistry, TableFactoryRegistry) {
    let mut catalogs = CatalogFactoryRegistry::default();
    catalogs
        .register("iceberg", Arc::new(MockIcebergFactory(iceberg_snapshot)))
        .unwrap();
    let mut tables = TableFactoryRegistry::default();
    tables
        .register("delta", Arc::new(MockDeltaFactory(delta_snapshot)))
        .unwrap();
    (catalogs, tables)
}

#[tokio::test]
async fn catalog_project_analyzes_qualified_tables_and_sql_views_without_execution() {
    let root = project_fixture("06_catalog_project");
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let analysis = compiler.analyze_project(&root).await.unwrap();
    let tables = analysis
        .datasets
        .iter()
        .filter(|(_, dataset)| {
            matches!(
                dataset.provenance.stage_kind,
                DatasetStageKind::CatalogTable | DatasetStageKind::SqlView
            )
        })
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
        tables["vega.popular"].logical_plan_fingerprint,
        tables["vega.popular_from_first"].logical_plan_fingerprint,
        "standard and FROM-first forms must reach equivalent DataFusion logical plans"
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

    let (catalog_factories, table_factories) =
        provider_registries("iceberg-snapshot-1", "delta-version-7");
    let denied_environment = Compiler::builder()
        .project_root(&root)
        .catalog_factories(catalog_factories)
        .table_factories(table_factories)
        .environment(Arc::new(MapEnvironmentProvider::new([(
            "ICEBERG_TOKEN".to_owned(),
            "fixture-token".to_owned(),
        )])))
        .build()
        .unwrap();
    let failure = denied_environment.analyze_project(&root).await.unwrap_err();
    assert_eq!(failure.diagnostics[0].code.as_str(), "AVENGER-DATA-060");

    let (catalog_factories, table_factories) =
        provider_registries("iceberg-snapshot-1", "delta-version-7");
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

    let project = compiler.compile_project(&root).await.unwrap();
    assert_eq!(
        project.project_fingerprint.as_str(),
        analysis.project_fingerprint.as_str()
    );
    assert!(project.charts.values().all(|chart| {
        chart.dependency_fingerprint.as_str() == analysis.project_fingerprint.as_str()
    }));

    let (catalog_factories, table_factories) =
        provider_registries("iceberg-snapshot-2", "delta-version-7");
    let mut capabilities = DataCapabilities::project();
    capabilities.allow_environment = true;
    capabilities
        .environment_names
        .insert("ICEBERG_TOKEN".to_owned());
    let changed = Compiler::builder()
        .project_root(&root)
        .catalog_factories(catalog_factories)
        .table_factories(table_factories)
        .data_capabilities(capabilities)
        .environment(Arc::new(MapEnvironmentProvider::new([(
            "ICEBERG_TOKEN".to_owned(),
            "fixture-token".to_owned(),
        )])))
        .build()
        .unwrap()
        .analyze_project(&root)
        .await
        .unwrap();
    assert_ne!(
        changed.project_fingerprint.as_str(),
        analysis.project_fingerprint.as_str()
    );
}

#[tokio::test]
async fn catalog_session_materialization_is_lazy_and_reused_within_one_generation() {
    let loader = InMemorySourceLoader::default()
        .with_source(LoadedSource::new(
            SourceOrigin::File("/project/chart.avenger".into()),
            "avenger 1; import 'catalog.data.avenger' as live; chart cartesian as chart {}",
            ContentVersion::new("chart-v1"),
        ))
        .with_source(LoadedSource::new(
            SourceOrigin::File("/project/catalog.data.avenger".into()),
            "avenger 1; schema tables as test { \
             table counting as rows { materialize: session; } }",
            ContentVersion::new("catalog-v1"),
        ));
    let scans = Arc::new(AtomicUsize::new(0));
    let mut factories = TableFactoryRegistry::default();
    factories
        .register("counting", Arc::new(CountingFactory(Arc::clone(&scans))))
        .unwrap();
    let generation = Compiler::builder()
        .project_root("/project")
        .source_loader(Arc::new(loader) as Arc<dyn SourceLoader>)
        .table_factories(factories)
        .build()
        .unwrap()
        .compile_file_generation_attempt("chart.avenger", 9)
        .await
        .result
        .unwrap();
    assert_eq!(
        scans.load(Ordering::SeqCst),
        0,
        "compilation is schema-only"
    );

    for _ in 0..2 {
        let rows = generation
            .environment
            .session_context()
            .sql("SELECT * FROM live.rows")
            .await
            .unwrap()
            .collect()
            .await
            .unwrap();
        assert_eq!(rows.iter().map(RecordBatch::num_rows).sum::<usize>(), 2);
    }
    assert_eq!(
        scans.load(Ordering::SeqCst),
        1,
        "the source provider is loaded once per session"
    );
}
