//! Tests for CSS Theme implementing the Theme trait

use avenger_chart::theme::css::CssTheme;
use avenger_chart::theme::{Theme, ThemeContext, ThemeProperty, ThemeValue};

#[test]
fn test_css_theme_implements_trait() {
    let css = r#"
        mark { fill: #4c78a8; stroke: #000; stroke-width: 2px; }
        mark[type="symbol"] { size: 100px; }
        axis { color: #333; font-size: 14px; }
        axis.label { font-weight: 300; }
        legend.title { font-size: 16px; font-weight: bold; }
    "#;

    let theme = CssTheme::from_css(css).unwrap();

    // Test basic mark styling
    let mark_context = ThemeContext::new("mark");
    let fill = theme.query(&mark_context, &ThemeProperty::FillColor);
    match fill {
        ThemeValue::Color(rgba) => {
            // Color #4c78a8
            assert_eq!(rgba.red, 0x4c);
            assert_eq!(rgba.green, 0x78);
            assert_eq!(rgba.blue, 0xa8);
        }
        ThemeValue::String(s) => assert!(s.starts_with("#")),
        _ => panic!("Expected color value"),
    }

    let stroke_width = theme.query(&mark_context, &ThemeProperty::StrokeWidth);
    assert!(matches!(stroke_width, ThemeValue::Length(2.0, _)));

    // Test mark with type attribute
    let symbol_context = ThemeContext::new("mark").with_subtype("symbol");
    let size = theme.query(&symbol_context, &ThemeProperty::Size);
    assert!(matches!(size, ThemeValue::Length(100.0, _)));

    // Test axis styling
    let axis_context = ThemeContext::new("axis");
    let color = theme.query(&axis_context, &ThemeProperty::Color);
    match color {
        ThemeValue::Color(rgba) => {
            // Color #333 (expanded to #333333)
            assert_eq!(rgba.red, 0x33);
            assert_eq!(rgba.green, 0x33);
            assert_eq!(rgba.blue, 0x33);
        }
        ThemeValue::String(s) => assert_eq!(s, "#333333"),
        _ => panic!("Expected color value"),
    }

    let font_size = theme.query(&axis_context, &ThemeProperty::FontSize);
    assert!(matches!(font_size, ThemeValue::Length(14.0, _)));

    // Test axis with class
    let label_context = ThemeContext::new("axis").with_class("label");
    let font_weight = theme.query(&label_context, &ThemeProperty::FontWeight);
    assert!(matches!(font_weight, ThemeValue::Double(300.0)));

    // Test legend with class
    let legend_title_context = ThemeContext::new("legend").with_class("title");
    let title_size = theme.query(&legend_title_context, &ThemeProperty::FontSize);
    assert!(matches!(title_size, ThemeValue::Length(16.0, _)));
}

#[test]
fn test_theme_trait_methods() {
    let css = r#"
        text { font-family: "Helvetica Neue"; font-size: 12px; }
        axis { color: #666; }
        mark { fill: steelblue; opacity: 0.8; }
    "#;

    let theme = CssTheme::from_css(css).unwrap();

    // Test helper methods from Theme trait
    let text_context = ThemeContext::new("text");
    assert_eq!(theme.font_family(&text_context), "Helvetica Neue");
    assert_eq!(theme.font_size(&text_context), 12.0);

    let axis_context = ThemeContext::new("axis");
    assert_eq!(theme.color(&axis_context), "#666666");

    let mark_context = ThemeContext::new("mark");
    assert_eq!(theme.fill_color(&mark_context), "#4682b4"); // steelblue hex
    assert_eq!(theme.opacity(&mark_context), 0.8);
}

#[test]
fn test_mark_defaults() {
    let css = r#"
        mark[type="symbol"] { fill: red; size: 50px; }
        mark[type="rect"] { fill: blue; stroke: black; stroke-width: 1px; }
        mark[type="text"] { font-family: Arial; font-size: 14px; }
    "#;

    let theme = CssTheme::from_css(css).unwrap();

    // Test mark defaults
    let symbol_fill = theme.mark_default("symbol", "fill");
    assert!(symbol_fill.is_some());

    let rect_stroke_width = theme.mark_default("rect", "stroke_width");
    match rect_stroke_width {
        Some(datafusion_common::ScalarValue::Float32(Some(w))) => assert_eq!(w, 1.0),
        _ => panic!("Expected stroke width"),
    }

    // Test text mark with computed fonts
    let text_font = theme.mark_default_with_computed_fonts("text", "font", 16.0, "System");
    match text_font {
        Some(datafusion_common::ScalarValue::Utf8(Some(f))) => assert_eq!(f, "System"),
        _ => panic!("Expected font family"),
    }
}

#[test]
fn test_clone_box() {
    let css = r#"mark { fill: green; }"#;
    let theme = CssTheme::from_css(css).unwrap();

    // Test that clone_box works
    let cloned: Box<dyn Theme> = theme.clone_box();
    let context = ThemeContext::new("mark");
    let fill = cloned.query(&context, &ThemeProperty::FillColor);

    match fill {
        ThemeValue::Color(rgba) => {
            // Green color
            assert_eq!(rgba.green, 128);
        }
        ThemeValue::String(s) => assert!(s.starts_with("#")),
        _ => panic!("Expected color value"),
    }
}

#[test]
fn test_categorical_colors_and_shapes() {
    let css = r#""#; // Empty CSS, will use defaults
    let theme = CssTheme::from_css(css).unwrap();

    // Test categorical colors
    let colors = theme.categorical_colors();
    assert!(!colors.is_empty());
    assert!(colors[0].starts_with("#"));

    // Test shape names
    let shapes = theme.shape_names();
    assert!(!shapes.is_empty());
    assert!(shapes.contains(&"circle".to_string()));

    // Test dash names
    let dashes = theme.dash_names();
    assert!(!dashes.is_empty());
    assert!(dashes.contains(&"solid".to_string()));
}
