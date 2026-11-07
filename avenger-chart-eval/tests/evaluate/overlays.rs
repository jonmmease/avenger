//! Plots of several layers: shared and independent scales, and the guides of both.

use std::sync::Arc;

use arrow::array::{Array, Float64Array, RecordBatch};
use avenger_chart::prelude::*;

use crate::support::*;

#[test]
fn bars_with_a_line_on_an_independent_y() {
    // Both layers keep the months in their order. The bars' band scale and the line's point
    // scale merge into a band scale, and the line stands at the middle of each band.
    let months_in_order = || {
        Encoding::scaled_field("month")
            .scale(Scale::builder().domain(Domain::explicit(Value::list(Arc::new(months())))))
    };
    let bars = CartesianLayer::bar(
        weather(),
        BarMark::builder()
            .x(months_in_order())
            .y(Encoding::scaled_field("precipitation").scale(
                Scale::builder()
                    .domain(DataDomain::builder().fit(IntervalFit::builder().zero(true))),
            )),
    );
    let line = CartesianLayer::line(
        weather(),
        LineMark::builder()
            .x(months_in_order())
            .y(Encoding::scaled_field("temperature"))
            .color(Encoding::direct_value("#e45756")),
    );
    let plot = Plot::cartesian(CartesianPlot::new([bars, line]))
        .resolve([Resolve::channel(Sharing::Independent, Channel::Y)]);
    let chart = Chart::builder(plot).build().unwrap();
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

#[test]
fn a_line_over_its_symbols() {
    let line = CartesianLayer::line(
        series(),
        LineMark::builder()
            .x(Encoding::scaled_field("date"))
            .y(Encoding::scaled_field("value"))
            .color(Encoding::scaled_field("symbol")),
    );
    let symbols = CartesianLayer::symbol(
        series(),
        SymbolMark::builder()
            .x(Encoding::scaled_field("date"))
            .y(Encoding::scaled_field("value"))
            .color(Encoding::scaled_field("symbol")),
    );
    let chart = Chart::builder(Plot::cartesian(CartesianPlot::new([line, symbols])))
        .build()
        .unwrap();
    render_both(&chart, "overlay-line-symbols");
}

#[test]
fn an_overlay_with_independent_colors() {
    // The legend of the line's series stands before the colorbar of the symbols' values, in the
    // order of the layers.
    let line = CartesianLayer::line(
        series(),
        LineMark::builder()
            .x(Encoding::scaled_field("date"))
            .y(Encoding::scaled_field("value"))
            .color(Encoding::scaled_field("symbol")),
    );
    let symbols = CartesianLayer::symbol(
        series(),
        SymbolMark::builder()
            .x(Encoding::scaled_field("date"))
            .y(Encoding::scaled_field("value"))
            .color(Encoding::scaled_field("value")),
    );
    let plot = Plot::cartesian(CartesianPlot::new([line, symbols]))
        .resolve([Resolve::channel(Sharing::Independent, Channel::Color)]);
    let chart = Chart::builder(plot).build().unwrap();
    render_both(&chart, "overlay-independent-colors");
}

#[test]
fn bars_share_y_with_a_rule_of_their_mean() {
    let bars = || {
        CartesianLayer::bar(
            categories(),
            BarMark::builder()
                .x(Encoding::scaled_field("category"))
                .y(Encoding::scaled_field("amount").scale(
                    Scale::builder()
                        .domain(DataDomain::builder().fit(IntervalFit::builder().zero(true))),
                )),
        )
    };
    let mean = || {
        let table = RecordBatch::try_from_iter([("mean", numbers(&[58.5]))]).unwrap();
        CartesianLayer::rule(
            table,
            RuleMark::builder()
                .y(Encoding::scaled_field("mean"))
                .color(Encoding::direct_value("#e45756")),
        )
    };

    let chart = Chart::builder(Plot::cartesian(CartesianPlot::new([bars(), mean()])))
        .build()
        .unwrap();
    render_both(&chart, "overlay-bars-mean");

    // With the rule first, the bars' fit, which contains zero, is still the merged fit.
    let chart = Chart::builder(Plot::cartesian(CartesianPlot::new([mean(), bars()])))
        .build()
        .unwrap();
    let evaluated = evaluate(&chart, [320.0, 240.0]);
    assert_eq!(domain(&evaluated, 1).0, 0.0);
    assert!(scene_texts(&evaluated.scene.marks).contains(&"mean, amount".to_string()));
}

#[test]
fn a_shared_y_of_a_summary_and_a_data_domain_fits_the_merge_of_both() {
    // The summary covers mpg from 0 to 30, and the rows of the second layer reach past 40.
    let summarized = CartesianLayer::symbol(
        cars(),
        SymbolMark::builder()
            .x(Encoding::scaled_field("weight"))
            .y(Encoding::scaled_field("mpg")
                .scale(Scale::builder().domain(SummaryDomain::new(linear_summary(&[0.0, 30.0]))))),
    );
    let rows = CartesianLayer::symbol(
        cars(),
        SymbolMark::builder()
            .x(Encoding::scaled_field("weight"))
            .y(Encoding::scaled_field("mpg")),
    );
    let chart = Chart::builder(Plot::cartesian(CartesianPlot::new([summarized, rows])))
        .build()
        .unwrap();

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
