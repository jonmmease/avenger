//! Charts of one unit evaluated at two sizes and compared with image baselines.
//!
//! A changed image is written to `tests/output/`; accept it by copying it to `tests/baselines/`.

use std::sync::Arc;

use std::collections::BTreeMap;

use arrow::array::UInt8Array;
use arrow::array::{
    Array, ArrayRef, Date32Array, Float64Array, Int64Array, ListArray, RecordBatch, StringArray,
    StructArray, UInt32Array,
};
use arrow::buffer::OffsetBuffer;
use arrow::datatypes::{DataType, Field};
use avenger_chart::{proto, Chart, Contribution, Encode, Scale, Selection, TupleTest, Value};
use avenger_chart_eval::{ChartEvaluator, EvaluatedChart, PlotCoord, Theme};
use avenger_format_datetime_d3::D3DateTimeFormatProvider;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_render_fixtures::raster::{assert_baseline, svg_to_png};
use avenger_scales::scales::domain_summary::{DomainSummary, IntervalSummary};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_svg::SvgRenderer;
use avenger_typst_label::{EngineOptions, LabelEngine};

fn engine() -> LabelEngine {
    LabelEngine::new(EngineOptions {
        fonts: avenger_render_fixtures::scene::fonts(),
    })
    .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
    .with_datetime_formatting(Arc::new(D3DateTimeFormatProvider::new()))
}

/// A table of 24 cars: a weight, a fuel economy, a count, an origin, a cylinder count, and a
/// model year.
fn cars() -> RecordBatch {
    let rows = 24;
    let weight: Vec<f64> = (0..rows)
        .map(|row| 1800.0 + ((row * 37) % 24) as f64 * 130.0)
        .collect();
    let mpg: Vec<f64> = weight
        .iter()
        .enumerate()
        .map(|(row, weight)| 48.0 - weight / 140.0 + ((row * 7) % 5) as f64)
        .collect();
    let origins = ["USA", "Europe", "Japan"];
    let columns: Vec<(&str, ArrayRef)> = vec![
        ("weight", Arc::new(Float64Array::from(weight))),
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
    ];
    RecordBatch::try_from_iter(columns).unwrap()
}

/// Evaluates `chart` at `size`, renders it, and compares it with the baseline `name`.
fn render(chart: &Chart, size: [f32; 2], name: &str) -> EvaluatedChart {
    let engine = engine();
    let mut evaluator = ChartEvaluator::new(chart, &engine, &Theme::default()).unwrap();
    let evaluated = evaluator.evaluate(size).unwrap();
    let svg = SvgRenderer::new()
        .render_scene_graph(&evaluated.scene, &engine)
        .unwrap();
    assert_baseline(env!("CARGO_MANIFEST_DIR"), &svg_to_png(&svg, 2.0), name);
    evaluated
}

/// Renders `chart` at 320 by 240 and at 640 by 360, writing both images before a failure of
/// either.
fn render_both(chart: &Chart, name: &str) {
    let results: Vec<_> = [[320.0, 240.0], [640.0, 360.0]]
        .into_iter()
        .map(|size| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                render(chart, size, &format!("{name}-{}x{}", size[0], size[1]));
            }))
        })
        .collect();
    for result in results {
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }
}

#[test]
fn symbols() {
    let chart = Chart::unit(cars(), proto::SymbolMark::default())
        .x("weight")
        .y("mpg")
        .color("origin")
        .build()
        .unwrap();
    render_both(&chart, "symbols");
}

#[test]
fn symbols_with_a_size_and_a_shape() {
    let chart = Chart::unit(cars(), proto::SymbolMark::default())
        .x("weight")
        .y(Encode::column("mpg").scale(Scale::new().nice(5.0)))
        .shape("cylinders")
        .size("count")
        .build()
        .unwrap();
    render_both(&chart, "symbols-size-shape");
}

/// Three series of twelve months each.
fn series() -> RecordBatch {
    let months = 12;
    let names = ["A", "B", "C"];
    let mut dates = Vec::new();
    let mut values = Vec::new();
    let mut symbols = Vec::new();
    for (index, name) in names.iter().enumerate() {
        for month in 0..months {
            dates.push(18262 + month * 30);
            let wave = ((month as f64 + index as f64 * 2.0) * 0.6).sin();
            values.push(20.0 + index as f64 * 8.0 + wave * 6.0);
            symbols.push(*name);
        }
    }
    RecordBatch::try_from_iter([
        ("date", Arc::new(Date32Array::from(dates)) as ArrayRef),
        ("value", Arc::new(Float64Array::from(values)) as ArrayRef),
        ("symbol", Arc::new(StringArray::from(symbols)) as ArrayRef),
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
        (
            "amount",
            Arc::new(Float64Array::from(vec![28.0, 55.0, 43.0, 91.0, 81.0, 53.0])) as ArrayRef,
        ),
    ])
    .unwrap()
}

