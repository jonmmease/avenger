use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use arrow::datatypes::{DataType, Field, Schema};
use async_trait::async_trait;
use avenger_lang_compiler::{
    CompileEnvironment, CompileEnvironmentError, CompileEnvironmentFactory,
    CompileEnvironmentRequest, Compiler, ModuleCompilationMode, TableFactory, TableFactoryError,
    TableFactoryRegistry,
};
use datafusion::datasource::{TableProvider, memory::MemTable};
use datafusion::prelude::SessionContext;
use tokio::sync::Notify;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/projects/08_multi_chart_project")
}

struct TempProject(PathBuf);

impl TempProject {
    fn copy_fixture() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "avenger-lang-phase10-{}-{nonce}",
            std::process::id()
        ));
        copy_tree(&fixture(), &path);
        Self(path)
    }

    fn empty() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "avenger-lang-phase10-empty-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    let mut entries = fs::read_dir(source)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    entries.sort_by_key(fs::DirEntry::path);
    for entry in entries {
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn named_artifacts(
    project: &avenger_lang_compiler::CompiledModule,
) -> BTreeMap<String, &avenger_lang_compiler::CompiledChartArtifact> {
    project
        .charts
        .values()
        .map(|chart| {
            (
                chart
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("{:?}", chart.id.selector)),
                chart,
            )
        })
        .collect()
}

fn same_compiled_plot(
    left: &avenger_lang_compiler::CompiledChartArtifact,
    right: &avenger_lang_compiler::CompiledChartArtifact,
) -> bool {
    Arc::ptr_eq(&left.compiled, &right.compiled)
}

#[tokio::test]
async fn project_compile_cold_and_warm_are_equivalent_and_reuse_all_artifacts() {
    let root = fixture();
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let cold = compiler.compile_module(&root).await.unwrap();
    let warm = compiler.compile_module(&root).await.unwrap();

    assert_eq!(
        cold.charts
            .values()
            .map(|chart| chart.name.as_deref().unwrap())
            .collect::<Vec<_>>(),
        ["cartesian", "geo", "polar"]
    );
    assert_eq!(cold.module_fingerprint, warm.module_fingerprint);
    assert_eq!(cold.dependency_fingerprints, warm.dependency_fingerprints);
    let analysis = compiler.analyze_module(&root).await.unwrap();
    assert_eq!(cold.module_fingerprint, analysis.module_fingerprint);
    assert_eq!(
        cold.dependency_fingerprints,
        analysis.dependency_fingerprints
    );
    assert_eq!(
        analysis.dependency_fingerprints.datasets.len(),
        analysis.datasets.iter().count(),
        "compiled and standalone analysis must describe every dataset stage"
    );
    let cold = named_artifacts(&cold);
    let warm = named_artifacts(&warm);
    for name in cold.keys() {
        assert!(
            same_compiled_plot(cold[name], warm[name]),
            "warm miss: {name}"
        );
        assert_eq!(
            cold[name].to_bytes().unwrap(),
            warm[name].to_bytes().unwrap()
        );
    }
    let cache = compiler.cache_snapshot();
    assert_eq!(cache.project_analyses, 1);
    assert_eq!(cache.chart_artifacts, 3);
    assert_eq!(cache.artifact_keys.len(), 3);

    assert_eq!(cold["cartesian"].interface.params.len(), 1);
    assert_eq!(cold["polar"].interface.params.len(), 1);
    assert_eq!(cold["geo"].interface.params.len(), 1);
    assert_ne!(
        cold["cartesian"].interface.params["point_size"].migration_key,
        cold["polar"].interface.params["point_size"].migration_key,
        "same public param spelling must retain distinct chart-local migration identities"
    );
}

#[tokio::test]
async fn project_compile_single_chart_edit_invalidates_only_that_artifact() {
    let project = TempProject::copy_fixture();
    let compiler = Compiler::builder()
        .project_root(&project.0)
        .build()
        .unwrap();
    let before = compiler.compile_module(&project.0).await.unwrap();
    let chart = project.0.join("cartesian.avenger");
    fs::write(
        &chart,
        fs::read_to_string(&chart)
            .unwrap()
            .replace("value: 64.0", "value: 96.0"),
    )
    .unwrap();
    let after = compiler.compile_module(&project.0).await.unwrap();
    let before = named_artifacts(&before);
    let after = named_artifacts(&after);

    assert!(!same_compiled_plot(before["cartesian"], after["cartesian"]));
    assert!(same_compiled_plot(before["polar"], after["polar"]));
    assert!(same_compiled_plot(before["geo"], after["geo"]));
    assert_eq!(
        compiler.cache_snapshot().chart_artifacts,
        3,
        "the superseded chart artifact must be evicted"
    );
}

