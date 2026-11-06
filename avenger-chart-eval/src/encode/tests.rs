//! The encoding of a layer's rows: series, baselines, conditional values, paints from the computed
//! styles, and the reach of marks.

use std::{collections::BTreeMap, sync::Arc};

use arrow::array::{ArrayRef, Float64Array, RecordBatch, StringArray};
use avenger_chart::{
    build::{self, ChartParts},
    proto::{self, cartesian_layer, conditional_value, encoding, Channel},
    Chart,
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

fn constant(chart_values: &mut ChartParts, value: ArrayRef) -> proto::Encoding {
    proto::Encoding {
        value: Some(encoding::Value::Constant(chart_values.value(value))),
        ..Default::default()
    }
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

/// A chart whose one layer draws `mark`, and the mark as the chart holds it.
fn chart_of(parts: ChartParts, mark: cartesian_layer::Mark) -> (Chart, cartesian_layer::Mark) {
    let mut parts = parts;
    let table = parts.table(table());
    let root = build::cartesian(vec![proto::CartesianLayer {
        table: Some(table),
        mark: Some(mark),
        ..Default::default()
    }]);
    let chart = parts.finish(build::message(root)).unwrap();
    let layer = match &chart.root().node {
        Some(proto::node::Node::Plot(plot)) => match &plot.coord {
            Some(proto::plot::Coord::Cartesian(cartesian)) => cartesian.layers[0].clone(),
            _ => unreachable!(),
        },
        _ => unreachable!(),
    };
    (chart, layer.mark.unwrap())
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
    let mark = cartesian_layer::Mark::Line(proto::LineMark {
        x: Some(build::scaled_column("a")),
        y: Some(build::scaled_column("b")),
        color: Some(build::column("group")),
        ..Default::default()
    });
    let (chart, mark) = chart_of(ChartParts::new(), mark);
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
    let mark = cartesian_layer::Mark::Bar(proto::BarMark {
        x: Some(build::scaled_column("a")),
        y: Some(build::scaled_column("b")),
        ..Default::default()
    });
    let (chart, mark) = chart_of(ChartParts::new(), mark);
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
    let mut parts = ChartParts::new();
    let selected = |parts: &mut ChartParts, color: &str| proto::ConditionalValue {
        predicate: Some(proto::Predicate {
            predicate: Some(proto::predicate::Predicate::Selected(proto::Selected {
                selection: "brush".to_string(),
                ..Default::default()
            })),
        }),
        value: Some(conditional_value::Value::Constant(
            parts.value(Arc::new(StringArray::from(vec![color]))),
        )),
    };
    let mut color = constant(&mut parts, Arc::new(StringArray::from(vec!["gray"])));
    color.conditional = vec![selected(&mut parts, "red"), selected(&mut parts, "blue")];
    let mark = cartesian_layer::Mark::Symbol(proto::SymbolMark {
        x: Some(build::scaled_column("a")),
        y: Some(build::scaled_column("b")),
        color: Some(color),
        ..Default::default()
    });
    let mut parts_with_selection = parts;
    let table = parts_with_selection.table(table());
    let mut message = build::message(build::cartesian(vec![proto::CartesianLayer {
        table: Some(table),
        mark: Some(mark),
        ..Default::default()
    }]));
    message.selections = vec![proto::Selection {
        name: "brush".to_string(),
        resolution: proto::SelectionResolution::Global as i32,
        ..Default::default()
    }];
    let chart = parts_with_selection.finish(message).unwrap();
    let mark = match &chart.root().node {
        Some(proto::node::Node::Plot(plot)) => match &plot.coord {
            Some(proto::plot::Coord::Cartesian(cartesian)) => {
                cartesian.layers[0].mark.clone().unwrap()
            }
            _ => unreachable!(),
        },
        _ => unreachable!(),
    };
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
    let mut parts = ChartParts::new();
    let color = constant(&mut parts, Arc::new(StringArray::from(vec!["red"])));
    let mark = cartesian_layer::Mark::Symbol(proto::SymbolMark {
        x: Some(build::scaled_column("a")),
        y: Some(build::scaled_column("b")),
        color: Some(color),
        ..Default::default()
    });
    let (chart, mark) = chart_of(parts, mark);
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
    let mut parts = ChartParts::new();
    let size = constant(&mut parts, Arc::new(Float64Array::from(vec![100.0])));
    let mark = cartesian_layer::Mark::Symbol(proto::SymbolMark {
        x: Some(build::scaled_column("a")),
        y: Some(build::scaled_column("b")),
        size: Some(size),
        ..Default::default()
    });
    let (chart, mark) = chart_of(parts, mark);
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

    let mut parts = ChartParts::new();
    let text = constant(&mut parts, Arc::new(StringArray::from(vec!["Gentoo"])));
    let mark = cartesian_layer::Mark::Text(proto::TextMark {
        x: Some(build::scaled_column("a")),
        y: Some(build::scaled_column("b")),
        text: Some(text),
        ..Default::default()
    });
    let (chart, mark) = chart_of(parts, mark);
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
        crate::marks::marks(&texts).unwrap()[0].mark,
        avenger_scenegraph::marks::mark::SceneMark::Text(_)
    ));
}
