use avenger_chart_schema::NativeSchemaSnapshot;
use avenger_lang_core::{
    ContentVersion, ImportCapabilities, InMemorySourceLoader, LoadedSource, ProjectLoadLimits,
    ProjectLoadRequest, ProjectLoader, ProjectRoot, ResolvedDeclaration, ResolvedTarget,
    ResolvedValue, SourceOrigin, expand_project, resolve_project,
};

fn bootstrap_schema() -> NativeSchemaSnapshot {
    serde_json::from_str(include_str!(
        "../../avenger-chart-lang-registry/snapshots/bootstrap-schema.json"
    ))
    .unwrap()
}

async fn project(sources: &[(&str, &str)]) -> avenger_lang_core::ParsedProject {
    let loader = InMemorySourceLoader::default();
    for (name, text) in sources {
        loader.insert(LoadedSource::new(
            SourceOrigin::Memory((*name).to_owned()),
            *text,
            ContentVersion::new("definitions-v1"),
        ));
    }
    ProjectLoader::new(&loader)
        .load(ProjectLoadRequest {
            project_root: "/project".into(),
            roots: vec![ProjectRoot::chart(SourceOrigin::Memory(
                "chart.avenger".to_owned(),
            ))],
            capabilities: ImportCapabilities::in_memory("/project"),
            schema_version: "semantic-v1".to_owned(),
            registry_version: "bootstrap".to_owned(),
            limits: ProjectLoadLimits::default(),
        })
        .await
        .result
        .unwrap()
}

#[tokio::test]
async fn definitions_validate_closed_match_block_exposure_parts_and_function_classes() {
    let project = project(&[
        (
            "chart.avenger",
            r#"
avenger 1;
import 'summary.mark.avenger';
import 'rolling.transform.avenger';
chart cartesian as chart {
  transform rolling as rolled { measure: "value"; agg: avg; }
  mark summary as result {
    mode: show;
    measure: rolled.value;
    zindex: 2;
    annotations: { mark text as label { x: "x"; y: "y"; text: 'ok'; } }
    part point { fill: value '#dc2626'; }
  }
}
"#,
        ),
        (
            "summary.mark.avenger",
            r#"
avenger 1;
define mark summary {
  slot expr as measure;
  slot enum as mode { values: [show, hide]; default: show; }
  slot block as annotations { exposes: [point]; default: { } }
  export point;
  mark symbol as point { x: "x"; y: measure; }
  match mode {
    show { annotations; }
    hide { }
  }
}
"#,
        ),
        (
            "rolling.transform.avenger",
            r#"
avenger 1;
define transform rolling {
  slot expr as measure;
  slot function as agg { class: aggregate; default: avg; }
  output value;
  transform sql { query: SELECT *, agg(measure) OVER () AS value FROM input; }
}
"#,
        ),
    ])
    .await;
    resolve_project(&project, &bootstrap_schema())
        .result
        .unwrap();
}

#[tokio::test]
async fn definitions_reject_incomplete_matches_invalid_splices_exposure_data_and_themes() {
    let project = project(&[
        (
            "chart.avenger",
            "avenger 1; import 'broken.mark.avenger'; chart cartesian { mark broken { mode: a; } }",
        ),
        (
            "broken.mark.avenger",
            r#"
avenger 1;
define mark broken {
  slot enum as mode { values: [a, b]; default: a; }
  slot block as content { exposes: [missing]; default: { } }
  group { data: { values: []; } }
  theme css: 'mark { opacity: 0.5; }';
  match mode {
    a { unknown; }
    c { }
  }
}
"#,
        ),
    ])
    .await;
    let failure = resolve_project(&project, &bootstrap_schema())
        .result
        .unwrap_err();
    let codes = failure
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for code in [
        "AVENGER-RESOLVE-153",
        "AVENGER-RESOLVE-155",
        "AVENGER-RESOLVE-156",
        "AVENGER-RESOLVE-157",
        "AVENGER-RESOLVE-162",
        "AVENGER-RESOLVE-163",
    ] {
        assert!(codes.contains(code), "missing diagnostic {code}: {codes:?}");
    }
}

