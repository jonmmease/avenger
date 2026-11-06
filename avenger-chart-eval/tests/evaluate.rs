//! Charts of one plot evaluated at sizes, with the base stylesheet, and compared with image
//! baselines.
//!
//! A changed image is written to `tests/output/`; accept it by copying it to `tests/baselines/`.

use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array, RecordBatch, StringArray};
use avenger_chart::{
    build::{self, ChartParts},
    proto::{self, cartesian_layer},
    Chart,
};
use avenger_chart_eval::{ChartEvaluator, EvalError, EvaluatedChart, Registry};
use avenger_format_datetime_d3::D3DateTimeFormatProvider;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_render_fixtures::raster::svg_to_png;
use avenger_style::{Cascade, Env, HostInputs, ParseOptions, Sheet, SourceId, StyleTable};
use avenger_svg::SvgRenderer;
use avenger_typst_label::{EngineOptions, LabelEngine};

fn engine() -> LabelEngine {
    LabelEngine::new(EngineOptions {
        fonts: avenger_render_fixtures::scene::fonts(),
    })
    .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
    .with_datetime_formatting(Arc::new(D3DateTimeFormatProvider::new()))
}

/// The cascade of a table whose stylesheet imports the base stylesheet, then holds `css`.
fn cascade(css: &str) -> Arc<Cascade> {
    let mut no_files = |url: &str, _: Option<&str>| Err(format!("no file `{url}`"));
    let options = ParseOptions {
        allow_import: true,
        ..Default::default()
    };
    let text = format!("@import 'std:base.css';\n{css}");
    let (sheet, diagnostics) = Sheet::load(&text, None, SourceId(1), options, &mut no_files);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    Arc::new(Cascade::new(
        &StyleTable::new().with_author(sheet, Env::new()),
    ))
}

fn evaluator(chart: Chart) -> ChartEvaluator {
    ChartEvaluator::new(chart, cascade(""), engine(), Registry::new()).unwrap()
}

/// A table of 24 cars: a weight, a fuel economy, and an origin.
fn cars() -> RecordBatch {
    let rows = 24;
    let weight: Vec<f64> = (0..rows)
        .map(|row| 1800.0 + ((row * 37) % 24) as f64 * 130.0)
        .collect();
    let mpg: Vec<f64> = (weight.iter().enumerate())
        .map(|(row, weight)| 48.0 - weight / 140.0 + ((row * 7) % 5) as f64)
        .collect();
    let origins = ["USA", "Europe", "Japan"];
    RecordBatch::try_from_iter([
        ("weight", Arc::new(Float64Array::from(weight)) as ArrayRef),
        ("mpg", Arc::new(Float64Array::from(mpg))),
        (
            "origin",
            Arc::new(StringArray::from_iter_values(
                (0..rows).map(|row| origins[row % 3]),
            )),
        ),
    ])
    .unwrap()
}

/// A chart of one plot of `layers` over the cars.
fn chart(layers: impl FnOnce(proto::TableRef) -> Vec<proto::CartesianLayer>) -> Chart {
    let mut parts = ChartParts::new();
    let table = parts.table(cars());
    parts
        .finish(build::message(build::cartesian(layers(table))))
        .unwrap()
}

fn symbols(table: proto::TableRef, mark: proto::SymbolMark) -> proto::CartesianLayer {
    proto::CartesianLayer {
        table: Some(table),
        mark: Some(cartesian_layer::Mark::Symbol(mark)),
        ..Default::default()
    }
}

/// A scatter of the cars' weights and fuel economies.
fn scatter() -> Chart {
    chart(|table| {
        vec![symbols(
            table,
            proto::SymbolMark {
                x: Some(build::scaled_column("weight")),
                y: Some(build::scaled_column("mpg")),
                color: Some(build::scaled_column("origin")),
                ..Default::default()
            },
        )]
    })
}

/// Evaluates `chart` at `size`, renders it, and writes the image to `tests/output/{name}.png`.
fn render(chart: Chart, size: [f32; 2], name: &str) -> EvaluatedChart {
    let engine = engine();
    let mut evaluator = evaluator(chart);
    let evaluated = evaluator
        .evaluate(&HostInputs::sized(size[0], size[1]))
        .unwrap();
    let svg = SvgRenderer::new()
        .render_scene_graph(&evaluated.scene, &engine)
        .unwrap();
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/output");
    std::fs::create_dir_all(&output).unwrap();
    svg_to_png(&svg, 2.0)
        .save(output.join(format!("{name}.png")))
        .unwrap();
    evaluated
}

#[test]
fn a_scatter_of_symbols_draws_its_axes_and_legend() {
    let evaluated = render(scatter(), [320.0, 240.0], "symbols-320x240");
    assert!(evaluated.warnings.is_empty(), "{:#?}", evaluated.warnings);
    assert_eq!(evaluated.scene.width, 320.0);
}

/// The coordinates of a Cartesian plot take its pixels as its positions, and its scales map the
/// data into the plot.
#[test]
fn the_coordinates_of_a_cartesian_plot_map_its_pixels_to_themselves() {
    let mut evaluator = evaluator(scatter());
    let evaluated = evaluator
        .evaluate(&HostInputs::sized(320.0, 240.0))
        .unwrap();
    let plot = &evaluated.plots[0];
    let projection = plot.coords.projection();
    for pixel in [[0.0, 0.0], [12.5, 40.0], [plot.rect[2], plot.rect[3]]] {
        assert_eq!(
            projection.position(pixel),
            Some([f64::from(pixel[0]), f64::from(pixel[1])])
        );
        assert_eq!(
            projection.pixel([f64::from(pixel[0]), f64::from(pixel[1])]),
            Some(pixel)
        );
    }
    let x = plot.coords.scale(0).unwrap();
    let (start, end) = x.numeric_interval_range().unwrap();
    assert_eq!((start, end), (0.0, f64::from(plot.rect[2])));
    assert_eq!(plot.layers.len(), 1);
    assert_eq!(
        plot.layers[0].coords.scale(1).unwrap().domain(),
        plot.coords.scale(1).unwrap().domain()
    );
}

#[test]
fn a_chart_of_a_concat_is_unsupported() {
    let row = proto::Concat {
        direction: proto::Direction::Row as i32,
        ..Default::default()
    };
    let concat = build::concat(row, vec![scatter(), scatter()]).unwrap();
    let error = ChartEvaluator::new(concat, cascade(""), engine(), Registry::new()).err();
    assert!(
        matches!(error, Some(EvalError::Unsupported(ref what)) if what == "a concat"),
        "{error:?}"
    );
}

/// A resize within a breakpoint keeps the styles and fits the scales at the new size.
#[test]
fn a_resize_fits_the_position_scales_at_the_new_size() {
    let mut evaluator = evaluator(scatter());
    let small = evaluator
        .evaluate(&HostInputs::sized(320.0, 240.0))
        .unwrap();
    let large = evaluator
        .evaluate(&HostInputs::sized(640.0, 360.0))
        .unwrap();
    assert!(large.plots[0].rect[2] > small.plots[0].rect[2]);
    assert_eq!(large.scene.width, 640.0);
    let x = large.plots[0].coords.scale(0).unwrap();
    assert_eq!(
        x.numeric_interval_range().unwrap().1,
        f64::from(large.plots[0].rect[2])
    );
}
