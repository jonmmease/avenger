//! Tests for the theme system

use avenger_chart::prelude::*;
use datafusion_common::ScalarValue;

#[test]
fn test_default_theme() {
    let theme = StructTheme::default();

    // Check default values
    assert_eq!(theme.base_font_family, "Atkinson Hyperlegible Next");
    assert_eq!(theme.base_font_size, 12.0);
    assert_eq!(theme.title_font_size(), 18.0);
    assert_eq!(theme.colors.default_color, "#4682b4");
}

#[test]
fn test_theme_builder() {
    let theme = StructTheme::default().with_font_family("Inter");
    assert_eq!(theme.base_font_family, "Inter");
}

#[test]
fn test_preset_themes() {
    // Test dark theme
    let dark = StructTheme::dark();
    assert_eq!(dark.background.plot_background, Some("#1E1E1E".to_string()));
    assert_eq!(
        dark.background.canvas_background,
        Some("#121212".to_string())
    );

    // Test light theme (should be same as default)
    let light = StructTheme::light();
    assert_eq!(light.base_font_family, "Atkinson Hyperlegible Next");

    // Test high contrast theme
    let high_contrast = StructTheme::high_contrast();
    assert_eq!(high_contrast.colors.default_color, "#000000");

    // Test colorblind-safe theme
    let colorblind = StructTheme::colorblind_safe();
    assert_eq!(colorblind.colors.categorical[0], "#0072B2");

    // Test publication theme
    let publication = StructTheme::publication();
    assert_eq!(publication.base_font_family, "Helvetica");
    assert_eq!(publication.base_font_size, 10.0);
    assert_eq!(publication.title_font_size(), 13.0); // 10.0 * 1.333 (compact scale) = 13.33, rounded to 13
}

#[test]
fn test_mark_defaults() {
    let theme = StructTheme::default();

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
    let theme = StructTheme::default()
        .set_mark_default(
            "symbol",
            "fill",
            ScalarValue::Utf8(Some("#FF6B6B".to_string())),
        )
        .set_mark_default("symbol", "size", ScalarValue::Float32(Some(100.0)));

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
        .theme(StructTheme::dark())
        .title("Dark Theme Plot")
        .mark(Symbol::new().x(col("x")).y(col("y")));

    // This test just ensures the API works correctly
}

#[test]
fn test_font_family_overrides() {
    // Test that we can override specific font families
    let theme = StructTheme::default()
        .with_font_family("Inter") // Base font
        .with_title_font_family("Georgia")
        .with_subtitle_font_family("Helvetica")
        .with_axis_title_font_family("Courier")
        .with_axis_label_font_family("Arial")
        .with_legend_title_font_family("Times")
        .with_legend_label_font_family("Verdana")
        .with_legend_tick_font_family("Monaco");

    // Check base font
    assert_eq!(theme.base_font_family, "Inter");

    // Check computed font families with overrides
    assert_eq!(theme.title_font_family(), "Georgia");
    assert_eq!(theme.subtitle_font_family(), "Helvetica");
    assert_eq!(theme.axis_title_font_family(), "Courier");
    assert_eq!(theme.axis_label_font_family(), "Arial");
    assert_eq!(theme.legend_title_font_family(), "Times");
    assert_eq!(theme.legend_label_font_family(), "Verdana");
    assert_eq!(theme.legend_tick_font_family(), "Monaco");
}

#[test]
fn test_font_family_inheritance() {
    // Test that components inherit from base when not overridden
    let theme = StructTheme::default()
        .with_font_family("CustomFont")
        .with_title_font_family("TitleFont"); // Only override title

    assert_eq!(theme.base_font_family, "CustomFont");
    assert_eq!(theme.title_font_family(), "TitleFont");

    // These should inherit from base
    assert_eq!(theme.subtitle_font_family(), "CustomFont");
    assert_eq!(theme.axis_title_font_family(), "CustomFont");
    assert_eq!(theme.axis_label_font_family(), "CustomFont");
    assert_eq!(theme.legend_title_font_family(), "CustomFont");
    assert_eq!(theme.legend_label_font_family(), "CustomFont");
    assert_eq!(theme.legend_tick_font_family(), "CustomFont");
}
