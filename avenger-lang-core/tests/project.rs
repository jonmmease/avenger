use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use avenger_lang_core::{
    ContentVersion, ImportCapabilities, InMemorySourceLoader, LoadedSource, ProjectDependencyRole,
    ProjectLoadLimits, ProjectLoadRequest, ProjectLoader, ProjectRoot, SourceLoader,
    SourceLoaderError, SourceOrigin, render_diagnostics,
};
use sha2::{Digest, Sha256};

fn source(origin: SourceOrigin, text: &str) -> LoadedSource {
    LoadedSource::new(origin, text, ContentVersion::new("fixture-v1"))
}

fn request(roots: Vec<ProjectRoot>) -> ProjectLoadRequest {
    ProjectLoadRequest {
        project_root: "/project".into(),
        roots,
        capabilities: ImportCapabilities::in_memory("/project"),
        schema_version: "schema-1".into(),
        registry_version: "registry-1".into(),
        limits: ProjectLoadLimits::default(),
    }
}

#[tokio::test]
async fn project_loads_relative_import_closure_once() {
    let chart = SourceOrigin::Memory("charts/chart.avenger".into());
    let mark = SourceOrigin::Memory("charts/marks/badge.mark.avenger".into());
    let loader = CountingLoader {
        inner: InMemorySourceLoader::default()
            .with_source(source(
                chart.clone(),
                "avenger 1; import 'marks/badge.mark.avenger'; chart cartesian as chart {}",
            ))
            .with_source(source(
                mark,
                "avenger 1; define mark badge { mark symbol {} }",
            )),
        count: AtomicUsize::new(0),
    };
    let attempt = ProjectLoader::new(&loader)
        .load(request(vec![ProjectRoot::chart(chart)]))
        .await;
    let project = attempt.result.unwrap();
    assert_eq!(project.files.len(), 2);
    assert_eq!(project.imports.len(), 1);
    assert_eq!(loader.count.load(Ordering::SeqCst), 2);
    assert_eq!(
        project.chart_roots[0].as_str(),
        "memory:charts/chart.avenger"
    );
    assert!(project.fingerprint.starts_with("sha256:"));
}

#[tokio::test]
async fn project_fingerprint_is_independent_of_root_discovery_order() {
    let a = SourceOrigin::Memory("a.avenger".into());
    let b = SourceOrigin::Memory("b.avenger".into());
    let loader = InMemorySourceLoader::default()
        .with_source(source(a.clone(), "avenger 1; chart cartesian as a {}"))
        .with_source(source(b.clone(), "avenger 1; chart cartesian as b {}"));
    let first = ProjectLoader::new(&loader)
        .load(request(vec![
            ProjectRoot::chart(a.clone()),
            ProjectRoot::chart(b.clone()),
        ]))
        .await
        .result
        .unwrap();
    let second = ProjectLoader::new(&loader)
        .load(request(vec![ProjectRoot::chart(b), ProjectRoot::chart(a)]))
        .await
        .result
        .unwrap();
    assert_eq!(first.fingerprint, second.fingerprint);
    assert_eq!(
        first.files.keys().collect::<Vec<_>>(),
        second.files.keys().collect::<Vec<_>>()
    );

    let b_original = ProjectLoader::new(&loader)
        .load(request(vec![ProjectRoot::chart(SourceOrigin::Memory(
            "b.avenger".into(),
        ))]))
        .await
        .result
        .unwrap();
    loader.insert(source(
        SourceOrigin::Memory("b.avenger".into()),
        "avenger 1; chart cartesian as b { mark symbol {} }",
    ));
    let changed = ProjectLoader::new(&loader)
        .load(request(vec![ProjectRoot::chart(SourceOrigin::Memory(
            "b.avenger".into(),
        ))]))
        .await
        .result
        .unwrap();
    assert_ne!(b_original.fingerprint, changed.fingerprint);

    let mut registry_changed = request(vec![ProjectRoot::chart(SourceOrigin::Memory(
        "a.avenger".into(),
    ))]);
    registry_changed.registry_version = "registry-2".into();
    let registry_changed = ProjectLoader::new(&loader)
        .load(registry_changed)
        .await
        .result
        .unwrap();
    let a_original = ProjectLoader::new(&loader)
        .load(request(vec![ProjectRoot::chart(SourceOrigin::Memory(
            "a.avenger".into(),
        ))]))
        .await
        .result
        .unwrap();
    assert_ne!(a_original.fingerprint, registry_changed.fingerprint);
}

