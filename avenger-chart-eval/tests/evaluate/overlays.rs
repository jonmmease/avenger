//! Plots of several layers: shared and independent scales, and the guides of both.

use std::sync::Arc;

use arrow::array::{Array, Float64Array, RecordBatch};
use avenger_chart::{
    build::{self},
    proto::{self, cartesian_layer, resolve, Channel},
    Chart,
};

use crate::support::*;

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