#[tokio::test]
async fn definitions_reject_wrong_function_class_and_anonymous_defined_tool() {
    let project = project(&[
        (
            "chart.avenger",
            r#"
avenger 1;
import 'rolling.transform.avenger';
import 'picker.tool.avenger';
chart cartesian {
  param as state { type: boolean; default: true; }
  mark symbol as points { x: "x"; y: "y"; }
  transform rolling as value { agg: row_number; window: 'wide'; }
  tool picker;
  tool picker as wrong_target { target: $state; }
}
"#,
        ),
        (
            "rolling.transform.avenger",
            r#"
avenger 1;
define transform rolling {
  slot function as agg { class: aggregate; }
  slot number as window;
  output value;
  transform sql { query: SELECT agg("x") AS value FROM input; }
}
"#,
        ),
        (
            "picker.tool.avenger",
            r#"
avenger 1;
define tool picker {
  slot ref as target { kind: mark; }
  on click { target: mark target; set cursor = pointer; }
}
"#,
        ),
    ])
    .await;
    let failure = resolve_project(&project, &bootstrap_schema())
        .result
        .unwrap_err();
    let codes = failure
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(codes.contains("AVENGER-RESOLVE-053"));
    assert!(codes.contains("AVENGER-RESOLVE-164"));
}

#[tokio::test]
async fn definitions_require_explicit_imports() {
    let project = project(&[(
        "chart.avenger",
        "avenger 1; chart cartesian { mark unimported_definition { } }",
    )])
    .await;
    let failure = resolve_project(&project, &bootstrap_schema())
        .result
        .unwrap_err();
    assert!(failure.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str() == "AVENGER-RESOLVE-020"
            && diagnostic
                .primary
                .message
                .contains("no registered or imported mark kind")
    }));
}

#[tokio::test]
async fn definitions_reject_recursive_import_graphs() {
    let loader = InMemorySourceLoader::default()
        .with_source(LoadedSource::new(
            SourceOrigin::Memory("chart.avenger".to_owned()),
            "avenger 1; import 'a.mark.avenger'; chart cartesian { mark a { } }",
            ContentVersion::new("definitions-v1"),
        ))
        .with_source(LoadedSource::new(
            SourceOrigin::Memory("a.mark.avenger".to_owned()),
            "avenger 1; import 'b.mark.avenger'; define mark a { mark b { } }",
            ContentVersion::new("definitions-v1"),
        ))
        .with_source(LoadedSource::new(
            SourceOrigin::Memory("b.mark.avenger".to_owned()),
            "avenger 1; import 'a.mark.avenger'; define mark b { mark a { } }",
            ContentVersion::new("definitions-v1"),
        ));
    let failure = ProjectLoader::new(&loader)
        .load(ProjectLoadRequest {
            project_root: "/project".into(),
            roots: vec![ProjectRoot::chart(SourceOrigin::Memory(
                "chart.avenger".to_owned(),
            ))],
            capabilities: ImportCapabilities::in_memory("/project"),
            schema_version: "semantic-v1".to_owned(),
            registry_version: "bootstrap".to_owned(),
            limits: ProjectLoadLimits::default(),
        })
        .await
        .result
        .unwrap_err();
    assert_eq!(failure.diagnostics[0].code.as_str(), "AVENGER-PROJECT-013");
    assert!(failure.diagnostics[0].trace.len() >= 2);
}

#[tokio::test]
async fn definitions_reject_ambient_capture_private_access_export_collisions_and_missing_binders() {
    let project = project(&[
        (
            "chart.avenger",
            r#"
avenger 1;
import 'capturing.mark.avenger';
import 'private_component.mark.avenger';
import 'colliding.mark.avenger';
import 'output_transform.transform.avenger';
chart cartesian as chart {
  param as ambient { type: boolean; default: true; }
  mark capturing as captured { }
  mark private_component as component { }
  transform output_transform { }
  on click {
    target: mark component.hidden;
    set cursor = pointer;
  }
}
"#,
        ),
        (
            "capturing.mark.avenger",
            r#"
avenger 1;
define mark capturing {
  mark symbol as glyph { x: "x"; y: "y"; visible: $ambient; }
}
"#,
        ),
        (
            "private_component.mark.avenger",
            r#"
avenger 1;
define mark private_component {
  mark symbol as hidden { x: "x"; y: "y"; }
}
"#,
        ),
        (
            "colliding.mark.avenger",
            r#"
avenger 1;
define mark colliding {
  export first as duplicate;
  export second as duplicate;
  mark symbol as first { x: "x"; y: "y"; }
  mark symbol as second { x: "x"; y: "y"; }
}
"#,
        ),
        (
            "output_transform.transform.avenger",
            r#"
avenger 1;
define transform output_transform {
  output value;
  transform sql { query: SELECT *, 1 AS value FROM input; }
}
"#,
        ),
    ])
    .await;
    let failure = resolve_project(&project, &bootstrap_schema())
        .result
        .unwrap_err();
    let codes = failure
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for code in [
        "AVENGER-RESOLVE-006",
        "AVENGER-RESOLVE-028",
        "AVENGER-RESOLVE-066",
    ] {
        assert!(codes.contains(code), "missing diagnostic {code}: {codes:?}");
    }
    assert!(failure.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str() == "AVENGER-RESOLVE-066"
            && diagnostic.primary.message.contains("ambient")
    }));
    assert!(failure.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str() == "AVENGER-RESOLVE-066"
            && diagnostic.primary.message.contains("component.hidden")
    }));
}

