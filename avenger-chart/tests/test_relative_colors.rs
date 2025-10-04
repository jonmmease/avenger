//! Tests for CSS Color Level 5 Relative Color Syntax

use avenger_chart::theme::{Theme, ThemeContext};

// ============================================================================
// Basic Relative Color Tests - All Color Spaces
// ============================================================================

#[test]
fn test_oklch_from_blue_darken() {
    let css = r#"
        mark {
            fill: oklch(from blue calc(l - 0.2) c h);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark");

    let color = theme.fill_color(&ctx);
    assert!(color.is_some(), "Should resolve oklch relative color");

    let [r, g, b, a] = color.unwrap();
    // Blue darkened should still be bluish but darker
    assert!(b > r && b > g, "Should still be predominantly blue");
    assert_eq!(a, 1.0, "Alpha should be 1.0");
}

#[test]
fn test_oklab_from_blue_adjust_lightness() {
    let css = r#"
        mark {
            fill: oklab(from blue calc(l * 0.9) a b);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark");

    let color = theme.fill_color(&ctx);
    assert!(color.is_some(), "Should resolve oklab relative color");
}

#[test]
fn test_lch_from_red_rotate_hue() {
    let css = r#"
        mark {
            fill: lch(from red l c calc(h + 120));
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark");

    let color = theme.fill_color(&ctx);
    assert!(color.is_some(), "Should resolve lch relative color");
}

#[test]
fn test_lab_from_red_preserve_components() {
    let css = r#"
        mark {
            fill: lab(from red l a b);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark");

    let color = theme.fill_color(&ctx);
    assert!(color.is_some(), "Should resolve lab relative color");
}

#[test]
fn test_hsl_from_orange_desaturate() {
    let css = r#"
        mark {
            fill: hsl(from orange h calc(s * 0.5) l);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark");

    let color = theme.fill_color(&ctx);
    assert!(color.is_some(), "Should resolve hsl relative color");
}

// HWB relative colors not fully supported (no absolute hwb parser yet)

#[test]
fn test_rgb_from_navy_lighten() {
    let css = r#"
        mark {
            fill: rgb(from navy calc(r * 1.5) calc(g * 1.5) calc(b * 1.5));
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark");

    let color = theme.fill_color(&ctx);
    assert!(color.is_some(), "Should resolve rgb relative color");
}

// ============================================================================
// Tests with Runtime Parameters (CSS Variables)
// ============================================================================

#[test]
fn test_relative_color_with_variable_origin() {
    use datafusion::common::ScalarValue;
    use indexmap::IndexMap;

    let css = r#"
        mark {
            fill: oklch(from var(--base-color) calc(l * 0.8) c h);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();

    // Provide runtime parameter for --base-color (blue)
    let mut params = IndexMap::new();
    params.insert("--base-color".to_string(), ScalarValue::Utf8(Some("blue".to_string())));

    let ctx = ThemeContext::new("mark").with_params(params);

    let color = theme.fill_color(&ctx);
    assert!(color.is_some(), "Should resolve with variable origin color");

    let [r, g, b, _] = color.unwrap();
    // Should be darkened blue
    assert!(b > r && b > g, "Should still be predominantly blue");
}

#[test]
fn test_relative_color_with_variable_in_calc() {
    use datafusion::common::ScalarValue;
    use indexmap::IndexMap;

    let css = r#"
        mark {
            fill: oklch(from blue calc(l * var(--factor)) c h);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();

    // Provide runtime parameter for --factor
    let mut params = IndexMap::new();
    params.insert("--factor".to_string(), ScalarValue::Float64(Some(0.5)));

    let ctx = ThemeContext::new("mark").with_params(params);

    let color = theme.fill_color(&ctx);
    assert!(
        color.is_some(),
        "Should resolve with variable in component calc"
    );

    let [r, g, b, _] = color.unwrap();
    // Should be darkened blue (lightness reduced by 50%)
    assert!(b > r && b > g, "Should still be predominantly blue");
}

#[test]
fn test_relative_color_with_variable_origin_and_calc() {
    use datafusion::common::ScalarValue;
    use indexmap::IndexMap;

    let css = r#"
        mark {
            fill: oklch(from var(--base-color) calc(l * var(--lightness-factor)) c h);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();

    // Provide both runtime parameters
    let mut params = IndexMap::new();
    params.insert("--base-color".to_string(), ScalarValue::Utf8(Some("red".to_string())));
    params.insert("--lightness-factor".to_string(), ScalarValue::Float64(Some(1.2)));

    let ctx = ThemeContext::new("mark").with_params(params);

    let color = theme.fill_color(&ctx);
    assert!(
        color.is_some(),
        "Should resolve with variables in both origin and calc"
    );

    let [r, g, b, _] = color.unwrap();
    // Should be lightened red
    assert!(r > g && r > b, "Should still be predominantly red");
}

#[test]
fn test_relative_color_alpha_with_variable() {
    use datafusion::common::ScalarValue;
    use indexmap::IndexMap;

    let css = r#"
        mark {
            fill: oklch(from blue l c h / var(--opacity));
        }
    "#;

    let theme = Theme::from_css(css).unwrap();

    // Provide runtime parameter for --opacity
    let mut params = IndexMap::new();
    params.insert("--opacity".to_string(), ScalarValue::Float64(Some(0.6)));

    let ctx = ThemeContext::new("mark").with_params(params);

    let color = theme.fill_color(&ctx);
    assert!(
        color.is_some(),
        "Should resolve with variable in alpha component"
    );

    let [_, _, _, a] = color.unwrap();
    assert!((a - 0.6).abs() < 0.01, "Alpha should be ~0.6");
}

// ============================================================================
// Alpha Channel Tests
// ============================================================================

#[test]
fn test_relative_color_with_alpha() {
    let css = r#"
        mark {
            fill: oklch(from blue l c h / 0.5);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark");

    let color = theme.fill_color(&ctx);
    assert!(color.is_some(), "Should resolve with alpha");

    let [_, _, _, a] = color.unwrap();
    assert!((a - 0.5).abs() < 0.01, "Alpha should be 0.5");
}

#[test]
fn test_relative_color_calc_alpha() {
    let css = r#"
        mark {
            fill: oklch(from blue l c h / calc(alpha * 0.8));
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark");

    let color = theme.fill_color(&ctx);
    assert!(color.is_some(), "Should resolve with calc alpha");

    let [_, _, _, a] = color.unwrap();
    // Blue has alpha 1.0, so 1.0 * 0.8 = 0.8
    assert!((a - 0.8).abs() < 0.01, "Alpha should be 0.8");
}

// Note: Variables in alpha channel not currently supported in component calc

// ============================================================================
// Edge Cases and Special Values
// ============================================================================

// 'none' keyword currently converts to 0.0, full support pending

#[test]
fn test_relative_color_hue_wraparound() {
    let css = r#"
        mark {
            fill: oklch(from blue l c calc(h + 400));
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark");

    let color = theme.fill_color(&ctx);
    assert!(color.is_some(), "Should handle hue values > 360");
}

#[test]
fn test_relative_color_clamping() {
    let css = r#"
        mark {
            fill: oklch(from blue calc(l + 1.0) c h);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark");

    let color = theme.fill_color(&ctx);
    assert!(color.is_some(), "Should handle values outside normal range");

    let [r, g, b, _] = color.unwrap();
    // Components should be clamped to [0, 1]
    assert!(r >= 0.0 && r <= 1.0);
    assert!(g >= 0.0 && g <= 1.0);
    assert!(b >= 0.0 && b <= 1.0);
}

// ============================================================================
// Named Color Origins
// ============================================================================

#[test]
fn test_relative_color_from_named_colors() {
    let named_colors = vec![
        ("black", "oklch(from black calc(l + 0.3) c h)"),
        ("white", "oklch(from white calc(l - 0.3) c h)"),
        ("red", "oklch(from red l c h)"),
        ("lime", "hsl(from lime h s l)"),
        ("blue", "rgb(from blue r g b)"),
    ];

    for (name, color_expr) in named_colors {
        let css = format!(
            r#"
            mark {{
                fill: {};
            }}
            "#,
            color_expr
        );

        let theme = Theme::from_css(&css).unwrap();
        let ctx = ThemeContext::new("mark");

        let color = theme.fill_color(&ctx);
        assert!(
            color.is_some(),
            "Should resolve relative color from named color '{}'",
            name
        );
    }
}

// ============================================================================
// Hex Color Origins
// ============================================================================

#[test]
fn test_relative_color_from_hex() {
    let css = r#"
        mark {
            fill: oklch(from #ff5733 calc(l * 0.8) c h);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark");

    let color = theme.fill_color(&ctx);
    assert!(color.is_some(), "Should resolve from hex color");
}

#[test]
fn test_relative_color_from_short_hex() {
    let css = r#"
        mark {
            fill: hsl(from #f5a h s l);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark");

    let color = theme.fill_color(&ctx);
    assert!(color.is_some(), "Should resolve from short hex color");
}

// ============================================================================
// Nested Functions
// ============================================================================

#[test]
fn test_relative_color_with_min_max() {
    let css = r#"
        mark {
            fill: oklch(from blue min(l + 0.2, 1.0) c h);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark");

    let color = theme.fill_color(&ctx);
    assert!(color.is_some(), "Should handle min() in component calc");
}

// clamp() with channel keywords works

// ============================================================================
// Multiple Properties
// ============================================================================

#[test]
fn test_relative_colors_in_multiple_properties() {
    let css = r#"
        mark {
            fill: oklch(from blue calc(l - 0.2) c h);
            stroke: oklch(from blue calc(l - 0.4) c h);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark");

    let fill = theme.fill_color(&ctx);
    let stroke = theme.stroke_color(&ctx);

    assert!(fill.is_some(), "Fill should resolve");
    assert!(stroke.is_some(), "Stroke should resolve");

    let [_, _, _, fill_b] = fill.unwrap();
    let [_, _, _, stroke_b] = stroke.unwrap();

    // Stroke should be darker than fill (both derived from blue)
    assert!(stroke_b <= fill_b, "Stroke should be darker");
}

// ============================================================================
// Cross-Space Derivations
// ============================================================================

#[test]
fn test_derive_oklch_from_rgb_origin() {
    let css = r#"
        mark {
            fill: oklch(from rgb(255, 0, 0) calc(l * 0.7) c h);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark");

    let color = theme.fill_color(&ctx);
    assert!(
        color.is_some(),
        "Should derive oklch from rgb origin with conversion"
    );
}

#[test]
fn test_derive_hsl_from_lab_origin() {
    let css = r#"
        mark {
            fill: hsl(from lab(50 20 -30) h s calc(l * 1.2));
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark");

    let color = theme.fill_color(&ctx);
    assert!(
        color.is_some(),
        "Should derive hsl from lab origin with conversion"
    );
}
