//! An overview of the movies of each year and a detail of their ratings, linked by a selection,
//! built with avenger-chart and shown by a chart app.
//!
//! The example reads the movies of `movie-search`. The overview at the bottom is an area of the
//! number of movies of each year, and the detail above it is a scatter plot of each movie's IMDB
//! rating against its year. The `x` scale of the detail takes its domain from the selection
//! `focus`, the years that a box in the overview selects, and fits its data while the selection
//! is empty.
//!
//! A drag on the overview draws a box on `x`, which the detail zooms to. A drag inside the box
//! moves it, and a drag on its edge resizes it. A drag on the detail pans its years and the wheel
//! zooms them, which moves the box in the overview. A double click on the detail clears the
//! selection, so the detail shows every year again, and so does a click on the overview. A
//! tooltip names the movie under the pointer.
//!
//! The app reports each change of the selection, and the example prints the years of each
//! settled box.

use std::io::Cursor;
use std::sync::Arc;

use arrow::array::{Array, ArrayRef, Float64Array, Int64Array, RecordBatch};
use arrow::compute::{cast, concat_batches};
use arrow::csv::ReaderBuilder;
use arrow::datatypes::{DataType, Field, Schema};
use avenger_chart::{
    proto::{self, select, Channel, SelectionResolution},
    Chart, Encode, Scale, Selection, TupleTest,
};
use avenger_chart_app::{ChartApp, ChartChange};
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_typst_label::LabelEngine;
use avenger_winit_wgpu::WinitWgpuAvengerApp;

const TITLE: &str = "title";
const IMDB: &str = "imdb_rating";
const ROTTEN_TOMATOES: &str = "rotten_tomatoes_rating";
const YEAR: &str = "year";
const MOVIES: &str = "movies";

/// The selection of the years that the detail shows.
const FOCUS: &str = "focus";

/// The size of the window.
const SIZE: [f32; 2] = [1000.0, 650.0];

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

/// The number of movies of each year from the first year of the movies to the last.
fn per_year(movies: &RecordBatch) -> RecordBatch {
    let years = cast(movies.column_by_name(YEAR).unwrap(), &DataType::Int64).unwrap();
    let years = years.as_any().downcast_ref::<Int64Array>().unwrap();
    let (first, last) = (
        arrow::compute::min(years).unwrap(),
        arrow::compute::max(years).unwrap(),
    );
    let mut counts = vec![0.0; (last - first + 1) as usize];
    for year in years.iter().flatten() {
        counts[(year - first) as usize] += 1.0;
    }
    let columns: [(&str, ArrayRef); 2] = [
        (
            YEAR,
            Arc::new(Float64Array::from_iter_values(
                (first..=last).map(|year| year as f64),
            )),
        ),
        (MOVIES, Arc::new(Float64Array::from(counts))),
    ];
    RecordBatch::try_from_iter(columns).unwrap()
}

fn focus() -> Selection {
    Selection::new(FOCUS, SelectionResolution::Global)
}

/// The years on `x`, labeled without a separator in the thousands, through `scale`.
fn years(scale: Scale) -> Encode {
    let axis = proto::Axis {
        format: "d".into(),
        ..Default::default()
    };
    Encode::column(YEAR).title("Year").scale(scale.guide(axis))
}

/// The tooltip of a movie: its title, its ratings, and its year.
fn tooltip() -> proto::Tooltip {
    let field = |column: &str, title: &str, format: &str| proto::TooltipField {
        column: column.into(),
        title: Some(title.into()),
        format: format.into(),
        ..Default::default()
    };
    proto::Tooltip {
        find: Some(proto::tooltip::Find::Nearest(proto::NearestFind {
            max_radius: 10.0,
            ..Default::default()
        })),
        header: Some(proto::TooltipField {
            column: TITLE.into(),
            ..Default::default()
        }),
        fields: vec![
            field(IMDB, "IMDB", ".1f"),
            field(ROTTEN_TOMATOES, "Rotten Tomatoes", "d"),
            field(YEAR, "Year", "d"),
        ],
        ..Default::default()
    }
}

/// The detail: the movies' IMDB ratings against their years, with the years of `focus`, which
/// a drag pans and the wheel zooms.
fn detail(movies: &RecordBatch) -> Chart {
    Chart::unit(movies.clone(), proto::SymbolMark::default())
        .id("detail")
        .title("Drag to pan the years, scroll to zoom, double click to show every year")
        .tool(proto::Navigate {
            channels: vec![Channel::X as i32],
            ..Default::default()
        })
        .tool(tooltip())
        .x(years(Scale::new().selection(FOCUS, Channel::X)))
        .y(Encode::column(IMDB).title("IMDB rating"))
        .opacity(Encode::constant(0.5))
        .selection(focus())
        .build()
        .unwrap()
}

/// The overview: the number of movies of each year, where a drag draws a box on `x` that writes
/// `focus`.
fn overview(movies: &RecordBatch) -> Chart {
    Chart::unit(per_year(movies), proto::AreaMark::default())
        .id("overview")
        .title("Drag a box of years")
        .tool(proto::Select {
            selection: FOCUS.into(),
            form: Some(select::Form::Interval(proto::IntervalSelect {
                channels: vec![Channel::X as i32],
            })),
            scope: Vec::new(),
        })
        .x(years(Scale::new()))
        .y(Encode::column(MOVIES).title("Movies"))
        .selection(focus())
        .build()
        .unwrap()
}

/// The chart: the detail above the overview, which takes a quarter of the height.
fn chart(movies: &RecordBatch) -> Chart {
    Chart::column([detail(movies), overview(movies)])
        .layout(proto::ConcatLayout {
            tracks: [3.0, 1.0]
                .map(|weight| proto::Track {
                    size: Some(proto::track::Size::Share(proto::Share {
                        weight,
                        max: None,
                    })),
                })
                .to_vec(),
            ..Default::default()
        })
        .build()
        .unwrap()
}

fn engine() -> LabelEngine {
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
}

/// The years of the box of `change`, when it settles a box, or `None` when it settles an empty
/// selection.
fn settled_years(change: &ChartChange) -> Option<Option<[f64; 2]>> {
    let ChartChange::Selection {
        contributions,
        settled: true,
        ..
    } = change
    else {
        return None;
    };
    let ends = (contributions.iter())
        .flat_map(|contribution| contribution.tuples.iter().flatten())
        .find_map(|test| match test {
            TupleTest::Range { ends, .. } => {
                let ends = ends.as_any().downcast_ref::<Float64Array>()?;
                (ends.len() == 2).then(|| [ends.value(0), ends.value(1)])
            }
            _ => None,
        });
    Some(ends)
}

/// Prints the years of each settled box.
fn report(change: &ChartChange) {
    match settled_years(change) {
        Some(Some([from, to])) => println!("{FOCUS}: {from:.1} to {to:.1}"),
        Some(None) => println!("{FOCUS}: every year"),
        None => {}
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