#[test]
fn lines() {
    let chart = Chart::unit(series(), proto::LineMark::default())
        .x("date")
        .y("value")
        .color("symbol")
        .build()
        .unwrap();
    render_both(&chart, "lines");
}

#[test]
fn trails() {
    let chart = Chart::unit(series(), proto::TrailMark::default())
        .x("date")
        .y("value")
        .color("symbol")
        .size("value")
        .build()
        .unwrap();
    render_both(&chart, "trails");
}

#[test]
fn areas() {
    let chart = Chart::unit(series(), proto::AreaMark::default())
        .x("date")
        .y("value")
        .color("symbol")
        .opacity(Encode::constant(0.6))
        .build()
        .unwrap();
    render_both(&chart, "areas");
}

#[test]
fn bars() {
    let chart = Chart::unit(categories(), proto::BarMark::default())
        .x("category")
        .y("amount")
        .build()
        .unwrap();
    render_both(&chart, "bars");
}

#[test]
fn horizontal_bars() {
    let chart = Chart::unit(categories(), proto::BarMark::default())
        .x("amount")
        .y("category")
        .color("category")
        .build()
        .unwrap();
    render_both(&chart, "bars-horizontal");
}

/// A histogram: rects from the start to the end of each bin, with an inset between bins.
fn histogram() -> Chart {
    let starts: Vec<f64> = (0..8).map(|bin| bin as f64 * 5.0).collect();
    let ends: Vec<f64> = starts.iter().map(|start| start + 5.0).collect();
    let table = RecordBatch::try_from_iter([
        ("start", Arc::new(Float64Array::from(starts)) as ArrayRef),
        ("end", Arc::new(Float64Array::from(ends)) as ArrayRef),
        (
            "count",
            Arc::new(Float64Array::from(vec![
                2.0, 5.0, 9.0, 14.0, 11.0, 7.0, 3.0, 1.0,
            ])) as ArrayRef,
        ),
    ])
    .unwrap();
    Chart::unit(
        table,
        proto::RectMark {
            inset: Some(1.0),
            ..Default::default()
        },
    )
    .x("start")
    .x2("end")
    .y(Encode::column("count").scale(Scale::new().zero(true)))
    .y2(Encode::scaled_constant(0.0))
    .build()
    .unwrap()
}

#[test]
fn rects() {
    render_both(&histogram(), "rects-histogram");
}

#[test]
fn a_rect_without_y_spans_the_height_of_its_band() {
    let chart = Chart::unit(categories(), proto::RectMark::default())
        .x("category")
        .color("amount")
        .build()
        .unwrap();
    render_both(&chart, "rects-bands");
}

#[test]
fn rules() {
    let chart = Chart::unit(cars(), proto::RuleMark::default())
        .x("weight")
        .y(Encode::column("mpg"))
        .y2(Encode::scaled_constant(15.0))
        .color("origin")
        .stroke_dash(Encode::constant("4 2"))
        .build()
        .unwrap();
    render_both(&chart, "rules");
}

#[test]
fn texts() {
    let chart = Chart::unit(categories(), proto::TextMark::default())
        .x("category")
        .y("amount")
        .text("amount")
        .color("category")
        .build()
        .unwrap();
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
            )) as ArrayRef,
        ),
        ("image", Arc::new(images) as ArrayRef),
    ])
    .unwrap()
}

#[test]
fn image_marks() {
    let chart = Chart::unit(
        images(),
        proto::ImageMark {
            width: Some(24.0),
            height: Some(24.0),
            smooth: Some(false),
            ..Default::default()
        },
    )
    .x("x")
    .y("y")
    .image("image")
    .build()
    .unwrap();
    render_both(&chart, "images");
}

