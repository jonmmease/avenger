//! The style tree of a chart: its nodes and attributes, the format's style fields as inline styles,
//! and the computed styles of the host inputs.

use std::sync::Arc;

use arrow::{
    array::{ArrayRef, Float64Array, RecordBatch, StringArray, StructArray},
    datatypes::{DataType, Field},
};
use avenger_chart::prelude::*;
use avenger_chart_eval::{ChartEvaluator, EvalWarning, EvaluatedChart, Registry};
use avenger_common::types::{StrokeCap, StrokeJoin};
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_style::{
    Cascade, Element, Env, HostInputs, NodeId, ParseOptions, Property, Sheet, SourceId, StyleTable,
    StyleTree,
};
use avenger_typst_label::{LabelEngine, LabelLineHeight, LabelWidth};

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

/// The child of `node` with `element`.
fn part(tree: &StyleTree, node: NodeId, element: Element) -> NodeId {
    (tree.children(node).iter().copied())
        .find(|child| tree.node(*child).element == element)
        .unwrap()
}

/// The angles of the text marks of a scene, in degrees, one for each text.
fn text_angles(evaluated: &EvaluatedChart) -> Vec<f32> {
    fn walk(marks: &[SceneMark], out: &mut Vec<f32>) {
        for mark in marks {
            match mark {
                SceneMark::Text(text) => out.extend(text.angle.as_vec(text.len as usize, None)),
                SceneMark::Group(group) => walk(&group.marks, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&evaluated.scene.marks, &mut out);
    out
}

#[test]
fn the_style_tree_has_a_node_for_each_part() {
    // Symbols of mass against flipper length, colored by species, and symbols filled by mass.
    let by_species = CartesianLayer::symbol(
        penguins(),
        SymbolMark::builder()
            .x(Encoding::scaled_field("flipper").axis(Axis::builder().id("flippers")))
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
    let css = "axis > label { angle: 10deg; } mark { stroke-width: 3px; }";
    // Symbols of mass against flipper length, colored by species, and symbols filled by mass,
    // with an axis whose labels the chart turns.
    let by_species = CartesianLayer::symbol(
        penguins(),
        SymbolMark::builder()
            .x(Encoding::scaled_field("flipper").axis(
                Axis::builder()
                    .labels(Labels::builder().angle(-45.0))
                    .id("flippers"),
            ))
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
    let label = {
        let tree = turned.style_tree();
        let axis = (nodes_of(tree, Element::Axis).into_iter())
            .find(|axis| attribute(tree, *axis, "channel") == "x")
            .unwrap();
        part(tree, axis, Element::Label)
    };
    let hollow = nodes_of(turned.style_tree(), Element::Mark)[1];
    let styles = turned.styles(&host);
    assert_eq!(styles[label].number(Property::Angle), Some(-45.0));
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
            .x(Encoding::scaled_field("flipper").axis(Axis::builder().id("flippers")))
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
    assert_eq!(styles[label].number(Property::Angle), Some(10.0));
}

#[test]
fn a_table_without_stylesheets_gives_initial_values() {
    let empty = Arc::new(Cascade::new(&StyleTable::new()));
    // Symbols of mass against flipper length, colored by species, and symbols filled by mass.
    let by_species = CartesianLayer::symbol(
        penguins(),
        SymbolMark::builder()
            .x(Encoding::scaled_field("flipper").axis(Axis::builder().id("flippers")))
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
    assert_eq!(styles[tick].number(Property::Length), Some(5.0));
}

#[test]
fn a_resize_within_a_breakpoint_keeps_the_computed_styles() {
    // Symbols of mass against flipper length, colored by species, and symbols filled by mass.
    let by_species = CartesianLayer::symbol(
        penguins(),
        SymbolMark::builder()
            .x(Encoding::scaled_field("flipper").axis(Axis::builder().id("flippers")))
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
        cascade("@container (width < 300px) { axis > label { angle: 30deg; } }"),
    );
    let first: *const _ = evaluator.styles(&HostInputs::sized(400.0, 300.0));
    let resized: *const _ = evaluator.styles(&HostInputs::sized(500.0, 320.0));
    assert!(std::ptr::eq(first, resized));
    let narrow: *const _ = evaluator.styles(&HostInputs::sized(250.0, 320.0));
    let axis = nodes_of(evaluator.style_tree(), Element::Axis)[0];
    let label = part(evaluator.style_tree(), axis, Element::Label);
    let styles = evaluator.styles(&HostInputs::sized(250.0, 320.0));
    assert_eq!(styles[label].number(Property::Angle), Some(30.0));
    let _ = narrow;
}

/// A field that shows a part shows it over a stylesheet that hides it, and a border color draws a
/// solid border.
#[test]
fn style_fields_show_parts_and_border_backgrounds() {
    // A scatter of the penguins with an x axis that shows its grid and a color legend with a red
    // border.
    let symbols = CartesianLayer::symbol(
        penguins(),
        SymbolMark::builder()
            .x(Encoding::scaled_field("flipper").axis(Axis::builder().grid(true)))
            .y("mass")
            .color(Encoding::scaled_field("species").legend(Legend::builder().border_color("red"))),
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
    // A scatter of the penguins with an x axis whose rule color is not a color, and a color
    // legend.
    let symbols = CartesianLayer::symbol(
        penguins(),
        SymbolMark::builder()
            .x(Encoding::scaled_field("flipper").axis(Axis::builder().rule_stroke("bluish")))
            .y("mass")
            .color(Encoding::scaled_field("species").legend(Legend::builder())),
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

/// The labels of the leaf of a nested band axis take the angle of the axis's labels, and the labels
/// of an outer level stay level.
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
    let mut evaluator = evaluator(chart, cascade("axis[channel=x] > label { angle: 30deg; }"));
    let tree = evaluator.style_tree();
    let axis = (nodes_of(tree, Element::Axis).into_iter())
        .find(|axis| attribute(tree, *axis, "channel") == "x")
        .unwrap();
    let labels: Vec<NodeId> = (tree.children(axis).iter().copied())
        .filter(|child| tree.node(*child).element == Element::Level)
        .map(|level| part(tree, level, Element::Label))
        .collect();
    let host = HostInputs::sized(400.0, 300.0);
    let styles = evaluator.styles(&host);
    assert_eq!(styles[labels[0]].number(Property::Angle), Some(0.0));
    // `auto` on the leaf's labels takes the angle of the axis's labels where the axis draws them.
    assert_eq!(styles[labels[1]].keyword(Property::Angle), Some("auto"));
    let angles = text_angles(&evaluator.evaluate(&host).unwrap());
    // The labels of the 3 channels turn, and those of the regions stay level.
    let turned = angles.iter().filter(|angle| **angle == 30.0).count();
    assert_eq!(turned, 3, "{angles:?}");
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
        stores(Select::point(
            "brush",
            PointSelect::builder().keys(["species"])
        )),
        Vec::<String>::new()
    );
}

/// A Boolean hides a part of an axis through its `show`, and the title and the labels of an axis
/// take their fonts without a text of their own.
#[test]
fn booleans_hide_the_parts_of_an_axis() {
    // A scatter of the penguins whose x axis hides its ticks, its title, and its labels, and whose
    // y axis styles its derived title.
    let symbols = CartesianLayer::symbol(
        penguins(),
        SymbolMark::builder()
            .x(Encoding::scaled_field("flipper")
                .axis(Axis::builder().ticks(false).title(false).labels(false)))
            .y(Encoding::scaled_field("mass")
                .axis(Axis::builder().title(Title::builder().font_size(17.0)))),
    );
    let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();
    let mut evaluator = evaluator(chart, cascade(""));
    let tree = evaluator.style_tree();
    let axis = |channel: &str| {
        (nodes_of(tree, Element::Axis).into_iter())
            .find(|axis| attribute(tree, *axis, "channel") == channel)
            .unwrap()
    };
    let part = |axis: NodeId, element: Element| {
        (tree.children(axis).iter().copied())
            .find(|child| tree.node(*child).element == element)
            .unwrap()
    };
    let (x, y) = (axis("x"), axis("y"));
    let hidden = [
        part(x, Element::Tick),
        part(x, Element::Title),
        part(x, Element::Label),
    ];
    let shown = [part(y, Element::Tick), part(y, Element::Label)];
    let y_title = part(y, Element::Title);
    let styles = evaluator.styles(&HostInputs::sized(400.0, 300.0));
    assert!(hidden.iter().all(|node| styles[*node].is_hidden()));
    assert!(shown.iter().all(|node| !styles[*node].is_hidden()));
    assert_eq!(styles[y_title].font_size(), 17.0);
}

/// Evaluates `chart` at 400 by 300 with the base stylesheet and `css`.
fn evaluated(chart: &Chart, css: &str) -> EvaluatedChart {
    evaluator(chart.clone(), cascade(css))
        .evaluate(&HostInputs::sized(400.0, 300.0))
        .unwrap()
}

/// The number of the texts of a scene.
fn text_count(evaluated: &EvaluatedChart) -> usize {
    text_angles(evaluated).len()
}

/// The left, the top, the width, and the height of each plot, less those of `plain`.
fn moved(evaluated: &EvaluatedChart, plain: &EvaluatedChart) -> Vec<[f32; 4]> {
    (evaluated.plots.iter().zip(&plain.plots))
        .map(|(plot, plain)| std::array::from_fn(|index| plot.rect[index] - plain.rect[index]))
        .collect()
}

/// The settings of a guide's labels in the format are inline styles of its `label` node, so they
/// draw what the stylesheet's rule of the same property draws.
#[test]
fn the_settings_of_labels_draw_as_their_stylesheet_rules() {
    let chart = |axis: AxisBuilder| {
        Chart::builder(Plot::cartesian([CartesianLayer::symbol(
            penguins(),
            SymbolMark::builder()
                .x(Encoding::scaled_field("flipper").axis(axis))
                .y("mass"),
        )]))
        .build()
        .unwrap()
    };
    let inline = evaluated(
        &chart(Axis::builder().labels(Labels::builder().angle(-45.0).text_align(TextAlign::End))),
        "",
    );
    let styled = evaluated(
        &chart(Axis::builder()),
        "axis[channel=x] > label { angle: -45; text-align: end; }",
    );
    assert!(text_angles(&inline).contains(&-45.0));
    assert_eq!(format!("{:?}", inline.scene), format!("{:?}", styled.scene));
}

/// A stylesheet that sets a property of a guide's part on the part, or a `value-` property on the
/// guide, changes what the guide draws.
#[test]
fn the_properties_of_parts_reach_their_guides() {
    // Symbols of mass against flipper length, colored by species, with a symbol legend.
    let species = Chart::builder(Plot::cartesian([CartesianLayer::symbol(
        penguins(),
        SymbolMark::builder()
            .x("flipper")
            .y("mass")
            .color("species"),
    )]))
    .build()
    .unwrap();
    let plain = evaluated(&species, "");
    let turned = evaluated(&species, "axis[channel=x] > label { angle: -45; }");
    assert!(!text_angles(&plain).contains(&-45.0));
    assert!(text_angles(&turned).contains(&-45.0));
    let fewer = evaluated(&species, "axis[channel=x] { value-count: 3; }");
    assert!(text_count(&fewer) < text_count(&plain));
    let spread = evaluated(&species, "axis { value-spacing: 100; }");
    assert!(text_count(&spread) < text_count(&plain));
    let crowded = evaluated(&species, "axis[channel=x] { value-count: 40; }");
    let overlapping = evaluated(
        &species,
        "axis[channel=x] { value-count: 40; } axis[channel=x] > label { overlap: none; }",
    );
    // Crowded labels keep apart unless `overlap: none` shows them all.
    assert!(text_count(&overlapping) > text_count(&crowded));
    // On a mark, `auto` is the initial 0, for the marks and the swatches of their legend.
    let auto = evaluated(&species, "mark { angle: auto; }");
    assert_eq!(format!("{:?}", auto.scene), format!("{:?}", plain.scene));
    // Longer ticks move the plot 4 from the y axis and the x axis, and labels 12 from their
    // swatches, 10 more than the initial 2, narrow it by 10.
    let long = evaluated(&species, "axis > tick { length: 9; }");
    assert_eq!(moved(&long, &plain), [[4.0, 0.0, -4.0, -4.0]]);
    let spaced = evaluated(&species, "legend > label { margin: 12; }");
    assert_eq!(moved(&spaced, &plain), [[0.0, 0.0, -10.0, 0.0]]);

    // Symbols filled by mass, with a colorbar.
    let mass = Chart::builder(Plot::cartesian([CartesianLayer::symbol(
        penguins(),
        SymbolMark::builder().x("flipper").y("mass").fill("mass"),
    )]))
    .build()
    .unwrap();
    let plain = evaluated(&mass, "");
    let apart = evaluated(&mass, "legend > ramp { margin: 8; }");
    // Symbols sized by mass, with a symbol legend of the scale's values.
    let sized = Chart::builder(Plot::cartesian([CartesianLayer::symbol(
        penguins(),
        SymbolMark::builder().x("flipper").y("mass").size("mass"),
    )]))
    .build()
    .unwrap();
    let fewer = evaluated(&sized, "legend[type=symbol] { value-count: 2; }");
    assert!(text_count(&fewer) < text_count(&evaluated(&sized, "")));
    assert_eq!(moved(&apart, &plain), [[0.0, 0.0, -8.0, 0.0]]);

    // A wrap of a plot for each species, with a facet that labels them.
    let panels = Node::partitions(&penguins(), "species", PartitionOrder::Ascending, |rows| {
        let symbols = CartesianLayer::symbol(rows, SymbolMark::builder().x("flipper").y("mass"));
        Ok(Plot::cartesian([symbols]))
    })
    .unwrap();
    let facets = Chart::builder(Wrap::builder().facet("species").children(panels))
        .build()
        .unwrap();
    let plain = evaluated(&facets, "");
    // Labels 10 from the panels, 6 more than the base stylesheet's 4, lower the panels by 6.
    let spaced = evaluated(&facets, "facet[type=wrap] > label { margin: 10; }");
    assert_eq!(moved(&spaced, &plain), [[0.0, 6.0, 0.0, -6.0]; 2]);
    let turned = evaluated(&facets, "facet[type=wrap] > label { angle: 30; }");
    let count = |evaluated: &EvaluatedChart| {
        (text_angles(evaluated).iter())
            .filter(|angle| **angle == 30.0)
            .count()
    };
    assert_eq!((count(&plain), count(&turned)), (0, 2));

    // A rose chart, whose theta axis labels its categories around the rim.
    let rose = Chart::builder(Plot::polar([PolarLayer::bar(
        penguins(),
        PolarBarMark::builder()
            .theta(Encoding::scaled_field("species").axis(Axis::builder()))
            .radius("mass"),
    )]))
    .build()
    .unwrap();
    let plain = evaluated(&rose, "");
    let radial = evaluated(&rose, "axis > label { text-orient: radial; }");
    assert_eq!(text_angles(&plain), [0.0, 0.0]);
    assert_eq!(text_angles(&radial), [90.0, 90.0]);
    // On a polar axis, `auto` is the initial 0, the plot's start angle.
    let auto = evaluated(&rose, "axis { angle: auto; }");
    assert_eq!(format!("{:?}", auto.scene), format!("{:?}", plain.scene));
    let spaced = evaluated(&rose, "axis > label { margin: 20; }");
    assert_ne!(
        format!("{:?}", spaced.scene),
        format!("{:?}", plain.scene),
        "labels farther from their ticks"
    );
}

/// The marks of a scene, with the marks inside its groups, in drawing order.
fn all_marks(evaluated: &EvaluatedChart) -> Vec<&SceneMark> {
    fn walk<'a>(marks: &'a [SceneMark], out: &mut Vec<&'a SceneMark>) {
        for mark in marks {
            out.push(mark);
            if let SceneMark::Group(group) = mark {
                walk(&group.marks, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(&evaluated.scene.marks, &mut out);
    out
}

/// The text mark named `name` of a scene.
fn text_named<'a>(
    evaluated: &'a EvaluatedChart,
    name: &str,
) -> &'a avenger_scenegraph::marks::text::SceneTextMark {
    (all_marks(evaluated).into_iter())
        .find_map(|mark| match mark {
            SceneMark::Text(text) if text.name == name => Some(text),
            _ => None,
        })
        .unwrap_or_else(|| panic!("a text mark named `{name}`"))
}

/// The fill and the stroke of the rectangle mark named `name` of a scene.
fn rect_paints(evaluated: &EvaluatedChart, name: &str) -> Option<[[f32; 4]; 2]> {
    use avenger_color::ColorOrGradient;
    let color = |paint: Option<&ColorOrGradient>| match paint {
        Some(ColorOrGradient::Color(color)) => *color,
        other => panic!("a color: {other:?}"),
    };
    (all_marks(evaluated).into_iter()).find_map(|mark| match mark {
        SceneMark::Rect(rect) if rect.name == name => {
            Some([color(rect.fill.first()), color(rect.stroke.first())])
        }
        _ => None,
    })
}

/// A subtitle is a part of its title, so it takes the title's text styles unless it sets its own.
#[test]
fn a_subtitle_takes_the_text_styles_of_its_title() {
    let chart = |title: TitleBuilder| {
        let symbols =
            CartesianLayer::symbol(penguins(), SymbolMark::builder().x("flipper").y("mass"));
        Chart::builder(Plot::cartesian([symbols]).title(title))
            .build()
            .unwrap()
    };
    let host = HostInputs::sized(400.0, 300.0);
    let red = Title::builder()
        .text("Penguins")
        .subtitle("By species")
        .color("red");
    let mut red = evaluator(chart(red), cascade(""));
    let (title, subtitle) = {
        let tree = red.style_tree();
        let title = nodes_of(tree, Element::Title)[0];
        (title, part(tree, title, Element::Subtitle))
    };
    let styles = red.styles(&host);
    assert_eq!(
        styles[subtitle].color(Property::Color),
        Some([1.0, 0.0, 0.0, 1.0])
    );
    // The base stylesheet keeps the subtitle smaller than its bold title, and at a normal weight.
    assert_eq!(styles[title].font_size(), 13.0);
    assert_eq!(styles[subtitle].font_size(), 12.0);
    assert_eq!(
        styles[subtitle].keyword(Property::FontWeight),
        Some("normal")
    );

    // A subtitle with a color of its own keeps it.
    let blue = Title::builder()
        .text("Penguins")
        .subtitle(Subtitle::new("By species").color("blue"))
        .color("red");
    let mut blue = evaluator(chart(blue), cascade(""));
    let styles = blue.styles(&host);
    assert_eq!(
        styles[subtitle].color(Property::Color),
        Some([0.0, 0.0, 1.0, 1.0])
    );

    // A stylesheet's title styles reach the subtitle too.
    let plain = Title::builder().text("Penguins").subtitle("By species");
    let mut plain = evaluator(
        chart(plain),
        cascade("chart > title { color: green; font-style: italic; }"),
    );
    let styles = plain.styles(&host);
    let green = [0.0, 128.0 / 255.0, 0.0, 1.0];
    assert_eq!(styles[subtitle].color(Property::Color), Some(green));
    assert_eq!(
        styles[subtitle].keyword(Property::FontStyle),
        Some("italic")
    );
}

/// A geo plot draws its sphere from its `sphere` node, with the inline styles of `SphereStyle` over
/// the stylesheets, and its box from its own `background` and border, as a Cartesian plot does.
#[cfg(feature = "geo")]
#[test]
fn a_geo_plot_draws_its_sphere_and_its_box_from_their_nodes() {
    use avenger_color::ColorOrGradient;
    let cities = RecordBatch::try_from_iter([
        (
            "lon",
            Arc::new(Float64Array::from(vec![-74.0, 2.35, 139.7])) as ArrayRef,
        ),
        ("lat", Arc::new(Float64Array::from(vec![40.7, 48.9, 35.7]))),
    ])
    .unwrap();
    let places = GeoLayer::symbol(
        cities,
        GeoSymbolMark::builder().longitude("lon").latitude("lat"),
    );
    let options = Geo::new(Projection::builtin(ProjectionType::EqualEarth).whole_sphere())
        .sphere(SphereStyle::builder().fill("red"));
    let chart = Chart::builder(Plot::geo(GeoPlot::new(options, [places])))
        .build()
        .unwrap();
    {
        let evaluator = evaluator(chart.clone(), cascade(""));
        let tree = evaluator.style_tree();
        let plot = nodes_of(tree, Element::Plot)[0];
        assert_eq!(elements(tree, plot)[0], Element::Sphere);
    }
    // The sphere's fill, its stroke, and the width of its stroke.
    let sphere = |evaluated: &EvaluatedChart| {
        let color = |paint: Option<&ColorOrGradient>| match paint {
            Some(ColorOrGradient::Color(color)) => *color,
            other => panic!("a color: {other:?}"),
        };
        (all_marks(evaluated).into_iter()).find_map(|mark| match mark {
            SceneMark::Path(path) => Some((
                color(path.fill.first()),
                color(path.stroke.first()),
                *path.stroke_width.first().unwrap(),
            )),
            _ => None,
        })
    };
    let background = |evaluated: &EvaluatedChart| {
        let name = format!("{}/background", evaluated.plots[0].name);
        rect_paints(evaluated, &name).map(|[fill, _]| fill)
    };

    // The field's fill, and the base stylesheet's outline, with no box behind the plot.
    let plain = evaluated(&chart, "");
    let (fill, stroke, width) = sphere(&plain).expect("a sphere");
    assert_eq!(fill, [1.0, 0.0, 0.0, 1.0]);
    assert!(stroke[3] > 0.0 && width == 1.0, "{stroke:?} {width}");
    assert_eq!(background(&plain), None);

    // A stylesheet outlines the sphere, and the plot's own background fills its box.
    let styled = evaluated(
        &chart,
        "plot[type=geo] > sphere { fill: blue; stroke: green; stroke-width: 3; }
         plot[type=geo] { background: yellow; }",
    );
    let green = [0.0, 128.0 / 255.0, 0.0, 1.0];
    assert_eq!(sphere(&styled), Some(([1.0, 0.0, 0.0, 1.0], green, 3.0)));
    assert_eq!(background(&styled), Some([1.0, 1.0, 0.0, 1.0]));

    // A hidden sphere draws nothing.
    let hidden = evaluated(&chart, "sphere { display: none; }");
    assert_eq!(sphere(&hidden), None);
}

/// The title, the labels of the items, and the readout of a widget take the fonts of their nodes,
/// and its rail and its thumbs their paints: the inline fonts of the format over the stylesheets.
#[test]
fn the_parts_of_widgets_take_the_styles_of_their_nodes() {
    use avenger_common::types::{FontStyle, FontWeight};
    use avenger_widgets::WidgetStyle;
    let data = || WidgetDomain::data(WidgetDataDomain::builder());
    let slider = Widget::slider(
        SliderWidget::new("filter", "mass", data())
            .step(50.0)
            .readout(
                Labels::builder()
                    .format(",.0f")
                    .font_size(15.0)
                    .color("red"),
            ),
    )
    .key("mass")
    .title(Title::builder().text("Mass").font_weight(700.0));
    let radio = Widget::radio(
        RadioWidget::new("filter", "species", data()).labels(Labels::builder().font_size(14.0)),
    )
    .key("species");
    let symbols = CartesianLayer::symbol(penguins(), SymbolMark::builder().x("flipper").y("mass"))
        .widgets([slider, radio]);
    let chart = Chart::builder(Plot::cartesian([symbols]))
        .number_format_syntax("d3")
        .selections([Selection::new("filter", SelectionResolution::Intersect)])
        .build()
        .unwrap();
    {
        let evaluator = evaluator(chart.clone(), cascade(""));
        let tree = evaluator.style_tree();
        let widgets = nodes_of(tree, Element::Widget);
        assert_eq!(
            elements(tree, widgets[0]),
            [
                Element::Title,
                Element::Rail,
                Element::Thumb,
                Element::Readout
            ]
        );
        assert_eq!(elements(tree, widgets[1]), [Element::Title, Element::Label]);
    }

    let plain = evaluated(&chart, "");
    assert!(plain.warnings.is_empty(), "{:#?}", plain.warnings);
    let [slider, radio] = &plain.widgets[..] else {
        panic!("two widgets: {:#?}", plain.widgets);
    };
    let red = [1.0, 0.0, 0.0, 1.0];
    assert_eq!(slider.style.title.font_weight, FontWeight::BOLD);
    assert_eq!(
        (slider.style.readout.font_size, slider.style.readout.color),
        (15.0, red)
    );
    assert_eq!(radio.style.label.font_size, 14.0);
    // A part without styles of its own takes the widget's font, and the rail and the thumbs take
    // the widget style's own paints.
    assert_eq!(radio.style.title.font_size, 11.0);
    assert_eq!(radio.style.title.font_weight, FontWeight::NORMAL);
    assert_eq!(slider.style.rail, WidgetStyle::default().rail);
    assert_eq!(slider.style.thumb, WidgetStyle::default().thumb);
    // The drawn texts take the parts' fonts, and the readout its color.
    assert_eq!(
        text_named(&plain, "slider_label").font_weight.first(),
        Some(&FontWeight::BOLD)
    );
    let readout = text_named(&plain, "readout");
    assert_eq!(readout.font_size.first(), Some(&15.0));
    assert_eq!(readout.color.first(), Some(&red));
    assert_eq!(
        text_named(&plain, "radio_label").font_size.first(),
        Some(&14.0)
    );

    // A stylesheet paints the rail and the thumbs and styles the texts, below the inline fonts.
    let styled = evaluated(
        &chart,
        "widget > rail { fill: green; }
         widget > thumb { fill: white; stroke: black; }
         widget > readout { font-size: 20px; font-style: italic; }
         widget[type=radio] > title { color: blue; }",
    );
    let green = [0.0, 128.0 / 255.0, 0.0, 1.0];
    assert_eq!(rect_paints(&styled, "rail").unwrap()[0], green);
    assert_eq!(
        rect_paints(&styled, "thumb"),
        Some([[1.0; 4], [0.0, 0.0, 0.0, 1.0]])
    );
    let readout = text_named(&styled, "readout");
    assert_eq!(readout.font_size.first(), Some(&15.0));
    assert_eq!(readout.font_style.first(), Some(&FontStyle::Italic));
    assert_eq!(
        text_named(&styled, "radio_group_label").color.first(),
        Some(&[0.0, 0.0, 1.0, 1.0])
    );
}

/// Each kind of widget has the nodes of the parts that it draws, and a range slider's readout and a
/// check box list's labels take their inline fonts.
#[test]
fn every_kind_of_widget_has_the_nodes_of_its_parts() {
    let data = || WidgetDomain::data(WidgetDataDomain::builder());
    let widgets = [
        Widget::range_slider(
            RangeSliderWidget::new("filter", "mass", data())
                .readout(Labels::builder().format(",.0f").font_size(15.0)),
        )
        .key("mass"),
        Widget::checkboxes(
            CheckboxesWidget::new("filter", "species", data())
                .labels(Labels::builder().font_size(14.0)),
        )
        .key("species"),
        Widget::checkbox(CheckboxWidget::new("filter", "species", "Adelie")).key("adelie"),
        Widget::mode(ModeWidget::group("drag")).key("drag"),
        Widget::mode(ModeWidget::toggle("tooltips")).key("tooltips"),
    ];
    let symbols = CartesianLayer::symbol(penguins(), SymbolMark::builder().x("flipper").y("mass"))
        .tools([LayerToolBuilder::from(
            Tooltip::builder().modes(["tooltips"]),
        )])
        .widgets(widgets);
    let tools = [
        ToolBuilder::from(Navigate::builder().modes(["pan"])),
        ToolBuilder::from(Select::interval("filter", IntervalSelect::builder()).modes(["box"])),
    ];
    let chart = Chart::builder(Plot::cartesian([symbols]).id("penguins").tools(tools))
        .number_format_syntax("d3")
        .selections([Selection::new("filter", SelectionResolution::Intersect)])
        .modes([
            Mode::new("pan").label("Pan").group("drag"),
            Mode::new("box").label("Box").group("drag"),
            Mode::new("tooltips").label("Tooltips").on(true),
        ])
        .build()
        .unwrap();
    {
        let evaluator = evaluator(chart.clone(), cascade(""));
        let tree = evaluator.style_tree();
        let parts: Vec<Vec<Element>> = (nodes_of(tree, Element::Widget).into_iter())
            .map(|widget| elements(tree, widget))
            .collect();
        use Element::{Label, Rail, Readout, Thumb, Title};
        assert_eq!(
            parts,
            [
                vec![Title, Rail, Thumb, Readout],
                vec![Title, Label],
                vec![Title],
                vec![Title, Label],
                vec![Title],
            ]
        );
    }
    let evaluated = evaluated(&chart, "");
    assert!(evaluated.warnings.is_empty(), "{:#?}", evaluated.warnings);
    let styles: Vec<_> = evaluated
        .widgets
        .iter()
        .map(|widget| &widget.style)
        .collect();
    assert_eq!(styles[0].readout.font_size, 15.0);
    assert_eq!(styles[1].label.font_size, 14.0);
    assert_eq!(
        text_named(&evaluated, "readout").font_size.first(),
        Some(&15.0)
    );
}

/// The scene marks of the layer at `layer` of the first plot of a scene, whose names are the
/// plot's name and the layer's position, with a series after them.
fn layer_marks(evaluated: &EvaluatedChart, layer: usize) -> Vec<&SceneMark> {
    let name = format!("{}/{layer}", evaluated.plots[0].name);
    let of_layer = |mark: &str| mark == name || mark.starts_with(&format!("{name}/"));
    (all_marks(evaluated).into_iter())
        .filter(|mark| match mark {
            SceneMark::Line(line) => of_layer(&line.name),
            SceneMark::Area(area) => of_layer(&area.name),
            SceneMark::Rule(rule) => of_layer(&rule.name),
            SceneMark::Path(path) => of_layer(&path.name),
            SceneMark::Text(text) => of_layer(&text.name),
            SceneMark::Symbol(symbol) => of_layer(&symbol.name),
            SceneMark::Arc(arc) => of_layer(&arc.name),
            _ => false,
        })
        .collect()
}

/// The caps and the joins of the line, area, rule, and path marks of a scene, in drawing order. A
/// rule has no joins.
fn caps_and_joins(
    evaluated: &EvaluatedChart,
    layer: usize,
) -> Vec<(StrokeCap, Option<StrokeJoin>)> {
    (layer_marks(evaluated, layer).into_iter())
        .filter_map(|mark| match mark {
            SceneMark::Line(line) => Some((line.stroke_cap, Some(line.stroke_join))),
            SceneMark::Area(area) => Some((area.stroke_cap, Some(area.stroke_join))),
            SceneMark::Rule(rule) => Some((*rule.stroke_cap.first().unwrap(), None)),
            SceneMark::Path(path) => Some((path.stroke_cap, Some(path.stroke_join))),
            _ => None,
        })
        .collect()
}

/// The text mark of the layer `layer` of a scene.
fn layer_text(
    evaluated: &EvaluatedChart,
    layer: usize,
) -> &avenger_scenegraph::marks::text::SceneTextMark {
    (layer_marks(evaluated, layer).into_iter())
        .find_map(|mark| match mark {
            SceneMark::Text(text) => Some(text),
            _ => None,
        })
        .unwrap_or_else(|| panic!("a text mark of layer {layer}"))
}

/// The horizontal positions of the symbols of the layer `layer` of a scene.
fn symbol_xs(evaluated: &EvaluatedChart, layer: usize) -> Vec<f32> {
    (layer_marks(evaluated, layer).into_iter())
        .find_map(|mark| match mark {
            SceneMark::Symbol(symbol) => Some(symbol.x.as_vec(symbol.len as usize, None)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("a symbol mark of layer {layer}"))
}

/// The caps and the joins of lines, areas, rules, and paths, the line height and the most width of
/// texts, and the band position of marks take the computed styles of the marks, whose initial
/// values draw as the scene graph's defaults do.
#[test]
fn the_styles_of_marks_reach_what_they_draw() {
    use StrokeCap::{Butt, Round, Square};
    use StrokeJoin::{Bevel, Miter};
    let lines = CartesianLayer::line(
        penguins(),
        LineMark::builder().x("flipper").y("mass").color("species"),
    );
    let areas = CartesianLayer::area(penguins(), AreaMark::builder().x("flipper").y("mass"));
    let rules = CartesianLayer::rule(penguins(), RuleMark::builder().x("flipper"));
    let paths = CartesianLayer::path(
        penguins(),
        PathMark::new(Encoding::direct_value("M0,0 L5,5 L10,0"))
            .x("flipper")
            .y("mass"),
    );
    let chart = Chart::builder(Plot::cartesian([lines, areas, rules, paths]))
        .build()
        .unwrap();
    let plain = evaluated(&chart, "");
    let initial = (Butt, Some(Miter));
    assert_eq!(caps_and_joins(&plain, 0), [initial; 2]);
    assert_eq!(caps_and_joins(&plain, 1), [initial]);
    assert_eq!(caps_and_joins(&plain, 2), [(Butt, None)]);
    assert_eq!(caps_and_joins(&plain, 3), [initial]);
    let styled = evaluated(
        &chart,
        "mark { stroke-linecap: round; stroke-linejoin: bevel; }
         mark[type=rule] { stroke-linecap: square; }",
    );
    assert_eq!(caps_and_joins(&styled, 0), [(Round, Some(Bevel)); 2]);
    assert_eq!(caps_and_joins(&styled, 1), [(Round, Some(Bevel))]);
    assert_eq!(caps_and_joins(&styled, 2), [(Square, None)]);
    assert_eq!(caps_and_joins(&styled, 3), [(Round, Some(Bevel))]);

    // Texts of the species, without a `width` channel, at the base stylesheet's 11 pixels.
    let texts = CartesianLayer::text(
        penguins(),
        TextMark::new(Encoding::direct_field("species"))
            .x("flipper")
            .y("mass"),
    );
    let chart = Chart::builder(Plot::cartesian([texts])).build().unwrap();
    let plain = evaluated(&chart, "");
    let text = layer_text(&plain, 0);
    assert_eq!(text.line_height.first(), Some(&LabelLineHeight::Auto));
    assert_eq!(text.width.first(), Some(&LabelWidth::Auto));
    assert!(!text.wrap);
    // A number multiplies the font size, and a length is the distance between baselines.
    let tall = evaluated(&chart, "mark[type=text] { line-height: 2; }");
    let text = layer_text(&tall, 0);
    assert_eq!(
        text.line_height.first(),
        Some(&LabelLineHeight::Fixed(22.0))
    );
    let fixed = evaluated(&chart, "chart { line-height: 15px; }");
    let text = layer_text(&fixed, 0);
    assert_eq!(
        text.line_height.first(),
        Some(&LabelLineHeight::Fixed(15.0))
    );
    // `max-width` is the most width of each text, which wraps it unless it does not wrap.
    let narrow = evaluated(&chart, "mark[type=text] { max-width: 40; }");
    let text = layer_text(&narrow, 0);
    assert_eq!(text.width.first(), Some(&LabelWidth::Max(40.0)));
    assert!(text.wrap);
    let unwrapped = evaluated(
        &chart,
        "mark[type=text] { max-width: 40; white-space: nowrap; }",
    );
    assert!(!layer_text(&unwrapped, 0).wrap);

    // Symbols in the bands of a band scale stand at the mark's band position, unless the
    // encoding gives its own.
    let banded = |x: EncodingBuilder| {
        let symbols = CartesianLayer::symbol(penguins(), SymbolMark::builder().x(x).y("mass"));
        Chart::builder(Plot::cartesian([symbols])).build().unwrap()
    };
    let band = || Encoding::scaled_field("species").scale(Scale::band(BandScale::builder()));
    let chart = banded(band());
    let middle = symbol_xs(&evaluated(&chart, ""), 0);
    let start = symbol_xs(&evaluated(&chart, "mark { band-position: 0; }"), 0);
    let end = symbol_xs(&evaluated(&chart, "mark { band-position: 1; }"), 0);
    for ((start, middle), end) in start.iter().zip(&middle).zip(&end) {
        assert!(start < middle && middle < end, "{start} {middle} {end}");
        assert!(((start + end) / 2.0 - middle).abs() < 1e-3);
    }
    let given = banded(band().band_position(1.0));
    assert_eq!(
        symbol_xs(&evaluated(&given, "mark { band-position: 0; }"), 0),
        end
    );
    // An axis takes no band position.
    let axis = evaluated(&chart, "axis { band-position: 0; }");
    assert_eq!(
        format!("{:?}", axis.scene),
        format!("{:?}", evaluated(&chart, "").scene)
    );
}

/// The lines, the rules, the outlines of areas, and the polygons of bars of a polar plot take the
/// caps and the joins of their marks.
#[test]
fn the_lines_of_polar_plots_take_the_caps_and_joins_of_their_marks() {
    use StrokeCap::{Butt, Round};
    use StrokeJoin::{Bevel, Miter};
    let lines = PolarLayer::line(
        penguins(),
        PolarLineMark::builder().theta("flipper").radius("mass"),
    );
    let rules = PolarLayer::rule(
        penguins(),
        PolarRuleMark::builder()
            .theta(0.5)
            .radius(Encoding::direct_value(0.0))
            .radius2(Encoding::direct_value(1.0)),
    );
    let areas = PolarLayer::area(
        penguins(),
        PolarAreaMark::builder().theta("flipper").radius("mass"),
    );
    let bars = PolarLayer::bar(
        penguins(),
        PolarBarMark::builder()
            .theta(Encoding::scaled_field("species"))
            .radius("mass")
            .route(Route::Canvas),
    );
    let lined = Chart::builder(Plot::polar([lines, rules, areas]))
        .build()
        .unwrap();
    let barred = Chart::builder(Plot::polar([bars])).build().unwrap();
    let outlined = "mark[type=area] { stroke: red; stroke-width: 1; }";
    let styled = format!("{outlined} mark {{ stroke-linecap: round; stroke-linejoin: bevel; }}");
    // The fill of an area draws no stroke, so only its outline, a line, takes them.
    for (chart, layer, fills) in [
        (&lined, 0, 0),
        (&lined, 1, 0),
        (&lined, 2, 1),
        (&barred, 0, 0),
    ] {
        let plain = caps_and_joins(&evaluated(chart, outlined), layer);
        let styled = caps_and_joins(&evaluated(chart, &styled), layer);
        assert!(plain.len() > fills, "layer {layer}");
        assert!(
            plain.iter().all(|mark| *mark == (Butt, Some(Miter))),
            "{plain:?}"
        );
        assert!(
            styled[fills..]
                .iter()
                .all(|mark| *mark == (Round, Some(Bevel))),
            "layer {layer}: {styled:?}"
        );
    }
}

/// The `pad-angle` of a polar bar pads its arc, and with the canvas route its polygon.
#[test]
fn the_pad_angle_of_polar_bars_pads_their_sectors() {
    let rose = |route: Route| {
        let bars = PolarLayer::bar(
            penguins(),
            PolarBarMark::builder()
                .theta(Encoding::scaled_field("species"))
                .radius("mass")
                .route(route),
        );
        Chart::builder(Plot::polar([bars])).build().unwrap()
    };
    let pads = |evaluated: &EvaluatedChart| match layer_marks(evaluated, 0)[..] {
        [SceneMark::Arc(arc)] => arc.pad_angle.as_vec(arc.len as usize, None),
        ref marks => panic!("one arc mark: {marks:?}"),
    };
    let arcs = rose(Route::Coordinate);
    assert!(pads(&evaluated(&arcs, "")).iter().all(|pad| *pad == 0.0));
    let padded = evaluated(&arcs, "mark[type=bar] { pad-angle: 2deg; }");
    for pad in pads(&padded) {
        assert!((pad - 2f32.to_radians()).abs() < 1e-6, "{pad}");
    }
    let polygons = rose(Route::Canvas);
    let path = |css: &str| {
        let evaluated = evaluated(&polygons, css);
        match layer_marks(&evaluated, 0)[..] {
            [SceneMark::Path(path)] => format!("{:?}", path.path),
            ref marks => panic!("one path mark: {marks:?}"),
        }
    };
    assert_ne!(path(""), path("mark[type=bar] { pad-angle: 4deg; }"));
}

/// The space between an axis's labels and its title, and between a facet's labels and its title,
/// is the computed `margin` of the title: by the base stylesheet, 2 on a continuous axis, 4 on a
/// discrete one, and 5 on a facet.
#[test]
fn the_margin_of_a_title_spaces_it_from_its_guide() {
    let symbols = |x: &str| {
        let symbols = CartesianLayer::symbol(penguins(), SymbolMark::builder().x(x).y("mass"));
        Chart::builder(Plot::cartesian([symbols])).build().unwrap()
    };
    let margins = |chart: &Chart| {
        let mut evaluator = evaluator(chart.clone(), cascade(""));
        let tree = evaluator.style_tree();
        let titles: Vec<NodeId> = (nodes_of(tree, Element::Axis).into_iter())
            .map(|axis| part(tree, axis, Element::Title))
            .collect();
        let styles = evaluator.styles(&HostInputs::sized(400.0, 300.0));
        (titles.into_iter())
            .map(|title| styles[title].number(Property::Margin).unwrap())
            .collect::<Vec<_>>()
    };
    let continuous = symbols("flipper");
    let discrete = symbols("species");
    assert_eq!(margins(&continuous), [2.0, 2.0]);
    assert_eq!(margins(&discrete), [4.0, 2.0]);
    // A wider margin of the x axis's title lowers the x axis and shortens the plot.
    for chart in [&continuous, &discrete] {
        let plain = evaluated(chart, "");
        let spaced = evaluated(chart, "axis[channel=x] > title { margin: 14; }");
        let [[_, _, _, shorter]] = moved(&spaced, &plain)[..] else {
            panic!("one plot");
        };
        assert!(shorter < -9.0, "{shorter}");
    }

    let panels = Node::partitions(&penguins(), "species", PartitionOrder::Ascending, |rows| {
        let symbols = CartesianLayer::symbol(rows, SymbolMark::builder().x("flipper").y("mass"));
        Ok(Plot::cartesian([symbols]))
    })
    .unwrap();
    let facets = Chart::builder(Wrap::builder().facet("species").children(panels))
        .build()
        .unwrap();
    let plain = evaluated(&facets, "");
    // A title 15 from the labels, 10 more than the base stylesheet's 5, lowers the panels by 10.
    let spaced = evaluated(&facets, "facet > title { margin: 15; }");
    assert_eq!(moved(&spaced, &plain), [[0.0, 10.0, 0.0, -10.0]; 2]);
}

/// A plot keeps its computed `aspect-ratio`: a ratio, any shape with `none`, and with `auto` its
/// natural aspect, none for a Cartesian plot. A polar plot keeps the shape of its sector.
#[test]
fn a_plot_keeps_its_aspect_ratio() {
    let shape = |evaluated: &EvaluatedChart| {
        let [_, _, width, height] = evaluated.plots[0].rect;
        width / height
    };
    let symbols = CartesianLayer::symbol(penguins(), SymbolMark::builder().x("flipper").y("mass"));
    let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();
    let plain = evaluated(&chart, "");
    assert_eq!(
        format!(
            "{:?}",
            evaluated(&chart, "plot { aspect-ratio: none; }").scene
        ),
        format!("{:?}", plain.scene)
    );
    let square = evaluated(&chart, "plot { aspect-ratio: 1; }");
    assert!((shape(&square) - 1.0).abs() < 1e-3, "{}", shape(&square));

    let rose = Chart::builder(Plot::polar([PolarLayer::bar(
        penguins(),
        PolarBarMark::builder()
            .theta(Encoding::scaled_field("species"))
            .radius("mass"),
    )]))
    .build()
    .unwrap();
    let plain = evaluated(&rose, "");
    for css in ["plot { aspect-ratio: 3; }", "plot { aspect-ratio: none; }"] {
        assert_eq!(
            format!("{:?}", evaluated(&rose, css).scene),
            format!("{:?}", plain.scene),
            "{css}"
        );
    }
}

/// A geo plot keeps the aspect of what its projection fits, unless its computed `aspect-ratio`,
/// which `Geo.aspect` sets inline, is a ratio or `none`.
#[cfg(feature = "geo")]
#[test]
fn a_geo_plot_keeps_its_aspect_ratio() {
    let cities = RecordBatch::try_from_iter([
        (
            "lon",
            Arc::new(Float64Array::from(vec![-74.0, 2.35, 139.7])) as ArrayRef,
        ),
        ("lat", Arc::new(Float64Array::from(vec![40.7, 48.9, 35.7]))),
    ])
    .unwrap();
    let chart = |options: GeoBuilder| {
        let places = GeoLayer::symbol(
            cities.clone(),
            GeoSymbolMark::builder().longitude("lon").latitude("lat"),
        );
        Chart::builder(Plot::geo(GeoPlot::new(options, [places])))
            .build()
            .unwrap()
    };
    let world = || Projection::builtin(ProjectionType::EqualEarth).whole_sphere();
    let at = |chart: &Chart, css: &str| {
        let evaluated = evaluator(chart.clone(), cascade(css))
            .evaluate(&HostInputs::sized(600.0, 600.0))
            .unwrap();
        let [_, _, width, height] = evaluated.plots[0].rect;
        [width, height]
    };
    let kept = chart(Geo::new(world()));
    let [width, height] = at(&kept, "");
    assert!(width / height > 1.5, "{width} {height}");
    assert_eq!(at(&kept, "plot { aspect-ratio: auto; }"), [width, height]);
    let [width, height] = at(&kept, "plot[type=geo] { aspect-ratio: 1; }");
    assert!((width - height).abs() < 1e-3, "{width} {height}");
    // `none` fills the space, as `Geo.free` does.
    let free = chart(Geo::free(world()));
    let filled = at(&free, "");
    assert!((filled[0] - filled[1]).abs() < 1.0, "{filled:?}");
    assert_eq!(at(&kept, "plot[type=geo] { aspect-ratio: none; }"), filled);
    // `Geo.aspect` is an inline style, which wins over the stylesheets.
    assert_eq!(at(&free, "plot[type=geo] { aspect-ratio: 3; }"), filled);
    let [width, height] = at(
        &chart(Geo::aspect_ratio(world(), 3.0)),
        "plot { aspect-ratio: 1; }",
    );
    assert!((width / height - 3.0).abs() < 1e-3, "{width} {height}");
}

/// The outlines of a geo plot's shapes are round at their ends and corners, and its lines take the
/// initial butt caps and miter joins, unless the computed `stroke-linecap` and `stroke-linejoin`
/// of their marks say otherwise.
#[cfg(feature = "geo")]
#[test]
fn the_caps_and_joins_of_shapes_take_their_mark_styles() {
    use arrow::datatypes::Schema;
    use geo_types::{Geometry, LineString, Polygon};
    use geoarrow_array::{builder::WkbBuilder, GeoArrowArray};
    use geoarrow_schema::WkbType;
    let mut builder = WkbBuilder::<i32>::new(WkbType::default());
    let ring = LineString::from(vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 0.0)]);
    builder
        .push_geometry(Some(&Geometry::Polygon(Polygon::new(ring, Vec::new()))))
        .unwrap();
    let geometry = builder.finish();
    let field = geometry.data_type().to_field("geometry", true);
    let table = RecordBatch::try_new(
        Arc::new(Schema::new(vec![field])),
        vec![geometry.into_array_ref()],
    )
    .unwrap();
    let shapes = GeoLayer::shape(table, GeoShapeMark::new("geometry"));
    let places = RecordBatch::try_from_iter([
        (
            "lon",
            Arc::new(Float64Array::from(vec![-74.0, 2.35, 139.7])) as ArrayRef,
        ),
        ("lat", Arc::new(Float64Array::from(vec![40.7, 48.9, 35.7]))),
    ])
    .unwrap();
    let lines = GeoLayer::line(
        places,
        GeoLineMark::builder().longitude("lon").latitude("lat"),
    );
    let options = Geo::new(Projection::builtin(ProjectionType::EqualEarth));
    let chart = Chart::builder(Plot::geo(GeoPlot::new(options, [shapes, lines])))
        .build()
        .unwrap();
    let plain = evaluated(&chart, "");
    assert_eq!(
        caps_and_joins(&plain, 0),
        [(StrokeCap::Round, Some(StrokeJoin::Round))]
    );
    assert_eq!(
        caps_and_joins(&plain, 1),
        [(StrokeCap::Butt, Some(StrokeJoin::Miter))]
    );
    let styled = evaluated(
        &chart,
        "mark[type=shape] { stroke-linecap: square; stroke-linejoin: miter; }
         mark[type=line] { stroke-linecap: round; stroke-linejoin: bevel; }",
    );
    assert_eq!(
        caps_and_joins(&styled, 0),
        [(StrokeCap::Square, Some(StrokeJoin::Miter))]
    );
    assert_eq!(
        caps_and_joins(&styled, 1),
        [(StrokeCap::Round, Some(StrokeJoin::Bevel))]
    );
}