#[tokio::test]
async fn project_compile_shared_definition_edit_invalidates_only_importers() {
    let project = TempProject::copy_fixture();
    let compiler = Compiler::builder()
        .project_root(&project.0)
        .build()
        .unwrap();
    let before = compiler.compile_module(&project.0).await.unwrap();
    let definition = project.0.join("pass.transform.avenger");
    fs::write(
        &definition,
        fs::read_to_string(&definition)
            .unwrap()
            .replace("predicate: true", "predicate: true AND true"),
    )
    .unwrap();
    let after = compiler.compile_module(&project.0).await.unwrap();
    let before = named_artifacts(&before);
    let after = named_artifacts(&after);

    assert!(!same_compiled_plot(before["cartesian"], after["cartesian"]));
    assert!(!same_compiled_plot(before["polar"], after["polar"]));
    assert!(same_compiled_plot(before["geo"], after["geo"]));
}

#[tokio::test]
async fn project_compile_shared_catalog_edit_invalidates_only_data_dependents() {
    let project = TempProject::copy_fixture();
    let compiler = Compiler::builder()
        .project_root(&project.0)
        .build()
        .unwrap();
    let before = compiler.compile_module(&project.0).await.unwrap();
    let catalog = project.0.join("shared.data.avenger");
    fs::write(
        &catalog,
        fs::read_to_string(&catalog)
            .unwrap()
            .replace("y: 2.0", "y: 2.25"),
    )
    .unwrap();
    let after = compiler.compile_module(&project.0).await.unwrap();
    let before = named_artifacts(&before);
    let after = named_artifacts(&after);

    assert!(!same_compiled_plot(before["cartesian"], after["cartesian"]));
    assert!(!same_compiled_plot(before["polar"], after["polar"]));
    assert!(same_compiled_plot(before["geo"], after["geo"]));
}

#[tokio::test]
async fn project_compile_reuses_dataset_schema_across_unrelated_chart_edit() {
    let project = TempProject::copy_fixture();
    let compiler = Compiler::builder()
        .project_root(&project.0)
        .build()
        .unwrap();
    let before = compiler.analyze_module(&project.0).await.unwrap();
    assert_eq!(
        before.dependency_fingerprints.datasets.len(),
        before.datasets.iter().count(),
        "every catalog and chart pipeline stage needs its own cache identity"
    );
    assert!(
        before
            .datasets
            .iter()
            .all(|(stage, _)| before.dependency_fingerprints.datasets.contains_key(stage))
    );
    let catalog_schema = before
        .datasets
        .iter()
        .find_map(|(_, dataset)| {
            (dataset.qualified_name.as_deref() == Some("shared.points"))
                .then(|| Arc::clone(&dataset.schema))
        })
        .unwrap();
    let geo = project.0.join("geo.avenger");
    fs::write(
        &geo,
        fs::read_to_string(&geo)
            .unwrap()
            .replace("value: 80.0", "value: 88.0"),
    )
    .unwrap();
    let after = compiler.analyze_module(&project.0).await.unwrap();
    let reused = after
        .datasets
        .iter()
        .find_map(|(_, dataset)| {
            (dataset.qualified_name.as_deref() == Some("shared.points"))
                .then(|| Arc::clone(&dataset.schema))
        })
        .unwrap();
    assert!(Arc::ptr_eq(&catalog_schema, &reused));
    assert!(compiler.cache_snapshot().dataset_analyses >= 1);
}

#[tokio::test]
async fn project_compile_parallel_and_sequential_artifacts_and_diagnostics_match() {
    let root = fixture();
    let sequential = Compiler::builder()
        .project_root(&root)
        .project_compilation_mode(ModuleCompilationMode::Sequential)
        .build()
        .unwrap()
        .compile_module(&root)
        .await
        .unwrap();
    let parallel = Compiler::builder()
        .project_root(&root)
        .project_compilation_mode(ModuleCompilationMode::Parallel)
        .build()
        .unwrap()
        .compile_module(&root)
        .await
        .unwrap();
    assert_eq!(sequential.module_fingerprint, parallel.module_fingerprint);
    assert_eq!(
        sequential.dependency_fingerprints,
        parallel.dependency_fingerprints
    );
    for (id, sequential) in &sequential.charts {
        let parallel = &parallel.charts[id];
        assert_eq!(sequential.interface, parallel.interface);
        assert_eq!(
            serde_json::to_value(sequential.compiled_plot()).unwrap(),
            serde_json::to_value(parallel.compiled_plot()).unwrap(),
            "semantic artifact mismatch for {id:?}"
        );
    }

    let invalid = TempProject::empty();
    for name in ["a", "b"] {
        fs::write(
            invalid.0.join(format!("{name}.avenger")),
            format!(
                "avenger 1; chart cartesian as {name} {{ data: {{ table: 'missing_{name}'; }} mark symbol {{ x: 'x'; y: 'y'; }} }}"
            ),
        )
        .unwrap();
    }
    let invalid_root = invalid.0.clone();
    let diagnostics = |mode| {
        let root = invalid_root.clone();
        async move {
            Compiler::builder()
                .project_root(&root)
                .project_compilation_mode(mode)
                .build()
                .unwrap()
                .compile_module(&root)
                .await
                .unwrap_err()
                .diagnostics
                .into_iter()
                .map(|diagnostic| {
                    (
                        diagnostic.code.as_str().to_owned(),
                        diagnostic.primary.span,
                        diagnostic.primary.message,
                    )
                })
                .collect::<Vec<_>>()
        }
    };
    let sequential = diagnostics(ModuleCompilationMode::Sequential).await;
    let parallel = diagnostics(ModuleCompilationMode::Parallel).await;
    assert_eq!(sequential.len(), 2);
    assert_eq!(sequential, parallel);
}

