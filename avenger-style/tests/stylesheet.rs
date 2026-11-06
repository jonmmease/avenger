//! Stylesheets: each error of the style language that a stylesheet causes, the spans of
//! diagnostics, and the structure of parsed rules.

use avenger_style::{
    condition::{Comparison, Condition, Feature, Preference, Query, SizeFeature},
    Declared, Diagnostic, Name, PaletteKind, ParseOptions, Property, Severity, SourceId,
    Stylesheet, Value,
};
use cssparser::ToCss;

fn parse(css: &str) -> (Stylesheet, Vec<Diagnostic>) {
    Stylesheet::parse(css, SourceId(7), ParseOptions::default())
}

fn file(css: &str) -> (Stylesheet, Vec<Diagnostic>) {
    Stylesheet::parse(
        css,
        SourceId(7),
        ParseOptions {
            allow_import: true,
            ..Default::default()
        },
    )
}

/// The one diagnostic of `css`, which has `severity` and whose message contains `message`.
#[track_caller]
fn one(css: &str, severity: Severity, message: &str) -> Diagnostic {
    let (_, diagnostics) = parse(css);
    assert_eq!(diagnostics.len(), 1, "{css}: {diagnostics:#?}");
    let diagnostic = diagnostics.into_iter().next().unwrap();
    assert_eq!(diagnostic.severity, severity, "{css}");
    assert!(
        diagnostic.message.contains(message),
        "{css}: `{}` contains `{message}`",
        diagnostic.message
    );
    diagnostic
}

#[track_caller]
fn error(css: &str, message: &str) -> Diagnostic {
    one(css, Severity::Error, message)
}

#[test]
fn unknown_elements_and_attributes_are_errors() {
    let bar = error(
        "bar { fill: red; }",
        "`bar` is not an element of the style tree",
    );
    assert_eq!(
        bar.help.as_deref(),
        Some("select the marks with mark[type=bar]")
    );
    let polar = error("polar_bar { fill: red; }", "`polar_bar` is not an element");
    assert_eq!(
        polar.help.as_deref(),
        Some("select the marks with mark[type=polar_bar]")
    );
    error(
        "axis[mark=bar] { stroke: red; }",
        "the element `axis` has no attribute `mark`",
    );
    error(
        "[colour] { fill: red; }",
        "no element has the attribute `colour`",
    );
}

#[test]
fn interaction_pseudo_classes_other_than_focus_visible_are_errors() {
    let hover = error(
        "mark:hover { opacity: 1; }",
        "`:hover` is not supported; the only interaction pseudo-class is :focus-visible",
    );
    assert_eq!(
        hover.help.as_deref(),
        Some("style the marks of a selection with a selection condition")
    );
    let focus = error(
        "mark:focus { outline: 2px solid; }",
        "`:focus` is not supported",
    );
    assert_eq!(focus.help.as_deref(), Some("use :focus-visible"));
    error(
        "title:focus-visible { outline-color: red; }",
        "`title` cannot have focus",
    );
    error(
        "mark:has(item:focus-visible) { outline-color: red; }",
        ":focus-visible is valid only in the last compound selector",
    );
    error(
        "mark:not(:focus-visible) { outline-color: red; }",
        ":focus-visible is valid only in the last compound selector",
    );
    error(
        "chart:focus-visible mark { outline-color: red; }",
        ":focus-visible is valid only in the last compound selector",
    );
    error(
        "axis::before { stroke: red; }",
        "pseudo-elements are not supported",
    );
}

