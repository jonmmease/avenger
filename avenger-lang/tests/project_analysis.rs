use std::path::PathBuf;

use avenger_lang::{Compiler, DatasetStageKind};

#[tokio::test]
async fn project_analysis_indexes_chart_sources_and_each_transform_stage() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../avenger-lang-compiler/tests/fixtures/projects/02_sql_pipeline");
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let analysis = compiler
        .analyze_module(root.join("chart.avenger"))
        .await
        .unwrap();
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

    let mut by_dataset = std::collections::BTreeMap::new();
    for stage in chart_datasets {
        by_dataset
            .entry(stage.id.clone())
            .or_insert_with(Vec::new)
            .push(stage);
    }
    let pipeline = by_dataset
        .values_mut()
        .find(|stages| stages.len() == 3)
        .expect("the named group owns source, aggregate, and SQL stages");
    pipeline.sort_by_key(|stage| stage.stage.ordinal);
    assert_eq!(
        pipeline
            .iter()
            .map(|stage| stage.stage.ordinal)
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert_eq!(
        pipeline[0]
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        ["amount", "category"]
    );
    assert_eq!(
        pipeline[1]
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        ["category", "total"]
    );
    assert_eq!(
        pipeline[2]
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        ["category", "doubled"]
    );
    for stages in by_dataset.values() {
        for (index, stage) in stages.iter().enumerate() {
            let lineage = analysis.lineage.get(&stage.stage).unwrap();
            assert_eq!(lineage.upstream_stages.len(), usize::from(index > 0));
        }
    }
}
