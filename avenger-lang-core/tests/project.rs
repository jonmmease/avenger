use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use avenger_chart_schema::NativeModuleId;
use avenger_lang_core::{
    AvailableNativeModule, ContentVersion, ImportCapabilities, InMemorySourceLoader, LoadedSource,
    ModuleDependencyTarget, ModuleGraphLoadLimits, ModuleGraphLoadRequest, ModuleGraphLoader,
    ModuleId, ModuleRoot, SourceLoader, SourceLoaderError, SourceOrigin,
};
use sha2::{Digest, Sha256};

fn source(origin: SourceOrigin, text: &str) -> LoadedSource {
    LoadedSource::new(origin, text, ContentVersion::new("fixture-v1"))
}

fn request(roots: Vec<ModuleRoot>) -> ModuleGraphLoadRequest {
    ModuleGraphLoadRequest {
        project_root: "/project".into(),
        roots,
        native_modules: BTreeMap::new(),
        capabilities: ImportCapabilities::in_memory("/project"),
        schema_version: "schema-1".into(),
        registry_version: "registry-1".into(),
        limits: ModuleGraphLoadLimits::default(),
    }
}

#[tokio::test]
async fn module_graph_loads_mixed_modules_and_preserves_import_clauses() {
    let root = SourceOrigin::Memory("charts/dashboard.avenger".into());
    let library = SourceOrigin::Memory("charts/library.avenger".into());
    let loader = CountingLoader {
        inner: InMemorySourceLoader::default()
            .with_source(source(
                root.clone(),
                "avenger 1;\
                 import { badge as point_badge, rows } from './library.avenger';\
                 import * as library from './library.avenger';\
                 table memory as local_rows {}\
                 chart cartesian as summary { mark point_badge {} }",
            ))
            .with_source(source(
                library,
                "avenger 1;\
                 export define mark badge { mark symbol {} }\
                 export table memory as rows {}\
                 chart cartesian as preview {}",
            )),
        count: AtomicUsize::new(0),
    };

    let graph = ModuleGraphLoader::new(&loader)
        .load(request(vec![ModuleRoot::requested(root)]))
        .await
        .result
        .unwrap();

    assert_eq!(graph.source_modules.len(), 2);
    assert_eq!(graph.imports.len(), 2);
    assert_eq!(loader.count.load(Ordering::SeqCst), 2);
    assert!(graph.imports.iter().any(|edge| {
        matches!(
            &edge.clause,
            avenger_lang_core::ast::ImportClause::Named(specifiers)
                if specifiers.len() == 2
                    && specifiers[0].imported.as_str() == "badge"
                    && specifiers[0].local.as_str() == "point_badge"
        )
    }));
    assert!(graph.imports.iter().all(|edge| {
        matches!(edge.imported, ModuleId::Source(_)) && edge.imported_source.is_some()
    }));
}

#[tokio::test]
async fn module_graph_loads_std_modules_with_multiple_exports() {
    let root = SourceOrigin::Memory("chart.avenger".into());
    let standard = SourceOrigin::Std("visuals.avenger".into());
    let loader = InMemorySourceLoader::default()
        .with_source(source(
            root.clone(),
            "avenger 1;\
             import { badge, normalize } from 'std:visuals.avenger';\
             chart cartesian as summary { mark badge {} transform normalize {} }",
        ))
        .with_source(source(
            standard,
            "avenger 1;\
             export define mark badge { mark symbol {} }\
             export define transform normalize {}",
        ));

    let graph = ModuleGraphLoader::new(&loader)
        .load(request(vec![ModuleRoot::requested(root)]))
        .await
        .result
        .unwrap();
    assert_eq!(graph.source_modules.len(), 2);
    assert_eq!(graph.imports.len(), 1);
}

#[tokio::test]
async fn one_remote_hash_pins_the_whole_module() {
    let root = SourceOrigin::Memory("chart.avenger".into());
    let remote = SourceOrigin::Http("https://example.test/library.avenger".into());
    let remote_text = "avenger 1;\
        export define mark badge { mark symbol {} }\
        export define transform normalize {}";
    let hash = format!("{:x}", Sha256::digest(remote_text.as_bytes()));
    let root_text = format!(
        "avenger 1;\
         import {{ badge, normalize }} from 'https://example.test/library.avenger' sha256 '{hash}';\
         chart cartesian as summary {{ mark badge {{}} transform normalize {{}} }}"
    );
    let loader = InMemorySourceLoader::default()
        .with_source(source(root.clone(), &root_text))
        .with_source(source(remote, remote_text));
    let mut load_request = request(vec![ModuleRoot::requested(root.clone())]);
    load_request.capabilities.allow_http = true;
    assert!(
        ModuleGraphLoader::new(&loader)
            .load(load_request)
            .await
            .result
            .is_ok()
    );

    loader.insert(source(
        root.clone(),
        &root_text.replace(&hash, &"0".repeat(64)),
    ));
    let mut mismatch = request(vec![ModuleRoot::requested(root)]);
    mismatch.capabilities.allow_http = true;
    let failure = ModuleGraphLoader::new(&loader)
        .load(mismatch)
        .await
        .result
        .unwrap_err();
    assert_eq!(failure.diagnostics[0].code.as_str(), "AVENGER-MODULE-009");
}

