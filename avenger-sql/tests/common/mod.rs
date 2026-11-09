//! A session with the marks and a small table of penguins, and the charts of a query.

use std::sync::Arc;

use arrow::array::{Float64Array, RecordBatch, StringArray};
use avenger_chart::{charts_from_arrow, Chart, Decode};
use datafusion::{datasource::MemTable, prelude::SessionContext};

/// Ten penguins: a species, an island, and a body mass in grams.
pub fn penguins() -> RecordBatch {
    let rows = [
        ("Adelie", "Torgersen", 3750.0),
        ("Adelie", "Biscoe", 3800.0),
        ("Adelie", "Dream", 3250.0),
        ("Adelie", "Dream", 3450.0),
        ("Chinstrap", "Dream", 3500.0),
        ("Chinstrap", "Dream", 3900.0),
        ("Gentoo", "Biscoe", 4500.0),
        ("Gentoo", "Biscoe", 5700.0),
        ("Gentoo", "Biscoe", 4450.0),
        ("Gentoo", "Biscoe", 5050.0),
    ];
    RecordBatch::try_from_iter(vec![
        (
            "species",
            Arc::new(StringArray::from_iter_values(rows.iter().map(|r| r.0))) as _,
        ),
        (
            "island",
            Arc::new(StringArray::from_iter_values(rows.iter().map(|r| r.1))) as _,
        ),
        (
            "body_mass",
            Arc::new(Float64Array::from_iter_values(rows.iter().map(|r| r.2))) as _,
        ),
    ])
    .unwrap()
}

/// A session with the marks and the table `penguins`.
pub fn session() -> SessionContext {
    let ctx = SessionContext::new();
    avenger_sql::register(&ctx);
    let table = penguins();
    let table = MemTable::try_new(table.schema(), vec![vec![table]]).unwrap();
    ctx.register_table("penguins", Arc::new(table)).unwrap();
    ctx
}

/// The charts of the column `chart` of `query`, in the order of its rows.
pub async fn charts(query: &str) -> Vec<Chart> {
    let ctx = session();
    let batches = avenger_sql::sql(&ctx, query)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let mut charts = Vec::new();
    for batch in batches {
        let column = batch.column_by_name("chart").expect("a chart column");
        for chart in charts_from_arrow(column, Decode::Auto).unwrap() {
            charts.push(chart.unwrap());
        }
    }
    charts
}
