use std::{fs, path::PathBuf};

use avenger_chart_schema::{NativeKindKey, NativeKindNamespace, NativeSchemaSnapshot};
use avenger_lang_core::{
    ContentVersion, ImportCapabilities, InMemorySourceLoader, LoadedSource, ModuleGraphLoadLimits,
    ModuleGraphLoadRequest, ModuleGraphLoader, ModuleRoot, ResolvedSelectionCombine,
    ResolvedSelectionEmpty, ResolvedTarget, ResolvedValue, SourceOrigin, render_diagnostics,
    resolve_module_graph,
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

async fn project(sources: &[(&str, &str)], root: &str) -> avenger_lang_core::ParsedModuleGraph {
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
            native_modules: Default::default(),
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
async fn resolve_valid_kernel_project_binds_forward_params_and_native_mark() {
    let valid_project = project(
        &[(
            "chart.avenger",
            r#"
avenger 1;
chart cartesian as chart {
  param $lower + 10 as upper;
  param 0 as lower;
  mark group as points {
    mark symbol as dots { x: encoded "x"; y: encoded "y"; }
  }
}
"#,
        )],
        "chart.avenger",
    )
    .await;
    let resolved = resolve_module_graph(&valid_project, &bootstrap_schema())
        .result
        .unwrap();
    assert_eq!(resolved.params.len(), 2);
    assert_eq!(resolved.param_initializer_order.len(), 2);
    assert!(resolved.public_targets.contains_key("chart.points.dots"));
    assert!(
        resolved
            .source_modules
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
    x: encoded "x" { scale: linear; }
    y: encoded "y";
  }
}
"#,
        )],
        "configured_channel.avenger",
    )
    .await;
    let resolved = resolve_module_graph(&valid_project, &bootstrap_schema())
        .result
        .unwrap();
    let chart = resolved
        .source_modules
        .values()
        .flat_map(|file| &file.roots)
        .find(|declaration| declaration.keyword == "chart")
        .unwrap();
    let mark = chart
        .children
        .iter()
        .find(|declaration| declaration.keyword == "mark")
        .unwrap();
    let avenger_lang_core::ResolvedValue::ChannelValue(channel) = &mark.properties["x"] else {
        panic!("configured channel must resolve to normalized channel semantics")
    };
    assert_eq!(
        channel.head.expression.as_ref(),
        &avenger_lang_core::ResolvedValue::Column("x".into())
    );
    assert!(
        matches!(channel.configuration.get("scale"), Some(avenger_lang_core::ResolvedValue::Atom(value)) if value == "linear")
    );
}

#[tokio::test]
async fn resolved_configured_value_normalizes_a_function_head_as_sql() {
    let valid_project = project(
        &[(
            "configured_aggregate_channel.avenger",
            r#"
avenger 1;
chart cartesian {
  data: { values: [{ x: 1.0; y: 2.0; }]; }
  mark subplot {
    x: encoded avg("x") { scale: linear { domain: [0.0, 2.0]; } }
    y: encoded avg("y");
    plot polar { mark symbol { r: direct 1.0; theta: direct 0.0; } }
  }
}

"#,
        )],
        "configured_aggregate_channel.avenger",
    )
    .await;
    let resolved = resolve_module_graph(&valid_project, &bootstrap_schema())
        .result
        .unwrap();
    let chart = resolved
        .source_modules
        .values()
        .flat_map(|file| &file.roots)
        .find(|declaration| declaration.keyword == "chart")
        .unwrap();
    let mark = chart
        .children
        .iter()
        .find(|declaration| declaration.keyword == "mark")
        .unwrap();
    let avenger_lang_core::ResolvedValue::ChannelValue(channel) = &mark.properties["x"] else {
        panic!("configured aggregate channel must resolve to normalized channel semantics")
    };
    assert!(matches!(
        channel.head.expression.as_ref(),
        avenger_lang_core::ResolvedValue::Expression(_)
    ));
}

#[tokio::test]
async fn conditional_channels_normalize_modes_order_and_effective_fallback() {
    let source = r#"avenger 1;
chart cartesian {
  mark symbol as points {
    fill: encoded "category" {
      when { predicate: "selected"; direct: '#2563eb'; }
      when { predicate: "alert"; encoded: "alert_category"; }
      otherwise: { direct: '#94a3b8'; }
      legend: { title: 'Category'; }
    }
  }
}
"#;
    let project = project(&[("chart.avenger", source)], "chart.avenger").await;
    let resolved = resolve_module_graph(&project, &bootstrap_schema())
        .result
        .unwrap();
    let chart = &resolved.source_modules.values().next().unwrap().roots[0];
    let mark = &chart.children[0];
    let ResolvedValue::ChannelValue(channel) = &mark.properties["fill"] else {
        panic!("channel must be normalized")
    };
    assert_eq!(
        channel.head.mode,
        avenger_lang_core::ast::ChannelMode::Encoded
    );
    assert_eq!(channel.conditions.len(), 2);
    assert_eq!(
        channel.conditions[0].branch.mode,
        avenger_lang_core::ast::ChannelMode::Direct
    );
    assert_eq!(
        channel.conditions[1].branch.mode,
        avenger_lang_core::ast::ChannelMode::Encoded
    );
    assert_eq!(
        channel.otherwise.as_ref().unwrap().mode,
        avenger_lang_core::ast::ChannelMode::Direct
    );
    assert_eq!(
        channel.effective_fallback().mode,
        avenger_lang_core::ast::ChannelMode::Direct
    );
    assert!(channel.configuration.contains_key("legend"));
}

