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
    let valid_project = project(
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
    let resolved = resolve_project(&valid_project, &bootstrap_schema())
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

#[tokio::test]
async fn resolve_imported_definition_exports_are_typed_and_instance_scoped() {
    let project = project(
        &[
            (
                "chart.avenger",
                r#"
avenger 1;
import 'controller.tool.avenger';
chart cartesian as chart {
  tool controller as first {}
  tool controller as second {}
  param as selected { type: int64; default: $first.value; }
}
"#,
            ),
            (
                "controller.tool.avenger",
                r#"
avenger 1;
define tool controller {
  param as threshold { type: int64; default: 0; }
  export threshold as value;
}
"#,
            ),
        ],
        "chart.avenger",
    )
    .await;
    let resolved = resolve_project(&project, &bootstrap_schema())
        .result
        .unwrap();
    let chart = resolved
        .files
        .values()
        .find(|file| matches!(file.kind, avenger_lang_core::ProjectFileKind::Chart))
        .unwrap();
    let instances = chart.roots[0]
        .children
        .iter()
        .filter(|child| child.keyword == "tool")
        .collect::<Vec<_>>();
    assert_eq!(instances.len(), 2);
    let first = instances[0].exports.get("value").unwrap();
    let second = instances[1].exports.get("value").unwrap();
    assert!(matches!(first, ResolvedTarget::DefinitionParam { .. }));
    assert!(matches!(second, ResolvedTarget::DefinitionParam { .. }));
    assert_ne!(first, second);
    let definition = resolved.definitions.values().next().unwrap();
    assert_eq!(
        definition.exports["value"].target_kind,
        avenger_lang_core::DefinitionExportKind::Param
    );
    assert_eq!(
        definition.exports["value"]
            .data_type
            .as_ref()
            .unwrap()
            .to_string(),
        "int64"
    );
}

#[tokio::test]
async fn resolve_transform_outputs_are_sequential_typed_handles() {
    let valid_project = project(
        &[(
            "pipeline.avenger",
            r#"
avenger 1;
chart cartesian as pipeline {
  transform aggregate as stats {
    measures: [{ name: 'total'; op: sum; expr: "amount"; }];
  }
  mark symbol as points { x: stats.total; y: "y"; }
}
"#,
        )],
        "pipeline.avenger",
    )
    .await;
    let resolved = resolve_project(&valid_project, &bootstrap_schema())
        .result
        .unwrap();
    let root = &resolved.files.values().next().unwrap().roots[0];
    let transform = &root.children[0];
    assert!(transform.transform_outputs.contains_key("total"));
    let mark = &root.children[1];
    let avenger_lang_core::ResolvedValue::Expression(expression) = &mark.properties["x"] else {
        panic!("x expression")
    };
    assert!(matches!(
        expression.references.as_slice(),
        [avenger_lang_core::ResolvedSqlReference {
            target: ResolvedTarget::Output(_),
            ..
        }]
    ));

    let invalid = project(
        &[(
            "future.avenger",
            r#"
avenger 1;
chart cartesian as future {
  mark symbol { x: stats.total; y: "y"; }
  transform aggregate as stats {
    measures: [{ name: 'total'; op: sum; expr: "amount"; }];
  }
}
"#,
        )],
        "future.avenger",
    )
    .await;
    let failure = resolve_project(&invalid, &bootstrap_schema())
        .result
        .unwrap_err();
    assert!(
        failure
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-083")
    );
}

#[tokio::test]
async fn resolve_table_dag_orders_relations_and_reports_cycles() {
    let valid_project = project(
        &[
            (
                "tables.avenger",
                "avenger 1; import 'catalog.data.avenger'; chart cartesian as tables {}",
            ),
            (
                "catalog.data.avenger",
                r#"
avenger 1;
schema tables as vega {
  table csv as base { path: 'base.csv'; }
  table sql as derived { sql: SELECT * FROM vega.base; }
}
"#,
            ),
        ],
        "tables.avenger",
    )
    .await;
    let resolved = resolve_project(&valid_project, &bootstrap_schema())
        .result
        .unwrap();
    assert_eq!(resolved.table_order.len(), 2);

    let cyclic = project(
        &[
            (
                "cycle.avenger",
                "avenger 1; import 'cycle.data.avenger'; chart cartesian as cycle {}",
            ),
            (
                "cycle.data.avenger",
                r#"
avenger 1;
schema tables as vega {
  table sql as a { sql: SELECT * FROM vega.b; }
  table sql as b { sql: SELECT * FROM vega.a; }
}
"#,
            ),
        ],
        "cycle.avenger",
    )
    .await;
    let failure = resolve_project(&cyclic, &bootstrap_schema())
        .result
        .unwrap_err();
    assert!(
        failure
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-101")
    );
}

#[tokio::test]
async fn resolve_event_routes_temporal_reads_and_ordered_typed_actions() {
    let valid_project = project(
        &[(
            "events.avenger",
            r#"
avenger 1;
chart cartesian as events {
  param as x { type: int8; default: 0; }
  mark symbol as points { x: "x"; y: "y"; }
  on cursor_moved as drag {
    target: mark points;
    scope: plot;
    surface: plot;
    between: {
      start: mouse_down { filter: $x >= 0; }
      end: mouse_up { filter: $x >= 0; }
    }
    set param x at start = $x@start + event_facet_value(0);
    set cursor = 'crosshair';
  }
}
"#,
        )],
        "events.avenger",
    )
    .await;
    let resolved = resolve_project(&valid_project, &bootstrap_schema())
        .result
        .unwrap();
    let event = &resolved.files.values().next().unwrap().roots[0].children[2];
    assert!(event.migration_key.is_some());
    assert!(event.public_path.is_none());
    assert_eq!(event.children.len(), 2);
    let avenger_lang_core::ResolvedValue::Expression(value) =
        &event.children[0].properties["value"]
    else {
        panic!("param action expression")
    };
    assert_eq!(value.helpers[0].name, "event_facet_value");
    assert!(matches!(
        value.helpers[0].arguments.as_slice(),
        [avenger_lang_core::ResolvedHelperArgument::Number(value)] if value == "0"
    ));

    let invalid = project(
        &[(
            "bad_events.avenger",
            r#"
avenger 1;
chart cartesian as bad_events {
  param as x { type: int8; default: 0; }
  on cursor_moved as drag {
    between: {
      start: mouse_down { filter: $x@previous > 0; }
      end: mouse_up {}
    }
    set param x = 128;
  }
}
"#,
        )],
        "bad_events.avenger",
    )
    .await;
    let failure = resolve_project(&invalid, &bootstrap_schema())
        .result
        .unwrap_err();
    let codes = failure
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect::<Vec<_>>();
    assert!(codes.contains(&"AVENGER-RESOLVE-106"));
    assert!(codes.contains(&"AVENGER-RESOLVE-076"));
}

fn contains_invalid(declaration: &avenger_lang_core::ResolvedDeclaration) -> bool {
    declaration
        .properties
        .values()
        .any(|value| matches!(value, avenger_lang_core::ResolvedValue::Invalid))
        || declaration.children.iter().any(contains_invalid)
}
