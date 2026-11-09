//! A session with the marks and the penguins, and the charts of a query.

use std::sync::Arc;

use arrow::{
    array::RecordBatch,
    csv::ReaderBuilder,
    datatypes::{DataType, Field, Schema},
};
use avenger_chart::{charts_from_arrow, Chart, Decode};
use datafusion::{datasource::MemTable, prelude::SessionContext};

/// The 344 penguins of the Palmer Archipelago dataset (CC0, from
/// <https://github.com/allisonhorst/palmerpenguins>, as seaborn-data ships it): `species`,
/// `island`, `bill_length_mm`, `bill_depth_mm`, `flipper_length_mm`, `body_mass_g`, and `sex`.
/// Two penguins have no measurements, and eleven no sex.
pub fn penguins() -> RecordBatch {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/penguins.csv");
    let number = |name| Field::new(name, DataType::Float64, true);
    let schema = Arc::new(Schema::new(vec![
        Field::new("species", DataType::Utf8, false),
        Field::new("island", DataType::Utf8, false),
        number("bill_length_mm"),
        number("bill_depth_mm"),
        number("flipper_length_mm"),
        number("body_mass_g"),
        Field::new("sex", DataType::Utf8, true),
    ]));
    let file = std::fs::File::open(path).expect("the penguins are in tests/data");
    let mut reader = ReaderBuilder::new(schema)
        .with_header(true)
        .with_batch_size(1024)
        .build(file)
        .unwrap();
    reader.next().unwrap().unwrap()
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
