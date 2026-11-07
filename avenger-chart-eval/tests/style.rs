//! The style tree of a chart: its nodes and attributes, the format's style fields as inline styles,
//! and the computed styles of the host inputs.

use std::sync::Arc;

use arrow::{
    array::{ArrayRef, Float64Array, RecordBatch, StringArray, StructArray},
    datatypes::{DataType, Field},
};
use avenger_chart::prelude::*;
use avenger_chart_eval::{ChartEvaluator, EvalWarning, Registry};
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
    // Symbols of mass against flipper length, colored by species, and symbols filled by mass.
    let by_species = CartesianLayer::symbol(
        penguins(),
        SymbolMark::builder()
            .x(Encoding::scaled_field("flipper").scale(Axis::builder().id("flippers")))
            .y("mass")
            .color("species"),
    )
    .id("points");
    let by_mass = CartesianLayer::symbol(
        penguins(),
        SymbolMark::builder()
            .x("flipper")
            .y("mass")
            .fill("mass")
            .filled(false),
    );
    let chart = Chart::builder(Plot::cartesian([by_species, by_mass]))
        .build()
        .unwrap();
    let evaluator = evaluator(chart, cascade(""));
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
    // Bars over a nested band scale of regions and channels.
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
    let sales = RecordBatch::try_from_iter([
        ("key", Arc::new(keys) as ArrayRef),
        ("sales", Arc::new(Float64Array::from(vec![3.0, 5.0, 2.0]))),
    ])
    .unwrap();
    let bars = CartesianLayer::bar(sales, BarMark::builder().x("key").y("sales"));
    let chart = Chart::builder(Plot::cartesian([bars])).build().unwrap();
    let evaluator = evaluator(chart, cascade(""));
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
    // Symbols of mass against flipper length, colored by species, and symbols filled by mass,
    // with an axis whose labels the chart turns.
    let by_species = CartesianLayer::symbol(
        penguins(),
        SymbolMark::builder()
            .x(Encoding::scaled_field("flipper")
                .scale(Axis::builder().label_angle(-45.0).id("flippers")))
            .y("mass")
            .color("species"),
    )
    .id("points");
    let by_mass = CartesianLayer::symbol(
        penguins(),
        SymbolMark::builder()
            .x("flipper")
            .y("mass")
            .fill("mass")
            .filled(false),
    );
    let turned = Chart::builder(Plot::cartesian([by_species, by_mass]))
        .build()
        .unwrap();
    let mut turned = evaluator(turned, cascade(css));
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

    // Without the field, the stylesheet gives the angle: the same symbols, whose axis sets no
    // label angle.
    let by_species = CartesianLayer::symbol(
        penguins(),
        SymbolMark::builder()
            .x(Encoding::scaled_field("flipper").scale(Axis::builder().id("flippers")))
            .y("mass")
            .color("species"),
    )
    .id("points");
    let by_mass = CartesianLayer::symbol(
        penguins(),
        SymbolMark::builder()
            .x("flipper")
            .y("mass")
            .fill("mass")
            .filled(false),
    );
    let plain = Chart::builder(Plot::cartesian([by_species, by_mass]))
        .build()
        .unwrap();
    let mut plain = evaluator(plain, cascade(css));
    let styles = plain.styles(&host);
    assert_eq!(styles[axis].number(Property::LabelAngle), Some(10.0));
}

#[test]
fn a_table_without_stylesheets_gives_initial_values() {
    let empty = Arc::new(Cascade::new(&StyleTable::new()));
    // Symbols of mass against flipper length, colored by species, and symbols filled by mass.
    let by_species = CartesianLayer::symbol(
        penguins(),
        SymbolMark::builder()
            .x(Encoding::scaled_field("flipper").scale(Axis::builder().id("flippers")))
            .y("mass")
            .color("species"),
    )
    .id("points");
    let by_mass = CartesianLayer::symbol(
        penguins(),
        SymbolMark::builder()
            .x("flipper")
            .y("mass")
            .fill("mass")
            .filled(false),
    );
    let chart = Chart::builder(Plot::cartesian([by_species, by_mass]))
        .build()
        .unwrap();
    let mut evaluator = evaluator(chart, empty);
    let tick = nodes_of(evaluator.style_tree(), Element::Tick)[0];
    let styles = evaluator.styles(&HostInputs::sized(400.0, 300.0));
    assert_eq!(styles[tick].number(Property::TickSize), Some(5.0));
}

