use std::{collections::BTreeMap, sync::Arc, time::Instant};

use async_trait::async_trait;
use avenger_lang_analysis::{
    AnalysisCancellation, AnalysisGeneration, AnalysisService, CompletionOptions, DocumentSnapshot,
    PositionRequest, SourceRevision, WorkspaceSnapshot,
};
use avenger_lang_compiler::Compiler;
use avenger_lang_core::{
    ContentVersion, ImportCapabilities, InMemorySourceLoader, LoadedSource, ModuleRoot,
    SourceLoader, SourceLoaderError, SourceOrigin,
};

fn loaded(origin: &SourceOrigin, text: &str, version: &str) -> LoadedSource {
    LoadedSource::new(
        origin.clone(),
        text,
        ContentVersion::new(version.to_owned()),
    )
}

#[tokio::test]
async fn independent_roots_preserve_healthy_analysis_and_unsaved_text() {
    let directory = tempfile::tempdir().unwrap();
    let project_root = std::fs::canonicalize(directory.path()).unwrap();
    let good = SourceOrigin::File(project_root.join("good.avenger"));
    let bad = SourceOrigin::File(project_root.join("bad.avenger"));
    let disk_loader = InMemorySourceLoader::default()
        .with_source(loaded(
            &good,
            "avenger 1; chart cartesian as good { mark symbol as disk {} }",
            "disk-good",
        ))
        .with_source(loaded(&bad, "avenger 1; chart", "disk-bad"));
    let compiler = Compiler::builder()
        .project_root(&project_root)
        .source_loader(Arc::new(disk_loader))
        .build()
        .unwrap();
    let open_good = DocumentSnapshot::new(
        good.clone(),
        SourceRevision::new("open-good"),
        "avenger 1; chart cartesian as good { mark symbol as unsaved {} }",
    );
    let snapshot = WorkspaceSnapshot {
        generation: AnalysisGeneration::new(7),
        project_root,
        roots: vec![
            ModuleRoot::requested(good.clone()),
            ModuleRoot::requested(bad.clone()),
        ],
        open_documents: BTreeMap::from([(good.clone(), open_good)]),
        known_disk_sources: vec![good.clone(), bad.clone()],
        native_registry_profile: compiler
            .language_host()
            .registry()
            .profile_id()
            .as_str()
            .to_owned(),
    };
    let service = AnalysisService::new(compiler);
    let mut after_delete = snapshot.clone();
    after_delete.generation = AnalysisGeneration::new(8);
    after_delete.roots = vec![ModuleRoot::requested(good.clone())];
    after_delete.known_disk_sources = vec![good.clone()];
    let analysis = service
        .analyze_workspace(snapshot, &AnalysisCancellation::default())
        .await
        .unwrap();
    assert!(
        analysis.semantic_roots[&good.canonical_uri()]
            .result
            .is_ok(),
        "good root failed: {:?}",
        analysis.semantic_roots[&good.canonical_uri()].result
    );
    assert!(
        analysis.semantic_roots[&bad.canonical_uri()]
            .result
            .is_err()
    );
    assert_eq!(analysis.generation, AnalysisGeneration::new(7));
    assert!(analysis.syntax.contains_key(&good));
    let after_delete = service
        .analyze_workspace(after_delete, &AnalysisCancellation::default())
        .await
        .unwrap();
    assert!(
        !after_delete
            .semantic_roots
            .contains_key(&bad.canonical_uri())
    );
}

