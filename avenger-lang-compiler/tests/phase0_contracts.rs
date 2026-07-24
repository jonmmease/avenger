use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use arrow::datatypes::{DataType, Field, Schema};
use avenger_chart_lang_registry::{NativeRegistryBuilder, ResolvedDeclaration, builtins};
use avenger_chart_schema::{
    KindSchema, NativeKindKey, NativeKindNamespace, NativeModuleId,
    NativeModuleImplementationProfileId, NativeSchemaSnapshot,
};
use avenger_lang_compiler::{
    AnalyzedDataset, ArtifactCacheKey, CompileEnvironment, CompileEnvironmentError,
    CompileEnvironmentFactory, CompileEnvironmentRequest, Compiler, DatasetProvenance,
    DatasetSchemaIndex, DatasetStageId, DatasetStageKind, DependencyFingerprint, ModuleDatasetId,
};
use avenger_lang_core::{
    ContentVersion, InMemorySourceLoader, LoadedSource, SourceFile, SourceId, SourceLoader,
    SourceMap, SourceOrigin, SourceSpan,
};

#[test]
fn full_v1_schema_round_trips_and_matches_version_snapshot() {
    let registry = builtins::stock_registry().unwrap();
    let checked: NativeSchemaSnapshot = serde_json::from_str(include_str!(
        "../../avenger-chart-lang-registry/snapshots/full-v1-authoring-schema.json"
    ))
    .unwrap();

    assert_eq!(registry.snapshot(), &checked);
    let canonical = registry.canonical_schema_json().unwrap();
    let decoded: NativeSchemaSnapshot = serde_json::from_slice(&canonical).unwrap();
    assert_eq!(decoded.canonical_json().unwrap(), canonical);

    let version_snapshot = serde_json::json!({
        "version": checked.version,
        "profile_label": checked.profile_label,
    });
    let rendered = serde_json::to_string_pretty(&version_snapshot).unwrap() + "\n";
    if std::env::var_os("AVENGER_LANG_UPDATE_BASELINES").is_some() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/baselines/full-v1-schema-version.json");
        fs::write(&path, &rendered).unwrap();
        eprintln!("updated {}", path.display());
    } else {
        assert_eq!(
            rendered,
            include_str!("baselines/full-v1-schema-version.json")
        );
    }
}

#[test]
fn full_v1_semantic_json_schema_is_reviewed() {
    let registry = Arc::new(builtins::stock_registry().unwrap());
    let schema = avenger_lang_compiler::LanguageHost::new(registry).semantic_json_schema();
    let rendered = serde_json::to_string_pretty(schema.as_value()).unwrap() + "\n";
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/baselines/full-v1-semantic-schema.json");
    if std::env::var_os("AVENGER_LANG_UPDATE_BASELINES").is_some() {
        fs::write(&path, &rendered).unwrap();
        eprintln!("updated {}", path.display());
    } else {
        assert_eq!(rendered, fs::read_to_string(&path).unwrap());
    }
}

