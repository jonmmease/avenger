//! A cross filter of movies, built with the builders of avenger-chart and shown by a chart app.
//!
//! The example reads the movies of `movie-search`. A scatter plot on the left shows each movie's
//! IMDB rating against its Rotten Tomatoes rating, and two scatter plots on the right show each
//! rating against the movie's year, over one axis of the years.
//!
//! A drag on the plot of the ratings draws a box, a drag on the plot of the IMDB ratings by year
//! draws a range of years, and a drag on the plot of the Rotten Tomatoes ratings by year draws a
//! lasso. All three write the selection `filter`. Its resolution is intersect, so each plot keeps
//! its own box, range, or lasso, and a movie matches the selection when it is inside every one.
//! Each plot checks the selection without its own part, so it fades the movies outside the box,
//! the range, and the lasso of the other plots, and its own gesture never fades the movies that it
//! brushes.
//!
//! A drag inside a box or a range moves it, a drag on its edge resizes it, and a Shift+drag adds a
//! path to a lasso. A click in a plot clears its box, range, or lasso.
//!
//! The app reports the changes when each gesture settles, and the example prints the plots that
//! hold a part of each settled selection.

use std::{io::Cursor, sync::Arc};

use arrow::{
    array::RecordBatch,
    compute::{cast, concat_batches},
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

const TITLE: &str = "title";
const IMDB: &str = "imdb_rating";
const ROTTEN_TOMATOES: &str = "rotten_tomatoes_rating";
const YEAR: &str = "year";

/// The selection of the box, the range, and the lasso.
const FILTER: &str = "filter";

/// The size of the window.
const SIZE: [f32; 2] = [1100.0, 700.0];

/// The movies, from the data of `movie-search`, which the binary embeds, with their years as
/// floating-point numbers.
fn movies() -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new(TITLE, DataType::Utf8, false),
        Field::new(IMDB, DataType::Float64, false),
        Field::new(ROTTEN_TOMATOES, DataType::Float64, false),
        Field::new("genre", DataType::Utf8, true),
        Field::new(YEAR, DataType::Int64, false),
    ]));
    let csv = Cursor::new(include_str!("../../movie-search/data/movies.csv"));
    let reader = (ReaderBuilder::new(schema.clone()).with_header(true))
        .build(csv)
        .unwrap();
    let batches = reader.collect::<Result<Vec<_>, _>>().unwrap();
    let movies = concat_batches(&schema, &batches).unwrap();
    let years = cast(movies.column_by_name(YEAR).unwrap(), &DataType::Float64).unwrap();
    let schema = movies.schema();
    let columns = (schema.fields().iter().zip(movies.columns())).map(|(field, column)| match field
        .name()
        .as_str()
    {
        YEAR => (YEAR.to_string(), years.clone()),
        name => (name.to_string(), column.clone()),
    });
    RecordBatch::try_from_iter(columns).unwrap()
}

/// The opacity of a plot's movies: opaque inside the box, the range, and the lasso of the other
/// plots, and faded outside them. The predicate skips the part of the plot that it styles.
fn crossfiltered() -> EncodingBuilder {
    let others = Predicate::selected(Selected::new(FILTER).exclude_own(true));
    Encoding::direct_value(0.05).conditional([ConditionalValue::direct_value(others, 0.7)])
}

/// An encoding of `column` through a scale whose axis has `title` and labels of `format`.
fn axis(column: &str, title: &str, format: &str) -> EncodingBuilder {
    let mut labels = Labels::builder();
    if !format.is_empty() {
        labels = labels.format(format);
    }
    Encoding::scaled_field(column).scale(Axis::builder().title(title).labels(labels))
}

fn imdb() -> EncodingBuilder {
    axis(IMDB, "IMDB rating", "")
}

fn tomatoes() -> EncodingBuilder {
    axis(ROTTEN_TOMATOES, "Rotten Tomatoes rating", "")
}

/// The years, labeled without a separator in the thousands.
fn years() -> EncodingBuilder {
    axis(YEAR, "Year", "d")
}

/// A scatter plot `id` of `x` and `y` titled `title`, whose `select` writes the filter.
fn scatter(
    movies: &RecordBatch,
    id: &str,
    title: &str,
    x: EncodingBuilder,
    y: EncodingBuilder,
    select: SelectBuilder,
) -> PlotBuilder {
    let symbols = CartesianLayer::symbol(
        movies.clone(),
        SymbolMark::builder().x(x).y(y).opacity(crossfiltered()),
    );
    Plot::cartesian([symbols])
        .id(id)
        .title(title)
        .tools([select])
}

/// The chart: the plot of the ratings, where a drag draws a box, beside the plots by year, which
/// share their years, where a drag draws a range of years or a lasso.
fn chart(movies: &RecordBatch) -> Chart {
    let ratings = scatter(
        movies,
        "ratings",
        "Drag a box",
        tomatoes(),
        imdb(),
        Select::interval(FILTER, IntervalSelect::builder()),
    );
    let imdb_by_year = scatter(
        movies,
        "imdb_by_year",
        "Drag a range of years",
        years(),
        imdb(),
        Select::interval(FILTER, IntervalSelect::builder().channels([Channel::X])),
    );
    let tomatoes_by_year = scatter(
        movies,
        "tomatoes_by_year",
        "Drag a lasso, Shift+drag for more",
        years(),
        tomatoes(),
        Select::lasso(FILTER, Vec::<String>::new()),
    );
    let by_year = Concat::new(Direction::Column)
        .resolve([Resolve::channel(Sharing::Shared, Channel::X)])
        .children([imdb_by_year, tomatoes_by_year]);
    Chart::builder(
        Concat::new(Direction::Row).children([NodeBuilder::from(ratings), by_year.into()]),
    )
    .number_format_syntax("d3")
    .datetime_format_syntax("d3")
    .selections([Selection::new(FILTER, SelectionResolution::Intersect)])
    .build()
    .unwrap()
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

/// The plots that hold a part of each selection of a settled change.
fn owners(change: &ChartChange) -> Vec<(String, Vec<String>)> {
    (change.selections.iter())
        .map(|(name, clauses)| {
            let owners = clauses.iter().map(|clause| clause.owner.clone()).collect();
            (name.clone(), owners)
        })
        .collect()
}

/// Prints the plots that hold a part of each settled selection.
fn report(change: &ChartChange) {
    for (name, owners) in owners(change) {
        match owners.is_empty() {
            true => println!("{name}: no plot"),
            false => println!("{name}: {}", owners.join(", ")),
        }
    }
}

fn main() {
    let app = ChartApp::new(chart(&movies()), cascade(), engine())
        .size(SIZE)
        .reports(Reports::Settled)
        .on_change(report);
    let avenger_app = pollster::block_on(app.build()).expect("Failed to create AvengerApp");
    let tokio_runtime = tokio::runtime::Builder::new_multi_thread().build().unwrap();
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