#[tokio::test]
async fn unsaved_new_imports_participate_in_the_exact_snapshot_closure() {
    let directory = tempfile::tempdir().unwrap();
    let project_root = std::fs::canonicalize(directory.path()).unwrap();
    let chart = SourceOrigin::File(project_root.join("chart.avenger"));
    let badge = SourceOrigin::File(project_root.join("badge.avenger"));
    let compiler = Compiler::builder()
        .project_root(&project_root)
        .source_loader(Arc::new(InMemorySourceLoader::default()))
        .build()
        .unwrap();
    let chart_text = "avenger 1; import { badge } from 'badge.avenger'; chart cartesian as chart { mark badge as imported {} }";
    let badge_text = "avenger 1; export define mark badge { mark symbol {} }";
    let snapshot = WorkspaceSnapshot {
        generation: AnalysisGeneration::new(9),
        project_root,
        roots: vec![ModuleRoot::requested(chart.clone())],
        open_documents: BTreeMap::from([
            (
                chart.clone(),
                DocumentSnapshot::new(
                    chart.clone(),
                    SourceRevision::from_text(chart_text),
                    chart_text,
                ),
            ),
            (
                badge.clone(),
                DocumentSnapshot::new(badge, SourceRevision::from_text(badge_text), badge_text),
            ),
        ]),
        known_disk_sources: Vec::new(),
        native_registry_profile: compiler
            .language_host()
            .registry()
            .profile_id()
            .as_str()
            .to_owned(),
    };
    let analysis = AnalysisService::new(compiler)
        .analyze_workspace(snapshot, &AnalysisCancellation::default())
        .await
        .unwrap();
    assert!(
        analysis.semantic_roots[&chart.canonical_uri()]
            .result
            .is_ok(),
        "unsaved import failed: {:?}",
        analysis.semantic_roots[&chart.canonical_uri()].result
    );
}

#[tokio::test]
async fn malformed_edit_keeps_last_good_semantic_identity_and_type() {
    let directory = tempfile::tempdir().unwrap();
    let project_root = std::fs::canonicalize(directory.path()).unwrap();
    let chart = SourceOrigin::File(project_root.join("chart.avenger"));
    let valid_text = "avenger 1; chart cartesian as chart {\n-- | Canvas width.\nparam CAST(640.0 AS DOUBLE) as width; mark symbol as points { size: encoded $width; } }";
    let invalid_text = "avenger 1; chart cartesian as chart {\n-- | Canvas width.\nparam CAST(640.0 AS DOUBLE) as width { mark symbol as points { size: encoded $width; }";
    let compiler = Compiler::builder()
        .project_root(&project_root)
        .source_loader(Arc::new(
            InMemorySourceLoader::default().with_source(loaded(&chart, valid_text, "disk")),
        ))
        .build()
        .unwrap();
    let profile = compiler
        .language_host()
        .registry()
        .profile_id()
        .as_str()
        .to_owned();
    let snapshot = |generation, text: &str| WorkspaceSnapshot {
        generation: AnalysisGeneration::new(generation),
        project_root: project_root.clone(),
        roots: vec![ModuleRoot::requested(chart.clone())],
        open_documents: BTreeMap::from([(
            chart.clone(),
            DocumentSnapshot::new(
                chart.clone(),
                SourceRevision::from_text(text),
                text.to_owned(),
            ),
        )]),
        known_disk_sources: vec![chart.clone()],
        native_registry_profile: profile.clone(),
    };
    let service = AnalysisService::new(compiler);
    let valid = service
        .analyze_workspace(snapshot(1, valid_text), &AnalysisCancellation::default())
        .await
        .unwrap();
    let invalid = service
        .analyze_workspace(snapshot(2, invalid_text), &AnalysisCancellation::default())
        .await
        .unwrap();
    assert!(
        invalid.semantic_roots[&chart.canonical_uri()]
            .result
            .is_err()
    );
    let merged = invalid.with_last_good_semantics(&valid);
    let width = merged.semantic_index.documents[&chart]
        .symbols
        .iter()
        .find(|symbol| symbol.name == "width")
        .unwrap();
    assert!(!width.identity.starts_with("syntax:"));
    let detail = width.detail.as_deref().unwrap();
    assert!(detail.starts_with("param: Float64"));
    assert!(detail.contains("Initializer: `CAST(640.0 AS DOUBLE)`"));
    assert!(detail.contains("Type source: inferred from the SQL initializer"));
    assert!(detail.contains("Nullability: typed nulls are permitted"));
    assert!(detail.contains("Sharing: `shared`"));
    let hover = merged
        .hover(
            &avenger_lang_analysis::PositionRequest {
                source: chart,
                byte_offset: invalid_text.rfind("$width").unwrap() + 2,
                source_revision: SourceRevision::from_text(invalid_text),
            },
            &AnalysisCancellation::default(),
        )
        .unwrap()
        .unwrap();
    assert!(hover.markdown.contains("param: Float64"));
    assert!(hover.markdown.contains("Canvas width."));
}

