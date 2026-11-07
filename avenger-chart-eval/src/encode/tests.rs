//! The encoding of a layer's rows: series, baselines, conditional values, paints from the computed
//! styles, and the reach of marks.

use std::{collections::BTreeMap, sync::Arc};

use arrow::array::{ArrayRef, Float64Array, RecordBatch, StringArray};
use avenger_chart::{
    prelude::*,
    proto::{cartesian_layer, node, plot},
};
use avenger_color::ColorOrGradient;
use avenger_scales::scales::{linear::LinearScale, log::LogScale, ConfiguredScale};
use avenger_style::{
    Cascade, ComputedStyles, Element, Env, HostInputs, NodeId, ParseOptions, Sheet, SourceId,
    StyleTable, StyleTree,
};
use avenger_typst_label::LabelEngine;

use super::{at, LayerRows};
use crate::reach::reach;

fn engine() -> LabelEngine {
    avenger_typst_label::bundled_label_engine()
}

/// The computed style of a `mark` of `kind`, which `css` styles after the base stylesheet.
fn mark_style(kind: &str, css: &str) -> (ComputedStyles, NodeId) {
    let mut no_files = |url: &str, _: Option<&str>| Err(format!("no file `{url}`"));
    let options = ParseOptions {
        allow_import: true,
        ..Default::default()
    };
    let text = format!("@import 'std:base.css';\n{css}");
    let (sheet, diagnostics) = Sheet::load(&text, None, SourceId(1), options, &mut no_files);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    let cascade = Cascade::new(&StyleTable::new().with_author(sheet, Env::new()));
    let mut tree = StyleTree::new(Element::Chart);
    let plot = tree.add(tree.root(), Element::Plot);
    let node = tree.add(plot, Element::Mark);
    tree.set_attribute_forms(node, "type", kind, kind);
    (
        cascade.compute(&tree, &HostInputs::sized(400.0, 300.0)),
        node,
    )
}

fn table() -> RecordBatch {
    RecordBatch::try_from_iter([
        (
            "a",
            Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0])) as ArrayRef,
        ),
        ("b", Arc::new(Float64Array::from(vec![4.0, 6.0, 5.0, 7.0]))),
        (
            "group",
            Arc::new(StringArray::from(vec!["p", "q", "p", "q"])),
        ),
    ])
    .unwrap()
}

/// The first layer of `chart`, as the chart holds it.
fn first_layer(chart: &Chart) -> &CartesianLayer {
    match &chart.root().node {
        Some(node::Node::Plot(plot)) => match &plot.coord {
            Some(plot::Coord::Cartesian(cartesian)) => &cartesian.layers[0],
            _ => unreachable!(),
        },
        _ => unreachable!(),
    }
}

/// The mark of the first layer of `chart`.
fn first_mark(chart: &Chart) -> cartesian_layer::Mark {
    first_layer(chart).mark.clone().unwrap()
}

/// The rows of `mark` of `chart`, with its style, its scales, and its masks.
fn rows<'a>(
    chart: &'a Chart,
    mark: &'a cartesian_layer::Mark,
    styles: &'a ComputedStyles,
    node: NodeId,
    scales: BTreeMap<Channel, &'a ConfiguredScale>,
    partition: &'a BTreeMap<String, ArrayRef>,
    engine: &'a LabelEngine,
) -> LayerRows<'a> {
    LayerRows {
        chart,
        layer: avenger_chart::layer::LayerRef::Cartesian(first_layer(chart)),
        mark,
        style: &styles[node],
        styles,
        node,
        table: chart.tables()[0].clone(),
        partition,
        scales,
        masks: BTreeMap::new(),
        kinds: {
            let message: &dyn avenger_chart::layer::MarkMessage = mark;
            (message.encodings())
                .filter_map(|(channel, encoding)| Some((channel, encoding.scale.as_ref()?)))
                .map(|(channel, spec)| {
                    let kind = crate::convert::scale::fit_kind(spec, &crate::Registry::new());
                    (channel, kind.unwrap())
                })
                .collect()
        },
        engine,
        size: [400.0, 300.0],
        name: "layer".to_string(),
    }
}

