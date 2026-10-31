//! Charts of one plot evaluated at two sizes and compared with image baselines.
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
    render_sizes(chart, &[[320.0, 240.0], [640.0, 360.0]], name);
}

/// Renders `chart` at each of `sizes`, writing every image before a failure of any.
fn render_sizes(chart: &Chart, sizes: &[[f32; 2]], name: &str) {
    let results: Vec<_> = sizes
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
fn a_plot_names_the_columns_that_selections_record_and_tests_its_rows() {
    let chart = Chart::unit(cars(), proto::SymbolMark::default())
        .id("plot")
        .partition("dataset", "cars")
        .x(Encode::column("weight").scale(Scale::new().selection("brush", proto::Channel::X)))
        .y(Encode::column("mpg").selection_field("count"))
        .selection(Selection::new("brush", proto::SelectionResolution::Union))
        .build()
        .unwrap();
    let engine = engine();
    let evaluator = ChartEvaluator::new(&chart, &engine, &Theme::default()).unwrap();
    assert_eq!(
        evaluator.domain_selection(0, proto::Channel::X),
        Some("brush")
    );
    assert_eq!(evaluator.domain_selection(0, proto::Channel::Y), None);
    assert_eq!(
        evaluator.selection_column(0, proto::Channel::X),
        Some("weight")
    );
    assert_eq!(
        evaluator.selection_column(0, proto::Channel::Y),
        Some("count")
    );

    let set = |column: &str, values: Vec<&str>| TupleTest::Set {
        column: column.into(),
        channel: proto::Channel::Unspecified,
        values: Arc::new(StringArray::from(values)),
    };
    let satisfies = |test: &TupleTest| evaluator.satisfies(0, test).unwrap();
    assert!(satisfies(&set("origin", vec!["USA", "Europe", "Japan"])));
    assert!(!satisfies(&set("origin", vec!["USA"])));
    assert!(satisfies(&set("dataset", vec!["cars"])));
    assert!(!satisfies(&set("dataset", vec!["trucks"])));
    assert!(!satisfies(&set("missing", vec!["USA"])));
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

/// The domain of a continuous scale of a plot.
fn interval(scale: &Option<avenger_scales::scales::ConfiguredScale>) -> (f64, f64) {
    scale
        .as_ref()
        .expect("a scale")
        .numeric_interval_domain()
        .unwrap()
}

/// The months of a year with their precipitation, in millimeters, and their mean temperature,
/// in degrees Celsius.
fn weather() -> RecordBatch {
    RecordBatch::try_from_iter([
        ("month", Arc::new(months()) as ArrayRef),
        (
            "precipitation",
            Arc::new(Float64Array::from(vec![
                49.0, 36.0, 43.0, 61.0, 74.0, 80.0, 91.0, 84.0, 66.0, 58.0, 51.0, 47.0,
            ])) as ArrayRef,
        ),
        (
            "temperature",
            Arc::new(Float64Array::from(vec![
                -2.0, 0.0, 5.0, 11.0, 17.0, 22.0, 25.0, 24.0, 19.0, 13.0, 6.0, 1.0,
            ])) as ArrayRef,
        ),
    ])
    .unwrap()
}

fn months() -> StringArray {
    StringArray::from(vec![
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ])
}

#[test]
fn bars_with_a_line_on_an_independent_y() {
    // Both units keep the months in their order. The bars' band scale and the line's point scale
    // merge into a band scale, and the line stands at the middle of each band.
    let months = || Scale::new().domain(Arc::new(months()));
    let bars = Chart::unit(weather(), proto::BarMark::default())
        .x(Encode::column("month").scale(months()))
        .y("precipitation")
        .build()
        .unwrap();
    let line = Chart::unit(weather(), proto::LineMark::default())
        .x(Encode::column("month").scale(months()))
        .y("temperature")
        .color(Encode::constant("#e45756"))
        .build()
        .unwrap();
    let chart = Chart::overlay([bars, line])
        .independent(proto::Channel::Y)
        .build()
        .unwrap();
    render_both(&chart, "overlay-dual-axis");
    let engine = engine();
    let mut evaluator = ChartEvaluator::new(&chart, &engine, &Theme::default()).unwrap();
    let evaluated = evaluator.evaluate([320.0, 240.0]).unwrap();
    let PlotCoord::Cartesian {
        y,
        second_y,
        second_x,
        ..
    } = &evaluated.plots[0].coord
    else {
        panic!("a Cartesian plot");
    };
    assert_eq!(interval(y).0, 0.0);
    assert!(interval(second_y).0 < 0.0);
    assert!(second_x.is_none());
    // The marks of each unit are named by the plot and the unit's position in it.
    assert!(evaluated.data.rows("root/0").is_some());
    assert!(evaluated.data.rows("root/1/0").is_some());
}

#[test]
fn a_line_over_its_symbols() {
    let line = Chart::unit(series(), proto::LineMark::default())
        .x("date")
        .y("value")
        .color("symbol")
        .build()
        .unwrap();
    let points = Chart::unit(series(), proto::SymbolMark::default())
        .x("date")
        .y("value")
        .color("symbol")
        .build()
        .unwrap();
    render_both(
        &Chart::overlay([line, points]).build().unwrap(),
        "overlay-line-symbols",
    );
}

#[test]
fn an_overlay_with_independent_colors() {
    // The legend of the line's series stands before the colorbar of the symbols' values, in the
    // order of the children.
    let line = Chart::unit(series(), proto::LineMark::default())
        .x("date")
        .y("value")
        .color("symbol")
        .build()
        .unwrap();
    let points = Chart::unit(series(), proto::SymbolMark::default())
        .x("date")
        .y("value")
        .color("value")
        .build()
        .unwrap();
    render_both(
        &Chart::overlay([line, points])
            .independent(proto::Channel::Color)
            .build()
            .unwrap(),
        "overlay-independent-colors",
    );
}

#[test]
fn bars_share_y_with_a_rule_of_their_mean() {
    let mean = || {
        let table = RecordBatch::try_from_iter([(
            "mean",
            Arc::new(Float64Array::from(vec![58.5])) as ArrayRef,
        )])
        .unwrap();
        Chart::unit(table, proto::RuleMark::default())
            .y("mean")
            .color(Encode::constant("#e45756"))
            .build()
            .unwrap()
    };
    let bars = || {
        Chart::unit(categories(), proto::BarMark::default())
            .x("category")
            .y("amount")
            .build()
            .unwrap()
    };
    render_both(
        &Chart::overlay([bars(), mean()]).build().unwrap(),
        "overlay-bars-mean",
    );
    // With the rule first, the bars' fit, which contains zero, is still the merged fit.
    let chart = Chart::overlay([mean(), bars()]).build().unwrap();
    let engine = engine();
    let mut evaluator = ChartEvaluator::new(&chart, &engine, &Theme::default()).unwrap();
    let evaluated = evaluator.evaluate([320.0, 240.0]).unwrap();
    let PlotCoord::Cartesian { y, second_y, .. } = &evaluated.plots[0].coord else {
        panic!("a Cartesian plot");
    };
    assert_eq!(interval(y).0, 0.0);
    assert!(second_y.is_none());
    assert!(scene_texts(&evaluated.scene.marks).contains(&"mean, amount".to_string()));
}

#[test]
fn a_shared_y_of_a_summary_and_a_data_domain_fits_the_merge_of_both() {
    // The summary covers mpg from 0 to 30, and the rows of the second unit reach past 40.
    let summary = DomainSummary::Interval(
        IntervalSummary::new(&[0.0, 30.0], &[0.0, 0.0], &[0.0, 0.0]).unwrap(),
    );
    let summarized = Chart::unit(cars(), proto::SymbolMark::default())
        .x("weight")
        .y(Encode::column("mpg").scale(Scale::new().summary(summary)))
        .build()
        .unwrap();
    let data = Chart::unit(cars(), proto::SymbolMark::default())
        .x("weight")
        .y("mpg")
        .build()
        .unwrap();
    let chart = Chart::overlay([summarized, data]).build().unwrap();
    let engine = engine();
    let mut evaluator = ChartEvaluator::new(&chart, &engine, &Theme::default()).unwrap();
    let evaluated = evaluator.evaluate([320.0, 240.0]).unwrap();
    let PlotCoord::Cartesian { y, .. } = &evaluated.plots[0].coord else {
        panic!("a Cartesian plot");
    };
    let (low, high) = interval(y);
    let largest = cars()
        .column_by_name("mpg")
        .unwrap()
        .as_any()
        .downcast_ref::<Float64Array>()
        .unwrap()
        .iter()
        .flatten()
        .fold(f64::NEG_INFINITY, f64::max);
    assert!(largest > 30.0);
    assert!(low <= 0.0, "{low}");
    assert!(
        high > largest,
        "{high} covers {largest} with the reach of its symbol"
    );
}

/// Symbols of `y` over `x` of the cars, colored by origin.
fn scatter(x: &str, y: &str) -> Chart {
    Chart::unit(cars(), proto::SymbolMark::default())
        .x(x)
        .y(y)
        .color("origin")
        .build()
        .unwrap()
}

/// How many text marks of an evaluation show `text`.
fn count_texts(evaluated: &EvaluatedChart, text: &str) -> usize {
    scene_texts(&evaluated.scene.marks)
        .iter()
        .filter(|known| *known == text)
        .count()
}

/// Evaluates `chart` at `size` with the default theme.
fn evaluate(chart: &Chart, size: [f32; 2]) -> EvaluatedChart {
    let engine = engine();
    let mut evaluator = ChartEvaluator::new(chart, &engine, &Theme::default()).unwrap();
    evaluator.evaluate(size).unwrap()
}

/// Equal titles of the axes of different scales on one side of one owner draw once: in a column
/// of rows whose plots take a family of `y` for each column, every family titled `mpg`, the root
/// owns both families and draws one title of `y` beside the grid, not one for each family.
#[test]
fn equal_titles_of_different_scales_draw_once() {
    let plot = |family: &str, x: &str| {
        Chart::unit(cars(), proto::SymbolMark::default())
            .x(x)
            .y(Encode::column("mpg").scale(Scale::new().family(family)))
            .build()
            .unwrap()
    };
    // The column owns the scale of each family, and each row owns its own `y`.
    let families = || {
        Chart::row([plot("left", "weight"), plot("right", "count")])
            .independent(proto::Channel::Y)
            .build()
            .unwrap()
    };
    let shared = || {
        Chart::row([scatter("weight", "mpg"), scatter("count", "mpg")])
            .share(proto::Channel::Y)
            .build()
            .unwrap()
    };
    for row in [families(), shared()] {
        let chart = Chart::column([row.clone(), row]).build().unwrap();
        let evaluated = evaluate(&chart, [640.0, 360.0]);
        assert!(evaluated.warnings.is_empty(), "{:?}", evaluated.warnings);
        assert_eq!(count_texts(&evaluated, "mpg"), 1);
    }
}

#[test]
fn a_row_with_a_shared_y() {
    let chart = Chart::row([scatter("weight", "mpg"), scatter("count", "mpg")])
        .share(proto::Channel::Y)
        .build()
        .unwrap();
    render_both(&chart, "row-shared-y");
    let evaluated = evaluate(&chart, [640.0, 360.0]);
    // The row shares color by default, so one legend stands beside the row, and the shared `y`
    // keeps one axis, at the left of the first plot.
    assert_eq!(count_texts(&evaluated, "origin"), 1);
    assert_eq!(count_texts(&evaluated, "mpg"), 1);
    assert_eq!(count_texts(&evaluated, "weight"), 1);
    assert_eq!(count_texts(&evaluated, "count"), 1);
}

#[test]
fn a_column_with_a_shared_x() {
    let chart = Chart::column([scatter("weight", "mpg"), scatter("weight", "count")])
        .share(proto::Channel::X)
        .build()
        .unwrap();
    render_both(&chart, "column-shared-x");
    let evaluated = evaluate(&chart, [640.0, 360.0]);
    assert_eq!(count_texts(&evaluated, "weight"), 1);
    assert_eq!(count_texts(&evaluated, "origin"), 1);
}

#[test]
fn a_row_with_a_shared_x_gives_its_plots_one_width() {
    // The second plot's `y` labels are wider than the first's.
    let chart = Chart::row([scatter("weight", "count"), scatter("weight", "mpg")])
        .share(proto::Channel::X)
        .build()
        .unwrap();
    render_both(&chart, "row-shared-x");
    for size in [[320.0, 240.0], [640.0, 360.0]] {
        let evaluated = evaluate(&chart, size);
        let [first, second] = [0, 1].map(|plot| evaluated.plots[plot].rect[2]);
        assert!((first - second).abs() < 0.01, "{first} and {second}");
        let x = |plot: usize| {
            let PlotCoord::Cartesian { x, .. } = &evaluated.plots[plot].coord else {
                panic!("a Cartesian plot");
            };
            interval(x)
        };
        assert_eq!(x(0), x(1));
        assert_eq!(count_texts(&evaluated, "weight"), 1);
    }
}

#[test]
fn a_row_sizes_its_plots_by_their_tracks() {
    let track = |weight: f32| proto::Track {
        size: Some(proto::track::Size::Share(proto::Share {
            weight,
            max: None,
        })),
    };
    let chart = Chart::row([scatter("weight", "mpg"), scatter("count", "mpg")])
        .layout(proto::ConcatLayout {
            tracks: vec![track(1.0), track(2.0)],
            ..Default::default()
        })
        .build()
        .unwrap();
    render_both(&chart, "row-tracks");
    let evaluated = evaluate(&chart, [640.0, 360.0]);
    let [first, second] = [0, 1].map(|plot| evaluated.plots[plot].rect[2]);
    assert!((second - 2.0 * first).abs() < 0.5, "{first} and {second}");
}

#[test]
fn concats_nested_three_levels() {
    let inner = Chart::column([scatter("weight", "mpg"), scatter("weight", "count")])
        .share(proto::Channel::X)
        .build()
        .unwrap();
    let row = Chart::row([scatter("count", "mpg"), inner])
        .build()
        .unwrap();
    let chart = Chart::column([row, scatter("year", "mpg")])
        .title(proto::Title {
            text: "Cars".into(),
            subtitle: "Three levels of rows and columns".into(),
            align: proto::TitleAlign::Start as i32,
            ..Default::default()
        })
        .build()
        .unwrap();
    render_both(&chart, "concat-nested");
    let evaluated = evaluate(&chart, [640.0, 360.0]);
    assert_eq!(evaluated.plots.len(), 4);
    assert_eq!(count_texts(&evaluated, "origin"), 1);
}

#[test]
fn two_columns_on_one_shared_axis_title_it_with_both_names() {
    let chart = Chart::column([scatter("weight", "mpg"), scatter("count", "mpg")])
        .share(proto::Channel::X)
        .build()
        .unwrap();
    let evaluated = evaluate(&chart, [640.0, 360.0]);
    assert_eq!(count_texts(&evaluated, "weight, count"), 1);
}

#[test]
fn the_shared_axes_of_a_column_merge_their_fields() {
    let with_axis = |axis: proto::Axis, y: &str| {
        Chart::unit(cars(), proto::SymbolMark::default())
            .x(Encode::column("weight").scale(Scale::new().guide(axis)))
            .y(y)
            .build()
            .unwrap()
    };
    let chart = Chart::column([
        with_axis(
            proto::Axis {
                title: Some("Weight (lb)".into()),
                ..Default::default()
            },
            "mpg",
        ),
        with_axis(
            proto::Axis {
                tick_count: Some(3.0),
                ..Default::default()
            },
            "count",
        ),
    ])
    .share(proto::Channel::X)
    .build()
    .unwrap();
    render_both(&chart, "column-merged-axis");
    let evaluated = evaluate(&chart, [640.0, 360.0]);
    assert_eq!(count_texts(&evaluated, "Weight (lb)"), 1);
    assert_eq!(count_texts(&evaluated, "weight"), 0);
}

#[test]
fn the_guide_rules_of_a_concat_keep_the_guides_of_each_plot() {
    let keep = |part: proto::GuidePart| proto::GuideRule {
        part: part as i32,
        rule: proto::GuideRuleKind::Keep as i32,
    };
    let row = |rules: Vec<proto::GuideRule>| {
        Chart::row([scatter("weight", "mpg"), scatter("count", "mpg")])
            .share(proto::Channel::Y)
            .layout(proto::ConcatLayout {
                guide_rules: rules,
                ..Default::default()
            })
            .build()
            .unwrap()
    };
    let merged = evaluate(&row(Vec::new()), [640.0, 360.0]);
    assert_eq!(count_texts(&merged, "mpg"), 1);
    assert_eq!(count_texts(&merged, "origin"), 1);
    let kept = evaluate(
        &row(vec![
            keep(proto::GuidePart::AxisTitle),
            keep(proto::GuidePart::Legend),
        ]),
        [640.0, 360.0],
    );
    assert_eq!(count_texts(&kept, "mpg"), 2);
    assert_eq!(count_texts(&kept, "origin"), 2);
}

const ISLANDS: [(&str, &[&str]); 3] = [
    ("Biscoe", &["Adelie", "Gentoo"]),
    ("Dream", &["Adelie", "Chinstrap"]),
    ("Torgersen", &["Adelie"]),
];
const SPECIES: [&str; 3] = ["Adelie", "Chinstrap", "Gentoo"];
const SEXES: [&str; 2] = ["Female", "Male"];

/// Penguins of the species of each island and of each sex, with the length and the depth of
/// their bills.
fn penguins() -> RecordBatch {
    let mut islands = Vec::new();
    let mut species = Vec::new();
    let mut sexes = Vec::new();
    let mut lengths = Vec::new();
    let mut depths = Vec::new();
    for (island_index, (island, kinds)) in ISLANDS.iter().enumerate() {
        for kind in kinds.iter() {
            let base = match *kind {
                "Adelie" => (38.0, 18.5),
                "Chinstrap" => (48.0, 18.0),
                _ => (47.0, 15.0),
            };
            for (sex_index, sex) in SEXES.iter().enumerate() {
                for row in 0..5 {
                    let wave = ((row * 7 + island_index * 3) % 5) as f64;
                    islands.push(*island);
                    species.push(*kind);
                    sexes.push(*sex);
                    lengths.push(base.0 + sex_index as f64 * 3.0 + wave);
                    depths.push(base.1 + sex_index as f64 * 1.2 + wave * 0.3);
                }
            }
        }
    }
    RecordBatch::try_from_iter([
        ("island", Arc::new(StringArray::from(islands)) as ArrayRef),
        ("species", Arc::new(StringArray::from(species)) as ArrayRef),
        ("sex", Arc::new(StringArray::from(sexes)) as ArrayRef),
        (
            "bill_length",
            Arc::new(Float64Array::from(lengths)) as ArrayRef,
        ),
        (
            "bill_depth",
            Arc::new(Float64Array::from(depths)) as ArrayRef,
        ),
    ])
    .unwrap()
}

/// The rows of `table` whose columns hold the values of `filters`.
fn rows_of(table: &RecordBatch, filters: &[(&str, &str)]) -> RecordBatch {
    let keep: Vec<bool> = (0..table.num_rows())
        .map(|row| {
            filters.iter().all(|(column, value)| {
                table
                    .column_by_name(column)
                    .unwrap()
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap()
                    .value(row)
                    == *value
            })
        })
        .collect();
    arrow::compute::filter_record_batch(table, &arrow::array::BooleanArray::from(keep)).unwrap()
}

/// A panel of the penguins of `filters`, which it declares as its partition, with `x` through
/// `x_scale`.
fn penguin_panel(filters: &[(&str, &str)], x_scale: Scale) -> Chart {
    let mut panel = Chart::unit(rows_of(&penguins(), filters), proto::SymbolMark::default())
        .x(Encode::column("bill_length").scale(x_scale))
        .y("bill_depth")
        .color("species");
    for (column, value) in filters {
        panel = panel.partition(*column, *value);
    }
    panel.build().unwrap()
}

/// A facet grid of the penguins: a column of the sexes, each a row of the species, with `rows`
/// changing each row and `x_scale` giving the scale of `x` of the panel of each species.
fn facet_grid(
    rows: impl Fn(avenger_chart::ConcatBuilder) -> avenger_chart::ConcatBuilder,
    x_scale: impl Fn(&str) -> Scale,
) -> avenger_chart::ConcatBuilder {
    let rows: Vec<Chart> = SEXES
        .iter()
        .map(|sex| {
            let panels = SPECIES.iter().map(|species| {
                penguin_panel(&[("sex", sex), ("species", species)], x_scale(species))
            });
            rows(Chart::row(panels).facet("species")).build().unwrap()
        })
        .collect();
    Chart::column(rows).facet("sex")
}

#[test]
fn a_facet_grid_with_shared_scales() {
    let chart = facet_grid(|row| row, |_| Scale::new()).build().unwrap();
    render_both(&chart, "facet-grid");
    let evaluated = evaluate(&chart, [640.0, 360.0]);
    // The species header of the first row stands for both, and so does each axis. The legend of
    // the species shows its title and each species once more.
    for text in [
        "species",
        "sex",
        "Adelie",
        "Female",
        "bill_length",
        "bill_depth",
    ] {
        let expected = if matches!(text, "species" | "Adelie") {
            2
        } else {
            1
        };
        assert_eq!(count_texts(&evaluated, text), expected, "{text}");
    }
}

#[test]
fn a_facet_grid_with_a_free_y_for_each_row() {
    let chart = facet_grid(|row| row, |_| Scale::new())
        .independent(proto::Channel::Y)
        .build()
        .unwrap();
    render_both(&chart, "facet-grid-free-y");
}

#[test]
fn a_facet_grid_with_a_free_x_for_each_column_and_a_free_y_for_each_row() {
    let chart = facet_grid(
        |row| row.independent(proto::Channel::X),
        |species| Scale::new().family(species),
    )
    .independent(proto::Channel::Y)
    .build()
    .unwrap();
    render_both(&chart, "facet-grid-free-x-y");
    let evaluated = evaluate(&chart, [640.0, 360.0]);
    // The panels of a column share `x`, and the columns do not.
    let x = |plot: usize| {
        let PlotCoord::Cartesian { x, .. } = &evaluated.plots[plot].coord else {
            panic!("a Cartesian plot");
        };
        interval(x)
    };
    assert_eq!(x(0), x(3));
    assert_ne!(x(0), x(1));
}

#[test]
fn an_empty_cell_keeps_its_header() {
    let row = |sex: &str| {
        let children = SPECIES.iter().map(|species| {
            if sex == "Female" && *species == "Chinstrap" {
                avenger_chart::Child::from(
                    Chart::empty()
                        .partition("sex", sex)
                        .partition("species", *species),
                )
            } else {
                avenger_chart::Child::from(penguin_panel(
                    &[("sex", sex), ("species", species)],
                    Scale::new(),
                ))
            }
        });
        Chart::row(children).facet("species").build().unwrap()
    };
    // The empty cell is in the upper row, whose header labels its column for both rows.
    let chart = Chart::column(SEXES.map(row)).facet("sex").build().unwrap();
    render_both(&chart, "facet-grid-empty");
}

#[test]
fn a_nested_facet_of_islands_and_their_species() {
    let islands = ISLANDS.iter().map(|(island, species)| {
        let rows = SEXES.map(|sex| {
            let panels = species.iter().map(|species| {
                penguin_panel(
                    &[("island", island), ("sex", sex), ("species", species)],
                    Scale::new(),
                )
            });
            Chart::row(panels).facet("species").build().unwrap()
        });
        Chart::column(rows).facet("sex").build().unwrap()
    });
    let chart = Chart::row(islands).facet("island").build().unwrap();
    render_both(&chart, "facet-islands");
    let evaluated = evaluate(&chart, [640.0, 360.0]);
    // Each island keeps its species header and its title, and the sex header of the last island
    // stands for all three. The legend's title is the fourth `species`.
    assert_eq!(count_texts(&evaluated, "island"), 1);
    assert_eq!(count_texts(&evaluated, "species"), 4);
    assert_eq!(count_texts(&evaluated, "sex"), 1);
}

#[test]
fn facet_headers_on_the_bottom_and_the_left() {
    let header = |side: proto::Side| proto::FacetHeader {
        side: side as i32,
        ..Default::default()
    };
    let chart = facet_grid(
        |row| row.facet_header(header(proto::Side::Bottom)),
        |_| Scale::new(),
    )
    .facet_header(header(proto::Side::Left))
    .build()
    .unwrap();
    render_both(&chart, "facet-headers-bottom-left");
}

#[test]
fn a_numeric_facet_header_with_a_format() {
    let panels = [4.0, 6.0, 8.0].map(|cylinders: f64| {
        let table = rows_of(&cars(), &[("cylinders", &cylinders.to_string())]);
        Chart::unit(table, proto::SymbolMark::default())
            .partition("liters", cylinders * 0.6)
            .x("weight")
            .y("mpg")
            .build()
            .unwrap()
    });
    let chart = Chart::row(panels)
        .facet("liters")
        .facet_header(proto::FacetHeader {
            title: Some("Displacement".into()),
            format: ".1f".into(),
            label: "#value L".into(),
            ..Default::default()
        })
        .build()
        .unwrap();
    render_both(&chart, "facet-header-numeric");
    let evaluated = evaluate(&chart, [640.0, 360.0]);
    assert_eq!(count_texts(&evaluated, "Displacement"), 1);
}

#[test]
fn a_facet_with_a_hidden_header() {
    let chart = facet_grid(|row| row.hide_facet_header(), |_| Scale::new())
        .build()
        .unwrap();
    render_both(&chart, "facet-hidden-header");
    let evaluated = evaluate(&chart, [640.0, 360.0]);
    assert_eq!(count_texts(&evaluated, "species"), 1);
    assert_eq!(count_texts(&evaluated, "Gentoo"), 1);
}

#[test]
fn a_shared_legend_holds_the_categories_of_every_facet() {
    // Torgersen has only Adelie penguins, and the legend still holds every species.
    let panels = ISLANDS
        .iter()
        .map(|(island, _)| penguin_panel(&[("island", island)], Scale::new()));
    let chart = Chart::row(panels).facet("island").build().unwrap();
    let evaluated = evaluate(&chart, [640.0, 360.0]);
    for species in SPECIES {
        assert_eq!(count_texts(&evaluated, species), 1, "{species}");
    }
}

/// Cars split into seven groups, a wrap of a panel for each.
fn seven_panels() -> Chart {
    let table = cars();
    let groups: ArrayRef = Arc::new(StringArray::from_iter_values(
        (0..table.num_rows()).map(|row| ["A", "B", "C", "D", "E", "F", "G"][row % 7]),
    ));
    let schema = table.schema();
    let mut columns: Vec<(&str, ArrayRef)> = schema
        .fields()
        .iter()
        .map(|field| field.name().as_str())
        .zip(table.columns().iter().cloned())
        .collect();
    columns.push(("group", groups));
    let table = RecordBatch::try_from_iter(columns).unwrap();
    let panels = ["A", "B", "C", "D", "E", "F", "G"].map(|group| {
        Chart::unit(
            rows_of(&table, &[("group", group)]),
            proto::SymbolMark::default(),
        )
        .partition("group", group)
        .x("weight")
        .y("mpg")
        .color("origin")
        .build()
        .unwrap()
    });
    Chart::wrap(panels).facet("group").build().unwrap()
}

#[test]
fn a_facet_wrap_reflows_with_its_width() {
    let chart = seven_panels();
    // Four columns of at least the theme's 160 pixels fit the wide size, and two the narrow one.
    let (wide, narrow) = ([960.0, 420.0], [480.0, 600.0]);
    render_sizes(&chart, &[wide, narrow], "facet-wrap");
    let engine = engine();
    let mut evaluator = ChartEvaluator::new(&chart, &engine, &Theme::default()).unwrap();
    for (size, columns) in [(wide, 4), (narrow, 2), (wide, 4)] {
        let evaluated = evaluator.evaluate(size).unwrap();
        // Every panel of the shared `x` has one width, and one title stands above the wrap.
        let widths: Vec<f32> = evaluated.plots.iter().map(|plot| plot.rect[2]).collect();
        assert!(
            widths.iter().all(|width| (width - widths[0]).abs() < 0.01),
            "{widths:?}"
        );
        let mut lefts: Vec<f32> = evaluated.plots.iter().map(|plot| plot.rect[0]).collect();
        lefts.dedup_by(|a, b| (*a - *b).abs() < 0.01);
        lefts.sort_by(f32::total_cmp);
        lefts.dedup_by(|a, b| (*a - *b).abs() < 0.01);
        assert_eq!(lefts.len(), columns, "{size:?}");
        assert_eq!(count_texts(&evaluated, "group"), 1);
        assert_eq!(count_texts(&evaluated, "weight"), 1);
        // An evaluation at a new size equals an evaluation of a new evaluator, which prepared its
        // scales from the rows, so a resize reads no rows.
        let fresh = evaluate(&chart, size);
        assert_eq!(
            format!("{:?}", evaluated.scene),
            format!("{:?}", fresh.scene)
        );
    }
}

#[test]
fn a_selection_tests_the_partition_of_a_panel_without_its_column() {
    // A point selection of `Tech` on a chart of sectors styles the panel of the Tech sector, whose
    // table has no `sector` column.
    let sectors = ["Energy", "Tech"];
    let picked = proto::Predicate {
        predicate: Some(proto::predicate::Predicate::Selection(
            proto::SelectionTest {
                selection: "pick".into(),
                ..Default::default()
            },
        )),
    };
    let panels = sectors.map(|sector| {
        Chart::unit(cars(), proto::SymbolMark::default())
            .partition("sector", sector)
            .x("weight")
            .y("mpg")
            .color(
                Encode::column("origin")
                    .when(not(picked.clone()), Value::Constant("lightgray".into())),
            )
            .selection(Selection::new("pick", proto::SelectionResolution::Union))
            .build()
            .unwrap()
    });
    let facets = Chart::row(panels).facet("sector").build().unwrap();
    let totals = RecordBatch::try_from_iter([
        (
            "sector",
            Arc::new(StringArray::from(sectors.to_vec())) as ArrayRef,
        ),
        (
            "total",
            Arc::new(Float64Array::from(vec![3.0, 5.0])) as ArrayRef,
        ),
    ])
    .unwrap();
    let bars = Chart::unit(totals, proto::BarMark::default())
        .id("sectors")
        .x("sector")
        .y("total")
        .selection(
            Selection::new("pick", proto::SelectionResolution::Union).initial(
                Contribution::new("sectors").tuple([TupleTest::Set {
                    column: "sector".into(),
                    channel: proto::Channel::X,
                    values: Arc::new(StringArray::from(vec!["Tech"])),
                }]),
            ),
        )
        .build()
        .unwrap();
    let chart = Chart::row([bars, facets]).build().unwrap();
    let evaluated = evaluate(&chart, [640.0, 360.0]);
    let gray = |plot: usize| {
        let SceneMark::Group(group) = &evaluated.scene.marks[plot] else {
            panic!("a plot's group");
        };
        let symbol = group
            .marks
            .iter()
            .find_map(|mark| match mark {
                SceneMark::Symbol(symbol) => Some(symbol),
                _ => None,
            })
            .unwrap();
        symbol
            .fill
            .as_vec(symbol.len as usize, None)
            .into_iter()
            .all(|color| {
                matches!(color, avenger_color::ColorOrGradient::Color(color)
                    if (color[0] - color[1]).abs() < 1e-3 && (color[1] - color[2]).abs() < 1e-3)
            })
    };
    // The plots are the bars, then the Energy panel and the Tech panel.
    assert!(gray(1), "the Energy panel is gray");
    assert!(!gray(2), "the Tech panel keeps its colors");
    assert!(evaluated.warnings.is_empty(), "{:?}", evaluated.warnings);
}

#[test]
fn the_legends_of_rows_stand_beside_them() {
    let panel = |filters: &[(&str, &str)], color: &str| {
        let mut panel = Chart::unit(rows_of(&penguins(), filters), proto::SymbolMark::default())
            .x("bill_length")
            .y("bill_depth")
            .color(color);
        for (column, value) in filters {
            panel = panel.partition(*column, *value);
        }
        panel.build().unwrap()
    };
    let row = |sex: &str, color: &str| {
        Chart::row(ISLANDS.map(|(island, _)| panel(&[("sex", sex), ("island", island)], color)))
            .facet("island")
    };
    // Each row of a facet column with an independent color shows its legend beside the row,
    // inside the column's header.
    let rows = Chart::column([
        row("Female", "species").build().unwrap(),
        row("Male", "island").build().unwrap(),
    ])
    .facet("sex")
    .independent(proto::Channel::Color)
    .build()
    .unwrap();
    render_both(&rows, "legends-of-rows");
    let evaluated = evaluate(&rows, [640.0, 360.0]);
    assert_eq!(count_texts(&evaluated, "species"), 1);
    assert_eq!(count_texts(&evaluated, "island"), 2);
}

#[cfg(not(feature = "geo"))]
#[test]
fn a_map_plot_needs_the_geo_feature() {
    let places = RecordBatch::try_from_iter([
        (
            "lon",
            Arc::new(Float64Array::from(vec![-70.0, 10.0])) as ArrayRef,
        ),
        ("lat", Arc::new(Float64Array::from(vec![40.0, 50.0]))),
    ])
    .unwrap();
    let chart = Chart::unit(places, proto::SymbolMark::default())
        .longitude("lon")
        .latitude("lat")
        .coord(proto::Geo {
            surface: Some(proto::geo::Surface::Spherical(proto::Spherical {
                projection: "equal_earth".into(),
                ..Default::default()
            })),
            fit: Some(proto::geo::Fit::FitData(proto::PlotData {})),
            ..Default::default()
        })
        .build()
        .unwrap();
    let engine = engine();
    let error = ChartEvaluator::new(&chart, &engine, &Theme::default())
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("the `geo` feature"), "{error}");
}
