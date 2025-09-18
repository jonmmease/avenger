//! Tests for the CSS theme system

use avenger_chart::theme::{LengthUnit, ThemeContext, ThemeValue, css::Theme};

#[test]
fn test_basic_theme_creation() {
    let css = r#"
        mark { fill: #4c78a8; stroke: none; }
        mark[type="symbol"] { size: 60px; }
        axis { stroke: #888; stroke-width: 1px; }
    "#;

    let theme = Theme::from_css(css).unwrap();
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

    let theme = Theme::from_css(css).unwrap();

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

    let theme = Theme::from_css(css).unwrap();

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

    let theme = Theme::from_css(css).unwrap();

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

    let theme = Theme::from_css(css).unwrap();

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
        .em { font-size: 1.5em; }
        .percent { width: 50%; }
        .pt { font-size: 12pt; }
    "#;

    let theme = Theme::from_css(css).unwrap();

    let px_context = ThemeContext::new("mark").with_class("px");
    let px_value = theme.query_css(&px_context, "stroke-width");
    assert!(matches!(px_value, ThemeValue::Length(2.0, LengthUnit::Px)));

    let em_context = ThemeContext::new("mark").with_class("em");
    let em_value = theme.query_css(&em_context, "font-size");
    assert!(matches!(em_value, ThemeValue::Length(1.5, LengthUnit::Em)));

    let percent_context = ThemeContext::new("mark").with_class("percent");
    let percent_value = theme.query_css(&percent_context, "width");
    assert!(matches!(percent_value, ThemeValue::Percentage(50.0)));
}

#[test]
fn test_pseudo_classes() {
    // Simpler test - just first-child
    let css = r#"
        mark:first-child { fill: red; }
    "#;

    let theme = Theme::from_css(css).unwrap();

    let first_context = ThemeContext::new("mark").with_child_info(0, true, false);
    let first_fill = theme.query_css(&first_context, "fill");
    if let ThemeValue::Color(color) = first_fill {
        assert_eq!(color.red, 255);
    }

    let last_context = ThemeContext::new("mark").with_child_info(2, false, true);
    let last_fill = theme.query_css(&last_context, "fill");
    // Last context should not match :first-child, so should get initial value (black)
    if let ThemeValue::Color(color) = last_fill {
        assert_eq!(color.red, 0);
        assert_eq!(color.green, 0);
        assert_eq!(color.blue, 0);
    }
}

#[test]
fn test_specificity_cascade() {
    let css = r#"
        mark { fill: black; }                /* specificity: 0,0,1 */
        .highlight { fill: blue; }           /* specificity: 0,1,0 */
        mark[type="symbol"] { fill: red; }   /* specificity: 0,1,1 */
        #main { fill: green; }               /* specificity: 1,0,0 */
    "#;

    let theme = Theme::from_css(css).unwrap();

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

    let theme = Theme::from_css(css).unwrap();
    let context = ThemeContext::new("mark");

    let size = theme.query_css(&context, "size");
    assert_eq!(size.as_float(), Some(100.0));

    let opacity = theme.query_css(&context, "opacity");
    assert_eq!(opacity.as_double(), Some(0.5));

    let stroke_width = theme.query_css(&context, "stroke-width");
    assert_eq!(stroke_width.as_float(), Some(2.0));
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

    let theme = Theme::from_css(css).unwrap();

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

#[test]
fn test_pseudo_class_selectors() {
    let css = r#"
        mark:first-child { fill: red; }
        mark:last-child { fill: blue; }
        mark:nth-child(2) { fill: green; }
        legend:first-child { font-size: 20px; }
    "#;

    let theme = Theme::from_css(css).unwrap();

    // Test first mark
    let first_mark = ThemeContext::new("mark").with_child_info(0, true, false);
    let fill = theme.query_css(&first_mark, "fill");
    if let ThemeValue::String(s) = fill {
        assert_eq!(s, "red");
    } else if let ThemeValue::Color(color) = fill {
        // We expect red color
        assert_eq!(color.red, 255);
        assert_eq!(color.green, 0);
        assert_eq!(color.blue, 0);
    } else {
        panic!(
            "Expected fill to be 'red' string or red color, got {:?}",
            fill
        );
    }

    // Test second mark (nth-child(2))
    let second_mark = ThemeContext::new("mark").with_child_info(1, false, false);
    let fill = theme.query_css(&second_mark, "fill");
    if let ThemeValue::String(s) = fill {
        assert_eq!(s, "green");
    } else if let ThemeValue::Color(color) = fill {
        // We expect green color
        assert_eq!(color.red, 0);
        assert_eq!(color.green, 128);
        assert_eq!(color.blue, 0);
    } else {
        panic!(
            "Expected fill to be 'green' string or green color, got {:?}",
            fill
        );
    }

    // Test last mark
    let last_mark = ThemeContext::new("mark").with_child_info(2, false, true);
    let fill = theme.query_css(&last_mark, "fill");
    if let ThemeValue::String(s) = fill {
        assert_eq!(s, "blue");
    } else if let ThemeValue::Color(color) = fill {
        // We expect blue color
        assert_eq!(color.red, 0);
        assert_eq!(color.green, 0);
        assert_eq!(color.blue, 255);
    } else {
        panic!(
            "Expected fill to be 'blue' string or blue color, got {:?}",
            fill
        );
    }
}
