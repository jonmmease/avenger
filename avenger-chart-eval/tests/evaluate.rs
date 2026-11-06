//! Charts of one plot evaluated at sizes with the base stylesheet, and compared with image
//! baselines.
//!
//! A changed image is written to `tests/output/`; accept it by copying it to `tests/baselines/`.

use std::sync::Arc;

use arrow::{
    array::{
        Array, ArrayRef, Date32Array, Float64Array, Int64Array, ListArray, RecordBatch,
        StringArray, StructArray, UInt32Array, UInt8Array,
    },
    buffer::OffsetBuffer,
    datatypes::{DataType, Field},
};
use avenger_chart::{
    build::{self, ChartParts},
    proto::{
        self, cartesian_layer, condition, domain, domain_fit, encoding, guide, guide_ticks,
        predicate, resolve, scale_spec, Channel,
    },
    Chart,
};
use avenger_chart_eval::{ChartEvaluator, EvalError, EvalWarning, EvaluatedChart, Registry};
use avenger_color::ColorOrGradient;
use avenger_format_datetime_d3::D3DateTimeFormatProvider;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_render_fixtures::raster::{assert_baseline, svg_to_png};
use avenger_scales::scales::{domain_summary::Extents, linear::LinearScale};
use avenger_scenegraph::marks::{
    group::SceneGroup, mark::SceneMark, symbol::SceneSymbolMark, text::text_origin,
};
use avenger_style::{Cascade, Env, HostInputs, ParseOptions, Sheet, SourceId, StyleTable};
use avenger_svg::SvgRenderer;
use avenger_typst_label::{EngineOptions, LabelEngine};

// ---------------------------------------------------------------------------------------------
// Evaluation and rendering

fn engine() -> LabelEngine {
    LabelEngine::new(EngineOptions {
        fonts: avenger_render_fixtures::scene::fonts(),
    })
    .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
    .with_datetime_formatting(Arc::new(D3DateTimeFormatProvider::new()))
}

/// The cascade of a table whose stylesheet imports the base stylesheet, then holds `css`.
fn cascade_with(css: &str) -> Arc<Cascade> {
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

fn cascade() -> Arc<Cascade> {
    cascade_with("")
}

fn evaluator(chart: &Chart) -> ChartEvaluator {
    ChartEvaluator::new(chart.clone(), cascade(), engine(), Registry::new()).unwrap()
}

fn host([width, height]: [f32; 2]) -> HostInputs {
    HostInputs::sized(width, height)
}

/// Evaluates `chart` at `size`.
fn evaluate(chart: &Chart, size: [f32; 2]) -> EvaluatedChart {
    evaluator(chart).evaluate(&host(size)).unwrap()
}

/// Renders an evaluation and compares it with the baseline `name`.
fn assert_image(evaluated: &EvaluatedChart, name: &str) {
    let svg = SvgRenderer::new()
        .render_scene_graph(&evaluated.scene, &engine())
        .unwrap();
    assert_baseline(env!("CARGO_MANIFEST_DIR"), &svg_to_png(&svg, 2.0), name);
}

/// Evaluates `chart` at `size`, renders it, and compares it with the baseline `name`.
fn render(chart: &Chart, size: [f32; 2], name: &str) -> EvaluatedChart {
    let evaluated = evaluate(chart, size);
    assert_image(&evaluated, name);
    evaluated
}

/// Renders `chart` at 320 by 240 and at 640 by 360, writing both images before a failure of
/// either.
fn render_both(chart: &Chart, name: &str) {
    let results: Vec<_> = [[320.0, 240.0], [640.0, 360.0]]
        .iter()
        .map(|size| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                render(chart, *size, &format!("{name}-{}x{}", size[0], size[1]));
            }))
        })
        .collect();
    for result in results {
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Charts

/// The parts of a chart, with helpers for the encodings of constants.
struct Parts(ChartParts);

impl Parts {
    fn new() -> Self {
        Self(ChartParts::new())
    }

    fn table(&mut self, batch: RecordBatch) -> proto::TableRef {
        self.0.table(batch)
    }

    fn list(&mut self, elements: ArrayRef) -> proto::ValueRef {
        self.0.list(elements)
    }

    fn value(&mut self, value: ArrayRef) -> proto::ValueRef {
        self.0.value(value)
    }

    /// A constant that the mark draws as it is.
    fn constant(&mut self, value: ArrayRef) -> proto::Encoding {
        proto::Encoding {
            value: Some(encoding::Value::Constant(self.value(value))),
            ..Default::default()
        }
    }

    fn number(&mut self, value: f64) -> proto::Encoding {
        self.constant(numbers(&[value]))
    }

    fn text(&mut self, value: &str) -> proto::Encoding {
        self.constant(Arc::new(StringArray::from(vec![value])))
    }

    /// A constant through the channel's scale.
    fn scaled_number(&mut self, value: f64) -> proto::Encoding {
        proto::Encoding {
            value: Some(encoding::Value::ScaledConstant(
                self.value(numbers(&[value])),
            )),
            ..Default::default()
        }
    }

    /// A summary domain of the values `values`, without the reach of their marks.
    fn summary(&mut self, values: &[f64]) -> proto::Domain {
        let summary = LinearScale::configured((0.0, 1.0), (0.0, 1.0))
            .summarize(&numbers(values), Extents::None)
            .unwrap();
        proto::Domain {
            domain: Some(domain::Domain::Summary(proto::SummaryDomain {
                value: Some(self.value(Arc::new(summary.to_arrow()))),
                fit: None,
            })),
        }
    }

    /// The chart of one Cartesian plot of `layers`.
    fn plot(self, layers: Vec<proto::CartesianLayer>) -> Chart {
        self.chart(build::cartesian(layers), Vec::new())
    }

    fn chart(self, root: proto::Node, selections: Vec<proto::Selection>) -> Chart {
        let mut message = build::message(root);
        message.selections = selections;
        message.number_format_syntax = "d3".to_string();
        message.datetime_format_syntax = "d3".to_string();
        self.0.finish(message).unwrap()
    }
}

fn numbers(values: &[f64]) -> ArrayRef {
    Arc::new(Float64Array::from(values.to_vec()))
}

/// An encoding of the column `name` through a scale that the chart infers.
fn col(name: &str) -> proto::Encoding {
    build::scaled_column(name)
}

/// An encoding of the column `name` without a scale.
fn raw(name: &str) -> proto::Encoding {
    build::column(name)
}

/// An encoding of the column `name` through `scale`, whose absent fields the chart infers.
fn scaled(name: &str, scale: proto::ScaleSpec) -> proto::Encoding {
    proto::Encoding {
        scale: Some(scale),
        ..col(name)
    }
}

/// A data domain that fits with `fit`.
fn fitted(fit: proto::IntervalFit) -> proto::ScaleSpec {
    proto::ScaleSpec {
        domain: Some(proto::Domain {
            domain: Some(domain::Domain::Data(proto::DataDomain {
                fit: Some(proto::DomainFit {
                    fit: Some(domain_fit::Fit::Interval(fit)),
                }),
            })),
        }),
        ..Default::default()
    }
}

/// An encoding of the column `name` through a scale whose domain contains zero, as the lengths of
/// bars and areas take.
fn zeroed(name: &str) -> proto::Encoding {
    scaled(
        name,
        fitted(proto::IntervalFit {
            zero: Some(true),
            ..Default::default()
        }),
    )
}

fn explicit(domain: proto::ValueRef) -> proto::Domain {
    proto::Domain {
        domain: Some(domain::Domain::Explicit(domain)),
    }
}

fn layer(table: proto::TableRef, mark: cartesian_layer::Mark) -> proto::CartesianLayer {
    proto::CartesianLayer {
        table: Some(table),
        mark: Some(mark),
        ..Default::default()
    }
}

/// A table of 24 cars: a weight, a fuel economy, a count, an origin, a cylinder count, and a
/// model year.
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
            "count",
            Arc::new(Int64Array::from_iter_values(
                (0..rows).map(|row| ((row * 5) % 9) as i64 + 1),
            )),
        ),
        (
            "origin",
            Arc::new(StringArray::from_iter_values(
                (0..rows).map(|row| origins[row % 3]),
            )),
        ),
        (
            "cylinders",
            Arc::new(StringArray::from_iter_values(
                (0..rows).map(|row| ["4", "6", "8"][(row / 3) % 3]),
            )),
        ),
        (
            "year",
            Arc::new(Date32Array::from_iter_values(
                (0..rows as i32).map(|row| 3650 + row * 120),
            )),
        ),
    ])
    .unwrap()
}

