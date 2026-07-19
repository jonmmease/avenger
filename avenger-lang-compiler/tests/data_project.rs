use std::{collections::BTreeMap, path::PathBuf};

use arrow::datatypes::DataType;
use avenger_lang_compiler::{Compiler, DatasetStageKind};

fn project_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/projects")
        .join(name)
}

#[tokio::test]
async fn data_project_propagates_exact_schemas_through_a_multi_query_dag_without_execution() {
    let root = project_fixture("phase9-schema-chain");
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let analysis = compiler.analyze_project(&root).await.unwrap();
    let tables = analysis
        .datasets
        .iter()
        .filter(|(_, dataset)| {
            matches!(
                dataset.provenance.stage_kind,
                DatasetStageKind::CatalogTable | DatasetStageKind::SqlView
            )
        })
        .filter_map(|(_, dataset)| {
            dataset
                .qualified_name
                .as_ref()
                .map(|name| (name.clone(), dataset))
        })
        .collect::<BTreeMap<_, _>>();

    assert_eq!(tables.len(), 6);
    let joined = tables["analytics.joined"];
    assert_eq!(
        joined
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        ["row_id", "category", "amount", "meta", "label"]
    );
    assert!(matches!(joined.columns[3].data_type, DataType::Struct(_)));
    assert!(
        joined.columns[4].nullable,
        "LEFT JOIN must nullable-extend label"
    );

    let adjusted = tables["analytics.adjusted"];
    assert_eq!(
        adjusted
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        [
            "row_id",
            "group_name",
            "amount",
            "adjusted_amount",
            "meta",
            "label",
        ]
    );
    assert!(matches!(adjusted.columns[4].data_type, DataType::Struct(_)));
    assert_eq!(adjusted.columns[3].data_type, DataType::Int64);
    assert!(adjusted.columns[5].nullable);

    let summarized = tables["analytics.summarized"];
    assert_eq!(
        summarized
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        ["group_name", "row_count", "total"]
    );
    assert_eq!(summarized.columns[1].data_type, DataType::Int64);
    assert_eq!(summarized.columns[2].data_type, DataType::Int64);
    assert!(summarized.columns[2].nullable);

    let final_table = tables["analytics.final"];
    assert_eq!(
        final_table
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        ["category", "total", "doubled"]
    );
    assert_eq!(final_table.columns[2].data_type, DataType::Int64);

    compiler.compile_file("chart.avenger").await.unwrap();
}
