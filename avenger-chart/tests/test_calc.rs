//! Tests for CSS calc() functionality
//!
//! Tests the CSS calc() function with various mathematical operations,
//! units (px, rem), variables, and built-in functions (min, max, clamp).

use avenger_chart::theme::{Theme, ThemeContext};
use datafusion::common::ScalarValue;
use indexmap::IndexMap;

// ============================================================================
// Basic Arithmetic Operations
// ============================================================================

/// Test calc() with simple addition
#[test]
fn test_basic_calc_addition() {
    let css = r#"
        mark {
            font-size: calc(10px + 5px);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark", IndexMap::new());

    // Font size should be 15px (10px + 5px)
    let font_size = theme.font_size(&ctx);
    assert_eq!(font_size, Some(15.0));
}

/// Test calc() with multiplication
#[test]
fn test_calc_multiplication() {
    let css = r#"
        mark {
            font-size: calc(8px * 2);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark", IndexMap::new());

    // Font size should be 16px (8px * 2)
    let font_size = theme.font_size(&ctx);
    assert_eq!(font_size, Some(16.0));
}

/// Test calc() with rem units (relative to base font size)
#[test]
fn test_calc_with_rem() {
    let css = r#"
        mark {
            font-size: calc(1rem + 4px);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark", IndexMap::new());

    // With default base font size 12px: 1rem = 12px, so 12px + 4px = 16px
    let font_size = theme.font_size(&ctx);
    assert_eq!(font_size, Some(16.0));
}

// ============================================================================
// CSS Variables in calc()
// ============================================================================

/// Test calc() with CSS custom properties (variables)
#[test]
fn test_calc_with_variables() {
    let css = r#"
        mark {
            font-size: calc(var(--base-size) * 2);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();

    // Create context with runtime parameter
    let mut params = IndexMap::new();
    params.insert("--base-size".to_string(), ScalarValue::Float64(Some(8.0)));

    let ctx = ThemeContext::new("mark", params);

    // Font size should be 16px (8 * 2)
    let font_size = theme.font_size(&ctx);
    assert_eq!(font_size, Some(16.0));
}

// ============================================================================
// CSS Math Functions
// ============================================================================

/// Test min() function to find minimum value
#[test]
fn test_calc_min_function() {
    let css = r#"
        mark {
            font-size: min(20px, 30px, 15px);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark", IndexMap::new());

    // Should be 15px (minimum)
    let font_size = theme.font_size(&ctx);
    assert_eq!(font_size, Some(15.0));
}

/// Test max() function to find maximum value
#[test]
fn test_calc_max_function() {
    let css = r#"
        mark {
            font-size: max(20px, 30px, 15px);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark", IndexMap::new());

    // Should be 30px (maximum)
    let font_size = theme.font_size(&ctx);
    assert_eq!(font_size, Some(30.0));
}

/// Test clamp() function to constrain value between min and max
#[test]
fn test_calc_clamp_function() {
    let css = r#"
        mark {
            font-size: clamp(10px, 25px, 20px);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark", IndexMap::new());

    // Should be 20px (clamped to max)
    let font_size = theme.font_size(&ctx);
    assert_eq!(font_size, Some(20.0));
}

// ============================================================================
// Complex Expressions
// ============================================================================

/// Test calc() with division operator
#[test]
fn test_calc_simple_division() {
    let css = r#"
        mark {
            font-size: calc(80px / 4);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark", IndexMap::new());

    // Should be 20px (80 / 4 = 20)
    let font_size = theme.font_size(&ctx);
    assert_eq!(font_size, Some(20.0));
}

/// Test calc() with parentheses and multiple operations
#[test]
fn test_calc_complex_expression() {
    let css = r#"
        mark {
            font-size: calc((100px - 20px) / 4);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark", IndexMap::new());

    // Should be 20px ((100 - 20) / 4 = 80 / 4 = 20)
    let font_size = theme.font_size(&ctx);
    assert_eq!(font_size, Some(20.0));
}

/// Test calc() combining multiple variables with operations
#[test]
fn test_calc_with_variables_and_operations() {
    let css = r#"
        mark {
            font-size: calc(var(--base) + var(--offset));
        }
    "#;

    let theme = Theme::from_css(css).unwrap();

    // Create context with runtime parameters
    let mut params = IndexMap::new();
    params.insert("--base".to_string(), ScalarValue::Float64(Some(12.0)));
    params.insert("--offset".to_string(), ScalarValue::Float64(Some(4.0)));

    let ctx = ThemeContext::new("mark", params);

    // Font size should be 16px (12 + 4)
    let font_size = theme.font_size(&ctx);
    assert_eq!(font_size, Some(16.0));
}

/// Test calc() with mathematical constants (pi, e)
#[test]
fn test_calc_constants() {
    let css = r#"
        mark {
            font-size: calc(pi * 5);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark", IndexMap::new());

    // Should be approximately 15.7 (π * 5)
    let font_size = theme.font_size(&ctx);
    assert!(font_size.is_some());
    let result = font_size.unwrap();
    assert!((result - 15.707963).abs() < 0.001);
}

/// Test calc() with custom base font size affecting rem units
#[test]
fn test_calc_with_custom_base_font_size() {
    let css = r#"
        :root {
            font-size: 16px;
        }
        mark {
            font-size: calc(1rem + 4px);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark", IndexMap::new());

    // With custom base font size 16px: 1rem = 16px, so 16px + 4px = 20px
    let font_size = theme.font_size(&ctx);
    assert_eq!(font_size, Some(20.0));
}
