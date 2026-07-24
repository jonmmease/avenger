use std::sync::{Arc, Mutex};

use avenger_lang_compiler::{
    CompileEnvironment, CompileEnvironmentError, CompileEnvironmentFactory,
    CompileEnvironmentRequest, Compiler,
};
use avenger_lang_core::{
    ContentVersion, InMemorySourceLoader, LoadedSource, ModuleRoot, SourceOrigin,
};
use datafusion::prelude::SessionContext;

#[derive(Default)]
struct RecordingEnvironmentFactory {
    generations: Mutex<Vec<u64>>,
}

impl CompileEnvironmentFactory for RecordingEnvironmentFactory {
    fn create(
        &self,
        request: &CompileEnvironmentRequest,
    ) -> Result<CompileEnvironment, CompileEnvironmentError> {
        self.generations.lock().unwrap().push(request.generation);
        Ok(CompileEnvironment::new(SessionContext::new()))
    }
}

fn source(origin: &SourceOrigin, mark_name: &str, version: &str) -> LoadedSource {
    LoadedSource::new(
        origin.clone(),
        format!("avenger 1; chart cartesian as chart {{ mark symbol as {mark_name} {{}} }}"),
        ContentVersion::new(version),
    )
}

#[tokio::test]
async fn explicit_roots_use_snapshot_loaders_generations_and_shared_caches() {
    let project = tempfile::tempdir().unwrap();
    let project_root = std::fs::canonicalize(project.path()).unwrap();
    let origin = SourceOrigin::File(project_root.join("chart.avenger"));
    let environments = Arc::new(RecordingEnvironmentFactory::default());
    let base_loader =
        Arc::new(InMemorySourceLoader::default().with_source(source(&origin, "before", "v1")));
    let compiler = Compiler::builder()
        .project_root(&project_root)
        .source_loader(base_loader)
        .environment_factory(environments.clone())
        .build()
        .unwrap();
    let roots = vec![ModuleRoot::requested(origin.clone())];
    let before = compiler
        .analyze_project_roots(roots.clone(), 41)
        .await
        .unwrap();

    let overlay =
        Arc::new(InMemorySourceLoader::default().with_source(source(&origin, "after", "v2")));
    let fork = compiler.fork_with_source_loader(overlay);
    let after = fork.analyze_project_roots(roots, 42).await.unwrap();

    assert_ne!(before.project_fingerprint, after.project_fingerprint);
    assert_eq!(*environments.generations.lock().unwrap(), vec![41, 42]);
    assert!(compiler.cache_snapshot().resolved_projects >= 2);
    assert_eq!(compiler.cache_snapshot(), fork.cache_snapshot());
    compiler.trim_editor_caches(1, 1);
    let trimmed = compiler.cache_snapshot();
    assert!(trimmed.resolved_projects <= 1);
    assert!(trimmed.project_analyses <= 1);
    assert!(trimmed.dataset_analyses <= 1);
    assert_eq!(trimmed, fork.cache_snapshot());
}
