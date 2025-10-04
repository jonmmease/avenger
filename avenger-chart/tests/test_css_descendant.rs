//! Test CSS descendant selectors

use avenger_chart::theme::Theme;
use avenger_chart::theme::ThemeContext;

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

    let theme = Theme::from_css(css).expect("Failed to parse CSS");

    // Test direct query on axis domain (child element)
    let axis_ctx = ThemeContext::new("axis");
    let domain_ctx = axis_ctx.child("domain");

    let stroke = theme
        .stroke_color(&domain_ctx)
        .expect("Should have stroke color");
    println!("axis domain stroke: {:?}", stroke);
    // Red #ff0000 = rgb(255, 0, 0) = [1.0, 0.0, 0.0, 1.0]
    assert!(
        (stroke[0] - 1.0).abs() < 0.01,
        "axis domain should have red stroke"
    );
    assert!(stroke[1].abs() < 0.01);
    assert!(stroke[2].abs() < 0.01);

    // Test query on axis itself
    let axis_stroke = theme
        .stroke_color(&axis_ctx)
        .expect("Should have stroke color");
    println!("axis stroke: {:?}", axis_stroke);
    // Blue #0000ff = rgb(0, 0, 255) = [0.0, 0.0, 1.0, 1.0]
    assert!(axis_stroke[0].abs() < 0.01, "axis should have blue stroke");
    assert!(axis_stroke[1].abs() < 0.01);
    assert!((axis_stroke[2] - 1.0).abs() < 0.01);
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

    let theme = Theme::from_css(css).expect("Failed to parse CSS");

    let title_ctx = ThemeContext::new("chart-title");
    let title_color = theme.text_color(&title_ctx).expect("Should have color");
    // Purple #800080 = rgb(128, 0, 128) = [128/255, 0, 128/255, 1]
    assert!(
        (title_color[0] - 128.0 / 255.0).abs() < 0.01,
        "chart-title should have purple color"
    );
    assert!(title_color[1].abs() < 0.01);
    assert!((title_color[2] - 128.0 / 255.0).abs() < 0.01);

    let subtitle_ctx = ThemeContext::new("chart-subtitle");
    let subtitle_color = theme.text_color(&subtitle_ctx).expect("Should have color");
    // Green #008000 = rgb(0, 128, 0) = [0, 128/255, 0, 1]
    assert!(subtitle_color[0].abs() < 0.01);
    assert!(
        (subtitle_color[1] - 128.0 / 255.0).abs() < 0.01,
        "chart-subtitle should have green color"
    );
    assert!(subtitle_color[2].abs() < 0.01);
}
