use std::{
    fs,
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
    CatalogFactory, CatalogFactoryError, CatalogFactoryRegistry, CompileEnvironment,
    CompileEnvironmentError, CompileEnvironmentFactory, CompileEnvironmentRequest, Compiler,
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

struct FreshEnvironmentFactory;

impl CompileEnvironmentFactory for FreshEnvironmentFactory {
    fn create(
        &self,
        _request: &CompileEnvironmentRequest,
    ) -> Result<CompileEnvironment, CompileEnvironmentError> {
        Ok(CompileEnvironment::new(
            datafusion::prelude::SessionContext::new(),
        ))
    }
}

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

struct CreateCountingFactory(Arc<AtomicUsize>);

#[async_trait]
impl TableFactory for CreateCountingFactory {
    async fn create(
        &self,
        _options: &serde_json::Value,
        _environment: &CompileEnvironment,
    ) -> Result<Arc<dyn TableProvider>, TableFactoryError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Int64, false),
            Field::new("y", DataType::Int64, false),
        ]));
        Ok(Arc::new(
            MemTable::try_new(schema, vec![Vec::new()])
                .map_err(|error| TableFactoryError::Message(error.to_string()))?,
        ))
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
    let analysis = compiler
        .analyze_module(root.join("data.avenger"))
        .await
        .unwrap();
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
        DataType::Decimal128(2, 1)
    );
    assert_eq!(
        tables["vega.movies"].qualified_path.as_deref(),
        Some(["vega".to_owned(), "movies".to_owned()].as_slice())
    );
    assert!(
        analysis
            .functions
            .functions
            .iter()
            .any(|function| function.name == "abs"
                && function.category == avenger_lang_compiler::FunctionCategory::Scalar)
    );
    assert!(
        analysis
            .physical_type_constructors
            .contains(&"struct".to_owned())
    );
}

#[tokio::test]
async fn catalog_project_compiles_two_charts_against_one_registered_catalog() {
    let root = project_fixture("06_catalog_project");
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let movies = compiler
        .compile_chart("movies.avenger", None)
        .await
        .unwrap();
    let regions = compiler
        .compile_chart("regions.avenger", None)
        .await
        .unwrap();
    assert_ne!(movies.id, regions.id);
}

#[tokio::test]
async fn catalog_project_compiles_and_evaluates_each_chart_in_its_retained_generation() {
    let root = project_fixture("06_catalog_project");
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    for (generation, chart) in [(1, "movies.avenger"), (2, "regions.avenger")] {
        let compiled = compiler
            .compile_chart_generation_attempt(chart, None, generation)
            .await
            .result
            .unwrap();
        let evaluated = compiled
            .artifact
            .compiled_plot()
            .evaluate(compiled.environment.session_context(), None)
            .await
            .unwrap();
        assert!(
            !evaluated.scene_graph.groups().is_empty(),
            "{chart} should evaluate to a renderable scene"
        );
    }
}