#[tokio::test]
async fn project_reports_cycles_with_import_trace() {
    let a = SourceOrigin::Memory("a.mark.avenger".into());
    let b = SourceOrigin::Memory("b.mark.avenger".into());
    let loader = InMemorySourceLoader::default()
        .with_source(source(
            a.clone(),
            "avenger 1; import 'b.mark.avenger'; define mark a { mark symbol {} }",
        ))
        .with_source(source(
            b,
            "avenger 1; import 'a.mark.avenger'; define mark b { mark symbol {} }",
        ));
    let failure = ProjectLoader::new(&loader)
        .load(request(vec![ProjectRoot {
            origin: a,
            role: ProjectDependencyRole::Import,
        }]))
        .await
        .result
        .unwrap_err();
    let diagnostic = &failure.diagnostics[0];
    assert_eq!(diagnostic.code.as_str(), "AVENGER-PROJECT-013");
    assert!(diagnostic.trace.len() >= 2);
    assert!(
        diagnostic
            .trace
            .iter()
            .all(|frame| !frame.span.range.is_empty())
    );
    assert_eq!(
        render_diagnostics(&failure.diagnostics, &failure.sources),
        include_str!("baselines/project/import-cycle.txt")
    );
}

#[tokio::test]
async fn project_enforces_import_matrix_and_duplicate_bindings() {
    let chart = SourceOrigin::Memory("chart.avenger".into());
    let other_chart = SourceOrigin::Memory("other.avenger".into());
    let loader = InMemorySourceLoader::default()
        .with_source(source(
            chart.clone(),
            "avenger 1; import 'other.avenger'; chart cartesian as chart {}",
        ))
        .with_source(source(
            other_chart,
            "avenger 1; chart cartesian as other {}",
        ));
    let failure = ProjectLoader::new(&loader)
        .load(request(vec![ProjectRoot::chart(chart)]))
        .await
        .result
        .unwrap_err();
    assert_eq!(failure.diagnostics[0].code.as_str(), "AVENGER-PROJECT-012");

    let chart = SourceOrigin::Memory("duplicate.avenger".into());
    let a = SourceOrigin::Memory("a.mark.avenger".into());
    let b = SourceOrigin::Memory("b.mark.avenger".into());
    let loader = InMemorySourceLoader::default()
        .with_source(source(
            chart.clone(),
            "avenger 1; import 'a.mark.avenger' as same; import 'b.mark.avenger' as same; chart cartesian as duplicate {}",
        ))
        .with_source(source(a, "avenger 1; define mark a { mark symbol {} }"))
        .with_source(source(b, "avenger 1; define mark b { mark symbol {} }"));
    let failure = ProjectLoader::new(&loader)
        .load(request(vec![ProjectRoot::chart(chart)]))
        .await
        .result
        .unwrap_err();
    assert_eq!(failure.diagnostics[0].code.as_str(), "AVENGER-PROJECT-011");
}

#[tokio::test]
async fn project_data_pack_binds_single_root_and_rejects_collisions() {
    let chart = SourceOrigin::Memory("chart.avenger".into());
    let pack = SourceOrigin::Memory("pack.data.avenger".into());
    let loader = InMemorySourceLoader::default()
        .with_source(source(
            chart.clone(),
            "avenger 1; import 'pack.data.avenger'; chart cartesian as chart {}",
        ))
        .with_source(source(
            pack.clone(),
            "avenger 1; schema tables as vega { table csv as rows { path: 'rows.csv'; } }",
        ));
    let project = ProjectLoader::new(&loader)
        .load(request(vec![ProjectRoot::chart(chart.clone())]))
        .await
        .result
        .unwrap();
    assert_eq!(project.imports[0].binding, "vega");

    loader.insert(source(
        pack.clone(),
        "avenger 1; schema tables as vega {} schema tables as other {}",
    ));
    let failure = ProjectLoader::new(&loader)
        .load(request(vec![ProjectRoot::chart(chart.clone())]))
        .await
        .result
        .unwrap_err();
    assert_eq!(failure.diagnostics[0].code.as_str(), "AVENGER-PROJECT-020");

    loader.insert(source(pack.clone(), "avenger 1; schema tables as vega {}"));
    let ambient = SourceOrigin::Memory("ambient.data.avenger".into());
    loader.insert(source(
        ambient.clone(),
        "avenger 1; schema tables as vega {}",
    ));
    let failure = ProjectLoader::new(&loader)
        .load(request(vec![
            ProjectRoot::chart(chart),
            ProjectRoot::data(ambient),
        ]))
        .await
        .result
        .unwrap_err();
    assert_eq!(failure.diagnostics[0].code.as_str(), "AVENGER-PROJECT-014");
}

