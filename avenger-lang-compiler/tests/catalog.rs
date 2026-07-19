use std::path::PathBuf;

use arrow::datatypes::DataType;
use avenger_lang_compiler::{Compiler, DatasetStageKind};

fn project_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/projects")
        .join(name)
}

#[tokio::test]
async fn catalog_project_analyzes_qualified_tables_and_sql_views_without_execution() {
    let root = project_fixture("06_catalog_project");
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let analysis = compiler.analyze_project(&root).await.unwrap();
    let tables = analysis
        .datasets
        .iter()
        .map(|(_, dataset)| {
            (
                dataset.qualified_name.as_deref().unwrap().to_owned(),
                dataset,
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();

    assert_eq!(tables.len(), 4);
    assert!(tables.contains_key("regions"));
    assert!(tables.contains_key("vega.movies"));
    assert!(matches!(
        tables["vega.popular"].provenance.stage_kind,
        DatasetStageKind::SqlView
    ));
    assert_eq!(
        tables["vega.popular"]
            .columns
            .iter()
            .map(|column| (&column.name, &column.data_type, column.nullable))
            .collect::<Vec<_>>(),
        tables["vega.popular_from_first"]
            .columns
            .iter()
            .map(|column| (&column.name, &column.data_type, column.nullable))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        tables["vega.popular"].columns[1].data_type,
        DataType::Float64
    );
}

#[tokio::test]
async fn catalog_project_compiles_two_charts_against_one_registered_catalog() {
    let root = project_fixture("06_catalog_project");
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let project = compiler.compile_project(&root).await.unwrap();
    assert_eq!(project.charts.len(), 2);
}
