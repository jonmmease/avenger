//! A chart that a SQL query builds: the number of movies of each genre, in a chart app.
//!
//! The example reads the movies of `movie-search` into a DataFusion session with the mark
//! functions of avenger-sql, and runs one query, whose `|> CHART` call counts the movies of each
//! genre in a bar chart:
//!
//! ```sql
//! FROM movies
//! |> WHERE genre IS NOT NULL
//! |> CHART bar(x => count(*), y => genre ORDER BY count(*) DESC)
//! ```
//!
//! The query returns one row, whose `chart` column holds the chart, which the chart app shows at
//! each size of the window.

use std::{io::Cursor, sync::Arc};

use arrow::{
    array::RecordBatch,
    compute::concat_batches,
    csv::ReaderBuilder,
    datatypes::{DataType, Field, Schema},
};
use avenger_chart::{charts_from_arrow, Chart, Decode};
use avenger_chart_app::ChartApp;
use avenger_format_datetime_d3::D3DateTimeFormatProvider;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_style::{Cascade, Env, ParseOptions, Sheet, SourceId, StyleTable};
use avenger_typst_label::LabelEngine;
use avenger_winit_wgpu::WinitWgpuAvengerApp;
use datafusion::{datasource::MemTable, prelude::SessionContext};

/// The query that builds the chart.
const QUERY: &str = "FROM movies
|> WHERE genre IS NOT NULL
|> CHART bar(x => count(*), y => genre ORDER BY count(*) DESC)";

/// The size of the window.
const SIZE: [f32; 2] = [800.0, 600.0];

/// The movies, from the data of `movie-search`, which the binary embeds.
fn movies() -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new("title", DataType::Utf8, false),
        Field::new("imdb_rating", DataType::Float64, false),
        Field::new("rotten_tomatoes_rating", DataType::Float64, false),
        Field::new("genre", DataType::Utf8, true),
        Field::new("year", DataType::Int64, false),
    ]));
    let csv = Cursor::new(include_str!("../../movie-search/data/movies.csv"));
    let reader = (ReaderBuilder::new(schema.clone()).with_header(true))
        .build(csv)
        .unwrap();
    let batches = reader.collect::<Result<Vec<_>, _>>().unwrap();
    concat_batches(&schema, &batches).unwrap()
}

/// The chart of the query over the movies.
async fn chart() -> datafusion::error::Result<Chart> {
    let ctx = SessionContext::new();
    avenger_sql::register(&ctx)?;
    let movies = movies();
    let table = MemTable::try_new(movies.schema(), vec![vec![movies]])?;
    ctx.register_table("movies", Arc::new(table))?;
    let batches = avenger_sql::sql(&ctx, QUERY).await?.collect().await?;
    let column = batches[0].column_by_name("chart").expect("a chart column");
    let mut charts = charts_from_arrow(column, Decode::Auto).expect("a column of charts");
    Ok(charts.remove(0).expect("a chart"))
}

fn engine() -> LabelEngine {
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
        .with_datetime_formatting(Arc::new(D3DateTimeFormatProvider::new()))
}

/// The cascade of the base stylesheet.
fn cascade() -> Arc<Cascade> {
    let mut no_files = |url: &str, _: Option<&str>| Err(format!("no file `{url}`"));
    let options = ParseOptions {
        allow_import: true,
        ..Default::default()
    };
    let (sheet, diagnostics) = Sheet::load(
        "@import 'std:base.css';",
        None,
        SourceId(1),
        options,
        &mut no_files,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    Arc::new(Cascade::new(
        &StyleTable::new().with_author(sheet, Env::new()),
    ))
}

fn main() {
    let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let chart = tokio_runtime.block_on(chart()).unwrap();
    let app = ChartApp::new(chart, cascade(), engine()).size(SIZE);
    let avenger_app = pollster::block_on(app.build()).expect("Failed to create AvengerApp");
    let avenger_app =
        (avenger_app.with_runtime(tokio_runtime.handle().clone())).expect("a multi-thread runtime");
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop(avenger_app, 2.0, tokio_runtime);
    event_loop
        .run_app(&mut app)
        .expect("Failed to run event loop");
}

#[cfg(test)]
mod tests {
    use avenger_chart::{proto::node, LayerRef};

    /// The query builds one bar chart, with a row for each genre of the movies.
    #[tokio::test]
    async fn the_query_builds_a_bar_for_each_genre() {
        let chart = super::chart().await.unwrap();
        let Some(node::Node::Plot(plot)) = &chart.root().node else {
            panic!("the root is a plot")
        };
        let layer = LayerRef::of(plot).remove(0);
        assert_eq!(layer.mark().unwrap().name(), "bar");
        let table = chart.table(layer.table().unwrap());
        let names: Vec<&str> = table
            .schema_ref()
            .fields()
            .iter()
            .map(|f| f.name().as_str())
            .collect();
        assert_eq!(names, ["n", "genre"]);
        assert_eq!(table.num_rows(), 12, "the genres of the movies");
    }
}
