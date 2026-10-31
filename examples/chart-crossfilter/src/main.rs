//! A cross filter of movies, built with avenger-chart and shown by a chart app.
//!
//! The example reads the movies of `movie-search`. A scatter plot on the left shows each movie's
//! IMDB rating against its Rotten Tomatoes rating, and two scatter plots on the right show each
//! rating against the movie's year, over one axis of the years.
//!
//! A drag on the plot of the ratings draws a box, a drag on the plot of the IMDB ratings by year
//! draws a range of years, and a drag on the plot of the Rotten Tomatoes ratings by year draws a
//! lasso. All three write the selection `filter`. Its resolution is intersect, so each plot
//! keeps its own box, range, or lasso, and a movie matches the selection when it is inside every
//! one. Each plot tests the selection without its own part, so it fades the movies outside the
//! box, the range, and the lasso of the other plots, and its own gesture never fades the movies
//! that it brushes.
//!
//! A drag inside a box or a range moves it, a drag on its edge resizes it, and a Shift+drag adds
//! a path to a lasso. A click in a plot clears its box, range, or lasso.
//!
//! The app reports each change of the selection, and the example prints the plots that hold a
//! part of each settled selection.

use std::io::Cursor;
use std::sync::Arc;

use arrow::array::RecordBatch;
use arrow::compute::{cast, concat_batches};
use arrow::csv::ReaderBuilder;
use arrow::datatypes::{DataType, Field, Schema};
use avenger_chart::{
    proto::{self, select, Channel, SelectionResolution},
    Chart, Encode, Scale, Selection, Value,
};
use avenger_chart_app::{ChartApp, ChartChange};
use avenger_format_number_d3::D3NumberFormatProvider;
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

/// The selection of the box, the range, and the lasso. A new gesture replaces only the part of
/// its own plot, and a movie matches the selection when it matches every part. Each plot
/// defines the selection, so each plot is a valid chart of its own.
fn filter() -> Selection {
    Selection::new(FILTER, SelectionResolution::Intersect)
}

/// The opacity of a plot: opaque for the movies inside the box, the range, and the lasso of the
/// other plots, and faded for the others. The test skips the part of the plot that it styles.
fn crossfiltered() -> Encode {
    let others = proto::Predicate {
        predicate: Some(proto::predicate::Predicate::Selection(
            proto::SelectionTest {
                selection: FILTER.into(),
                exclude_own: true,
                ..Default::default()
            },
        )),
    };
    Encode::constant(0.05).when(others, Value::Constant(0.7.into()))
}

/// A gesture of `form` that writes the selection `filter`.
fn select(form: select::Form) -> proto::Select {
    proto::Select {
        selection: FILTER.into(),
        form: Some(form),
        scope: Vec::new(),
    }
}

fn imdb() -> Encode {
    Encode::column(IMDB).title("IMDB rating")
}

fn tomatoes() -> Encode {
    Encode::column(ROTTEN_TOMATOES).title("Rotten Tomatoes rating")
}

/// The years, labeled without a separator in the thousands.
fn years() -> Encode {
    let axis = proto::Axis {
        format: "d".into(),
        ..Default::default()
    };
    Encode::column(YEAR)
        .title("Year")
        .scale(Scale::new().guide(axis))
}

/// The scatter plot of the ratings, where a drag draws a box on both axes.
fn ratings(movies: &RecordBatch) -> Chart {
    Chart::unit(movies.clone(), proto::SymbolMark::default())
        .id("ratings")
        .title("Drag a box")
        .tool(select(select::Form::Interval(
            proto::IntervalSelect::default(),
        )))
        .x(tomatoes())
        .y(imdb())
        .opacity(crossfiltered())
        .selection(filter())
        .build()
        .unwrap()
}

/// The scatter plot of the IMDB ratings by year, where a drag draws a range of years.
fn imdb_by_year(movies: &RecordBatch) -> Chart {
    Chart::unit(movies.clone(), proto::SymbolMark::default())
        .id("imdb_by_year")
        .title("Drag a range of years")
        .tool(select(select::Form::Interval(proto::IntervalSelect {
            channels: vec![Channel::X as i32],
        })))
        .x(years())
        .y(imdb())
        .opacity(crossfiltered())
        .selection(filter())
        .build()
        .unwrap()
}

/// The scatter plot of the Rotten Tomatoes ratings by year, where a drag draws a lasso.
fn tomatoes_by_year(movies: &RecordBatch) -> Chart {
    Chart::unit(movies.clone(), proto::SymbolMark::default())
        .id("tomatoes_by_year")
        .title("Drag a lasso, Shift+drag for more")
        .tool(select(select::Form::Lasso(proto::LassoSelect {})))
        .x(years())
        .y(tomatoes())
        .opacity(crossfiltered())
        .selection(filter())
        .build()
        .unwrap()
}

/// The chart: the plot of the ratings beside the two plots by year, which share their years.
fn chart(movies: &RecordBatch) -> Chart {
    let by_year = Chart::column([imdb_by_year(movies), tomatoes_by_year(movies)])
        .share(Channel::X)
        .build()
        .unwrap();
    Chart::row([ratings(movies), by_year]).build().unwrap()
}

fn engine() -> LabelEngine {
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
}

/// Prints the plots that hold a part of each settled selection.
fn report(change: &ChartChange) {
    if let ChartChange::Selection {
        name,
        contributions,
        settled: true,
    } = change
    {
        let owners: Vec<&str> = (contributions.iter())
            .map(|contribution| contribution.owner.as_str())
            .collect();
        if owners.is_empty() {
            println!("{name}: no plot");
        } else {
            println!("{name}: {}", owners.join(", "));
        }
    }
}

fn main() {
    let app = ChartApp::new(chart(&movies()), engine())
        .size(SIZE)
        .on_change(report);
    let avenger_app = pollster::block_on(app.build()).expect("Failed to create AvengerApp");
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop(avenger_app, 2.0, tokio_runtime);
    event_loop
        .run_app(&mut app)
        .expect("Failed to run event loop");
}

#[cfg(test)]
mod tests;
