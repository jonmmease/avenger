//! Tests for the theme system

use avenger_chart::prelude::*;
use datafusion_common::ScalarValue;

#[test]
fn test_default_theme() {
    let plot = Plot::<Cartesian>::new();
    let theme = plot.get_theme();

    // Check default values
    assert_eq!(theme.default_font, "Atkinson Hyperlegible Next");
    assert_eq!(theme.title.title_font_size, 18.0);
    assert_eq!(theme.colors.default_color, "#4682b4");
}

#[test]
fn test_theme_builder() {
    let plot = Plot::<Cartesian>::new().with_theme(|t| t.with_font_family("Inter"));

    let theme = plot.get_theme();
    assert_eq!(theme.default_font, "Inter");
}

#[test]
fn test_preset_themes() {
    // Test dark theme
    let dark = Theme::dark();
    assert_eq!(dark.background.plot_background, Some("#1E1E1E".to_string()));
    assert_eq!(
        dark.background.canvas_background,
        Some("#121212".to_string())
    );

    // Test light theme (should be same as default)
    let light = Theme::light();
    assert_eq!(light.default_font, "Atkinson Hyperlegible Next");

    // Test high contrast theme
    let high_contrast = Theme::high_contrast();
    assert_eq!(high_contrast.colors.default_color, "#000000");

    // Test colorblind-safe theme
    let colorblind = Theme::colorblind_safe();
    assert_eq!(colorblind.colors.categorical[0], "#0072B2");

    // Test publication theme
    let publication = Theme::publication();
    assert_eq!(publication.default_font, "Helvetica");
    assert_eq!(publication.title.title_font_size, 14.0);
}

#[test]
fn test_mark_defaults() {
    let theme = Theme::default();

    // Test symbol defaults
    assert_eq!(
        theme.mark_defaults.get("symbol", "fill"),
        Some(&ScalarValue::Utf8(Some("#4682b4".to_string())))
    );

    // Test rect defaults
    assert_eq!(
        theme.mark_defaults.get("rect", "stroke_width"),
        Some(&ScalarValue::Float32(Some(1.0)))
    );

    // Test line defaults
    assert_eq!(
        theme.mark_defaults.get("line", "stroke_width"),
        Some(&ScalarValue::Float32(Some(2.0)))
    );
}

#[test]
fn test_custom_mark_defaults() {
    let plot = Plot::<Cartesian>::new().with_theme(|t| {
        t.set_mark_default(
            "symbol",
            "fill",
            ScalarValue::Utf8(Some("#FF6B6B".to_string())),
        )
        .set_mark_default("symbol", "size", ScalarValue::Float32(Some(100.0)))
    });

    let theme = plot.get_theme();
    assert_eq!(
        theme.mark_defaults.get("symbol", "fill"),
        Some(&ScalarValue::Utf8(Some("#FF6B6B".to_string())))
    );
    assert_eq!(
        theme.mark_defaults.get("symbol", "size"),
        Some(&ScalarValue::Float32(Some(100.0)))
    );
}

#[test]
fn test_theme_with_plot() {
    let _plot = Plot::<Cartesian>::new()
        .theme(Theme::dark())
        .title("Dark Theme Plot")
        .mark(Symbol::new().x(col("x")).y(col("y")));

    // This test just ensures the API works correctly
}
