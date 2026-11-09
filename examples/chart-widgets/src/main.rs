//! Movies filtered by widgets, built with the builders of avenger-chart and shown by a chart app.
//!
//! The example reads the movies of `movie-search` in six genres and draws each movie's IMDB
//! rating against its Rotten Tomatoes rating, colored by its genre. The layer of the movies
//! declares three widgets that write the selection `filter`: a range slider of the years and a
//! slider of the least IMDB rating above the plot, which share a row while the window is wide
//! enough and wrap into two rows when it is not, and a check box list of the genres beside the
//! plot. Each widget's domain fits the movies. The selection's resolution is intersect, so a movie
//! matches it when it is inside every widget's clause, and the plot fades the movies outside it.
//!
//! A widget at its clear state, a range over every year, a rating at its least, or every genre
//! checked, writes no clause. A tooltip names the movie under the pointer.
//!
//! The app reports the selection when each gesture settles, at the end of a slider's drag and at
//! each click of a check box, and the example prints the clause of each widget. `--svg <PATH>`
//! writes the chart at 960 by 640 pixels as SVG, with the widgets at their clear states, instead
//! of opening a window.

use std::{io::Cursor, sync::Arc};

use arrow::{
    array::{Array, AsArray, RecordBatch},
    compute::{cast, concat_batches, filter_record_batch},
    csv::ReaderBuilder,
    datatypes::{DataType, Field, Schema},
    util::display::array_value_to_string,
};
use avenger_chart::prelude::*;
use avenger_chart_app::{ChartApp, ChartChange, Reports};
use avenger_chart_eval::{ChartEvaluator, Registry};
use avenger_format_datetime_d3::D3DateTimeFormatProvider;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_selection::Condition;
use avenger_style::{Cascade, Env, HostInputs, ParseOptions, Sheet, SourceId, StyleTable};
use avenger_svg::SvgRenderer;
use avenger_typst_label::LabelEngine;
use avenger_winit_wgpu::WinitWgpuAvengerApp;

const TITLE: &str = "title";
const IMDB: &str = "imdb_rating";
const ROTTEN_TOMATOES: &str = "rotten_tomatoes_rating";
const GENRE: &str = "genre";
const YEAR: &str = "year";

/// The genres of the movies.
const GENRES: [&str; 6] = [
    "Action",
    "Adventure",
    "Comedy",
    "Drama",
    "Horror",
    "Thriller/Suspense",
];

/// The selection of the widgets.
const FILTER: &str = "filter";

/// The size of the window, and of the SVG that `--svg` writes.
const SIZE: [f32; 2] = [960.0, 640.0];

/// The movies of the six genres, from the data of `movie-search`, which the binary embeds, with
/// their years as floating-point numbers, so that a slider can run over them.
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
    let genres = movies.column_by_name(GENRE).unwrap().as_string::<i32>();
    let keep = genres
        .iter()
        .map(|genre| Some(genre.is_some_and(|genre| GENRES.contains(&genre))))
        .collect();
    let movies = filter_record_batch(&movies, &keep).unwrap();
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

/// A target of the selection `filter` over `column`.
fn target(column: &str) -> WidgetTargetBuilder {
    WidgetTarget::new(FILTER, column)
}

/// A domain that fits the movies' values of the target's column.
fn fitted() -> WidgetDomainBuilder {
    WidgetDomain::data(WidgetDataDomain::builder())
}

/// The widgets of the movies: the years and the least IMDB rating above the plot, and the genres
/// beside it.
fn widgets() -> [WidgetBuilder; 3] {
    let years = Widget::range_slider(
        "years",
        RangeSliderWidget::new(target(YEAR), fitted())
            .step(1.0)
            .readout(Labels::builder().format("d")),
    )
    .title("Years");
    let rating = Widget::slider(
        "rating",
        SliderWidget::new(target(IMDB), fitted())
            .comparison(SliderComparison::AtLeast)
            .step(0.1)
            .readout(Labels::builder().format(".1f")),
    )
    .title("IMDB rating at least");
    let genres = Widget::checkboxes("genres", CheckboxesWidget::new(target(GENRE), fitted()))
        .side(Side::Right)
        .title("Genres");
    [years, rating, genres]
}

