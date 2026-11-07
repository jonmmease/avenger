//! A grid of scatter plots of movies' ratings, built with the builders of avenger-chart and drawn
//! at the window's size by a chart app.
//!
//! The example reads the movies of `movie-search`, names the era of each movie with Arrow compute,
//! and partitions the movies of three genres by era, in the order of the years, and by genre, in
//! the order of the names: a facet column of the eras, each a facet row of the genres. The headers name each genre above its column and each era beside its
//! row. Every panel shares the scale of IMDB ratings and the color scale of the years, so the grid
//! draws one colorbar. The panels of a genre share a scale of Rotten Tomatoes ratings through a
//! family named for the genre, so each column has its own axis.
//!
//! Each panel declares a navigation and a tooltip. A drag or the wheel pans and zooms the panel's
//! scales, so the panels of its genre and every panel of the grid move with it, and a double click
//! returns them to their fitted domains. A tooltip names the movie under the pointer.
//!
//! The chart goes through its bytes, as a chart stored in an Arrow column would, before the app
//! shows it. `--svg <PATH>` writes the chart at 960 by 640 pixels as SVG instead of opening a
//! window.

use std::{io::Cursor, sync::Arc};

use arrow::{
    array::{ArrayRef, AsArray, Int64Array, RecordBatch, StringArray},
    compute::{concat_batches, filter_record_batch, sort_to_indices, take_record_batch},
    csv::ReaderBuilder,
    datatypes::{DataType, Field, Int64Type, Schema},
};
use avenger_chart::prelude::*;
use avenger_chart_app::ChartApp;
use avenger_chart_eval::{ChartEvaluator, Registry};
use avenger_format_datetime_d3::D3DateTimeFormatProvider;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_style::{Cascade, Env, HostInputs, ParseOptions, Sheet, SourceId, StyleTable};
use avenger_svg::SvgRenderer;
use avenger_typst_label::LabelEngine;
use avenger_winit_wgpu::WinitWgpuAvengerApp;

const IMDB: &str = "imdb_rating";
const ROTTEN_TOMATOES: &str = "rotten_tomatoes_rating";
const GENRE: &str = "genre";
const YEAR: &str = "year";
const ERA: &str = "era";

/// The genres of the columns of the grid.
const GENRES: [&str; 3] = ["Drama", "Comedy", "Action"];

/// The eras of the rows of the grid: each name with the first year after it.
const ERAS: [(&str, i64); 3] = [("Before 1990", 1990), ("1990s", 2000), ("2000s", i64::MAX)];

/// The size of the window, and of the SVG that `--svg` writes.
const SIZE: [f32; 2] = [960.0, 640.0];

/// The movies of the three genres, from the data of `movie-search`, which the binary embeds, with
/// the era of each.
fn movies() -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new("title", DataType::Utf8, false),
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
    let years: &Int64Array = movies
        .column_by_name(YEAR)
        .unwrap()
        .as_primitive::<Int64Type>();
    let eras: StringArray = years
        .values()
        .iter()
        .map(|year| {
            ERAS.iter()
                .find(|(_, until)| year < until)
                .map(|(era, _)| *era)
        })
        .collect();
    let mut columns = movies.columns().to_vec();
    columns.push(Arc::new(eras) as ArrayRef);
    let mut fields: Vec<Field> = (schema.fields().iter())
        .map(|field| field.as_ref().clone())
        .collect();
    fields.push(Field::new(ERA, DataType::Utf8, true));
    let movies = RecordBatch::try_new(Arc::new(Schema::new(fields)), columns).unwrap();
    // In the order of their years, so the eras come in their order.
    let order = sort_to_indices(movies.column_by_name(YEAR).unwrap(), None, None).unwrap();
    take_record_batch(&movies, &order).unwrap()
}

/// The tooltip of a movie: its title, its ratings, and its year.
fn tooltip() -> TooltipBuilder {
    let field = |column: &str, title: &str, format: &str| {
        TooltipField::builder()
            .column(column)
            .title(title)
            .format(format)
    };
    Tooltip::nearest(NearestFind::builder().max_radius(12.0))
        .header(TooltipField::builder().column("title"))
        .fields([
            field(IMDB, "IMDB", ".1f"),
            field(ROTTEN_TOMATOES, "Rotten Tomatoes", "d"),
            field(YEAR, "Year", "d"),
        ])
}

/// The panel of the movies of one genre in one era, `movies`, with a navigation and a tooltip.
fn panel(movies: RecordBatch) -> PlotBuilder {
    let genre = movies
        .column_by_name(GENRE)
        .unwrap()
        .as_string::<i32>()
        .value(0)
        .to_string();
    let symbols = CartesianLayer::symbol(
        SymbolMark::builder()
            .x(Encoding::scaled_column(ROTTEN_TOMATOES).scale(
                Scale::builder().family(genre).guide(
                    Axis::builder().title(GuideTitle::builder().text("Rotten Tomatoes rating")),
                ),
            ))
            .y(Encoding::scaled_column(IMDB).scale(
                Scale::builder()
                    .guide(Axis::builder().title(GuideTitle::builder().text("IMDB rating"))),
            ))
            .color(
                Encoding::scaled_column(YEAR).scale(
                    Scale::builder().guide(
                        Colorbar::builder().axis(
                            ColorbarAxis::builder()
                                .title(GuideTitle::builder().text("Year"))
                                .labels(GuideLabels::builder().format("d")),
                        ),
                    ),
                ),
            )
            .opacity(Encoding::constant(0.6)),
    )
    .table(movies);
    Plot::cartesian(CartesianPlot::builder().layers([symbols]))
        .id("panel")
        .tools([ToolBuilder::from(Navigate::builder()), tooltip().into()])
}

/// The grid: a facet column of the eras, each a facet row of the genres, with its own scale of
/// Rotten Tomatoes ratings for each genre.
fn grid(movies: &RecordBatch) -> Chart {
    let rows = Node::partitions(movies, ERA, PartitionOrder::Appearance, |era| {
        let panels = Node::partitions(&era, GENRE, PartitionOrder::Ascending, |cell| {
            Ok(panel(cell))
        })?;
        Ok(Concat::builder()
            .direction(Direction::Row)
            .facet(Facet::builder().column(GENRE))
            .resolve([Resolve::channel(Channel::X).sharing(Sharing::Independent)])
            .children(panels))
    })
    .unwrap();
    Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(
            Concat::builder()
                .direction(Direction::Column)
                .facet(Facet::builder().column(ERA))
                .children(rows),
        )
        .infer_scales()
        .build()
        .unwrap()
}

/// `chart` decoded from its bytes, as a reader of a stored chart gets it.
fn round_trip(chart: &Chart) -> Chart {
    Chart::from_bytes(chart.to_bytes().unwrap()).unwrap()
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

/// The evaluator of the grid, after a round trip through its bytes.
fn evaluator(engine: &LabelEngine) -> ChartEvaluator {
    let chart = round_trip(&grid(&movies()));
    ChartEvaluator::new(chart, cascade(), engine.clone(), Registry::new()).unwrap()
}

/// Writes the chart at `SIZE` as SVG to `path`.
fn write_svg(path: &str) {
    let engine = engine();
    let [width, height] = SIZE;
    let scene = (evaluator(&engine).evaluate(&HostInputs::sized(width, height)))
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
    let chart = round_trip(&grid(&movies()));
    let app = ChartApp::new(chart, cascade(), engine()).size(SIZE);
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
