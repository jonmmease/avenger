//! A session with the marks, the penguins, and the earthquakes, and the charts of a query.

use std::{io::Cursor, sync::Arc};

use arrow::{
    array::{ArrayRef, AsArray, Float64Array, RecordBatch},
    compute::concat_batches,
    csv::ReaderBuilder,
    datatypes::{DataType, Field, Float64Type, Schema},
    ipc::reader::StreamReader,
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

/// The 3,729 earthquakes of magnitude 6 and larger from 2000 through 2024, from the USGS
/// earthquake catalog as avenger-guides-geo ships them: `longitude`, `latitude`, `magnitude`, and
/// `depth_km`, largest first, so that small ones draw on top.
pub fn earthquakes() -> RecordBatch {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../avenger-guides-geo/examples/data/earthquakes.arrows"
    );
    let bytes = std::fs::read(path).expect("the earthquakes of avenger-guides-geo");
    assert!(
        !bytes.starts_with(b"version https://git-lfs"),
        "the earthquakes are a Git LFS pointer file: run `git lfs pull`"
    );
    let reader = StreamReader::try_new(Cursor::new(bytes), None).unwrap();
    let schema = reader.schema();
    let batches = reader.collect::<Result<Vec<_>, _>>().unwrap();
    let quakes = concat_batches(&schema, &batches).unwrap();
    let column = |name: &str| quakes.column_by_name(name).unwrap().clone();
    // The earthquakes are native points with interleaved coordinates.
    let points = column("geometry");
    let coordinates = (points.as_fixed_size_list().values()).as_primitive::<Float64Type>();
    let [longitude, latitude]: [Float64Array; 2] = [0, 1].map(|axis| {
        (coordinates.values().iter().skip(axis).step_by(2))
            .copied()
            .collect()
    });
    RecordBatch::try_from_iter([
        ("longitude", Arc::new(longitude) as ArrayRef),
        ("latitude", Arc::new(latitude)),
        ("magnitude", column("magnitude")),
        ("depth_km", column("depth_km")),
    ])
    .unwrap()
}

/// A session with the marks and the tables `penguins` and `earthquakes`.
pub fn session() -> SessionContext {
    let ctx = SessionContext::new();
    avenger_sql::register(&ctx).unwrap();
    for (name, table) in [("penguins", penguins()), ("earthquakes", earthquakes())] {
        let table = MemTable::try_new(table.schema(), vec![vec![table]]).unwrap();
        ctx.register_table(name, Arc::new(table)).unwrap();
    }
    ctx
}

/// The charts of the column `chart` of `query`, in the order of its rows.
#[allow(dead_code)]
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
