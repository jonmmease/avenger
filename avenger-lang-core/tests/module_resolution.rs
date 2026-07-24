use std::collections::BTreeMap;

use avenger_chart_schema::{
    NativeKindKey, NativeModuleId, NativeModuleSchema, NativeSchemaSnapshot,
};
use avenger_lang_core::{
    AvailableNativeModule, BindingCategory, ChartSelector, ContentVersion, ImportCapabilities,
    InMemorySourceLoader, LoadedSource, ModuleGraphLoadLimits, ModuleGraphLoadRequest,
    ModuleGraphLoader, ModuleRoot, ResolvedKindBinding, SourceOrigin, resolve_project,
};

fn bootstrap_schema() -> NativeSchemaSnapshot {
    serde_json::from_str(include_str!(
        "../../avenger-chart-lang-registry/snapshots/bootstrap-schema.json"
    ))
    .unwrap()
}

async fn load(
    sources: &[(&str, &str)],
    root: &str,
    native_modules: BTreeMap<NativeModuleId, AvailableNativeModule>,
) -> avenger_lang_core::ParsedModuleGraph {
    let loader = InMemorySourceLoader::default();
    for (name, text) in sources {
        loader.insert(LoadedSource::new(
            SourceOrigin::Memory((*name).to_owned()),
            *text,
            ContentVersion::new("fixture-v1"),
        ));
    }
    ModuleGraphLoader::new(&loader)
        .load(ModuleGraphLoadRequest {
            project_root: "/project".into(),
            roots: vec![ModuleRoot::requested(SourceOrigin::Memory(root.to_owned()))],
            native_modules,
            capabilities: ImportCapabilities::in_memory("/project"),
            schema_version: "semantic-v1".to_owned(),
            registry_version: "bootstrap".to_owned(),
            limits: ModuleGraphLoadLimits::default(),
        })
        .await
        .result
        .unwrap()
}

#[tokio::test]
async fn local_and_imported_definitions_resolve_by_item_and_category() {
    let graph = load(
        &[
            (
                "root.avenger",
                "avenger 1;\
                 import { badge as imported_badge } from './library.avenger';\
                 define mark local_badge { mark symbol {} }\
                 define transform normalize {}\
                 chart cartesian as summary {\
                   transform normalize {}\
                   mark local_badge {}\
                   mark imported_badge {}\
                 }",
            ),
            (
                "library.avenger",
                "avenger 1;\
                 export define mark badge { mark symbol {} }\
                 define mark private_badge { mark symbol {} }",
            ),
        ],
        "root.avenger",
        BTreeMap::new(),
    )
    .await;
    let resolved = resolve_project(&graph, &bootstrap_schema()).result.unwrap();

    assert_eq!(resolved.definitions.len(), 4);
    let root = resolved
        .files
        .values()
        .find(|module| module.roots.iter().any(|item| item.keyword == "chart"))
        .unwrap();
    let chart = root
        .roots
        .iter()
        .find(|item| item.keyword == "chart")
        .unwrap();
    assert!(matches!(
        chart.children[0].kind_binding,
        Some(ResolvedKindBinding::Definition(_))
    ));
    assert!(matches!(
        chart.children[1].kind_binding,
        Some(ResolvedKindBinding::Definition(_))
    ));
    assert!(matches!(
        chart.children[2].kind_binding,
        Some(ResolvedKindBinding::Definition(_))
    ));
    assert_eq!(resolved.module_items.len(), 5);
}

#[tokio::test]
async fn private_exports_and_wrong_names_fail_at_the_import() {
    let graph = load(
        &[
            (
                "root.avenger",
                "avenger 1;\
                 import { private_badge, missing } from './library.avenger';\
                 chart cartesian as summary {}",
            ),
            (
                "library.avenger",
                "avenger 1;\
                 export define mark badge { mark symbol {} }\
                 define mark private_badge { mark symbol {} }",
            ),
        ],
        "root.avenger",
        BTreeMap::new(),
    )
    .await;
    let failure = resolve_project(&graph, &bootstrap_schema())
        .result
        .unwrap_err();
    assert_eq!(
        failure
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-057")
            .count(),
        2
    );
}