fn colors(
    values: &avenger_common::value::ScalarOrArray<ColorOrGradient>,
    len: usize,
) -> Vec<[f32; 4]> {
    (0..len)
        .map(|row| match at(values, row) {
            ColorOrGradient::Color(color) => color,
            other => panic!("a color, not {other:?}"),
        })
        .collect()
}

#[test]
fn series_group_rows_by_their_color_in_the_order_of_first_rows() {
    let lines = CartesianLayer::line(
        table(),
        LineMark::builder()
            .x("a")
            .y("b")
            .color(Encoding::direct_field("group")),
    );
    let chart = Chart::builder(Plot::cartesian([lines])).build().unwrap();
    let mark = first_mark(&chart);
    let (styles, node) = mark_style("line", "");
    let (partition, engine) = (BTreeMap::new(), engine());
    let rows = rows(
        &chart,
        &mark,
        &styles,
        node,
        BTreeMap::new(),
        &partition,
        &engine,
    );
    assert_eq!(rows.series().unwrap(), [vec![0, 2], vec![1, 3]]);
}

#[test]
fn a_baseline_stands_at_zero_inside_the_range_or_at_its_start() {
    let bars = CartesianLayer::bar(table(), BarMark::builder().x("a").y("b"));
    let chart = Chart::builder(Plot::cartesian([bars])).build().unwrap();
    let mark = first_mark(&chart);
    let (styles, node) = mark_style("bar", "");
    let (partition, engine) = (BTreeMap::new(), engine());
    let linear = LinearScale::configured((-5.0, 10.0), (300.0, 0.0));
    let at_zero = rows(
        &chart,
        &mark,
        &styles,
        node,
        BTreeMap::from([(Channel::Y, &linear)]),
        &partition,
        &engine,
    );
    assert!((at_zero.baseline(Channel::Y).unwrap() - 200.0).abs() < 1e-3);
    let log = LogScale::configured((1.0, 100.0), (300.0, 0.0));
    let at_start = rows(
        &chart,
        &mark,
        &styles,
        node,
        BTreeMap::from([(Channel::Y, &log)]),
        &partition,
        &engine,
    );
    assert_eq!(at_start.baseline(Channel::Y).unwrap(), 300.0);
}

#[test]
fn the_first_conditional_value_that_selects_a_row_wins() {
    // Gray, unless the first of two conditional values that selects a row gives it red or blue.
    let selected =
        |color: &str| ConditionalValue::direct_value(Predicate::selected("brush"), color);
    let symbols = CartesianLayer::symbol(
        table(),
        SymbolMark::builder()
            .x("a")
            .y("b")
            .color(Encoding::direct_value("gray").conditional([selected("red"), selected("blue")])),
    );
    let chart = Chart::builder(Plot::cartesian([symbols]))
        .selections([Selection::new("brush", SelectionResolution::Global)])
        .build()
        .unwrap();
    let mark = first_mark(&chart);
    let (styles, node) = mark_style("symbol", "");
    let (partition, engine) = (BTreeMap::new(), engine());
    let mut rows = rows(
        &chart,
        &mark,
        &styles,
        node,
        BTreeMap::new(),
        &partition,
        &engine,
    );
    rows.masks = BTreeMap::from([(
        Channel::Color,
        vec![
            vec![true, false, false, true],
            vec![true, true, false, false],
        ],
    )]);
    let values = rows.colors(Channel::Color).unwrap().unwrap();
    let [red, blue, gray] = [[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], {
        let gray = 128.0 / 255.0;
        [gray, gray, gray, 1.0]
    }];
    let colors = colors(&values, 4);
    assert_eq!(colors[0], red);
    assert_eq!(colors[1], blue);
    assert!((colors[2][0] - gray[0]).abs() < 1e-3);
    assert_eq!(colors[3], red);
}