#[tokio::test]
async fn registry_profile_is_stable_distinct_and_propagated() {
    let left = Arc::new(builtins::stock_registry().unwrap());
    let right = builtins::stock_registry().unwrap();
    assert_eq!(left.profile_id(), right.profile_id());

    let mut builder = NativeRegistryBuilder::new(1, builtins::STOCK_V1_PROFILE_LABEL);
    builtins::register_stock_builtins(&mut builder).unwrap();
    let module_id = NativeModuleId::new("native:com.acme.host-layout@1").unwrap();
    let layout_key = NativeKindKey::new(NativeKindNamespace::Layout, "host_layout");
    let mut module = builder
        .native_module(
            module_id.clone(),
            "Host layout fixture.",
            NativeModuleImplementationProfileId::new("host-layout-rust-v1").unwrap(),
        )
        .unwrap();
    module
        .registry()
        .register_object(
            KindSchema::new(
                layout_key.clone(),
                "Host layout used to prove profile identity changes.",
            ),
            Arc::new(|_| Ok(Box::new(()))),
        )
        .unwrap();
    module
        .export("host_layout", layout_key, "Host-provided layout fixture.")
        .unwrap();
    module.finish().unwrap();
    let extended = builder.build().unwrap();
    assert_ne!(left.profile_id(), extended.profile_id());
    assert_eq!(left.builtin_profile_id(), extended.builtin_profile_id());
    assert!(extended.native_export(&module_id, "host_layout").is_ok());

    let compiler = Compiler::builder()
        .project_root("/project")
        .native_registry(left.clone())
        .build()
        .unwrap();
    assert!(Arc::ptr_eq(
        compiler.language_host().registry(),
        &compiler.options().native_registry
    ));
    assert_eq!(
        compiler.language_host().semantic_json_schema().as_value()["x-avenger-native-profile"],
        left.profile_id().as_str()
    );
    compiler
        .language_host()
        .validate_native_declaration(
            &NativeKindKey::new(NativeKindNamespace::Coordinate, "cartesian"),
            &ResolvedDeclaration::new("cartesian"),
        )
        .unwrap();
    let artifact = compiler.compile_phase0_example().await.unwrap();
    let analysis = compiler.analyze_phase0_empty();
    assert_eq!(
        &artifact.native_requirements.builtin_profile,
        left.builtin_profile_id()
    );
    assert!(artifact.native_requirements.modules.is_empty());
    assert_eq!(&analysis.native_registry_profile, left.profile_id());

    let requirements = avenger_lang_compiler::NativeRequirementSet::builtin_only(&left);
    let cache_key = ArtifactCacheKey::new(
        &requirements,
        DependencyFingerprint::new("fixture-dependencies"),
    );
    assert_eq!(cache_key.native_requirements, requirements.fingerprint());

    let extended = Arc::new(extended);
    let custom_compiler = Compiler::builder()
        .project_root("/project")
        .native_registry(extended.clone())
        .build()
        .unwrap();
    let custom_artifact = custom_compiler.compile_phase0_example().await.unwrap();
    assert_eq!(
        custom_artifact.native_requirements,
        artifact.native_requirements
    );
}

#[tokio::test]
async fn programmatic_registry_chart_uses_the_public_artifact_wrapper() {
    let compiler = Compiler::builder()
        .project_root("/project")
        .build()
        .unwrap();
    let artifact = compiler.compile_phase0_example().await.unwrap();

    assert!(matches!(
        artifact.id.selector,
        avenger_lang_core::ChartSelector::Named(ref name) if name == "phase0-example"
    ));
    assert_eq!(artifact.compiled_plot().marks().len(), 1);
    assert_eq!(
        &artifact.native_requirements.builtin_profile,
        compiler.options().native_registry.builtin_profile_id()
    );

    let context = datafusion::prelude::SessionContext::new();
    let evaluated = artifact
        .compiled_plot()
        .evaluate(&context, None)
        .await
        .unwrap();
    assert!(!evaluated.scene_graph.groups().is_empty());
}

#[tokio::test]
async fn compile_attempt_retains_discovered_dependencies_on_success() {
    let origin = SourceOrigin::File("/project/chart.avenger".into());
    let loader = Arc::new(
        InMemorySourceLoader::default().with_source(LoadedSource::new(
            origin.clone(),
            "avenger 1; chart cartesian as chart {}",
            ContentVersion::new("sha256:test"),
        )),
    );
    let compiler = Compiler::builder()
        .project_root("/project")
        .source_loader(loader as Arc<dyn SourceLoader>)
        .build()
        .unwrap();

    let attempt = compiler.compile_chart_attempt("chart.avenger", None).await;
    let artifact = attempt.result.unwrap();
    assert_eq!(artifact.name.as_deref(), Some("chart"));
    let dependencies = attempt.dependencies.iter().collect::<Vec<_>>();
    assert_eq!(dependencies.len(), 1);
    assert_eq!(dependencies[0].canonical_origin, origin);
    assert_eq!(
        dependencies[0].content_version.as_deref(),
        Some("sha256:test")
    );
}

#[derive(Default)]
struct RecordingEnvironmentFactory {
    requests: Mutex<Vec<CompileEnvironmentRequest>>,
}

impl CompileEnvironmentFactory for RecordingEnvironmentFactory {
    fn create(
        &self,
        request: &CompileEnvironmentRequest,
    ) -> Result<CompileEnvironment, CompileEnvironmentError> {
        self.requests.lock().unwrap().push(request.clone());
        Ok(CompileEnvironment::new(
            datafusion::prelude::SessionContext::new(),
        ))
    }
}