#[test]
fn paths() {
    let arrow = "M -8 -4 L 4 -4 L 4 -8 L 12 0 L 4 8 L 4 4 L -8 4 Z";
    let chart = Chart::unit(cars(), proto::PathMark::default())
        .x("weight")
        .y("mpg")
        .path(Encode::constant(arrow))
        .angle_values("count")
        .color("origin")
        .build()
        .unwrap();
    render_both(&chart, "paths");
}

/// The symbol mark of the plot of an evaluation, which is the first mark of the scene.
fn plot_symbols(evaluated: &EvaluatedChart) -> &avenger_scenegraph::marks::symbol::SceneSymbolMark {
    let SceneMark::Group(plot) = &evaluated.scene.marks[0] else {
        panic!("the plot's group");
    };
    plot.marks
        .iter()
        .find_map(|mark| match mark {
            SceneMark::Symbol(symbol) => Some(symbol),
            _ => None,
        })
        .expect("a symbol mark")
}

/// The x position, the y position, and the size of each symbol of the plot of an evaluation.
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

#[test]
fn the_symbol_at_the_maximum_of_the_data_stays_inside_its_plot() {
    let chart = Chart::unit(cars(), proto::SymbolMark::default())
        .x("weight")
        .y("mpg")
        .size(Encode::constant(400.0))
        .build()
        .unwrap();
    let engine = engine();
    let mut evaluator = ChartEvaluator::new(&chart, &engine, &Theme::default()).unwrap();
    for size in [[320.0, 240.0], [640.0, 360.0]] {
        let evaluated = evaluator.evaluate(size).unwrap();
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
    let chart = Chart::unit(cars(), proto::SymbolMark::default())
        .x("weight")
        .y("mpg")
        .size(Encode::constant(20_000.0))
        .build()
        .unwrap();
    let engine = engine();
    let mut evaluator = ChartEvaluator::new(&chart, &engine, &Theme::default()).unwrap();
    let evaluated = evaluator.evaluate([200.0, 150.0]).unwrap();
    assert!(
        evaluated
            .warnings
            .iter()
            .any(|warning| warning.message.contains("too small")),
        "{:?}",
        evaluated.warnings
    );
}

#[test]
fn the_rows_of_each_mark_are_its_instances() {
    let x = Float64Array::from(vec![Some(1.0), None, Some(3.0), Some(4.0), None, Some(6.0)]);
    let table = RecordBatch::try_from_iter([
        ("x", Arc::new(x) as ArrayRef),
        (
            "y",
            Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 2.0, 1.0, 2.0])) as ArrayRef,
        ),
        (
            "group",
            Arc::new(StringArray::from(vec!["a", "a", "a", "b", "b", "b"])) as ArrayRef,
        ),
    ])
    .unwrap();
    let engine = engine();
    for mark in [
        proto::mark::Mark::Symbol(Default::default()),
        proto::mark::Mark::Line(Default::default()),
    ] {
        let chart = Chart::unit(table.clone(), mark)
            .id("plot")
            .x("x")
            .y("y")
            .color("group")
            .build()
            .unwrap();
        let mut evaluator = ChartEvaluator::new(&chart, &engine, &Theme::default()).unwrap();
        let evaluated = evaluator.evaluate([300.0, 200.0]).unwrap();
        SceneGraphRTree::from_scene_graph_with_data(&evaluated.scene, &evaluated.data, &engine)
            .unwrap();
        let rows: usize = ["plot/0", "plot/0/0", "plot/0/1"]
            .iter()
            .filter_map(|name| evaluated.data.rows(name))
            .map(|rows| rows.num_rows())
            .sum();
        assert_eq!(rows, 4);
    }
}

#[test]
fn an_explicit_domain_that_excludes_data_clips_its_plot() {
    let chart = Chart::unit(cars(), proto::SymbolMark::default())
        .x(Encode::column("weight")
            .scale(Scale::new().domain(Arc::new(Float64Array::from(vec![2500.0, 4000.0])))))
        .y("mpg")
        .size(Encode::constant(200.0))
        .build()
        .unwrap();
    render_both(&chart, "explicit-domain-clipped");
}

/// The domain of the x scale of the first plot of an evaluation.
fn x_domain(evaluated: &EvaluatedChart) -> (f64, f64) {
    let PlotCoord::Cartesian { x: Some(x), .. } = &evaluated.plots[0].coord else {
        panic!("a Cartesian plot with an x scale");
    };
    x.numeric_interval_domain().unwrap()
}

