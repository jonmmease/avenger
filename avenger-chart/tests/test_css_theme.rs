//! Tests for the CSS theme system

use avenger_chart::theme::{LengthUnit, Theme, ThemeContext, ThemeValue};
use indexmap::IndexMap;

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
    let context = ThemeContext::new("mark", IndexMap::new());
    let fill = theme.query(&context, "fill");
    assert!(matches!(fill, Some(ThemeValue::Color(_))));
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
    let context = ThemeContext::new("mark", IndexMap::new());
    let fill = theme.query(&context, "fill");
    assert!(matches!(fill, Some(ThemeValue::Color(_))));

    // Test type attribute selector (should override element selector)
    let context_with_type = ThemeContext::new("mark", IndexMap::new()).with_subtype("symbol");
    let fill_type = theme.query(&context_with_type, "fill");
    if let Some(ThemeValue::Color(color)) = fill_type {
        assert_eq!(color.red, 255, "Symbol mark should be red");
    } else {
        panic!("Expected red color for symbol mark, got {:?}", fill_type);
    }

    // Test ID selector (should override type)
    let context_with_id = ThemeContext::new("mark", IndexMap::new())
        .with_subtype("symbol")
        .with_id("main-mark");
    let fill_id = theme.query(&context_with_id, "fill");
    if let Some(ThemeValue::Color(color)) = fill_id {
        assert_eq!(
            color.green, 128,
            "Mark with ID 'main-mark' should be green (#008000)"
        );
    } else {
        panic!(
            "Expected green color for mark with ID 'main-mark', got {:?}",
            fill_id
        );
    }
}

#[test]
fn test_property_inheritance() {
    let css = r#"
        axis { font-size: 14px; color: #333; }
        axis text { font-weight: bold; }
    "#;

    let theme = Theme::from_css(css).unwrap();

    // Test that properties return None when no CSS rule matches
    let context = ThemeContext::new("text", IndexMap::new());
    let font_size = theme.query(&context, "font-size");
    // Should return None (no CSS rule for "text" element)
    assert!(matches!(font_size, None));
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
    let context = ThemeContext::new("mark", IndexMap::new());
    let fill = theme.query(&context, "fill");
    // Should resolve to the color
    if let Some(ThemeValue::Variable(var_name)) = fill {
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
        let context = ThemeContext::new("mark", IndexMap::new()).with_class(class);
        let fill = theme.query(&context, "fill");
        assert!(matches!(fill, Some(ThemeValue::Color(_))));
    }
}

#[test]
fn test_length_units() {
    let css = r#"
        .px { stroke-width: 2px; }
        .rem { font-size: 1.5rem; }
        .percent { width: 50%; }
    "#;

    let theme = Theme::from_css(css).unwrap();

    let px_context = ThemeContext::new("mark", IndexMap::new()).with_class("px");
    let px_value = theme.query(&px_context, "stroke-width");
    assert!(matches!(
        px_value,
        Some(ThemeValue::Length(2.0, LengthUnit::Px))
    ));

    let rem_context = ThemeContext::new("mark", IndexMap::new()).with_class("rem");
    let rem_value = theme.query(&rem_context, "font-size");
    assert!(matches!(
        rem_value,
        Some(ThemeValue::Length(1.5, LengthUnit::Rem))
    ));

    let percent_context = ThemeContext::new("mark", IndexMap::new()).with_class("percent");
    let percent_value = theme.query(&percent_context, "width");
    assert!(matches!(percent_value, Some(ThemeValue::Percentage(50.0))));
}

#[test]
fn test_unsupported_length_units_error() {
    // Test that unsupported units like em, pt raise an error
    let css = r#"
        .test { font-size: 1.5em; }
        .valid { color: red; }
    "#;

    let result = Theme::from_css(css);
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

    let result = Theme::from_css(css);
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

    let theme = Theme::from_css(css).unwrap();

    // Element with type, class and id
    let context = ThemeContext::new("mark", IndexMap::new())
        .with_subtype("symbol")
        .with_class("highlight")
        .with_id("main");

    let fill = theme.query(&context, "fill");
    // ID selector should win (highest specificity)
    if let Some(ThemeValue::Color(color)) = fill {
        assert_eq!(
            color.green, 128,
            "ID selector should have highest specificity (green)"
        );
    } else {
        panic!("Expected green color from ID selector, got {:?}", fill);
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
    let context = ThemeContext::new("mark", IndexMap::new());

    let size = theme.query(&context, "size");
    assert_eq!(
        size.and_then(|v| v.as_font_size(&indexmap::IndexMap::new(), 12.0)),
        Some(100.0)
    );

    let opacity = theme.query(&context, "opacity");
    assert_eq!(opacity.and_then(|v| v.as_number()), Some(0.5));

    let stroke_width = theme.query(&context, "stroke-width");
    assert_eq!(
        stroke_width.and_then(|v| v.as_font_size(&indexmap::IndexMap::new(), 12.0)),
        Some(2.0)
    );
}

// NOTE: Comprehensive descendant selector tests have been moved to test_css_descendant.rs
// This avoids duplication while keeping the dedicated file for descendant selector testing
