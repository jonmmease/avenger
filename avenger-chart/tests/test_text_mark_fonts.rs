//! Test text mark font family inheritance

use avenger_chart::prelude::*;
use datafusion_common::ScalarValue;

#[test]
fn test_text_mark_font_family_defaults() {
    // Test 1: Text marks inherit base font family by default
    let theme = StructTheme::default().with_font_family("CustomBase");

    // Simulate getting the default font for a text mark
    let text_font = theme.mark_defaults.get_with_computed_fonts(
        "text",
        "font",
        12.0, // font size doesn't matter for this test
        &theme.base_font_family,
    );

    assert_eq!(
        text_font,
        Some(ScalarValue::Utf8(Some("CustomBase".to_string())))
    );

    // Test 2: Text marks can have their font family overridden
    let theme_with_override = theme.clone().with_text_mark_font_family("TextMarkFont");

    let text_font_override = theme_with_override.mark_defaults.get_with_computed_fonts(
        "text",
        "font",
        12.0,
        &theme_with_override.base_font_family,
    );

    assert_eq!(
        text_font_override,
        Some(ScalarValue::Utf8(Some("TextMarkFont".to_string())))
    );
}

#[test]
fn test_text_mark_font_size_computed() {
    let theme = StructTheme::default().with_font_size(16.0); // Base font size

    // Text mark font size should be computed from base * scale
    let text_font_size = theme.mark_defaults.get_with_computed_fonts(
        "text",
        "font_size",
        theme.text_mark_font_size(),
        &theme.base_font_family,
    );

    // text_mark_font_size = 16.0 * 1.0 (text_mark_scale)
    assert_eq!(text_font_size, Some(ScalarValue::Float32(Some(16.0))));
}

#[test]
fn test_text_mark_inherits_after_font_family_reset() {
    // When base font family is changed, text marks should inherit the new font
    let mut theme = StructTheme::default()
        .with_font_family("OldFont")
        .with_text_mark_font_family("CustomTextFont");

    // Text mark has custom font
    let text_font_before =
        theme
            .mark_defaults
            .get_with_computed_fonts("text", "font", 12.0, &theme.base_font_family);
    assert_eq!(
        text_font_before,
        Some(ScalarValue::Utf8(Some("CustomTextFont".to_string())))
    );

    // Reset base font family (which clears overrides)
    theme.set_font_family("NewFont");

    // Text mark should now inherit from new base
    let text_font_after =
        theme
            .mark_defaults
            .get_with_computed_fonts("text", "font", 12.0, &theme.base_font_family);
    assert_eq!(
        text_font_after,
        Some(ScalarValue::Utf8(Some("NewFont".to_string())))
    );
}

#[test]
fn test_font_hierarchy_complete() {
    // Test the complete hierarchy of font family inheritance
    let theme = StructTheme::default()
        .with_font_family("BaseFont")
        .with_title_font_family("TitleFont")
        .with_axis_title_font_family("AxisTitleFont")
        .with_legend_title_font_family("LegendTitleFont")
        .with_text_mark_font_family("TextMarkFont");

    // Each component should have its own font
    assert_eq!(theme.title_font_family(), "TitleFont");
    assert_eq!(theme.axis_title_font_family(), "AxisTitleFont");
    assert_eq!(theme.legend_title_font_family(), "LegendTitleFont");

    // Text marks should use their override
    let text_font =
        theme
            .mark_defaults
            .get_with_computed_fonts("text", "font", 12.0, &theme.base_font_family);
    assert_eq!(
        text_font,
        Some(ScalarValue::Utf8(Some("TextMarkFont".to_string())))
    );

    // Components without overrides inherit from base
    assert_eq!(theme.subtitle_font_family(), "BaseFont");
    assert_eq!(theme.axis_label_font_family(), "BaseFont");
}
