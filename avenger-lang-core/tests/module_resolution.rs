use std::collections::BTreeMap;

use avenger_chart_schema::{
    NativeKindKey, NativeModuleId, NativeModuleSchema, NativeSchemaSnapshot,
};
use avenger_lang_core::{
    AvailableNativeModule, BindingCategory, ChartSelector, ContentVersion, ImportCapabilities,
    InMemorySourceLoader, LoadedSource, ModuleGraphLoadLimits, ModuleGraphLoadRequest,
    ModuleGraphLoader, ModuleRoot, ResolvedKindBinding, SourceOrigin, resolve_module_graph,
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
    let resolved = resolve_module_graph(&graph, &bootstrap_schema())
        .result
        .unwrap();

    assert_eq!(resolved.definitions.len(), 4);
    let root = resolved
        .source_modules
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
    assert_eq!(resolved.items.len(), 5);
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
    let failure = resolve_module_graph(&graph, &bootstrap_schema())
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
    let resolved = resolve_module_graph(&imported, &schema).result.unwrap();
    let mark = &resolved
        .source_modules
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
    let failure = resolve_module_graph(&unimported, &schema)
        .result
        .unwrap_err();
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
    let failure = resolve_module_graph(&unnamed, &bootstrap_schema())
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
    let failure = resolve_module_graph(&duplicate_export, &bootstrap_schema())
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
             chart cartesian as first { param 1 as value; }\
             chart cartesian as second { param 2 as value; }",
        )],
        "root.avenger",
        BTreeMap::new(),
    )
    .await;
    let first_resolved = resolve_module_graph(&first, &bootstrap_schema())
        .result
        .unwrap();
    assert_eq!(first_resolved.entrypoints.len(), 2);
    assert!(first_resolved.entrypoints.iter().all(|(id, chart)| {
        matches!(id.selector, ChartSelector::Named(_))
            && chart.params.len() == 1
            && chart.stores.is_empty()
    }));
    let badge_id = first_resolved
        .items
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
             chart cartesian as first { param 1 as value;\
             }",
        )],
        "root.avenger",
        BTreeMap::new(),
    )
    .await;
    let second_resolved = resolve_module_graph(&second, &bootstrap_schema())
        .result
        .unwrap();
    let second_badge_id = second_resolved
        .items
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

    let failure = resolve_module_graph(&graph, &bootstrap_schema())
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

    let failure = resolve_module_graph(&graph, &bootstrap_schema())
        .result
        .unwrap_err();
    assert!(
        failure
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-020")
    );
}

#[tokio::test]
async fn relation_references_resolve_through_namespaces_and_drive_chart_closures() {
    let graph = load(
        &[
            (
                "root.avenger",
                "avenger 1;\
                 import * as data from './library.avenger';\
                 table sql as derived { sql: SELECT * FROM data.movies; }\
                 chart cartesian { data: { table: derived; } }",
            ),
            (
                "library.avenger",
                "avenger 1; export table memory as movies {}",
            ),
        ],
        "root.avenger",
        BTreeMap::new(),
    )
    .await;

    let resolved = resolve_module_graph(&graph, &bootstrap_schema())
        .result
        .unwrap();
    assert_eq!(resolved.catalog_tables.len(), 2);
    let derived = resolved
        .items
        .values()
        .find(|item| item.source_name.as_deref() == Some("derived"))
        .unwrap();
    let movies = resolved
        .items
        .values()
        .find(|item| item.source_name.as_deref() == Some("movies"))
        .unwrap();
    let chart = resolved.entrypoints.values().next().unwrap();
    assert!(chart.reachable_items.contains(&derived.id));
    assert!(chart.reachable_items.contains(&movies.id));
    assert!(resolved.item_dependencies.edges.iter().any(|edge| {
        edge.from == derived.id
            && edge.to == movies.id
            && edge.cause == avenger_lang_core::ItemDependencyCause::RelationUse
    }));
}

