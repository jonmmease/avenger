//! Test CSS descendant selectors

use avenger_chart::theme::css::CssTheme;
use avenger_chart::theme::{Theme, ThemeContext};

#[test]
fn test_descendant_selectors() {
    let css = r#"
        axis domain {
            stroke: red;
        }
        axis {
            stroke: blue;
        }
    "#;

    let theme = CssTheme::from_css(css).expect("Failed to parse CSS");

    // Test direct query on axis domain (child element)
    let axis_ctx = ThemeContext::new("axis");
    let domain_ctx = axis_ctx.child("domain");

    let stroke = theme.stroke_color(&domain_ctx);
    println!("axis domain stroke: {}", stroke);
    assert_eq!(stroke, "#ff0000", "axis domain should have red stroke");

    // Test query on axis itself
    let axis_stroke = theme.stroke_color(&axis_ctx);
    println!("axis stroke: {}", axis_stroke);
    assert_eq!(axis_stroke, "#0000ff", "axis should have blue stroke");
}

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

    let theme = CssTheme::from_css(css).expect("Failed to parse CSS");

    let title_ctx = ThemeContext::new("chart-title");
    let title_color = theme.color(&title_ctx);
    assert_eq!(
        title_color, "#800080",
        "chart-title should have purple color"
    );

    let subtitle_ctx = ThemeContext::new("chart-subtitle");
    let subtitle_color = theme.color(&subtitle_ctx);
    assert_eq!(
        subtitle_color, "#008000",
        "chart-subtitle should have green color"
    );
}
