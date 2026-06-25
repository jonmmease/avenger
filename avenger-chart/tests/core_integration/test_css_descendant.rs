//! Test CSS descendant selectors
//!
//! This file tests the descendant combinator (space) in CSS selectors,
//! which matches elements that are descendants of a specified ancestor.
//! For example: "coords mark" matches <mark> elements inside <coords>.

use avenger_chart::theme::Theme;
use avenger_chart::theme::{ThemeContext, ThemeValue};
use indexmap::IndexMap;

// ============================================================================
// Helper Functions
// ============================================================================

/// Helper to assert that a color value matches expected RGB components
fn assert_color_eq(value: Option<ThemeValue>, r: u8, g: u8, b: u8, message: &str) {
    match value {
        Some(ThemeValue::Color(color)) => {
            assert_eq!(color.red, r, "{} - red component", message);
            assert_eq!(color.green, g, "{} - green component", message);
            assert_eq!(color.blue, b, "{} - blue component", message);
        }
        other => panic!(
            "{}: Expected Color({}, {}, {}), got {:?}",
            message, r, g, b, other
        ),
    }
}

// ============================================================================
// Basic Descendant Selector Tests
// ============================================================================

/// Test basic descendant selector: parent child
#[test]
fn test_basic_descendant_selector() {
    let css = r#"
        axis domain {
            stroke: red;
        }
        axis {
            stroke: blue;
        }
    "#;

    let theme = Theme::from_css(css).expect("Failed to parse CSS");

    // Test descendant selector "axis domain" - should get red
    let axis_ctx = ThemeContext::new("axis", IndexMap::new());
    let domain_ctx = axis_ctx.child("domain");

    let stroke = theme
        .stroke_color(&domain_ctx)
        .expect("axis domain should have stroke color");

    // Red #ff0000 = rgb(255, 0, 0) = [1.0, 0.0, 0.0, 1.0]
    assert!(
        (stroke[0] - 1.0).abs() < 0.01,
        "axis domain should have red stroke (R=1.0)"
    );
    assert!(stroke[1].abs() < 0.01, "axis domain red has no green");
    assert!(stroke[2].abs() < 0.01, "axis domain red has no blue");

    // Test direct selector "axis" - should get blue
    let axis_stroke = theme
        .stroke_color(&axis_ctx)
        .expect("axis should have stroke color");

    // Blue #0000ff = rgb(0, 0, 255) = [0.0, 0.0, 1.0, 1.0]
    assert!(axis_stroke[0].abs() < 0.01, "axis blue has no red");
    assert!(axis_stroke[1].abs() < 0.01, "axis blue has no green");
    assert!(
        (axis_stroke[2] - 1.0).abs() < 0.01,
        "axis should have blue stroke (B=1.0)"
    );
}

/// Test descendant selectors with attribute selectors
#[test]
fn test_descendant_with_attributes() {
    let css = r#"
        coords mark { fill: blue; }
        coords[type="cartesian"] mark { fill: red; }
        coords[type="cartesian"] mark[type="symbol"] { size: 100px; fill: green; }
    "#;

    let theme = Theme::from_css(css).unwrap();

    let canvas = ThemeContext::new("canvas", IndexMap::new());
    let coords = canvas.child("coords");
    let mark = coords.child("mark");

    // Test "coords mark" - should be blue
    let fill = theme.query(&mark, "fill");
    assert_color_eq(fill, 0, 0, 255, "coords mark should be blue");

    // Test "coords[type=cartesian] mark" - should be red
    let cartesian_coords = canvas.child("coords").with_subtype("cartesian");
    let mark_in_cartesian = cartesian_coords.child("mark");
    let fill = theme.query(&mark_in_cartesian, "fill");
    assert_color_eq(fill, 255, 0, 0, "coords[type=cartesian] mark should be red");

    // Test "coords[type=cartesian] mark[type=symbol]" - should be green with size 100px
    let symbol_in_cartesian = cartesian_coords.child("mark").with_subtype("symbol");
    let size = theme.query(&symbol_in_cartesian, "size");
    assert!(
        matches!(size, Some(ThemeValue::Length(100.0, _))),
        "Symbol in cartesian should have size 100px"
    );
    let fill = theme.query(&symbol_in_cartesian, "fill");
    assert_color_eq(
        fill,
        0,
        128,
        0,
        "coords[type=cartesian] mark[type=symbol] should be green",
    );
}

/// Test multi-level descendant selectors
#[test]
fn test_multi_level_descendants() {
    let css = r#"
        canvas coords guide { background: yellow; }
        guide axis { font-size: 12px; }
        canvas title { font-size: 20px; }
    "#;

    let theme = Theme::from_css(css).unwrap();

    let canvas = ThemeContext::new("canvas", IndexMap::new());
    let coords = canvas.child("coords");

    // Test "guide axis" (2 levels)
    let guide = coords.child("guide");
    let axis = guide.child("axis");
    let font_size = theme.query(&axis, "font-size");
    assert!(
        matches!(font_size, Some(ThemeValue::Length(12.0, _))),
        "guide axis should have font-size 12px"
    );

    // Test "canvas title" (2 levels)
    let title = canvas.child("title");
    let title_size = theme.query(&title, "font-size");
    assert!(
        matches!(title_size, Some(ThemeValue::Length(20.0, _))),
        "canvas title should have font-size 20px"
    );
}

// ============================================================================
// Chart Title and Subtitle Tests
// ============================================================================

/// Test styling for chart-title and chart-subtitle elements
#[test]
fn test_chart_title_subtitle() {
    let css = r#"
        chart-title {
            color: purple;
        }
        chart-subtitle {
            color: green;
        }
    "#;

    let theme = Theme::from_css(css).expect("Failed to parse CSS");

    let title_ctx = ThemeContext::new("chart-title", IndexMap::new());
    let title_color = theme.text_color(&title_ctx).expect("Should have color");
    // Purple #800080 = rgb(128, 0, 128) = [128/255, 0, 128/255, 1]
    assert!(
        (title_color[0] - 128.0 / 255.0).abs() < 0.01,
        "chart-title should have purple color"
    );
    assert!(title_color[1].abs() < 0.01);
    assert!((title_color[2] - 128.0 / 255.0).abs() < 0.01);

    let subtitle_ctx = ThemeContext::new("chart-subtitle", IndexMap::new());
    let subtitle_color = theme.text_color(&subtitle_ctx).expect("Should have color");
    // Green #008000 = rgb(0, 128, 0) = [0, 128/255, 0, 1]
    assert!(subtitle_color[0].abs() < 0.01);
    assert!(
        (subtitle_color[1] - 128.0 / 255.0).abs() < 0.01,
        "chart-subtitle should have green color"
    );
    assert!(subtitle_color[2].abs() < 0.01);
}