#[tokio::test]
async fn channel_mode_contract_reports_focused_diagnostics() {
    for (body, expected) in [
        ("mark symbol { x: 1.0; }", "AVENGER-RESOLVE-195"),
        (
            "mark symbol { x: direct 1.0 { scale: linear; } }",
            "AVENGER-RESOLVE-198",
        ),
        (
            "mark symbol { visible: direct true; }",
            "AVENGER-RESOLVE-202",
        ),
        (
            "mark box_plot { x: none; y: encoded 1.0; }",
            "AVENGER-RESOLVE-194",
        ),
        (
            "mark symbol { fill: encoded 'a' { when { predicate: true; encoded: 'a'; direct: 'b'; } } }",
            "AVENGER-RESOLVE-200",
        ),
        (
            "mark symbol { fill: encoded 'a' { when { predicate: true; scaled: 'a'; } } }",
            "AVENGER-RESOLVE-201",
        ),
        (
            "mark symbol { fill: encoded 'a' { otherwise: { value: 'a'; } } }",
            "AVENGER-RESOLVE-201",
        ),
    ] {
        let source = format!("avenger 1; chart cartesian {{ {body} }}");
        let project = project(&[("chart.avenger", &source)], "chart.avenger").await;
        let failure = resolve_module_graph(&project, &bootstrap_schema())
            .result
            .unwrap_err();
        assert!(
            failure
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.as_str() == expected),
            "{body}: {:?}",
            failure.diagnostics
        );
    }
}

#[tokio::test]
async fn typed_object_does_not_satisfy_a_configured_expression_shape() {
    let configured = project(
        &[(
            "configured_facet.avenger",
            r#"
avenger 1;
chart facet_wrap {
  facet: "region" { slots: shared; }
  cell cartesian { mark symbol { x: encoded "x"; y: encoded "y"; } }
}
"#,
        )],
        "configured_facet.avenger",
    )
    .await;
    assert!(
        resolve_module_graph(&configured, &bootstrap_schema())
            .result
            .is_ok()
    );

    let typed = project(
        &[(
            "typed_facet.avenger",
            r#"
avenger 1;
chart facet_wrap {
  facet: linear { slots: shared; }
  cell cartesian { mark symbol { x: encoded "x"; y: encoded "y"; } }
}
"#,
        )],
        "typed_facet.avenger",
    )
    .await;
    let failure = resolve_module_graph(&typed, &bootstrap_schema())
        .result
        .unwrap_err();
    assert!(
        failure
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-050")
    );
}

#[tokio::test]
async fn resolve_shared_param_store_namespace_shadows_without_kind_fallback() {
    for declarations in [
        "param 0 as state; store as state { field int64 id; }",
        "param 0 as state; selection as state {}",
        "store as state { field int64 id; } selection as state {}",
    ] {
        let source = format!("avenger 1; chart cartesian {{ {declarations} }}");
        let project = project(&[("duplicate.avenger", &source)], "duplicate.avenger").await;
        let failure = resolve_module_graph(&project, &bootstrap_schema())
            .result
            .unwrap_err();
        assert!(
            failure
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-010"),
            "{declarations}: {:?}",
            failure.diagnostics
        );
    }

    let shadowed = project(
        &[(
            "shadowed.avenger",
            r#"avenger 1;
chart cartesian {
  param 1 as state;
  mark group {
    store as state { field int64 id; }
    mark symbol { x: encoded "x"; y: encoded "y"; size: encoded $state; }
  }
}"#,
        )],
        "shadowed.avenger",
    )
    .await;
    let failure = resolve_module_graph(&shadowed, &bootstrap_schema())
        .result
        .unwrap_err();
    assert!(
        failure
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-064"),
        "nearest wrong-category state must stop lookup: {:?}",
        failure.diagnostics
    );

    let cursor = project(
        &[(
            "cursor.avenger",
            "avenger 1; chart cartesian { param 0 as cursor; }",
        )],
        "cursor.avenger",
    )
    .await;
    let failure = resolve_module_graph(&cursor, &bootstrap_schema())
        .result
        .unwrap_err();
    assert!(
        failure
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("cursor")),
        "{:?}",
        failure.diagnostics
    );
}

