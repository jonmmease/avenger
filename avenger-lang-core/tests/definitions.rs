use avenger_chart_schema::NativeSchemaSnapshot;
use avenger_lang_core::{
    ContentVersion, ImportCapabilities, InMemorySourceLoader, LoadedSource, ProjectLoadRequest,
    ProjectLoader, ProjectRoot, SourceOrigin, expand_project, resolve_project,
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
  transform rolling as value { agg: row_number; }
  tool picker;
}
"#,
        ),
        (
            "rolling.transform.avenger",
            r#"
avenger 1;
define transform rolling {
  slot function as agg { class: aggregate; }
  output value;
  transform sql { query: SELECT agg("x") AS value FROM input; }
}
"#,
        ),
        (
            "picker.tool.avenger",
            "avenger 1; define tool picker { on click { set cursor = pointer; } }",
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
      mark text as label {
        band_axis: "category";
        value_axis: "value";
        text: value 'ok';
      }
    }
    part point { fill: value '#dc2626'; }
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
    assert!(text.contains("export body.point as point;"), "{text}");
    assert!(text.contains("private group as body"), "{text}");
    assert!(text.contains("y: \"category\";"), "{text}");
    assert!(text.contains("x: \"value\";"), "{text}");
    assert!(text.contains("public mark text as label"), "{text}");
    assert!(text.contains("fill: value '#dc2626';"), "{text}");
    assert!(text.contains("widget slider as threshold"), "{text}");
    assert!(!expanded.source_map.mappings.is_empty());

    resolve_project(&expanded.project, &bootstrap_schema())
        .result
        .unwrap();
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
    assert!(text.contains("__rolled_private"), "{text}");
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
    assert!(text.contains("export hovered;"), "{text}");
    assert!(text.contains("private selection as hovered"), "{text}");
    assert!(text.contains("target: mark points;"), "{text}");

    resolve_project(&expanded.project, &bootstrap_schema())
        .result
        .unwrap();
}
