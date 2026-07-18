use std::{fs, path::PathBuf};

use avenger_chart_schema::{NativeKindKey, NativeKindNamespace, NativeSchemaSnapshot};
use avenger_lang_core::{
    ContentVersion, ImportCapabilities, InMemorySourceLoader, LoadedSource, ProjectLoadRequest,
    ProjectLoader, ProjectRoot, ResolvedSelectionCombine, ResolvedSelectionEmpty, ResolvedTarget,
    SourceOrigin, render_diagnostics, resolve_project,
};

fn bootstrap_schema() -> NativeSchemaSnapshot {
    serde_json::from_str(include_str!(
        "../../avenger-chart-lang-registry/snapshots/bootstrap-schema.json"
    ))
    .unwrap()
}

fn assert_resolve_baseline(name: &str, actual: &str) {
    let baseline = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/baselines/resolve")
        .join(name);
    if std::env::var_os("AVENGER_LANG_UPDATE_BASELINES").is_some() {
        fs::create_dir_all(baseline.parent().unwrap()).unwrap();
        fs::write(&baseline, actual).unwrap();
    } else {
        let expected = fs::read_to_string(&baseline).unwrap_or_else(|error| {
            panic!(
                "failed to read reviewed baseline {}: {error}",
                baseline.display()
            )
        });
        assert_eq!(actual, expected, "resolve baseline changed: {name}");
    }
}