#[tokio::test]
async fn resolve_namespace_matrix_unifies_state_but_separates_structure_and_events() {
    let valid = project(
        &[(
            "namespaces.avenger",
            r#"
avenger 1;
chart cartesian as namespaces {
  private param 0 as shared_value;
  private selection as shared {}
  mark symbol as shared { x: encoded "x"; y: encoded "y"; }
  on click as shared {
    target: mark shared;
    set shared = clear;
    set shared_value = $shared_value + 1;
  }
}
"#,
        )],
        "namespaces.avenger",
    )
    .await;
    let resolved = resolve_module_graph(&valid, &bootstrap_schema())
        .result
        .unwrap();
    assert_eq!(resolved.params.len(), 1);
    assert_eq!(resolved.selections.len(), 1);
    let event = &resolved.source_modules.values().next().unwrap().roots[0].children[3];
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
  mark symbol as points { x: encoded "x"; y: encoded "y"; }
  mark symbol as points { x: encoded "x"; y: encoded "y"; }
  on click as handler {}
  on click as handler {}
}
"#,
        )],
        "duplicate_namespaces.avenger",
    )
    .await;
    let failure = resolve_module_graph(&invalid, &bootstrap_schema())
        .result
        .unwrap_err();
    let codes = failure
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect::<Vec<_>>();
    assert!(codes.contains(&"AVENGER-RESOLVE-010"));
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
  param $first.value as selected;
}
"#,
        )],
        "widgets.avenger",
    )
    .await;
    let resolved = resolve_module_graph(&project, &bootstrap_schema())
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
  param $first.x_domain as domain;
}
"#,
        )],
        "tools.avenger",
    )
    .await;
    let resolved = resolve_module_graph(&project, &bootstrap_schema())
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
async fn resolve_widget_existing_state_parts_and_defers_typed_requirements() {
    let valid = project(
        &[(
            "chart.avenger",
            r#"
avenger 1;
chart cartesian as chart {
  param 'a' as selected;
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
    let resolved = resolve_module_graph(&valid, &bootstrap_schema())
        .result
        .unwrap();
    assert_eq!(
        resolved.params.len(),
        1,
        "existing state is not regenerated"
    );
    assert!(matches!(
        resolved.public_targets.get("chart.choice.control"),
        Some(ResolvedTarget::Part { alias, .. }) if alias == "control"
    ));
    let widget = &resolved.source_modules.values().next().unwrap().roots[0].children[1];
    assert!(matches!(
        widget.exports.get("value"),
        Some(ResolvedTarget::Param(id)) if resolved.params.contains_key(id)
    ));
    assert_eq!(resolved.param_type_requirements.len(), 1);
    assert!(matches!(
        &resolved.param_type_requirements[0].target,
        ResolvedTarget::Param(id) if resolved.params.contains_key(id)
    ));

    let invalid = project(
        &[(
            "invalid.avenger",
            r#"
avenger 1;
chart cartesian {
  param 1 as wrong;
  store as wrong_kind { field utf8 id; }
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
  param $choice.value as also_wrong;
}
"#,
        )],
        "invalid.avenger",
    )
    .await;
    let failure = resolve_module_graph(&invalid, &bootstrap_schema())
        .result
        .unwrap_err();
    let codes = failure
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect::<Vec<_>>();
    assert!(codes.contains(&"AVENGER-RESOLVE-064"));
}

#[tokio::test]
async fn resolve_native_schema_contracts_cover_placement_body_children_and_coordinates() {
    let invalid_shape_and_placement = project(
        &[(
            "contracts.avenger",
            r#"
avenger 1;
chart cartesian as contracts {
  mark group {
    widget radio_button_list as nested {
      data: { values: [{ value: 'a'; label: 'A'; }]; }
      position: top;
    }
  }
  widget radio_button_list as expanded {
    data: { values: [{ value: 'a'; label: 'A'; }]; }
    position: top;
    mark symbol { x: encoded "x"; y: encoded "y"; }
  }
  mark symbol { x: encoded not_sql(); y: encoded "y";
    view cartesian {}
    view cartesian {}
  }
}
"#,
        )],
        "contracts.avenger",
    )
    .await;
    let failure = resolve_module_graph(&invalid_shape_and_placement, &bootstrap_schema())
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
    let failure = resolve_module_graph(&polar, &registry).result.unwrap_err();
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
  mark group as controls {
    private param 7 as internal;
    export internal as value;
  }
  private mark group as implementation {
    public mark symbol as visible { x: encoded "x"; y: encoded "y"; }
  }
  param $controls.value as copied;
}
"#,
        )],
        "chart.avenger",
    )
    .await;
    let resolved = resolve_module_graph(&valid, &bootstrap_schema())
        .result
        .unwrap();
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
  param 1 as data;
  mark group as nested {
    store as data { field int64 id; }
    param $data as copy;
  }
}
"#,
        )],
        "shadow.avenger",
    )
    .await;
    let failure = resolve_module_graph(&invalid, &bootstrap_schema())
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
  mark group as component {
    private mark symbol as first { x: encoded "x"; y: encoded "y"; }
    private mark symbol as second { x: encoded "x"; y: encoded "y"; }
    export first as glyph;
    export second as glyph;
  }
}
"#,
        )],
        "exports.avenger",
    )
    .await;
    let failure = resolve_module_graph(&duplicate_exports, &bootstrap_schema())
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
    let resolved = resolve_module_graph(&valid, &bootstrap_schema())
        .result
        .unwrap();
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
    let failure = resolve_module_graph(&invalid, &bootstrap_schema())
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
  param $b as a;
  param $a as b;
  mark symbol { x: encoded "x"; bogus: 1; }
  resource tiles as missing_url { kind: xyz; }
}
"#,
        )],
        "invalid.avenger",
    )
    .await;
    let failure = resolve_module_graph(&project, &bootstrap_schema())
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
async fn resolve_validates_registered_adjustment_schemas_and_binders() {
    let project = project(
        &[(
            "invalid_adjustments.avenger",
            r#"
avenger 1;
chart cartesian {
  mark symbol {
    x: encoded "x";
    y: encoded "y";
    adjust expr { x: item.channel.x; }
    adjust nudge as nudged { dx: 1.0; }
    adjust jitter {
      bogus: true;
      apply: { x: jittered.x; }
    }
  }
}
"#,
        )],
        "invalid_adjustments.avenger",
    )
    .await;
    let failure = resolve_module_graph(&project, &bootstrap_schema())
        .result
        .unwrap_err();
    let codes = failure
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect::<Vec<_>>();

    assert!(codes.contains(&"AVENGER-RESOLVE-021"));
    assert!(codes.contains(&"AVENGER-RESOLVE-022"));
    assert!(codes.contains(&"AVENGER-RESOLVE-168"));
    assert!(!codes.contains(&"AVENGER-RESOLVE-020"));
}