#[tokio::test]
async fn generation_compile_retains_its_environment_and_generation() {
    let origin = SourceOrigin::File("/project/chart.avenger".into());
    let loader = Arc::new(
        InMemorySourceLoader::default().with_source(LoadedSource::new(
            origin,
            "avenger 1; chart cartesian as chart {}",
            ContentVersion::new("sha256:generation"),
        )),
    );
    let factory = Arc::new(RecordingEnvironmentFactory::default());
    let compiler = Compiler::builder()
        .project_root("/project")
        .source_loader(loader as Arc<dyn SourceLoader>)
        .environment_factory(factory.clone())
        .build()
        .unwrap();

    let compiled = compiler
        .compile_chart_generation_attempt("chart.avenger", None, 42)
        .await
        .result
        .unwrap();

    assert_eq!(compiled.generation, 42);
    assert_eq!(
        factory.requests.lock().unwrap().as_slice(),
        &[CompileEnvironmentRequest {
            generation: 42,
            native_registry_profile: compiler
                .options()
                .native_registry
                .profile_id()
                .as_str()
                .to_string(),
            local_resource_versions: Vec::new(),
        }]
    );
    let context = compiled.environment.session_context_arc();
    assert!(std::ptr::eq(
        context.as_ref(),
        compiled.environment.session_context()
    ));

    let next = compiler
        .compile_chart_generation_attempt("chart.avenger", None, 43)
        .await
        .result
        .unwrap();
    assert!(!Arc::ptr_eq(
        &compiled.environment.session_context_arc(),
        &next.environment.session_context_arc()
    ));
    assert_eq!(
        factory.requests.lock().unwrap()[1].generation,
        43,
        "each reload requests a fresh generation environment"
    );
}

#[tokio::test]
async fn generation_environment_receives_immutable_local_resource_versions() {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/projects/phase9-files");
    let factory = Arc::new(RecordingEnvironmentFactory::default());
    let compiler = Compiler::builder()
        .project_root(&root)
        .environment_factory(factory.clone())
        .build()
        .unwrap();

    compiler
        .compile_chart_generation_attempt(root.join("chart.avenger"), None, 9)
        .await
        .result
        .expect("compile local-file fixture");
    let requests = factory.requests.lock().unwrap();
    let request = requests.last().expect("generation environment request");
    assert_eq!(request.generation, 9);
    assert!(request.local_resource_versions.iter().any(|version| {
        version.path.ends_with("data/rows.csv")
            && !version.recursive
            && version.content_version.starts_with("sha256:")
    }));
    assert!(request.local_resource_versions.iter().any(|version| {
        version.path.ends_with("data/parts")
            && version.recursive
            && (version.content_version.starts_with("directory-sha256:")
                || version.content_version.starts_with("glob-sha256:"))
    }));
}

#[test]
fn dataset_schema_index_keeps_physical_schema_and_source_provenance() {
    let source_id = SourceId::new(9);
    let source = SourceFile::new(
        source_id,
        SourceOrigin::Memory("data.avenger".to_string()),
        "dataset movies {}",
    );
    let mut sources = SourceMap::default();
    sources.insert(source).unwrap();

    let dataset = ModuleDatasetId::new("memory:data.avenger#movies");
    let stage = DatasetStageId::new(dataset.clone(), 0);
    let schema = Arc::new(Schema::new(vec![Field::new(
        "release_date",
        DataType::Date32,
        true,
    )]));
    let mut index = DatasetSchemaIndex::default();
    index
        .insert(AnalyzedDataset {
            id: dataset.clone(),
            stage: stage.clone(),
            provenance: DatasetProvenance {
                declaration_span: SourceSpan::new(source_id, 0, 7).unwrap(),
                stage_span: SourceSpan::new(source_id, 0, 7).unwrap(),
                stage_kind: DatasetStageKind::DatasetSource,
            },
            qualified_name: None,
            qualified_path: None,
            columns: Vec::new(),
            schema: schema.clone(),
            logical_plan_fingerprint: None,
        })
        .unwrap();

    let analyzed = index.get(&stage).unwrap();
    assert_eq!(analyzed.schema, schema);
    assert_eq!(analyzed.provenance.declaration_span.source, source_id);
    let original_provenance = analyzed.provenance.clone();
    assert_eq!(index.stages_for(&dataset).count(), 1);

    let replacement_schema = Arc::new(Schema::new(vec![Field::new(
        "replacement",
        DataType::Utf8,
        false,
    )]));
    assert!(
        index
            .insert(AnalyzedDataset {
                id: dataset,
                stage: stage.clone(),
                provenance: original_provenance,
                qualified_name: None,
                qualified_path: None,
                columns: Vec::new(),
                schema: replacement_schema,
                logical_plan_fingerprint: None,
            })
            .is_err()
    );
    assert_eq!(index.get(&stage).unwrap().schema, schema);
}
