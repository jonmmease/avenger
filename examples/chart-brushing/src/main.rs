//! Linked selections of movies, built with avenger-chart and shown by a chart app.
//!
//! The example reads the movies of `movie-search` in the seven most common genres. Two scatter
//! plots show each movie's IMDB rating against its Rotten Tomatoes rating and against its year,
//! and a bar chart below them counts the movies of each genre. The plots share one color scale of
//! the genres.
//!
//! A drag on the plot of the ratings draws a box, and a drag on the plot of the years draws a
//! lasso. Both write the global selection `brush`, so a new box replaces a lasso and a new lasso
//! replaces a box. A drag inside a box moves it, a drag on its edge resizes it, and a click
//! clears it. A Shift+drag adds a path to a lasso. A click on a bar selects its genre, a
//! Shift+click adds or removes a genre, and a click on the bar chart away from the bars clears
//! the genres. The movies that both selections hold stay opaque in the scatter plots while the
//! others fade, and the bars of the genres that the clicks leave out fade. A tooltip names the
//! movie under the pointer.
//!
//! The app reports each change of a selection, and the example prints the number of tuples of
//! each settled selection.

use std::io::Cursor;
use std::sync::Arc;

use arrow::array::{ArrayRef, BooleanArray, Float64Array, RecordBatch, StringArray};
use arrow::compute::{cast, concat_batches, filter_record_batch};
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
const GENRE: &str = "genre";
const YEAR: &str = "year";
const MOVIES: &str = "movies";

/// The selection of the box and the lasso.
const BRUSH: &str = "brush";
/// The selection of the clicks on the bars.
const GENRES: &str = "genres";

/// The genres that the example shows, from the most movies to the fewest.
const SHOWN: [&str; 7] = [
    "Drama",
    "Comedy",
    "Action",
    "Adventure",
    "Thriller/Suspense",
    "Horror",
    "Romantic Comedy",
];

/// The size of the window.
const SIZE: [f32; 2] = [1100.0, 720.0];

/// The movies of the shown genres, from the data of `movie-search`, which the binary embeds, with
/// their years as floating-point numbers.
fn movies() -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new(TITLE, DataType::Utf8, false),
        Field::new(IMDB, DataType::Float64, false),
        Field::new(ROTTEN_TOMATOES, DataType::Float64, false),
        Field::new(GENRE, DataType::Utf8, true),
        Field::new(YEAR, DataType::Int64, false),
    ]));
    let csv = Cursor::new(include_str!("../../movie-search/data/movies.csv"));
    let reader = (ReaderBuilder::new(schema.clone()).with_header(true))
        .build(csv)
        .unwrap();
    let batches = reader.collect::<Result<Vec<_>, _>>().unwrap();
    let movies = concat_batches(&schema, &batches).unwrap();
    let genres = movies.column_by_name(GENRE).unwrap();
    let genres = genres.as_any().downcast_ref::<StringArray>().unwrap();
    let shown: BooleanArray = (genres.iter())
        .map(|genre| Some(genre.is_some_and(|genre| SHOWN.contains(&genre))))
        .collect();
    let movies = filter_record_batch(&movies, &shown).unwrap();
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

/// The number of movies of each shown genre.
fn counts(movies: &RecordBatch) -> RecordBatch {
    let genres = movies.column_by_name(GENRE).unwrap();
    let genres = genres.as_any().downcast_ref::<StringArray>().unwrap();
    let counts = SHOWN.map(|shown| genres.iter().filter(|genre| *genre == Some(shown)).count());
    let columns: [(&str, ArrayRef); 2] = [
        (GENRE, Arc::new(StringArray::from(SHOWN.to_vec()))),
        (
            MOVIES,
            Arc::new(Float64Array::from_iter_values(
                counts.map(|count| count as f64),
            )),
        ),
    ];
    RecordBatch::try_from_iter(columns).unwrap()
}

/// A predicate of the rows that the selection `name` holds. A selection without contributions
/// holds every row.
fn holds(name: &str) -> proto::Predicate {
    proto::Predicate {
        predicate: Some(proto::predicate::Predicate::Selection(
            proto::SelectionTest {
                selection: name.into(),
                ..Default::default()
            },
        )),
    }
}

