use std::{fs, path::PathBuf};

use avenger_lang_compiler::Compiler;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/projects")
        .join(name)
}

#[tokio::test]
async fn selected_chart_bundle_is_standalone_and_deterministic() {
    let root = fixture("04_custom_error_bar");
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let first = compiler.bundle_chart("chart.avenger", None).await.unwrap();
    let second = compiler.bundle_chart("chart.avenger", None).await.unwrap();
    assert_eq!(first.text, second.text);
    assert!(!first.text.contains(" from '"));
    assert!(!first.text.contains("error_bar.avenger"));
    assert!(first.text.contains("define mark __av_"));
    assert!(first.text.contains("mark __av_"));

    let original = compiler.compile_chart("chart.avenger", None).await.unwrap();
    let bundled = tempfile::tempdir().unwrap();
    fs::write(bundled.path().join("bundle.avenger"), &first.text).unwrap();
    let bundled_compiler = Compiler::builder()
        .project_root(bundled.path())
        .build()
        .unwrap();
    let compiled = bundled_compiler
        .compile_chart("bundle.avenger", None)
        .await
        .unwrap();
    assert_eq!(
        compiled.interface.params.keys().collect::<Vec<_>>(),
        original.interface.params.keys().collect::<Vec<_>>()
    );
    assert_eq!(
        compiled
            .interface
            .params
            .values()
            .map(|binding| &binding.runtime_id)
            .collect::<Vec<_>>(),
        original
            .interface
            .params
            .values()
            .map(|binding| &binding.runtime_id)
            .collect::<Vec<_>>()
    );
    assert_eq!(compiled.interface.stores, original.interface.stores);
    assert_eq!(compiled.interface.selections, original.interface.selections);
    assert_eq!(
        compiled.interface.widget_exports,
        original.interface.widget_exports
    );
    assert_eq!(
        compiled.interface.public_targets,
        original.interface.public_targets
    );
    assert_eq!(compiled.native_requirements, original.native_requirements);
    assert_eq!(
        compiled.compiled_plot().marks().len(),
        original.compiled_plot().marks().len()
    );
}

#[tokio::test]
async fn dataset_bundle_rewrites_imported_sql_relations() {
    let root = fixture("phase9-pack");
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let bundle = compiler.bundle_chart("chart.avenger", None).await.unwrap();
    assert!(!bundle.text.contains("data.avenger"));
    assert!(bundle.text.contains("table inline as movies"));
    assert!(bundle.text.contains("table sql as popular"));

    let bundled = tempfile::tempdir().unwrap();
    fs::write(bundled.path().join("bundle.avenger"), &bundle.text).unwrap();
    Compiler::builder()
        .project_root(bundled.path())
        .build()
        .unwrap()
        .compile_chart("bundle.avenger", None)
        .await
        .unwrap();
}

#[tokio::test]
async fn module_bundle_preserves_all_entrypoints_while_chart_bundle_prunes_siblings() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("library.avenger"),
        r#"avenger 1;
export define mark badge {
  mark symbol as glyph { x: encoded "x"; y: encoded "y"; }
}
"#,
    )
    .unwrap();
    fs::write(
        root.path().join("charts.avenger"),
        r#"avenger 1;
import { badge } from './library.avenger';

chart cartesian as first {
  data: { values: [{ x: 1.0; y: 2.0; }]; }
  mark badge as points {}
}

chart cartesian as second {
  data: { values: [{ x: 2.0; y: 3.0; }]; }
  mark badge as points {}
}
"#,
    )
    .unwrap();
    let compiler = Compiler::builder()
        .project_root(root.path())
        .build()
        .unwrap();
    let module = compiler.bundle_module("charts.avenger").await.unwrap();
    assert!(module.text.contains("chart cartesian as first"));
    assert!(module.text.contains("chart cartesian as second"));
    assert!(!module.text.contains("library.avenger"));

    let selected = compiler
        .bundle_chart("charts.avenger", Some("second"))
        .await
        .unwrap();
    assert!(!selected.text.contains("chart cartesian as first"));
    assert!(selected.text.contains("chart cartesian as second"));

    let bundled = tempfile::tempdir().unwrap();
    fs::write(bundled.path().join("module.avenger"), module.text).unwrap();
    let compiled = Compiler::builder()
        .project_root(bundled.path())
        .build()
        .unwrap()
        .compile_module("module.avenger")
        .await
        .unwrap();
    assert_eq!(compiled.charts.len(), 2);
}

#[tokio::test]
async fn module_bundle_preserves_the_requested_modules_public_export_table() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("library.avenger"),
        r#"avenger 1;