#[tokio::test]
async fn namespace_imports_bind_native_exports_but_unimported_modules_are_invisible() {
    let module_id = NativeModuleId::new("native:com.acme.visuals@1").unwrap();
    let mut schema = bootstrap_schema();
    let mut module = NativeModuleSchema::new(module_id.clone(), "Acme visual kinds.");
    module
        .add_export(
            "dot",
            NativeKindKey::mark("cartesian", "symbol"),
            "A public symbol-mark spelling.",
        )
        .unwrap();
    schema.modules.insert(module_id.clone(), module);
    let available = BTreeMap::from([(
        module_id,
        AvailableNativeModule {
            schema_profile: "schema-acme-1".into(),
            implementation_profile: "implementation-acme-1".into(),
        },
    )]);

    let imported = load(
        &[(
            "root.avenger",
            "avenger 1;\
             import * as acme from 'native:com.acme.visuals@1';\
             chart cartesian as summary { mark acme.dot {} }",
        )],
        "root.avenger",
        available,
    )
    .await;
    let resolved = resolve_project(&imported, &schema).result.unwrap();
    let mark = &resolved
        .files
        .values()
        .next()
        .unwrap()
        .roots
        .iter()
        .find(|item| item.keyword == "chart")
        .unwrap()
        .children[0];
    assert!(matches!(
        mark.kind_binding,
        Some(ResolvedKindBinding::Native { .. })
    ));

    let unimported = load(
        &[(
            "root.avenger",
            "avenger 1; chart cartesian as summary { mark dot {} }",
        )],
        "root.avenger",
        BTreeMap::new(),
    )
    .await;
    let failure = resolve_project(&unimported, &schema).result.unwrap_err();
    assert!(
        failure
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-020")
    );
}

#[tokio::test]
async fn chart_naming_and_export_collision_rules_are_eager() {
    let unnamed = load(
        &[(
            "root.avenger",
            "avenger 1; chart cartesian {} chart polar as second {}",
        )],
        "root.avenger",
        BTreeMap::new(),
    )
    .await;
    let failure = resolve_project(&unnamed, &bootstrap_schema())
        .result
        .unwrap_err();
    assert!(
        failure
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-051")
    );

    let duplicate_export = load(
        &[(
            "root.avenger",
            "avenger 1;\
             export define mark same { mark symbol {} }\
             export table memory as same {}",
        )],
        "root.avenger",
        BTreeMap::new(),
    )
    .await;
    let failure = resolve_project(&duplicate_export, &bootstrap_schema())
        .result
        .unwrap_err();
    assert!(
        failure
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-055")
    );
}

#[tokio::test]
async fn chart_entrypoints_own_state_and_private_siblings_do_not_change_item_identity() {
    let first = load(
        &[(
            "root.avenger",
            "avenger 1;\
             define mark badge { mark symbol {} }\
             chart cartesian as first { param int64 as value { value: 1; } }\
             chart cartesian as second { param int64 as value { value: 2; } }",
        )],
        "root.avenger",
        BTreeMap::new(),
    )
    .await;
    let first_resolved = resolve_project(&first, &bootstrap_schema()).result.unwrap();
    assert_eq!(first_resolved.entrypoints.len(), 2);
    assert!(first_resolved.entrypoints.iter().all(|(id, chart)| {
        matches!(id.selector, ChartSelector::Named(_))
            && chart.params.len() == 1
            && chart.stores.is_empty()
    }));
    let badge_id = first_resolved
        .module_items
        .values()
        .find(|item| {
            item.category
                == BindingCategory::NativeKind(avenger_chart_schema::NativeKindNamespace::Mark)
                && item.source_name.as_deref() == Some("badge")
        })
        .unwrap()
        .id
        .clone();

    let second = load(
        &[(
            "root.avenger",
            "avenger 1;\
             table memory as unrelated {}\
             define mark badge { mark symbol {} }\
             chart cartesian as first { param int64 as value { value: 1; }\
             }",
        )],
        "root.avenger",
        BTreeMap::new(),
    )
    .await;
    let second_resolved = resolve_project(&second, &bootstrap_schema())
        .result
        .unwrap();
    let second_badge_id = second_resolved
        .module_items
        .values()
        .find(|item| item.source_name.as_deref() == Some("badge"))
        .unwrap()
        .id
        .clone();
    assert_eq!(badge_id, second_badge_id);
}

#[tokio::test]
async fn definition_dependency_cycles_report_the_complete_item_path() {
    let graph = load(
        &[(
            "root.avenger",
            "avenger 1;\
             define mark alpha { mark beta {} }\
             define mark beta { mark alpha {} }\
             chart cartesian { mark alpha {} }",
        )],
        "root.avenger",
        BTreeMap::new(),
    )
    .await;

    let failure = resolve_project(&graph, &bootstrap_schema())
        .result
        .unwrap_err();
    let diagnostic = failure
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-059")
        .expect("item cycle diagnostic");
    assert!(diagnostic.message.contains("dependency cycle"));
    assert!(diagnostic.primary.message.contains("alpha"));
    assert!(diagnostic.primary.message.contains("beta"));
}

#[tokio::test]
async fn imported_bindings_are_checked_in_the_expected_category() {
    let graph = load(
        &[
            (
                "root.avenger",
                "avenger 1;\
                 import { badge } from './library.avenger';\
                 chart cartesian { transform badge {} }",
            ),
            (
                "library.avenger",
                "avenger 1;\
                 export define mark badge { mark symbol {} }",
            ),
        ],
        "root.avenger",
        BTreeMap::new(),
    )
    .await;

    let failure = resolve_project(&graph, &bootstrap_schema())
        .result
        .unwrap_err();
    assert!(
        failure
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-020")
    );
}