#[tokio::test]
async fn definition_block_content_is_validated_at_its_expanded_splice_site() {
    let project = project(&[
        (
            "chart.avenger",
            r#"
avenger 1;
import 'shell.mark.avenger';
chart cartesian {
  mark shell {
    content: {
      param as invalid_here { type: boolean; default: true; }
    }
  }
}
"#,
        ),
        (
            "shell.mark.avenger",
            r#"
avenger 1;
define mark shell {
  slot block as content;
  mark symbol { x: "x"; y: "y"; content; }
}
"#,
        ),
    ])
    .await;
    let resolved = resolve_project(&project, &bootstrap_schema())
        .result
        .expect("caller block resolution is deferred until expansion");
    let expanded = expand_project(&project, &resolved).unwrap();
    let failure = resolve_project(&expanded.project, &bootstrap_schema())
        .result
        .unwrap_err();
    assert!(
        failure
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-040")
    );
}

#[tokio::test]
async fn expansion_inlines_mark_slots_channels_matches_blocks_parts_and_exports() {
    let project = project(&[
        (
            "chart.avenger",
            r#"
avenger 1;
import 'summary.mark.avenger';
chart cartesian as chart {
  mark summary as result {
    band_axis: y;
    value_axis: x;
    measure: "value";
    mode: show;
    annotations: {
      on click {
        target: mark point;
        set cursor = crosshair;
      }
      mark text as label {
        band_axis: "category";
        value_axis: "value";
        text: value 'ok';
      }
    }
    part point { fill: value '#dc2626'; }
  }
  mark summary as hidden {
    band_axis: x;
    value_axis: y;
    measure: "value";
    mode: hide;
    annotations: {
      mark text as hidden_label { x: "x"; y: "y"; text: 'not selected'; }
    }
  }
  widget slider as threshold {
    position: bottom;
    min: 0.0;
    max: 10.0;
  }
}
"#,
        ),
        (
            "summary.mark.avenger",
            r#"
avenger 1;
define mark summary {
  channel band_axis: x;
  channel value_axis: y;
  slot expr as measure;
  slot number as width { default: 0.6; }
  slot number as cap_width { default: width / 2; }
  slot enum as mode { values: [show, hide]; default: show; }
  slot block as annotations { exposes: [point]; default: { } }
  export body.point as point;
  group as body {
    mark symbol as point {
      band_axis: "category";
      value_axis: measure;
      size: cap_width * 100;
    }
    match mode {
      show { annotations; }
      hide { }
    }
  }
}
"#,
        ),
    ])
    .await;
    let resolved = resolve_project(&project, &bootstrap_schema())
        .result
        .unwrap();
    let expanded = expand_project(&project, &resolved).unwrap();
    let chart = project.chart_roots.first().unwrap();
    let text = expanded.texts.get(chart).unwrap();

    assert!(!text.contains("import 'summary.mark.avenger'"));
    assert!(!text.contains("mark summary"));
    assert!(!text.contains("match mode"));
    assert!(!text.contains("annotations;"));
    assert!(text.contains("group as result"), "{text}");
    assert!(text.contains("component_kind: summary;"), "{text}");
    assert!(text.contains("export __av_"), "{text}");
    assert!(text.contains(" as point;"), "{text}");
    assert!(text.contains("private group as __av_"), "{text}");
    assert!(text.contains("y: \"category\";"), "{text}");
    assert!(text.contains("x: \"value\";"), "{text}");
    assert!(text.contains("public mark text as label"), "{text}");
    assert!(!text.contains("hidden_label"), "{text}");
    assert!(text.contains("target: mark __av_"), "{text}");
    assert!(text.contains("fill: value '#dc2626';"), "{text}");
    assert!(text.contains("widget slider as threshold"), "{text}");
    assert!(!expanded.source_map.mappings.is_empty());

    resolve_project(&expanded.project, &bootstrap_schema())
        .result
        .unwrap();
}