fn weights() -> Vec<f64> {
    let cars = cars();
    let weights = cars.column_by_name("weight").unwrap();
    (weights.as_any().downcast_ref::<Float64Array>())
        .unwrap()
        .values()
        .to_vec()
}

/// Three series of twelve months each.
fn series() -> RecordBatch {
    let names = ["A", "B", "C"];
    let mut dates = Vec::new();
    let mut values = Vec::new();
    let mut symbols = Vec::new();
    for (index, name) in names.iter().enumerate() {
        for month in 0..12 {
            dates.push(18262 + month * 30);
            let wave = ((month as f64 + index as f64 * 2.0) * 0.6).sin();
            values.push(20.0 + index as f64 * 8.0 + wave * 6.0);
            symbols.push(*name);
        }
    }
    RecordBatch::try_from_iter([
        ("date", Arc::new(Date32Array::from(dates)) as ArrayRef),
        ("value", Arc::new(Float64Array::from(values))),
        ("symbol", Arc::new(StringArray::from(symbols))),
    ])
    .unwrap()
}

/// Six categories with an amount each.
fn categories() -> RecordBatch {
    RecordBatch::try_from_iter([
        (
            "category",
            Arc::new(StringArray::from(vec!["A", "B", "C", "D", "E", "F"])) as ArrayRef,
        ),
        ("amount", numbers(&[28.0, 55.0, 43.0, 91.0, 81.0, 53.0])),
    ])
    .unwrap()
}

/// The chart of one layer of `mark` over the table `batch`.
fn one(batch: RecordBatch, mark: impl FnOnce(&mut Parts) -> cartesian_layer::Mark) -> Chart {
    let mut parts = Parts::new();
    let table = parts.table(batch);
    let mark = mark(&mut parts);
    parts.plot(vec![layer(table, mark)])
}

/// Symbols of the cars' fuel economies over their weights.
fn cars_symbols(extra: impl FnOnce(&mut Parts, &mut proto::SymbolMark)) -> Chart {
    one(cars(), |parts| {
        let mut mark = proto::SymbolMark {
            x: Some(col("weight")),
            y: Some(col("mpg")),
            ..Default::default()
        };
        extra(parts, &mut mark);
        cartesian_layer::Mark::Symbol(mark)
    })
}

// ---------------------------------------------------------------------------------------------
// Scenes

/// The group of the first plot of an evaluation.
fn plot_group(evaluated: &EvaluatedChart) -> &SceneGroup {
    let name = &evaluated.plots[0].name;
    (evaluated.scene.marks.iter())
        .find_map(|mark| match mark {
            SceneMark::Group(group) if &group.name == name => Some(group),
            _ => None,
        })
        .expect("the plot's group")
}

/// The symbol mark of the first plot of an evaluation.
fn plot_symbols(evaluated: &EvaluatedChart) -> &SceneSymbolMark {
    (plot_group(evaluated).marks.iter())
        .find_map(|mark| match mark {
            SceneMark::Symbol(symbol) => Some(symbol),
            _ => None,
        })
        .expect("a symbol mark")
}

/// The x position, the y position, and the size of each symbol of the first plot.
fn symbol_positions(evaluated: &EvaluatedChart) -> Vec<(f32, f32, f32)> {
    let symbol = plot_symbols(evaluated);
    let len = symbol.len as usize;
    let x = symbol.x.as_vec(len, None);
    let y = symbol.y.as_vec(len, None);
    let size = symbol.size.as_vec(len, None);
    x.into_iter()
        .zip(y)
        .zip(size)
        .map(|((x, y), size)| (x, y, size))
        .collect()
}

/// The text of every text mark of a scene, its groups included.
fn scene_texts(marks: &[SceneMark]) -> Vec<String> {
    let mut texts = Vec::new();
    for mark in marks {
        match mark {
            SceneMark::Group(group) => texts.extend(scene_texts(&group.marks)),
            SceneMark::Text(text) => texts.extend(text.text.as_vec(text.len as usize, None)),
            _ => {}
        }
    }
    texts
}

/// The domain of the scale of `dimension`, 0 for `x` and 1 for `y`, of the first plot.
fn domain(evaluated: &EvaluatedChart, dimension: usize) -> (f64, f64) {
    (evaluated.plots[0].coords.scale(dimension))
        .expect("a scale")
        .numeric_interval_domain()
        .unwrap()
}

fn warns(evaluated: &EvaluatedChart, text: &str) -> bool {
    evaluated.warnings.iter().any(|warning| match warning {
        EvalWarning::NotDrawn { reason, .. } | EvalWarning::Fit { reason, .. } => {
            reason.contains(text)
        }
        EvalWarning::Style(_) => false,
    })
}

// ---------------------------------------------------------------------------------------------
// Marks

#[test]
fn symbols() {
    let chart = cars_symbols(|_, mark| mark.color = Some(col("origin")));
    render_both(&chart, "symbols");
}

#[test]
fn symbols_with_a_size_and_a_shape() {
    let chart = cars_symbols(|_, mark| {
        mark.y = Some(scaled(
            "mpg",
            fitted(proto::IntervalFit {
                nice: Some(5.0),
                ..Default::default()
            }),
        ));
        mark.shape = Some(col("cylinders"));
        mark.size = Some(col("count"));
    });
    render_both(&chart, "symbols-size-shape");
}

#[test]
fn lines() {
    let chart = one(series(), |_| {
        cartesian_layer::Mark::Line(proto::LineMark {
            x: Some(col("date")),
            y: Some(col("value")),
            color: Some(col("symbol")),
            ..Default::default()
        })
    });
    render_both(&chart, "lines");
}

#[test]
fn trails() {
    let chart = one(series(), |_| {
        cartesian_layer::Mark::Trail(proto::TrailMark {
            x: Some(col("date")),
            y: Some(col("value")),
            color: Some(col("symbol")),
            size: Some(col("value")),
            ..Default::default()
        })
    });
    render_both(&chart, "trails");
}

#[test]
fn areas() {
    let chart = one(series(), |parts| {
        cartesian_layer::Mark::Area(proto::AreaMark {
            x: Some(col("date")),
            y: Some(zeroed("value")),
            color: Some(col("symbol")),
            opacity: Some(parts.number(0.6)),
            ..Default::default()
        })
    });
    render_both(&chart, "areas");
}

#[test]
fn bars() {
    let chart = one(categories(), |_| {
        cartesian_layer::Mark::Bar(proto::BarMark {
            x: Some(col("category")),
            y: Some(zeroed("amount")),
            ..Default::default()
        })
    });
    render_both(&chart, "bars");
}

#[test]
fn horizontal_bars() {
    let chart = one(categories(), |_| {
        cartesian_layer::Mark::Bar(proto::BarMark {
            x: Some(zeroed("amount")),
            y: Some(col("category")),
            color: Some(col("category")),
            ..Default::default()
        })
    });
    render_both(&chart, "bars-horizontal");
}