define transform private_identity {
  transform sql { query: SELECT * FROM input; }
}
export define mark badge {
  transform private_identity {}
  mark symbol as glyph { x: encoded "x"; y: encoded "y"; }
}
"#,
    )
    .unwrap();
    fs::write(
        root.path().join("module.avenger"),
        r#"avenger 1;
import { badge } from './library.avenger';

export table inline as observations {
  values: [{ x: 1.0; y: 2.0; }];
}

export chart cartesian as summary {
  data: { table: observations; }
  mark badge as points {}
}
"#,
    )
    .unwrap();

    let compiler = Compiler::builder()
        .project_root(root.path())
        .build()
        .unwrap();
    let original = compiler.compile_module("module.avenger").await.unwrap();
    let bundle = compiler.bundle_module("module.avenger").await.unwrap();
    assert!(!bundle.text.contains("import "));

    let standalone = tempfile::tempdir().unwrap();
    fs::write(standalone.path().join("module.avenger"), bundle.text).unwrap();
    let bundled = Compiler::builder()
        .project_root(standalone.path())
        .build()
        .unwrap()
        .compile_module("module.avenger")
        .await
        .unwrap();

    assert_eq!(
        original
            .exports
            .exports
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["observations", "summary"]
    );
    assert_eq!(
        bundled
            .exports
            .exports
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["observations", "summary"]
    );
    assert_eq!(bundled.charts.len(), 1);
}

#[tokio::test]
async fn bundle_preserves_namespace_imported_dynamic_output_projections() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("library.avenger"),
        r#"avenger 1;
export define transform summarize {
  slot outputs measures;
  transform aggregate { expressions: measures; }
}
"#,
    )
    .unwrap();
    fs::write(
        root.path().join("chart.avenger"),
        r#"avenger 1;
import * as stats from './library.avenger';

chart cartesian as chart {
  data: {
    values: [
      { category: 'A'; amount: 2.0; },
      { category: 'B'; amount: 4.0; }
    ];
  }
  transform stats.summarize as summary {
    measures: sum("amount") AS Total;
  }
  mark symbol { x: encoded summary.Total; y: encoded summary.Total; }
}
"#,
    )
    .unwrap();

    let compiler = Compiler::builder()
        .project_root(root.path())
        .build()
        .unwrap();
    let original = compiler
        .compile_chart("chart.avenger", Some("chart"))
        .await
        .unwrap();
    let bundle = compiler
        .bundle_chart("chart.avenger", Some("chart"))
        .await
        .unwrap();
    assert!(!bundle.text.contains("import "));
    assert!(bundle.text.contains("slot outputs measures"));
    assert!(bundle.text.contains("sum(\"amount\") AS Total"));

    let standalone = tempfile::tempdir().unwrap();
    fs::write(standalone.path().join("bundle.avenger"), bundle.text).unwrap();
    let bundled = Compiler::builder()
        .project_root(standalone.path())
        .build()
        .unwrap()
        .compile_chart("bundle.avenger", Some("chart"))
        .await
        .unwrap();
    assert_eq!(
        bundled.compiled_plot().marks().len(),
        original.compiled_plot().marks().len()
    );
}

#[tokio::test]
async fn bundle_alpha_renames_colliding_exports_and_private_helpers() {
    let root = tempfile::tempdir().unwrap();
    for (module, helper_kind) in [("left", "symbol"), ("right", "rect")] {
        fs::write(
            root.path().join(format!("{module}.avenger")),
            format!(
                r#"avenger 1;
define mark helper {{
  mark {helper_kind} as primitive {{ x: encoded "x"; y: encoded "y"; }}
}}
export define mark badge {{
  mark helper as nested {{}}
}}
"#
            ),
        )
        .unwrap();
    }
    fs::write(
        root.path().join("chart.avenger"),
        r#"avenger 1;
import { badge as left_badge } from './left.avenger';
import * as right from './right.avenger';

chart cartesian as comparison {
  data: { values: [{ x: 1.0; y: 1.0; }]; }
  mark left_badge as left {}
  mark right.badge as right {}
}
"#,
    )
    .unwrap();

    let compiler = Compiler::builder()
        .project_root(root.path())
        .build()
        .unwrap();
    let bundle = compiler
        .bundle_chart("chart.avenger", Some("comparison"))
        .await
        .unwrap();
    assert!(!bundle.text.contains("import * as right"));
    assert!(!bundle.text.contains("left_badge"));
    assert!(!bundle.text.contains("right.badge"));
    assert_eq!(bundle.text.matches("define mark __av_").count(), 4);

    let bundled = tempfile::tempdir().unwrap();
    fs::write(bundled.path().join("bundle.avenger"), &bundle.text).unwrap();
    let compiled = Compiler::builder()
        .project_root(bundled.path())
        .build()
        .unwrap()
        .compile_chart("bundle.avenger", Some("comparison"))
        .await
        .unwrap();
    assert_eq!(compiled.compiled_plot().marks().len(), 2);
}
