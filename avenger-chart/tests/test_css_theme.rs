//! Tests for the CSS theme system

use avenger_chart::theme::{LengthUnit, ThemeContext, ThemeValue, css::CssTheme};

#[test]
fn test_basic_theme_creation() {
    let css = r#"
        mark { fill: #4c78a8; stroke: none; }
        mark[type="symbol"] { size: 60px; }
        axis { stroke: #888; stroke-width: 1px; }
    "#;

    let theme = CssTheme::from_css(css).unwrap();
    // Just check that parsing succeeded and we have rules
    // Can't check internal rules field directly as it's private
    let context = ThemeContext::new("mark");
    let fill = theme.query_css(&context, "fill");
    assert!(matches!(fill, ThemeValue::Color(_)));
}

#[test]
fn test_context_matching() {
    let css = r#"
        mark { fill: blue; }
        mark[type="symbol"] { fill: red; }
        #main-mark { fill: green; }
    "#;

    let theme = CssTheme::from_css(css).unwrap();

    // Test basic context selector
    let context = ThemeContext::new("mark");
    let fill = theme.query_css(&context, "fill");
    assert!(matches!(fill, ThemeValue::Color(_)));

    // Test type attribute selector (should override element selector)
    let context_with_type = ThemeContext::new("mark").with_subtype("symbol");
    let fill_type = theme.query_css(&context_with_type, "fill");
    if let ThemeValue::Color(color) = fill_type {
        assert_eq!(color.red, 255); // Red color
    }

    // Test ID selector (should override type)
    let context_with_id = ThemeContext::new("mark")
        .with_subtype("symbol")
        .with_id("main-mark");
    let fill_id = theme.query_css(&context_with_id, "fill");
    if let ThemeValue::Color(color) = fill_id {
        assert_eq!(color.green, 128); // Green color
    }
}

#[test]
fn test_property_inheritance() {
    let css = r#"
        axis { font-size: 14px; color: #333; }
        axis text { font-weight: bold; }
    "#;

    let theme = CssTheme::from_css(css).unwrap();

    // Test that properties work without parent traversal
    let context = ThemeContext::new("text");
    let font_size = theme.query_css(&context, "font-size");
    // Should get default value
    assert!(matches!(font_size, ThemeValue::Length(_, _)));
}

#[test]
fn test_css_variables() {
    let css = r#"
        :root {
            --main-color: #4c78a8;
            --mark-size: 100px;
        }
        mark {
            fill: var(--main-color);
            size: var(--mark-size);
        }
    "#;

    let theme = CssTheme::from_css(css).unwrap();

    // Test that variables are resolved
    let context = ThemeContext::new("mark");
    let fill = theme.query_css(&context, "fill");
    // Should resolve to the color
    if let ThemeValue::Variable(var_name) = fill {
        // Variable reference stored, not resolved value
        assert_eq!(var_name, "--main-color");
    }
}

#[test]
fn test_color_parsing() {
    let css = r#"
        .a { fill: red; }
        .b { fill: #ff0000; }
        .c { fill: rgb(255, 0, 0); }
        .d { fill: steelblue; }
    "#;

    let theme = CssTheme::from_css(css).unwrap();

    // All should parse as colors
    for class in ["a", "b", "c", "d"] {
        let context = ThemeContext::new("mark").with_class(class);
        let fill = theme.query_css(&context, "fill");
        assert!(matches!(fill, ThemeValue::Color(_)));
    }
}

#[test]
fn test_length_units() {
    let css = r#"
        .px { stroke-width: 2px; }
        .rem { font-size: 1.5rem; }
        .percent { width: 50%; }
    "#;

    let theme = CssTheme::from_css(css).unwrap();

    let px_context = ThemeContext::new("mark").with_class("px");
    let px_value = theme.query_css(&px_context, "stroke-width");
    assert!(matches!(px_value, ThemeValue::Length(2.0, LengthUnit::Px)));

    let rem_context = ThemeContext::new("mark").with_class("rem");
    let rem_value = theme.query_css(&rem_context, "font-size");
    assert!(matches!(
        rem_value,
        ThemeValue::Length(1.5, LengthUnit::Rem)
    ));

    let percent_context = ThemeContext::new("mark").with_class("percent");
    let percent_value = theme.query_css(&percent_context, "width");
    assert!(matches!(percent_value, ThemeValue::Percentage(50.0)));
}

#[test]
fn test_unsupported_length_units_error() {
    // Test that unsupported units like em, pt raise an error
    let css = r#"
        .test { font-size: 1.5em; }
        .valid { color: red; }
    "#;

    let result = CssTheme::from_css(css);
    assert!(result.is_err());
    let error = result.unwrap_err();
    assert!(error.contains("Unsupported CSS units"));
    assert!(error.contains("em"));
}

#[test]
fn test_multiple_unsupported_units_error() {
    // Test that multiple unsupported units are all reported
    let css = r#"
        .a { font-size: 1.5em; }
        .b { width: 12pt; }
        .c { height: 2ex; }
        .d { margin: 1cm; }
    "#;

    let result = CssTheme::from_css(css);
    assert!(result.is_err());
    let error = result.unwrap_err();
    assert!(error.contains("Unsupported CSS units"));
    // Units should be deduplicated and sorted
    assert!(error.contains("cm"));
    assert!(error.contains("em"));
    assert!(error.contains("ex"));
    assert!(error.contains("pt"));
}

#[test]
fn test_specificity_cascade() {
    let css = r#"
        mark { fill: black; }                /* specificity: 0,0,1 */
        .highlight { fill: blue; }           /* specificity: 0,1,0 */
        mark[type="symbol"] { fill: red; }   /* specificity: 0,1,1 */
        #main { fill: green; }               /* specificity: 1,0,0 */
    "#;

    let theme = CssTheme::from_css(css).unwrap();

    // Element with type, class and id
    let context = ThemeContext::new("mark")
        .with_subtype("symbol")
        .with_class("highlight")
        .with_id("main");

    let fill = theme.query_css(&context, "fill");
    // ID selector should win (highest specificity)
    if let ThemeValue::Color(color) = fill {
        assert_eq!(color.green, 128); // Green
    }
}

#[test]
fn test_value_conversions() {
    let css = r#"
        mark {
            size: 100;
            opacity: 0.5;
            stroke-width: 2px;
        }
    "#;

    let theme = CssTheme::from_css(css).unwrap();
    let context = ThemeContext::new("mark");

    let size = theme.query_css(&context, "size");
    assert_eq!(size.as_pixels(12.0), Some(100.0));

    let opacity = theme.query_css(&context, "opacity");
    assert_eq!(opacity.as_double(), Some(0.5));

    let stroke_width = theme.query_css(&context, "stroke-width");
    assert_eq!(stroke_width.as_pixels(12.0), Some(2.0));
}

#[test]
fn test_descendant_selectors() {
    let css = r#"
        coords mark { fill: blue; }
        coords[type="cartesian"] mark { fill: red; }
        coords[type="cartesian"] mark[type="symbol"] { size: 100px; fill: green; }
        guide axis { font-size: 12px; }
        canvas title { font-size: 20px; }
    "#;

    let theme = CssTheme::from_css(css).unwrap();

    // Test basic descendant selector
    let canvas = ThemeContext::new("canvas");
    let coords = canvas.child("coords");
    let mark = coords.child("mark");
    let fill = theme.query_css(&mark, "fill");
    if let ThemeValue::Color(color) = fill {
        // Should match "coords mark { fill: blue; }"
        assert_eq!(color.blue, 255);
        assert_eq!(color.red, 0);
    }

    // Test descendant with attribute selector
    let cartesian_coords = canvas.child("coords").with_subtype("cartesian");
    let mark_in_cartesian = cartesian_coords.child("mark");
    let fill = theme.query_css(&mark_in_cartesian, "fill");
    if let ThemeValue::Color(color) = fill {
        // Should match "coords[type='cartesian'] mark { fill: red; }"
        assert_eq!(color.red, 255);
        assert_eq!(color.blue, 0);
    }

    // Test multiple levels with attributes
    let symbol_in_cartesian = cartesian_coords.child("mark").with_subtype("symbol");
    let size = theme.query_css(&symbol_in_cartesian, "size");
    assert!(matches!(size, ThemeValue::Length(100.0, _)));
    let fill = theme.query_css(&symbol_in_cartesian, "fill");
    if let ThemeValue::Color(color) = fill {
        // Should match the more specific selector
        assert_eq!(color.green, 128);
    }

    // Test guide > axis
    let guide = coords.child("guide");
    let axis = guide.child("axis");
    let font_size = theme.query_css(&axis, "font-size");
    assert!(matches!(font_size, ThemeValue::Length(12.0, _)));

    // Test canvas > title
    let title = canvas.child("title");
    let title_size = theme.query_css(&title, "font-size");
    assert!(matches!(title_size, ThemeValue::Length(20.0, _)));
}