/// The opacity of the movies: opaque inside the selection, and faded outside it.
fn filtered() -> EncodingBuilder {
    let selected = Predicate::selected(FILTER);
    Encoding::direct_value(0.05).conditional([ConditionalValue::direct_value(selected, 0.7)])
}

/// An encoding of `column` through a scale whose axis has `title`.
fn axis(column: &str, title: &str) -> EncodingBuilder {
    Encoding::scaled_field(column).scale(Axis::builder().title(title))
}

/// The tooltip of a movie: its title, its ratings, its genre, and its year.
fn tooltip() -> TooltipBuilder {
    let field = |column: &str, title: &str, format: &str| {
        TooltipField::builder()
            .column(column)
            .title(title)
            .labels(Labels::builder().format(format))
    };
    Tooltip::nearest(NearestFind::builder().max_radius(12.0))
        .title(TooltipField::builder().column(TITLE))
        .fields([
            field(IMDB, "IMDB", ".1f"),
            field(ROTTEN_TOMATOES, "Rotten Tomatoes", "d"),
            TooltipField::builder().column(GENRE).title("Genre"),
            field(YEAR, "Year", "d"),
        ])
}

/// The chart: the movies' ratings, colored by genre, with the widgets that filter them.
fn chart(movies: &RecordBatch) -> Chart {
    let symbols = CartesianLayer::symbol(
        movies.clone(),
        SymbolMark::builder()
            .x(axis(ROTTEN_TOMATOES, "Rotten Tomatoes rating"))
            .y(axis(IMDB, "IMDB rating"))
            .color(GENRE)
            .opacity(filtered()),
    )
    .widgets(widgets());
    Chart::builder(
        Plot::cartesian([symbols])
            .id("movies")
            .tools([ToolBuilder::from(tooltip())]),
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

/// The key of the widget that wrote a clause of `owner`, which names the widget's key and its
/// anchor, apart by NUL characters.
fn widget_key(owner: &str) -> &str {
    owner.split('\0').nth(1).unwrap_or(owner)
}

/// A condition as text: the ends of a range, a null end unbounded, or the values of a set.
fn describe(condition: &Condition) -> String {
    let values = |array: &arrow::array::ArrayRef| {
        (0..array.len())
            .map(|row| match array.is_null(row) {
                true => "…".to_string(),
                false => array_value_to_string(array, row).unwrap_or_default(),
            })
            .collect::<Vec<_>>()
    };
    match condition {
        Condition::Range { column, ends, .. } => {
            format!("{column} in {}", values(ends).join(".."))
        }
        Condition::Set {
            columns,
            values: sets,
            complement,
            ..
        } => {
            let sets = sets
                .iter()
                .map(|set| values(set).join(", "))
                .collect::<Vec<_>>();
            let within = if *complement { "not in" } else { "in" };
            format!("{} {within} [{}]", columns.join(", "), sets.join("; "))
        }
        condition => format!("{condition:?}"),
    }
}

/// The clause of each widget of the settled selection, as text, by the widget's key.
fn clauses(change: &ChartChange) -> Vec<String> {
    (change.selections.iter())
        .flat_map(|(_, clauses)| clauses.iter())
        .map(|clause| {
            let conditions = clause.conditions.iter().map(describe).collect::<Vec<_>>();
            format!(
                "{}: {}",
                widget_key(&clause.owner),
                conditions.join(" and ")
            )
        })
        .collect()
}

/// Prints the clause of each widget of the settled selection.
fn report(change: &ChartChange) {
    let clauses = clauses(change);
    match clauses.is_empty() {
        true => println!("{FILTER}: every movie"),
        false => println!("{FILTER}: {}", clauses.join("; ")),
    }
}

/// Writes the chart at `SIZE` as SVG to `path`.
fn write_svg(path: &str) {
    let engine = engine();
    let evaluator =
        ChartEvaluator::new(chart(&movies()), cascade(), engine.clone(), Registry::new());
    let [width, height] = SIZE;
    let scene = (evaluator
        .unwrap()
        .evaluate(&HostInputs::sized(width, height)))
    .unwrap()
    .scene;
    let svg = SvgRenderer::new()
        .render_scene_graph(&scene, &engine)
        .unwrap();
    std::fs::write(path, svg).unwrap();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let [_, flag, path] = args.as_slice() {
        if flag == "--svg" {
            write_svg(path);
            return;
        }
    }
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