fn assert_closed_cycle(diagnostic: &avenger_lang_core::Diagnostic) {
    let path = diagnostic
        .primary
        .message
        .strip_prefix("cycle: ")
        .expect("cycle diagnostic includes its dependency path")
        .split(" -> ")
        .collect::<Vec<_>>();
    assert!(path.len() >= 3, "cycle must include at least one edge");
    assert_eq!(path.first(), path.last(), "cycle path must close");
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
async fn resolved_configured_value_preserves_its_value_head() {
    let valid_project = project(
        &[(
            "configured_channel.avenger",
            r#"
avenger 1;
chart cartesian {
  mark symbol as dots {
    x: "x" { scale: linear; }
    y: "y";
  }
}
"#,
        )],
        "configured_channel.avenger",
    )
    .await;
    let resolved = resolve_project(&valid_project, &bootstrap_schema())
        .result
        .unwrap();
    let chart = resolved
        .files
        .values()
        .flat_map(|file| &file.roots)
        .find(|declaration| declaration.keyword == "chart")
        .unwrap();
    let mark = chart
        .children
        .iter()
        .find(|declaration| declaration.keyword == "mark")
        .unwrap();
    let avenger_lang_core::ResolvedValue::Object {
        head,
        kind,
        properties,
        ..
    } = &mark.properties["x"]
    else {
        panic!("configured channel must resolve to an object")
    };
    assert_eq!(
        head.as_deref(),
        Some(&avenger_lang_core::ResolvedValue::Column("x".into()))
    );
    assert_eq!(kind, &None);
    assert!(
        matches!(properties.get("scale"), Some(avenger_lang_core::ResolvedValue::Atom(value)) if value == "linear")
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
async fn resolve_namespace_matrix_separates_value_selection_structure_and_events() {
    let valid = project(
        &[(
            "namespaces.avenger",
            r#"
avenger 1;
chart cartesian as namespaces {
  private param as shared { type: int64; default: 0; }
  private selection as shared {}
  mark symbol as shared { x: "x"; y: "y"; }
  on click as shared {
    target: mark shared;
    set selection shared = clear;
    set param shared = $shared + 1;
  }
}
"#,
        )],
        "namespaces.avenger",
    )
    .await;
    let resolved = resolve_project(&valid, &bootstrap_schema()).result.unwrap();
    assert_eq!(resolved.params.len(), 1);
    assert_eq!(resolved.selections.len(), 1);
    let event = &resolved.files.values().next().unwrap().roots[0].children[3];
    assert!(event.public_path.is_none());
    assert!(matches!(
        event.event_binding.as_ref().unwrap().targets.as_slice(),
        [ResolvedTarget::Mark(_)]
    ));
    assert!(matches!(
        event.children[0].state_lvalue.as_ref().unwrap().target,
        ResolvedTarget::Selection(_)
    ));
    assert!(matches!(
        event.children[1].state_lvalue.as_ref().unwrap().target,
        ResolvedTarget::Param(_)
    ));

    let invalid = project(
        &[(
            "duplicate_namespaces.avenger",
            r#"
avenger 1;
chart cartesian {
  selection as picked {}
  selection as picked {}
  mark symbol as points { x: "x"; y: "y"; }
  mark symbol as points { x: "x"; y: "y"; }
  on click as handler {}
  on click as handler {}
}
"#,
        )],
        "duplicate_namespaces.avenger",
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
    assert!(codes.contains(&"AVENGER-RESOLVE-011"));
    assert!(codes.contains(&"AVENGER-RESOLVE-012"));
    assert!(codes.contains(&"AVENGER-RESOLVE-013"));
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
    data: { values: [{ value: 'a'; label: 'A'; }, { value: 'b'; label: 'B'; }]; }
    position: top;
  }
  widget radio_button_list as second {
    data: { values: [{ value: 'a'; label: 'A'; }, { value: 'b'; label: 'B'; }]; }
    position: top;
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
    let (Some(ResolvedTarget::Widget(first)), Some(ResolvedTarget::Widget(second))) = (
        resolved.public_targets.get("widgets.first"),
        resolved.public_targets.get("widgets.second"),
    ) else {
        panic!("widget instance targets")
    };
    assert_ne!(first, second);
    assert!(resolved.public_targets.values().any(|target| {
        matches!(target, ResolvedTarget::Param(id) if generated.iter().any(|param| &param.id == id))
    }));
}

#[tokio::test]
async fn resolve_native_tools_get_instance_scoped_generated_state() {
    let project = project(
        &[(
            "tools.avenger",
            r#"
avenger 1;
chart cartesian as tools {
  tool pan_scroll_zoom as first {}
  tool pan_scroll_zoom as second {}
  param as domain {
    type: fixed_size_list(float64, 2);
    default: $first.x_domain;
  }
}
"#,
        )],
        "tools.avenger",
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
    assert_eq!(generated.len(), 6);
    assert_eq!(
        generated
            .iter()
            .map(|param| &param.id)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        6
    );
    assert_eq!(
        generated
            .iter()
            .filter_map(|param| param.migration_key.as_ref())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        6
    );
}

#[tokio::test]
async fn resolve_widget_existing_state_parts_and_typed_failures() {
    let valid = project(
        &[(
            "chart.avenger",
            r#"
avenger 1;
chart cartesian as chart {
  param as selected { type: utf8; default: 'a'; }
  widget radio_button_list as choice {
    data: { values: [{ value: 'a'; label: 'A'; }, { value: 'b'; label: 'B'; }]; }
    position: top;
    value_param: $selected;
  }
  on click { target: mark choice.control; }
}
"#,
        )],
        "chart.avenger",
    )
    .await;
    let resolved = resolve_project(&valid, &bootstrap_schema()).result.unwrap();
    assert_eq!(
        resolved.params.len(),
        1,
        "existing state is not regenerated"
    );
    assert!(matches!(
        resolved.public_targets.get("chart.choice.control"),
        Some(ResolvedTarget::Part { alias, .. }) if alias == "control"
    ));
    let widget = &resolved.files.values().next().unwrap().roots[0].children[1];
    assert!(matches!(
        widget.exports.get("value"),
        Some(ResolvedTarget::Param(id)) if resolved.params.contains_key(id)
    ));

    let invalid = project(
        &[(
            "invalid.avenger",
            r#"
avenger 1;
chart cartesian {
  param as wrong { type: int64; default: 1; }
  store as wrong_kind { field id: utf8; }
  widget radio_button_list as choice {
    data: { values: [{ value: 'a'; label: 'A'; }]; }
    position: top;
    value_param: $wrong;
  }
  widget radio_button_list as wrong_binding_kind {
    data: { values: [{ value: 'a'; label: 'A'; }]; }
    position: top;
    value_param: $wrong_kind;
  }
  param as also_wrong { type: utf8; default: $choice.value; }
}
"#,
        )],
        "invalid.avenger",
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
    assert!(codes.contains(&"AVENGER-RESOLVE-029"));
    assert!(codes.contains(&"AVENGER-RESOLVE-064"));
    assert!(codes.contains(&"AVENGER-RESOLVE-077"));
}

#[tokio::test]
async fn resolve_native_schema_contracts_cover_placement_body_children_and_coordinates() {
    let invalid_shape_and_placement = project(
        &[(
            "contracts.avenger",
            r#"
avenger 1;
chart cartesian as contracts {
  group {
    widget radio_button_list as nested {
      data: { values: [{ value: 'a'; label: 'A'; }]; }
      position: top;
    }
  }
  widget radio_button_list as expanded {
    data: { values: [{ value: 'a'; label: 'A'; }]; }
    position: top;
    mark symbol { x: "x"; y: "y"; }
  }
  mark symbol { x: not_sql(); y: "y";
    view cartesian {}
    view cartesian {}
  }
}
"#,
        )],
        "contracts.avenger",
    )
    .await;
    let failure = resolve_project(&invalid_shape_and_placement, &bootstrap_schema())
        .result
        .unwrap_err();
    let codes = failure
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect::<Vec<_>>();
    assert!(codes.contains(&"AVENGER-RESOLVE-016"));
    assert!(codes.contains(&"AVENGER-RESOLVE-015"));
    assert!(codes.contains(&"AVENGER-RESOLVE-038"));
    assert!(codes.contains(&"AVENGER-RESOLVE-024"));
    assert!(codes.contains(&"AVENGER-RESOLVE-044"));
    // Function calls are syntactically valid SQL expressions at this phase;
    // DataFusion resolves the function name against the active session during
    // planning rather than treating the call as a schema-shape mismatch.

    let polar = project(
        &[(
            "polar.avenger",
            "avenger 1; chart polar as polar { tool pan_scroll_zoom {} }",
        )],
        "polar.avenger",
    )
    .await;
    let mut registry = bootstrap_schema();
    let cartesian_key = NativeKindKey::new(NativeKindNamespace::Coordinate, "cartesian");
    let mut polar_schema = registry.entries[&cartesian_key].clone();
    polar_schema.key = NativeKindKey::new(NativeKindNamespace::Coordinate, "polar");
    registry
        .entries
        .insert(polar_schema.key.clone(), polar_schema);
    let failure = resolve_project(&polar, &registry).result.unwrap_err();
    assert!(
        failure
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-023")
    );
}

#[tokio::test]
async fn resolve_private_exports_hoisting_and_nearest_value_shadowing() {
    let valid = project(
        &[(
            "chart.avenger",
            r#"
avenger 1;
chart cartesian as chart {
  group as controls {
    private param as internal { type: int64; default: 7; }
    export internal as value;
  }
  private group as implementation {
    public mark symbol as visible { x: "x"; y: "y"; }
  }
  param as copied { type: int64; default: $controls.value; }
}
"#,
        )],
        "chart.avenger",
    )
    .await;
    let resolved = resolve_project(&valid, &bootstrap_schema()).result.unwrap();
    assert!(resolved.public_targets.contains_key("chart.controls.value"));
    assert!(resolved.public_targets.contains_key("chart.visible"));
    assert!(
        !resolved
            .public_targets
            .keys()
            .any(|path| path.contains("internal") || path.contains("implementation"))
    );

    let invalid = project(
        &[(
            "shadow.avenger",
            r#"
avenger 1;
chart cartesian {
  param as data { type: int64; default: 1; }
  group as nested {
    store as data { field id: int64; }
    param as copy { type: int64; default: $data; }
  }
}
"#,
        )],
        "shadow.avenger",
    )
    .await;
    let failure = resolve_project(&invalid, &bootstrap_schema())
        .result
        .unwrap_err();
    assert!(
        failure
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-064")
    );

    let duplicate_exports = project(
        &[(
            "exports.avenger",
            r#"
avenger 1;
chart cartesian as exports {
  group as component {
    private mark symbol as first { x: "x"; y: "y"; }
    private mark symbol as second { x: "x"; y: "y"; }
    export first as glyph;
    export second as glyph;
  }
}
"#,
        )],
        "exports.avenger",
    )
    .await;
    let failure = resolve_project(&duplicate_exports, &bootstrap_schema())
        .result
        .unwrap_err();
    assert!(
        failure
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-130")
    );
}

#[tokio::test]
async fn resolve_selection_contract_is_hoisted_and_core_properties_are_closed() {
    let valid = project(
        &[(
            "selection.avenger",
            r#"
avenger 1;
chart cartesian {
  selection as picked { empty: all; combine: intersect; }
}
"#,
        )],
        "selection.avenger",
    )
    .await;
    let resolved = resolve_project(&valid, &bootstrap_schema()).result.unwrap();
    let selection = resolved.selections.values().next().unwrap();
    assert_eq!(selection.empty, ResolvedSelectionEmpty::All);
    assert_eq!(selection.combine, ResolvedSelectionCombine::Intersect);

    let invalid = project(
        &[(
            "bad_selection.avenger",
            r#"
avenger 1;
chart cartesian {
  selection as picked { empty: maybe; combine: either; bogus: true; }
}
"#,
        )],
        "bad_selection.avenger",
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
    assert!(codes.contains(&"AVENGER-RESOLVE-141"));
    assert!(codes.contains(&"AVENGER-RESOLVE-143"));
    assert!(codes.contains(&"AVENGER-RESOLVE-144"));
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
  resource tiles as missing_url { kind: xyz; }
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
    let cycle = failure
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-100")
        .unwrap();
    assert_closed_cycle(cycle);
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
    let definition_param = resolved
        .params
        .values()
        .find(|param| param.source_name == "threshold")
        .unwrap();
    assert!(definition_param.migration_key.is_none());
    assert!(definition_param.definition_local_seed.is_some());
}

#[tokio::test]
async fn resolve_definition_templates_keep_logical_channels_and_slot_bindings_typed() {
    let project = project(
        &[
            (
                "chart.avenger",
                r#"
avenger 1;
import 'point_pair.mark.avenger';
chart cartesian as chart {
  mark point_pair as pair { horizontal_value: "x"; vertical_value: "y"; }
  on click { target: mark pair.point; }
}
"#,
            ),
            (
                "point_pair.mark.avenger",
                r#"
avenger 1;
define mark point_pair {
  channel horizontal: x;
  channel vertical: y;
  slot expr as horizontal_value;
  slot expr as vertical_value;
  group as body {
    mark symbol as point {
      horizontal: horizontal_value;
      vertical: vertical_value;
      size: channel(horizontal) + 1;
    }
  }
  export body.point as point;
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

    let definition_file = resolved
        .files
        .values()
        .find(|file| matches!(file.kind, avenger_lang_core::ProjectFileKind::Definition(_)))
        .unwrap();
    let point = &definition_file.roots[0].children[4].children[0];
    assert!(matches!(
        point.property_channels.get("horizontal"),
        Some(ResolvedTarget::DefinitionChannel { name, .. }) if name == "horizontal"
    ));
    assert!(matches!(
        point.properties.get("horizontal"),
        Some(avenger_lang_core::ResolvedValue::DefinitionArgument(
            ResolvedTarget::DefinitionSlot { name, .. }
        )) if name == "horizontal_value"
    ));
    let avenger_lang_core::ResolvedValue::Expression(size) = &point.properties["size"] else {
        panic!("definition channel helper expression")
    };
    assert!(matches!(
        size.helpers[0].arguments.as_slice(),
        [avenger_lang_core::ResolvedHelperArgument::DefinitionChannel {
            target: ResolvedTarget::DefinitionChannel { name, .. },
            family_suffix,
        }] if name == "horizontal" && family_suffix.is_empty()
    ));

    let chart_file = resolved
        .files
        .values()
        .find(|file| matches!(file.kind, avenger_lang_core::ProjectFileKind::Chart))
        .unwrap();
    let instance = &chart_file.roots[0].children[0];
    assert!(matches!(
        instance.parts.get("point"),
        Some(part) if part.targetable && part.source_alias == "point"
    ));
    assert!(matches!(
        instance.exports.get("point"),
        Some(ResolvedTarget::DefinitionStructural { .. })
    ));
}

#[tokio::test]
async fn resolve_definition_import_dag_orders_dependencies_before_dependents() {
    let project = project(
        &[
            (
                "chart.avenger",
                "avenger 1; import 'high.tool.avenger'; chart cartesian as chart { tool high {} }",
            ),
            (
                "high.tool.avenger",
                r#"
avenger 1;
import 'base.tool.avenger';
define tool high { tool base as inner {} }
"#,
            ),
            (
                "base.tool.avenger",
                "avenger 1; define tool base { param as state { type: int64; default: 0; } }",
            ),
        ],
        "chart.avenger",
    )
    .await;
    let resolved = resolve_project(&project, &bootstrap_schema())
        .result
        .unwrap();
    let names = resolved
        .definition_import_order
        .iter()
        .map(|id| resolved.definitions[id].source_name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["base", "high"]);
}

#[tokio::test]
async fn resolve_definition_outputs_are_typed_and_require_bound_instances() {
    let valid_project = project(
        &[
            (
                "chart.avenger",
                r#"
avenger 1;
import 'project.transform.avenger';
chart cartesian as chart {
  transform project as projected { measure: "x"; }
  mark symbol { x: projected.result; y: "y"; }
}
"#,
            ),
            (
                "project.transform.avenger",
                r#"
avenger 1;
define transform project {
  slot expr as measure;
  output result: measure;
}
"#,
            ),
        ],
        "chart.avenger",
    )
    .await;
    let resolved = resolve_project(&valid_project, &bootstrap_schema())
        .result
        .unwrap();
    let definition = resolved.definitions.values().next().unwrap();
    assert!(matches!(
        definition.outputs.get("result"),
        Some(Some(avenger_lang_core::ResolvedValue::DefinitionArgument(
            ResolvedTarget::DefinitionSlot { name, .. }
        ))) if name == "measure"
    ));
    let chart = resolved
        .files
        .values()
        .find(|file| matches!(file.kind, avenger_lang_core::ProjectFileKind::Chart))
        .unwrap();
    let transform = &chart.roots[0].children[0];
    assert!(transform.transform_outputs.contains_key("result"));
    let avenger_lang_core::ResolvedValue::Expression(expression) =
        &chart.roots[0].children[1].properties["x"]
    else {
        panic!("definition output expression")
    };
    assert!(matches!(
        expression.references.as_slice(),
        [avenger_lang_core::ResolvedSqlReference {
            target: ResolvedTarget::Output(_),
            ..
        }]
    ));

    let unbound = project(
        &[
            (
                "chart.avenger",
                "avenger 1; import 'project.transform.avenger'; chart cartesian as chart { transform project { measure: \"x\"; } }",
            ),
            (
                "project.transform.avenger",
                "avenger 1; define transform project { slot expr as measure; output result: measure; }",
            ),
        ],
        "chart.avenger",
    )
    .await;
    let failure = resolve_project(&unbound, &bootstrap_schema())
        .result
        .unwrap_err();
    assert!(
        failure
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-028")
    );
}

#[tokio::test]
async fn resolve_inline_views_are_body_lexical_and_never_public() {
    let valid = project(
        &[(
            "chart.avenger",
            r#"
avenger 1;
chart cartesian as chart {
  group as owner {
    view cartesian as viewport {
      x_domain: "x";
      y_domain: "y";
      transform filter as visible { predicate: view_x(viewport, pixels) > 0; }
      mark symbol as points { x: "x"; y: "y"; }
    }
  }
}
"#,
        )],
        "chart.avenger",
    )
    .await;
    let resolved = resolve_project(&valid, &bootstrap_schema()).result.unwrap();
    assert!(!resolved.public_targets.contains_key("chart.owner.viewport"));
    let view = &resolved.files.values().next().unwrap().roots[0].children[0].children[0];
    let predicate = view.children[0].properties.get("predicate").unwrap();
    assert!(matches!(
        predicate,
        avenger_lang_core::ResolvedValue::Expression(expression)
            if matches!(
                expression.helpers[0].arguments[0],
                avenger_lang_core::ResolvedHelperArgument::Target(
                    ResolvedTarget::Declaration(_)
                )
            )
    ));

    let outside = project(
        &[(
            "outside.avenger",
            r#"
avenger 1;
chart cartesian {
  group {
    view cartesian as viewport {
      x_domain: "x";
      y_domain: "y";
      mark symbol { x: "x"; y: "y"; }
    }
    mark symbol { x: view_x(viewport, pixels) + 0; y: "y"; }
  }
}
"#,
        )],
        "outside.avenger",
    )
    .await;
    let failure = resolve_project(&outside, &bootstrap_schema())
        .result
        .unwrap_err();
    assert!(
        failure
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-066"),
        "{:#?}",
        failure.diagnostics
    );
}

#[tokio::test]
async fn resolve_rejects_reusable_exported_and_nested_inline_views() {
    let project = project(
        &[(
            "invalid_views.avenger",
            r#"
avenger 1;
chart cartesian {
  group {
    public view cartesian as reusable {
      export child;
      view cartesian as nested {
        mark symbol as child { x: "x"; y: "y"; }
      }
    }
  }
}
"#,
        )],
        "invalid_views.avenger",
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
    assert!(codes.contains(&"AVENGER-RESOLVE-035"));
    assert!(codes.contains(&"AVENGER-RESOLVE-036"));
    assert!(codes.contains(&"AVENGER-RESOLVE-037"));
}

#[tokio::test]
async fn resolve_named_runtime_ids_ignore_irrelevant_sibling_order() {
    async fn identities(source: &str) -> (String, String, String) {
        let project = project(&[("stable.avenger", source)], "stable.avenger").await;
        let resolved = resolve_project(&project, &bootstrap_schema())
            .result
            .unwrap();
        let param = resolved
            .params
            .values()
            .find(|param| param.source_name == "stable_param")
            .unwrap();
        let widget = resolved
            .params
            .values()
            .find(|param| param.generated_by.is_some())
            .unwrap();
        let mark = resolved.public_targets.get("stable.stable_mark").unwrap();
        (
            param.id.to_string(),
            widget.migration_key.as_ref().unwrap().to_string(),
            format!("{mark:?}"),
        )
    }

    let first = identities(
        r#"
avenger 1;
chart cartesian as stable {
  param as unrelated { type: boolean; default: true; }
  mark symbol as stable_mark { x: "x"; y: "y"; }
  widget radio_button_list as stable_widget {
    data: { values: [{ value: 1; label: 'one'; }]; }
    position: top;
  }
  param as stable_param { type: int64; default: 1; }
}
"#,
    )
    .await;
    let second = identities(
        r#"
avenger 1;
chart cartesian as stable {
  param as stable_param { type: int64; default: 1; }
  widget radio_button_list as stable_widget {
    data: { values: [{ value: 1; label: 'one'; }]; }
    position: top;
  }
  mark symbol as stable_mark { x: "x"; y: "y"; }
  param as unrelated { type: boolean; default: true; }
}
"#,
    )
    .await;
    assert_eq!(first, second);
}

#[tokio::test]
async fn resolve_imported_definition_diagnostics_retain_import_trace() {
    let project = project(
        &[
            (
                "chart.avenger",
                r#"
avenger 1;
import 'broken.mark.avenger';
chart cartesian as chart { mark broken { value: "x"; } }
"#,
            ),
            (
                "broken.mark.avenger",
                r#"
avenger 1;
define mark broken {
  slot expr as later;
  slot enum as mode { values: [a, b]; default: missing; }
  group {}
}
"#,
            ),
        ],
        "chart.avenger",
    )
    .await;
    let failure = resolve_project(&project, &bootstrap_schema())
        .result
        .unwrap_err();
    let diagnostic = failure
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-009")
        .expect("definition schema error");
    assert_eq!(diagnostic.trace.len(), 1);
    assert_resolve_baseline(
        "definition-import-trace.txt",
        &render_diagnostics(&failure.diagnostics, &failure.sources),
    );
}

#[tokio::test]
async fn resolve_public_mark_and_component_part_provenance_matches_baseline() {
    let project = project(
        &[(
            "provenance.avenger",
            r#"
avenger 1;
chart cartesian as provenance {
  group as composite {
    component_kind: point_pair;
    mark symbol as point { x: "x"; y: "y"; }
    export point as glyph;
  }
  widget radio_button_list as choice {
    data: { values: [{ value: 'a'; label: 'A'; }]; }
    position: top;
  }
}
"#,
        )],
        "provenance.avenger",
    )
    .await;
    let resolved = resolve_project(&project, &bootstrap_schema())
        .result
        .unwrap();
    let root = &resolved.files.values().next().unwrap().roots[0];
    let component = &root.children[0];
    let widget = &root.children[1];
    let generated_state = resolved
        .params
        .values()
        .filter(|param| param.generated_by.is_some())
        .collect::<Vec<_>>();
    let snapshot = serde_json::json!({
        "public_targets": resolved.public_targets,
        "component": {
            "id": component.id,
            "component_kind": component.component_kind,
            "public_path": component.public_path,
            "parts": component.parts,
            "exports": component.exports,
        },
        "widget": {
            "id": widget.id,
            "public_path": widget.public_path,
            "parts": widget.parts,
            "exports": widget.exports,
        },
        "generated_state": generated_state,
    });
    assert_resolve_baseline(
        "public-interface.json",
        &format!("{}\n", serde_json::to_string_pretty(&snapshot).unwrap()),
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
    total: sum("amount");
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
    total: sum("amount");
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
async fn resolve_pipeline_defers_output_interfaces_but_preserves_stage_order() {
    let valid = project(
        &[(
            "pipeline.avenger",
            r#"
avenger 1;
chart cartesian as pipeline {
  transform pipeline as summarized {
    output final: second.final;
    transform aggregate as first {
      measures: [{ name: 'total'; op: sum; expr: "amount"; }];
    }
    transform aggregate as second {
      measures: [{ name: 'final'; op: sum; expr: first.total; }];
    }
  }
  mark symbol as points { x: summarized.final; y: "y"; }
}
"#,
        )],
        "pipeline.avenger",
    )
    .await;
    let resolved = resolve_project(&valid, &bootstrap_schema()).result.unwrap();
    let pipeline = &resolved.files.values().next().unwrap().roots[0].children[0];
    let handle = pipeline.transform_outputs.get("final").unwrap();
    assert_eq!(handle.ordinal, 0);
    let output = &pipeline.children[0];
    let avenger_lang_core::ResolvedValue::Expression(value) = &output.properties["value"] else {
        panic!("pipeline output expression")
    };
    assert!(matches!(
        value.references.as_slice(),
        [avenger_lang_core::ResolvedSqlReference {
            target: ResolvedTarget::Output(_),
            ..
        }]
    ));

    let invalid = project(
        &[(
            "duplicate.avenger",
            r#"
avenger 1;
chart cartesian {
  transform pipeline as duplicate {
    transform aggregate { measures: [{ name: 'total'; op: sum; expr: "x"; }]; }
    output total: "total";
    output total: "total";
  }
}
"#,
        )],
        "duplicate.avenger",
    )
    .await;
    let failure = resolve_project(&invalid, &bootstrap_schema())
        .result
        .unwrap_err();
    assert!(
        failure
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-150")
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
    let cycle = failure
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-101")
        .unwrap();
    assert_closed_cycle(cycle);
}

#[tokio::test]
async fn resolve_sql_placeholders_distinguish_scalar_and_table_bindings() {
    let valid = project(
        &[(
            "bindings.avenger",
            r#"
avenger 1;
chart cartesian as bindings {
  param as minimum { type: int64; default: 1; }
  store as rows { field id: int64; }
  transform sql as filtered {
    query:
      SELECT * FROM $rows WHERE "id" >= $minimum;
  }
}
"#,
        )],
        "bindings.avenger",
    )
    .await;
    let resolved = resolve_project(&valid, &bootstrap_schema()).result.unwrap();
    let query = &resolved.files.values().next().unwrap().roots[0].children[2].properties["query"];
    let avenger_lang_core::ResolvedValue::Query(query) = query else {
        panic!("resolved SQL query")
    };
    assert!(query.bindings.iter().any(|binding| {
        binding.kind == avenger_lang_core::ast::BindingKind::Store
            && matches!(binding.target, ResolvedTarget::Store(_))
    }));
    assert!(query.bindings.iter().any(|binding| {
        binding.kind == avenger_lang_core::ast::BindingKind::Param
            && matches!(binding.target, ResolvedTarget::Param(_))
    }));

    let invalid = project(
        &[(
            "wrong_bindings.avenger",
            r#"
avenger 1;
chart cartesian as wrong_bindings {
  param as scalar { type: int64; default: 1; }
  store as relation { field id: int64; }
  transform sql {
    query:
      SELECT * FROM $scalar;
  }
  mark symbol { x: $relation + 1; y: "y"; }
}
"#,
        )],
        "wrong_bindings.avenger",
    )
    .await;
    let failure = resolve_project(&invalid, &bootstrap_schema())
        .result
        .unwrap_err();
    assert_eq!(
        failure
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-064")
            .count(),
        2
    );
}

#[tokio::test]
async fn resolve_nested_arrow_state_and_store_rows_at_exact_boundaries() {
    let valid = project(
        &[(
            "state.avenger",
            r#"
avenger 1;
chart cartesian as state {
  param as pointer {
    type: struct(field('position',struct(field('x',float64),field('y',float64))),field('labels',list(utf8)));
    default: { position: { x: 1; y: NULL; } labels: ['a', 'b']; }
  }
  param as empty_pointer {
    type: struct(field('position',struct(field('x',float64),field('y',float64))),field('labels',list(utf8)));
    default: NULL;
  }
  store as rows {
    field id: utf8;
    field payload: struct(field('count',int8),field('tags',list(utf8))) nullable;
    primary_key: [id];
    row { id: 'one'; }
  }
}
"#,
        )],
        "state.avenger",
    )
    .await;
    let resolved = resolve_project(&valid, &bootstrap_schema()).result.unwrap();
    let store = resolved.stores.values().next().unwrap();
    assert!(matches!(
        store.rows[0].get("payload"),
        Some(avenger_lang_core::ResolvedValue::Null)
    ));

    let invalid = project(
        &[(
            "bad_state.avenger",
            r#"
avenger 1;
chart cartesian as bad_state {
  store as rows {
    field __avenger_store_owner: utf8;
    field id: int8;
    primary_key: [id, id];
    row { id: 128; }
    row { }
  }
}
"#,
        )],
        "bad_state.avenger",
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
    assert!(codes.contains(&"AVENGER-RESOLVE-142"));
    assert!(codes.contains(&"AVENGER-RESOLVE-145"));
    assert!(codes.contains(&"AVENGER-RESOLVE-076"));
    assert!(codes.contains(&"AVENGER-RESOLVE-075"));
}

#[tokio::test]
async fn resolve_catalog_table_params_are_owned_literal_and_self_contained() {
    let valid = project(
        &[
            (
                "chart.avenger",
                "avenger 1; import 'views.data.avenger'; chart cartesian as chart {}",
            ),
            (
                "views.data.avenger",
                r#"
avenger 1;
schema tables as local {
  table parquet as base { path: 'base.parquet'; }
  table sql as filtered {
    param as minimum { type: int64; default: 10; }
    sql: SELECT * FROM local.base WHERE "value" >= $minimum;
  }
}
"#,
            ),
        ],
        "chart.avenger",
    )
    .await;
    let resolved = resolve_project(&valid, &bootstrap_schema()).result.unwrap();
    let table_param = resolved
        .params
        .values()
        .find(|param| param.source_name == "minimum")
        .unwrap();
    assert!(table_param.table_owner.is_some());

    let invalid = project(
        &[
            (
                "chart.avenger",
                "avenger 1; import 'bad.data.avenger'; chart cartesian as chart {}",
            ),
            (
                "bad.data.avenger",
                r#"
avenger 1;
schema tables as local {
  table sql as bad {
    param as first { type: int64; default: 1; }
    param as second { type: int64; default: $first; }
    sql: VALUES ($second);
  }
}
"#,
            ),
        ],
        "chart.avenger",
    )
    .await;
    let failure = resolve_project(&invalid, &bootstrap_schema())
        .result
        .unwrap_err();
    assert!(
        failure
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-135")
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

#[tokio::test]
async fn resolve_reserved_helpers_bind_typed_targets_and_registered_channels() {
    let valid = project(
        &[(
            "helpers.avenger",
            r#"
avenger 1;
chart cartesian as helpers {
  param as cursor_x { type: float64; default: 0; }
  selection as picked {}
  mark symbol as points { x: channel(y) + 1; y: "y"; }
  on click {
    target: mark points;
    filter: selection_contains(picked, datum('id')) = true;
    set param cursor_x = event_coord(x) + 0;
  }
}
"#,
        )],
        "helpers.avenger",
    )
    .await;
    let resolved = resolve_project(&valid, &bootstrap_schema()).result.unwrap();
    let root = &resolved.files.values().next().unwrap().roots[0];
    let avenger_lang_core::ResolvedValue::Expression(x) = &root.children[2].properties["x"] else {
        panic!("mark channel expression")
    };
    assert!(matches!(
        x.helpers.as_slice(),
        [avenger_lang_core::ResolvedHelper {
            name,
            class: avenger_lang_core::HelperClass::Channel,
            arguments,
        }] if name == "channel"
            && matches!(arguments.as_slice(), [avenger_lang_core::ResolvedHelperArgument::Name(channel)] if channel == "y")
    ));
    let event = &root.children[3];
    let avenger_lang_core::ResolvedValue::Expression(filter) = &event.properties["filter"] else {
        panic!("event filter expression")
    };
    assert!(filter.helpers.iter().any(|helper| {
        helper.name == "selection_contains"
            && matches!(
                helper.arguments.first(),
                Some(avenger_lang_core::ResolvedHelperArgument::Target(
                    ResolvedTarget::Selection(_)
                ))
            )
    }));
    assert!(filter.helpers.iter().any(|helper| helper.name == "datum"));
    let avenger_lang_core::ResolvedValue::Expression(action) =
        &event.children[0].properties["value"]
    else {
        panic!("event action expression")
    };
    assert_eq!(action.helpers[0].name, "event_coord");

    let invalid = project(
        &[(
            "bad_helpers.avenger",
            r#"
avenger 1;
chart cartesian as bad_helpers {
  mark symbol as points { x: channel(missing) + 1; y: "y"; }
  on click { target: mark points; filter: event_coord(missing) > 0; }
}
"#,
        )],
        "bad_helpers.avenger",
    )
    .await;
    let failure = resolve_project(&invalid, &bootstrap_schema())
        .result
        .unwrap_err();
    assert_eq!(
        failure
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-154")
            .count(),
        2
    );
}

#[tokio::test]
async fn resolve_event_scope_targets_and_action_lvalues_are_fully_typed() {
    let project = project(
        &[(
            "events.avenger",
            r#"
avenger 1;
chart cartesian as events {
  param as x { type: float64; default: 0; sharing: free; }
  store as rows {
    field id: utf8;
    field value: float64 nullable;
    primary_key: [id];
  }
  selection as picked { empty: none; combine: union; }
  group as overview { mark symbol as points { x: "x"; y: "y"; } }
  group as detail { mark symbol as points { x: "x"; y: "y"; } }
  on cursor_moved as drag {
    target: marks [overview.points, detail.points];
    scope: subplots [overview, detail];
    surface: all;
    between: {
      start: mouse_down { target: mark overview.points; filter: $x >= 0; }
      end: mouse_up { filter: $x >= 0; }
    }
    set param x at start replacing scopes = event_coord(x);
    set store rows = insert_rows { row { id: 'cursor'; value: event_coord(x); } }
    set selection picked = replace_all_from_scene_query {
      geometry: polygon(event_path());
      policy: intersects;
      marks: [overview.points, detail.points];
      fields: [{ id: 'x'; datum: 'x'; field: "x"; }];
    }
    set cursor = crosshair;
  }
}
"#,
        )],
        "events.avenger",
    )
    .await;
    let resolved = resolve_project(&project, &bootstrap_schema())
        .result
        .unwrap();
    let event = &resolved.files.values().next().unwrap().roots[0].children[5];
    let binding = event.event_binding.as_ref().unwrap();
    assert_eq!(binding.targets.len(), 2);
    assert!(matches!(
        binding.scope,
        avenger_lang_core::ResolvedEventScope::Subplots { ref targets, .. }
            if targets.len() == 2
    ));
    assert!(matches!(
        binding.surface,
        avenger_lang_core::ResolvedEventSurface::All(_)
    ));
    assert_eq!(event.children.len(), 4, "source action order is retained");
    let lvalue = event.children[0].state_lvalue.as_ref().unwrap();
    assert!(matches!(
        lvalue.route,
        avenger_lang_core::ResolvedActionRoute::Start
    ));
    assert!(lvalue.replacing_scopes);
    assert!(matches!(lvalue.target, ResolvedTarget::Param(_)));
    assert!(matches!(
        event.children[1].state_lvalue.as_ref().unwrap().target,
        ResolvedTarget::Store(_)
    ));
    assert!(matches!(
        event.children[2].state_lvalue.as_ref().unwrap().target,
        ResolvedTarget::Selection(_)
    ));
    let avenger_lang_core::ResolvedValue::Object { properties, .. } =
        &event.children[2].properties["value"]
    else {
        panic!("selection update payload")
    };
    let avenger_lang_core::ResolvedValue::Array(scene_targets) = &properties["marks"] else {
        panic!("typed scene-query mark target list")
    };
    assert_eq!(scene_targets.len(), 2);
    assert!(scene_targets.iter().all(|target| matches!(
        target,
        avenger_lang_core::ResolvedValue::Reference(reference)
            if reference.kind == avenger_lang_core::ast::RefKind::Mark
                && matches!(reference.target, ResolvedTarget::Mark(_))
    )));
    assert!(event.children[3].state_lvalue.is_none());
}

#[tokio::test]
async fn resolve_rejects_invalid_scene_query_mark_targets() {
    let project = project(
        &[(
            "bad_scene_targets.avenger",
            r#"
avenger 1;
chart cartesian as bad_scene_targets {
  selection as picked { empty: none; combine: union; }
  group as panel { mark symbol as points { x: "x"; y: "y"; } }
  on cursor_moved {
    set selection picked = replace_all_from_scene_query {
      geometry: polygon(event_path());
      policy: intersects;
      marks: [panel.points, panel.points, panel, 1 + 2];
    }
  }
}
"#,
        )],
        "bad_scene_targets.avenger",
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
    assert!(codes.contains(&"AVENGER-RESOLVE-155"));
    assert!(codes.contains(&"AVENGER-RESOLVE-156"));
    assert!(codes.contains(&"AVENGER-RESOLVE-157"));
}

#[tokio::test]
async fn resolve_rejects_malformed_event_and_between_stream_contracts() {
    let project = project(
        &[(
            "bad_events.avenger",
            r#"
avenger 1;
chart cartesian as bad_events {
  on cursor_moved {
    throttle_ms: 'fast';
    consume: maybe;
    mode: eventual;
    between: {
      start: invented { bogus: true; }
      end: mouse_up { scope: plot; }
    }
  }
}
"#,
        )],
        "bad_events.avenger",
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
    assert!(codes.contains(&"AVENGER-RESOLVE-146"));
    assert!(codes.contains(&"AVENGER-RESOLVE-147"));
    assert!(codes.contains(&"AVENGER-RESOLVE-148"));
    assert!(codes.contains(&"AVENGER-RESOLVE-105"));
}

fn contains_invalid(declaration: &avenger_lang_core::ResolvedDeclaration) -> bool {
    declaration
        .properties
        .values()
        .any(|value| matches!(value, avenger_lang_core::ResolvedValue::Invalid))
        || declaration.children.iter().any(contains_invalid)
}
