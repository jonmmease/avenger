use std::path::PathBuf;

use avenger_lang::{Compiler, DatasetStageKind};

#[tokio::test]
async fn project_analysis_indexes_chart_sources_and_each_transform_stage() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../avenger-lang-compiler/tests/fixtures/projects/02_sql_pipeline");
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let analysis = compiler.analyze_project(&root).await.unwrap();
    let mut chart_datasets = analysis
        .datasets
        .iter()
        .filter(|(_, dataset)| {
            dataset
                .qualified_name
                .as_deref()
                .is_some_and(|name| name.starts_with("chart:"))
        })
        .map(|(_, dataset)| dataset)
        .collect::<Vec<_>>();
    chart_datasets.sort_by(|left, right| left.stage.cmp(&right.stage));

    assert!(chart_datasets.len() >= 4);
    assert!(matches!(
        chart_datasets[0].provenance.stage_kind,
        DatasetStageKind::DatasetSource
    ));
    let transformed = chart_datasets
        .iter()
        .filter(|dataset| {
            matches!(
                dataset.provenance.stage_kind,
                DatasetStageKind::Transform { .. }
            )
        })
        .collect::<Vec<_>>();
    assert!(
        transformed
            .iter()
            .any(|dataset| { dataset.columns.iter().any(|column| column.name == "total") })
    );
    assert!(transformed.iter().any(|dataset| {
        dataset
            .columns
            .iter()
            .any(|column| column.name == "doubled")
    }));
}
