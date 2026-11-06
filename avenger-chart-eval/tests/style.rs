//! The style tree of a chart: its nodes and attributes, the format's style fields as inline styles,
//! and the computed styles of the host inputs.

use std::sync::Arc;

use arrow::{
    array::{ArrayRef, Float64Array, RecordBatch, StringArray, StructArray},
    datatypes::{DataType, Field},
};
use avenger_chart::{
    build::{self, ChartParts},
    proto::{self, cartesian_layer, domain, guide, scale_spec},
    Chart,
};
use avenger_chart_eval::{ChartEvaluator, Registry};
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_style::{
    Cascade, Element, Env, HostInputs, NodeId, ParseOptions, Property, Sheet, SourceId, StyleTable,
    StyleTree,
};
use avenger_typst_label::LabelEngine;

fn engine() -> LabelEngine {
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
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

fn penguins() -> RecordBatch {
    RecordBatch::try_from_iter([
        (
            "flipper",
            Arc::new(Float64Array::from(vec![181.0, 186.0, 195.0, 210.0])) as ArrayRef,
        ),
        (
            "mass",
            Arc::new(Float64Array::from(vec![3750.0, 3800.0, 3250.0, 4500.0])),
        ),
        (
            "species",
            Arc::new(StringArray::from(vec![
                "Adelie", "Adelie", "Gentoo", "Gentoo",
            ])),
        ),
    ])
    .unwrap()
}

/// Symbols of mass against flipper length, colored by species, and symbols filled by mass, with
/// an axis whose labels the chart turns.
fn scatter(label_angle: Option<f32>) -> Chart {
    let mut parts = ChartParts::new();
    let table = parts.table(penguins());
    let mut x = build::scaled_column("flipper");
    x.scale.as_mut().unwrap().guide = Some(proto::Guide {
        guide: Some(guide::Guide::Axis(proto::Axis {
            label_angle,
            id: "flippers".to_string(),
            ..Default::default()
        })),
    });
    let by_species = proto::CartesianLayer {
        id: "points".to_string(),
        table: Some(table),
        mark: Some(cartesian_layer::Mark::Symbol(proto::SymbolMark {
            x: Some(x),
            y: Some(build::scaled_column("mass")),
            color: Some(build::scaled_column("species")),
            ..Default::default()
        })),
        ..Default::default()
    };
    let by_mass = proto::CartesianLayer {
        table: Some(table),
        mark: Some(cartesian_layer::Mark::Symbol(proto::SymbolMark {
            x: Some(build::scaled_column("flipper")),
            y: Some(build::scaled_column("mass")),
            fill: Some(build::scaled_column("mass")),
            filled: Some(false),
            ..Default::default()
        })),
        ..Default::default()
    };
    parts
        .finish(build::message(build::cartesian(vec![by_species, by_mass])))
        .unwrap()
}

/// Bars over a nested band scale of regions and channels.
fn nested() -> Chart {
    let mut parts = ChartParts::new();
    let keys = StructArray::from(vec![
        (
            Arc::new(Field::new("region", DataType::Utf8, false)),
            Arc::new(StringArray::from(vec!["East", "East", "West"])) as ArrayRef,
        ),
        (
            Arc::new(Field::new("channel", DataType::Utf8, false)),
            Arc::new(StringArray::from(vec!["Web", "Store", "Web"])) as ArrayRef,
        ),
    ]);
    let table = parts.table(
        RecordBatch::try_from_iter([
            ("key", Arc::new(keys) as ArrayRef),
            ("sales", Arc::new(Float64Array::from(vec![3.0, 5.0, 2.0]))),
        ])
        .unwrap(),
    );
    let mut x = build::column("key");
    x.scale = Some(proto::ScaleSpec {
        r#type: Some(scale_spec::Type::NestedBand(
            proto::NestedBandScale::default(),
        )),
        domain: Some(proto::Domain {
            domain: Some(domain::Domain::Data(proto::DataDomain::default())),
        }),
        ..Default::default()
    });
    let bars = proto::CartesianLayer {
        table: Some(table),
        mark: Some(cartesian_layer::Mark::Bar(proto::BarMark {
            x: Some(x),
            y: Some(build::scaled_column("sales")),
            ..Default::default()
        })),
        ..Default::default()
    };
    parts
        .finish(build::message(build::cartesian(vec![bars])))
        .unwrap()
}

fn evaluator(chart: Chart, cascade: Arc<Cascade>) -> ChartEvaluator {
    ChartEvaluator::new(chart, cascade, engine(), Registry::new()).unwrap()
}

/// The nodes of `element` in tree order.
fn nodes_of(tree: &StyleTree, element: Element) -> Vec<NodeId> {
    tree.nodes()
        .filter(|node| tree.node(*node).element == element)
        .collect()
}

fn attribute(tree: &StyleTree, node: NodeId, name: &str) -> String {
    tree.node(node)
        .attribute_str(name)
        .unwrap_or_default()
        .to_string()
}

fn elements(tree: &StyleTree, node: NodeId) -> Vec<Element> {
    (tree.children(node).iter())
        .map(|child| tree.node(*child).element)
        .collect()
}