#[test]
fn virtual_nodes_match_only_through_focus_visible() {
    error(
        "item { outline: 2px solid red; }",
        "`item` is a virtual node; select it with item:focus-visible",
    );
    error(
        "item[type=point]:focus-visible { outline-color: red; }",
        "the virtual node `item` has no attributes",
    );
    let (sheet, diagnostics) = parse(
        "mark[type=bar] > item:focus-visible { outline: 3px solid darkorange; outline-offset: 1px; }
         legend > entry:focus-visible { outline-style: dashed; }",
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    assert_eq!(sheet.rules.len(), 2);
    // The `outline` shorthand sets its three longhands.
    assert_eq!(sheet.rules[0].declarations.len(), 4);
}

#[test]
fn outline_properties_apply_only_through_focus_visible() {
    error(
        "legend { outline: 1px solid; }",
        "outline properties apply only through :focus-visible",
    );
    error(
        "item:focus-visible { opacity: 1; }",
        "only outline properties apply through :focus-visible",
    );
}

#[test]
fn invalid_values_are_errors_and_unknown_properties_are_warnings() {
    error(
        "mark { stroke-width: wide; }",
        "`wide` is not a valid value of `stroke-width`",
    );
    one(
        "mark { stroke-wdth: 2; }",
        Severity::Warning,
        "unknown property `stroke-wdth`",
    );
    error(
        "axis > label { text-align: left; }",
        "`left` is not a valid value of `text-align`; use start, center, end, or auto",
    );
    error(
        "plot[type=facet_grid] { gap: 10 20 30; }",
        "`10 20 30` is not a valid value of `gap`; use one length or two",
    );
    error(
        "plot[type=facet_wrap] { axes: all; }",
        "`all` is not a valid value of `axes`; use outer or each",
    );
    error(
        "mark[type=symbol] { label-position: center; }",
        "`center` is not a valid value of `label-position`",
    );
    error(
        "mark[type=bar] { label-overflow: clip; }",
        "`clip` is not a valid value of `label-overflow`; use hide, outside, shrink, or show",
    );
    error(
        "chart { forced-color-adjust: preserve-parent-color; }",
        "`preserve-parent-color` is not a valid value of `forced-color-adjust`",
    );
}

#[test]
fn palettes_check_their_descriptors() {
    error(
        "@palette p { colors: red, blue; }",
        "@palette `p` has no `kind`",
    );
    error(
        "@palette p { kind: diverging; colors: red, blue; }",
        "a diverging palette needs at least three colors",
    );
    error(
        "@palette p { kind: categorical; colors: red, blue; cvd-safe: yes; }",
        "`yes` is not a valid value of `cvd-safe`; use true or false",
    );
    error(
        "@palette p { kind: categorical; colors: red; shade: dark; }",
        "`shade` is not a descriptor of @palette",
    );

    let (sheet, diagnostics) = parse(
        "@palette Brand { kind: categorical; colors: env(accent, #2a6f97), #e9a03b, #8e8e8e; }
         @palette warm { kind: sequential; colors: #fee, #c00; interpolate: oklch; cvd-safe: true; }",
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    assert_eq!(sheet.palettes.len(), 2);
    let brand = &sheet.palettes[0];
    assert_eq!(brand.name, "brand", "palette names are case-insensitive");
    assert!(matches!(brand.colors, Declared::Pending { .. }));
    assert_eq!(sheet.env_names().collect::<Vec<_>>(), ["accent"]);
    let warm = &sheet.palettes[1];
    assert_eq!(warm.kind, PaletteKind::Sequential);
    assert!(warm.cvd_safe);
    assert!(matches!(&warm.colors, Declared::Value(Value::List(colors)) if colors.len() == 2));
}

#[test]
fn conditions_read_only_the_host_box_and_the_preferences() {
    let viewport = error(
        "@media (max-width: 400px) { chart { color: red; } }",
        "a chart has no viewport; the media feature `max-width` is not supported",
    );
    assert_eq!(
        viewport.help.as_deref(),
        Some("use @container (width <= 400px) to test the host box")
    );
    error(
        "@media (hover: hover) { chart { color: red; } }",
        "the media feature `hover` is not supported",
    );
    error(
        "@container sidebar (width < 400px) { chart { color: red; } }",
        "the host box is the only container; remove the container name `sidebar`",
    );
    error(
        "chart { container-type: inline-size; }",
        "`container-type` is not supported; the host box is the only container",
    );
    error(
        "@container (width < env(narrow)) { chart { color: red; } }",
        "env() is not valid in a condition",
    );

    let (sheet, diagnostics) = parse(
        "@media (prefers-color-scheme: dark) and (forced-colors) { chart { color: white; } }
         @container (400px <= width < 800px) { chart { font-size: 12px; } }",
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    let Condition::Media(queries) = &sheet.rules[0].conditions[0] else {
        panic!("a media condition")
    };
    let Some(Query::And(tests)) = &queries[0].condition else {
        panic!("two tests")
    };
    assert_eq!(
        tests[1],
        Query::Feature(Feature::Preference {
            name: Preference::ForcedColors,
            value: None
        })
    );
    let Condition::Container(Query::Feature(Feature::Size { feature, tests })) =
        &sheet.rules[1].conditions[0]
    else {
        panic!("a size range")
    };
    assert_eq!(*feature, SizeFeature::Width);
    let comparisons: Vec<_> = tests.iter().map(|(comparison, _)| *comparison).collect();
    assert_eq!(comparisons, [Comparison::GreaterOrEqual, Comparison::Less]);
}

#[test]
fn layers_and_imports() {
    error(
        "@layer avenger.base { chart { color: red; } }",
        "the layer name `avenger` is reserved",
    );
    let import = error(
        "@import 'brand.css';",
        "@import is valid only in a stylesheet file",
    );
    // The span of an at-rule's diagnostic starts at its prelude's text.
    assert_eq!((import.span.start, import.span.end), (8, 19));
    assert_eq!(
        import.help.as_deref(),
        Some("load `brand.css` through the host")
    );

    let (sheet, diagnostics) = file(
        "@import 'brand.css' layer(brand);
         @layer base, theme;
         @layer theme { chart { color: red; } }
         @layer base.inner { chart { color: blue; } }",
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    assert_eq!(sheet.imports[0].url, "brand.css");
    assert_eq!(sheet.imports[0].layer, Some(vec!["brand".to_string()]));
    let layers: Vec<String> = (sheet.layers.iter())
        .map(|layer| layer.name.join("."))
        .collect();
    assert_eq!(layers, ["brand", "base", "theme", "base.inner"]);
    assert_eq!(
        sheet.rules[1].layer,
        Some(vec!["base".to_string(), "inner".to_string()])
    );

    let (_, diagnostics) = file("chart { color: red; } @import 'late.css';");
    assert!(diagnostics[0]
        .message
        .contains("comes before every other rule"));
    let (_, diagnostics) = Stylesheet::parse(
        "@layer avenger.reset { * { all: unset } }",
        SourceId(0),
        ParseOptions {
            allow_reserved_layers: true,
            ..Default::default()
        },
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
}

#[test]
fn diagnostics_carry_the_spans_of_their_text() {
    let css = "chart { color: red; }\nmark { stroke-width: wide; }";
    let (_, diagnostics) = parse(css);
    let span = diagnostics[0].span;
    assert_eq!(span.source, SourceId(7));
    assert_eq!(&css[span.start..span.end], "stroke-width: wide");

    let css = "chart {}\naxis[mark=bar] > label { fill: red; }";
    let (_, diagnostics) = parse(css);
    let span = diagnostics[0].span;
    assert_eq!(&css[span.start..span.end], "axis[mark=bar] > label");

    // Error recovery keeps the valid declarations of a rule and the rules after a bad one.
    let (sheet, diagnostics) =
        parse("mark { fill: red; stroke-width: wide; opacity: 0.5 } bar {} axis { offset: 4 }");
    assert_eq!(diagnostics.len(), 2);
    assert_eq!(sheet.rules.len(), 2);
    assert_eq!(sheet.rules[0].declarations.len(), 2);
}

#[test]
fn nested_rules_flatten_relative_to_their_parent() {
    let (sheet, diagnostics) = parse(
        "axis {
           label-angle: 45;
           > label { font-size: 10px; }
           &[channel=x] > title { font-weight: bold; }
           @container (width < 400px) { label-angle: 90; }
           offset: 4;
         }",
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    let texts: Vec<String> = sheet
        .rules
        .iter()
        .map(|rule| {
            let names: Vec<&str> = rule
                .declarations
                .iter()
                .map(|declaration| match &declaration.name {
                    Name::Property(property) => property.name(),
                    Name::Custom(name) => name.as_str(),
                })
                .collect();
            names.join(",")
        })
        .collect();
    assert_eq!(
        texts,
        [
            "label-angle",
            "font-size",
            "font-weight",
            "label-angle",
            "offset"
        ]
    );
    assert_eq!(sheet.rules[3].conditions.len(), 1);
    error(
        "& > label { fill: red; }",
        "`&` is valid only in a nested rule",
    );
}

#[test]
fn values_with_var_or_env_wait_for_the_cascade() {
    let (sheet, diagnostics) = parse(
        "chart { --brand: teal; color: var(--brand); background: env(background, white); font-family: env(font-family, Lato, sans-serif); }",
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    let values: Vec<&Declared> = (sheet.rules[0].declarations.iter())
        .map(|declaration| &declaration.value)
        .collect();
    assert_eq!(values[0], &Declared::Tokens("teal".to_string()));
    let (reset, _) = parse("chart { --brand: initial; }");
    assert_eq!(
        reset.rules[0].declarations[0].value,
        Declared::Wide(avenger_style::Wide::Initial)
    );
    assert!(matches!(values[1], Declared::Pending { text, .. } if text == "var(--brand)"));
    let mut names: Vec<&str> = sheet.env_names().collect();
    names.sort();
    assert_eq!(names, ["background", "font-family"]);
    error(
        "chart { color: var(brand); }",
        "var() names a custom property",
    );
}

#[test]
fn shorthands_set_their_longhands() {
    let (sheet, diagnostics) = parse(
        ":root { padding: 5 10; }
         scale { padding: 0.5; }
         plot { border: 1 solid #ddd; gap: 4px 16px; }",
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    let properties = |rule: usize| -> Vec<Property> {
        (sheet.rules[rule].declarations.iter())
            .map(|declaration| match declaration.name {
                Name::Property(property) => property,
                Name::Custom(_) => panic!("a property"),
            })
            .collect()
    };
    assert_eq!(
        properties(0),
        [
            Property::PaddingTop,
            Property::PaddingRight,
            Property::PaddingBottom,
            Property::PaddingLeft
        ]
    );
    assert!(properties(1).contains(&Property::PaddingInner));
    assert!(properties(1).contains(&Property::PaddingOuter));
    assert_eq!(
        properties(2),
        [
            Property::BorderWidth,
            Property::BorderStyle,
            Property::BorderColor,
            Property::RowGap,
            Property::ColumnGap
        ]
    );
}

#[test]
fn important_declarations_keep_their_flag() {
    let (sheet, diagnostics) = parse("chart { color: red !important; background: white; }");
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    let flags: Vec<bool> = (sheet.rules[0].declarations.iter())
        .map(|declaration| declaration.important)
        .collect();
    assert_eq!(flags, [true, false]);
}

#[test]
fn malformed_text_reports_diagnostics_without_panicking() {
    for css in [
        "chart {",
        "}}}{{{",
        "chart { color: ; }",
        "chart { color: red",
        "@media",
        "@media (",
        "@container (400px <= width <",
        "@palette { kind: categorical; }",
        "@layer a. { }",
        "@import",
        "mark[type=",
        "mark:nth-child(2 of ) { fill: red }",
        "mark { & & & { fill: red } }",
        "chart { --x: { a; b }; color: var(--x, var(--y, red)); }",
        "chart { color: env(); }",
        "chart { font-family: \"unterminated }",
        "→ { fill: red } ünïcödé { color: blue }",
        "chart { padding: 1 2 3 4 5; gap: ; border: 1 1 1; }",
        "chart { color: color-mix(in srgb, red); }",
        "chart { color: calc(1 / ); size: calc(((1))); }",
    ] {
        let _ = parse(css);
        let _ = file(css);
    }
}

#[test]
fn ids_select_nodes_with_the_specificity_of_an_id() {
    let (sheet, diagnostics) = parse(
        "#sales { color: red; }
         plot#sales > mark.muted { color: red; }
         axis#time > label { color: red; }
         #\\31 st { color: red; }",
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    let specificities: Vec<u32> = (sheet.rules.iter())
        .map(|rule| rule.selectors.slice()[0].specificity())
        .collect();
    let (id, class, element) = (1 << 20, 1 << 10, 1);
    assert_eq!(
        specificities,
        [id, id + class + 2 * element, id + 2 * element, id]
    );

    // An id that is not a CSS identifier needs an escape.
    let digit = error("#1st { color: red; }", "`1st` is not a CSS identifier");
    assert_eq!(digit.help.as_deref(), Some("escape it, as in `#\\31 st`"));
    error(
        "plot > #2023.sales { color: red; }",
        "`#2023` is not an id selector",
    );
    error(
        "plot > { color: red; }",
        "a combinator needs a selector after it",
    );
}

#[test]
fn layer_statements_name_one_layer_or_several() {
    let (sheet, diagnostics) = file(
        "@layer theme;
         @layer base, brand;
         @import 'brand.css' layer(brand.colors);
         @layer extra;
         @layer { chart { color: red; } }
         @layer theme { chart { color: blue; } }",
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    let layers: Vec<String> = (sheet.layers.iter())
        .map(|layer| layer.name.join("."))
        .collect();
    assert_eq!(
        layers[..5],
        ["theme", "base", "brand", "brand.colors", "extra"]
    );
    // The import comes after the layers that the statements and its own layer declare.
    assert_eq!(sheet.imports[0].layers_before, 4);
    assert_eq!(sheet.rules[1].layer, Some(vec!["theme".to_string()]));

    error(
        "@layer a, b { chart { color: red; } }",
        "a layer block names one layer",
    );
    // A layer block is a rule, after which an import is too late.
    let (_, diagnostics) = file("@layer a { chart { color: red; } } @import 'late.css';");
    assert!(diagnostics[0]
        .message
        .contains("comes before every other rule"));
}

#[test]
fn focus_visible_is_checked_with_the_parent_in_place_of_the_nesting_selector() {
    for css in [
        "mark:focus-visible { & { outline-color: red; } }",
        ":is(mark:focus-visible) { outline-color: red; }",
        "mark:where(:focus-visible) { outline-color: red; }",
        "mark { &:focus-visible { outline: 2px solid red; } }",
        "legend { > entry:focus-visible { outline-offset: 2px; } }",
    ] {
        let (_, diagnostics) = parse(css);
        assert!(diagnostics.is_empty(), "{css}: {diagnostics:#?}");
    }
    error(
        "mark:focus-visible { & { fill: red; } }",
        "only outline properties apply through :focus-visible",
    );
    error(
        "mark:focus-visible { & > label { color: red; } }",
        ":focus-visible is valid only in the last compound selector",
    );
    error(
        ":is(mark:focus-visible, axis) { outline-color: red; }",
        "the selectors of a rule all end with :focus-visible, or none does",
    );
    error(
        "mark { & item { outline-color: red; } }",
        "`item` is a virtual node",
    );
}

#[test]
fn nested_selectors_without_the_nesting_selector_are_descendants() {
    let (sheet, diagnostics) = parse("axis { label, > tick { stroke: red; } }");
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    assert_eq!(
        sheet.rules[0].selectors.to_css_string(),
        ":is(axis) label, :is(axis) > tick"
    );
}

#[test]
fn values_with_error_tokens_are_invalid() {
    error(
        "chart { --x: blue; color: var(--x) ]; }",
        "unexpected `]` in a value",
    );
    error("chart { --x: a ) b; }", "unexpected `)` in a value");
    let (sheet, _) = parse("chart { color: red; color: var(--x) ]; }");
    assert_eq!(
        sheet.rules[0].declarations.len(),
        1,
        "the valid declaration stays"
    );
}

#[test]
fn property_names_are_case_insensitive_and_custom_property_names_are_not() {
    let (sheet, diagnostics) = parse("mark { FILL: red; Stroke-Width: 2; --Accent: blue; }");
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    let names: Vec<&Name> = (sheet.rules[0].declarations.iter())
        .map(|declaration| &declaration.name)
        .collect();
    assert_eq!(
        names,
        [
            &Name::Property(Property::Fill),
            &Name::Property(Property::StrokeWidth),
            &Name::Custom("--Accent".to_string()),
        ]
    );
}

#[test]
fn palette_descriptors_read_tokens() {
    let (sheet, diagnostics) = parse(
        "@palette brand {
           kind: categorical/**/;
           colors: red, blue;
           interpolate: /* a space */ OKLab;
           cvd-safe: TRUE;
         }",
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    let palette = &sheet.palettes[0];
    assert_eq!(palette.kind, PaletteKind::Categorical);
    assert_eq!(palette.interpolate, avenger_style::Interpolation::Oklab);
    assert!(palette.cvd_safe);
    error(
        "@palette brand { kind: categorical sequential; colors: red; }",
        "is not a valid value of `kind`",
    );
}

#[test]
fn compound_selectors_are_checked_with_the_alternatives_of_is_and_where() {
    for (css, message) in [
        (
            "title { &:focus-visible { outline-color: red; } }",
            "`title` cannot have focus",
        ),
        (
            "title:where(:focus-visible) { outline-color: red; }",
            "`title` cannot have focus",
        ),
        (
            "title { &[type=bar] { color: red; } }",
            "the element `title` has no attribute `type`",
        ),
        (
            "item:focus-visible { &.foo { outline-color: red; } }",
            "the virtual node `item` takes no selector other than :focus-visible",
        ),
        (
            ":is(mark, title):focus-visible { outline-color: red; }",
            "`title` cannot have focus",
        ),
    ] {
        error(css, message);
    }
    for css in [
        ":is(item):focus-visible { outline-color: red; }",
        ":is(mark, axis):focus-visible { outline-color: red; }",
        "mark:is([type=bar], [type=line]) { color: red; }",
        "axis { :is(label, tick) { color: red; } }",
    ] {
        let (_, diagnostics) = parse(css);
        assert!(diagnostics.is_empty(), "{css}: {diagnostics:#?}");
    }
}

#[test]
fn the_selectors_of_a_rule_all_need_focus_or_none_does() {
    let mixed = error(
        "mark:focus-visible, axis { outline: 2px solid red; }",
        "the selectors of a rule all end with :focus-visible, or none does",
    );
    assert_eq!(
        mixed.help.as_deref(),
        Some("write the selectors with :focus-visible as a rule of their own")
    );
    error(
        ":is(mark:focus-visible, axis) { color: red; }",
        "the selectors of a rule all end with :focus-visible, or none does",
    );
    error(
        "mark { &:focus-visible, & > label { color: red; } }",
        "the selectors of a rule all end with :focus-visible, or none does",
    );
}

#[test]
fn layer_declarations_keep_their_conditions() {
    let (sheet, diagnostics) = parse(
        "@media print { @layer a, b; }
         @layer b { chart { color: blue; } }
         @layer a { chart { color: red; } }",
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    let declarations: Vec<(String, usize)> = (sheet.layers.iter())
        .map(|layer| (layer.name.join("."), layer.conditions.len()))
        .collect();
    assert_eq!(
        declarations,
        [
            ("a".to_string(), 1),
            ("b".to_string(), 1),
            ("b".to_string(), 0),
            ("a".to_string(), 0)
        ]
    );
}

#[test]
fn container_units_are_errors_that_point_to_container_conditions() {
    for css in [
        "mark { stroke-width: 1cqw; }",
        "chart { font-size: calc(1rem + 2cqmin); }",
        "chart { padding: 2cqh; }",
    ] {
        let invalid = error(css, "is not a valid value of");
        assert_eq!(
            invalid.help.as_deref(),
            Some("container units are not supported; change the value at a breakpoint with @container"),
            "{css}"
        );
    }
}
