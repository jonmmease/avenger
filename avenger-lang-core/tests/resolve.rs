use avenger_chart_schema::NativeSchemaSnapshot;
use avenger_lang_core::{
    ContentVersion, ImportCapabilities, InMemorySourceLoader, LoadedSource, ProjectLoadRequest,
    ProjectLoader, ProjectRoot, ResolvedTarget, SourceOrigin, resolve_project,
};

fn bootstrap_schema() -> NativeSchemaSnapshot {
    serde_json::from_str(include_str!(
        "../../avenger-chart-lang-registry/snapshots/bootstrap-schema.json"
    ))
    .unwrap()
}

async fn project(sources: &[(&str, &str)], root: &str) -> avenger_lang_core::ParsedProject {
    let loader = InMemorySourceLoader::default();
    for (name, text) in sources {
        loader.insert(LoadedSource::new(
            SourceOrigin::Memory((*name).to_owned()),
            *text,
            ContentVersion::new("fixture-v1"),
        ));
    }
    ProjectLoader::new(&loader)
        .load(ProjectLoadRequest {
            project_root: "/project".into(),
            roots: vec![ProjectRoot::chart(SourceOrigin::Memory(root.to_owned()))],
            capabilities: ImportCapabilities::in_memory("/project"),
            schema_version: "semantic-v1".to_owned(),
            registry_version: "bootstrap".to_owned(),
        })
        .await
        .result
        .unwrap()
}

#[tokio::test]
async fn resolve_valid_kernel_project_binds_forward_params_and_native_mark() {
    let project = project(
        &[(
            "chart.avenger",
            r#"
avenger 1;
chart cartesian as chart {
  param as upper { type: int64; default: $lower + 10; }
  param as lower { type: int64; default: 0; }
  group as points {
    mark symbol as dots { x: "x"; y: "y"; }
  }
}
"#,
        )],
        "chart.avenger",
    )
    .await;
    let resolved = resolve_project(&project, &bootstrap_schema())
        .result
        .unwrap();
    assert_eq!(resolved.params.len(), 2);
    assert_eq!(resolved.param_default_order.len(), 2);
    assert!(resolved.public_targets.contains_key("chart.points.dots"));
    assert!(
        resolved
            .files
            .values()
            .all(|file| { file.roots.iter().all(|root| !contains_invalid(root)) })
    );
}

#[tokio::test]
async fn resolve_shared_param_store_namespace_shadows_without_kind_fallback() {
    let project = project(
        &[(
            "duplicate.avenger",
            r#"
avenger 1;
chart cartesian {
  param as state { type: int64; default: 0; }
  store as state { field id: int64; }
}
"#,
        )],
        "duplicate.avenger",
    )
    .await;
    let failure = resolve_project(&project, &bootstrap_schema())
        .result
        .unwrap_err();
    assert!(
        failure
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-010")
    );
}

#[tokio::test]
async fn resolve_widget_instances_get_distinct_generated_state_and_migration_ids() {
    let project = project(
        &[(
            "widgets.avenger",
            r#"
avenger 1;
chart cartesian as widgets {
  widget radio_button_list as first {
    id: 'first';
    items: [{ value: 'a'; label: 'A'; }, { value: 'b'; label: 'B'; }];
  }
  widget radio_button_list as second {
    id: 'second';
    items: [{ value: 'a'; label: 'A'; }, { value: 'b'; label: 'B'; }];
  }
  param as selected { type: utf8; default: $first.value; }
}
"#,
        )],
        "widgets.avenger",
    )
    .await;
    let resolved = resolve_project(&project, &bootstrap_schema())
        .result
        .unwrap();
    let generated = resolved
        .params
        .values()
        .filter(|param| param.generated_by.is_some())
        .collect::<Vec<_>>();
    assert_eq!(generated.len(), 2);
    assert_ne!(generated[0].id, generated[1].id);
    assert_ne!(generated[0].migration_key, generated[1].migration_key);
    assert!(resolved.public_targets.values().any(|target| {
        matches!(target, ResolvedTarget::Param(id) if generated.iter().any(|param| &param.id == id))
    }));
}

#[tokio::test]
async fn resolve_reports_complete_param_cycle_and_independent_schema_errors() {
    let project = project(
        &[(
            "invalid.avenger",
            r#"
avenger 1;
chart cartesian {
  param as a { type: int64; default: $b; }
  param as b { type: int64; default: $a; }
  mark symbol { x: "x"; bogus: 1; }
}
"#,
        )],
        "invalid.avenger",
    )
    .await;
    let failure = resolve_project(&project, &bootstrap_schema())
        .result
        .unwrap_err();
    let codes = failure
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect::<Vec<_>>();
    assert!(codes.contains(&"AVENGER-RESOLVE-100"));
    assert!(codes.contains(&"AVENGER-RESOLVE-021"));
    assert!(codes.contains(&"AVENGER-RESOLVE-022"));
}

fn contains_invalid(declaration: &avenger_lang_core::ResolvedDeclaration) -> bool {
    declaration
        .properties
        .values()
        .any(|value| matches!(value, avenger_lang_core::ResolvedValue::Invalid))
        || declaration.children.iter().any(contains_invalid)
}
