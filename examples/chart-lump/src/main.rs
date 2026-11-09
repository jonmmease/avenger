//! The top genres of movies, lumped by the `lump` transform of avenger-datafusion-transforms in
//! DataFusion, and shown by a chart app.
//!
//! The example reads the movies of `movie-search` into DataFusion and lumps their genres to the 5
//! with the most movies, with the rest, the movies without a genre among them, in the Other group:
//!
//! ```sql
//! SELECT *, lump(genre, 'n', 5) OVER () AS lumped FROM movies
//! ```
//!
//! The bars on the left count the movies of each lumped genre, with the kept genres by their
//! counts and the Other bar last, gray with stripes, labeled with its number of movies. The plot on
//! the right shows each movie's IMDB rating against its Rotten Tomatoes rating, colored by its
//! lumped genre, with a legend that ends with Other and a tooltip of the movie.
//!
//! A click on a bar selects its genre, and a click on the Other bar selects every genre outside
//! the kept ones. A Shift+click adds a bar to the selection or removes it. The selection names the
//! source column `genre`, not the lumps, so the movies of the scatter plot test their own genres,
//! and a query could filter its rows by the selection before it lumps them again. Both plots gray
//! the bars and the movies outside the selection.

use std::{io::Cursor, sync::Arc};

use arrow::{
    array::RecordBatch,
    compute::concat_batches,
    csv::ReaderBuilder,
    datatypes::{DataType, Field, Schema},
};
use avenger_chart::prelude::*;
use avenger_chart_app::{ChartApp, ChartChange, Reports};
use avenger_format_datetime_d3::D3DateTimeFormatProvider;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_style::{Cascade, Env, ParseOptions, Sheet, SourceId, StyleTable};
use avenger_typst_label::LabelEngine;
use avenger_winit_wgpu::WinitWgpuAvengerApp;
use datafusion::prelude::SessionContext;

/// The number of genres that the lump keeps.
const KEPT: usize = 5;

/// The selection of the genres.
const GENRES: &str = "genres";

/// The size of the window.
const SIZE: [f32; 2] = [1200.0, 600.0];

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

/// The tables of the chart, from the movies through DataFusion: each movie with its lumped genre,
/// and the number of movies of each lumped genre.
struct Tables {
    movies: RecordBatch,
    genres: RecordBatch,
}

async fn tables(movies: RecordBatch) -> datafusion::error::Result<Tables> {
    let ctx = SessionContext::new();
    avenger_datafusion_transforms::register(&ctx)?;
    ctx.register_batch("movies", movies)?;
    let lumped = format!("SELECT *, lump(genre, 'n', {KEPT}) OVER () AS lumped FROM movies");
    ctx.register_table("lumped", ctx.sql(&lumped).await?.into_view())?;
    let collect = |sql: &'static str| {
        let ctx = ctx.clone();
        async move {
            let frame = ctx.sql(sql).await?;
            let schema = Arc::new(frame.schema().as_arrow().clone());
            let batches = frame.collect().await?;
            Ok::<_, datafusion::error::DataFusionError>(concat_batches(&schema, &batches)?)
        }
    };
    Ok(Tables {
        movies: collect("SELECT * FROM lumped").await?,
        genres: collect("SELECT lumped, count(*) AS movies FROM lumped GROUP BY lumped").await?,
    })
}

/// The color of a lumped genre, light gray outside the selection.
fn color() -> EncodingBuilder {
    Encoding::scaled_field("lumped")
        .scale(Legend::builder().title("Genre"))
        .conditional([ConditionalValue::direct_value(
            Predicate::negate(Predicate::selected(GENRES)),
            "lightgray",
        )])
}