#[tokio::test]
async fn resolve_imported_definition_exports_are_typed_and_instance_scoped() {
    let project = project(
        &[
            (
                "chart.avenger",
                r#"
avenger 1;
import { controller } from 'controller.avenger';
chart cartesian as chart {
  tool controller as first {}
  tool controller as second {}
  param $first.value as selected;
}
"#,
            ),
            (
                "controller.avenger",
                r#"
avenger 1;
export define tool controller {
  param 0 as threshold;
  export threshold as value;
}
"#,
            ),
        ],
        "chart.avenger",
    )
    .await;
    let resolved = resolve_module_graph(&project, &bootstrap_schema())
        .result
        .unwrap();
    let chart = resolved
        .source_modules
        .values()
        .find(|file| file.roots.iter().any(|root| root.keyword == "chart"))
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
import { point_pair } from 'point_pair.avenger';
chart cartesian as chart {
  mark point_pair as pair { horizontal_value: "x"; vertical_value: "y"; }
  on click { target: mark pair.point; }
}
"#,
            ),
            (
                "point_pair.avenger",
                r#"
avenger 1;
export define mark point_pair {
  slot channel horizontal { default: x; }
  slot channel vertical { default: y; }
  slot expr horizontal_value;
  slot expr vertical_value;
  mark group as body {
    mark symbol as point {
      horizontal: encoded horizontal_value;
      vertical: encoded vertical_value;
      size: encoded channel.horizontal + 1;
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
    let resolved = resolve_module_graph(&project, &bootstrap_schema())
        .result
        .unwrap();

    let definition_file = resolved
        .source_modules
        .values()
        .find(|file| file.roots.iter().any(|root| root.keyword == "define"))
        .unwrap();
    let point = &definition_file.roots[0].children[4].children[0];
    assert!(matches!(
        point.property_channels.get("horizontal"),
        Some(ResolvedTarget::DefinitionChannel { name, .. }) if name == "horizontal"
    ));
    let Some(avenger_lang_core::ResolvedValue::ChannelValue(horizontal)) =
        point.properties.get("horizontal")
    else {
        panic!("definition channel head")
    };
    assert!(matches!(
        &*horizontal.head.expression,
        avenger_lang_core::ResolvedValue::DefinitionArgument(
            ResolvedTarget::DefinitionSlot { name, .. }
        ) if name == "horizontal_value"
    ));
    let avenger_lang_core::ResolvedValue::ChannelValue(size) = &point.properties["size"] else {
        panic!("definition channel helper expression")
    };
    let avenger_lang_core::ResolvedValue::Expression(size) = &*size.head.expression else {
        panic!("definition channel helper expression")
    };
    assert!(matches!(
        &size.contextual_accesses[0].kind,
        avenger_lang_core::ResolvedContextualAccessKind::MarkChannel {
            channel: avenger_lang_core::ResolvedChannelMember::Definition {
                target: ResolvedTarget::DefinitionChannel { name, .. },
                family_suffix,
                ..
            },
        } if name == "horizontal" && family_suffix.is_empty()
    ));

    let chart_file = resolved
        .source_modules
        .values()
        .find(|file| file.roots.iter().any(|root| root.keyword == "chart"))
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
                "avenger 1; import { high } from 'high.avenger'; chart cartesian as chart { tool high as high {} }",
            ),
            (
                "high.avenger",
                r#"
avenger 1;
import { base } from 'base.avenger';
export define tool high { tool base as inner {} }
"#,
            ),
            (
                "base.avenger",
                "avenger 1; export define tool base { param 0 as state; }",
            ),
        ],
        "chart.avenger",
    )
    .await;
    let resolved = resolve_module_graph(&project, &bootstrap_schema())
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
import { project } from 'project.avenger';
chart cartesian as chart {
  transform project as projected { measure: "x"; }
  mark symbol { x: encoded projected.result; y: encoded "y"; }
}
"#,
            ),
            (
                "project.avenger",
                r#"
avenger 1;
export define transform project {
  slot expr measure;
  output measure as result;
}
"#,
            ),
        ],
        "chart.avenger",
    )
    .await;
    let resolved = resolve_module_graph(&valid_project, &bootstrap_schema())
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
        .source_modules
        .values()
        .find(|file| file.roots.iter().any(|root| root.keyword == "chart"))
        .unwrap();
    let transform = &chart.roots[0].children[0];
    assert!(transform.transform_outputs.contains_key("result"));
    let avenger_lang_core::ResolvedValue::ChannelValue(channel) =
        &chart.roots[0].children[1].properties["x"]
    else {
        panic!("definition output expression")
    };
    let avenger_lang_core::ResolvedValue::Expression(expression) = channel.head.expression.as_ref()
    else {
        panic!("definition output channel expression")
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
                "avenger 1; import { project } from 'project.avenger'; chart cartesian as chart { transform project { measure: \"x\"; } }",
            ),
            (
                "project.avenger",
                "avenger 1; export define transform project { slot expr measure; output measure as result; }",
            ),
        ],
        "chart.avenger",
    )
    .await;
    let failure = resolve_module_graph(&unbound, &bootstrap_schema())
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
  mark group as owner {
    view cartesian as viewport {
      x_domain: "x";
      y_domain: "y";
      transform filter as visible { predicate: viewport.x.pixels > 0; }
      mark symbol as points { x: encoded "x"; y: encoded "y"; }
    }
  }
}
"#,
        )],
        "chart.avenger",
    )
    .await;
    let resolved = resolve_module_graph(&valid, &bootstrap_schema())
        .result
        .unwrap();
    assert!(!resolved.public_targets.contains_key("chart.owner.viewport"));
    let view = &resolved.source_modules.values().next().unwrap().roots[0].children[0].children[0];
    let predicate = view.children[0].properties.get("predicate").unwrap();
    assert!(matches!(
        predicate,
        avenger_lang_core::ResolvedValue::Expression(expression)
            if matches!(
                &expression.contextual_accesses[0].kind,
                avenger_lang_core::ResolvedContextualAccessKind::ViewField {
                    target: ResolvedTarget::Declaration(_),
                    ..
                }
            )
    ));

    let outside = project(
        &[(
            "outside.avenger",
            r#"
avenger 1;
chart cartesian {
  mark group {
    view cartesian as viewport {
      x_domain: "x";
      y_domain: "y";
      mark symbol { x: encoded "x"; y: encoded "y"; }
    }
    mark symbol { x: encoded viewport.x.pixels + 0; y: encoded "y"; }
  }
}
"#,
        )],
        "outside.avenger",
    )
    .await;
    let failure = resolve_module_graph(&outside, &bootstrap_schema())
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
  mark group {
    public view cartesian as reusable {
      export child;
      view cartesian as nested {
        mark symbol as child { x: encoded "x"; y: encoded "y"; }
      }
    }
  }
}
"#,
        )],
        "invalid_views.avenger",
    )
    .await;
    let failure = resolve_module_graph(&project, &bootstrap_schema())
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
        let resolved = resolve_module_graph(&project, &bootstrap_schema())
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
  param true as unrelated;
  mark symbol as stable_mark { x: encoded "x"; y: encoded "y"; }
  widget radio_button_list as stable_widget {
    data: { values: [{ value: 1; label: 'one'; }]; }
    position: top;
  }
  param 1 as stable_param;
}
"#,
    )
    .await;
    let second = identities(
        r#"
avenger 1;
chart cartesian as stable {
  param 1 as stable_param;
  widget radio_button_list as stable_widget {
    data: { values: [{ value: 1; label: 'one'; }]; }
    position: top;
  }
  mark symbol as stable_mark { x: encoded "x"; y: encoded "y"; }
  param true as unrelated;
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
import { broken } from 'broken.avenger';
chart cartesian as chart { mark broken { value: "x"; } }
"#,
            ),
            (
                "broken.avenger",
                r#"
avenger 1;
export define mark broken {
  slot expr later;
  slot enum mode { values: [a, b]; default: missing; }
  mark group {}
}
"#,
            ),
        ],
        "chart.avenger",
    )
    .await;
    let failure = resolve_module_graph(&project, &bootstrap_schema())
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
  mark group as composite {
    component_kind: point_pair;
    mark symbol as point { x: encoded "x"; y: encoded "y"; }
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
    let resolved = resolve_module_graph(&project, &bootstrap_schema())
        .result
        .unwrap();
    let root = &resolved.source_modules.values().next().unwrap().roots[0];
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
async fn resolve_legend_overlay_uses_private_cartesian_mark_pipeline() {
    let project = project(
        &[
            (
                "chart.avenger",
                r#"
avenger 1;
import { band } from 'band.avenger';
chart cartesian as chart {
  data: { values: [{ x: 1.0; y: 2.0; value: 5.0; }]; }
  mark symbol as points {
    x: encoded "x";
    y: encoded "y";
    fill: encoded "value" {
      legend: {
        overlay: {
          mark group as thresholds {
            data: { values: [{ lo: 2.0; hi: 7.0; }]; }
            mark rect as band {
              x: encoded 0.0;
              x2: encoded 1.0;
              y: encoded "lo";
              y2: encoded "hi";
              fill: direct 'rgba(37, 99, 235, 0.20)';
            }
          }
          mark band as imported_band {}
        }
      }
    }
  }
}
"#,
            ),
            (
                "band.avenger",
                r#"
avenger 1;
export define mark band {
  mark rect {
    x: encoded 0.0;
    x2: encoded 1.0;
    y: encoded 3.0;
    y2: encoded 4.0;
    fill: direct 'rgba(220, 38, 38, 0.20)';
  }
}
"#,
            ),
        ],
        "chart.avenger",
    )
    .await;
    let resolved = resolve_module_graph(&project, &bootstrap_schema())
        .result
        .unwrap();
    let chart = &resolved
        .source_modules
        .values()
        .find(|file| file.roots.iter().any(|root| root.keyword == "chart"))
        .unwrap()
        .roots[0];
    let mark = &chart.children[0];
    let ResolvedValue::ChannelValue(fill) = &mark.properties["fill"] else {
        panic!("configured fill channel")
    };
    let ResolvedValue::Object {
        properties: legend, ..
    } = &fill.configuration["legend"]
    else {
        panic!("legend block")
    };
    let ResolvedValue::Object {
        children: overlay, ..
    } = &legend["overlay"]
    else {
        panic!("overlay mark block")
    };
    assert_eq!(overlay.len(), 2);
    let group = &overlay[0];
    assert_eq!(group.keyword, "mark");
    assert_eq!(group.kind.as_deref(), Some("group"));
    assert!(group.public_path.is_none());
    assert!(matches!(
        group.runtime_target,
        Some(ResolvedTarget::Mark(_))
    ));
    assert_eq!(group.children[0].keyword, "mark");
    assert!(group.children[0].public_path.is_none());
    let imported = &overlay[1];
    assert_eq!(imported.keyword, "mark");
    assert_eq!(imported.kind.as_deref(), Some("band"));
    assert!(imported.public_path.is_none());
}

#[tokio::test]
async fn resolve_legend_overlay_rejects_empty_properties_and_non_mark_children() {
    for (overlay, code) in [
        ("overlay: {}", "AVENGER-RESOLVE-179"),
        (
            "overlay: { data: { values: []; } mark rect {} }",
            "AVENGER-RESOLVE-178",
        ),
        (
            "overlay: { transform filter { predicate: true; } }",
            "AVENGER-RESOLVE-178",
        ),
    ] {
        let source = format!(
            "avenger 1; chart cartesian {{ mark symbol {{ fill: encoded 1.0 {{ legend: {{ {overlay} }} }} }} }}"
        );
        let project = project(&[("chart.avenger", &source)], "chart.avenger").await;
        let failure = resolve_module_graph(&project, &bootstrap_schema())
            .result
            .unwrap_err();
        assert!(
            failure
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.as_str() == code),
            "{overlay}: {:?}",
            failure
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code.as_str())
                .collect::<Vec<_>>()
        );
    }
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
    expressions: sum("amount") AS total;
  }
  mark symbol as points { x: encoded stats.total; y: encoded "y"; }
}
"#,
        )],
        "pipeline.avenger",
    )
    .await;
    let resolved = resolve_module_graph(&valid_project, &bootstrap_schema())
        .result
        .unwrap();
    let root = &resolved.source_modules.values().next().unwrap().roots[0];
    let transform = &root.children[0];
    assert!(transform.transform_outputs.contains_key("total"));
    let mark = &root.children[1];
    let avenger_lang_core::ResolvedValue::ChannelValue(channel) = &mark.properties["x"] else {
        panic!("x expression")
    };
    let avenger_lang_core::ResolvedValue::Expression(expression) = channel.head.expression.as_ref()
    else {
        panic!("x channel expression")
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
  mark symbol { x: encoded stats.total; y: encoded "y"; }
  transform aggregate as stats {
    expressions: sum("amount") AS total;
  }
}
"#,
        )],
        "future.avenger",
    )
    .await;
    let failure = resolve_module_graph(&invalid, &bootstrap_schema())
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
async fn resolve_projection_policies_reject_ambiguous_output_names() {
    for expressions in [
        "sum(\"amount\")",
        "sum(\"amount\") AS \"quoted\"",
        "sum(\"amount\") AS total, avg(\"amount\") AS total",
    ] {
        let source = format!(
            r#"avenger 1;
chart cartesian {{
  transform aggregate as stats {{ expressions: {expressions}; }}
}}"#
        );
        let project = project(&[("chart.avenger", &source)], "chart.avenger").await;
        let failure = resolve_module_graph(&project, &bootstrap_schema())
            .result
            .unwrap_err();
        assert!(
            failure.diagnostics.iter().any(|diagnostic| matches!(
                diagnostic.code.as_str(),
                "AVENGER-RESOLVE-050" | "AVENGER-RESOLVE-150"
            )),
            "{expressions}: {:#?}",
            failure.diagnostics
        );
    }

    let valid = project(
        &[(
            "chart.avenger",
            r#"
avenger 1;
chart cartesian {
  transform select as projected {
    expressions: "category", "amount" * 2 AS doubled;
  }
  mark symbol { x: encoded "category"; y: encoded projected.doubled; }
}
"#,
        )],
        "chart.avenger",
    )
    .await;
    let resolved = resolve_module_graph(&valid, &bootstrap_schema())
        .result
        .unwrap();
    let transform = &resolved.source_modules.values().next().unwrap().roots[0].children[0];
    assert_eq!(
        transform
            .transform_outputs
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        ["doubled"]
    );

    let invalid = project(
        &[(
            "chart.avenger",
            r#"
avenger 1;
chart cartesian {
  transform select { expressions: "amount" * 2; }
}
"#,
        )],
        "chart.avenger",
    )
    .await;
    assert!(
        resolve_module_graph(&invalid, &bootstrap_schema())
            .result
            .is_err()
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
    output second.final as final;
    transform aggregate as first {
      expressions: sum("amount") AS total;
    }
    transform aggregate as second {
      expressions: sum(first.total) AS final;
    }
  }
  mark symbol as points { x: encoded summarized.final; y: encoded "y"; }
}
"#,
        )],
        "pipeline.avenger",
    )
    .await;
    let resolved = resolve_module_graph(&valid, &bootstrap_schema())
        .result
        .unwrap();
    let pipeline = &resolved.source_modules.values().next().unwrap().roots[0].children[0];
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
    transform aggregate { expressions: sum("x") AS total; }
    output "total" as total;
    output "total" as total;
  }
}
"#,
        )],
        "duplicate.avenger",
    )
    .await;
    let failure = resolve_module_graph(&invalid, &bootstrap_schema())
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
async fn transform_definition_outputs_may_forward_reference_child_stages() {
    let valid = project(
        &[(
            "definition.avenger",
            r#"
avenger 1;
define transform binned {
  slot expr field;
  output bins.start as start;
  transform bin as bins {
    field: field;
    maxbins: 10;
  }
}
"#,
        )],
        "definition.avenger",
    )
    .await;
    let resolved = resolve_module_graph(&valid, &bootstrap_schema())
        .result
        .unwrap();
    let definition = &resolved.source_modules.values().next().unwrap().roots[0];
    let output = definition
        .children
        .iter()
        .find(|child| child.keyword == "output")
        .expect("output declaration");
    let avenger_lang_core::ResolvedValue::Expression(source) =
        output.properties.values().next().expect("output source")
    else {
        panic!("definition output source")
    };
    assert!(matches!(
        source.references.as_slice(),
        [avenger_lang_core::ResolvedSqlReference {
            target: ResolvedTarget::Output(_),
            ..
        }]
    ));
}