#[test]
fn a_resize_within_a_breakpoint_keeps_the_computed_styles() {
    // Symbols of mass against flipper length, colored by species, and symbols filled by mass.
    let by_species = CartesianLayer::symbol(
        penguins(),
        SymbolMark::builder()
            .x(Encoding::scaled_field("flipper").scale(Axis::builder().id("flippers")))
            .y("mass")
            .color("species"),
    )
    .id("points");
    let by_mass = CartesianLayer::symbol(
        penguins(),
        SymbolMark::builder()
            .x("flipper")
            .y("mass")
            .fill("mass")
            .filled(false),
    );
    let chart = Chart::builder(Plot::cartesian([by_species, by_mass]))
        .build()
        .unwrap();
    let mut evaluator = evaluator(
        chart,
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

/// A field that shows a part shows it over a stylesheet that hides it, and a stroke of a
/// background draws a solid border.
#[test]
fn style_fields_show_parts_and_stroke_backgrounds() {
    // A scatter of the penguins with an x axis that shows its grid and a color legend with a red
    // stroke.
    let symbols = CartesianLayer::symbol(
        penguins(),
        SymbolMark::builder()
            .x(Encoding::scaled_field("flipper").scale(Axis::builder().show_grid(true)))
            .y("mass")
            .color(
                Encoding::scaled_field("species")
                    .scale(Legend::builder().background(Background::builder().stroke("red"))),
            ),
    );
    let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();
    let mut evaluator = evaluator(chart, cascade("grid { display: none; }"));
    let tree = evaluator.style_tree();
    let grids = nodes_of(tree, Element::Grid);
    let x_grid = (grids.iter().copied())
        .find(|grid| attribute(tree, tree.parent(*grid).unwrap(), "channel") == "x")
        .unwrap();
    let y_grid = (grids.iter().copied())
        .find(|grid| *grid != x_grid)
        .unwrap();
    let legend = nodes_of(tree, Element::Legend)[0];
    let styles = evaluator.styles(&HostInputs::sized(400.0, 300.0));
    assert!(!styles[x_grid].is_hidden());
    assert!(styles[y_grid].is_hidden());
    assert_eq!(styles[legend].keyword(Property::BorderStyle), Some("solid"));
    assert_eq!(
        styles[legend].color(Property::BorderColor),
        Some([1.0, 0.0, 0.0, 1.0])
    );
}

/// The reader does not check the CSS text of style fields, so text that is not a value of its
/// property sets nothing and warns.
#[test]
fn style_text_that_is_not_a_value_warns() {
    // A scatter of the penguins with an x axis whose domain color is not a color, and a color
    // legend.
    let symbols = CartesianLayer::symbol(
        penguins(),
        SymbolMark::builder()
            .x(Encoding::scaled_field("flipper").scale(Axis::builder().domain_color("bluish")))
            .y("mass")
            .color(Encoding::scaled_field("species").scale(Legend::builder())),
    );
    let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();
    let evaluator = evaluator(chart, cascade(""));
    assert!(
        evaluator.warnings().iter().any(|warning| matches!(warning,
            EvalWarning::NotDrawn { reason, .. } if reason.contains("`bluish`"))),
        "{:?}",
        evaluator.warnings()
    );
}

/// The leaf of a nested band axis takes the axis's label angle, and an outer level stays level.
#[test]
fn the_leaf_of_a_nested_band_axis_takes_the_axis_label_angle() {
    // Bars over a nested band scale of regions and channels.
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
    let sales = RecordBatch::try_from_iter([
        ("key", Arc::new(keys) as ArrayRef),
        ("sales", Arc::new(Float64Array::from(vec![3.0, 5.0, 2.0]))),
    ])
    .unwrap();
    let bars = CartesianLayer::bar(sales, BarMark::builder().x("key").y("sales"));
    let chart = Chart::builder(Plot::cartesian([bars])).build().unwrap();
    let mut evaluator = evaluator(chart, cascade("axis { label-angle: 30deg; }"));
    let tree = evaluator.style_tree();
    let axis = (nodes_of(tree, Element::Axis).into_iter())
        .find(|axis| attribute(tree, *axis, "channel") == "x")
        .unwrap();
    let levels: Vec<NodeId> = (tree.children(axis).iter().copied())
        .filter(|child| tree.node(*child).element == Element::Level)
        .collect();
    let styles = evaluator.styles(&HostInputs::sized(400.0, 300.0));
    assert_eq!(styles[levels[0]].number(Property::LabelAngle), Some(0.0));
    assert_eq!(styles[levels[1]].number(Property::LabelAngle), Some(30.0));
}

/// A select without a form is an interval, so its store has the `mark` of a box, as an interval
/// select's does. A lasso's store is a path, and a point selection has no store.
#[test]
fn a_select_without_a_form_stores_a_box() {
    let stores = |select: SelectBuilder| {
        let symbols =
            CartesianLayer::symbol(penguins(), SymbolMark::builder().x("flipper").y("mass"));
        let chart = Chart::builder(Plot::cartesian([symbols]).id("plot").tools([select]))
            .selections([Selection::new("brush", SelectionResolution::Global)])
            .build()
            .unwrap();
        let evaluator = evaluator(chart, cascade(""));
        let tree = evaluator.style_tree();
        (nodes_of(tree, Element::Mark).into_iter())
            .filter(|node| attribute(tree, *node, "store") == "brush")
            .map(|node| attribute(tree, node, "type"))
            .collect::<Vec<_>>()
    };
    assert_eq!(stores(Select::new("brush")), ["rect"]);
    assert_eq!(
        stores(Select::interval("brush", IntervalSelect::builder())),
        ["rect"]
    );
    assert_eq!(
        stores(Select::lasso("brush", Vec::<String>::new())),
        ["path"]
    );
    assert_eq!(
        stores(Select::points(
            "brush",
            PointSelect::builder().keys(["species"])
        )),
        Vec::<String>::new()
    );
}