/// A histogram: rects from the start to the end of each bin, with an inset between bins.
#[test]
fn rects() {
    let starts: Vec<f64> = (0..8).map(|bin| bin as f64 * 5.0).collect();
    let ends: Vec<f64> = starts.iter().map(|start| start + 5.0).collect();
    let table = RecordBatch::try_from_iter([
        ("start", numbers(&starts)),
        ("end", numbers(&ends)),
        (
            "count",
            numbers(&[2.0, 5.0, 9.0, 14.0, 11.0, 7.0, 3.0, 1.0]),
        ),
    ])
    .unwrap();
    let chart = one(table, |parts| {
        cartesian_layer::Mark::Rect(proto::RectMark {
            inset: Some(1.0),
            x: Some(col("start")),
            x2: Some(raw("end")),
            y: Some(zeroed("count")),
            y2: Some(parts.scaled_number(0.0)),
            ..Default::default()
        })
    });
    render_both(&chart, "rects-histogram");
}

#[test]
fn a_rect_without_y_spans_the_height_of_its_band() {
    let chart = one(categories(), |_| {
        cartesian_layer::Mark::Rect(proto::RectMark {
            x: Some(col("category")),
            color: Some(col("amount")),
            ..Default::default()
        })
    });
    render_both(&chart, "rects-bands");
}

#[test]
fn rules() {
    let chart = one(cars(), |parts| {
        cartesian_layer::Mark::Rule(proto::RuleMark {
            x: Some(col("weight")),
            y: Some(col("mpg")),
            y2: Some(parts.scaled_number(15.0)),
            color: Some(col("origin")),
            stroke_dash: Some(parts.text("4 2")),
            ..Default::default()
        })
    });
    render_both(&chart, "rules");
}