#[tokio::test]
async fn resolve_table_dag_orders_relations_and_reports_cycles() {
    let valid_project = project(
        &[
            (
                "tables.avenger",
                "avenger 1; import { vega } from 'data.avenger'; chart cartesian as tables {}",
            ),
            (
                "data.avenger",
                r#"
avenger 1;
export schema tables as vega {
  table csv as base { path: 'base.csv'; }
  table sql as derived { sql: SELECT * FROM vega.base; }
}
"#,
            ),
        ],
        "tables.avenger",
    )
    .await;
    let resolved = resolve_module_graph(&valid_project, &bootstrap_schema())
        .result
        .unwrap();
    assert_eq!(resolved.table_order.len(), 2);

    let cyclic = project(
        &[
            (
                "cycle-root.avenger",
                "avenger 1; import { vega } from 'cycle.avenger'; chart cartesian as cycle {}",
            ),
            (
                "cycle.avenger",
                r#"
avenger 1;
export schema tables as vega {
  table sql as a { sql: SELECT * FROM vega.b; }
  table sql as b { sql: SELECT * FROM vega.a; }
}
"#,
            ),
        ],
        "cycle-root.avenger",
    )
    .await;
    let failure = resolve_module_graph(&cyclic, &bootstrap_schema())
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
  param 1 as minimum;
  store as rows { field int64 id; }
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
    let resolved = resolve_module_graph(&valid, &bootstrap_schema())
        .result
        .unwrap();
    let query =
        &resolved.source_modules.values().next().unwrap().roots[0].children[2].properties["query"];
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
  param 1 as scalar;
  store as relation { field int64 id; }
  transform sql {
    query:
      SELECT * FROM $scalar;
  }
  mark symbol { x: encoded $relation + 1; y: encoded "y"; }
}
"#,
        )],
        "wrong_bindings.avenger",
    )
    .await;
    let failure = resolve_module_graph(&invalid, &bootstrap_schema())
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
async fn resolve_selection_binding_as_current_row_boolean_predicate() {
    let valid = project(
        &[(
            "selection_predicate.avenger",
            r#"
avenger 1;
chart cartesian as selection_predicate {
  data: { values: [{x: 1.0; y: 2.0;}]; }
  selection as picked { empty: none; }
  mark symbol as points {
    x: encoded "x";
    y: encoded "y";
    fill: direct '#cbd5e1' {
      when { predicate: $picked; direct: '#2563eb'; }
    }
  }
}
"#,
        )],
        "selection_predicate.avenger",
    )
    .await;
    let resolved = resolve_module_graph(&valid, &bootstrap_schema())
        .result
        .unwrap();
    let chart = &resolved.source_modules.values().next().unwrap().roots[0];
    let mark = chart
        .children
        .iter()
        .find(|child| child.keyword == "mark")
        .unwrap();
    let ResolvedValue::ChannelValue(fill) = &mark.properties["fill"] else {
        panic!("resolved conditional fill channel")
    };
    let predicate = &fill.conditions[0].predicate;
    let binding = match predicate.as_ref() {
        ResolvedValue::Binding(binding) => binding,
        ResolvedValue::Expression(expression) => expression.bindings.first().unwrap(),
        other => panic!("resolved selection predicate binding, got {other:?}"),
    };
    assert_eq!(binding.kind, avenger_lang_core::ast::BindingKind::Selection);
    assert!(matches!(binding.target, ResolvedTarget::Selection(_)));

    let invalid = project(
        &[(
            "selection_query.avenger",
            r#"
avenger 1;
chart cartesian as selection_query {
  data: { values: [{x: 1.0;}]; }
  selection as picked { empty: none; }
  transform sql { query: SELECT * FROM input WHERE $picked; }
}
"#,
        )],
        "selection_query.avenger",
    )
    .await;
    let failure = resolve_module_graph(&invalid, &bootstrap_schema())
        .result
        .unwrap_err();
    assert!(failure.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str() == "AVENGER-RESOLVE-063"
            && diagnostic.message.contains("current data row")
    }));

    let invalid = project(
        &[(
            "selection_initializer.avenger",
            r#"
avenger 1;
chart cartesian as selection_initializer {
  selection as picked { empty: none; }
  param $picked as impossible;
}
"#,
        )],
        "selection_initializer.avenger",
    )
    .await;
    let failure = resolve_module_graph(&invalid, &bootstrap_schema())
        .result
        .unwrap_err();
    assert!(failure.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str() == "AVENGER-RESOLVE-063"
            && diagnostic.message.contains("current data row")
    }));
}

