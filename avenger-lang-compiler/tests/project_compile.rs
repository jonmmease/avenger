use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use arrow::datatypes::{DataType, Field, Schema};
use async_trait::async_trait;
use avenger_common::canvas::CanvasDimensions;
use avenger_lang_compiler::{
    CompileEnvironment, CompileEnvironmentError, CompileEnvironmentFactory,
    CompileEnvironmentRequest, Compiler, ModuleCompilationMode, TableFactory, TableFactoryError,
    TableFactoryRegistry,
};
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use datafusion::datasource::{TableProvider, memory::MemTable};
use datafusion::prelude::SessionContext;
use tokio::sync::Notify;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/modules/multi_chart")
}

fn module_path(root: &Path) -> PathBuf {
    root.join("charts.avenger")
}

struct TempProject(PathBuf);

static TEMP_PROJECT_ID: AtomicU64 = AtomicU64::new(0);

impl TempProject {
    fn copy_fixture() -> Self {
        let path = unique_temp_path("module");
        copy_tree(&fixture(), &path);
        Self(path)
    }

    fn empty() -> Self {
        let path = unique_temp_path("empty-module");
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

fn unique_temp_path(label: &str) -> PathBuf {
    let id = TEMP_PROJECT_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("avenger-lang-{label}-{}-{id}", std::process::id()))
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
    let cold = compiler.compile_module(module_path(&root)).await.unwrap();
    let warm = compiler.compile_module(module_path(&root)).await.unwrap();

    assert_eq!(
        cold.charts
            .values()
            .map(|chart| chart.name.as_deref().unwrap())
            .collect::<Vec<_>>(),
        ["cartesian", "geo", "polar"]
    );
    assert_eq!(cold.module_fingerprint, warm.module_fingerprint);
    assert_eq!(cold.dependency_fingerprints, warm.dependency_fingerprints);
    let analysis = compiler.analyze_module(module_path(&root)).await.unwrap();
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
    assert_eq!(cache.module_analyses, 1);
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
async fn project_compile_does_not_cache_nondeterministic_param_initializer_values() {
    let project = TempProject::empty();
    let chart = module_path(&project.0);
    fs::write(
        &chart,
        "avenger 1; chart zerod as chart { param random() as nonce; }",
    )
    .unwrap();
    let compiler = Compiler::builder()
        .project_root(&project.0)
        .build()
        .unwrap();

    let first = compiler.compile_module(&chart).await.unwrap();
    let second = compiler.compile_module(&chart).await.unwrap();
    let first_chart = first.charts.values().next().unwrap();
    let second_chart = second.charts.values().next().unwrap();
    let first_value = first_chart.compiled_plot().get_default_params()["nonce"].clone();
    let second_value = second_chart.compiled_plot().get_default_params()["nonce"].clone();

    assert_ne!(first_value, second_value);
    assert!(!same_compiled_plot(first_chart, second_chart));
    assert_eq!(
        first_chart.dependency_fingerprint,
        second_chart.dependency_fingerprint
    );
    assert_eq!(first.module_fingerprint, second.module_fingerprint);
    let cache = compiler.cache_snapshot();
    assert_eq!(cache.module_analyses, 1);
    assert_eq!(cache.chart_artifacts, 1);
}

#[tokio::test]
async fn project_compile_single_chart_edit_invalidates_only_that_artifact() {
    let project = TempProject::copy_fixture();
    let compiler = Compiler::builder()
        .project_root(&project.0)
        .build()
        .unwrap();
    let before = compiler
        .compile_module(module_path(&project.0))
        .await
        .unwrap();
    let chart = module_path(&project.0);
    fs::write(
        &chart,
        fs::read_to_string(&chart)
            .unwrap()
            .replace("CAST(64.0 AS DOUBLE)", "CAST(96.0 AS DOUBLE)"),
    )
    .unwrap();
    let after = compiler
        .compile_module(module_path(&project.0))
        .await
        .unwrap();
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
    let before = compiler
        .compile_module(module_path(&project.0))
        .await
        .unwrap();
    let definition = project.0.join("transforms.avenger");
    fs::write(
        &definition,
        fs::read_to_string(&definition)
            .unwrap()
            .replace("predicate: true", "predicate: true AND true"),
    )
    .unwrap();
    let after = compiler
        .compile_module(module_path(&project.0))
        .await
        .unwrap();
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
    let before = compiler
        .compile_module(module_path(&project.0))
        .await
        .unwrap();
    let catalog = project.0.join("data.avenger");
    fs::write(
        &catalog,
        fs::read_to_string(&catalog)
            .unwrap()
            .replace("y: 2.0", "y: 2.25"),
    )
    .unwrap();
    let after = compiler
        .compile_module(module_path(&project.0))
        .await
        .unwrap();
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
    let before = compiler
        .analyze_module(module_path(&project.0))
        .await
        .unwrap();
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
    let geo = module_path(&project.0);
    fs::write(
        &geo,
        fs::read_to_string(&geo)
            .unwrap()
            .replace("CAST(80.0 AS DOUBLE)", "CAST(88.0 AS DOUBLE)"),
    )
    .unwrap();
    let after = compiler
        .analyze_module(module_path(&project.0))
        .await
        .unwrap();
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
        .module_compilation_mode(ModuleCompilationMode::Sequential)
        .build()
        .unwrap()
        .compile_module(module_path(&root))
        .await
        .unwrap();
    let parallel = Compiler::builder()
        .project_root(&root)
        .module_compilation_mode(ModuleCompilationMode::Parallel)
        .build()
        .unwrap()
        .compile_module(module_path(&root))
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
    fs::write(
        module_path(&invalid.0),
        "avenger 1;\
         chart cartesian as a { data: { table: missing_a; } mark symbol { x: encoded 'x'; y: encoded 'y'; } }\
         chart cartesian as b { data: { table: missing_b; } mark symbol { x: encoded 'x'; y: encoded 'y'; } }",
    )
    .unwrap();
    let invalid_root = invalid.0.clone();
    let diagnostics = |mode| {
        let root = invalid_root.clone();
        async move {
            Compiler::builder()
                .project_root(&root)
                .module_compilation_mode(mode)
                .build()
                .unwrap()
                .compile_module(module_path(&root))
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

#[tokio::test]
async fn private_lexical_renames_preserve_compiled_behavior_and_state_identity() {
    let project = TempProject::empty();
    fs::write(
        project.0.join("marks.avenger"),
        r#"avenger 1;
export define mark badge {
  mark symbol as glyph { x: encoded "x"; y: encoded "y"; }
}
"#,
    )
    .unwrap();
    fs::write(
        project.0.join("data.avenger"),
        r#"avenger 1;
export table inline as observations {
  values: [{ x: 1.0; y: 2.0; }];
}
"#,
    )
    .unwrap();
    let source = |mark_alias: &str, data_alias: &str, private_mark: &str, private_table: &str| {
        format!(
            r#"avenger 1;
import {{ badge as {mark_alias} }} from './marks.avenger';
import * as {data_alias} from './data.avenger';

define mark {private_mark} {{
  mark {mark_alias} as nested {{}}
}}

table sql as {private_table} {{
  sql: SELECT * FROM {data_alias}.observations;
}}

chart cartesian as chart {{
  data: {{ table: {private_table}; }}
  param 64.0 as point_size;
  mark {private_mark} as points {{}}
  mark symbol as state_probe {{
    x: encoded "x";
    y: encoded "y";
    size: direct $point_size;
    visible: false;
  }}
}}
"#
        )
    };
    let module = module_path(&project.0);
    fs::write(
        &module,
        source("local_badge", "data_pack", "private_badge", "private_rows"),
    )
    .unwrap();

    let compiler = Compiler::builder()
        .project_root(&project.0)
        .build()
        .unwrap();
    let before = compiler
        .compile_chart(&module, Some("chart"))
        .await
        .unwrap();

    fs::write(
        &module,
        source(
            "renamed_badge",
            "renamed_data",
            "renamed_private_badge",
            "renamed_private_rows",
        ),
    )
    .unwrap();
    let after = compiler
        .compile_chart(&module, Some("chart"))
        .await
        .unwrap();

    assert_eq!(before.interface, after.interface);
    assert_eq!(before.native_requirements, after.native_requirements);
    assert_eq!(render(&before).await, render(&after).await);
}

async fn render(artifact: &avenger_lang_compiler::CompiledChartArtifact) -> image::RgbaImage {
    let evaluated = artifact
        .compiled_plot()
        .evaluate(&SessionContext::new(), None)
        .await
        .unwrap();
    let dimensions = CanvasDimensions {
        size: [evaluated.scene_graph.width, evaluated.scene_graph.height],
        scale: 1.0,
    };
    let mut canvas = PngCanvas::new(dimensions, CanvasConfig::default())
        .await
        .unwrap();
    canvas.set_scene(&evaluated.scene_graph).unwrap();
    canvas.render().await.unwrap()
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
                .compile_module(module_path(&root))
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
        project.0.join("data.avenger"),
        "avenger 1; export table delta as rows { uri: 'memory://rows'; }",
    )
    .unwrap();
    fs::write(
        module_path(&project.0),
        "avenger 1;\
         import { rows } from './data.avenger';\
         chart cartesian as chart { data: { table: rows; } mark symbol { x: encoded 'x'; y: encoded 'y'; } }",
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
        let compile = compiler.compile_module(module_path(&project.0));
        tokio::pin!(compile);
        tokio::select! {
            _ = entered.notified() => {}
            result = &mut compile => panic!("compile unexpectedly completed: {result:?}"),
        }
    }
    let cancelled = compiler.cache_snapshot();
    assert_eq!(cancelled.chart_artifacts, 0);
    assert_eq!(cancelled.module_analyses, 0);
    assert_eq!(cancelled.dataset_analyses, 0);

    released.store(true, Ordering::SeqCst);
    release.notify_waiters();
    let completed = compiler
        .compile_module(module_path(&project.0))
        .await
        .unwrap();
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
        .compile_module(module_path(&root))
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