#[tokio::test]
async fn malformed_edit_keeps_last_good_dataset_stages_for_sql_completion() {
    let directory = tempfile::tempdir().unwrap();
    let project_root = std::fs::canonicalize(directory.path()).unwrap();
    let chart = SourceOrigin::File(project_root.join("chart.avenger"));
    let valid_text = r#"avenger 1;
schema tables as local {
  table inline as points {
    values: [{ x: 1.0; y: 2.0; }];
  }
}
chart cartesian as chart {
  data: { table: local.points; }
  mark symbol as points {
    x: encoded "x";
    y: encoded "y";
  }
}
"#;
    let invalid_text = valid_text.replace("x: encoded \"x\"", "x: encoded \"\"");
    let compiler = Compiler::builder()
        .project_root(&project_root)
        .source_loader(Arc::new(
            InMemorySourceLoader::default().with_source(loaded(&chart, valid_text, "disk")),
        ))
        .build()
        .unwrap();
    let profile = compiler
        .language_host()
        .registry()
        .profile_id()
        .as_str()
        .to_owned();
    let snapshot = |generation, text: &str| WorkspaceSnapshot {
        generation: AnalysisGeneration::new(generation),
        project_root: project_root.clone(),
        roots: vec![ModuleRoot::requested(chart.clone())],
        open_documents: BTreeMap::from([(
            chart.clone(),
            DocumentSnapshot::new(
                chart.clone(),
                SourceRevision::from_text(text),
                text.to_owned(),
            ),
        )]),
        known_disk_sources: vec![chart.clone()],
        native_registry_profile: profile.clone(),
    };
    let service = AnalysisService::new(compiler);
    let valid = service
        .analyze_workspace(snapshot(1, valid_text), &AnalysisCancellation::default())
        .await
        .unwrap();
    assert!(valid.semantic_roots[&chart.canonical_uri()].result.is_ok());
    let invalid = service
        .analyze_workspace(snapshot(2, &invalid_text), &AnalysisCancellation::default())
        .await
        .unwrap();
    assert!(
        invalid.semantic_roots[&chart.canonical_uri()]
            .result
            .is_err()
    );

    let merged = invalid.with_last_good_semantics(&valid);
    let cursor = invalid_text.find("x: encoded \"\"").unwrap() + "x: encoded \"".len();
    let completion = merged
        .complete(
            &PositionRequest {
                source: chart,
                byte_offset: cursor,
                source_revision: SourceRevision::from_text(&invalid_text),
            },
            CompletionOptions::default(),
            &AnalysisCancellation::default(),
        )
        .unwrap();
    let labels = completion
        .items
        .iter()
        .map(|item| item.label.as_str())
        .collect::<Vec<_>>();
    assert!(
        labels.contains(&"x"),
        "completion items: {:#?}",
        completion.items
    );
    assert!(
        labels.contains(&"y"),
        "completion items: {:#?}",
        completion.items
    );
    assert!(!completion.is_incomplete);
}

#[derive(Debug)]
struct PendingLoader;

#[async_trait]
impl SourceLoader for PendingLoader {
    async fn load(
        &self,
        _origin: &SourceOrigin,
        _capabilities: &ImportCapabilities,
    ) -> Result<LoadedSource, SourceLoaderError> {
        futures::future::pending().await
    }
}

