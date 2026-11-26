//! A project of files in memory builds its charts, takes the changes of an editor, and answers its
//! requests, natively and, under `wasm-pack test`, in a browser.

use std::sync::Arc;

use avenger_sql_web::SqlProject;
use datafusion::arrow::{
    array::{Int64Array, RecordBatch, StringArray},
    datatypes::{DataType, Field, Schema},
};

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const CHART: &str = "charts/revenue.av.sql";
const BARS: &str = "FROM sales |> AGGREGATE mark_bar(x => region, y => SUM(revenue)) AS chart";

/// A Parquet file of the revenues of the regions `north` and `south`.
fn parquet() -> Vec<u8> {
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("region", DataType::Utf8, false),
            Field::new("revenue", DataType::Int64, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec!["north", "south"])),
            Arc::new(Int64Array::from(vec![30, 40])),
        ],
    )
    .unwrap();
    let mut bytes = Vec::new();
    let mut writer =
        datafusion::parquet::arrow::ArrowWriter::try_new(&mut bytes, batch.schema(), None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
    bytes
}

fn files() -> Vec<(String, Vec<u8>)> {
    [
        ("avenger.toml", b"[project]\nname = \"sales\"\n".to_vec()),
        (
            "data/sources.av.sql",
            b"CREATE EXTERNAL TABLE sales OPTIONS (format = 'CSV', uris = ['data/files/sales.csv']);\n\
              CREATE EXTERNAL TABLE more OPTIONS (format = 'PARQUET', uris = ['data/files/more.parquet']);\n"
                .to_vec(),
        ),
        ("data/files/sales.csv", b"region,revenue\neast,10\nwest,20\neast,5\n".to_vec()),
        ("data/files/more.parquet", parquet()),
        (CHART, BARS.as_bytes().to_vec()),
    ]
    .into_iter()
    .map(|(path, bytes)| (path.to_string(), bytes))
    .collect()
}

async fn a_project_builds_its_charts_and_answers_an_editor() {
    let project = SqlProject::open("sales", files()).await.unwrap();
    assert_eq!(project.chart_scripts(), [CHART]);
    assert!(project.diagnostics(CHART).is_empty());
    let first = project.charts(CHART).await.unwrap();
    assert_eq!(first.len(), 1);

    // A chart of the Parquet file's table.
    let parquet = BARS.replace("FROM sales", "FROM more");
    project.set_text(CHART, parquet).await.unwrap();
    let other = project.charts(CHART).await.unwrap();
    assert_ne!(other, first);

    // A data script that changes loads again, and the chart reads its new table.
    let sources =
        "CREATE EXTERNAL TABLE sales OPTIONS (format = 'CSV', uris = ['data/files/sales.csv']);\n\
                   CREATE VIEW more AS (FROM sales |> WHERE region = 'east');\n";
    project
        .set_text("data/sources.av.sql", sources.into())
        .await
        .unwrap();
    let east = project.charts(CHART).await.unwrap();
    assert_ne!(east, other);

    // The editor's requests.
    let typed = "FROM sales |> WHERE ";
    project.set_text(CHART, typed.into()).await.unwrap();
    let completions = project.completions(CHART, typed.len() as u32).unwrap();
    assert!(completions.items.iter().any(|item| item.label == "revenue"));
    assert!(!project.diagnostics(CHART).is_empty());
    project.set_text(CHART, BARS.into()).await.unwrap();
    let at = BARS.find("SUM").unwrap() as u32 + 1;
    assert!(project.hover(CHART, at).is_some());
    assert!(!project.symbols(CHART).is_empty());
}

#[cfg(not(target_arch = "wasm32"))]
#[tokio::test]
async fn a_project_runs_natively() {
    a_project_builds_its_charts_and_answers_an_editor().await;
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen_test::wasm_bindgen_test]
async fn a_project_runs_in_a_browser() {
    a_project_builds_its_charts_and_answers_an_editor().await;
}