#[tokio::test]
async fn expansion_alpha_renames_private_state_without_capturing_caller_block_bindings() {
    let project = project(&[
        (
            "chart.avenger",
            r#"
avenger 1;
import 'shell.mark.avenger';
chart cartesian as chart {
  param as enabled { type: boolean; default: false; }
  mark shell as instance {
    content: {
      mark symbol as caller_mark {
        x: "x";
        y: "y";
        visible: $enabled;
      }
    }
  }
  mark shell as second { content: { } }
}
"#,
        ),
        (
            "shell.mark.avenger",
            r#"
avenger 1;
define mark shell {
  slot block as content { exposes: [inside]; default: { } }
  export inside;
  param as enabled { type: boolean; default: true; }
  mark symbol as inside { x: "x"; y: "y"; visible: $enabled; }
  content;
}
"#,
        ),
    ])
    .await;
    let resolved = resolve_project(&project, &bootstrap_schema())
        .result
        .unwrap();
    let first = expand_project(&project, &resolved).unwrap();
    let second = expand_project(&project, &resolved).unwrap();
    assert_eq!(first.texts, second.texts);
    let chart = project.chart_roots.first().unwrap();
    let text = first.texts.get(chart).unwrap();
    assert!(text.contains("private param as __av_"), "{text}");
    assert!(text.contains("visible: $__av_"), "{text}");
    assert!(text.contains("visible: $enabled;"), "{text}");

    let expanded = resolve_project(&first.project, &bootstrap_schema())
        .result
        .unwrap();
    let root_param = expanded
        .params
        .values()
        .find(|param| param.source_name == "enabled")
        .expect("caller param");
    let internal = expanded
        .params
        .values()
        .filter(|param| {
            param.source_name.starts_with("__av_") && param.source_name.ends_with("enabled")
        })
        .collect::<Vec<_>>();
    assert_eq!(internal.len(), 2);
    assert_ne!(internal[0].id, internal[1].id);
    assert_ne!(internal[0].migration_key, internal[1].migration_key);
    assert!(internal.iter().all(|param| param.migration_key.is_some()));
    assert!(
        internal
            .iter()
            .all(|param| param.definition_local_seed.is_none())
    );
    assert!(internal.iter().all(|param| root_param.id != param.id));

    let caller_mark = expanded
        .files
        .values()
        .flat_map(|file| file.roots.iter())
        .find_map(|root| find_resolved_declaration(root, "caller_mark"))
        .expect("caller-owned block mark");
    assert!(matches!(
        caller_mark.properties.get("visible"),
        Some(ResolvedValue::Binding(binding))
            if binding.target == ResolvedTarget::Param(root_param.id.clone())
    ));
}

#[tokio::test]
async fn definition_state_migration_tracks_source_binders_not_public_export_aliases() {
    let original = expanded_param_identity(
        r#"
avenger 1;
define mark shell {
  export local as exposed;
  param as local { type: boolean; default: true; }
  mark symbol { x: "x"; y: "y"; visible: $local; }
}
"#,
        "exposed",
    )
    .await;
    let public_alias_renamed = expanded_param_identity(
        r#"
avenger 1;
define mark shell {
  export local as renamed_export;
  param as local { type: boolean; default: true; }
  mark symbol { x: "x"; y: "y"; visible: $local; }
}
"#,
        "renamed_export",
    )
    .await;
    let source_binder_renamed = expanded_param_identity(
        r#"
avenger 1;
define mark shell {
  export renamed_state as exposed;
  param as renamed_state { type: boolean; default: true; }
  mark symbol { x: "x"; y: "y"; visible: $renamed_state; }
}
"#,
        "exposed",
    )
    .await;

    assert_eq!(original, public_alias_renamed);
    assert_ne!(original.0, source_binder_renamed.0);
    assert_ne!(original.1, source_binder_renamed.1);
}