#[tokio::test]
async fn catalog_default_and_host_generation_environments_are_analysis_and_artifact_equivalent() {
    let root = project_fixture("06_catalog_project");
    let default = Compiler::builder().project_root(&root).build().unwrap();
    let host = Compiler::builder()
        .project_root(&root)
        .environment_factory(Arc::new(FreshEnvironmentFactory))
        .build()
        .unwrap();

    let catalog = root.join("data.avenger");
    let default_analysis = default.analyze_module(&catalog).await.unwrap();
    let host_analysis = host.analyze_module(&catalog).await.unwrap();
    let snapshot = |analysis: &avenger_lang_compiler::ModuleAnalysis| {
        analysis
            .datasets
            .iter()
            .map(|(_, dataset)| {
                (
                    (dataset.id.as_str().to_owned(), dataset.stage.ordinal),
                    (
                        dataset.qualified_name.clone(),
                        dataset.columns.clone(),
                        dataset.schema.as_ref().clone(),
                        dataset.logical_plan_fingerprint.clone(),
                    ),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    assert_eq!(snapshot(&default_analysis), snapshot(&host_analysis));
    assert_eq!(
        default_analysis.module_fingerprint,
        host_analysis.module_fingerprint
    );

    let default_project = default.compile_module("movies.avenger").await.unwrap();
    let host_project = host.compile_module("movies.avenger").await.unwrap();
    assert_eq!(
        default_project.module_fingerprint,
        host_project.module_fingerprint
    );
    for (id, default_chart) in &default_project.charts {
        let host_chart = &host_project.charts[id];
        assert_eq!(default_chart.interface, host_chart.interface);
        assert_eq!(
            default_chart.dependency_fingerprint,
            host_chart.dependency_fingerprint
        );
        assert_eq!(default_chart.name, host_chart.name);
        assert_eq!(default_chart.source, host_chart.source);
        assert_eq!(
            default_chart.compiled_plot().marks().len(),
            host_chart.compiled_plot().marks().len(),
            "host environment must not alter the compiled chart surface for {id:?}"
        );
    }
}

#[tokio::test]
async fn catalog_parameterized_tables_bind_defaults_named_args_and_forwarding() {
    let root = project_fixture("phase9-parameterized");
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let analysis = compiler.analyze_module("chart.avenger").await.unwrap();
    let forwarded = analysis
        .datasets
        .iter()
        .map(|(_, dataset)| dataset)
        .find(|dataset| dataset.qualified_name.as_deref() == Some("local.forwarded"))
        .unwrap();
    assert_eq!(forwarded.columns[0].data_type, DataType::Utf8);
    assert_eq!(forwarded.columns[1].data_type, DataType::Int64);
    let configured = analysis
        .datasets
        .iter()
        .map(|(_, dataset)| dataset)
        .find(|dataset| dataset.qualified_name.as_deref() == Some("local.configured_forwarded"))
        .unwrap();
    assert!(matches!(
        configured.columns[0].data_type,
        DataType::Struct(_)
    ));

    let artifact = compiler.compile_chart("chart.avenger", None).await.unwrap();
    assert!(artifact.interface.params.contains_key("selected"));
}

#[tokio::test]
async fn catalog_pack_alias_does_not_change_defining_dataset_identity() {
    let root = project_fixture("phase9-pack");
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let analysis = compiler.analyze_module("chart.avenger").await.unwrap();
    let names = analysis
        .datasets
        .iter()
        .filter_map(|(_, dataset)| dataset.qualified_name.as_deref())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(names.contains("vega.movies"));
    assert!(names.contains("vega.popular"));
    compiler.compile_chart("chart.avenger", None).await.unwrap();
}

#[tokio::test]
async fn catalog_file_providers_expose_csv_json_parquet_directory_glob_and_ipc_schemas() {
    let root = project_fixture("phase9-files");
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let analysis = compiler.analyze_module("chart.avenger").await.unwrap();
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
    compiler.compile_chart("chart.avenger", None).await.unwrap();
}

#[tokio::test]
async fn catalog_provider_factories_are_explicit_schema_only_and_environment_gated() {
    let root = project_fixture("phase9-providers");

    let denied = Compiler::builder().project_root(&root).build().unwrap();
    let failure = denied.analyze_module("chart.avenger").await.unwrap_err();
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
    let failure = denied_environment
        .analyze_module("chart.avenger")
        .await
        .unwrap_err();
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
    let analysis = compiler.analyze_module("chart.avenger").await.unwrap();
    let names = analysis
        .datasets
        .iter()
        .filter_map(|(_, dataset)| dataset.qualified_name.as_deref())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(names.contains("warehouse.analytics.remote_events"));
    assert!(names.contains("local.events"));

    let project = compiler.compile_module("chart.avenger").await.unwrap();
    assert!(project.charts.values().all(|chart| {
        analysis
            .dependency_fingerprints
            .charts
            .get(&chart.id)
            .is_some_and(|fingerprint| fingerprint == &chart.dependency_fingerprint)
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
        .analyze_module("chart.avenger")
        .await
        .unwrap();
    assert_ne!(
        changed.module_fingerprint.as_str(),
        analysis.module_fingerprint.as_str()
    );
}

#[tokio::test]
async fn namespace_qualified_deep_relations_are_rewritten_before_datafusion_planning() {
    let project = tempfile::tempdir().unwrap();
    fs::write(
        project.path().join("data.avenger"),
        r#"avenger 1;
export catalog iceberg as warehouse {
  uri: 'https://catalog.example.invalid';
  token: env 'ICEBERG_TOKEN';
  schema namespace as analytics {
    path: ['organization', 'analytics'];
  }
}
"#,
    )
    .unwrap();
    fs::write(
        project.path().join("charts.avenger"),
        r#"avenger 1;
import * as data from './data.avenger';

table sql as selected {
  sql:
    SELECT provider_value
    FROM data.warehouse.analytics.remote_events;
}

chart cartesian as chart {
  data: { table: selected; }
}
"#,
    )
    .unwrap();

    let (catalog_factories, table_factories) =
        provider_registries("iceberg-snapshot-1", "delta-version-7");
    let mut capabilities = DataCapabilities::project();
    capabilities.allow_environment = true;
    capabilities
        .environment_names
        .insert("ICEBERG_TOKEN".to_owned());
    let compiler = Compiler::builder()
        .project_root(project.path())
        .catalog_factories(catalog_factories)
        .table_factories(table_factories)
        .data_capabilities(capabilities)
        .environment(Arc::new(MapEnvironmentProvider::new([(
            "ICEBERG_TOKEN".to_owned(),
            "fixture-token".to_owned(),
        )])))
        .build()
        .unwrap();

    let analysis = compiler.analyze_module("charts.avenger").await.unwrap();
    let selected = analysis
        .datasets
        .iter()
        .find_map(|(_, dataset)| {
            (dataset.qualified_name.as_deref() == Some("selected")).then_some(dataset)
        })
        .expect("derived table analysis");
    assert_eq!(selected.columns[0].name, "provider_value");
    assert!(
        analysis.datasets.iter().any(|(_, dataset)| {
            dataset.qualified_name.as_deref() == Some("warehouse.analytics.remote_events")
        }),
        "tooling provenance must retain the defining module's catalog path"
    );

    let chart_path = project.path().join("charts.avenger");
    fs::write(
        &chart_path,
        fs::read_to_string(&chart_path)
            .unwrap()
            .replace("remote_events", "missing_events"),
    )
    .unwrap();
    let failure = compiler.analyze_module("charts.avenger").await.unwrap_err();
    let diagnostic = failure
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code.as_str() == "AVENGER-DATA-108")
        .expect("provider relation diagnostic");
    assert!(
        diagnostic
            .primary
            .message
            .contains("data.warehouse.analytics.missing_events"),
        "{diagnostic:#?}"
    );
}

#[tokio::test]
async fn same_spelled_external_catalogs_in_different_modules_remain_distinct() {
    let project = tempfile::tempdir().unwrap();
    let catalog = r#"avenger 1;
export catalog iceberg as warehouse {
  uri: 'https://catalog.example.invalid';
  token: env 'ICEBERG_TOKEN';
  schema namespace as analytics {
    path: ['organization', 'analytics'];
  }
}
"#;
    fs::write(project.path().join("left.avenger"), catalog).unwrap();
    fs::write(project.path().join("right.avenger"), catalog).unwrap();
    fs::write(
        project.path().join("charts.avenger"),
        r#"avenger 1;
import * as left from './left.avenger';
import * as right from './right.avenger';

table sql as paired {
  sql:
    SELECT l.provider_value AS left_value,
           r.provider_value AS right_value
    FROM left.warehouse.analytics.remote_events AS l
    CROSS JOIN right.warehouse.analytics.remote_events AS r;
}

chart cartesian {
  data: { table: paired; }
}
"#,
    )
    .unwrap();

    let (catalog_factories, table_factories) =
        provider_registries("iceberg-snapshot-1", "delta-version-7");
    let mut capabilities = DataCapabilities::project();
    capabilities.allow_environment = true;
    capabilities
        .environment_names
        .insert("ICEBERG_TOKEN".to_owned());
    let analysis = Compiler::builder()
        .project_root(project.path())
        .catalog_factories(catalog_factories)
        .table_factories(table_factories)
        .data_capabilities(capabilities)
        .environment(Arc::new(MapEnvironmentProvider::new([(
            "ICEBERG_TOKEN".to_owned(),
            "fixture-token".to_owned(),
        )])))
        .build()
        .unwrap()
        .analyze_module("charts.avenger")
        .await
        .unwrap();
    let paired = analysis
        .datasets
        .iter()
        .find_map(|(_, dataset)| {
            (dataset.qualified_name.as_deref() == Some("paired")).then_some(dataset)
        })
        .expect("paired table analysis");
    assert_eq!(
        paired
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        ["left_value", "right_value"]
    );
}

#[tokio::test]
async fn catalog_session_materialization_is_schema_only_during_analysis() {
    let loader = InMemorySourceLoader::default()
        .with_source(LoadedSource::new(
            SourceOrigin::File("/project/chart.avenger".into()),
            "avenger 1;\
             import { test as live } from './data.avenger';\
             chart cartesian as chart { data: { table: live.rows; } }",
            ContentVersion::new("chart-v1"),
        ))
        .with_source(LoadedSource::new(
            SourceOrigin::File("/project/data.avenger".into()),
            "avenger 1; export schema tables as test { \
             table counting as rows { materialize: session; } }",
            ContentVersion::new("catalog-v1"),
        ));
    let scans = Arc::new(AtomicUsize::new(0));
    let mut factories = TableFactoryRegistry::default();
    factories
        .register("counting", Arc::new(CountingFactory(Arc::clone(&scans))))
        .unwrap();
    Compiler::builder()
        .project_root("/project")
        .source_loader(Arc::new(loader) as Arc<dyn SourceLoader>)
        .table_factories(factories)
        .build()
        .unwrap()
        .analyze_module("chart.avenger")
        .await
        .unwrap();
    assert_eq!(
        scans.load(Ordering::SeqCst),
        0,
        "compilation is schema-only"
    );
}

#[tokio::test]
async fn selected_chart_initializes_only_reachable_table_providers() {
    let loader = InMemorySourceLoader::default()
        .with_source(LoadedSource::new(
            SourceOrigin::File("/project/chart.avenger".into()),
            "avenger 1;\
             import { used } from './used.avenger';\
             import { unused } from './unused.avenger';\
             chart cartesian as chart {\
               data: { table: used; }\
               mark symbol { x: encoded 'x'; y: encoded 'y'; }\
             }",
            ContentVersion::new("chart-v1"),
        ))
        .with_source(LoadedSource::new(
            SourceOrigin::File("/project/used.avenger".into()),
            "avenger 1; export table counting as used {}",
            ContentVersion::new("used-v1"),
        ))
        .with_source(LoadedSource::new(
            SourceOrigin::File("/project/unused.avenger".into()),
            "avenger 1; export table counting as unused {}",
            ContentVersion::new("unused-v1"),
        ));
    let creates = Arc::new(AtomicUsize::new(0));
    let mut factories = TableFactoryRegistry::default();
    factories
        .register(
            "counting",
            Arc::new(CreateCountingFactory(Arc::clone(&creates))),
        )
        .unwrap();
    let compiler = Compiler::builder()
        .project_root("/project")
        .source_loader(Arc::new(loader) as Arc<dyn SourceLoader>)
        .table_factories(factories)
        .build()
        .unwrap();

    compiler.compile_chart("chart.avenger", None).await.unwrap();
    assert_eq!(
        creates.load(Ordering::SeqCst),
        1,
        "the unused imported table must not request credentials or construct a provider"
    );

    compiler.analyze_module("chart.avenger").await.unwrap();
    assert_eq!(
        creates.load(Ordering::SeqCst),
        3,
        "editor/check analysis remains eager so both imported schemas are available"
    );
}