#[tokio::test]
async fn native_modules_are_registry_leaves_and_never_loaded_as_source() {
    let root = SourceOrigin::Memory("chart.avenger".into());
    let module = NativeModuleId::new("native:com.acme.visuals@1").unwrap();
    let loader = CountingLoader {
        inner: InMemorySourceLoader::default().with_source(source(
            root.clone(),
            "avenger 1;\
             import * as acme from 'native:com.acme.visuals@1';\
             chart acme.cartesian as summary { mark acme.hexbin {} }",
        )),
        count: AtomicUsize::new(0),
    };
    let mut available = request(vec![ModuleRoot::requested(root.clone())]);
    available.native_modules.insert(
        module.clone(),
        AvailableNativeModule {
            schema_profile: "schema-acme-1".into(),
            implementation_profile: "implementation-acme-1".into(),
        },
    );
    let graph = ModuleGraphLoader::new(&loader)
        .load(available)
        .await
        .result
        .unwrap();
    assert_eq!(loader.count.load(Ordering::SeqCst), 1);
    assert_eq!(graph.native_modules.len(), 1);
    assert!(matches!(
        graph.imports[0].imported,
        ModuleId::Native(ref id) if id == &module
    ));

    let failure = ModuleGraphLoader::new(&loader)
        .load(request(vec![ModuleRoot::requested(root)]))
        .await
        .result
        .unwrap_err();
    assert_eq!(loader.count.load(Ordering::SeqCst), 2);
    assert_eq!(failure.diagnostics[0].code.as_str(), "AVENGER-MODULE-016");
}

#[tokio::test]
async fn basename_segments_have_no_semantic_role() {
    let root = SourceOrigin::Memory("dashboard.avenger".into());
    let loader = InMemorySourceLoader::default().with_source(source(
        root.clone(),
        "avenger 1;\
         export table memory as rows {}\
         export define mark badge { mark symbol {} }\
         chart cartesian as summary {}",
    ));
    let graph = ModuleGraphLoader::new(&loader)
        .load(request(vec![ModuleRoot::requested(root)]))
        .await
        .result
        .unwrap();
    let module = graph.source_modules.values().next().unwrap();
    assert_eq!(module.parsed.ast.items.len(), 3);
}

#[tokio::test]
async fn source_cycles_are_reported_with_complete_import_edges() {
    let a = SourceOrigin::Memory("a.avenger".into());
    let b = SourceOrigin::Memory("b.avenger".into());
    let loader = InMemorySourceLoader::default()
        .with_source(source(
            a.clone(),
            "avenger 1;\
             import { b } from './b.avenger';\
             export define mark a { mark b {} }",
        ))
        .with_source(source(
            b,
            "avenger 1;\
             import { a } from './a.avenger';\
             export define mark b { mark a {} }",
        ));
    let failure = ModuleGraphLoader::new(&loader)
        .load(request(vec![ModuleRoot::requested(a)]))
        .await
        .result
        .unwrap_err();
    let diagnostic = &failure.diagnostics[0];
    assert_eq!(diagnostic.code.as_str(), "AVENGER-MODULE-013");
    assert_eq!(diagnostic.trace.len(), 2);
    assert!(
        diagnostic
            .trace
            .iter()
            .all(|frame| !frame.span.range.is_empty())
    );
}