async fn expanded_param_identity(definition: &str, export_alias: &str) -> (String, String) {
    let project = project(&[
        (
            "chart.avenger",
            r#"
avenger 1;
import 'shell.mark.avenger';
chart cartesian as chart {
  mark shell as instance { }
}
"#,
        ),
        ("shell.mark.avenger", definition),
    ])
    .await;
    let resolved = resolve_project(&project, &bootstrap_schema())
        .result
        .unwrap();
    let expanded = expand_project(&project, &resolved).unwrap();
    let resolved = resolve_project(&expanded.project, &bootstrap_schema())
        .result
        .unwrap();
    let param = resolved
        .params
        .values()
        .find(|param| param.source_name.starts_with("__av_"))
        .expect("expanded definition-owned param");
    assert!(matches!(
        resolved
            .public_targets
            .get(&format!("chart.instance.{export_alias}")),
        Some(ResolvedTarget::Param(id)) if id == &param.id
    ));
    (
        param.id.as_str().to_owned(),
        param
            .migration_key
            .as_ref()
            .expect("expanded state migration key")
            .as_str()
            .to_owned(),
    )
}

fn find_resolved_declaration<'a>(
    declaration: &'a ResolvedDeclaration,
    name: &str,
) -> Option<&'a ResolvedDeclaration> {
    if declaration.name.as_deref() == Some(name) {
        return Some(declaration);
    }
    declaration
        .children
        .iter()
        .find_map(|child| find_resolved_declaration(child, name))
}

#[tokio::test]
async fn expansion_inlines_transform_functions_lists_outputs_and_intermediates() {
    let project = project(&[
        (
            "chart.avenger",
            r#"
avenger 1;
import 'rolling.transform.avenger';
chart cartesian {
  transform rolling as rolled {
    measure: "value";
    keys: ["group", "region"];
    agg: avg;
  }
  mark symbol { x: "group"; y: rolled.value; }
}
"#,
        ),
        (
            "rolling.transform.avenger",
            r#"
avenger 1;
define transform rolling {
  slot expr as measure;
  slot expr_list as keys;
  slot function as agg { class: aggregate; }
  output value;
  transform sql {
    query:
      SELECT *, agg(measure) OVER (PARTITION BY keys) AS value,
        measure AS __rolling_private
      FROM input;
  }
}
"#,
        ),
    ])
    .await;
    let resolved = resolve_project(&project, &bootstrap_schema())
        .result
        .unwrap();
    let expanded = expand_project(&project, &resolved).unwrap();
    let chart = project.chart_roots.first().unwrap();
    let text = expanded.texts.get(chart).unwrap();

    assert!(text.contains("transform pipeline as rolled"), "{text}");
    assert!(text.contains("output value;"), "{text}");
    assert!(text.contains("avg(\"value\")"), "{text}");
    assert!(
        text.contains("PARTITION BY \"group\", \"region\""),
        "{text}"
    );
    assert!(text.contains("__rolled_"), "{text}");
    assert!(text.contains("_private"), "{text}");
    assert!(!text.contains("__rolling_private"), "{text}");

    resolve_project(&expanded.project, &bootstrap_schema())
        .result
        .unwrap();
}

#[tokio::test]
async fn expansion_inlines_tool_state_events_references_and_exports() {
    let project = project(&[
        (
            "chart.avenger",
            r#"
avenger 1;
import 'hover.tool.avenger';
chart cartesian {
  mark symbol as points { x: "x"; y: "y"; }
  tool hover as highlighter { target: points; }
}
"#,
        ),
        (
            "hover.tool.avenger",
            r#"
avenger 1;
define tool hover {
  slot ref as target { kind: mark; }
  export hovered;
  selection as hovered { empty: none; }
  on mark_mouse_enter {
    target: mark target;
    set selection hovered = clear;
  }
}
"#,
        ),
    ])
    .await;
    let resolved = resolve_project(&project, &bootstrap_schema())
        .result
        .unwrap();
    let expanded = expand_project(&project, &resolved).unwrap();
    let chart = project.chart_roots.first().unwrap();
    let text = expanded.texts.get(chart).unwrap();

    assert!(text.contains("tool behavior as highlighter"), "{text}");
    assert!(text.contains("component_kind: hover;"), "{text}");
    assert!(text.contains(" as hovered;"), "{text}");
    assert!(text.contains("private selection as __av_"), "{text}");
    assert!(text.contains("set selection __av_"), "{text}");
    assert!(text.contains("target: mark points;"), "{text}");

    resolve_project(&expanded.project, &bootstrap_schema())
        .result
        .unwrap();
}