/// A continuous `y` runs up the plot, so its range starts at the bottom, and a reversed one runs
/// down it.
#[test]
fn a_continuous_y_runs_up_the_plot_and_a_reversed_one_down_it() {
    let range = |reverse: bool| {
        let chart = cars_symbols(|_, mark| {
            mark.y = Some(scaled(
                "mpg",
                proto::ScaleSpec {
                    range: Some(proto::Range {
                        reverse,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ));
        });
        let evaluated = evaluate(&chart, [320.0, 240.0]);
        (evaluated.plots[0].coords.scale(1))
            .unwrap()
            .numeric_interval_range()
            .unwrap()
    };
    let (start, end) = range(false);
    assert!(start > end, "{start} to {end}");
    let (start, end) = range(true);
    assert!(start < end, "{start} to {end}");
}

#[test]
fn texts() {
    let chart = one(categories(), |_| {
        cartesian_layer::Mark::Text(proto::TextMark {
            x: Some(col("category")),
            y: Some(col("amount")),
            text: Some(raw("amount")),
            color: Some(col("category")),
            ..Default::default()
        })
    });
    render_both(&chart, "texts");
}

/// A table of eight images of 4 by 4 pixels, each one color, at points.
fn images() -> RecordBatch {
    let colors = [
        [230u8, 85, 13, 255],
        [49, 130, 189, 255],
        [49, 163, 84, 255],
        [117, 107, 177, 255],
    ];
    let rows = 8;
    let bytes: Vec<u8> = (0..rows)
        .flat_map(|row| (0..16).flat_map(move |_| colors[row % colors.len()]))
        .collect();
    let pixels = ListArray::new(
        Arc::new(Field::new_list_field(DataType::UInt8, false)),
        OffsetBuffer::from_lengths(vec![64; rows]),
        Arc::new(UInt8Array::from(bytes)),
        None,
    );
    let images = StructArray::from(vec![
        (
            Arc::new(Field::new("width", DataType::UInt32, false)),
            Arc::new(UInt32Array::from(vec![4; rows])) as ArrayRef,
        ),
        (
            Arc::new(Field::new("height", DataType::UInt32, false)),
            Arc::new(UInt32Array::from(vec![4; rows])) as ArrayRef,
        ),
        (
            Arc::new(Field::new("data", pixels.data_type().clone(), true)),
            Arc::new(pixels) as ArrayRef,
        ),
    ]);
    RecordBatch::try_from_iter([
        (
            "x",
            Arc::new(Float64Array::from_iter_values(
                (0..rows).map(|row| row as f64),
            )) as ArrayRef,
        ),
        (
            "y",
            Arc::new(Float64Array::from_iter_values(
                (0..rows).map(|row| ((row * 3) % 5) as f64),
            )),
        ),
        ("image", Arc::new(images)),
        (
            "size",
            Arc::new(Float64Array::from_iter_values(
                (0..rows).map(|row| 12.0 + 4.0 * (row % 4) as f64),
            )),
        ),
    ])
    .unwrap()
}

#[test]
fn image_marks() {
    let chart = one(images(), |parts| {
        cartesian_layer::Mark::Image(proto::ImageMark {
            smooth: Some(false),
            x: Some(col("x")),
            y: Some(col("y")),
            image: Some(raw("image")),
            width: Some(parts.number(24.0)),
            height: Some(parts.number(24.0)),
            ..Default::default()
        })
    });
    render_both(&chart, "images");
}

#[test]
fn images_with_sizes_from_columns() {
    let chart = one(images(), |_| {
        cartesian_layer::Mark::Image(proto::ImageMark {
            smooth: Some(false),
            x: Some(col("x")),
            y: Some(col("y")),
            image: Some(raw("image")),
            width: Some(raw("size")),
            height: Some(raw("size")),
            ..Default::default()
        })
    });
    render_both(&chart, "images-sizes");
}

/// Four labels at points, with a weight, a style, and a width for each.
fn labels() -> RecordBatch {
    RecordBatch::try_from_iter([
        ("x", numbers(&[1.0, 2.0, 3.0, 4.0])),
        ("y", numbers(&[1.0, 3.0, 2.0, 4.0])),
        (
            "name",
            Arc::new(StringArray::from(vec![
                "A short label",
                "A label of several words that wraps",
                "Long names wrap at their width",
                "The last label",
            ])) as ArrayRef,
        ),
        ("weight", numbers(&[300.0, 400.0, 700.0, 900.0])),
        (
            "style",
            Arc::new(StringArray::from(vec![
                "normal", "italic", "oblique", "normal",
            ])),
        ),
        ("width", numbers(&[60.0, 90.0, 120.0, 60.0])),
    ])
    .unwrap()
}

/// Texts of the labels' names at their points.
fn label_texts(mark: impl FnOnce(&mut Parts, &mut proto::TextMark)) -> Chart {
    one(labels(), |parts| {
        let mut text = proto::TextMark {
            x: Some(col("x")),
            y: Some(col("y")),
            text: Some(raw("name")),
            ..Default::default()
        };
        mark(parts, &mut text);
        cartesian_layer::Mark::Text(text)
    })
}

#[test]
fn texts_with_weights_and_styles_from_columns() {
    let chart = label_texts(|_, mark| {
        mark.font_weight = Some(raw("weight"));
        mark.font_style = Some(raw("style"));
    });
    render_both(&chart, "texts-fonts");
}

#[test]
fn texts_wrap_at_their_widths() {
    let wrapped = label_texts(|_, mark| mark.width = Some(raw("width")));
    render_both(&wrapped, "texts-wrapped");
    // A fixed width, and one line with an ellipsis where the width cuts it.
    let cut = label_texts(|parts, mark| {
        mark.width_mode = proto::TextWidthMode::Fixed as i32;
        mark.max_lines = Some(1);
        mark.ellipsis = Some(true);
        mark.width = Some(parts.number(70.0));
    });
    render_both(&cut, "texts-cut");
}

#[test]
fn a_wrapped_label_at_the_edge_stays_inside_its_plot() {
    let chart = label_texts(|parts, mark| mark.width = Some(parts.number(90.0)));
    let engine = engine();
    let mut evaluator = evaluator(&chart);
    for size in [[320.0, 240.0], [640.0, 360.0]] {
        let evaluated = evaluator.evaluate(&host(size)).unwrap();
        let [_, _, width, height] = evaluated.plots[0].rect;
        let text = (plot_group(&evaluated).marks.iter())
            .find_map(|mark| match mark {
                SceneMark::Text(text) => Some(text),
                _ => None,
            })
            .expect("a text mark");
        for label in text.labels() {
            let bounds = engine.bounds(&label.label).unwrap();
            let [x, y] = text_origin(&bounds, label.position, label.align, label.baseline);
            assert!(
                x >= -0.5 && x + bounds.width <= width + 0.5,
                "{x} + {} in {width}",
                bounds.width
            );
            assert!(
                y >= -0.5 && y + bounds.height <= height + 0.5,
                "{y} + {} in {height}",
                bounds.height
            );
        }
    }
}

#[test]
fn paths() {
    let chart = one(cars(), |parts| {
        cartesian_layer::Mark::Path(proto::PathMark {
            x: Some(col("weight")),
            y: Some(col("mpg")),
            path: Some(parts.text("M -8 -4 L 4 -4 L 4 -8 L 12 0 L 4 8 L 4 4 L -8 4 Z")),
            angle: Some(raw("count")),
            color: Some(col("origin")),
            ..Default::default()
        })
    });
    render_both(&chart, "paths");
}

// ---------------------------------------------------------------------------------------------
// Fits, rows, and coordinates

#[test]
fn the_symbol_at_the_maximum_of_the_data_stays_inside_its_plot() {
    let chart = cars_symbols(|parts, mark| mark.size = Some(parts.number(400.0)));
    let mut evaluator = evaluator(&chart);
    for size in [[320.0, 240.0], [640.0, 360.0]] {
        let evaluated = evaluator.evaluate(&host(size)).unwrap();
        let [_, _, width, height] = evaluated.plots[0].rect;
        let radius = 400.0f32.sqrt() / 2.0;
        for (x, y, _) in symbol_positions(&evaluated) {
            assert!(
                x - radius >= -0.5 && x + radius <= width + 0.5,
                "{x} in {width}"
            );
            assert!(
                y - radius >= -0.5 && y + radius <= height + 0.5,
                "{y} in {height}"
            );
        }
        assert!(evaluated.warnings.is_empty(), "{:?}", evaluated.warnings);
    }
}

#[test]
fn a_plot_too_small_for_its_symbols_warns() {
    let chart = cars_symbols(|parts, mark| mark.size = Some(parts.number(20_000.0)));
    let evaluated = evaluate(&chart, [200.0, 150.0]);
    assert!(warns(&evaluated, "too small"), "{:?}", evaluated.warnings);
}

#[test]
fn the_rows_of_each_mark_are_its_instances() {
    let x = Float64Array::from(vec![Some(1.0), None, Some(3.0), Some(4.0), None, Some(6.0)]);
    let table = RecordBatch::try_from_iter([
        ("x", Arc::new(x) as ArrayRef),
        ("y", numbers(&[1.0, 2.0, 3.0, 2.0, 1.0, 2.0])),
        (
            "group",
            Arc::new(StringArray::from(vec!["a", "a", "a", "b", "b", "b"])),
        ),
    ])
    .unwrap();
    let engine = engine();
    let marks = [
        cartesian_layer::Mark::Symbol(proto::SymbolMark {
            x: Some(col("x")),
            y: Some(col("y")),
            color: Some(col("group")),
            ..Default::default()
        }),
        cartesian_layer::Mark::Line(proto::LineMark {
            x: Some(col("x")),
            y: Some(col("y")),
            color: Some(col("group")),
            ..Default::default()
        }),
    ];
    for mark in marks {
        let chart = one(table.clone(), |_| mark);
        let evaluated = evaluate(&chart, [300.0, 200.0]);
        SceneGraphRTree::from_scene_graph_with_data(&evaluated.scene, &evaluated.data, &engine)
            .unwrap();
        let rows: usize = (evaluated.plots[0].layers[0].marks.iter())
            .filter_map(|name| evaluated.data.rows(name))
            .map(|rows| rows.num_rows())
            .sum();
        assert_eq!(rows, 4);
    }
}

#[test]
fn an_explicit_domain_that_excludes_data_clips_its_plot() {
    let mut parts = Parts::new();
    let table = parts.table(cars());
    let domain = parts.list(numbers(&[2500.0, 4000.0]));
    let size = parts.number(200.0);
    let chart = parts.plot(vec![layer(
        table,
        cartesian_layer::Mark::Symbol(proto::SymbolMark {
            x: Some(scaled(
                "weight",
                proto::ScaleSpec {
                    domain: Some(explicit(domain)),
                    ..Default::default()
                },
            )),
            y: Some(col("mpg")),
            size: Some(size),
            ..Default::default()
        }),
    )]);
    render_both(&chart, "explicit-domain-clipped");
}

/// The coordinates of a Cartesian plot take its pixels as its positions, which the ranges of its
/// scales span.
#[test]
fn a_cartesian_plot_maps_its_pixels_to_themselves() {
    let chart = cars_symbols(|_, _| {});
    let evaluated = evaluate(&chart, [320.0, 240.0]);
    let plot = &evaluated.plots[0];
    let [_, _, width, height] = plot.rect;
    let projection = plot.coords.projection();
    assert_eq!(
        projection.extent(),
        [[0.0, f64::from(width)], [0.0, f64::from(height)]]
    );
    for pixel in [[0.0, 0.0], [10.0, 20.0], [width, height]] {
        let position = [f64::from(pixel[0]), f64::from(pixel[1])];
        assert_eq!(projection.position(pixel), Some(position));
        assert_eq!(projection.pixel(position), Some(pixel));
    }
    let (start, end) = plot
        .coords
        .scale(0)
        .unwrap()
        .numeric_interval_range()
        .unwrap();
    assert_eq!([start.min(end), start.max(end)], [0.0, f64::from(width)]);
    assert_eq!(
        plot.layers[0].coords.scale(1).unwrap().domain(),
        plot.coords.scale(1).unwrap().domain()
    );
}

/// A resize within a breakpoint fits the position scales at the new size.
#[test]
fn a_resize_fits_the_position_scales_at_the_new_size() {
    let mut evaluator = evaluator(&cars_symbols(|_, _| {}));
    let small = evaluator.evaluate(&host([320.0, 240.0])).unwrap();
    let large = evaluator.evaluate(&host([640.0, 360.0])).unwrap();
    assert!(large.plots[0].rect[2] > small.plots[0].rect[2]);
    assert_eq!(large.scene.width, 640.0);
    let x = large.plots[0].coords.scale(0).unwrap();
    assert_eq!(
        x.numeric_interval_range().unwrap().1,
        f64::from(large.plots[0].rect[2])
    );
}

#[test]
fn a_chart_of_a_concat_is_unsupported() {
    let row = proto::Concat {
        direction: proto::Direction::Row as i32,
        ..Default::default()
    };
    let scatter = || cars_symbols(|_, _| {});
    let concat = build::concat(row, vec![scatter(), scatter()]).unwrap();
    let error = ChartEvaluator::new(concat, cascade(), engine(), Registry::new()).err();
    assert!(
        matches!(error, Some(EvalError::Unsupported(ref what)) if what == "a concat"),
        "{error:?}"
    );
}

#[test]
fn a_summary_domain_covers_more_values_than_the_rows() {
    let mut parts = Parts::new();
    let table = parts.table(cars());
    let summary = parts.summary(&[0.0, 10_000.0]);
    let chart = parts.plot(vec![layer(
        table,
        cartesian_layer::Mark::Symbol(proto::SymbolMark {
            x: Some(scaled(
                "weight",
                proto::ScaleSpec {
                    domain: Some(summary),
                    ..Default::default()
                },
            )),
            y: Some(col("mpg")),
            ..Default::default()
        }),
    )]);
    let evaluated = evaluate(&chart, [320.0, 240.0]);
    assert_eq!(domain(&evaluated, 0), (0.0, 10_000.0));
}

#[test]
fn a_summary_without_the_reach_of_the_drawn_symbols_warns() {
    // The summary holds the extremes of the data without the reach of their symbols.
    let mut parts = Parts::new();
    let table = parts.table(cars());
    let summary = parts.summary(&[1800.0, 4790.0]);
    let size = parts.number(400.0);
    let chart = parts.plot(vec![layer(
        table,
        cartesian_layer::Mark::Symbol(proto::SymbolMark {
            x: Some(scaled(
                "weight",
                proto::ScaleSpec {
                    domain: Some(summary),
                    ..Default::default()
                },
            )),
            y: Some(col("mpg")),
            size: Some(size),
            ..Default::default()
        }),
    )]);
    let evaluated = evaluate(&chart, [320.0, 240.0]);
    assert!(warns(&evaluated, "summary"), "{:?}", evaluated.warnings);
}

// ---------------------------------------------------------------------------------------------
// Selections

fn selected(selected: proto::Selected) -> proto::Predicate {
    proto::Predicate {
        predicate: Some(predicate::Predicate::Selected(selected)),
    }
}

fn not(predicate: proto::Predicate) -> proto::Predicate {
    proto::Predicate {
        predicate: Some(predicate::Predicate::Negate(Box::new(predicate))),
    }
}

fn brush_of(name: &str) -> proto::Selected {
    proto::Selected {
        selection: name.to_string(),
        ..Default::default()
    }
}

/// A clause of `owner` with `condition`.
fn clause(owner: &str, condition: condition::Kind) -> proto::Clause {
    proto::Clause {
        owner: owner.to_string(),
        conditions: vec![proto::Condition {
            kind: Some(condition),
        }],
        ..Default::default()
    }
}

/// The clauses of the brush of a chart, with the values that they refer to.
type Clauses = Box<dyn FnOnce(&mut Parts) -> Vec<proto::Clause>>;

/// A clause of `owner` that selects the weights from `start` to `end`.
fn weights_between(owner: &'static str, start: f64, end: f64) -> Clauses {
    Box::new(move |parts| vec![weight_range(parts, owner, start, end)])
}

fn weight_range(parts: &mut Parts, owner: &str, start: f64, end: f64) -> proto::Clause {
    let low = parts.value(numbers(&[start]));
    let high = parts.value(numbers(&[end]));
    clause(
        owner,
        condition::Kind::Range(proto::ValueRange {
            column: "weight".to_string(),
            channel: Channel::X as i32,
            low: Some(low),
            high: Some(high),
            exclude_end: false,
        }),
    )
}

/// A `Select` tool of the selection `selection` with a box.
fn select(selection: &str) -> proto::Select {
    proto::Select {
        selection: selection.to_string(),
        form: Some(proto::select::Form::Interval(
            proto::IntervalSelect::default(),
        )),
        ..Default::default()
    }
}

/// A scatter of the cars whose rows outside the brush are gray, with the brush's `resolution` and
/// initial clauses. The plot `plot` and its layer `points` each have a box of the brush, so each
/// owns clauses.
fn brushed(
    resolution: proto::SelectionResolution,
    predicate: proto::Selected,
    initial: Clauses,
) -> Chart {
    let mut parts = Parts::new();
    let table = parts.table(cars());
    let gray = parts.value(Arc::new(StringArray::from(vec!["lightgray"])));
    let initial = initial(&mut parts);
    let color = proto::Encoding {
        conditional: vec![proto::ConditionalValue {
            predicate: Some(not(selected(predicate))),
            value: Some(proto::conditional_value::Value::Constant(gray)),
        }],
        ..col("origin")
    };
    let symbols = proto::CartesianLayer {
        id: "points".to_string(),
        tools: vec![proto::LayerTool {
            kind: Some(proto::layer_tool::Kind::Select(select("brush"))),
        }],
        ..layer(
            table,
            cartesian_layer::Mark::Symbol(proto::SymbolMark {
                x: Some(col("weight")),
                y: Some(col("mpg")),
                color: Some(color),
                ..Default::default()
            }),
        )
    };
    let mut root = build::cartesian(vec![symbols]);
    if let Some(proto::node::Node::Plot(plot)) = root.node.as_mut() {
        plot.id = "plot".to_string();
        plot.tools = vec![proto::Tool {
            kind: Some(proto::tool::Kind::Select(select("brush"))),
        }];
    }
    let brush = proto::Selection {
        name: "brush".to_string(),
        resolution: resolution as i32,
        initial,
    };
    parts.chart(root, vec![brush])
}

/// Whether each symbol of the first plot is gray.
fn grays(evaluated: &EvaluatedChart) -> Vec<bool> {
    let symbol = plot_symbols(evaluated);
    (symbol.fill.as_vec(symbol.len as usize, None).into_iter())
        .map(|color| {
            matches!(color, ColorOrGradient::Color(color)
                if (color[0] - color[1]).abs() < 1e-3 && (color[1] - color[2]).abs() < 1e-3)
        })
        .collect()
}

fn gray_rows(chart: &Chart) -> Vec<bool> {
    grays(&evaluate(chart, [320.0, 240.0]))
}

fn no_clauses() -> Clauses {
    Box::new(|_| Vec::new())
}

#[test]
fn a_condition_grays_the_rows_outside_a_brush() {
    use proto::SelectionResolution::Union;
    let without = brushed(Union, brush_of("brush"), no_clauses());
    let evaluated = render(&without, [320.0, 240.0], "condition-without-brush");
    assert!(grays(&evaluated).iter().all(|gray| !gray));

    let with = brushed(
        Union,
        brush_of("brush"),
        weights_between("plot", 2500.0, 3500.0),
    );
    let evaluated = render(&with, [320.0, 240.0], "condition-with-brush");
    let grays = grays(&evaluated);
    assert!(grays.iter().any(|gray| *gray) && grays.iter().any(|gray| !gray));
}

#[test]
fn a_cyclic_range_selects_across_its_wrap() {
    use proto::SelectionResolution::Union;
    let cyclic = |start: f64, end: f64, period: f64| -> Clauses {
        Box::new(move |_| {
            vec![clause(
                "plot",
                condition::Kind::CyclicRange(proto::CyclicRange {
                    column: "weight".to_string(),
                    channel: Channel::Unspecified as i32,
                    start,
                    end,
                    period,
                }),
            )]
        })
    };
    let rows = |clauses| gray_rows(&brushed(Union, brush_of("brush"), clauses));
    // With a period of 3000, a range from 4500 to 2300 holds 4500 to 5300 and 1500 to 2300.
    let wrapped = rows(cyclic(4500.0, 2300.0, 3000.0));
    for (index, weight) in weights().iter().enumerate() {
        let inside = (4500.0..=5300.0).contains(weight) || (1500.0..=2300.0).contains(weight);
        assert_eq!(wrapped[index], !inside, "{weight}");
    }
    assert!(wrapped.iter().any(|gray| *gray) && wrapped.iter().any(|gray| !gray));
    // The condition has no origin of the wrap, so the same range a period earlier selects alike.
    assert_eq!(rows(cyclic(1500.0, 2300.0, 3000.0)), wrapped);
    // A range a whole period wide holds every row.
    assert!(rows(cyclic(-180.0, 180.0, 360.0)).iter().all(|gray| !gray));
}

#[test]
fn selected_predicates_follow_their_empty_behavior_owners_and_resolutions() {
    use proto::SelectionResolution::{Intersect, Union};
    let predicate = |empty_selects_none: bool, exclude_own: bool| proto::Selected {
        selection: "brush".into(),
        empty_selects_none,
        exclude_own,
    };
    // Without clauses, a predicate selects every row, or none when it says so.
    let rows = gray_rows(&brushed(Union, predicate(true, false), no_clauses()));
    assert!(rows.iter().all(|gray| *gray));
    let rows = gray_rows(&brushed(Union, predicate(false, false), no_clauses()));
    assert!(rows.iter().all(|gray| !gray));

    // A predicate that skips its own plot's clause selects every row.
    let own = weights_between("plot", 2500.0, 3000.0);
    let rows = gray_rows(&brushed(Union, predicate(false, true), own));
    assert!(rows.iter().all(|gray| !gray));

    // Union selects the rows of either owner, and intersect only the rows of both.
    let both = || -> Clauses {
        Box::new(|parts| {
            vec![
                weight_range(parts, "plot", 2000.0, 3000.0),
                weight_range(parts, "points", 2800.0, 4000.0),
            ]
        })
    };
    let union = gray_rows(&brushed(Union, predicate(false, false), both()));
    let intersect = gray_rows(&brushed(Intersect, predicate(false, false), both()));
    for (index, weight) in weights().iter().enumerate() {
        assert_eq!(
            union[index],
            !(2000.0..=4000.0).contains(weight),
            "{weight}"
        );
        assert_eq!(
            intersect[index],
            !(2800.0..=3000.0).contains(weight),
            "{weight}"
        );
    }
}

#[test]
fn a_test_of_a_missing_column_selects_no_row_and_warns() {
    let price = Box::new(|parts: &mut Parts| {
        let values = parts.list(numbers(&[1.0]));
        vec![clause(
            "points",
            condition::Kind::Set(proto::ValueSet {
                columns: vec![proto::SetColumn {
                    column: "price".to_string(),
                    channel: Channel::Unspecified as i32,
                    values: Some(values),
                }],
            }),
        )]
    });
    let chart = brushed(proto::SelectionResolution::Union, brush_of("brush"), price);
    let evaluated = evaluate(&chart, [320.0, 240.0]);
    assert!(grays(&evaluated).iter().all(|gray| *gray));
    assert!(warns(&evaluated, "`price`"), "{:?}", evaluated.warnings);
}

#[test]
fn a_selection_domain_takes_the_envelope_of_a_brush_and_its_fallback_without_one() {
    let chart = |initial: Clauses| {
        let mut parts = Parts::new();
        let table = parts.table(cars());
        let initial = initial(&mut parts);
        let spec = proto::ScaleSpec {
            domain: Some(proto::Domain {
                domain: Some(domain::Domain::Selection(proto::SelectionDomain {
                    selection: "brush".to_string(),
                    channel: Channel::X as i32,
                    fallback: Some(proto::selection_domain::Fallback::Data(
                        proto::DataDomain::default(),
                    )),
                })),
            }),
            ..Default::default()
        };
        let symbols = proto::CartesianLayer {
            id: "points".to_string(),
            tools: vec![proto::LayerTool {
                kind: Some(proto::layer_tool::Kind::Select(select("brush"))),
            }],
            ..layer(
                table,
                cartesian_layer::Mark::Symbol(proto::SymbolMark {
                    x: Some(scaled("weight", spec)),
                    y: Some(col("mpg")),
                    ..Default::default()
                }),
            )
        };
        let mut root = build::cartesian(vec![symbols]);
        if let Some(proto::node::Node::Plot(plot)) = root.node.as_mut() {
            plot.id = "plot".to_string();
            plot.tools = vec![proto::Tool {
                kind: Some(proto::tool::Kind::Select(select("brush"))),
            }];
        }
        let brush = proto::Selection {
            name: "brush".to_string(),
            resolution: proto::SelectionResolution::Union as i32,
            initial,
        };
        parts.chart(root, vec![brush])
    };
    let fallback = domain(&evaluate(&chart(no_clauses()), [320.0, 240.0]), 0);
    assert!(fallback.0 <= 1800.0 && fallback.1 >= 4790.0, "{fallback:?}");
    let two = chart(Box::new(|parts| {
        vec![
            weight_range(parts, "plot", 2500.0, 3000.0),
            weight_range(parts, "points", 2800.0, 3600.0),
        ]
    }));
    assert_eq!(domain(&evaluate(&two, [320.0, 240.0]), 0), (2500.0, 3600.0));
}

// ---------------------------------------------------------------------------------------------
// Guides

/// A scatter of the cars colored by origin, with the plot's title and attribution.
fn titled(title: proto::Title, attribution: Option<proto::Attribution>) -> Chart {
    let mut parts = Parts::new();
    let table = parts.table(cars());
    let mut root = build::cartesian(vec![layer(
        table,
        cartesian_layer::Mark::Symbol(proto::SymbolMark {
            x: Some(col("weight")),
            y: Some(col("mpg")),
            color: Some(col("origin")),
            ..Default::default()
        }),
    )]);
    if let Some(proto::node::Node::Plot(plot)) = root.node.as_mut() {
        plot.title = Some(title);
        plot.attribution = attribution;
    }
    parts.chart(root, Vec::new())
}

#[test]
fn titles_and_attributions() {
    let chart = titled(
        proto::Title {
            text: "Fuel economy".into(),
            subtitle: "By weight and origin".into(),
            align: proto::TitleAlign::Start as i32,
            ..Default::default()
        },
        Some(proto::Attribution {
            credits: vec!["Source: StatLib".into()],
            ..Default::default()
        }),
    );
    render_both(&chart, "title-subtitle-attribution");
    let caption = titled(
        proto::Title {
            text: "Figure 1: heavier cars travel fewer miles on a gallon.".into(),
            side: proto::TitleSide::Bottom as i32,
            ..Default::default()
        },
        None,
    );
    render_both(&caption, "caption");
}

#[test]
fn legends() {
    let mut parts = Parts::new();
    let table = parts.table(cars());
    let thresholds = parts.list(numbers(&[20.0, 30.0]));
    let threshold = parts.plot(vec![layer(
        table,
        cartesian_layer::Mark::Symbol(proto::SymbolMark {
            x: Some(col("weight")),
            y: Some(col("mpg")),
            color: Some(scaled(
                "mpg",
                proto::ScaleSpec {
                    r#type: Some(scale_spec::Type::Threshold(proto::ThresholdScale::default())),
                    domain: Some(explicit(thresholds)),
                    ..Default::default()
                },
            )),
            ..Default::default()
        }),
    )]);
    render_both(&threshold, "legend-threshold");
    let merged = cars_symbols(|_, mark| {
        mark.color = Some(col("origin"));
        mark.shape = Some(col("origin"));
    });
    render_both(&merged, "legend-color-shape");
}

#[test]
fn an_axis_with_explicit_ticks() {
    let mut parts = Parts::new();
    let table = parts.table(cars());
    let ticks = parts.list(numbers(&[2000.0, 3000.0, 4000.0]));
    let axis = proto::Axis {
        ticks: Some(proto::GuideTicks {
            at: Some(guide_ticks::At::Values(ticks)),
            ..Default::default()
        }),
        ..Default::default()
    };
    let chart = parts.plot(vec![layer(
        table,
        cartesian_layer::Mark::Symbol(proto::SymbolMark {
            x: Some(scaled(
                "weight",
                proto::ScaleSpec {
                    guide: Some(proto::Guide {
                        guide: Some(guide::Guide::Axis(axis)),
                    }),
                    ..Default::default()
                },
            )),
            y: Some(col("mpg")),
            ..Default::default()
        }),
    )]);
    render_both(&chart, "axis-explicit-ticks");
}

#[test]
fn a_colorbar_with_a_longest_length_beside_a_tall_plot() {
    let chart = cars_symbols(|_, mark| {
        mark.color = Some(scaled(
            "mpg",
            proto::ScaleSpec {
                guide: Some(proto::Guide {
                    guide: Some(guide::Guide::Colorbar(proto::Colorbar {
                        max_length: Some(120.0),
                        ..Default::default()
                    })),
                }),
                ..Default::default()
            },
        ));
    });
    render(&chart, [320.0, 480.0], "colorbar-longest-length");
}

/// The fills of the symbols outside the plot's group: the swatches of the legends.
fn legend_symbol_fills(evaluated: &EvaluatedChart) -> Vec<ColorOrGradient> {
    fn fills(marks: &[SceneMark], out: &mut Vec<ColorOrGradient>) {
        for mark in marks {
            match mark {
                SceneMark::Group(group) => fills(&group.marks, out),
                SceneMark::Symbol(symbol) => {
                    out.extend(symbol.fill.as_vec(symbol.len as usize, None))
                }
                _ => {}
            }
        }
    }
    let plot = &evaluated.plots[0].name;
    let mut out = Vec::new();
    for mark in &evaluated.scene.marks {
        match mark {
            SceneMark::Group(group) if &group.name == plot => {}
            mark => fills(std::slice::from_ref(mark), &mut out),
        }
    }
    out
}

#[test]
fn legends_draw_the_constant_colors_of_their_marks() {
    // A size legend of red symbols shows red symbols.
    let symbols = cars_symbols(|parts, mark| {
        mark.size = Some(col("count"));
        mark.color = Some(parts.text("#e45756"));
    });
    let red = [228.0 / 255.0, 87.0 / 255.0, 86.0 / 255.0, 1.0];
    let fills = legend_symbol_fills(&evaluate(&symbols, [320.0, 240.0]));
    assert!(!fills.is_empty());
    assert!(
        fills
            .iter()
            .all(|fill| matches!(fill, ColorOrGradient::Color(color)
            if color.iter().zip(&red).all(|(a, b)| (a - b).abs() < 1e-3))),
        "{fills:?}"
    );
    render_both(&symbols, "legend-constant-color-symbols");

    // A dash legend of green lines shows green dashes.
    let lines = one(cars(), |parts| {
        cartesian_layer::Mark::Line(proto::LineMark {
            x: Some(col("year")),
            y: Some(col("mpg")),
            detail: Some(raw("origin")),
            stroke_dash: Some(col("origin")),
            color: Some(parts.text("#54a24b")),
            ..Default::default()
        })
    });
    render_both(&lines, "legend-constant-color-lines");
}

// ---------------------------------------------------------------------------------------------
// Overlays

/// The months of a year with their precipitation, in millimeters, and their mean temperature,
/// in degrees Celsius.
fn weather() -> RecordBatch {
    RecordBatch::try_from_iter([
        ("month", Arc::new(months()) as ArrayRef),
        (
            "precipitation",
            numbers(&[
                49.0, 36.0, 43.0, 61.0, 74.0, 80.0, 91.0, 84.0, 66.0, 58.0, 51.0, 47.0,
            ]),
        ),
        (
            "temperature",
            numbers(&[
                -2.0, 0.0, 5.0, 11.0, 17.0, 22.0, 25.0, 24.0, 19.0, 13.0, 6.0, 1.0,
            ]),
        ),
    ])
    .unwrap()
}

fn months() -> StringArray {
    StringArray::from(vec![
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ])
}

/// One plot of the layers of `children`, with `resolve` rules.
fn overlay(children: Vec<Chart>, resolve: Vec<proto::Resolve>) -> Chart {
    let plot = proto::Plot {
        resolve,
        ..Default::default()
    };
    build::overlay(plot, children).unwrap()
}

fn independent(channel: Channel) -> proto::Resolve {
    proto::Resolve {
        key: Some(resolve::Key::Channel(channel as i32)),
        sharing: proto::Sharing::Independent as i32,
    }
}

#[test]
fn bars_with_a_line_on_an_independent_y() {
    // Both layers keep the months in their order. The bars' band scale and the line's point
    // scale merge into a band scale, and the line stands at the middle of each band.
    let in_order = |parts: &mut Parts| proto::ScaleSpec {
        domain: Some(explicit(parts.list(Arc::new(months())))),
        ..Default::default()
    };
    let bars = one(weather(), |parts| {
        cartesian_layer::Mark::Bar(proto::BarMark {
            x: Some(scaled("month", in_order(parts))),
            y: Some(zeroed("precipitation")),
            ..Default::default()
        })
    });
    let line = one(weather(), |parts| {
        cartesian_layer::Mark::Line(proto::LineMark {
            x: Some(scaled("month", in_order(parts))),
            y: Some(col("temperature")),
            color: Some(parts.text("#e45756")),
            ..Default::default()
        })
    });
    let chart = overlay(vec![bars, line], vec![independent(Channel::Y)]);
    render_both(&chart, "overlay-dual-axis");
    let evaluated = evaluate(&chart, [320.0, 240.0]);
    let plot = &evaluated.plots[0];
    let y = |layer: usize| {
        (plot.layers[layer].coords.scale(1))
            .unwrap()
            .numeric_interval_domain()
            .unwrap()
    };
    assert_eq!(y(0).0, 0.0);
    assert!(y(1).0 < 0.0);
    assert_eq!(
        plot.layers[0].coords.scale(0).unwrap().domain(),
        plot.layers[1].coords.scale(0).unwrap().domain()
    );
    // The marks of each layer are named by the plot and the layer's position in it.
    assert!(evaluated.data.rows(&format!("{}/0", plot.name)).is_some());
    assert!(evaluated.data.rows(&format!("{}/1/0", plot.name)).is_some());
}

fn series_layer(
    mark: fn(proto::Encoding, proto::Encoding, proto::Encoding) -> cartesian_layer::Mark,
    color: &str,
) -> Chart {
    one(series(), |_| mark(col("date"), col("value"), col(color)))
}

fn series_line(
    x: proto::Encoding,
    y: proto::Encoding,
    color: proto::Encoding,
) -> cartesian_layer::Mark {
    cartesian_layer::Mark::Line(proto::LineMark {
        x: Some(x),
        y: Some(y),
        color: Some(color),
        ..Default::default()
    })
}

fn series_symbols(
    x: proto::Encoding,
    y: proto::Encoding,
    color: proto::Encoding,
) -> cartesian_layer::Mark {
    cartesian_layer::Mark::Symbol(proto::SymbolMark {
        x: Some(x),
        y: Some(y),
        color: Some(color),
        ..Default::default()
    })
}

#[test]
fn a_line_over_its_symbols() {
    let chart = overlay(
        vec![
            series_layer(series_line, "symbol"),
            series_layer(series_symbols, "symbol"),
        ],
        Vec::new(),
    );
    render_both(&chart, "overlay-line-symbols");
}

#[test]
fn an_overlay_with_independent_colors() {
    // The legend of the line's series stands before the colorbar of the symbols' values, in the
    // order of the layers.
    let chart = overlay(
        vec![
            series_layer(series_line, "symbol"),
            series_layer(series_symbols, "value"),
        ],
        vec![independent(Channel::Color)],
    );
    render_both(&chart, "overlay-independent-colors");
}

#[test]
fn bars_share_y_with_a_rule_of_their_mean() {
    let mean = || {
        let table = RecordBatch::try_from_iter([("mean", numbers(&[58.5]))]).unwrap();
        one(table, |parts| {
            cartesian_layer::Mark::Rule(proto::RuleMark {
                y: Some(col("mean")),
                color: Some(parts.text("#e45756")),
                ..Default::default()
            })
        })
    };
    let bars = || {
        one(categories(), |_| {
            cartesian_layer::Mark::Bar(proto::BarMark {
                x: Some(col("category")),
                y: Some(zeroed("amount")),
                ..Default::default()
            })
        })
    };
    render_both(
        &overlay(vec![bars(), mean()], Vec::new()),
        "overlay-bars-mean",
    );
    // With the rule first, the bars' fit, which contains zero, is still the merged fit.
    let evaluated = evaluate(&overlay(vec![mean(), bars()], Vec::new()), [320.0, 240.0]);
    assert_eq!(domain(&evaluated, 1).0, 0.0);
    assert!(scene_texts(&evaluated.scene.marks).contains(&"mean, amount".to_string()));
}

#[test]
fn a_shared_y_of_a_summary_and_a_data_domain_fits_the_merge_of_both() {
    // The summary covers mpg from 0 to 30, and the rows of the second layer reach past 40.
    let summarized = {
        let mut parts = Parts::new();
        let table = parts.table(cars());
        let summary = parts.summary(&[0.0, 30.0]);
        parts.plot(vec![layer(
            table,
            cartesian_layer::Mark::Symbol(proto::SymbolMark {
                x: Some(col("weight")),
                y: Some(scaled(
                    "mpg",
                    proto::ScaleSpec {
                        domain: Some(summary),
                        ..Default::default()
                    },
                )),
                ..Default::default()
            }),
        )])
    };
    let chart = overlay(vec![summarized, cars_symbols(|_, _| {})], Vec::new());
    let evaluated = evaluate(&chart, [320.0, 240.0]);
    let (low, high) = domain(&evaluated, 1);
    let mpg = cars().column_by_name("mpg").unwrap().clone();
    let largest = (mpg.as_any().downcast_ref::<Float64Array>().unwrap().iter())
        .flatten()
        .fold(f64::NEG_INFINITY, f64::max);
    assert!(largest > 30.0);
    assert!(low <= 0.0, "{low}");
    assert!(
        high > largest,
        "{high} covers {largest} with the reach of its symbol"
    );
}

// ---------------------------------------------------------------------------------------------
// Guides that styles hide, band positions, conditional paths, and Typst text

/// `display: none` on the node of a guide hides the whole guide.
#[test]
fn a_hidden_guide_draws_nothing() {
    let chart = cars_symbols(|_, mark| mark.color = Some(col("origin")));
    let texts = scene_texts(&evaluate(&chart, [320.0, 240.0]).scene.marks);
    assert!(texts.contains(&"Europe".to_string()) && texts.contains(&"weight".to_string()));
    let css = "legend, axis[channel=x] { display: none; }";
    let mut hidden =
        ChartEvaluator::new(chart, cascade_with(css), engine(), Registry::new()).unwrap();
    let evaluated = hidden.evaluate(&host([320.0, 240.0])).unwrap();
    let texts = scene_texts(&evaluated.scene.marks);
    assert!(!texts.contains(&"Europe".to_string()), "{texts:?}");
    assert!(!texts.contains(&"weight".to_string()), "{texts:?}");
    assert!(texts.contains(&"mpg".to_string()), "{texts:?}");
}

/// The left edges of the rects of the first plot.
fn rect_lefts(evaluated: &EvaluatedChart) -> Vec<f32> {
    (plot_group(evaluated).marks.iter())
        .find_map(|mark| match mark {
            SceneMark::Rect(rect) => Some(rect.x.as_vec(rect.len as usize, None)),
            _ => None,
        })
        .expect("a rect mark")
}

/// A band scale places a category's position `band` of the way across its band, and a bar still
/// spans its band.
#[test]
fn a_band_position_keeps_bars_across_their_bands() {
    let chart = |band: Option<f64>| {
        one(categories(), |_| {
            cartesian_layer::Mark::Bar(proto::BarMark {
                x: Some(scaled(
                    "category",
                    proto::ScaleSpec {
                        r#type: Some(scale_spec::Type::Band(proto::BandScale {
                            band,
                            ..Default::default()
                        })),
                        ..Default::default()
                    },
                )),
                y: Some(zeroed("amount")),
                ..Default::default()
            })
        })
    };
    let start = rect_lefts(&evaluate(&chart(None), [320.0, 240.0]));
    let middle = rect_lefts(&evaluate(&chart(Some(0.5)), [320.0, 240.0]));
    for (start, middle) in start.iter().zip(&middle) {
        assert!((start - middle).abs() < 1e-3, "{start} {middle}");
    }
}