struct FingerprintedEnvironmentFactory(String);

impl CompileEnvironmentFactory for FingerprintedEnvironmentFactory {
    fn create(
        &self,
        _request: &CompileEnvironmentRequest,
    ) -> Result<CompileEnvironment, CompileEnvironmentError> {
        Ok(CompileEnvironment::new(SessionContext::new())
            .with_dependency_fingerprint(self.0.clone()))
    }
}

#[tokio::test]
async fn project_compile_environment_snapshot_participates_in_every_chart_key() {
    let root = fixture();
    let compile = |fingerprint: &str| {
        let root = root.clone();
        let fingerprint = fingerprint.to_owned();
        async move {
            Compiler::builder()
                .project_root(&root)
                .environment_factory(Arc::new(FingerprintedEnvironmentFactory(fingerprint)))
                .build()
                .unwrap()
                .compile_module(&root)
                .await
                .unwrap()
        }
    };
    let first = compile("environment-snapshot-a").await;
    let second = compile("environment-snapshot-b").await;
    assert_ne!(first.module_fingerprint, second.module_fingerprint);
    assert_ne!(
        first.dependency_fingerprints.compile_environment,
        second.dependency_fingerprints.compile_environment
    );
    for (id, chart) in &first.charts {
        assert_ne!(
            chart.dependency_fingerprint, second.charts[id].dependency_fingerprint,
            "environment change must invalidate {id:?}"
        );
    }
}

struct BlockingTableFactory {
    entered: Arc<Notify>,
    release: Arc<Notify>,
    released: Arc<AtomicBool>,
}

#[async_trait]
impl TableFactory for BlockingTableFactory {
    async fn create(
        &self,
        _options: &serde_json::Value,
        _environment: &CompileEnvironment,
    ) -> Result<Arc<dyn TableProvider>, TableFactoryError> {
        self.entered.notify_one();
        if !self.released.load(Ordering::SeqCst) {
            self.release.notified().await;
        }
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        Ok(Arc::new(
            MemTable::try_new(schema, vec![Vec::new()])
                .map_err(|error| TableFactoryError::Message(error.to_string()))?,
        ))
    }
}

#[tokio::test]
async fn project_compile_cancellation_publishes_no_partial_analysis_or_artifacts() {
    let project = TempProject::empty();
    fs::write(
        project.0.join("catalog.data.avenger"),
        "avenger 1; table delta as rows { uri: 'memory://rows'; }",
    )
    .unwrap();
    fs::write(
        project.0.join("chart.avenger"),
        "avenger 1; chart cartesian as chart { data: { table: 'rows'; } mark symbol { x: 'x'; y: 'y'; } }",
    )
    .unwrap();
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let released = Arc::new(AtomicBool::new(false));
    let mut factories = TableFactoryRegistry::default();
    factories
        .register(
            "delta",
            Arc::new(BlockingTableFactory {
                entered: Arc::clone(&entered),
                release: Arc::clone(&release),
                released: Arc::clone(&released),
            }),
        )
        .unwrap();
    let compiler = Compiler::builder()
        .project_root(&project.0)
        .table_factories(factories)
        .build()
        .unwrap();

    {
        let compile = compiler.compile_module(&project.0);
        tokio::pin!(compile);
        tokio::select! {
            _ = entered.notified() => {}
            result = &mut compile => panic!("compile unexpectedly completed: {result:?}"),
        }
    }
    let cancelled = compiler.cache_snapshot();
    assert_eq!(cancelled.chart_artifacts, 0);
    assert_eq!(cancelled.project_analyses, 0);
    assert_eq!(cancelled.dataset_analyses, 0);

    released.store(true, Ordering::SeqCst);
    release.notify_waiters();
    let completed = compiler.compile_module(&project.0).await.unwrap();
    assert_eq!(completed.charts.len(), 1);
    assert_eq!(compiler.cache_snapshot().chart_artifacts, 1);
}

#[tokio::test]
async fn project_compile_interfaces_match_reviewed_snapshot() {
    let root = fixture();
    let project = Compiler::builder()
        .project_root(&root)
        .build()
        .unwrap()
        .compile_module(&root)
        .await
        .unwrap();
    let interfaces = project
        .charts
        .values()
        .map(|chart| (chart.name.as_deref().unwrap().to_owned(), &chart.interface))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        format!("{}\n", serde_json::to_string_pretty(&interfaces).unwrap()),
        include_str!("baselines/project/08_multi_chart_interfaces.json")
    );
}
