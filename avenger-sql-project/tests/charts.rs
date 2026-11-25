//! What the charts of a project share: the order of the base stylesheet, the project's, and an
//! extra one in the cascade, the reader's color scheme, and the background of the scheme.

use avenger_sql_project::charts::preferences;
use avenger_style::chart_cascade;
use avenger_style::{
    ColorScheme, Element, HostInputs, ParseOptions, Property, Sheet, SourceId, StyleTree,
};

fn sheet(css: &str) -> Sheet {
    let mut no_files = |url: &str, _: Option<&str>| Err(format!("no file `{url}`"));
    let (sheet, diagnostics) = Sheet::load(
        css,
        None,
        SourceId(7),
        ParseOptions::default(),
        &mut no_files,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    sheet
}

/// The computed color of a bar of a chart, for a reader who prefers `scheme`, on the scheme's
/// background, styled by `sheets` after the base stylesheet, as `#rrggbb`.
fn bar_color(scheme: ColorScheme, sheets: &[&Sheet]) -> String {
    let mut tree = StyleTree::new(Element::Chart);
    let plot = tree.add(tree.root(), Element::Plot);
    tree.set_attribute(plot, "type", "cartesian");
    let bar = tree.add(plot, Element::Mark);
    tree.set_attribute_forms(bar, "type", "bar", "bar");
    let host = HostInputs {
        preferences: preferences(scheme),
        ..HostInputs::default()
    };
    let cascade = chart_cascade(scheme.background(), false, sheets.iter().copied());
    let computed = cascade.compute(&tree, &host);
    let color = computed[bar].color(Property::Color).expect("a color");
    let [r, g, b] = [0, 1, 2].map(|channel| (color[channel] * 255.0).round() as u8);
    format!("#{r:02x}{g:02x}{b:02x}")
}

#[test]
fn later_stylesheets_win_and_the_color_scheme_applies() {
    let project = sheet(
        "mark { color: #ff0000; } @media (prefers-color-scheme: dark) { mark { color: #00ff00; } }",
    );
    let extra = sheet("mark { color: #0000ff; }");
    assert_eq!(bar_color(ColorScheme::Light, &[&project]), "#ff0000");
    assert_eq!(bar_color(ColorScheme::Dark, &[&project]), "#00ff00");
    assert_eq!(
        bar_color(ColorScheme::Light, &[&project, &extra]),
        "#0000ff"
    );
}

#[test]
fn the_background_of_the_color_scheme_is_env_background() {
    let project = sheet("mark { color: env(background); }");
    assert_eq!(bar_color(ColorScheme::Dark, &[&project]), "#1e1e1e");
    assert_eq!(bar_color(ColorScheme::Light, &[&project]), "#ffffff");
}
