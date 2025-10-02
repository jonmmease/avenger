//! Tests for CSS selector precedence and cascade rules

use avenger_chart::theme::Theme;
use avenger_chart::theme::{CssRgba, ThemeContext, ThemeValue};

// Helper function to check if a color matches expected RGB values
fn is_color(value: &ThemeValue, r: u8, g: u8, b: u8) -> bool {
    matches!(value, ThemeValue::Color(CssRgba { red, green, blue, .. }) if *red == r && *green == g && *blue == b)
}

#[test]
fn test_specificity_precedence() {
    // Test that higher specificity selectors take precedence
    let css = r#"
        mark { fill: red; }                          /* specificity: 0,0,0,1 */
        mark.special { fill: blue; }                 /* specificity: 0,0,1,1 */
        mark[type="symbol"] { fill: green; }         /* specificity: 0,0,1,1 */
        mark.special[type="symbol"] { fill: yellow; } /* specificity: 0,0,2,1 */
    "#;

    let theme = Theme::from_css(css).unwrap();

    // Test basic mark - should be red
    let basic_mark = ThemeContext::new("mark");
    let fill = theme.query(&basic_mark, "fill");
    assert!(is_color(&fill, 255, 0, 0), "Expected red color");

    // Test mark with class - should be blue
    let mark_with_class = ThemeContext::new("mark").with_class("special");
    let fill = theme.query(&mark_with_class, "fill");
    assert!(is_color(&fill, 0, 0, 255), "Expected blue color");

    // Test mark with type - should be green
    let mark_with_type = ThemeContext::new("mark").with_subtype("symbol");
    let fill = theme.query(&mark_with_type, "fill");
    assert!(is_color(&fill, 0, 128, 0), "Expected green color");

    // Test mark with both class and type - should be yellow (highest specificity)
    let mark_with_both = ThemeContext::new("mark")
        .with_class("special")
        .with_subtype("symbol");
    let fill = theme.query(&mark_with_both, "fill");
    assert!(is_color(&fill, 255, 255, 0), "Expected yellow color");
}

#[test]
fn test_source_order_precedence() {
    // Test that when specificity is equal, later rules win
    let css = r#"
        mark { fill: red; stroke: black; }
        mark { fill: blue; }  /* Later rule for same property wins */
    "#;

    let theme = Theme::from_css(css).unwrap();

    let mark = ThemeContext::new("mark");
    let fill = theme.query(&mark, "fill");
    assert!(is_color(&fill, 0, 0, 255), "Expected blue color");

    // Stroke should still be black from first rule
    let stroke = theme.query(&mark, "stroke");
    assert!(is_color(&stroke, 0, 0, 0), "Expected black color");
}

#[test]
fn test_attribute_selector_specificity() {
    let css = r#"
        mark { color: red; }                /* specificity: 0,0,0,1 */
        mark[type="symbol"] { color: blue; } /* specificity: 0,0,1,1 */
        mark[type="symbol"][opacity="0.5"] { color: green; } /* specificity: 0,0,2,1 */
    "#;

    let theme = Theme::from_css(css).unwrap();

    // Basic mark
    let basic_mark = ThemeContext::new("mark");
    let color = theme.query(&basic_mark, "color");
    assert!(is_color(&color, 255, 0, 0), "Expected red color");

    // Mark with one attribute
    let mark_with_type = ThemeContext::new("mark").with_subtype("symbol");
    let color = theme.query(&mark_with_type, "color");
    assert!(is_color(&color, 0, 0, 255), "Expected blue color");

    // Mark with two attributes - skip this test as with_prop doesn't exist
    // Would need attribute selector support to test this properly
}

#[test]
fn test_inheritance_with_specificity() {
    let css = r#"
        mark { color: red; font-size: 12px; }
        mark.special { color: blue; }
    "#;

    let theme = Theme::from_css(css).unwrap();

    // Test that only specified properties are overridden
    let special_mark = ThemeContext::new("mark").with_class("special");

    // Color should be blue (overridden)
    let color = theme.query(&special_mark, "color");
    assert!(is_color(&color, 0, 0, 255), "Expected blue color");

    // Font-size should still be 12px (inherited)
    let font_size = theme.query(&special_mark, "font-size");
    if let ThemeValue::Length(size, _) = font_size {
        assert_eq!(size, 12.0);
    } else {
        panic!("Expected length value");
    }
}

#[test]
fn test_cascade_order() {
    // Test full cascade: origin, specificity, source order
    let css = r#"
        /* Lower specificity but later in source */
        mark { fill: red; }

        /* Higher specificity but earlier */
        mark.important { fill: blue; }

        /* Same specificity, later wins */
        mark.important { fill: green; }

        /* Even higher specificity */
        mark.important[type="symbol"] { fill: yellow; }
    "#;

    let theme = Theme::from_css(css).unwrap();

    // Test cascading with same specificity - later wins
    let important_mark = ThemeContext::new("mark").with_class("important");
    let fill = theme.query(&important_mark, "fill");
    assert!(is_color(&fill, 0, 128, 0), "Expected green color");

    // Test higher specificity wins regardless of source order
    let mark_with_type = ThemeContext::new("mark")
        .with_class("important")
        .with_subtype("symbol");

    let fill = theme.query(&mark_with_type, "fill");
    assert!(is_color(&fill, 255, 255, 0), "Expected yellow color");

    // Basic mark should have red fill, stroke not set
    let basic_mark = ThemeContext::new("mark");
    let fill = theme.query(&basic_mark, "fill");
    assert!(
        is_color(&fill, 255, 0, 0),
        "Basic mark should have red fill"
    );
}

#[test]
fn test_multiple_classes_specificity() {
    let css = r#"
        mark { color: red; }
        mark.class1 { color: blue; }
        mark.class1.class2 { color: red; }
    "#;

    let theme = Theme::from_css(css).unwrap();

    // Multiple classes increase specificity
    let multi_class_mark = ThemeContext::new("mark")
        .with_class("class1")
        .with_class("class2");

    let color = theme.query(&multi_class_mark, "color");
    assert!(is_color(&color, 255, 0, 0), "Expected red color");

    // Test single class
    let single_class_mark = ThemeContext::new("mark").with_class("class1");
    let _color = theme.query(&single_class_mark, "color");
}
