//! Fits and domains: room for the reach of marks, explicit, summary, selection, and pinned domains,
//! the coordinates of plots, resizes, and natural sizes.

use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array, RecordBatch, StringArray};
use avenger_chart::{
    build::{self},
    proto::{self, cartesian_layer, condition, domain, Channel},
    Chart,
};
use avenger_chart_eval::{ChartEvaluator, EvalError, Registry};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_style::HostInputs;

use crate::support::*;

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

/// A scatter of the cars whose `x` has a selection domain over a fit of the data, with the brush's
/// initial clauses.
fn selection_scatter(initial: Clauses) -> Chart {
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
    let mut root = build::cartesian(vec![layer(
        table,
        cartesian_layer::Mark::Symbol(proto::SymbolMark {
            x: Some(scaled("weight", spec)),
            y: Some(col("mpg")),
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
    parts.chart(root, vec![brush])
}

/// A selection's range with one open end pins that end of the fit, so the axis labels the
/// selection's domain, which starts at the pin, and the plot clips the marks before it.
#[test]
fn the_axis_of_a_selection_with_one_open_end_starts_at_the_selection() {
    let open = Box::new(|parts: &mut Parts| {
        let low = parts.value(numbers(&[2500.0]));
        vec![clause(
            "plot",
            condition::Kind::Range(proto::ValueRange {
                column: "weight".to_string(),
                channel: Channel::X as i32,
                low: Some(low),
                high: None,
                exclude_end: false,
            }),
        )]
    });
    let evaluated = evaluate(&selection_scatter(open), [320.0, 240.0]);
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
    let mut parts = Parts::new();
    let table = parts.table(cars());
    let mid = parts.value(numbers(&[3000.0]));
    let fit = proto::IntervalFit {
        domain_pin: Some(proto::interval_fit::DomainPin::Mid(mid)),
        ..Default::default()
    };
    let chart = parts.plot(vec![layer(
        table,
        cartesian_layer::Mark::Symbol(proto::SymbolMark {
            x: Some(scaled("weight", fitted(fit))),
            y: Some(col("mpg")),
            ..Default::default()
        }),
    )]);
    let (low, high) = domain(&evaluate(&chart, [320.0, 240.0]), 0);
    assert!(((low + high) / 2.0 - 3000.0).abs() < 1e-6, "{low} {high}");
    assert!(high >= 4790.0, "{high}");
}

/// A chart without a size from its host or its own takes its natural size, where a plot without
/// a size counts at its `contain-intrinsic-size`, and a host's size stretches the plot.
#[test]
fn a_chart_without_a_size_takes_its_natural_size() {
    let chart = cars_symbols(|_, mark| mark.color = Some(col("origin")));
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