#[tokio::test]
async fn transform_definitions_may_join_input_but_mark_definitions_cannot_capture_data() {
    let valid = load(
        &[
            (
                "root.avenger",
                "avenger 1;\
                 import { movies } from './library.avenger';\
                 define transform enrich {\
                   output value;\
                   transform sql { query: SELECT * FROM input JOIN movies USING (id); }\
                 }",
            ),
            (
                "library.avenger",
                "avenger 1; export table memory as movies {}",
            ),
        ],
        "root.avenger",
        BTreeMap::new(),
    )
    .await;
    let resolved = resolve_module_graph(&valid, &bootstrap_schema())
        .result
        .unwrap();
    let enrich = resolved
        .items
        .values()
        .find(|item| item.source_name.as_deref() == Some("enrich"))
        .unwrap();
    let movies = resolved
        .items
        .values()
        .find(|item| item.source_name.as_deref() == Some("movies"))
        .unwrap();
    assert!(resolved.item_dependencies.edges.iter().any(|edge| {
        edge.from == enrich.id
            && edge.to == movies.id
            && edge.cause == avenger_lang_core::ItemDependencyCause::RelationUse
    }));

    let invalid = load(
        &[(
            "root.avenger",
            "avenger 1;\
             table memory as rows {}\
             define mark captured { mark symbol { data: { table: rows; } } }",
        )],
        "root.avenger",
        BTreeMap::new(),
    )
    .await;
    let failure = resolve_module_graph(&invalid, &bootstrap_schema())
        .result
        .unwrap_err();
    assert!(
        failure
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-280")
    );

    let transitive = load(
        &[(
            "root.avenger",
            "avenger 1;\
             table memory as rows {}\
             define transform enrich {\
               output id;\
               transform sql { query: SELECT * FROM input JOIN rows USING (id); }\
             }\
             define mark captured { transform enrich {} mark symbol {} }",
        )],
        "root.avenger",
        BTreeMap::new(),
    )
    .await;
    let failure = resolve_module_graph(&transitive, &bootstrap_schema())
        .result
        .unwrap_err();
    let diagnostic = failure
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-280")
        .expect("transitive dataset-capture diagnostic");
    assert!(
        diagnostic
            .primary
            .message
            .contains("captured -> enrich -> rows"),
        "{diagnostic:#?}"
    );
}

#[tokio::test]
async fn transform_definitions_record_standard_dataset_dependencies() {
    let root = SourceOrigin::Memory("root.avenger".to_owned());
    let loader = InMemorySourceLoader::default()
        .with_source(LoadedSource::new(
            root.clone(),
            r#"avenger 1;
import { country_names } from 'std:datasets.avenger';
export define transform attach_country_name {
  output country_name;
  transform sql {
    query:
      SELECT rows.*, countries.name AS country_name
      FROM input AS rows
      JOIN country_names AS countries
        ON rows.country_code = countries.code;
  }
}"#,
            ContentVersion::new("root-v1"),
        ))
        .with_source(LoadedSource::new(
            SourceOrigin::Std("datasets.avenger".to_owned()),
            "avenger 1; export table memory as country_names {}",
            ContentVersion::new("std-datasets-v1"),
        ));
    let graph = ModuleGraphLoader::new(&loader)
        .load(ModuleGraphLoadRequest {
            project_root: "/project".into(),
            roots: vec![ModuleRoot::requested(root)],
            native_modules: BTreeMap::new(),
            capabilities: ImportCapabilities::in_memory("/project"),
            schema_version: "semantic-v1".to_owned(),
            registry_version: "bootstrap".to_owned(),
            limits: ModuleGraphLoadLimits::default(),
        })
        .await
        .result
        .unwrap();
    let resolved = resolve_module_graph(&graph, &bootstrap_schema())
        .result
        .unwrap();
    let transform = resolved
        .items
        .values()
        .find(|item| item.source_name.as_deref() == Some("attach_country_name"))
        .expect("transform item");
    let dataset = resolved
        .items
        .values()
        .find(|item| item.source_name.as_deref() == Some("country_names"))
        .expect("standard dataset item");
    assert!(resolved.item_dependencies.edges.iter().any(|edge| {
        edge.from == transform.id
            && edge.to == dataset.id
            && edge.cause == avenger_lang_core::ItemDependencyCause::RelationUse
    }));
}
