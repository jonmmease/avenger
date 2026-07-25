use std::{path::PathBuf, sync::Arc};

use avenger_lang_compiler::Compiler;
use avenger_lang_core::{
    ChartSelector, ContentVersion, InMemorySourceLoader, LoadedSource, ModuleId, SourceLoader,
    SourceOrigin,
};

fn modules_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../avenger-lang-core/tests/fixtures/modules")
}

#[tokio::test]
async fn compiles_anonymous_named_and_mixed_module_entrypoints() {
    let root = modules_dir();
    let compiler = Compiler::builder().project_root(&root).build().unwrap();

    let singleton = compiler
        .compile_chart(root.join("mixed-singleton.avenger"), None)
        .await
        .unwrap();
    assert_eq!(singleton.id.selector, ChartSelector::Anonymous);

    let failure = compiler
        .compile_chart(root.join("multi-chart.avenger"), None)
        .await
        .unwrap_err();
    assert!(
        failure
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-242")
    );

    let scatter = compiler
        .compile_chart(root.join("multi-chart.avenger"), Some("scatter"))
        .await
        .unwrap();
    let bars = compiler
        .compile_chart(root.join("multi-chart.avenger"), Some("bars"))
        .await
        .unwrap();
    assert_ne!(scatter.id, bars.id);
    assert_eq!(
        scatter.id.selector,
        ChartSelector::Named("scatter".to_owned())
    );
    assert_eq!(bars.id.selector, ChartSelector::Named("bars".to_owned()));

    let compiled = compiler
        .compile_module(root.join("multi-chart.avenger"))
        .await
        .unwrap();
    assert_eq!(compiled.charts.len(), 2);
    assert!(compiled.charts.contains_key(&scatter.id));
    assert!(compiled.charts.contains_key(&bars.id));
}

#[tokio::test]
async fn imported_mixed_library_compiles_without_treating_its_items_as_entrypoints() {
    let root = modules_dir();
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let artifact = compiler
        .compile_chart(root.join("consumer.avenger"), None)
        .await
        .unwrap();
    assert_eq!(artifact.id.selector, ChartSelector::Anonymous);

    let compiled = compiler
        .compile_module(root.join("consumer.avenger"))
        .await
        .unwrap();
    assert_eq!(compiled.charts.len(), 1);
    assert!(
        compiled
            .charts
            .keys()
            .all(|entrypoint| entrypoint.module == artifact.id.module)
    );
    assert!(
        compiled.exports.exports.is_empty(),
        "the consumer module's private chart must not inherit its dependency's exports"
    );
}

#[tokio::test]
async fn compiled_module_publishes_only_the_requested_modules_export_index() {
    let root = modules_dir();
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let compiled = compiler
        .compile_module(root.join("library.avenger"))
        .await
        .unwrap();

    assert!(compiled.charts.is_empty());
    assert_eq!(
        compiled
            .exports
            .exports
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["example_rows", "library_point"]
    );
    let defining_module = compiled
        .exports
        .exports
        .values()
        .next()
        .expect("library export")
        .module
        .clone();
    assert!(matches!(&defining_module, ModuleId::Source(_)));
    assert!(
        compiled
            .exports
            .exports
            .values()
            .all(|export| export.module == defining_module)
    );
}

#[tokio::test]
async fn compiled_transform_may_join_input_to_an_explicit_standard_dataset() {
    let loader = InMemorySourceLoader::default()
        .with_source(LoadedSource::new(
            SourceOrigin::File("/project/chart.avenger".into()),
            r#"avenger 1;
import { country_names } from 'std:datasets.avenger';

define transform attach_country_name {
  output country_name;
  transform sql {
    query:
      SELECT rows.*, countries.name AS country_name
      FROM input AS rows
      JOIN country_names AS countries
        ON rows.country_code = countries.code;
  }
}

chart cartesian {
  data: { values: [{ country_code: 'US'; value: 1.0; }]; }
  transform attach_country_name as enriched {}
  mark text { text: "country_name"; x: value 1.0; y: "value"; }
}
"#,
            ContentVersion::new("chart-v1"),
        ))
        .with_source(LoadedSource::new(
            SourceOrigin::Std("datasets.avenger".into()),
            r#"avenger 1;
export table inline as country_names {
  values: [{ code: 'US'; name: 'United States'; }];
}
"#,
            ContentVersion::new("datasets-v1"),
        ));
    let artifact = Compiler::builder()
        .project_root("/project")
        .source_loader(Arc::new(loader) as Arc<dyn SourceLoader>)
        .build()
        .unwrap()
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    assert_eq!(artifact.compiled_plot().marks().len(), 1);
}