/// The path of a row is the first conditional value that selects it.
#[test]
fn a_conditional_path_draws_for_the_rows_that_it_selects() {
    let mut parts = Parts::new();
    let table = parts.table(cars());
    let line = parts.text("M 0 0 L 8 0");
    let arrow = parts.value(Arc::new(StringArray::from(vec![
        "M 0 0 L 8 0 L 8 8 L 0 8 Z",
    ])));
    let initial = vec![weight_range(&mut parts, "plot", 2500.0, 3500.0)];
    let path = proto::Encoding {
        conditional: vec![proto::ConditionalValue {
            predicate: Some(selected(brush_of("brush"))),
            value: Some(proto::conditional_value::Value::Constant(arrow)),
        }],
        ..line
    };
    let mut root = build::cartesian(vec![layer(
        table,
        cartesian_layer::Mark::Path(proto::PathMark {
            x: Some(col("weight")),
            y: Some(col("mpg")),
            path: Some(path),
            ..Default::default()
        }),
    )]);
    if let Some(proto::node::Node::Plot(plot)) = root.node.as_mut() {
        plot.id = "plot".to_string();
        plot.tools = vec![proto::Tool {
            kind: Some(proto::tool::Kind::Select(select("brush"))),
        }];
    }
    let brush = proto::Selection {
        name: "brush".to_string(),
        resolution: proto::SelectionResolution::Union as i32,
        initial,
    };
    let evaluated = evaluate(&parts.chart(root, vec![brush]), [320.0, 240.0]);
    let paths = (plot_group(&evaluated).marks.iter())
        .find_map(|mark| match mark {
            SceneMark::Path(path) => Some(path.path.as_vec(path.len as usize, None)),
            _ => None,
        })
        .expect("a path mark");
    for (path, weight) in paths.iter().zip(weights()) {
        let events = path.iter().count();
        let inside = (2500.0..=3500.0).contains(&weight);
        assert_eq!(events > 3, inside, "{weight}: {events} events");
    }
}

/// Under Typst syntax, a formatted value is plain text, so its `$` does not open math.
#[test]
fn a_formatted_value_under_typst_syntax_is_escaped() {
    let chart = one(categories(), |_| {
        cartesian_layer::Mark::Text(proto::TextMark {
            x: Some(col("category")),
            y: Some(col("amount")),
            text: Some(raw("amount")),
            format: "$,.2f".to_string(),
            syntax: proto::TextSyntax::Typst as i32,
            ..Default::default()
        })
    });
    let text = (plot_group(&evaluate(&chart, [320.0, 240.0])).marks.iter())
        .find_map(|mark| match mark {
            SceneMark::Text(text) => Some(text.text.as_vec(text.len as usize, None)),
            _ => None,
        })
        .expect("a text mark");
    assert!(text[0].contains("\\$"), "{text:?}");
}