#[tokio::test]
async fn cancellation_drops_in_flight_source_or_provider_analysis() {
    let directory = tempfile::tempdir().unwrap();
    let project_root = std::fs::canonicalize(directory.path()).unwrap();
    let root = SourceOrigin::File(project_root.join("pending.avenger"));
    let compiler = Compiler::builder()
        .project_root(&project_root)
        .source_loader(Arc::new(PendingLoader))
        .build()
        .unwrap();
    let snapshot = WorkspaceSnapshot {
        generation: AnalysisGeneration::new(8),
        project_root,
        roots: vec![ModuleRoot::requested(root.clone())],
        open_documents: BTreeMap::new(),
        known_disk_sources: vec![root],
        native_registry_profile: compiler
            .language_host()
            .registry()
            .profile_id()
            .as_str()
            .to_owned(),
    };
    let cancellation = AnalysisCancellation::default();
    let service = AnalysisService::new(compiler);
    let future = service.analyze_workspace(snapshot, &cancellation);
    tokio::pin!(future);
    tokio::select! {
        result = &mut future => panic!("analysis unexpectedly completed: {result:?}"),
        () = tokio::task::yield_now() => {}
    }
    cancellation.cancel();
    assert!(future.await.is_err());
}

#[tokio::test]
async fn registry_profile_mismatches_never_publish_analysis() {
    let directory = tempfile::tempdir().unwrap();
    let project_root = std::fs::canonicalize(directory.path()).unwrap();
    let origin = SourceOrigin::File(project_root.join("chart.avenger"));
    let text = "avenger 1; chart cartesian as chart {}";
    let compiler = Compiler::builder()
        .project_root(&project_root)
        .source_loader(Arc::new(
            InMemorySourceLoader::default().with_source(loaded(&origin, text, "v1")),
        ))
        .build()
        .unwrap();
    let service = AnalysisService::new(compiler);
    let result = service
        .analyze_workspace(
            WorkspaceSnapshot {
                generation: AnalysisGeneration::new(1),
                project_root,
                roots: vec![ModuleRoot::requested(origin.clone())],
                open_documents: BTreeMap::from([(
                    origin.clone(),
                    DocumentSnapshot::new(origin, SourceRevision::new("v1"), text),
                )]),
                known_disk_sources: Vec::new(),
                native_registry_profile: "wrong-profile".to_owned(),
            },
            &AnalysisCancellation::default(),
        )
        .await;
    assert!(result.is_err());
}

#[tokio::test]
#[ignore = "manual timing baseline; not a CI threshold"]
async fn record_project_analysis_timing_baseline() {
    let directory = tempfile::tempdir().unwrap();
    let project_root = std::fs::canonicalize(directory.path()).unwrap();
    let origin = SourceOrigin::File(project_root.join("chart.avenger"));
    let text = "avenger 1; chart cartesian as chart { mark symbol as points {} }";
    let loader = InMemorySourceLoader::default().with_source(loaded(&origin, text, "v1"));
    let compiler = Compiler::builder()
        .project_root(&project_root)
        .source_loader(Arc::new(loader))
        .build()
        .unwrap();
    let snapshot = WorkspaceSnapshot {
        generation: AnalysisGeneration::new(1),
        project_root,
        roots: vec![ModuleRoot::requested(origin.clone())],
        open_documents: BTreeMap::from([(
            origin.clone(),
            DocumentSnapshot::new(origin, SourceRevision::new("v1"), text),
        )]),
        known_disk_sources: Vec::new(),
        native_registry_profile: compiler
            .language_host()
            .registry()
            .profile_id()
            .as_str()
            .to_owned(),
    };
    let service = AnalysisService::new(compiler);
    let cancellation = AnalysisCancellation::default();
    let start = Instant::now();
    std::hint::black_box(
        service
            .analyze_workspace(snapshot.clone(), &cancellation)
            .await
            .unwrap(),
    );
    let cold = start.elapsed();
    let mut samples = Vec::with_capacity(100);
    for generation in 2..102 {
        let mut current = snapshot.clone();
        current.generation = AnalysisGeneration::new(generation);
        let start = Instant::now();
        std::hint::black_box(
            service
                .analyze_workspace(current, &cancellation)
                .await
                .unwrap(),
        );
        samples.push(start.elapsed());
    }
    samples.sort_unstable();
    eprintln!(
        "project cold={cold:?} warm_p50={:?} warm_p95={:?}",
        samples[samples.len() / 2],
        samples[samples.len() * 95 / 100]
    );
}
