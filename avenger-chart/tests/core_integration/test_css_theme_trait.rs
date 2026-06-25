//! Tests for CSS Theme

use avenger_chart::theme::Theme;
use avenger_chart::theme::{ThemeContext, ThemeValue};
use indexmap::IndexMap;

#[test]
fn test_css_theme_implements_trait() {
    let css = r#"
        mark { fill: #4c78a8; stroke: #000; stroke-width: 2px; }
        mark[type="symbol"] { size: 100px; }
        axis { color: #333; font-size: 14px; }
        axis.label { font-weight: 300; }
        legend.title { font-size: 16px; font-weight: bold; }
    "#;

    let theme = Theme::from_css(css).unwrap();

    // Test basic mark styling
    let mark_context = ThemeContext::new("mark", IndexMap::new());
    let fill = theme.query(&mark_context, "fill");
    match fill {
        Some(ThemeValue::Color(rgba)) => {
            // Color #4c78a8 (steel blue)
            assert_eq!(rgba.red, 0x4c, "Fill color red component should be 0x4c");
            assert_eq!(
                rgba.green, 0x78,
                "Fill color green component should be 0x78"
            );
            assert_eq!(rgba.blue, 0xa8, "Fill color blue component should be 0xa8");
        }
        Some(ThemeValue::String(s)) => {
            assert!(s.starts_with("#"), "String color should start with #")
        }
        _ => panic!("Expected color value for fill, got {:?}", fill),
    }

    let stroke_width = theme.query(&mark_context, "stroke-width");
    assert!(matches!(stroke_width, Some(ThemeValue::Length(2.0, _))));

    // Test mark with type attribute
    let symbol_context = ThemeContext::new("mark", IndexMap::new()).with_subtype("symbol");
    let size = theme.query(&symbol_context, "size");
    assert!(matches!(size, Some(ThemeValue::Length(100.0, _))));

    // Test axis styling
    let axis_context = ThemeContext::new("axis", IndexMap::new());
    let color = theme.query(&axis_context, "color");
    match color {
        Some(ThemeValue::Color(rgba)) => {
            // Color #333 (expanded to #333333)
            assert_eq!(rgba.red, 0x33, "Axis color red component should be 0x33");
            assert_eq!(
                rgba.green, 0x33,
                "Axis color green component should be 0x33"
            );
            assert_eq!(rgba.blue, 0x33, "Axis color blue component should be 0x33");
        }
        Some(ThemeValue::String(s)) => {
            assert_eq!(s, "#333333", "Axis color string should be #333333")
        }
        _ => panic!("Expected color value for axis color, got {:?}", color),
    }

    let font_size = theme.query(&axis_context, "font-size");
    assert!(
        matches!(font_size, Some(ThemeValue::Length(14.0, _))),
        "Axis font-size should be 14px"
    );

    // Test axis with class
    let label_context = ThemeContext::new("axis", IndexMap::new()).with_class("label");
    let font_weight = theme.query(&label_context, "font-weight");
    assert!(
        matches!(font_weight, Some(ThemeValue::Number(n)) if n == 300.0),
        "Axis label font-weight should be 300"
    );

    // Test legend with class
    let legend_title_context = ThemeContext::new("legend", IndexMap::new()).with_class("title");
    let title_size = theme.query(&legend_title_context, "font-size");
    assert!(
        matches!(title_size, Some(ThemeValue::Length(16.0, _))),
        "Legend title font-size should be 16px"
    );
}

#[test]
fn test_theme_trait_methods() {
    let css = r#"
        text { font-family: "Helvetica Neue"; font-size: 12px; }
        axis { color: #666; }
        mark { fill: steelblue; opacity: 0.8; }
    "#;

    let theme = Theme::from_css(css).unwrap();

    // Test helper methods from Theme trait
    let text_context = ThemeContext::new("text", IndexMap::new());
    assert_eq!(
        theme.font_family(&text_context),
        Some("Helvetica Neue".to_string())
    );
    assert_eq!(theme.font_size(&text_context), Some(12.0));

    let axis_context = ThemeContext::new("axis", IndexMap::new());
    // Colors are now returned as [f32; 4] arrays
    let axis_color = theme.text_color(&axis_context).unwrap();
    assert!((axis_color[0] - 0.4).abs() < 0.01); // ~102/255 = 0.4
    assert!((axis_color[1] - 0.4).abs() < 0.01);
    assert!((axis_color[2] - 0.4).abs() < 0.01);

    let mark_context = ThemeContext::new("mark", IndexMap::new());
    let fill = theme.fill_color(&mark_context).unwrap();
    // Steelblue #4682b4 = rgb(70, 130, 180)
    assert!((fill[0] - 70.0 / 255.0).abs() < 0.01);
    assert!((fill[1] - 130.0 / 255.0).abs() < 0.01);
    assert!((fill[2] - 180.0 / 255.0).abs() < 0.01);
    assert_eq!(theme.opacity(&mark_context), Some(0.8));
}

#[test]
fn test_clone_box() {
    let css = r#"mark { fill: green; }"#;
    let theme = Theme::from_css(css).unwrap();

    // Test that clone works
    let cloned = theme.clone();
    let context = ThemeContext::new("mark", IndexMap::new());
    let fill = cloned.query(&context, "fill");

    match fill {
        Some(ThemeValue::Color(rgba)) => {
            // Green color (#008000)
            assert_eq!(rgba.green, 128, "Cloned theme should have green fill");
        }
        Some(ThemeValue::String(s)) => {
            assert!(s.starts_with("#"), "String color should start with #")
        }
        _ => panic!("Expected color value for cloned theme fill, got {:?}", fill),
    }
}