/// A predicate of the rows that both the box or lasso and the clicks hold.
fn both() -> proto::Predicate {
    proto::Predicate {
        predicate: Some(proto::predicate::Predicate::And(proto::PredicateList {
            predicates: vec![holds(BRUSH), holds(GENRES)],
        })),
    }
}

/// The opacity of a scatter plot: opaque for the movies that both selections hold, and faded for
/// the others.
fn faded() -> Encode {
    Encode::constant(0.05).when(both(), Value::Constant(0.75.into()))
}

/// The selection of the box and the lasso. Each plot defines the selections that it names, so
/// each plot is a valid chart of its own.
fn brush() -> Selection {
    Selection::new(BRUSH, SelectionResolution::Global)
}

/// The selection of the clicks on the bars.
fn clicked() -> Selection {
    Selection::new(GENRES, SelectionResolution::Global)
}

/// The tooltip of a movie: its title, its ratings, its genre, and its year.
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
            field(GENRE, "Genre", ""),
            field(YEAR, "Year", "d"),
        ],
        ..Default::default()
    }
}

/// A selection of `form` that writes `selection`.
fn select(selection: &str, form: select::Form) -> proto::Select {
    proto::Select {
        selection: selection.into(),
        form: Some(form),
        scope: Vec::new(),
    }
}

/// The IMDB ratings, which both scatter plots show on `y`.
fn imdb() -> Encode {
    Encode::column(IMDB).title("IMDB rating")
}

/// The genres, which color every plot.
fn genre() -> Encode {
    Encode::column(GENRE).title("Genre")
}

/// The scatter plot of the ratings, where a drag draws a box on both axes.
fn ratings(movies: &RecordBatch) -> Chart {
    Chart::unit(movies.clone(), proto::SymbolMark::default())
        .id("ratings")
        .title("Drag a box")
        .tool(select(
            BRUSH,
            select::Form::Interval(proto::IntervalSelect::default()),
        ))
        .tool(tooltip())
        .x(Encode::column(ROTTEN_TOMATOES).title("Rotten Tomatoes rating"))
        .y(imdb())
        .color(genre())
        .opacity(faded())
        .selection(brush())
        .selection(clicked())
        .build()
        .unwrap()
}

/// The scatter plot of the years, where a drag draws a lasso.
fn years(movies: &RecordBatch) -> Chart {
    let years = proto::Axis {
        format: "d".into(),
        ..Default::default()
    };
    Chart::unit(movies.clone(), proto::SymbolMark::default())
        .id("years")
        .title("Drag a lasso")
        .tool(select(
            BRUSH,
            select::Form::Lasso(proto::LassoSelect::default()),
        ))
        .tool(tooltip())
        .x(Encode::column(YEAR)
            .title("Year")
            .scale(Scale::new().guide(years)))
        .y(imdb())
        .color(genre())
        .opacity(faded())
        .selection(brush())
        .selection(clicked())
        .build()
        .unwrap()
}

/// The bar chart of the genres, where a click selects a genre.
fn genres(movies: &RecordBatch) -> Chart {
    let order = Scale::new().domain(Arc::new(StringArray::from(SHOWN.to_vec())));
    Chart::unit(counts(movies), proto::BarMark::default())
        .id("genres")
        .title("Click a bar, Shift+click for more")
        .tool(select(
            GENRES,
            select::Form::Points(proto::PointSelect {
                key: GENRE.into(),
                toggle: true,
            }),
        ))
        .x(genre().scale(order))
        .y(Encode::column(MOVIES).title("Movies"))
        .color(genre())
        .opacity(Encode::constant(0.2).when(holds(GENRES), Value::Constant(1.0.into())))
        .selection(clicked())
        .build()
        .unwrap()
}

/// The chart: the two scatter plots above the bar chart.
fn chart(movies: &RecordBatch) -> Chart {
    let scatters = Chart::row([ratings(movies), years(movies)])
        .share(Channel::Y)
        .build()
        .unwrap();
    Chart::column([scatters, genres(movies)])
        .layout(proto::ConcatLayout {
            tracks: [2.0, 1.0]
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

/// Prints the number of tuples of each settled selection.
fn report(change: &ChartChange) {
    if !change.settled {
        return;
    }
    for (name, contributions) in &change.selections {
        let tuples: usize = (contributions.iter())
            .map(|contribution| contribution.tuples.len())
            .sum();
        println!("{name}: {tuples} tuples");
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
