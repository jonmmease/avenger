use avenger_chart_schema::NativeSchemaSnapshot;
use avenger_lang_core::{
    ContentVersion, ImportCapabilities, InMemorySourceLoader, LoadedSource, ModuleGraphLoadLimits,
    ModuleGraphLoadRequest, ModuleGraphLoader, ModuleRoot, ResolvedDeclaration, ResolvedTarget,
    ResolvedValue, SourceOrigin, expand_module_graph, resolve_module_graph,
};

fn bootstrap_schema() -> NativeSchemaSnapshot {
    serde_json::from_str(include_str!(
        "../../avenger-chart-lang-registry/snapshots/bootstrap-schema.json"
    ))
    .unwrap()
}

async fn project(sources: &[(&str, &str)]) -> avenger_lang_core::ParsedModuleGraph {
    let loader = InMemorySourceLoader::default();
    for (name, text) in sources {
        loader.insert(LoadedSource::new(
            SourceOrigin::Memory((*name).to_owned()),
            *text,
            ContentVersion::new("definitions-v1"),
        ));
    }
    ModuleGraphLoader::new(&loader)
        .load(ModuleGraphLoadRequest {
            project_root: "/project".into(),
            roots: vec![ModuleRoot::requested(SourceOrigin::Memory(
                "chart.avenger".to_owned(),
            ))],
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
async fn definitions_validate_closed_match_block_exposure_and_parts() {
    let project = project(&[
        (
            "chart.avenger",
            r#"
avenger 1;
import { summary } from 'summary.avenger';
chart cartesian as chart {
  mark summary as result {
    mode: show;
    measure: "value";
    zindex: 2;
    annotations: { mark text as label { x: encoded "x"; y: encoded "y"; text: direct 'ok'; } }
    part point { fill: direct '#dc2626'; }
  }
}
"#,
        ),
        (
            "summary.avenger",
            r#"
avenger 1;
export define mark summary {
  slot expr measure;
  slot enum mode { values: [show, hide]; default: show; }
  slot block annotations { exposes: [point]; default: { } }
  export point;
  mark symbol as point { x: encoded "x"; y: encoded measure; }
  match mode {
    show { annotations; }
    hide { }
  }
}
"#,
        ),
    ])
    .await;
    resolve_module_graph(&project, &bootstrap_schema())
        .result
        .unwrap();
}

#[tokio::test]
async fn expansion_source_map_keeps_non_chart_declarations_in_alignment() {
    let project = project(&[
        (
            "chart.avenger",
            r#"avenger 1;
import { pass } from 'pass.avenger';
schema tables as local {
  table inline as points { values: [{ x: 1; }]; }
}
chart cartesian {
  data: { table: local.points; }
  transform sql { query: SELECT * FROM input WHERE "value" > 10; }
  transform pass {}
  mark symbol { x: encoded "x"; }
}
"#,
        ),
        (
            "pass.avenger",
            "avenger 1; export define transform pass { transform filter { predicate: true; } }",
        ),
    ])
    .await;
    let resolved = resolve_module_graph(&project, &bootstrap_schema())
        .result
        .unwrap();
    let expanded = expand_module_graph(&project, &resolved).unwrap();
    let resolved = resolve_module_graph(&expanded.module_graph, &bootstrap_schema())
        .result
        .unwrap();
    let chart = resolved
        .source_modules
        .values()
        .flat_map(|module| &module.roots)
        .find(|declaration| declaration.keyword == "chart")
        .unwrap();
    let sql = chart
        .children
        .iter()
        .find(|declaration| declaration.kind.as_deref() == Some("sql"))
        .unwrap();
    let authored = expanded.source_map.authored_span(sql.span);
    let source = resolved.sources.get(authored.source).unwrap();
    assert!(
        source.text()[authored.range.as_range()].starts_with("transform sql"),
        "SQL stage mapped to {:?}",
        &source.text()[authored.range.as_range()]
    );
}

#[tokio::test]
async fn definition_parts_replace_complete_configured_channel_values() {
    let project = project(&[
        (
            "chart.avenger",
            r#"
avenger 1;
import { summary } from 'summary.avenger';
chart cartesian as chart {
  mark summary as result {
    part point {
      fill: encoded "category" {
        when {
          predicate: true;
          direct: '#dc2626';
        }
        otherwise: {
          direct: '#94a3b8';
        }
      }
    }
  }
}
"#,
        ),
        (
            "summary.avenger",
            r#"
avenger 1;
export define mark summary {
  export point;
  mark symbol as point {
    x: encoded "x";
    y: encoded "y";
    fill: encoded "series" {
      legend: {
        title: 'Original';
      }
    }
  }
}
"#,
        ),
    ])
    .await;
    let resolved = resolve_module_graph(&project, &bootstrap_schema())
        .result
        .unwrap();
    let expanded = expand_module_graph(&project, &resolved).unwrap();
    let chart = project.requested_modules.first().unwrap();
    let text = expanded.texts.get(chart).unwrap();

    assert!(text.contains("fill: encoded \"category\" {"), "{text}");
    assert!(text.contains("predicate: true;"), "{text}");
    assert!(text.contains("otherwise: {"), "{text}");
    assert!(!text.contains("\"series\""), "{text}");
    assert!(!text.contains("title: 'Original';"), "{text}");

    resolve_module_graph(&expanded.module_graph, &bootstrap_schema())
        .result
        .unwrap();
}

#[tokio::test]
async fn definition_parts_reject_direct_child_declarations() {
    let project = project(&[
        (
            "chart.avenger",
            r#"
avenger 1;
import { summary } from 'summary.avenger';
chart cartesian as chart {
  mark summary as result {
    part point {
      adjust expr {
        size: direct 2.0;
      }
    }
  }
}
"#,
        ),
        (
            "summary.avenger",
            r#"
avenger 1;
export define mark summary {
  export point;
  mark symbol as point { x: encoded "x"; y: encoded "y"; }
}
"#,
        ),
    ])
    .await;
    let failure = resolve_module_graph(&project, &bootstrap_schema())
        .result
        .unwrap_err();

    assert!(failure.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str() == "AVENGER-RESOLVE-167"
            && diagnostic
                .primary
                .message
                .contains("no direct child declarations")
    }));
}

#[tokio::test]
async fn definitions_reject_incomplete_matches_invalid_splices_exposure_data_and_themes() {
    let project = project(&[
        (
            "chart.avenger",
            "avenger 1; import { broken } from 'broken.avenger'; chart cartesian { mark broken { mode: a; } }",
        ),
        (
            "broken.avenger",
            r#"
avenger 1;
export define mark broken {
  slot enum mode { values: [a, b]; default: a; }
  slot block content { exposes: [missing]; default: { } }
  mark group { data: { values: []; } }
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
    let failure = resolve_module_graph(&project, &bootstrap_schema())
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
async fn definitions_reject_anonymous_defined_tool_and_wrong_reference_kind() {
    let project = project(&[
        (
            "chart.avenger",
            r#"
avenger 1;
import { picker } from 'picker.avenger';
chart cartesian {
  param true as state;
  mark symbol as points { x: encoded "x"; y: encoded "y"; }
  tool picker;
  tool picker as wrong_target { target: $state; }
}
"#,
        ),
        (
            "picker.avenger",
            r#"
avenger 1;
export define tool picker {
  slot ref target { kind: mark; }
  on click { target: mark target; set cursor to pointer; }
}
"#,
        ),
    ])
    .await;
    let failure = resolve_module_graph(&project, &bootstrap_schema())
        .result
        .unwrap_err();
    let codes = failure
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(codes.contains("AVENGER-RESOLVE-164"));
}

#[tokio::test]
async fn definitions_require_explicit_imports() {
    let project = project(&[(
        "chart.avenger",
        "avenger 1; chart cartesian { mark unimported_definition { } }",
    )])
    .await;
    let failure = resolve_module_graph(&project, &bootstrap_schema())
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
            "avenger 1; import { a } from 'a.avenger'; chart cartesian { mark a { } }",
            ContentVersion::new("definitions-v1"),
        ))
        .with_source(LoadedSource::new(
            SourceOrigin::Memory("a.avenger".to_owned()),
            "avenger 1; import { b } from 'b.avenger'; export define mark a { mark b { } }",
            ContentVersion::new("definitions-v1"),
        ))
        .with_source(LoadedSource::new(
            SourceOrigin::Memory("b.avenger".to_owned()),
            "avenger 1; import { a } from 'a.avenger'; export define mark b { mark a { } }",
            ContentVersion::new("definitions-v1"),
        ));
    let failure = ModuleGraphLoader::new(&loader)
        .load(ModuleGraphLoadRequest {
            project_root: "/project".into(),
            roots: vec![ModuleRoot::requested(SourceOrigin::Memory(
                "chart.avenger".to_owned(),
            ))],
            native_modules: Default::default(),
            capabilities: ImportCapabilities::in_memory("/project"),
            schema_version: "semantic-v1".to_owned(),
            registry_version: "bootstrap".to_owned(),
            limits: ModuleGraphLoadLimits::default(),
        })
        .await
        .result
        .unwrap_err();
    assert_eq!(failure.diagnostics[0].code.as_str(), "AVENGER-MODULE-013");
    assert!(failure.diagnostics[0].trace.len() >= 2);
}

#[tokio::test]
async fn definitions_reject_ambient_capture_private_access_export_collisions_and_missing_binders() {
    let project = project(&[
        (
            "chart.avenger",
            r#"
avenger 1;
import { capturing } from 'capturing.avenger';
import { private_component } from 'private_component.avenger';
import { colliding } from 'colliding.avenger';
import { output_transform } from 'output_transform.avenger';
chart cartesian as chart {
  param true as ambient;
  mark capturing as captured { }
  mark private_component as component { }
  transform output_transform { }
  on click {
    target: mark component.hidden;
    set cursor to pointer;
  }
}
"#,
        ),
        (
            "capturing.avenger",
            r#"
avenger 1;
export define mark capturing {
  mark symbol as glyph { x: encoded "x"; y: encoded "y"; visible: $ambient; }
}
"#,
        ),
        (
            "private_component.avenger",
            r#"
avenger 1;
export define mark private_component {
  mark symbol as hidden { x: encoded "x"; y: encoded "y"; }
}
"#,
        ),
        (
            "colliding.avenger",
            r#"
avenger 1;
export define mark colliding {
  export first as duplicate;
  export second as duplicate;
  mark symbol as first { x: encoded "x"; y: encoded "y"; }
  mark symbol as second { x: encoded "x"; y: encoded "y"; }
}
"#,
        ),
        (
            "output_transform.avenger",
            r#"
avenger 1;
export define transform output_transform {
  output value;
  transform sql { query: SELECT *, 1 AS value FROM input; }
}
"#,
        ),
    ])
    .await;
    let failure = resolve_module_graph(&project, &bootstrap_schema())
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
import { shell } from 'shell.avenger';
chart cartesian {
  mark shell {
    content: {
      param true as invalid_here;
    }
  }
}
"#,
        ),
        (
            "shell.avenger",
            r#"
avenger 1;
export define mark shell {
  slot block content;
  mark symbol { x: encoded "x"; y: encoded "y"; content; }
}
"#,
        ),
    ])
    .await;
    let resolved = resolve_module_graph(&project, &bootstrap_schema())
        .result
        .expect("caller block resolution is deferred until expansion");
    let expanded = expand_module_graph(&project, &resolved).unwrap();
    let failure = resolve_module_graph(&expanded.module_graph, &bootstrap_schema())
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
import { summary } from 'summary.avenger';
chart cartesian as chart {
  mark summary as result {
    band_axis: y;
    value_axis: x;
    measure: "value";
    mode: show;
    annotations: {
      on click {
        target: mark point;
        set cursor to crosshair;
      }
      mark text as label {
        band_axis: encoded "category";
        value_axis: encoded "value";
        text: direct 'ok';
      }
    }
    part point { fill: direct '#dc2626'; }
  }
  mark summary as hidden {
    band_axis: x;
    value_axis: y;
    measure: "value";
    mode: hide;
    annotations: {
      mark text as hidden_label { x: encoded "x"; y: encoded "y"; text: direct 'not selected'; }
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
            "summary.avenger",
            r#"
avenger 1;
export define mark summary {
  slot channel band_axis { default: x; }
  slot channel value_axis { default: y; }
  slot expr measure;
  slot number width { default: 0.6; }
  slot number cap_width { default: width / 2; }
  slot enum mode { values: [show, hide]; default: show; }
  slot block annotations { exposes: [point]; default: { } }
  export body.point as point;
  mark group as body {
    mark symbol as point {
      band_axis: encoded "category";
      value_axis: encoded measure;
      size: encoded cap_width * 100;
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
    let resolved = resolve_module_graph(&project, &bootstrap_schema())
        .result
        .unwrap();
    let expanded = expand_module_graph(&project, &resolved).unwrap();
    let chart = project.requested_modules.first().unwrap();
    let text = expanded.texts.get(chart).unwrap();

    assert!(!text.contains("import { summary } from 'summary.avenger'"));
    assert!(!text.contains("mark summary"));
    assert!(!text.contains("match mode"));
    assert!(!text.contains("annotations;"));
    assert!(text.contains("mark group as result"), "{text}");
    assert!(text.contains("component_kind: summary;"), "{text}");
    assert!(text.contains("export __av_"), "{text}");
    assert!(text.contains(" as point;"), "{text}");
    assert!(text.contains("private mark group as __av_"), "{text}");
    assert!(text.contains("y: encoded \"category\";"), "{text}");
    assert!(text.contains("x: encoded \"value\";"), "{text}");
    assert!(text.contains("public mark text as label"), "{text}");
    assert!(!text.contains("hidden_label"), "{text}");
    assert!(text.contains("target: mark __av_"), "{text}");
    assert!(text.contains("fill: direct '#dc2626';"), "{text}");
    assert!(text.contains("widget slider as threshold"), "{text}");
    assert!(!expanded.source_map.mappings.is_empty());

    resolve_module_graph(&expanded.module_graph, &bootstrap_schema())
        .result
        .unwrap();
}

#[tokio::test]
async fn definition_marks_expand_inside_legend_overlay_mark_blocks() {
    let project = project(&[
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
    ])
    .await;
    let resolved = resolve_module_graph(&project, &bootstrap_schema())
        .result
        .unwrap();
    let expanded = expand_module_graph(&project, &resolved).unwrap();
    let chart = project.requested_modules.first().unwrap();
    let text = expanded.texts.get(chart).unwrap();

    assert!(text.contains("overlay: {"), "{text}");
    assert!(text.contains("mark group as imported_band"), "{text}");
    assert!(text.contains("component_kind: band;"), "{text}");
    resolve_module_graph(&expanded.module_graph, &bootstrap_schema())
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
import { shell } from 'shell.avenger';
chart cartesian as chart {
  param false as enabled;
  mark shell as instance {
    content: {
      mark symbol as caller_mark {
        x: encoded "x";
        y: encoded "y";
        visible: $enabled;
      }
    }
  }
  mark shell as second { content: { } }
}
"#,
        ),
        (
            "shell.avenger",
            r#"
avenger 1;
export define mark shell {
  slot block content { exposes: [inside]; default: { } }
  export inside;
  param true as enabled;
  mark symbol as inside { x: encoded "x"; y: encoded "y"; visible: $enabled; }
  content;
}
"#,
        ),
    ])
    .await;
    let resolved = resolve_module_graph(&project, &bootstrap_schema())
        .result
        .unwrap();
    let first = expand_module_graph(&project, &resolved).unwrap();
    let second = expand_module_graph(&project, &resolved).unwrap();
    assert_eq!(first.texts, second.texts);
    let chart = project.requested_modules.first().unwrap();
    let text = first.texts.get(chart).unwrap();
    assert!(text.contains("private param true as __av_"), "{text}");
    assert!(text.contains("visible: $__av_"), "{text}");
    assert!(text.contains("visible: $enabled;"), "{text}");

    let expanded = resolve_module_graph(&first.module_graph, &bootstrap_schema())
        .result
        .unwrap();
    let root_param = expanded
        .params
        .values()
        .find(|param| param.source_name == "enabled" && param.definition_local_seed.is_none())
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
        .source_modules
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
export define mark shell {
  export local as exposed;
  param true as local;
  mark symbol { x: encoded "x"; y: encoded "y"; visible: $local; }
}
"#,
        "exposed",
    )
    .await;
    let public_alias_renamed = expanded_param_identity(
        r#"
avenger 1;
export define mark shell {
  export local as renamed_export;
  param true as local;
  mark symbol { x: encoded "x"; y: encoded "y"; visible: $local; }
}
"#,
        "renamed_export",
    )
    .await;
    let source_binder_renamed = expanded_param_identity(
        r#"
avenger 1;
export define mark shell {
  export renamed_state as exposed;
  param true as renamed_state;
  mark symbol { x: encoded "x"; y: encoded "y"; visible: $renamed_state; }
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
import { shell } from 'shell.avenger';
chart cartesian as chart {
  mark shell as instance { }
}
"#,
        ),
        ("shell.avenger", definition),
    ])
    .await;
    let resolved = resolve_module_graph(&project, &bootstrap_schema())
        .result
        .unwrap();
    let expanded = expand_module_graph(&project, &resolved).unwrap();
    let resolved = resolve_module_graph(&expanded.module_graph, &bootstrap_schema())
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
async fn expansion_inlines_transform_lists_outputs_and_intermediates() {
    let project = project(&[
        (
            "chart.avenger",
            r#"
avenger 1;
import { rolling } from 'rolling.avenger';
chart cartesian {
  transform rolling as rolled {
    measure: "value";
    keys: ["group", "region"];
  }
  mark symbol { x: encoded "group"; y: encoded rolled.value; }
}
"#,
        ),
        (
            "rolling.avenger",
            r#"
avenger 1;
export define transform rolling {
  slot expr measure;
  slot expr_list keys;
  output value;
  transform sql {
    query:
      SELECT *, avg(measure) OVER (PARTITION BY keys) AS value,
        measure AS "__private_value",
        '__private_literal' AS marker
      FROM input;
  }
}
"#,
        ),
    ])
    .await;
    let resolved = resolve_module_graph(&project, &bootstrap_schema())
        .result
        .unwrap();
    let expanded = expand_module_graph(&project, &resolved).unwrap();
    let chart = project.requested_modules.first().unwrap();
    let text = expanded.texts.get(chart).unwrap();

    assert!(text.contains("transform pipeline as rolled"), "{text}");
    assert!(text.contains("output value;"), "{text}");
    assert!(text.contains("avg(\"value\")"), "{text}");
    assert!(
        text.contains("PARTITION BY \"group\", \"region\""),
        "{text}"
    );
    assert!(text.contains("\"__av_col_"), "{text}");
    assert!(text.contains("_value\""), "{text}");
    assert!(text.contains("'__private_literal'"), "{text}");
    assert!(!text.contains("\"__private_value\""), "{text}");

    resolve_module_graph(&expanded.module_graph, &bootstrap_schema())
        .result
        .unwrap();
}

#[tokio::test]
async fn outputs_slot_publishes_call_site_aliases_and_expands_pipeline_outputs() {
    let project = project(&[
        (
            "chart.avenger",
            r#"
avenger 1;
import { summarize } from 'summarize.avenger';
chart cartesian {
  transform summarize as stats {
    measures: sum("amount") AS Total, avg("amount") AS average;
  }
  mark symbol { x: encoded stats.Total; y: encoded stats.average; }
}
"#,
        ),
        (
            "summarize.avenger",
            r#"
avenger 1;
export define transform summarize {
  slot outputs measures;
  transform aggregate { expressions: measures; }
}
"#,
        ),
    ])
    .await;
    let resolved = resolve_module_graph(&project, &bootstrap_schema())
        .result
        .unwrap();
    let chart = resolved
        .source_modules
        .values()
        .find_map(|module| module.roots.iter().find(|root| root.keyword == "chart"))
        .unwrap();
    assert_eq!(
        chart.children[0]
            .transform_outputs
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        ["Total", "average"]
    );

    let expanded = expand_module_graph(&project, &resolved).unwrap();
    let chart_module = project.requested_modules.first().unwrap();
    let text = &expanded.texts[chart_module];
    assert!(text.contains("transform pipeline as stats"), "{text}");
    assert!(text.contains("output Total;"), "{text}");
    assert!(text.contains("output average;"), "{text}");
    assert!(text.contains("sum(\"amount\") AS Total"), "{text}");
    resolve_module_graph(&expanded.module_graph, &bootstrap_schema())
        .result
        .unwrap();
}

#[tokio::test]
async fn outputs_slot_splices_exact_quoted_aliases_into_full_select() {
    let project = project(&[
        (
            "chart.avenger",
            r#"
avenger 1;
import { project } from 'project.avenger';
chart cartesian {
  transform project as projected {
    columns: "amount" + 1 AS Total;
  }
}
"#,
        ),
        (
            "project.avenger",
            r#"
avenger 1;
export define transform project {
  slot outputs columns;
  transform sql { query: SELECT columns FROM input; }
}
"#,
        ),
    ])
    .await;
    let resolved = resolve_module_graph(&project, &bootstrap_schema())
        .result
        .unwrap();
    let expanded = expand_module_graph(&project, &resolved).unwrap();
    let chart_module = project.requested_modules.first().unwrap();
    let text = &expanded.texts[chart_module];
    assert!(text.contains("AS \"Total\""), "{text}");
    assert!(text.contains("output Total;"), "{text}");
    resolve_module_graph(&expanded.module_graph, &bootstrap_schema())
        .result
        .unwrap();
}

#[tokio::test]
async fn outputs_slot_requires_one_placeholder_occurrence() {
    let project = project(&[(
        "chart.avenger",
        r#"
avenger 1;
define transform duplicate {
  slot outputs columns;
  transform sql {
    query: SELECT columns, columns FROM input;
  }
}
"#,
    )])
    .await;
    let failure = resolve_module_graph(&project, &bootstrap_schema())
        .result
        .unwrap_err();
    assert!(
        failure.diagnostics.iter().any(|diagnostic| {
            diagnostic.code.as_str() == "AVENGER-RESOLVE-167"
                && diagnostic.primary.message.contains("used 2 times")
        }),
        "{:#?}",
        failure.diagnostics
    );
}

#[tokio::test]
async fn outputs_slot_requires_a_whole_projection_destination() {
    let project = project(&[(
        "chart.avenger",
        r#"
avenger 1;
define transform misplaced {
  slot outputs columns;
  transform sql {
    query: SELECT "value" FROM input WHERE columns;
  }
}
"#,
    )])
    .await;
    let failure = resolve_module_graph(&project, &bootstrap_schema())
        .result
        .unwrap_err();
    assert!(
        failure.diagnostics.iter().any(|diagnostic| {
            diagnostic.code.as_str() == "AVENGER-RESOLVE-170"
                && diagnostic.message.contains("projection splice")
        }),
        "{:#?}",
        failure.diagnostics
    );
}

#[tokio::test]
async fn outputs_slot_requires_a_binder_and_rejects_static_output_collisions() {
    let project = project(&[
        (
            "chart.avenger",
            r#"
avenger 1;
import { summarize } from 'summarize.avenger';
chart cartesian {
  transform summarize {
    measures: sum("amount") AS total;
  }
}
"#,
        ),
        (
            "summarize.avenger",
            r#"
avenger 1;
export define transform summarize {
  slot outputs measures;
  output total;
  transform aggregate { expressions: measures; }
}
"#,
        ),
    ])
    .await;
    let failure = resolve_module_graph(&project, &bootstrap_schema())
        .result
        .unwrap_err();
    let codes = failure
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(codes.contains("AVENGER-RESOLVE-169"), "{codes:?}");
    assert!(codes.contains("AVENGER-RESOLVE-150"), "{codes:?}");
}

#[tokio::test]
async fn resolver_reserves_compiler_names_and_definition_private_columns() {
    let project = project(&[(
        "chart.avenger",
        r#"
avenger 1;
chart cartesian as chart {
  param true as __av_authored;
  private param true as __av_authored_private;
  transform sql {
    query: SELECT *, __private_value AS copied FROM input;
  }
}
"#,
    )])
    .await;
    let failure = resolve_module_graph(&project, &bootstrap_schema())
        .result
        .unwrap_err();
    let codes = failure
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(codes.contains("AVENGER-RESOLVE-181"), "{codes:?}");
    assert!(codes.contains("AVENGER-RESOLVE-182"), "{codes:?}");
    assert!(
        failure
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-181")
            .count()
            >= 2
    );
}

#[tokio::test]
async fn expansion_rejects_an_authored_private_binder_matching_its_generated_name() {
    let definition = r#"
avenger 1;
export define mark shell {
  mark symbol as inside { x: encoded "x"; y: encoded "y"; }
}
"#;
    let first_chart = r#"
avenger 1;
import { shell } from 'shell.avenger';
chart cartesian as chart {
  mark shell as instance {}
}
"#;
    let first = project(&[
        ("chart.avenger", first_chart),
        ("shell.avenger", definition),
    ])
    .await;
    let resolved = resolve_module_graph(&first, &bootstrap_schema())
        .result
        .unwrap();
    let expanded = expand_module_graph(&first, &resolved).unwrap();
    let chart = first.requested_modules.first().unwrap();
    let generated = expanded
        .texts
        .get(chart)
        .unwrap()
        .split(|character: char| !(character.is_alphanumeric() || character == '_'))
        .find(|word| word.starts_with("__av_") && word.ends_with("_inside"))
        .expect("generated private binder");
    let colliding_chart = format!(
        r#"
avenger 1;
import {{ shell }} from 'shell.avenger';
chart cartesian as chart {{
  private mark symbol as {generated} {{ x: encoded "x"; y: encoded "y"; }}
  mark shell as instance {{}}
}}
"#
    );
    let colliding = project(&[
        ("chart.avenger", colliding_chart.as_str()),
        ("shell.avenger", definition),
    ])
    .await;
    let resolved = resolve_module_graph(&colliding, &bootstrap_schema())
        .result
        .unwrap();
    let failure = expand_module_graph(&colliding, &resolved).unwrap_err();
    assert!(
        failure
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "AVENGER-EXPAND-009"),
        "{:?}",
        failure.diagnostics
    );
}

#[tokio::test]
async fn expansion_inlines_tool_state_events_references_and_exports() {
    let project = project(&[
        (
            "chart.avenger",
            r#"
avenger 1;
import { hover } from 'hover.avenger';
chart cartesian {
  mark symbol as points { x: encoded "x"; y: encoded "y"; }
  tool hover as highlighter { target: points; }
}
"#,
        ),
        (
            "hover.avenger",
            r#"
avenger 1;
export define tool hover {
  slot ref target { kind: mark; }
  export hovered;
  selection as hovered { empty: none; }
  on mark_mouse_enter {
    target: mark target;
    clear hovered;
  }
}
"#,
        ),
    ])
    .await;
    let resolved = resolve_module_graph(&project, &bootstrap_schema())
        .result
        .unwrap();
    let expanded = expand_module_graph(&project, &resolved).unwrap();
    let chart = project.requested_modules.first().unwrap();
    let text = expanded.texts.get(chart).unwrap();

    assert!(text.contains("tool behavior as highlighter"), "{text}");
    assert!(text.contains("component_kind: hover;"), "{text}");
    assert!(text.contains(" as hovered;"), "{text}");
    assert!(text.contains("private selection as __av_"), "{text}");
    assert!(text.contains("clear __av_"), "{text}");
    assert!(text.contains("target: mark points;"), "{text}");

    resolve_module_graph(&expanded.module_graph, &bootstrap_schema())
        .result
        .unwrap();
}