#[test]
fn paints_follow_the_computed_styles_and_the_color_channel() {
    let symbols = CartesianLayer::symbol(
        table(),
        SymbolMark::builder()
            .x("a")
            .y("b")
            .color(Encoding::direct_value("red")),
    );
    let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();
    let mark = first_mark(&chart);
    let (partition, engine) = (BTreeMap::new(), engine());
    // A filled symbol takes the color as its fill, and the base stylesheet strokes none.
    let (styles, node) = mark_style("symbol", "");
    let rows_filled = rows(
        &chart,
        &mark,
        &styles,
        node,
        BTreeMap::new(),
        &partition,
        &engine,
    );
    let (fill, stroke) = rows_filled.paint().unwrap();
    assert_eq!(colors(&fill, 1)[0], [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(colors(&stroke, 1)[0][3], 0.0);
    // A hollow symbol strokes in the row's color, as `currentColor` reads it.
    let (styles, node) = mark_style("symbol", "mark { fill: none; stroke: currentColor; }");
    let hollow = rows(
        &chart,
        &mark,
        &styles,
        node,
        BTreeMap::new(),
        &partition,
        &engine,
    );
    let (fill, stroke) = hollow.paint().unwrap();
    assert_eq!(colors(&fill, 1)[0][3], 0.0);
    assert_eq!(colors(&stroke, 1)[0], [1.0, 0.0, 0.0, 1.0]);
    // Opacity scales the alpha of both.
    let (styles, node) = mark_style("symbol", "mark { opacity: 0.5; }");
    let faded = rows(
        &chart,
        &mark,
        &styles,
        node,
        BTreeMap::new(),
        &partition,
        &engine,
    );
    let (fill, _) = faded.paint().unwrap();
    assert!((colors(&fill, 1)[0][3] - 0.5).abs() < 1e-6);
}

#[test]
fn a_symbol_reaches_half_its_width_and_a_text_its_bounds() {
    let symbols = CartesianLayer::symbol(table(), SymbolMark::builder().x("a").y("b").size(100.0));
    let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();
    let mark = first_mark(&chart);
    let (styles, node) = mark_style("symbol", "");
    let (partition, engine) = (BTreeMap::new(), engine());
    let symbols = rows(
        &chart,
        &mark,
        &styles,
        node,
        BTreeMap::new(),
        &partition,
        &engine,
    );
    let symbol_reach = reach(&symbols).unwrap().unwrap();
    assert_eq!(symbol_reach.len(), 4);
    // A circle of area 100 is 10 wide, bounded by its polygon of 16 sides.
    for side in symbol_reach[0] {
        assert!((5.0..5.2).contains(&side), "{:?}", symbol_reach[0]);
    }

    let texts = CartesianLayer::text(
        table(),
        TextMark::new(Encoding::direct_value("Gentoo"))
            .x("a")
            .y("b"),
    );
    let chart = Chart::builder(Plot::cartesian([texts])).build().unwrap();
    let mark = first_mark(&chart);
    let (styles, node) = mark_style("text", "");
    let texts = rows(
        &chart,
        &mark,
        &styles,
        node,
        BTreeMap::new(),
        &partition,
        &engine,
    );
    let text_reach = reach(&texts).unwrap().unwrap();
    let [left, right, top, bottom] = text_reach[0];
    assert!(
        left > 10.0 && (left - right).abs() < 1e-3,
        "{:?}",
        text_reach[0]
    );
    assert!(
        top > 2.0 && (top - bottom).abs() < 1.0,
        "{:?}",
        text_reach[0]
    );
    assert!(matches!(
        crate::marks::marks(&texts, &mark).unwrap()[0].mark,
        avenger_scenegraph::marks::mark::SceneMark::Text(_)
    ));
}
