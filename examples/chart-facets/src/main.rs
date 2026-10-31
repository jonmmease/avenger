//! A grid of scatter plots of movies' ratings, built with avenger-chart and drawn at the window's
//! size.
//!
//! The example reads the movies of `movie-search` and splits them by genre and by era with Arrow
//! compute. Each genre and era gives one chart, which declares its genre and its era as its
//! partition values. A facet row of the genres for each era, in a facet column of the eras, makes
//! the grid, whose headers name each genre above its column and each era beside its row. Every
//! panel shares the scale of IMDB ratings and the color scale of the years, so the grid draws one
//! axis for each row and one colorbar. The panels of a genre share a scale of Rotten Tomatoes
//! ratings through a family named for the genre, so each column has its own axis.
//!
//! Each panel declares a navigation and a tooltip. A drag or the wheel pans and zooms the panel's
//! scales, so the panels of its genre and every panel of the grid move with it, and a double click
//! returns them to their fitted domains. A tooltip names the movie under the pointer.
//!
//! The chart goes through its bytes, as a chart stored in an Arrow column would, before a chart
//! app shows it at each size of the window. `--svg <PATH>` writes the chart at 960 by 640 pixels
//! as SVG instead of opening a window.

use std::io::Cursor;
use std::sync::Arc;

use arrow::array::{Int64Array, RecordBatch, Scalar, StringArray};
use arrow::compute::kernels::cmp::{eq, gt_eq, lt};
use arrow::compute::{and, concat_batches, filter_record_batch};
use arrow::csv::ReaderBuilder;
use arrow::datatypes::{DataType, Field, Schema};
use avenger_chart::{proto, Chart, Encode, Scale};
use avenger_chart_app::ChartApp;
use avenger_chart_eval::{ChartEvaluator, Theme};
use avenger_format_number_d3::D3NumberFormatProvider;
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

/// The eras of the rows of the grid: each name with the first year in it and the first year after
/// it.
const ERAS: [(&str, i64, i64); 3] = [
    ("Before 1990", i64::MIN, 1990),
    ("1990s", 1990, 2000),
    ("2000s", 2000, i64::MAX),
];

/// The size of the window, and of the SVG that `--svg` writes.
const SIZE: [f32; 2] = [960.0, 640.0];

/// The movies, from the data of `movie-search`, which the binary embeds.
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
    concat_batches(&schema, &batches).unwrap()
}

/// The movies of `genre` from the first year of an era up to the first year after it.
fn movies_of(movies: &RecordBatch, genre: &str, (_, from, to): (&str, i64, i64)) -> RecordBatch {
    let column = |name| movies.column_by_name(name).unwrap();
    let genre = eq(column(GENRE), &Scalar::new(StringArray::from(vec![genre]))).unwrap();
    let after = gt_eq(column(YEAR), &Int64Array::new_scalar(from)).unwrap();
    let before = lt(column(YEAR), &Int64Array::new_scalar(to)).unwrap();
    let keep = and(&genre, &and(&after, &before).unwrap()).unwrap();
    filter_record_batch(movies, &keep).unwrap()
}

/// The panel of the movies of `genre` in `era`, which declares both as its partition values,
/// with a navigation and a tooltip.
fn panel(movies: &RecordBatch, genre: &str, era: (&str, i64, i64)) -> Chart {
    Chart::unit(movies_of(movies, genre, era), proto::SymbolMark::default())
        .id("panel")
        .tool(proto::Navigate::default())
        .tool(tooltip())
        .x(Encode::column(ROTTEN_TOMATOES)
            .title("Rotten Tomatoes rating")
            .scale(Scale::new().family(genre)))
        .y(Encode::column(IMDB).title("IMDB rating"))
        .color(
            Encode::column(YEAR)
                .title("Year")
                .scale(Scale::new().guide(years())),
        )
        .opacity(Encode::constant(0.6))
        .partition(GENRE, genre)
        .partition(ERA, era.0)
        .build()
        .unwrap()
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
            max_radius: 12.0,
            ..Default::default()
        })),
        header: Some(proto::TooltipField {
            column: "title".into(),
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

/// The colorbar of the years, labeled without a separator in the thousands.
fn years() -> proto::Colorbar {
    proto::Colorbar {
        axis: Some(proto::Axis {
            format: "d".into(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// The grid: a facet row of the genres for each era, with its own scale of Rotten Tomatoes
/// ratings for each genre, in a facet column of the eras.
fn grid(movies: &RecordBatch) -> Chart {
    let rows = ERAS.map(|era| {
        let panels = GENRES.map(|genre| panel(movies, genre, era));
        Chart::row(panels)
            .facet(GENRE)
            .independent(proto::Channel::X)
            .build()
            .unwrap()
    });
    Chart::column(rows).facet(ERA).build().unwrap()
}

/// `chart` decoded from its bytes, as a reader of a stored chart gets it.
fn round_trip(chart: &Chart) -> Chart {
    Chart::from_bytes(chart.to_bytes().unwrap()).unwrap()
}

fn engine() -> LabelEngine {
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
}

/// The evaluator of the grid, after a round trip through its bytes.
fn evaluator(engine: &LabelEngine) -> ChartEvaluator {
    let chart = round_trip(&grid(&movies()));
    ChartEvaluator::new(&chart, engine, &Theme::default()).unwrap()
}

/// Writes the chart at `SIZE` as SVG to `path`.
fn write_svg(path: &str) {
    let engine = engine();
    let scene = evaluator(&engine).evaluate(SIZE).unwrap().scene;
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
    let app = ChartApp::new(chart, engine()).size(SIZE);
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
