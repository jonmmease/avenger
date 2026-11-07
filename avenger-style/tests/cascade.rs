//! The cascade: origins, layers, importance, specificity, inheritance, units, conditions,
//! substitution, the standard stylesheets, and palettes.

use std::sync::Arc;

use avenger_style::{
    table, Cascade, ColorScheme, Computed, ComputedStyles, Element, Env, HostInputs, HostTheme,
    Loaded, NodeId, Paint, PaletteKind, ParseOptions, Preferences, Property, ScaleKind, SourceId,
    StyleTable, StyleTree, StyleWarning, Stylesheet, Value,
};

/// The nodes of a small chart: a Cartesian plot with bars and a line, an x axis, and a color
/// scale.
struct Nodes {
    chart: NodeId,
    plot: NodeId,
    bar: NodeId,
    bar_label: NodeId,
    line: NodeId,
    axis: NodeId,
    axis_label: NodeId,
    tick: NodeId,
    rule: NodeId,
    grid: NodeId,
    scale: NodeId,
}

fn chart() -> (StyleTree, Nodes) {
    let mut tree = StyleTree::new(Element::Chart);
    let chart = tree.root();
    let plot = tree.add(chart, Element::Plot);
    tree.set_attribute(plot, "type", "cartesian");
    let bar = tree.add(plot, Element::Mark);
    tree.set_attribute_forms(bar, "type", "bar", "bar");
    tree.add_class(bar, "highlight");
    let bar_label = tree.add(bar, Element::Label);
    let line = tree.add(plot, Element::Mark);
    tree.set_attribute_forms(line, "type", "line", "line");
    let axis = tree.add(plot, Element::Axis);
    tree.set_attribute(axis, "channel", "x");
    tree.set_attribute(axis, "scale", "linear");
    tree.set_attribute(axis, "kind", "sequential");
    let _title = tree.add(axis, Element::Title);
    let axis_label = tree.add(axis, Element::Label);
    let tick = tree.add(axis, Element::Tick);
    let rule = tree.add(axis, Element::Rule);
    let grid = tree.add(axis, Element::Grid);
    let scale = tree.add(plot, Element::Scale);
    tree.set_attribute(scale, "channel", "color");
    tree.set_attribute(scale, "type", "ordinal");
    tree.set_attribute(scale, "kind", "categorical");
    tree.set_attribute_forms(scale, "marks", "bar line", "bar line");
    (
        tree,
        Nodes {
            chart,
            plot,
            bar,
            bar_label,
            line,
            axis,
            axis_label,
            tick,
            rule,
            grid,
            scale,
        },
    )
}

fn sheet(css: &str) -> Stylesheet {
    let (sheet, diagnostics) = Stylesheet::parse(css, SourceId(1), ParseOptions::default());
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    sheet
}

fn env(pairs: &[(&str, &str)]) -> Env {
    (pairs.iter())
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
}

fn styles(table: &StyleTable, tree: &StyleTree, host: &HostInputs) -> ComputedStyles {
    Cascade::new(table).compute(tree, host)
}

