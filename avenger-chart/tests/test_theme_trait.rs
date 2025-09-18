//! Tests for the theme trait system

use avenger_chart::theme::{
    ContextBuilder, StructTheme, Theme, ThemeContext, ThemeProperty, ThemeValue,
};

#[test]
fn test_theme_trait_queries() {
    let theme = StructTheme::default();

    // Test title queries
    let title_ctx = ThemeContext::new("title");
    assert!(theme.font_family(&title_ctx).contains("Atkinson"));
    assert_eq!(theme.font_size(&title_ctx), 18.0); // 12 * 1.5

    // Test subtitle queries
    let subtitle_ctx = ThemeContext::new("subtitle");
    assert_eq!(theme.font_size(&subtitle_ctx), 14.0); // 12 * 1.167

    // Test axis label queries
    let axis_label_ctx = ThemeContext::new("axis").with_class("label");
    assert_eq!(theme.font_size(&axis_label_ctx), 10.0); // 12 * 0.833
}

#[test]
fn test_theme_trait_with_overrides() {
    let theme = StructTheme::default()
        .with_font_family("CustomFont")
        .with_title_font_family("TitleFont");

    let title_ctx = ThemeContext::new("title");
    assert_eq!(theme.font_family(&title_ctx), "TitleFont");

    let subtitle_ctx = ThemeContext::new("subtitle");
    assert_eq!(theme.font_family(&subtitle_ctx), "CustomFont"); // Inherits from base
}

#[test]
fn test_mark_queries() {
    let theme = StructTheme::default();

    // Test symbol mark defaults
    let symbol_ctx = ThemeContext::new("mark").with_mark("symbol");
    let fill = theme.query(&symbol_ctx, &ThemeProperty::FillColor);
    if let ThemeValue::String(color) = fill {
        assert_eq!(color, "#4682b4");
    } else {
        panic!("Expected string value for fill color");
    }

    // Test text mark font size
    let text_ctx = ThemeContext::new("mark").with_mark("text");
    assert_eq!(theme.font_size(&text_ctx), 12.0); // Should equal base * text_scale
}

#[test]
fn test_legend_queries() {
    let theme = StructTheme::default();

    // Test legend title
    let legend_title_ctx = ThemeContext::new("legend").with_class("title");
    assert_eq!(theme.font_size(&legend_title_ctx), 12.0); // 12 * 1.0

    // Test legend label
    let legend_label_ctx = ThemeContext::new("legend").with_class("label");
    assert_eq!(theme.font_size(&legend_label_ctx), 11.0); // 12 * 0.917
}

#[test]
fn test_axis_grid_properties() {
    let theme = StructTheme::default();

    let axis_ctx = ThemeContext::new("axis");
    let grid_color = theme.query(&axis_ctx, &ThemeProperty::GridColor);
    if let ThemeValue::String(color) = grid_color {
        assert_eq!(color, "#e0e0e0");
    }

    let grid_opacity = theme.query(&axis_ctx, &ThemeProperty::GridOpacity);
    if let ThemeValue::Float(opacity) = grid_opacity {
        assert_eq!(opacity, 0.5);
    }
}

#[test]
fn test_context_builder_helpers() {
    // Use helper methods to build contexts
    let axis_x = ThemeContext::axis_context("x");
    assert_eq!(axis_x.element_type, "axis");
    assert!(axis_x.classes.contains(&"x".to_string()));

    let legend_discrete = ThemeContext::legend_context("discrete");
    assert_eq!(legend_discrete.element_type, "legend");
    assert!(legend_discrete.classes.contains(&"discrete".to_string()));

    let mark_symbol = ThemeContext::mark_context("symbol");
    assert_eq!(mark_symbol.element_type, "mark");
    assert_eq!(mark_symbol.subtype, Some("symbol".to_string()));
}

#[test]
fn test_theme_value_conversions() {
    let string_val = ThemeValue::String("test".to_string());
    assert_eq!(string_val.as_string(), Some("test"));
    assert_eq!(string_val.as_float(), None);

    let float_val = ThemeValue::Float(12.5);
    assert_eq!(float_val.as_float(), Some(12.5));
    assert_eq!(float_val.as_integer(), Some(12));

    let int_val = ThemeValue::Integer(42);
    assert_eq!(int_val.as_integer(), Some(42));
    assert_eq!(int_val.as_float(), Some(42.0));

    let bool_val = ThemeValue::Boolean(true);
    assert_eq!(bool_val.as_bool(), Some(true));

    let none_val = ThemeValue::None;
    assert_eq!(none_val.as_string(), None);
}

#[test]
fn test_query_with_fallback() {
    let theme = StructTheme::default();
    let ctx = ThemeContext::new("unknown_element");

    // Query a property that doesn't exist, with fallback
    let value = theme.query_or(
        &ctx,
        &ThemeProperty::Custom("nonexistent".to_string()),
        ThemeValue::String("fallback".to_string()),
    );

    if let ThemeValue::String(s) = value {
        assert_eq!(s, "fallback");
    } else {
        panic!("Expected fallback value");
    }
}