#[test]
fn a_summary_domain_covers_more_values_than_the_rows() {
    let summary = DomainSummary::Interval(
        IntervalSummary::new(&[0.0, 10_000.0], &[0.0, 0.0], &[0.0, 0.0]).unwrap(),
    );
    let chart = Chart::unit(cars(), proto::SymbolMark::default())
        .x(Encode::column("weight").scale(Scale::new().summary(summary)))
        .y("mpg")
        .build()
        .unwrap();
    let engine = engine();
    let mut evaluator = ChartEvaluator::new(&chart, &engine, &Theme::default()).unwrap();
    let evaluated = evaluator.evaluate([320.0, 240.0]).unwrap();
    assert_eq!(x_domain(&evaluated), (0.0, 10_000.0));
}

#[test]
fn a_summary_without_the_reach_of_the_drawn_symbols_warns() {
    // The summary holds the extremes of the data without the reach of their symbols.
    let summary = DomainSummary::Interval(
        IntervalSummary::new(&[1800.0, 4790.0], &[0.0, 0.0], &[0.0, 0.0]).unwrap(),
    );
    let chart = Chart::unit(cars(), proto::SymbolMark::default())
        .x(Encode::column("weight").scale(Scale::new().summary(summary)))
        .y("mpg")
        .size(Encode::constant(400.0))
        .build()
        .unwrap();
    let engine = engine();
    let mut evaluator = ChartEvaluator::new(&chart, &engine, &Theme::default()).unwrap();
    let evaluated = evaluator.evaluate([320.0, 240.0]).unwrap();
    assert!(
        evaluated
            .warnings
            .iter()
            .any(|warning| warning.message.contains("summary")),
        "{:?}",
        evaluated.warnings
    );
}

fn not(predicate: proto::Predicate) -> proto::Predicate {
    proto::Predicate {
        predicate: Some(proto::predicate::Predicate::Not(Box::new(predicate))),
    }
}

/// A scatter plot whose rows outside the brush are gray.
fn brushed(resolution: proto::SelectionResolution, test: proto::SelectionTest) -> Chart {
    let predicate = proto::Predicate {
        predicate: Some(proto::predicate::Predicate::Selection(test)),
    };
    Chart::unit(cars(), proto::SymbolMark::default())
        .id("plot")
        .x("weight")
        .y("mpg")
        .color(Encode::column("origin").when(not(predicate), Value::Constant("lightgray".into())))
        .selection(Selection::new("brush", resolution))
        .build()
        .unwrap()
}

/// A contribution of `owner` that selects the weights from `start` to `end`.
fn weights(owner: &str, start: f64, end: f64) -> Contribution {
    Contribution::new(owner).tuple([TupleTest::Range {
        column: "weight".into(),
        channel: proto::Channel::X,
        ends: Arc::new(Float64Array::from(vec![start, end])),
        exclude_end: false,
    }])
}

/// Whether each symbol of the plot of an evaluation is gray.
fn grays(evaluated: &EvaluatedChart) -> Vec<bool> {
    let symbol = plot_symbols(evaluated);
    symbol
        .fill
        .as_vec(symbol.len as usize, None)
        .into_iter()
        .map(|color| {
            matches!(color, avenger_color::ColorOrGradient::Color(color)
                if (color[0] - color[1]).abs() < 1e-3 && (color[1] - color[2]).abs() < 1e-3)
        })
        .collect()
}

#[test]
fn a_condition_grays_the_rows_outside_a_brush() {
    let chart = brushed(
        proto::SelectionResolution::Union,
        proto::SelectionTest {
            selection: "brush".into(),
            ..Default::default()
        },
    );
    let engine = engine();
    let mut evaluator = ChartEvaluator::new(&chart, &engine, &Theme::default()).unwrap();
    let evaluated = evaluator.evaluate([320.0, 240.0]).unwrap();
    assert!(grays(&evaluated).iter().all(|gray| !gray));
    render(&chart, [320.0, 240.0], "condition-without-brush");

    evaluator
        .set_selections(BTreeMap::from([(
            "brush".to_string(),
            vec![weights("plot", 2500.0, 3500.0)],
        )]))
        .unwrap();
    let evaluated = evaluator.evaluate([320.0, 240.0]).unwrap();
    let svg = avenger_svg::SvgRenderer::new()
        .render_scene_graph(&evaluated.scene, &engine)
        .unwrap();
    let grays = grays(&evaluated);
    assert!(grays.iter().any(|gray| *gray) && grays.iter().any(|gray| !gray));
    assert_baseline(
        env!("CARGO_MANIFEST_DIR"),
        &svg_to_png(&svg, 2.0),
        "condition-with-brush",
    );
}