#[tokio::test]
async fn resolve_nested_arrow_state_and_store_rows_at_exact_boundaries() {
    let valid = project(
        &[(
            "state.avenger",
            r#"
avenger 1;
chart cartesian as state {
  param named_struct(
    'position', named_struct('x', CAST(1 AS DOUBLE), 'y', CAST(NULL AS DOUBLE)),
    'labels', ['a', 'b']
  ) as pointer;
  param CASE WHEN false THEN named_struct('x', CAST(0 AS DOUBLE)) ELSE NULL END as empty_pointer;
  store as rows {
    field utf8 id;
    field struct(field(int8, 'count'),field(list(utf8), 'tags')) payload nullable;
    primary_key: [id];
    row { id: 'one'; }
  }
}
"#,
        )],
        "state.avenger",
    )
    .await;
    let resolved = resolve_module_graph(&valid, &bootstrap_schema())
        .result
        .unwrap();
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
    field utf8 __avenger_store_owner;
    field int8 id;
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
    let failure = resolve_module_graph(&invalid, &bootstrap_schema())
        .result
        .unwrap_err();
    let codes = failure
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect::<Vec<_>>();
    assert!(codes.contains(&"AVENGER-RESOLVE-142"));
    assert!(codes.contains(&"AVENGER-RESOLVE-145"));
    assert!(codes.contains(&"AVENGER-RESOLVE-075"));
}