#[tokio::test]
async fn project_pinned_http_imports_succeed_and_reject_mismatch_or_denial() {
    let chart = SourceOrigin::Memory("chart.avenger".into());
    let remote = SourceOrigin::Http("https://example.test/badge.mark.avenger".into());
    let remote_text = "avenger 1; define mark badge { mark symbol {} }";
    let hash = format!("{:x}", Sha256::digest(remote_text.as_bytes()));
    let chart_text = format!(
        "avenger 1; import 'https://example.test/badge.mark.avenger' sha256 '{hash}'; chart cartesian as chart {{}}"
    );
    let loader = InMemorySourceLoader::default()
        .with_source(source(chart.clone(), &chart_text))
        .with_source(source(remote.clone(), remote_text));
    let mut allowed = request(vec![ProjectRoot::chart(chart.clone())]);
    allowed.capabilities.allow_http = true;
    assert!(
        ProjectLoader::new(&loader)
            .load(allowed)
            .await
            .result
            .is_ok()
    );

    let bad_text = chart_text.replace(&hash, &"0".repeat(64));
    loader.insert(source(chart.clone(), &bad_text));
    let mut mismatch = request(vec![ProjectRoot::chart(chart.clone())]);
    mismatch.capabilities.allow_http = true;
    let failure = ProjectLoader::new(&loader)
        .load(mismatch)
        .await
        .result
        .unwrap_err();
    assert_eq!(failure.diagnostics[0].code.as_str(), "AVENGER-PROJECT-009");

    loader.insert(source(chart.clone(), &chart_text));
    let denied = ProjectLoader::new(&loader)
        .load(request(vec![ProjectRoot::chart(chart)]))
        .await
        .result
        .unwrap_err();
    assert_eq!(denied.diagnostics[0].code.as_str(), "AVENGER-PROJECT-015");
}

#[tokio::test]
async fn project_records_http_redirect_candidate_and_canonical_origin() {
    let chart = SourceOrigin::Memory("chart.avenger".into());
    let requested = SourceOrigin::Http("https://example.test/redirect.mark.avenger".into());
    let canonical = SourceOrigin::Http("https://cdn.example.test/badge.mark.avenger".into());
    let remote_text = "avenger 1; define mark badge { mark symbol {} }";
    let hash = format!("{:x}", Sha256::digest(remote_text.as_bytes()));
    let chart_text = format!(
        "avenger 1; import 'https://example.test/redirect.mark.avenger' sha256 '{hash}' as badge; chart cartesian as chart {{}}"
    );
    let loader = RedirectLoader {
        inner: InMemorySourceLoader::default()
            .with_source(source(chart.clone(), &chart_text))
            .with_source(source(canonical.clone(), remote_text)),
        requested: requested.clone(),
        canonical: canonical.clone(),
    };
    let mut load_request = request(vec![ProjectRoot::chart(chart)]);
    load_request.capabilities.allow_http = true;
    let attempt = ProjectLoader::new(&loader).load(load_request).await;
    assert!(attempt.result.is_ok());
    let redirected = attempt
        .dependencies
        .iter()
        .find(|dependency| dependency.requested_origin == requested)
        .unwrap();
    assert_eq!(redirected.canonical_origin.as_ref(), Some(&canonical));
}

#[tokio::test]
async fn project_failed_attempt_keeps_prefix_and_repairs_through_the_same_api() {
    let chart = SourceOrigin::Memory("chart.avenger".into());
    let missing = SourceOrigin::Memory("missing.mark.avenger".into());
    let loader = InMemorySourceLoader::default().with_source(source(
        chart.clone(),
        "avenger 1; import 'missing.mark.avenger'; chart cartesian as chart {}",
    ));
    let attempt = ProjectLoader::new(&loader)
        .load(request(vec![ProjectRoot::chart(chart.clone())]))
        .await;
    assert!(attempt.result.is_err());
    assert_eq!(attempt.dependencies.len(), 2);
    assert_eq!(attempt.dependencies[1].requested_origin, missing.clone());
    assert!(attempt.dependencies[1].content_version.is_none());
    let failure = attempt.result.unwrap_err();
    assert_eq!(
        render_diagnostics(&failure.diagnostics, &failure.sources),
        include_str!("baselines/project/missing-import.txt")
    );

    loader.insert(source(
        missing,
        "avenger 1; define mark missing { mark symbol {} }",
    ));
    assert!(
        ProjectLoader::new(&loader)
            .load(request(vec![ProjectRoot::chart(chart)]))
            .await
            .result
            .is_ok()
    );
}

