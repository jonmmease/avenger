use std::path::PathBuf;

use avenger_lang_compiler::Compiler;
use avenger_lang_core::ChartSelector;

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
}