#[tokio::test]
async fn resolve_catalog_table_params_are_owned_row_free_and_self_contained() {
    let valid = project(
        &[
            (
                "chart.avenger",
                "avenger 1; import { local } from 'views.avenger'; chart cartesian as chart {}",
            ),
            (
                "views.avenger",
                r#"
avenger 1;
export schema tables as local {
  table parquet as base { path: 'base.parquet'; }
  table sql as filtered {
    param 5 + 5 as minimum;
    sql: SELECT * FROM local.base WHERE "value" >= $minimum;
  }
}
"#,
            ),
        ],
        "chart.avenger",
    )
    .await;
    let resolved = resolve_module_graph(&valid, &bootstrap_schema())
        .result
        .unwrap();
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
                "avenger 1; import { local } from 'bad.avenger'; chart cartesian as chart {}",
            ),
            (
                "bad.avenger",
                r#"
avenger 1;
export schema tables as local {
  table sql as bad {
    param 1 as first;
    param $first as second;
    sql: VALUES ($second);
  }
}
"#,
            ),
        ],
        "chart.avenger",
    )
    .await;
    let failure = resolve_module_graph(&invalid, &bootstrap_schema())
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
  param 0 as x;
  mark symbol as points { x: encoded "x"; y: encoded "y"; }
  on cursor_moved as drag {
    target: mark points;
    scope: plot;
    surface: plot;
    between: {
      start: mouse_down { filter: $x >= 0; }
      end: mouse_up { filter: $x >= 0; }
    }
    set x at start = $x@start + event.facet[1];
    set cursor = 'crosshair';
  }
}
"#,
        )],
        "events.avenger",
    )
    .await;
    let resolved = resolve_module_graph(&valid_project, &bootstrap_schema())
        .result
        .unwrap();
    let event = &resolved.source_modules.values().next().unwrap().roots[0].children[2];
    assert!(event.migration_key.is_some());
    assert!(event.public_path.is_none());
    assert_eq!(event.children.len(), 2);
    let avenger_lang_core::ResolvedValue::Expression(value) =
        &event.children[0].properties["value"]
    else {
        panic!("param action expression")
    };
    assert!(matches!(
        &value.contextual_accesses[0].kind,
        avenger_lang_core::ResolvedContextualAccessKind::EventFacet { one_based_index: 1 }
    ));

    let invalid = project(
        &[(
            "bad_events.avenger",
            r#"
avenger 1;
chart cartesian as bad_events {
  param 0 as x;
  on cursor_moved as drag {
    between: {
      start: mouse_down { filter: $x@previous > 0; }
      end: mouse_up {}
    }
    set x = 128;
  }
}
"#,
        )],
        "bad_events.avenger",
    )
    .await;
    let failure = resolve_module_graph(&invalid, &bootstrap_schema())
        .result
        .unwrap_err();
    let codes = failure
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect::<Vec<_>>();
    assert!(codes.contains(&"AVENGER-RESOLVE-106"));
}

