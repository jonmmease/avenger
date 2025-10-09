//! Integration tests for contrast-color() CSS function
//!
//! Tests the full pipeline: CSS parsing → color resolution → WCAG contrast calculation

use avenger_chart::theme::{Theme, ThemeContext};
use datafusion_common::ScalarValue;
use indexmap::IndexMap;

// ============================================================================
// Test Constants
// ============================================================================

/// Epsilon for floating-point comparisons
const COLOR_EPSILON: f32 = 0.01;

/// Larger epsilon for color mixing calculations (due to rounding)
const COLOR_MIX_EPSILON: f32 = 0.02;

/// Conversion factor from u8 (0-255) to f32 (0.0-1.0)
const U8_TO_F32: f32 = 1.0 / 255.0;

// ============================================================================
// Basic Contrast Color Tests
// ============================================================================

/// Test contrast-color() with black background - should return white
#[test]
fn test_contrast_color_with_black() {
    let css = r#"
        mark {
            fill: contrast-color(#000000);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark", IndexMap::new());
    let color = theme.fill_color(&ctx).unwrap();

    // Black background should get white text
    assert_eq!(color[0], 1.0); // R
    assert_eq!(color[1], 1.0); // G
    assert_eq!(color[2], 1.0); // B
}

/// Test contrast-color() with white background - should return black
#[test]
fn test_contrast_color_with_white() {
    let css = r#"
        mark {
            fill: contrast-color(white);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark", IndexMap::new());
    let color = theme.fill_color(&ctx).unwrap();

    // White background should get black text
    assert_eq!(color[0], 0.0);
    assert_eq!(color[1], 0.0);
    assert_eq!(color[2], 0.0);
}

#[test]
fn test_contrast_color_with_named_color() {
    let css = r#"
        mark {
            background: blue;
            fill: contrast-color(blue);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark", IndexMap::new());
    let color = theme.fill_color(&ctx).unwrap();

    // Blue is dark, should get white text
    assert_eq!(color[0], 1.0);
    assert_eq!(color[1], 1.0);
    assert_eq!(color[2], 1.0);
}

#[test]
fn test_contrast_color_with_hex() {
    let css = r#"
        mark {
            fill: contrast-color(#ffff00);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark", IndexMap::new());
    let color = theme.fill_color(&ctx).unwrap();

    // Yellow is light, should get black text
    assert_eq!(color[0], 0.0);
    assert_eq!(color[1], 0.0);
    assert_eq!(color[2], 0.0);
}

// ============================================================================
// Tests with CSS Variables
// ============================================================================

/// Test contrast-color() with CSS variable for background color
#[test]
fn test_contrast_color_with_css_variable() {

    let css = r#"
        :root {
            --bg-color: #ffffff;
        }

        mark {
            fill: contrast-color(var(--bg-color));
        }
    "#;

    let theme = Theme::from_css(css).unwrap();

    // Create context with params from :root
    let mut params = IndexMap::new();
    params.insert(
        "--bg-color".to_string(),
        ScalarValue::Utf8(Some("#ffffff".to_string())),
    );

    let ctx = ThemeContext::new("mark", params);
    let color = theme.fill_color(&ctx).unwrap();

    // White background should get black text
    assert_eq!(color[0], 0.0); // R
    assert_eq!(color[1], 0.0); // G
    assert_eq!(color[2], 0.0); // B
}

#[test]
fn test_contrast_color_with_runtime_params() {

    let css = r#"
        mark {
            fill: contrast-color(var(--user-bg));
        }
    "#;

    let theme = Theme::from_css(css).unwrap();

    // Test with dark background
    let mut params_dark = IndexMap::new();
    params_dark.insert(
        "--user-bg".to_string(),
        ScalarValue::Utf8(Some("#222222".to_string())),
    );
    let ctx_dark = ThemeContext::new("mark", params_dark);
    let color_dark = theme.fill_color(&ctx_dark).unwrap();
    assert_eq!(color_dark[0], 1.0); // White for dark background

    // Test with light background
    let mut params_light = IndexMap::new();
    params_light.insert(
        "--user-bg".to_string(),
        ScalarValue::Utf8(Some("#eeeeee".to_string())),
    );
    let ctx_light = ThemeContext::new("mark", params_light);
    let color_light = theme.fill_color(&ctx_light).unwrap();
    assert_eq!(color_light[0], 0.0); // Black for light background
}

#[test]
fn test_contrast_color_with_rgb_function() {
    let css = r#"
        mark {
            fill: contrast-color(rgb(255, 0, 0));
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark", IndexMap::new());
    let color = theme.fill_color(&ctx).unwrap();

    // Pure red has luminance ~0.2126
    // Black provides better contrast (~5.64:1) than white (~3.72:1)
    assert_eq!(color[0], 0.0);
    assert_eq!(color[1], 0.0);
    assert_eq!(color[2], 0.0);
}

#[test]
fn test_contrast_color_multiple_marks() {
    let css = r#"
        mark[type="dark"] {
            background: #222;
            fill: contrast-color(#222);
        }

        mark[type="light"] {
            background: #eee;
            fill: contrast-color(#eee);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();

    // Dark mark should get white text
    let dark_ctx = ThemeContext::new("mark", IndexMap::new()).with_subtype("dark");
    let dark_color = theme.fill_color(&dark_ctx).unwrap();
    assert_eq!(dark_color[0], 1.0);

    // Light mark should get black text
    let light_ctx = ThemeContext::new("mark", IndexMap::new()).with_subtype("light");
    let light_color = theme.fill_color(&light_ctx).unwrap();
    assert_eq!(light_color[0], 0.0);
}

#[test]
fn test_contrast_color_with_oklch() {
    let css = r#"
        mark {
            fill: contrast-color(oklch(0.5 0.1 180));
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark", IndexMap::new());
    let color = theme.fill_color(&ctx);

    // Should successfully parse and evaluate
    assert!(color.is_some());
}

#[test]
fn test_contrast_color_cascading() {
    let css = r#"
        mark {
            background: black;
            fill: contrast-color(black);
        }

        mark[type="special"] {
            background: white;
            fill: contrast-color(white);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();

    // Default mark: black background → white text
    let default_ctx = ThemeContext::new("mark", IndexMap::new());
    let default_color = theme.fill_color(&default_ctx).unwrap();
    assert_eq!(default_color[0], 1.0);

    // Special mark: white background → black text
    let special_ctx = ThemeContext::new("mark", IndexMap::new()).with_subtype("special");
    let special_color = theme.fill_color(&special_ctx).unwrap();
    assert_eq!(special_color[0], 0.0);
}

#[test]
fn test_contrast_color_mid_tone() {
    let css = r#"
        mark {
            fill: contrast-color(#808080);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark", IndexMap::new());
    let color = theme.fill_color(&ctx).unwrap();

    // Mid-gray should get black (per WCAG 2.1 algorithm)
    // Black provides ~5.3:1 contrast vs white's ~4.0:1
    assert_eq!(color[0], 0.0);
    assert_eq!(color[1], 0.0);
    assert_eq!(color[2], 0.0);
}

#[test]
fn test_contrast_color_accessibility_use_case() {
    // Real-world scenario: dynamic backgrounds with accessible text
    let css = r#"
        mark[category="A"] {
            background: #E69F00;
            fill: contrast-color(#E69F00);
        }

        mark[category="B"] {
            background: #56B4E9;
            fill: contrast-color(#56B4E9);
        }

        mark[category="C"] {
            background: #009E73;
            fill: contrast-color(#009E73);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();

    // Category A: orange - should get black
    let ctx_a = ThemeContext::new("mark", IndexMap::new()).with_attribute("category", "A");
    let color_a = theme.fill_color(&ctx_a).unwrap();
    assert!(
        color_a[0] == 0.0,
        "Orange should get black text for better contrast"
    );

    // Category B: light blue - should get black
    let ctx_b = ThemeContext::new("mark", IndexMap::new()).with_attribute("category", "B");
    let color_b = theme.fill_color(&ctx_b).unwrap();
    assert!(
        color_b[0] == 0.0,
        "Light blue should get black text for better contrast"
    );

    // Category C: green - could be either, but algorithm should choose one consistently
    let ctx_c = ThemeContext::new("mark", IndexMap::new()).with_attribute("category", "C");
    let color_c = theme.fill_color(&ctx_c).unwrap();
    assert!(
        color_c[0] == 0.0 || color_c[0] == 1.0,
        "Should get either black or white"
    );
}

// ============================================================================
// Extended Syntax: Candidate Lists
// ============================================================================

/// Test contrast-color() with static candidate list
#[test]
fn test_contrast_color_with_candidates_static() {
    let css = r#"
        mark {
            fill: contrast-color(#333, #aaa, #eee);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark", IndexMap::new());
    let color = theme.fill_color(&ctx).unwrap();

    // #eee should be chosen (better contrast with #333)
    let expected = 0xee as f32 * U8_TO_F32;
    assert!((color[0] - expected).abs() < COLOR_EPSILON, "Red component should be ~{}", expected);
    assert!((color[1] - expected).abs() < COLOR_EPSILON, "Green component should be ~{}", expected);
    assert!((color[2] - expected).abs() < COLOR_EPSILON, "Blue component should be ~{}", expected);
}

#[test]
fn test_contrast_color_with_brand_palette() {
    let css = r#"
        mark {
            background: lightblue;
            fill: contrast-color(lightblue, navy, maroon, purple, teal);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark", IndexMap::new());
    let color = theme.fill_color(&ctx).unwrap();

    // Navy should provide best contrast (rgb 0, 0, 128)
    let expected_blue = 128.0 * U8_TO_F32;
    assert!((color[2] - expected_blue).abs() < COLOR_EPSILON, "Blue component should be ~{}", expected_blue);
}

#[test]
fn test_contrast_color_candidates_with_variable() {

    let css = r#"
        mark {
            fill: contrast-color(var(--bg), #aaa, #eee);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();

    // Test with dark gray background
    let mut params = IndexMap::new();
    params.insert(
        "--bg".to_string(),
        ScalarValue::Utf8(Some("#333333".to_string())),
    );

    let ctx = ThemeContext::new("mark", params);
    let color = theme.fill_color(&ctx).unwrap();

    // #eee should be chosen
    let expected = 0xee as f32 * U8_TO_F32;
    assert!((color[0] - expected).abs() < COLOR_EPSILON, "Red component should be ~{}", expected);
}

#[test]
fn test_contrast_color_all_variable_candidates() {

    let css = r#"
        mark {
            fill: contrast-color(var(--bg), var(--text1), var(--text2));
        }
    "#;

    let theme = Theme::from_css(css).unwrap();

    let mut params = IndexMap::new();
    params.insert(
        "--bg".to_string(),
        ScalarValue::Utf8(Some("#333333".to_string())), // Dark gray
    );
    params.insert(
        "--text1".to_string(),
        ScalarValue::Utf8(Some("#aaaaaa".to_string())), // Light gray
    );
    params.insert(
        "--text2".to_string(),
        ScalarValue::Utf8(Some("#eeeeee".to_string())), // Lighter gray
    );

    let ctx = ThemeContext::new("mark", params);
    let color = theme.fill_color(&ctx).unwrap();

    // #eee should be chosen (better contrast)
    let expected = 0xee as f32 * U8_TO_F32;
    assert!((color[0] - expected).abs() < COLOR_EPSILON, "Red component should be ~{}", expected);
    assert!((color[1] - expected).abs() < COLOR_EPSILON, "Green component should be ~{}", expected);
    assert!((color[2] - expected).abs() < COLOR_EPSILON, "Blue component should be ~{}", expected);
}

#[test]
fn test_contrast_color_candidates_fallback_to_black_white() {
    let css = r#"
        mark {
            fill: contrast-color(blue, 5, 10);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark", IndexMap::new());
    let color = theme.fill_color(&ctx).unwrap();

    // Invalid candidates should fall back to black/white
    // Blue is dark, should get white
    assert_eq!(color[0], 1.0);
    assert_eq!(color[1], 1.0);
    assert_eq!(color[2], 1.0);
}

// ============================================================================
// Integration with color-mix()
// ============================================================================

/// Test contrast-color() used within color-mix() for subtle shading
#[test]
fn test_contrast_color_with_color_mix() {

    // Use color-mix to create a lighter shade by mixing background toward its contrast color
    let css = r#"
        mark {
            fill: color-mix(
                in srgb,
                var(--bg) 70%,
                contrast-color(var(--bg)) 30%
            );
        }
    "#;

    let theme = Theme::from_css(css).unwrap();

    // Test with dark background
    let mut params_dark = IndexMap::new();
    params_dark.insert(
        "--bg".to_string(),
        ScalarValue::Utf8(Some("#222222".to_string())),
    );

    let ctx_dark = ThemeContext::new("mark", params_dark);
    let color_dark = theme.fill_color(&ctx_dark).unwrap();

    // Dark bg (#222) gets white contrast color
    // 70% #222 + 30% white = lighter gray
    // #222 is rgb(34, 34, 34), white is rgb(255, 255, 255)
    // Result: 0.7*34 + 0.3*255 = 23.8 + 76.5 = 100.3 ≈ 100
    let expected = 100.0 * U8_TO_F32;
    assert!((color_dark[0] - expected).abs() < COLOR_EPSILON);

    // Test with light background
    let mut params_light = IndexMap::new();
    params_light.insert(
        "--bg".to_string(),
        ScalarValue::Utf8(Some("#eeeeee".to_string())),
    );

    let ctx_light = ThemeContext::new("mark", params_light);
    let color_light = theme.fill_color(&ctx_light).unwrap();

    // Light bg (#eee) gets black contrast color
    // 70% #eee + 30% black = darker gray
    // #eee is rgb(238, 238, 238), black is rgb(0, 0, 0)
    // Result: 0.7*238 + 0.3*0 = 166.6 ≈ 167
    let expected = 167.0 * U8_TO_F32;
    assert!((color_light[0] - expected).abs() < COLOR_EPSILON);
}

#[test]
fn test_contrast_color_with_color_mix_and_candidates() {

    // Mix background toward chosen candidate color (not black/white)
    let css = r#"
        mark {
            fill: color-mix(
                in srgb,
                var(--bg) 80%,
                contrast-color(var(--bg), navy, maroon) 20%
            );
        }
    "#;

    let theme = Theme::from_css(css).unwrap();

    // Light background will choose navy (darker candidate)
    let mut params = IndexMap::new();
    params.insert(
        "--bg".to_string(),
        ScalarValue::Utf8(Some("#f0f0f0".to_string())), // Very light gray
    );

    let ctx = ThemeContext::new("mark", params);
    let color = theme.fill_color(&ctx).unwrap();

    // Should be a subtle mix toward navy
    // Light bg mixed with dark navy should be slightly darker than bg
    // #f0f0f0 is rgb(240, 240, 240)
    // Navy is rgb(0, 0, 128)
    // 80% of 240 + 20% of 0/0/128 = 192, 192, 217.6
    assert!((color[0] - 192.0 * U8_TO_F32).abs() < COLOR_EPSILON, "Red component should be ~192"); // R
    assert!((color[1] - 192.0 * U8_TO_F32).abs() < COLOR_EPSILON, "Green component should be ~192"); // G
    assert!((color[2] - 217.6 * U8_TO_F32).abs() < COLOR_MIX_EPSILON, "Blue component should be ~217.6"); // B (blue component from navy)
}