#[tokio::test]
async fn redirects_capability_denials_and_failed_dependencies_retain_identity() {
    let root = SourceOrigin::Memory("chart.avenger".into());
    let requested = SourceOrigin::Http("https://example.test/redirect.avenger".into());
    let canonical = SourceOrigin::Http("https://cdn.example.test/library.avenger".into());
    let remote_text = "avenger 1; export define mark badge { mark symbol {} }";
    let hash = format!("{:x}", Sha256::digest(remote_text.as_bytes()));
    let root_text = format!(
        "avenger 1;\
         import {{ badge }} from 'https://example.test/redirect.avenger' sha256 '{hash}';\
         chart cartesian as summary {{ mark badge {{}} }}"
    );
    let loader = RedirectLoader {
        inner: InMemorySourceLoader::default()
            .with_source(source(root.clone(), &root_text))
            .with_source(source(canonical.clone(), remote_text)),
        requested: requested.clone(),
        canonical: canonical.clone(),
    };
    let mut allowed = request(vec![ModuleRoot::requested(root.clone())]);
    allowed.capabilities.allow_http = true;
    let attempt = ModuleGraphLoader::new(&loader).load(allowed).await;
    assert!(attempt.result.is_ok());
    let redirected = attempt
        .dependencies
        .iter()
        .find(|dependency| {
            dependency.requested == ModuleDependencyTarget::Source(requested.clone())
        })
        .unwrap();
    assert_eq!(redirected.canonical_origin.as_ref(), Some(&canonical));

    let denied = ModuleGraphLoader::new(&loader)
        .load(request(vec![ModuleRoot::requested(root)]))
        .await
        .result
        .unwrap_err();
    assert_eq!(denied.diagnostics[0].code.as_str(), "AVENGER-MODULE-015");
}

#[tokio::test]
async fn ambient_data_roots_are_explicit_and_reject_mixed_content() {
    let root = SourceOrigin::Memory("chart.avenger".into());
    let ambient = SourceOrigin::Memory("ambient.avenger".into());
    let loader = InMemorySourceLoader::default()
        .with_source(source(
            root.clone(),
            "avenger 1; chart cartesian as summary {}",
        ))
        .with_source(source(
            ambient.clone(),
            "avenger 1; export table memory as movies {}",
        ));
    let graph = ModuleGraphLoader::new(&loader)
        .load(request(vec![
            ModuleRoot::requested(root.clone()),
            ModuleRoot::ambient_data(ambient.clone()),
        ]))
        .await
        .result
        .unwrap();
    assert_eq!(graph.ambient_data_modules.len(), 1);
    assert_eq!(graph.ambient_catalog.len(), 1);

    loader.insert(source(
        ambient.clone(),
        "avenger 1;\
         export table memory as movies {}\
         chart cartesian as accidental {}",
    ));
    let failure = ModuleGraphLoader::new(&loader)
        .load(request(vec![
            ModuleRoot::requested(root),
            ModuleRoot::ambient_data(ambient),
        ]))
        .await
        .result
        .unwrap_err();
    assert_eq!(failure.diagnostics[0].code.as_str(), "AVENGER-MODULE-002");
}

#[tokio::test]
async fn graph_fingerprint_tracks_source_bytes_and_referenced_native_profiles() {
    let root = SourceOrigin::Memory("chart.avenger".into());
    let module = NativeModuleId::new("native:com.acme.visuals@1").unwrap();
    let loader = InMemorySourceLoader::default().with_source(source(
        root.clone(),
        "avenger 1;\
         import * as acme from 'native:com.acme.visuals@1';\
         chart acme.cartesian as summary {}",
    ));
    let mut first_request = request(vec![ModuleRoot::requested(root.clone())]);
    first_request.native_modules.insert(
        module.clone(),
        AvailableNativeModule {
            schema_profile: "schema-1".into(),
            implementation_profile: "implementation-1".into(),
        },
    );
    let first = ModuleGraphLoader::new(&loader)
        .load(first_request)
        .await
        .result
        .unwrap();

    let mut changed_request = request(vec![ModuleRoot::requested(root.clone())]);
    changed_request.native_modules.insert(
        module.clone(),
        AvailableNativeModule {
            schema_profile: "schema-1".into(),
            implementation_profile: "implementation-2".into(),
        },
    );
    let changed_profile = ModuleGraphLoader::new(&loader)
        .load(changed_request)
        .await
        .result
        .unwrap();
    assert_ne!(first.fingerprint, changed_profile.fingerprint);

    loader.insert(source(
        root.clone(),
        "avenger 1;\
         import * as acme from 'native:com.acme.visuals@1';\
         chart acme.cartesian as changed {}",
    ));
    let mut changed_source_request = request(vec![ModuleRoot::requested(root)]);
    changed_source_request.native_modules.insert(
        module,
        AvailableNativeModule {
            schema_profile: "schema-1".into(),
            implementation_profile: "implementation-1".into(),
        },
    );
    let changed_source = ModuleGraphLoader::new(&loader)
        .load(changed_source_request)
        .await
        .result
        .unwrap();
    assert_ne!(first.fingerprint, changed_source.fingerprint);
}

struct CountingLoader {
    inner: InMemorySourceLoader,
    count: AtomicUsize,
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