/// A table whose first stylesheet imports the base stylesheet, with the arguments `env`.
fn base_table(env: Env) -> StyleTable {
    let mut no_files = |url: &str, _: Option<&str>| Err(format!("no file `{url}`"));
    let options = ParseOptions {
        allow_import: true,
        ..Default::default()
    };
    let (sheet, diagnostics) = avenger_style::Sheet::load(
        "@import 'std:base.css';",
        None,
        SourceId(3),
        options,
        &mut no_files,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    StyleTable::new().with_author(sheet, env)
}

/// A table of the base stylesheet and the author stylesheet `css`.
fn with(css: &str) -> StyleTable {
    base_table(Env::new()).with_author(sheet(css), Env::new())
}

fn rgb8(color: [f32; 4]) -> [u8; 3] {
    [0, 1, 2].map(|channel| (color[channel] * 255.0).round() as u8)
}

fn hex(color: Option<[f32; 4]>) -> String {
    let [r, g, b] = rgb8(color.expect("a color"));
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn paint_hex(paint: Paint) -> String {
    match paint {
        Paint::Color(color) => hex(Some(color)),
        other => panic!("a color paint, got {other:?}"),
    }
}

#[test]
fn origins_inline_styles_and_importance_rank_as_css_does() {
    let (mut tree, nodes) = chart();
    let host = HostInputs::default();
    // An author rule wins over the base stylesheet.
    let computed = styles(&with("plot > mark { color: red; }"), &tree, &host);
    assert_eq!(hex(computed[nodes.bar].color(Property::Color)), "#ff0000");

    // An inline style wins over a normal author declaration, and an important one wins over it.
    tree.set_inline(
        nodes.bar,
        Property::Color,
        Property::Color.parse_value("blue").unwrap(),
    );
    let computed = styles(&with("plot > mark { color: red; }"), &tree, &host);
    assert_eq!(hex(computed[nodes.bar].color(Property::Color)), "#0000ff");
    let computed = styles(
        &with("plot > mark { color: red !important; }"),
        &tree,
        &host,
    );
    assert_eq!(hex(computed[nodes.bar].color(Property::Color)), "#ff0000");

    // The host theme ranks between the base stylesheet and the author's normal declarations.
    let (tree, nodes) = chart();
    let theme = Arc::new(HostTheme {
        sheet: sheet("chart { background: green; color: purple; }").into(),
        env: Env::new(),
    });
    let host = HostInputs {
        theme: Some(theme),
        ..Default::default()
    };
    let computed = styles(&with("chart { color: red; }"), &tree, &host);
    assert_eq!(
        hex(computed[nodes.chart].color(Property::Background)),
        "#008000"
    );
    assert_eq!(hex(computed[nodes.chart].color(Property::Color)), "#ff0000");
}

#[test]
fn forced_colors_win_except_where_forced_color_adjust_is_none() {
    let (tree, nodes) = chart();
    let host = HostInputs {
        preferences: Preferences {
            forced_colors: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let computed = styles(&with("chart { color: red !important; }"), &tree, &host);
    // `CanvasText` is black in the light scheme.
    assert_eq!(hex(computed[nodes.chart].color(Property::Color)), "#000000");
    let computed = styles(
        &with("chart { color: red !important; forced-color-adjust: none; }"),
        &tree,
        &host,
    );
    assert_eq!(hex(computed[nodes.chart].color(Property::Color)), "#ff0000");
}

#[test]
fn layers_order_declarations_and_reverse_for_importance() {
    let (tree, nodes) = chart();
    let host = HostInputs::default();
    let layered = "@layer a, b; @layer b { mark { fill: red; } } @layer a { mark { fill: blue; } }";
    let computed = styles(&with(layered), &tree, &host);
    assert_eq!(
        paint_hex(computed[nodes.bar].paint(Property::Fill)),
        "#ff0000"
    );
    let computed = styles(
        &with(&format!("{layered} mark {{ fill: green; }}")),
        &tree,
        &host,
    );
    assert_eq!(
        paint_hex(computed[nodes.bar].paint(Property::Fill)),
        "#008000"
    );
    let important = "@layer a, b; @layer a { mark { fill: blue !important; } } @layer b { mark { fill: red !important; } }";
    let computed = styles(&with(important), &tree, &host);
    assert_eq!(
        paint_hex(computed[nodes.bar].paint(Property::Fill)),
        "#0000ff"
    );
    // A layer's own declarations come after its sublayers.
    let nested = "@layer a { mark { fill: blue; } @layer inner { mark { fill: red; } } }";
    let computed = styles(&with(nested), &tree, &host);
    assert_eq!(
        paint_hex(computed[nodes.bar].paint(Property::Fill)),
        "#0000ff"
    );
}

#[test]
fn the_reset_removes_the_base_stylesheet_and_keeps_later_rules() {
    let (tree, nodes) = chart();
    let host = HostInputs::default();
    let base = styles(&base_table(Env::new()), &tree, &host);
    assert_eq!(
        base[nodes.chart].font_size(),
        11.0,
        "the base gives charts 1.1rem"
    );
    let reset = base_table(Env::new())
        .with_author(table::reset().clone(), Env::new())
        .with_author(sheet("mark { stroke-width: 3; }"), Env::new());
    let computed = styles(&reset, &tree, &host);
    assert_eq!(
        computed[nodes.chart].font_size(),
        10.0,
        "the initial value is 1rem"
    );
    assert_eq!(
        computed[nodes.line].number(Property::StrokeWidth),
        Some(3.0)
    );
}

#[test]
fn specificity_then_order_decide_within_a_layer() {
    let (tree, nodes) = chart();
    let host = HostInputs::default();
    let computed = styles(
        &with("mark[type=bar] { fill: blue; } mark { fill: red; } .highlight { stroke: navy; } mark { stroke: teal; }"),
        &tree,
        &host,
    );
    assert_eq!(
        paint_hex(computed[nodes.bar].paint(Property::Fill)),
        "#0000ff"
    );
    assert_eq!(
        paint_hex(computed[nodes.bar].paint(Property::Stroke)),
        "#000080"
    );
    let computed = styles(
        &with("mark { fill: red; } mark { fill: blue; }"),
        &tree,
        &host,
    );
    assert_eq!(
        paint_hex(computed[nodes.bar].paint(Property::Fill)),
        "#0000ff"
    );
}

#[test]
fn inherited_properties_pass_through_nodes_that_do_not_accept_them() {
    let (tree, nodes) = chart();
    let host = HostInputs::default();
    let computed = styles(
        &with("chart { font-family: Georgia; } axis { stroke: red; offset: 7; }"),
        &tree,
        &host,
    );
    let family = computed[nodes.axis_label].get(Property::FontFamily);
    assert_eq!(
        family,
        &Computed::List(vec![Computed::String("Georgia".into())].into())
    );
    // An axis does not accept `stroke`, so its tick keeps the base color.
    assert_eq!(
        paint_hex(computed[nodes.tick].paint(Property::Stroke)),
        "#888888"
    );
    assert_eq!(computed[nodes.axis].number(Property::Offset), Some(7.0));
}

#[test]
fn lengths_resolve_em_rem_and_the_text_scale() {
    let (tree, nodes) = chart();
    let computed = styles(
        &with("chart { font-size: 20px; } axis > label { font-size: 0.5em; } axis { offset: calc(1rem + 2em); }"),
        &tree,
        &HostInputs::default(),
    );
    assert_eq!(computed[nodes.axis_label].font_size(), 10.0);
    // `em` reads the node's own font size, which the axis inherits from the chart.
    assert_eq!(computed[nodes.axis].number(Property::Offset), Some(50.0));
    let scaled = HostInputs {
        preferences: Preferences {
            text_scale: 2.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let computed = styles(&base_table(Env::new()), &tree, &scaled);
    assert_eq!(computed[nodes.axis_label].font_size(), 20.0);
}

#[test]
fn conditions_read_the_host_box_and_the_preferences() {
    let (tree, nodes) = chart();
    let table = with(
        "@container (width < 400px) { mark { stroke-width: 5; } }
         @media (prefers-color-scheme: dark) { chart { color: white; } }",
    );
    let narrow = styles(&table, &tree, &HostInputs::sized(300.0, 200.0));
    assert_eq!(narrow[nodes.line].number(Property::StrokeWidth), Some(5.0));
    let wide = styles(&table, &tree, &HostInputs::sized(500.0, 200.0));
    assert_eq!(wide[nodes.line].number(Property::StrokeWidth), Some(2.0));
    let no_size = styles(&table, &tree, &HostInputs::default());
    assert_eq!(no_size[nodes.line].number(Property::StrokeWidth), Some(2.0));
    assert!(!narrow.needs_update(&HostInputs::sized(350.0, 250.0)));
    assert!(narrow.needs_update(&HostInputs::sized(500.0, 250.0)));

    let dark = HostInputs {
        preferences: Preferences {
            color_scheme: ColorScheme::Dark,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(narrow.needs_update(&dark));
    let computed = styles(&table, &tree, &dark);
    assert_eq!(hex(computed[nodes.chart].color(Property::Color)), "#ffffff");
}

#[test]
fn a_resize_keeps_the_styles_within_a_breakpoint() {
    let (tree, _) = chart();
    let table = with("@container (width < 400px) { axis { label-angle: 45; } }");
    let wide = styles(&table, &tree, &HostInputs::sized(800.0, 600.0));
    // No value reads the size of the host box, so only crossing the breakpoint needs new styles.
    assert!(!wide.needs_update(&HostInputs::sized(500.0, 300.0)));
    assert!(wide.needs_update(&HostInputs::sized(300.0, 300.0)));
}

#[test]
fn var_and_env_substitute_with_fallbacks() {
    let (tree, nodes) = chart();
    let host = HostInputs::default();
    let computed = styles(
        &with("chart { --brand: teal; } mark[type=bar] { fill: var(--brand); } mark[type=line] { stroke: var(--missing, orange); }"),
        &tree,
        &host,
    );
    assert_eq!(
        paint_hex(computed[nodes.bar].paint(Property::Fill)),
        "#008080"
    );
    assert_eq!(
        paint_hex(computed[nodes.line].paint(Property::Stroke)),
        "#ffa500"
    );

    let computed = styles(
        &with("chart { --a: var(--b); --b: var(--a); } mark[type=bar] { fill: var(--a); }"),
        &tree,
        &host,
    );
    assert!(computed.warnings().iter().any(
        |warning| matches!(warning, StyleWarning::Property { property, .. } if property == "fill")
    ));

    let table = base_table(Env::new()).with_author(
        sheet("mark[type=bar] { fill: env(brand, gray); stroke: env(unset, crimson); }"),
        env(&[("brand", "rebeccapurple")]),
    );
    let computed = styles(&table, &tree, &host);
    assert_eq!(
        paint_hex(computed[nodes.bar].paint(Property::Fill)),
        "#663399"
    );
    assert_eq!(
        paint_hex(computed[nodes.bar].paint(Property::Stroke)),
        "#dc143c"
    );
}

/// A paint of `currentColor` resolves against the color of each row of a mark: a mix of it tints
/// each row's color, and `auto` paints it.
#[test]
fn a_paint_of_current_color_resolves_against_a_row_color() {
    let (tree, nodes) = chart();
    let css = "mark[type=line] { stroke: color-mix(in srgb, currentColor 50%, white); }";
    let computed = styles(&with(css), &tree, &HostInputs::default());
    let red = [1.0, 0.0, 0.0, 1.0];
    match computed.paint_with(nodes.line, Property::Stroke, red) {
        Paint::Color([r, g, b, a]) => {
            assert!((r - 1.0).abs() < 1e-3 && (g - 0.5).abs() < 1e-3 && (b - 0.5).abs() < 1e-3);
            assert_eq!(a, 1.0);
        }
        other => panic!("a color, not {other:?}"),
    }
    assert_eq!(
        computed.paint_with(nodes.bar, Property::Fill, red),
        Paint::Color(red)
    );
    assert_eq!(
        computed.paint_with(nodes.bar, Property::Stroke, red),
        Paint::None
    );
}

/// A trail is a line of varying width, which paints with its stroke.
#[test]
fn a_trail_paints_its_color_with_its_stroke() {
    let mut tree = StyleTree::new(Element::Chart);
    let plot = tree.add(tree.root(), Element::Plot);
    tree.set_attribute(plot, "type", "cartesian");
    let trail = tree.add(plot, Element::Mark);
    tree.set_attribute_forms(trail, "type", "trail", "trail");
    let computed = styles(&base_table(Env::new()), &tree, &HostInputs::default());
    assert_eq!(
        paint_hex(computed[trail].paint(Property::Stroke)),
        "#4c78a8"
    );
    assert_eq!(computed[trail].paint(Property::Fill), Paint::None);
}

#[test]
fn the_base_stylesheet_derives_its_colors_from_its_parameters() {
    let (tree, nodes) = chart();
    let host = HostInputs::default();
    let white = styles(&base_table(Env::new()), &tree, &host);
    assert_eq!(
        paint_hex(white[nodes.rule].paint(Property::Stroke)),
        "#888888"
    );
    assert_eq!(
        paint_hex(white[nodes.grid].paint(Property::Stroke)),
        "#dddddd"
    );
    assert_eq!(hex(white[nodes.chart].color(Property::Color)), "#000000");
    assert_eq!(paint_hex(white[nodes.bar].paint(Property::Fill)), "#4c78a8");
    assert_eq!(white[nodes.bar].paint(Property::Stroke), Paint::None);
    assert_eq!(
        paint_hex(white[nodes.line].paint(Property::Stroke)),
        "#4c78a8"
    );
    assert_eq!(white[nodes.line].paint(Property::Fill), Paint::None);
    assert!(matches!(
        white[nodes.bar_label].paint(Property::Fill),
        Paint::Contrast { .. }
    ));
    assert!(
        !white[nodes.grid].is_hidden(),
        "a sequential axis draws its grid"
    );
    // The border shorthand of a Cartesian plot substitutes and expands to its longhands.
    assert_eq!(
        hex(white[nodes.plot].color(Property::BorderColor)),
        "#dddddd"
    );
    assert_eq!(
        white[nodes.plot].keyword(Property::BorderStyle),
        Some("solid")
    );
    assert_eq!(white[nodes.plot].number(Property::BorderWidth), Some(1.0));

    let dark = styles(&base_table(env(&[("background", "#1e1e2e")])), &tree, &host);
    assert_eq!(
        hex(dark[nodes.chart].color(Property::Background)),
        "#1e1e2e"
    );
    assert_eq!(hex(dark[nodes.chart].color(Property::Color)), "#ffffff");
    assert_eq!(
        paint_hex(dark[nodes.rule].paint(Property::Stroke)),
        "#878790"
    );
    assert_eq!(
        paint_hex(dark[nodes.grid].paint(Property::Stroke)),
        "#3c3c4a"
    );

    let accent = styles(&base_table(env(&[("accent", "teal")])), &tree, &host);
    assert_eq!(
        paint_hex(accent[nodes.bar].paint(Property::Fill)),
        "#008080"
    );
    assert_eq!(
        white[nodes.scale].get(Property::Scheme),
        &Computed::Sequence(vec![Computed::String("tableau10".into())].into())
    );
}

#[test]
fn the_vega_lite_stylesheet_gives_its_looks() {
    let mut tree = StyleTree::new(Element::Chart);
    let plot = tree.add(tree.root(), Element::Plot);
    tree.set_attribute(plot, "type", "cartesian");
    let symbol = tree.add(plot, Element::Mark);
    tree.set_attribute_forms(symbol, "type", "symbol", "symbol");
    let table = base_table(Env::new()).with_author(table::vegalite().clone(), Env::new());
    let computed = styles(&table, &tree, &HostInputs::default());
    assert_eq!(computed[symbol].paint(Property::Fill), Paint::None);
    assert_eq!(
        paint_hex(computed[symbol].paint(Property::Stroke)),
        "#4c78a8"
    );
    assert_eq!(computed[plot].number(Property::Width), Some(300.0));
    assert_eq!(
        computed[tree.root()].number(Property::PaddingTop),
        Some(5.0)
    );
}

#[test]
fn a_mark_matches_its_kind_and_its_function_name() {
    let mut tree = StyleTree::new(Element::Chart);
    let plot = tree.add(tree.root(), Element::Plot);
    tree.set_attribute(plot, "type", "polar");
    let bar = tree.add(plot, Element::Mark);
    tree.set_attribute_forms(bar, "type", "bar", "polar_bar");
    let host = HostInputs::default();
    for (css, expected) in [
        ("mark[type=bar] { fill: red; }", "#ff0000"),
        ("mark[type=polar_bar] { fill: red; }", "#ff0000"),
        ("mark[type^=polar_] { fill: red; }", "#ff0000"),
        (
            "plot[type=polar] > mark[type=bar] { fill: red; }",
            "#ff0000",
        ),
        ("mark:not([type=bar]) { fill: red; }", "#4c78a8"),
        ("mark[type=geo_bar] { fill: red; }", "#4c78a8"),
    ] {
        let computed = styles(&with(css), &tree, &host);
        assert_eq!(
            paint_hex(computed[bar].paint(Property::Fill)),
            expected,
            "{css}"
        );
    }
}

#[test]
fn structural_pseudo_classes_and_has_match_the_tree() {
    let mut tree = StyleTree::new(Element::Chart);
    let row = tree.add(tree.root(), Element::Plot);
    tree.set_attribute(row, "type", "hconcat");
    let first = tree.add(row, Element::Chart);
    let second = tree.add(row, Element::Chart);
    let plot = tree.add(second, Element::Plot);
    tree.set_attribute(plot, "type", "cartesian");
    let bar = tree.add(plot, Element::Mark);
    tree.set_attribute_forms(bar, "type", "bar", "bar");
    let computed = styles(
        &with(
            "plot[type=hconcat] > chart:nth-of-type(2) { background: red; }
             plot:has(> mark[type=bar]) { background: blue; }
             plot[type=hconcat] > chart:first-child { background: green; }
             :root { background: yellow; }",
        ),
        &tree,
        &HostInputs::default(),
    );
    assert_eq!(hex(computed[second].color(Property::Background)), "#ff0000");
    assert_eq!(hex(computed[first].color(Property::Background)), "#008000");
    assert_eq!(hex(computed[plot].color(Property::Background)), "#0000ff");
    assert_eq!(
        hex(computed[tree.root()].color(Property::Background)),
        "#ffff00"
    );
}

#[test]
fn palettes_come_from_rules_then_from_the_built_in_palettes() {
    let (tree, _) = chart();
    let computed = styles(&base_table(Env::new()), &tree, &HostInputs::default());
    let tableau10 = computed.palette("Tableau10").expect("a built-in palette");
    assert_eq!(tableau10.colors.len(), 10);
    assert_eq!(hex(Some(tableau10.colors[0])), "#4c78a8");
    assert_eq!(computed.palette("okabe-ito").unwrap().colors.len(), 8);
    assert!(computed.palette("viridis").unwrap().cvd_safe);
    assert!(computed
        .palette("blueorange")
        .unwrap()
        .fits(ScaleKind::Diverging));
    assert!(!computed
        .palette("blues")
        .unwrap()
        .fits(ScaleKind::Categorical));

    let table = base_table(Env::new()).with_author(
        sheet(
            "@palette brand { kind: categorical; colors: env(accent, #2a6f97), #e9a03b; }
             @palette blues { kind: sequential; colors: white, navy; interpolate: srgb; }",
        ),
        env(&[("accent", "teal")]),
    );
    let computed = styles(&table, &tree, &HostInputs::default());
    let brand = computed.palette("brand").unwrap();
    assert_eq!(hex(Some(brand.colors[0])), "#008080");
    let blues = computed.palette("blues").unwrap();
    assert_eq!(blues.kind, PaletteKind::Sequential);
    assert_eq!(
        blues.colors.len(),
        2,
        "a document redefines a built-in name"
    );
    // An ordinal scale of one value takes the middle of a sequential palette, not an end.
    let sample = blues.sample(1);
    assert_eq!(rgb8(sample[0]), [128, 128, 192]);
}

#[test]
fn inline_values_parse_as_their_property() {
    assert_eq!(
        Property::LabelAngle.parse_value("45"),
        Some(Value::Numeric(avenger_style::values::Calc::Leaf(
            45.0,
            avenger_style::values::Unit::Number
        )))
    );
}

#[test]
fn the_standard_stylesheets_parse_without_diagnostics() {
    for (text, options) in [
        (table::BASE_CSS, ParseOptions::default()),
        (table::VEGALITE_CSS, ParseOptions::default()),
        (
            table::RESET_CSS,
            ParseOptions {
                allow_reserved_layers: true,
                ..Default::default()
            },
        ),
    ] {
        let (_, diagnostics) = Stylesheet::parse(text, SourceId(0), options);
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }
}

#[test]
fn ids_outrank_classes_and_select_every_node_that_shares_them() {
    // Two facets give their plots one id, as the format lets them, and a third plot's id needs an
    // escape.
    let mut tree = StyleTree::new(Element::Chart);
    let chart = tree.root();
    let facets = [0, 1].map(|_| {
        let plot = tree.add(chart, Element::Plot);
        tree.set_id(plot, "sales");
        tree.add_class(plot, "wide");
        plot
    });
    let first = tree.add(chart, Element::Plot);
    tree.set_id(first, "1st");
    tree.add_class(first, "wide");
    let axis = tree.add(first, Element::Axis);
    tree.set_id(axis, "time");
    let label = tree.add(axis, Element::Label);

    // The id rule comes first, so it wins by specificity, not by order.
    let table = with(
        "#sales { color: red; }
         #\\31 st { color: orange; }
         axis#time > label { color: purple; }
         .wide { color: green; }
         plot { color: blue; }",
    );
    let computed = styles(&table, &tree, &HostInputs::default());
    for plot in facets {
        assert_eq!(hex(computed[plot].color(Property::Color)), "#ff0000");
    }
    assert_eq!(hex(computed[first].color(Property::Color)), "#ffa500");
    assert_eq!(hex(computed[label].color(Property::Color)), "#800080");

    // An inline style wins over an id rule.
    tree.set_inline(
        facets[1],
        Property::Color,
        Property::Color.parse_value("blue").unwrap(),
    );
    let computed = styles(&table, &tree, &HostInputs::default());
    assert_eq!(hex(computed[facets[0]].color(Property::Color)), "#ff0000");
    assert_eq!(hex(computed[facets[1]].color(Property::Color)), "#0000ff");
}

/// The `color` of the root of a tree of one node with the author stylesheet `sheet`.
fn root_color(sheet: impl Into<avenger_style::Sheet>) -> String {
    let tree = StyleTree::new(Element::Chart);
    let table = base_table(Env::new()).with_author(sheet, Env::new());
    hex(styles(&table, &tree, &HostInputs::default())[tree.root()].color(Property::Color))
}

/// A loader of the stylesheets of `files`, by name, whose URLs resolve against the importing
/// file's directory, and which records each request.
fn files<'a>(
    files: &'a [(&'a str, &'a str)],
    requests: &'a mut Vec<(String, Option<String>)>,
) -> impl FnMut(&str, Option<&str>) -> Result<Loaded, String> + 'a {
    move |url: &str, base: Option<&str>| {
        requests.push((url.to_string(), base.map(str::to_string)));
        let directory = base
            .and_then(|base| base.rsplit_once('/'))
            .map_or("", |(directory, _)| directory);
        let resolved = match directory {
            "" => url.to_string(),
            directory => format!("{directory}/{url}"),
        };
        let position = (files.iter())
            .position(|(name, _)| *name == resolved)
            .ok_or_else(|| "not found".to_string())?;
        Ok(Loaded {
            url: resolved,
            source: SourceId(10 + position as u32),
            text: files[position].1.to_string(),
        })
    }
}

fn load(
    text: &str,
    files_: &[(&str, &str)],
) -> (avenger_style::Sheet, Vec<avenger_style::Diagnostic>) {
    let mut requests = Vec::new();
    let options = ParseOptions {
        allow_import: true,
        ..Default::default()
    };
    let loaded = avenger_style::Sheet::load(
        text,
        Some("main.css"),
        SourceId(2),
        options,
        &mut files(files_, &mut requests),
    );
    loaded
}

#[test]
fn layer_statements_before_an_import_order_its_layers() {
    let imported = [(
        "x.css",
        "@layer b { chart { color: blue; } } @layer a { chart { color: red; } }",
    )];
    // The statement declares `a` before `b`, so `b` wins.
    let (before, diagnostics) = load("@layer a, b; @import 'x.css';", &imported);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    assert_eq!(root_color(before), "#0000ff");
    // After the import, the statement declares nothing new, and the import's order holds.
    let (after, _) = load("@import 'x.css'; @layer a, b;", &imported);
    assert_eq!(root_color(after), "#ff0000");
}

#[test]
fn the_loader_loads_imports_relative_to_their_files() {
    let mut requests = Vec::new();
    let files_ = [
        (
            "theme/brand.css",
            "@import 'colors.css' layer(colors); chart { font-size: 2rem; }",
        ),
        ("theme/colors.css", "chart { color: teal; }"),
    ];
    let (sheet, diagnostics) = avenger_style::Sheet::load(
        "@import 'std:vegalite.css'; @import 'theme/brand.css';",
        Some("main.css"),
        SourceId(2),
        ParseOptions {
            allow_import: true,
            ..Default::default()
        },
        &mut files(&files_, &mut requests),
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    // `std:` URLs load without the loader, and each request names the importing file.
    assert_eq!(
        requests,
        [
            ("theme/brand.css".to_string(), Some("main.css".to_string())),
            (
                "colors.css".to_string(),
                Some("theme/brand.css".to_string())
            ),
        ]
    );
    assert_eq!(sheet.imports().len(), 2);
    assert_eq!(
        sheet.imports()[0].stylesheet().layers[0].name,
        ["vegalite".to_string()]
    );
    assert_eq!(sheet.imports()[1].imports().len(), 1);
    assert_eq!(root_color(sheet), "#008080");
}

#[test]
fn imports_that_do_not_load_are_errors_at_their_spans() {
    let files_ = [
        ("a.css", "@import 'b.css'; chart { color: red; }"),
        ("b.css", "@import 'a.css';"),
        ("bad.css", "chart { color: nope; }"),
    ];
    let css = "@import 'missing.css';
               @import 'std:base.css' layer(base);
               @import 'std:fonts.css';
               @import 'a.css';
               @import 'bad.css';";
    let (sheet, diagnostics) = load(css, &files_);
    let messages: Vec<&str> = (diagnostics.iter())
        .map(|diagnostic| diagnostic.message.as_str())
        .collect();
    assert_eq!(
        messages,
        [
            "cannot load `missing.css`: not found",
            "`std:base.css` loads in the user-agent origin, outside every layer, so its import \
             takes no layer",
            "`std:fonts.css` is not a standard stylesheet",
            "`a.css` imports itself: a.css → b.css → a.css",
            "`nope` is not a valid value of `color`",
        ]
    );
    // The diagnostics of an imported file are at the spans of its own source, and the others at
    // their `@import` rules in the importing file.
    assert_eq!(diagnostics[0].span.source, SourceId(2));
    assert_eq!(
        &css[diagnostics[0].span.start..diagnostics[0].span.end],
        "'missing.css'"
    );
    assert_eq!(diagnostics[3].span.source, SourceId(11));
    assert_eq!(diagnostics[4].span.source, SourceId(12));
    // Every import has a sheet, which is empty when it does not load, and the rest still apply.
    assert_eq!(sheet.imports().len(), 5);
    assert_eq!(root_color(sheet), "#ff0000");
}

#[test]
fn missing_color_components_take_the_other_colors_values_in_a_mix() {
    assert_eq!(
        root_color(sheet(
            "chart { color: color-mix(in srgb, color(srgb none 0 0), red); }"
        )),
        "#ff0000"
    );
    // A missing hue draws as zero.
    assert_eq!(
        root_color(sheet("chart { color: oklch(0.5 0.2 none); }")),
        root_color(sheet("chart { color: oklch(0.5 0.2 0); }"))
    );
    // A missing hue carries to the hue of the interpolation space, so the mix has the other
    // color's hue.
    let tree = StyleTree::new(Element::Chart);
    let computed = styles(
        &with("chart { color: color-mix(in oklch, hsl(none 50% 50%), oklch(0.6 0.15 120)); }"),
        &tree,
        &HostInputs::default(),
    );
    let mixed = computed[tree.root()].color(Property::Color).unwrap();
    let hue = avenger_color::AbsoluteColor::from_rgba(mixed)
        .to_color_space(avenger_color::ColorSpace::Oklch)
        .components[2];
    // The sRGB gamut moves the hue a little. Without the carried hue, it would be near 72.
    assert!((hue - 120.0).abs() < 2.0, "the hue is {hue}");
    // Negative chroma is zero, a gray.
    let tree = StyleTree::new(Element::Chart);
    let computed = styles(
        &with("chart { color: oklch(0.5 -0.2 0); }"),
        &tree,
        &HostInputs::default(),
    );
    let [r, g, b] = rgb8(computed[tree.root()].color(Property::Color).unwrap());
    assert!(r == g && g == b, "{r} {g} {b}");
}

#[test]
fn calc_values_clamp_to_the_range_of_their_property() {
    let (tree, nodes) = chart();
    let table = with("chart { font-size: calc(-1px); } mark { opacity: calc(0.5 * 3); }");
    let computed = styles(&table, &tree, &HostInputs::default());
    assert_eq!(computed[nodes.chart].font_size(), 0.0);
    assert_eq!(computed[nodes.bar].number(Property::Opacity), Some(1.0));
}

#[test]
fn custom_properties_compute_where_they_are_declared() {
    let (tree, nodes) = chart();
    let color = |table: &StyleTable, node: NodeId| {
        hex(styles(table, &tree, &HostInputs::default())[node].color(Property::Color))
    };

    // A child inherits the computed value, so its own `--a` does not change the parent's `--b`.
    let table = with("chart { --a: red; --b: var(--a); } plot { --a: blue; color: var(--b); }");
    assert_eq!(color(&table, nodes.plot), "#ff0000");

    // `env()` reads the values of the stylesheet that declares the custom property.
    let env = |value: &str| env(&[("brand", value)]);
    let table = base_table(Env::new())
        .with_author(sheet("chart { --c: env(brand); }"), env("red"))
        .with_author(sheet("plot { color: var(--c); }"), env("blue"));
    assert_eq!(color(&table, nodes.plot), "#ff0000");

    // Properties on a cycle are invalid, also through fallbacks, so `color` inherits.
    let table = with(
        "chart { color: green; }
         plot { --a: var(--b, red); --b: var(--a, blue); color: var(--a); }",
    );
    assert_eq!(color(&table, nodes.plot), "#008000");
    // A property that refers to a cycle takes its fallback.
    let table = with(
        "chart { color: green; }
         plot { --a: var(--b); --b: var(--a); --c: var(--a, orange); color: var(--c); }",
    );
    assert_eq!(color(&table, nodes.plot), "#ffa500");

    // `initial` removes an inherited value, and `inherit` keeps it.
    let table = with(
        "chart { --a: red; color: green; }
         plot { --a: initial; color: var(--a, blue); }
         mark { --a: inherit; }",
    );
    assert_eq!(color(&table, nodes.plot), "#0000ff");
}

#[test]
fn the_base_stylesheet_applies_only_when_a_stylesheet_imports_it() {
    let tree = StyleTree::new(Element::Chart);
    let root = tree.root();
    let family = |table: &StyleTable, host: &HostInputs| {
        styles(table, &tree, host)[root]
            .get(Property::FontFamily)
            .clone()
    };
    let initial = family(&StyleTable::new(), &HostInputs::default());
    let based = family(&base_table(Env::new()), &HostInputs::default());
    assert_ne!(initial, based, "the base stylesheet sets the font");

    // An imported base stylesheet is in the user-agent origin, so a host theme wins over it, and
    // an author stylesheet wins over both.
    let theme = |css: &str| HostInputs {
        theme: Some(Arc::new(HostTheme {
            sheet: sheet(css).into(),
            env: Env::new(),
        })),
        ..Default::default()
    };
    let host = theme("chart { background: navy; }");
    let computed = styles(&base_table(Env::new()), &tree, &host);
    assert_eq!(hex(computed[root].color(Property::Background)), "#000080");
    let computed = styles(&with("chart { background: teal; }"), &tree, &host);
    assert_eq!(hex(computed[root].color(Property::Background)), "#008080");
}

#[test]
fn layer_declarations_count_only_when_their_conditions_hold() {
    let tree = StyleTree::new(Element::Chart);
    let css = "@media print { @layer a, b; }
               @layer b { chart { color: blue; } }
               @layer a { chart { color: red; } }";
    let table = StyleTable::new().with_author(sheet(css), Env::new());
    let cascade = Cascade::new(&table);
    let screen = HostInputs::default();
    let print = HostInputs {
        preferences: Preferences {
            print: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let color =
        |host: &HostInputs| hex(cascade.compute(&tree, host)[tree.root()].color(Property::Color));
    // On screen, the blocks declare `b` before `a`, so `a` wins.
    assert_eq!(color(&screen), "#ff0000");
    // In print, the statement declares `a` before `b`, so `b` wins.
    assert_eq!(color(&print), "#0000ff");
    // The condition of the statement decides whether new host inputs change the styles.
    assert!(cascade.compute(&tree, &screen).needs_update(&print));
}

#[test]
fn anonymous_import_layers_of_different_stylesheets_are_different_layers() {
    let files_ = [
        ("first.css", "chart { color: red !important; }"),
        ("second.css", "chart { color: blue !important; }"),
    ];
    let (first, _) = load("@import 'first.css' layer;", &files_);
    let (second, _) = load("@import 'second.css' layer;", &files_);
    let tree = StyleTree::new(Element::Chart);
    let table = StyleTable::new()
        .with_author(first, Env::new())
        .with_author(second, Env::new());
    // Important declarations reverse the order of the layers, so the first layer wins.
    let computed = styles(&table, &tree, &HostInputs::default());
    assert_eq!(hex(computed[tree.root()].color(Property::Color)), "#ff0000");
}

#[test]
fn missing_components_carry_through_mixes_and_draw_as_zero() {
    let rgba = |color: &str| {
        let tree = StyleTree::new(Element::Chart);
        let table = with(&format!("chart {{ color: {color}; }}"));
        let computed = styles(&table, &tree, &HostInputs::default());
        computed[tree.root()]
            .color(Property::Color)
            .unwrap()
            .map(|channel| (channel * 1000.0).round() / 1000.0)
    };
    // A missing component of a color in the interpolation space stays missing, also one without
    // analogues, such as the whiteness of HWB.
    assert_eq!(
        rgba("color-mix(in hwb, hwb(0 none 0%), hwb(0 50% 0%))"),
        [1.0, 0.5, 0.5, 1.0]
    );
    // The lightness, chroma, and hue of OKLCH have no analogues in sRGB, so the color converts
    // as black.
    assert_eq!(
        rgba("color-mix(in srgb, oklch(none none none), red)"),
        [0.5, 0.0, 0.0, 1.0]
    );
    // Two missing alphas keep the channels of the colors, and the mix's alpha stays missing for
    // the mix around it.
    assert_eq!(
        rgba(
            "color-mix(in srgb, color-mix(in srgb, rgb(255 0 0 / none), rgb(255 0 0 / none)), blue)"
        ),
        [0.5, 0.0, 0.5, 1.0]
    );
    // A missing hue of a mix draws as zero, also after `longer` interpolation.
    assert_eq!(
        rgba("color-mix(in hsl longer hue, hsl(none 100% 50%), hsl(none 100% 50%))"),
        [1.0, 0.0, 0.0, 1.0]
    );
    // A missing alpha draws as zero.
    assert_eq!(rgba("rgb(255 0 0 / none)")[3], 0.0);
    assert_eq!(
        rgba("color-mix(in srgb, rgb(255 0 0 / none), rgb(0 0 255 / none))")[3],
        0.0
    );
}

#[test]
fn inherited_colors_that_read_current_color_follow_each_nodes_color() {
    let (tree, nodes) = chart();
    // Without the base stylesheet, which declares the paints of labels, the label inherits them.
    let only = |css: &str| StyleTable::new().with_author(sheet(css), Env::new());
    let table = only(
        "mark { color: red; fill: currentColor; stroke: color-mix(in srgb, currentColor, white); }
         mark > label { color: blue; }",
    );
    let computed = styles(&table, &tree, &HostInputs::default());
    assert_eq!(hex(computed[nodes.bar].color(Property::Fill)), "#ff0000");
    // The label inherits `currentColor` and the mix, which read its own blue.
    assert_eq!(
        hex(computed[nodes.bar_label].color(Property::Fill)),
        "#0000ff"
    );
    assert_eq!(
        hex(computed[nodes.bar_label].color(Property::Stroke)),
        "#8080ff"
    );
    assert_eq!(
        computed[nodes.bar_label].paint(Property::Fill),
        Paint::Color([0.0, 0.0, 1.0, 1.0])
    );
    // `color: currentColor` is the parent's color, which inherits as a color.
    let table =
        with("plot { color: green; } mark { color: currentColor; } mark > label { color: blue; }");
    let computed = styles(&table, &tree, &HostInputs::default());
    assert_eq!(hex(computed[nodes.bar].color(Property::Color)), "#008000");
}

#[test]
fn palettes_resolve_with_the_host_preferences() {
    let tree = StyleTree::new(Element::Chart);
    let table = StyleTable::new().with_author(
        sheet("@palette ink { kind: categorical; colors: CanvasText, Canvas; }"),
        Env::new(),
    );
    let dark = HostInputs {
        preferences: Preferences {
            color_scheme: ColorScheme::Dark,
            ..Default::default()
        },
        ..Default::default()
    };
    let computed = styles(&table, &tree, &dark);
    let colors: Vec<String> = (computed.palette("ink").unwrap().colors.iter())
        .map(|color| hex(Some(*color)))
        .collect();
    assert_eq!(colors, ["#ffffff", "#000000"]);
}

#[test]
fn palettes_with_env_are_checked_after_substitution() {
    let tree = StyleTree::new(Element::Chart);
    let computed = |stops: &str| {
        let table = StyleTable::new().with_author(
            sheet("@palette split { kind: diverging; colors: env(stops); }"),
            env(&[("stops", stops)]),
        );
        styles(&table, &tree, &HostInputs::default())
    };
    let valid = computed("red, white, blue");
    assert_eq!(valid.palette("split").unwrap().colors.len(), 3);
    assert!(valid.warnings().is_empty());

    for (stops, message) in [
        ("red", "a diverging palette needs at least three colors"),
        (
            "red, 12px, blue",
            "`red, 12px, blue` is not a list of colors for @palette `split`",
        ),
    ] {
        let invalid = computed(stops);
        assert!(invalid.palette("split").is_none(), "{stops}");
        let warnings: Vec<&str> = invalid
            .warnings()
            .iter()
            .map(StyleWarning::message)
            .collect();
        assert_eq!(warnings, [message]);
        assert!(matches!(
            &invalid.warnings()[0],
            StyleWarning::Palette { name, .. } if name == "split"
        ));
    }
}

#[test]
fn a_sheet_lists_the_env_names_of_its_imports() {
    let files_ = [(
        "brand.css",
        "chart { --accent: env(accent, teal); } @palette brand { kind: categorical; colors: env(brand-colors, red); }",
    )];
    let (sheet, diagnostics) = load(
        "@import 'std:base.css'; @import 'brand.css'; chart { font-size: env(size, 12px); }",
        &files_,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    let names: Vec<&str> = sheet.env_names().into_iter().collect();
    assert_eq!(
        names,
        [
            "accent",
            "background",
            "brand-colors",
            "font-family",
            "foreground",
            "size"
        ]
    );
    // A stylesheet's own names exclude those of its imports.
    let own: Vec<&str> = sheet.stylesheet().env_names().collect();
    assert_eq!(own, ["size"]);
}

#[test]
fn siblings_share_styles_only_when_selectors_cannot_tell_them_apart() {
    // An axis with three labels; the second has a class and the third an inline style.
    let mut tree = StyleTree::new(Element::Chart);
    let plot = tree.add(tree.root(), Element::Plot);
    let axis = tree.add(plot, Element::Axis);
    tree.set_attribute(axis, "channel", "x");
    let labels: Vec<NodeId> = (0..4).map(|_| tree.add(axis, Element::Label)).collect();
    tree.add_class(labels[2], "muted");
    tree.set_inline(
        labels[3],
        Property::Color,
        Property::Color.parse_value("blue").unwrap(),
    );
    let colors = |css: &str| {
        let computed = styles(&with(css), &tree, &HostInputs::default());
        labels
            .iter()
            .map(|label| hex(computed[*label].color(Property::Color)))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        colors("label { color: red; } .muted { color: gray; }"),
        ["#ff0000", "#ff0000", "#808080", "#0000ff"]
    );
    // Selectors that read positions or descendants tell twins apart.
    assert_eq!(
        colors("label { color: red; } label:nth-child(2) { color: green; }"),
        ["#ff0000", "#008000", "#ff0000", "#0000ff"]
    );
    assert_eq!(
        colors("label { color: red; } label + label { color: green; }"),
        ["#ff0000", "#008000", "#008000", "#0000ff"]
    );
    assert_eq!(
        colors("label { color: red; } :is(label:first-child) { color: green; }"),
        ["#008000", "#ff0000", "#ff0000", "#0000ff"]
    );

    // A twin has the warnings of the sibling whose style it shares.
    let computed = styles(
        &with("label { color: var(--missing); }"),
        &tree,
        &HostInputs::default(),
    );
    let warned: Vec<NodeId> = (computed.warnings().iter())
        .filter_map(|warning| match warning {
            StyleWarning::Property { node, property, .. } if property == "color" => Some(*node),
            _ => None,
        })
        .collect();
    assert_eq!(warned, labels[..3]);
}

#[test]
fn equal_style_trees_have_equal_styles() {
    let (first, nodes) = chart();
    let (second, _) = chart();
    assert_eq!(first, second, "trees built the same way are equal");
    let table = with(".highlight { fill: crimson; }");
    let host = HostInputs::sized(400.0, 300.0);
    let (a, b) = (
        styles(&table, &first, &host),
        styles(&table, &second, &host),
    );
    for node in first.nodes() {
        assert_eq!(a[node].get(Property::Fill), b[node].get(Property::Fill));
    }

    // A class, an attribute, an inline style, or another node makes a different tree.
    let mut classed = first.clone();
    classed.add_class(nodes.line, "highlight");
    let mut attributed = first.clone();
    attributed.set_attribute(nodes.axis, "channel", "y");
    let mut inline = first.clone();
    inline.set_inline(
        nodes.bar,
        Property::Opacity,
        Property::Opacity.parse_value("0.5").unwrap(),
    );
    let mut grown = first.clone();
    grown.add(nodes.axis, Element::Label);
    for changed in [classed, attributed, inline, grown] {
        assert_ne!(first, changed);
    }
}
