use std::{fs, path::PathBuf, sync::Arc};

use arrow::datatypes::{DataType, Field, Schema};
use avenger_chart_lang_registry::{NativeRegistryBuilder, ResolvedDeclaration, builtins};
use avenger_chart_schema::{KindSchema, NativeKindKey, NativeKindNamespace, NativeSchemaSnapshot};
use avenger_lang_compiler::{
    AnalyzedDataset, ArtifactCacheKey, Compiler, DatasetProvenance, DatasetSchemaIndex,
    DatasetStageId, DatasetStageKind, DependencyFingerprint, ProjectDatasetId,
};
use avenger_lang_core::{
    ContentVersion, InMemorySourceLoader, LoadedSource, SourceFile, SourceId, SourceLoader,
    SourceMap, SourceOrigin, SourceSpan,
};

#[test]
fn bootstrap_schema_round_trips_and_matches_version_snapshot() {
    let registry = builtins::bootstrap_registry().unwrap();
    let checked: NativeSchemaSnapshot = serde_json::from_str(include_str!(
        "../../avenger-chart-lang-registry/snapshots/bootstrap-schema.json"
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
            .join("tests/baselines/bootstrap-schema-version.json");
        fs::write(&path, &rendered).unwrap();
        eprintln!("updated {}", path.display());
    } else {
        assert_eq!(
            rendered,
            include_str!("baselines/bootstrap-schema-version.json")
        );
    }
}

#[tokio::test]
async fn registry_profile_is_stable_distinct_and_propagated() {
    let left = Arc::new(builtins::bootstrap_registry().unwrap());
    let right = builtins::bootstrap_registry().unwrap();
    assert_eq!(left.profile_id(), right.profile_id());

    let mut builder = NativeRegistryBuilder::new(1, builtins::BOOTSTRAP_PROFILE_LABEL);
    builtins::register_bootstrap_builtins(&mut builder).unwrap();
    builder
        .register_object(
            KindSchema::new(
                NativeKindKey::new(NativeKindNamespace::Layout, "host_layout"),
                "Host layout used to prove profile identity changes.",
            ),
            Arc::new(|_| Ok(Box::new(()))),
        )
        .unwrap();
    let extended = builder.build().unwrap();
    assert_ne!(left.profile_id(), extended.profile_id());

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
    assert_eq!(&artifact.native_registry_profile, left.profile_id());
    assert_eq!(&analysis.native_registry_profile, left.profile_id());

    let cache_key = ArtifactCacheKey::new(
        left.profile_id(),
        DependencyFingerprint::new("fixture-dependencies"),
    );
    assert_eq!(
        cache_key.native_registry_profile,
        left.profile_id().as_str()
    );

    let extended = Arc::new(extended);
    let custom_compiler = Compiler::builder()
        .project_root("/project")
        .native_registry(extended.clone())
        .build()
        .unwrap();
    let custom_artifact = custom_compiler.compile_phase0_example().await.unwrap();
    assert_eq!(
        &custom_artifact.native_registry_profile,
        extended.profile_id()
    );
}

#[tokio::test]
async fn programmatic_registry_chart_uses_the_public_artifact_wrapper() {
    let compiler = Compiler::builder()
        .project_root("/project")
        .build()
        .unwrap();
    let artifact = compiler.compile_phase0_example().await.unwrap();

    assert_eq!(artifact.id.as_str(), "phase0-example");
    assert_eq!(artifact.compiled_plot().marks().len(), 1);
    assert_eq!(
        &artifact.native_registry_profile,
        compiler.options().native_registry.profile_id()
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
async fn compile_attempt_retains_discovered_dependencies_on_failure() {
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

    let attempt = compiler.compile_file_attempt("chart.avenger").await;
    assert_eq!(
        attempt.result.unwrap_err().diagnostics[0].code.as_str(),
        "AV0005"
    );
    let dependencies = attempt.dependencies.iter().collect::<Vec<_>>();
    assert_eq!(dependencies.len(), 1);
    assert_eq!(dependencies[0].canonical_origin, origin);
    assert_eq!(
        dependencies[0].content_version.as_deref(),
        Some("sha256:test")
    );
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

    let dataset = ProjectDatasetId::new("memory:data.avenger#movies");
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
            schema: schema.clone(),
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
                schema: replacement_schema,
            })
            .is_err()
    );
    assert_eq!(index.get(&stage).unwrap().schema, schema);
}
