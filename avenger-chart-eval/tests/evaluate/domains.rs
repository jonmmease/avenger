//! Fits and domains: room for the reach of marks, explicit, summary, selection, and pinned domains,
//! the coordinates of plots, resizes, and natural sizes.

use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array, RecordBatch, StringArray};
use avenger_chart::prelude::*;
use avenger_chart_eval::{ChartEvaluator, EvalError, Registry};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_style::HostInputs;

use crate::support::*;

/// A continuous `y` runs up the plot, so its range starts at the bottom, and a reversed one runs
/// down it.
#[test]
fn a_continuous_y_runs_up_the_plot_and_a_reversed_one_down_it() {
    let range = |reverse: bool| {
        let symbols = CartesianLayer::symbol(
            cars(),
            SymbolMark::builder()
                .x("weight")
                .y(Encoding::scaled_field("mpg").scale(Range::builder().reverse(reverse))),
        );
        let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();
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
fn the_symbol_at_the_maximum_of_the_data_stays_inside_its_plot() {
    let symbols = CartesianLayer::symbol(
        cars(),
        SymbolMark::builder().x("weight").y("mpg").size(400.0),
    );
    let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();

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
    let symbols = CartesianLayer::symbol(
        cars(),
        SymbolMark::builder().x("weight").y("mpg").size(20_000.0),
    );
    let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();
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
    let symbols = CartesianLayer::symbol(
        table.clone(),
        SymbolMark::builder().x("x").y("y").color("group"),
    );
    let lines = CartesianLayer::line(table, LineMark::builder().x("x").y("y").color("group"));

    let engine = engine();
    for layer in [symbols, lines] {
        let chart = Chart::builder(Plot::cartesian([layer])).build().unwrap();
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
    let symbols = CartesianLayer::symbol(
        cars(),
        SymbolMark::builder()
            .x(Encoding::scaled_field("weight").scale(Domain::explicit([2500.0, 4000.0])))
            .y("mpg")
            .size(200.0),
    );
    let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();
    render_both(&chart, "explicit-domain-clipped");
}

/// The coordinates of a Cartesian plot take its pixels as its positions, which the ranges of its
/// scales span.
#[test]
fn a_cartesian_plot_maps_its_pixels_to_themselves() {
    let symbols = CartesianLayer::symbol(cars(), SymbolMark::builder().x("weight").y("mpg"));
    let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();

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
    let symbols = CartesianLayer::symbol(cars(), SymbolMark::builder().x("weight").y("mpg"));
    let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();

    let mut evaluator = evaluator(&chart);
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
    let scatter = || {
        let symbols = CartesianLayer::symbol(cars(), SymbolMark::builder().x("weight").y("mpg"));
        Plot::cartesian([symbols])
    };
    let concat = Chart::builder(Concat::new(Direction::Row).children([scatter(), scatter()]))
        .build()
        .unwrap();

    let error = ChartEvaluator::new(concat, cascade(), engine(), Registry::new()).err();
    assert!(
        matches!(error, Some(EvalError::Unsupported(ref what)) if what == "a concat"),
        "{error:?}"
    );
}

#[test]
fn a_summary_domain_covers_more_values_than_the_rows() {
    let symbols = CartesianLayer::symbol(
        cars(),
        SymbolMark::builder()
            .x(
                Encoding::scaled_field("weight").scale(Domain::summary(SummaryDomain::new(
                    linear_summary(&[0.0, 10_000.0]),
                ))),
            )
            .y("mpg"),
    );
    let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();

    let evaluated = evaluate(&chart, [320.0, 240.0]);
    assert_eq!(domain(&evaluated, 0), (0.0, 10_000.0));
}

#[test]
fn a_summary_without_the_reach_of_the_drawn_symbols_warns() {
    // The summary holds the extremes of the data without the reach of their symbols, and the fit
    // leaves the domain at them, without zero or nice ends.
    let symbols = CartesianLayer::symbol(
        cars(),
        SymbolMark::builder()
            .x(Encoding::scaled_field("weight").scale(Domain::summary(
                SummaryDomain::new(linear_summary(&[1800.0, 4790.0]))
                    .fit(IntervalFit::builder().zero(false).nice(false)),
            )))
            .y("mpg")
            .size(400.0),
    );
    let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();

    let evaluated = evaluate(&chart, [320.0, 240.0]);
    assert!(warns(&evaluated, "summary"), "{:?}", evaluated.warnings);
}

/// A selection's range with one open end pins that end of the fit, so the axis labels the
/// selection's domain, which starts at the pin, and the plot clips the marks before it.
#[test]
fn the_axis_of_a_selection_with_one_open_end_starts_at_the_selection() {
    // The cars, whose `x` has a selection domain over a fit of the data, and a brush whose range
    // starts at a weight of 2500 and has no end.
    let symbols =
        CartesianLayer::symbol(
            cars(),
            SymbolMark::builder()
                .x(Encoding::scaled_field("weight").scale(Domain::selection(
                    SelectionDomain::data("brush", Channel::X, DataDomain::builder()),
                )))
                .y("mpg"),
        );
    let plot = Plot::cartesian([symbols])
        .id("plot")
        .tools([Select::interval("brush", IntervalSelect::builder())]);
    let brush = Selection::new("brush", SelectionResolution::Union)
        .initial([
            Clause::new([ValueRange::new("weight").channel(Channel::X).low(2500.0)]).owner("plot"),
        ]);
    let chart = Chart::builder(plot).selections([brush]).build().unwrap();

    let evaluated = evaluate(&chart, [320.0, 240.0]);
    let (low, high) = domain(&evaluated, 0);
    assert_eq!(low, 2500.0);
    assert!(high >= 4790.0, "{high}");
    assert!(matches!(
        plot_group(&evaluated).clip,
        avenger_scenegraph::marks::group::Clip::Rect { .. }
    ));
    let texts = scene_texts(&evaluated.scene.marks);
    assert!(!texts.contains(&"2,000".to_string()), "{texts:?}");
    assert!(texts.contains(&"3,000".to_string()), "{texts:?}");
}

/// A fit that pins a middle gives a domain symmetric about it.
#[test]
fn a_pinned_middle_centers_the_domain() {
    let symbols = CartesianLayer::symbol(
        cars(),
        SymbolMark::builder()
            .x(Encoding::scaled_field("weight").scale(Domain::data(
                DataDomain::builder().fit(IntervalFit::builder().mid(3000.0)),
            )))
            .y("mpg"),
    );
    let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();

    let (low, high) = domain(&evaluate(&chart, [320.0, 240.0]), 0);
    assert!(((low + high) / 2.0 - 3000.0).abs() < 1e-6, "{low} {high}");
    assert!(high >= 4790.0, "{high}");
}

/// A chart without a size from its host or its own takes its natural size, where a plot without
/// a size counts at its `contain-intrinsic-size`, and a host's size stretches the plot.
#[test]
fn a_chart_without_a_size_takes_its_natural_size() {
    let symbols = CartesianLayer::symbol(
        cars(),
        SymbolMark::builder().x("weight").y("mpg").color("origin"),
    );
    let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();

    let natural = evaluator(&chart).evaluate(&HostInputs::default()).unwrap();
    let [_, _, width, height] = natural.plots[0].rect;
    assert!(
        (width - 200.0).abs() < 0.5 && (height - 200.0).abs() < 0.5,
        "{width} {height}"
    );
    assert!(natural.scene.width > 200.0 && natural.scene.height > 200.0);
    let hosted = evaluate(&chart, [640.0, 360.0]);
    assert!(hosted.plots[0].rect[2] > 200.0);
}

/// A stroke that only a conditional value shows still widens the reach of its marks, so the
/// selected outlines stay inside the plot.
#[test]
fn a_conditionally_shown_stroke_widens_the_reach_of_its_marks() {
    let stroke =
        Encoding::direct_value("transparent").conditional([ConditionalValue::direct_value(
            Predicate::selected("brush"),
            "black",
        )]);
    let symbols = CartesianLayer::symbol(
        cars(),
        SymbolMark::builder()
            .x("weight")
            .y("mpg")
            .size(100.0)
            .stroke_width(40.0)
            .stroke(stroke),
    );
    let chart = Chart::builder(Plot::cartesian([symbols]))
        .selections([Selection::new("brush", SelectionResolution::Global)])
        .build()
        .unwrap();
    let evaluated = evaluate(&chart, [640.0, 360.0]);
    assert!(evaluated.warnings.is_empty(), "{:#?}", evaluated.warnings);
    let symbol = plot_symbols(&evaluated);
    let [_, _, width, height] = evaluated.plots[0].rect;
    // Each symbol is 10 pixels across with a stroke of 40, so it reaches 25 pixels.
    let xs = symbol.x.as_vec(symbol.len as usize, None);
    let ys = symbol.y.as_vec(symbol.len as usize, None);
    for (x, y) in xs.into_iter().zip(ys) {
        assert!(x >= 25.0 - 0.5 && x <= width - 25.0 + 0.5, "{x} {width}");
        assert!(y >= 25.0 - 0.5 && y <= height - 25.0 + 0.5, "{y} {height}");
    }
}