#[tokio::test]
async fn resolve_reserved_helpers_bind_typed_targets_and_registered_channels() {
    let valid = project(
        &[(
            "helpers.avenger",
            r#"
avenger 1;
chart cartesian as helpers {
  param 0 as cursor_x;
  selection as picked {}
  mark symbol as points { x: encoded channel.y + 1; y: encoded "y"; }
  on click {
    target: mark points;
    filter: selection_contains(picked, datum."id") = true;
    set cursor_x = event.coord.x + 0;
  }
}

"#,
        )],
        "helpers.avenger",
    )
    .await;
    let resolved = resolve_module_graph(&valid, &bootstrap_schema())
        .result
        .unwrap();
    let root = &resolved.source_modules.values().next().unwrap().roots[0];
    let avenger_lang_core::ResolvedValue::ChannelValue(channel) = &root.children[2].properties["x"]
    else {
        panic!("mark channel expression")
    };
    let avenger_lang_core::ResolvedValue::Expression(x) = channel.head.expression.as_ref() else {
        panic!("mark channel SQL expression")
    };
    assert!(matches!(
        x.contextual_accesses.as_slice(),
        [avenger_lang_core::ResolvedContextualAccess {
            kind: avenger_lang_core::ResolvedContextualAccessKind::MarkChannel {
                channel: avenger_lang_core::ResolvedChannelMember::Named { name },
            },
        }] if name == "y"
    ));
    let event = &root.children[3];
    let avenger_lang_core::ResolvedValue::Expression(filter) = &event.properties["filter"] else {
        panic!("event filter expression")
    };
    assert!(filter.helpers.iter().any(|helper| {
        helper.name == "selection_contains"
            && matches!(
                helper.arguments.as_slice(),
                [
                    avenger_lang_core::ResolvedHelperArgument::Target {
                        target: ResolvedTarget::Selection(_),
                        ..
                    },
                    avenger_lang_core::ResolvedHelperArgument::DatumField(field),
                ] if field == "id"
            )
    }));
    assert!(matches!(
        &filter.contextual_accesses[0].kind,
        avenger_lang_core::ResolvedContextualAccessKind::DatumField { field }
            if field == "id"
    ));
    let avenger_lang_core::ResolvedValue::Expression(action) =
        &event.children[0].properties["value"]
    else {
        panic!("event action expression")
    };
    assert!(matches!(
        &action.contextual_accesses[0].kind,
        avenger_lang_core::ResolvedContextualAccessKind::EventCoord { .. }
    ));

    let invalid = project(
        &[(
            "bad_helpers.avenger",
            r#"
avenger 1;
chart cartesian as bad_helpers {
  mark symbol as points { x: encoded channel.missing + 1; y: encoded "y"; }
  on click { target: mark points; filter: event.coord.missing > 0; }
}
"#,
        )],
        "bad_helpers.avenger",
    )
    .await;
    let failure = resolve_module_graph(&invalid, &bootstrap_schema())
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
async fn resolve_datum_fields_are_contextual_and_reject_removed_forms() {
    let valid = project(
        &[(
            "datum.avenger",
            r#"
avenger 1;
chart cartesian as datum_chart {
  data: { values: [{ id: 1; }]; }
  mark symbol as points { x: encoded 1; y: encoded 1; }
  on click {
    target: mark points;
    filter: DATUM."id" = datum."id";
  }
}
"#,
        )],
        "datum.avenger",
    )
    .await;
    let resolved = resolve_module_graph(&valid, &bootstrap_schema())
        .result
        .unwrap();
    let root = &resolved.source_modules.values().next().unwrap().roots[0];
    let avenger_lang_core::ResolvedValue::Expression(filter) =
        &root.children[1].properties["filter"]
    else {
        panic!("event filter expression")
    };
    assert_eq!(filter.sql, r#"datum."id" = datum."id""#);
    assert_eq!(filter.contextual_accesses.len(), 1);
    assert!(matches!(
        &filter.contextual_accesses[0].kind,
        avenger_lang_core::ResolvedContextualAccessKind::DatumField { field }
            if field == "id"
    ));

    for (source, expected) in [
        ("datum('id') IS NOT NULL", "AVENGER-RESOLVE-183"),
        ("datum.id IS NOT NULL", "AVENGER-RESOLVE-186"),
        ("datum IS NOT NULL", "AVENGER-RESOLVE-186"),
        (r#"datum."id".value IS NOT NULL"#, "AVENGER-RESOLVE-186"),
    ] {
        let invalid_source = format!(
            r#"
avenger 1;
chart cartesian as bad_datum {{
  on click {{ filter: {source}; }}
}}
"#
        );
        let invalid = project(
            &[("bad_datum.avenger", invalid_source.as_str())],
            "bad_datum.avenger",
        )
        .await;
        let failure = resolve_module_graph(&invalid, &bootstrap_schema())
            .result
            .unwrap_err();
        assert!(
            failure
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.as_str() == expected),
            "missing {expected} for {source}: {:?}",
            failure.diagnostics
        );
    }

    let outside = project(
        &[(
            "outside.avenger",
            r#"
avenger 1;
chart cartesian as outside {
  mark symbol { x: encoded datum."id"; y: encoded 1; }
}
"#,
        )],
        "outside.avenger",
    )
    .await;
    let failure = resolve_module_graph(&outside, &bootstrap_schema())
        .result
        .unwrap_err();
    assert!(
        failure
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-110")
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
  param 0 as x { sharing: free; }
  store as rows {
    field utf8 id;
    field float64 value nullable;
    primary_key: [id];
  }
  selection as picked { empty: none; combine: union; }
  cell cartesian as overview { mark symbol as points { x: encoded "x"; y: encoded "y"; } }
  cell cartesian as detail { mark symbol as points { x: encoded "x"; y: encoded "y"; } }
  on cursor_moved as drag {
    target: marks [overview.points, detail.points];
    scope: subplots [overview, detail];
    surface: all;
    between: {
      start: mouse_down { target: mark overview.points; filter: $x >= 0; }
      end: mouse_up { filter: $x >= 0; }
    }
    set x at start replacing scopes = event.coord.x;
    set rows = insert_rows { row { id: 'cursor'; value: event.coord.x; } }
    set picked = replace_all_from_scene_query {
      geometry: polygon(event.path);
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
    let resolved = resolve_module_graph(&project, &bootstrap_schema())
        .result
        .unwrap();
    let event = &resolved.source_modules.values().next().unwrap().roots[0].children[5];
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
  mark group as panel { mark symbol as points { x: encoded "x"; y: encoded "y"; } }
  widget radio_button_list as choice {
    data: { values: [{ value: 'a'; label: 'A'; }]; }
    position: top;
  }
  on cursor_moved {
    set picked = replace_all_from_scene_query {
      geometry: polygon(event.path);
      policy: intersects;
      marks: [panel.points, panel.points, choice.container, 1 + 2];
    }
  }
}
"#,
        )],
        "bad_scene_targets.avenger",
    )
    .await;
    let failure = resolve_module_graph(&project, &bootstrap_schema())
        .result
        .unwrap_err();
    let codes = failure
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect::<Vec<_>>();
    assert!(codes.contains(&"AVENGER-RESOLVE-155"), "{codes:?}");
    assert!(codes.contains(&"AVENGER-RESOLVE-156"), "{codes:?}");
    assert!(codes.contains(&"AVENGER-RESOLVE-157"), "{codes:?}");
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
    let failure = resolve_module_graph(&project, &bootstrap_schema())
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