/// The gray rows of `chart` with `contributions` of the brush.
fn gray_rows(chart: &Chart, contributions: Vec<Contribution>) -> Vec<bool> {
    let engine = engine();
    let mut evaluator = ChartEvaluator::new(chart, &engine, &Theme::default()).unwrap();
    evaluator
        .set_selections(BTreeMap::from([("brush".to_string(), contributions)]))
        .unwrap();
    grays(&evaluator.evaluate([320.0, 240.0]).unwrap())
}

#[test]
fn selection_tests_follow_their_empty_behavior_owners_and_resolutions() {
    let test = |empty_selects_none: bool, exclude_own: bool| proto::SelectionTest {
        selection: "brush".into(),
        empty_selects_none,
        exclude_own,
    };
    // Without contributions, a test selects every row, or none when it says so.
    let chart = brushed(proto::SelectionResolution::Union, test(true, false));
    assert!(gray_rows(&chart, vec![]).iter().all(|gray| *gray));
    let chart = brushed(proto::SelectionResolution::Union, test(false, false));
    assert!(gray_rows(&chart, vec![]).iter().all(|gray| !gray));

    // A test that skips its own plot's contribution selects every row.
    let chart = brushed(proto::SelectionResolution::Union, test(false, true));
    assert!(gray_rows(&chart, vec![weights("plot", 2500.0, 3000.0)])
        .iter()
        .all(|gray| !gray));

    // Union selects the rows of either owner, and intersect only the rows of both.
    let both = || {
        vec![
            weights("left", 2000.0, 3000.0),
            weights("right", 2800.0, 4000.0),
        ]
    };
    let union = gray_rows(
        &brushed(proto::SelectionResolution::Union, test(false, false)),
        both(),
    );
    let intersect = gray_rows(
        &brushed(proto::SelectionResolution::Intersect, test(false, false)),
        both(),
    );
    let weights = cars()
        .column_by_name("weight")
        .unwrap()
        .as_any()
        .downcast_ref::<Float64Array>()
        .unwrap()
        .values()
        .to_vec();
    for (index, weight) in weights.iter().enumerate() {
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
    let chart = brushed(
        proto::SelectionResolution::Union,
        proto::SelectionTest {
            selection: "brush".into(),
            ..Default::default()
        },
    );
    let engine = engine();
    let mut evaluator = ChartEvaluator::new(&chart, &engine, &Theme::default()).unwrap();
    evaluator
        .set_selections(BTreeMap::from([(
            "brush".to_string(),
            vec![Contribution::new("other").tuple([TupleTest::Set {
                column: "price".into(),
                channel: proto::Channel::Unspecified,
                values: Arc::new(Float64Array::from(vec![1.0])),
            }])],
        )]))
        .unwrap();
    let evaluated = evaluator.evaluate([320.0, 240.0]).unwrap();
    assert!(grays(&evaluated).iter().all(|gray| *gray));
    assert!(
        evaluated
            .warnings
            .iter()
            .any(|warning| warning.message.contains("`price`")),
        "{:?}",
        evaluated.warnings
    );
}

#[test]
fn a_selection_domain_takes_the_envelope_of_a_brush_and_its_fallback_without_one() {
    let chart = Chart::unit(cars(), proto::SymbolMark::default())
        .x(Encode::column("weight").scale(Scale::new().selection("brush", proto::Channel::X)))
        .y("mpg")
        .selection(Selection::new("brush", proto::SelectionResolution::Union))
        .build()
        .unwrap();
    let engine = engine();
    let mut evaluator = ChartEvaluator::new(&chart, &engine, &Theme::default()).unwrap();
    let fallback = x_domain(&evaluator.evaluate([320.0, 240.0]).unwrap());
    assert!(fallback.0 <= 1800.0 && fallback.1 >= 4790.0, "{fallback:?}");
    evaluator
        .set_selections(BTreeMap::from([(
            "brush".to_string(),
            vec![
                weights("overview", 2500.0, 3000.0),
                weights("other", 2800.0, 3600.0),
            ],
        )]))
        .unwrap();
    assert_eq!(
        x_domain(&evaluator.evaluate([320.0, 240.0]).unwrap()),
        (2500.0, 3600.0)
    );
}

#[test]
fn titles_and_attributions() {
    let base = || {
        Chart::unit(cars(), proto::SymbolMark::default())
            .x("weight")
            .y("mpg")
            .color("origin")
    };
    let titled = base()
        .title(proto::Title {
            text: "Fuel economy".into(),
            subtitle: "By weight and origin".into(),
            align: proto::TitleAlign::Start as i32,
            ..Default::default()
        })
        .attribution(proto::Attribution {
            credits: vec!["Source: StatLib".into()],
            ..Default::default()
        })
        .build()
        .unwrap();
    render_both(&titled, "title-subtitle-attribution");
    let caption = base()
        .title(proto::Title {
            text: "Figure 1: heavier cars travel fewer miles on a gallon.".into(),
            side: proto::TitleSide::Bottom as i32,
            ..Default::default()
        })
        .build()
        .unwrap();
    render_both(&caption, "caption");
}

#[test]
fn legends() {
    let threshold =
        Chart::unit(cars(), proto::SymbolMark::default())
            .x("weight")
            .y("mpg")
            .color(Encode::column("mpg").scale(
                Scale::of("threshold").domain(Arc::new(Float64Array::from(vec![20.0, 30.0]))),
            ))
            .build()
            .unwrap();
    render_both(&threshold, "legend-threshold");
    let merged = Chart::unit(cars(), proto::SymbolMark::default())
        .x("weight")
        .y("mpg")
        .color("origin")
        .shape("origin")
        .build()
        .unwrap();
    render_both(&merged, "legend-color-shape");
}

#[test]
fn an_axis_with_explicit_ticks() {
    let chart = Chart::unit(cars(), proto::SymbolMark::default())
        .x(Encode::column("weight").scale(
            Scale::new()
                .guide(proto::Axis::default())
                .guide_values(Arc::new(Float64Array::from(vec![2000.0, 3000.0, 4000.0]))),
        ))
        .y("mpg")
        .build()
        .unwrap();
    render_both(&chart, "axis-explicit-ticks");
}

#[test]
fn a_colorbar_with_a_longest_length_beside_a_tall_plot() {
    let chart = Chart::unit(cars(), proto::SymbolMark::default())
        .x("weight")
        .y("mpg")
        .color(
            Encode::column("mpg").scale(Scale::new().guide(proto::Colorbar {
                max_length: Some(120.0),
                ..Default::default()
            })),
        )
        .build()
        .unwrap();
    render(&chart, [320.0, 480.0], "colorbar-longest-length");
}

/// The fills of the symbols of the legends of an evaluation: every symbol mark outside its plots.
fn legend_symbol_fills(evaluated: &EvaluatedChart) -> Vec<avenger_color::ColorOrGradient> {
    fn fills(marks: &[SceneMark], out: &mut Vec<avenger_color::ColorOrGradient>) {
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
    let mut out = Vec::new();
    fills(&evaluated.scene.marks[1..], &mut out);
    out
}

#[test]
fn legends_draw_the_constant_colors_of_their_marks() {
    // A size legend of red symbols shows red symbols.
    let symbols = Chart::unit(cars(), proto::SymbolMark::default())
        .x("weight")
        .y("mpg")
        .size("count")
        .color(Encode::constant("#e45756"))
        .build()
        .unwrap();
    let red =
        avenger_color::ColorOrGradient::Color([228.0 / 255.0, 87.0 / 255.0, 86.0 / 255.0, 1.0]);
    let engine = engine();
    let mut evaluator = ChartEvaluator::new(&symbols, &engine, &Theme::default()).unwrap();
    let evaluated = evaluator.evaluate([320.0, 240.0]).unwrap();
    let fills = legend_symbol_fills(&evaluated);
    assert!(!fills.is_empty());
    assert!(
        fills.iter().all(|fill| match (fill, &red) {
            (
                avenger_color::ColorOrGradient::Color(a),
                avenger_color::ColorOrGradient::Color(b),
            ) => a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-3),
            _ => false,
        }),
        "{fills:?}"
    );
    render_both(&symbols, "legend-constant-color-symbols");

    // A dash legend of green lines shows green dashes.
    let lines = Chart::unit(cars(), proto::LineMark::default())
        .x("year")
        .y("mpg")
        .detail("origin")
        .stroke_dash("origin")
        .color(Encode::constant("#54a24b"))
        .build()
        .unwrap();
    render_both(&lines, "legend-constant-color-lines");
}