#[test]
fn the_style_tree_has_a_node_for_each_part() {
    let evaluator = evaluator(scatter(None), cascade(""));
    let tree = evaluator.style_tree();
    let plot = nodes_of(tree, Element::Plot);
    assert_eq!(plot.len(), 1);
    assert_eq!(attribute(tree, plot[0], "type"), "cartesian");

    let marks = nodes_of(tree, Element::Mark);
    assert_eq!(marks.len(), 2);
    assert_eq!(attribute(tree, marks[0], "type"), "symbol");
    assert_eq!(tree.node(marks[0]).id.as_deref(), Some("points"));

    let scales = nodes_of(tree, Element::Scale);
    let described: Vec<(String, String, String)> = (scales.iter())
        .map(|node| {
            (
                attribute(tree, *node, "channel"),
                attribute(tree, *node, "type"),
                attribute(tree, *node, "kind"),
            )
        })
        .collect();
    let has = |channel: &str, type_name: &str, kind: &str| {
        described.contains(&(channel.to_string(), type_name.to_string(), kind.to_string()))
    };
    assert!(has("x", "linear", "sequential"), "{described:?}");
    assert!(has("color", "ordinal", "categorical"), "{described:?}");
    assert!(has("fill", "linear", "sequential"), "{described:?}");

    let axes = nodes_of(tree, Element::Axis);
    assert_eq!(axes.len(), 2);
    let x_axis = (axes.iter())
        .find(|axis| attribute(tree, **axis, "channel") == "x")
        .unwrap();
    assert_eq!(tree.node(*x_axis).id.as_deref(), Some("flippers"));
    assert_eq!(
        elements(tree, *x_axis),
        [
            Element::Title,
            Element::Label,
            Element::Tick,
            Element::Rule,
            Element::Grid
        ]
    );

    let legends = nodes_of(tree, Element::Legend);
    let types: Vec<String> = legends
        .iter()
        .map(|node| attribute(tree, *node, "type"))
        .collect();
    assert_eq!(types, ["symbol", "colorbar"]);
    assert_eq!(
        elements(tree, legends[0]),
        [Element::Title, Element::Swatch, Element::Label]
    );
    assert_eq!(
        elements(tree, legends[1]),
        [
            Element::Title,
            Element::Ramp,
            Element::Label,
            Element::Tick,
            Element::Rule
        ]
    );
}

#[test]
fn a_nested_band_axis_has_a_level_for_each_level() {
    let evaluator = evaluator(nested(), cascade(""));
    let tree = evaluator.style_tree();
    let axis = (nodes_of(tree, Element::Axis).into_iter())
        .find(|axis| attribute(tree, *axis, "channel") == "x")
        .unwrap();
    let levels: Vec<NodeId> = (tree.children(axis).iter().copied())
        .filter(|child| tree.node(*child).element == Element::Level)
        .collect();
    assert_eq!(levels.len(), 2);
    assert_eq!(elements(tree, levels[0]), [Element::Label]);
    let plot = nodes_of(tree, Element::Plot)[0];
    assert_eq!(attribute(tree, plot, "discrete"), "x");
}

#[test]
fn style_fields_are_inline_styles_that_win_over_stylesheets() {
    let css = "axis { label-angle: 10deg; } mark { stroke-width: 3px; }";
    let mut turned = evaluator(scatter(Some(-45.0)), cascade(css));
    let host = HostInputs::sized(400.0, 300.0);
    let axis = {
        let tree = turned.style_tree();
        (nodes_of(tree, Element::Axis).into_iter())
            .find(|axis| attribute(tree, *axis, "channel") == "x")
            .unwrap()
    };
    let hollow = nodes_of(turned.style_tree(), Element::Mark)[1];
    let styles = turned.styles(&host);
    assert_eq!(styles[axis].number(Property::LabelAngle), Some(-45.0));
    // `filled: false` declares no fill and a stroke of the mark's color.
    assert_eq!(
        styles[hollow].paint(Property::Fill),
        avenger_style::Paint::None
    );

    // Without the field, the stylesheet gives the angle.
    let mut plain = evaluator(scatter(None), cascade(css));
    let styles = plain.styles(&host);
    assert_eq!(styles[axis].number(Property::LabelAngle), Some(10.0));
}

#[test]
fn a_table_without_stylesheets_gives_initial_values() {
    let empty = Arc::new(Cascade::new(&StyleTable::new()));
    let mut evaluator = evaluator(scatter(None), empty);
    let tick = nodes_of(evaluator.style_tree(), Element::Tick)[0];
    let styles = evaluator.styles(&HostInputs::sized(400.0, 300.0));
    assert_eq!(styles[tick].number(Property::TickSize), Some(5.0));
}

#[test]
fn a_resize_within_a_breakpoint_keeps_the_computed_styles() {
    let mut evaluator = evaluator(
        scatter(None),
        cascade("@container (width < 300px) { axis { label-angle: 30deg; } }"),
    );
    let first: *const _ = evaluator.styles(&HostInputs::sized(400.0, 300.0));
    let resized: *const _ = evaluator.styles(&HostInputs::sized(500.0, 320.0));
    assert!(std::ptr::eq(first, resized));
    let narrow: *const _ = evaluator.styles(&HostInputs::sized(250.0, 320.0));
    let axis = nodes_of(evaluator.style_tree(), Element::Axis)[0];
    let styles = evaluator.styles(&HostInputs::sized(250.0, 320.0));
    assert_eq!(styles[axis].number(Property::LabelAngle), Some(30.0));
    let _ = narrow;
}