#[tokio::test]
async fn project_enforces_source_and_import_closure_limits() {
    let chart = SourceOrigin::Memory("chart.avenger".into());
    let a = SourceOrigin::Memory("a.mark.avenger".into());
    let b = SourceOrigin::Memory("b.mark.avenger".into());
    let loader = InMemorySourceLoader::default()
        .with_source(source(
            chart.clone(),
            "avenger 1; import 'a.mark.avenger'; chart cartesian as chart {}",
        ))
        .with_source(source(
            a.clone(),
            "avenger 1; import 'b.mark.avenger'; define mark a { mark symbol {} }",
        ))
        .with_source(source(
            b.clone(),
            "avenger 1; define mark b { mark symbol {} }",
        ));

    let mut limited = request(vec![ProjectRoot::chart(chart.clone())]);
    limited.limits.max_import_depth = 1;
    let failure = ProjectLoader::new(&loader)
        .load(limited)
        .await
        .result
        .unwrap_err();
    assert_eq!(failure.diagnostics[0].code.as_str(), "AVENGER-PROJECT-021");

    let mut limited = request(vec![ProjectRoot::chart(chart.clone())]);
    limited.limits.max_source_bytes = 16;
    let failure = ProjectLoader::new(&loader)
        .load(limited)
        .await
        .result
        .unwrap_err();
    assert_eq!(failure.diagnostics[0].code.as_str(), "AVENGER-PROJECT-022");

    let mut limited = request(vec![ProjectRoot::chart(chart.clone())]);
    limited.limits.max_sources = 1;
    let failure = ProjectLoader::new(&loader)
        .load(limited)
        .await
        .result
        .unwrap_err();
    assert_eq!(failure.diagnostics[0].code.as_str(), "AVENGER-PROJECT-023");

    let root_bytes = loader
        .load(&chart, &ImportCapabilities::in_memory("/project"))
        .await
        .unwrap()
        .text
        .len();
    let mut limited = request(vec![ProjectRoot::chart(chart.clone())]);
    limited.limits.max_total_source_bytes = root_bytes;
    let failure = ProjectLoader::new(&loader)
        .load(limited)
        .await
        .result
        .unwrap_err();
    assert_eq!(failure.diagnostics[0].code.as_str(), "AVENGER-PROJECT-024");

    let second = SourceOrigin::Memory("second.mark.avenger".into());
    loader.insert(source(
        second,
        "avenger 1; define mark second { mark symbol {} }",
    ));
    loader.insert(source(
        chart.clone(),
        "avenger 1; import 'a.mark.avenger'; import 'second.mark.avenger'; chart cartesian as chart {}",
    ));
    let mut limited = request(vec![ProjectRoot::chart(chart)]);
    limited.limits.max_imports_per_source = 1;
    let failure = ProjectLoader::new(&loader)
        .load(limited)
        .await
        .result
        .unwrap_err();
    assert_eq!(failure.diagnostics[0].code.as_str(), "AVENGER-PROJECT-025");
}

struct CountingLoader {
    inner: InMemorySourceLoader,
    count: AtomicUsize,
}

struct RedirectLoader {
    inner: InMemorySourceLoader,
    requested: SourceOrigin,
    canonical: SourceOrigin,
}

#[async_trait]
impl SourceLoader for RedirectLoader {
    async fn load(
        &self,
        origin: &SourceOrigin,
        capabilities: &ImportCapabilities,
    ) -> Result<LoadedSource, SourceLoaderError> {
        if origin == &self.requested {
            self.inner.load(&self.canonical, capabilities).await
        } else {
            self.inner.load(origin, capabilities).await
        }
    }
}

#[async_trait]
impl SourceLoader for CountingLoader {
    async fn load(
        &self,
        origin: &SourceOrigin,
        capabilities: &ImportCapabilities,
    ) -> Result<LoadedSource, SourceLoaderError> {
        self.count.fetch_add(1, Ordering::SeqCst);
        self.inner.load(origin, capabilities).await
    }
}