/// The bars of the number of movies of each lumped genre, where a click selects a genre and a
/// click on the Other bar selects every genre outside the kept ones.
fn bars(genres: &RecordBatch) -> PlotBuilder {
    // The label and the tooltip title of the Other bar name the number of genres it holds.
    let genre = || Labels::builder().markup("#if other [#value (#count)] else [#value]");
    let x = Encoding::scaled_field("lumped").scale(Axis::builder().title("Genre").labels(genre()));
    let y = Encoding::scaled_field("movies").scale(Axis::builder().title("Movies"));
    let tooltip = Tooltip::exact()
        .title(TooltipField::builder().column("lumped").labels(genre()))
        .fields([TooltipField::builder().column("movies").title("Movies")]);
    let bars = CartesianLayer::bar(genres.clone(), BarMark::builder().x(x).y(y).color(color()))
        .id("bars")
        .tools([LayerToolBuilder::from(tooltip)]);
    let points = PointSelect::builder()
        .keys(["lumped"])
        .toggle(true)
        .sources([KeySource::new("lumped", "genre")]);
    Plot::cartesian([bars])
        .id("genres")
        .title("Click a genre, Shift+click for more")
        .tools([Select::points(GENRES, points)])
}

/// The ratings of the movies, colored by their lumped genres.
fn ratings(movies: &RecordBatch) -> PlotBuilder {
    let x = Encoding::scaled_field("rotten_tomatoes_rating")
        .scale(Axis::builder().title("Rotten Tomatoes rating"));
    let y = Encoding::scaled_field("imdb_rating").scale(Axis::builder().title("IMDB rating"));
    let tooltip = Tooltip::nearest(NearestFind::builder().max_radius(10.0))
        .title(TooltipField::builder().column("title"))
        .fields([
            TooltipField::builder().column("genre").title("Genre"),
            TooltipField::builder().column("lumped").title("Group"),
            TooltipField::builder()
                .column("year")
                .title("Year")
                .labels(Labels::builder().format("d")),
        ]);
    let symbols = CartesianLayer::symbol(
        movies.clone(),
        SymbolMark::builder().x(x).y(y).color(color()).opacity(0.7),
    )
    .id("movies")
    .tools([LayerToolBuilder::from(tooltip)]);
    Plot::cartesian([symbols]).id("ratings").title("Ratings")
}

/// The chart: the bars of the lumped genres beside the ratings of the movies.
fn chart(tables: &Tables) -> Chart {
    Chart::builder(Concat::new(Direction::Row).children([
        NodeBuilder::from(bars(&tables.genres)),
        ratings(&tables.movies).into(),
    ]))
    .number_format_syntax("d3")
    .datetime_format_syntax("d3")
    .selections([Selection::new(GENRES, SelectionResolution::Union)])
    .build()
    .unwrap()
}

fn engine() -> LabelEngine {
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
        .with_datetime_formatting(Arc::new(D3DateTimeFormatProvider::new()))
}

/// The stylesheet: the base stylesheet, with the Other group black, so it stands apart from the
/// light gray of the movies outside the selection, and the Other bar striped.
const STYLES: &str = r#"
@import 'std:base.css';
mark > other { color: black; }
mark[type=bar] > other { pattern: "diagonal-down"; }
"#;

/// The cascade of the stylesheet.
fn cascade() -> Arc<Cascade> {
    let mut no_files = |url: &str, _: Option<&str>| Err(format!("no file `{url}`"));
    let options = ParseOptions {
        allow_import: true,
        ..Default::default()
    };
    let (sheet, diagnostics) = Sheet::load(STYLES, None, SourceId(1), options, &mut no_files);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    Arc::new(Cascade::new(
        &StyleTable::new().with_author(sheet, Env::new()),
    ))
}

/// Prints the genres of each settled selection.
fn report(change: &ChartChange) {
    for (name, clauses) in &change.selections {
        if clauses.is_empty() {
            println!("{name}: every genre");
        }
        for clause in clauses {
            println!("{name}: {:?}", clause.conditions);
        }
    }
}

fn main() {
    let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let tables = tokio_runtime.block_on(tables(movies())).unwrap();
    let app = ChartApp::new(chart(&tables), cascade(), engine())
        .size(SIZE)
        .reports(Reports::Settled)
        .on_change(report);
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
mod tests;
