//! Test that font-size changes in media queries cascade to rem-based values

use avenger_chart::theme::{Theme, ThemeContext};
use datafusion_common::ScalarValue;
use indexmap::IndexMap;

#[test]
fn test_root_font_size_cascades_in_media_query() {
    let css = r#"
        /* Base styles with rem units */
        :root {
            font-size: 12px;
        }

        chart-title {
            font-size: 2.0rem;
            color: #000000;
        }

        axis title {
            font-size: 1.0rem;
        }

        legend label {
            font-size: 0.9rem;
        }

        /* Small height - reduce root font size */
        @media (height < 300px) {
            :root {
                font-size: 10px;
            }

            chart-title {
                color: #ff0000;
            }
        }

        /* Large height - increase root font size */
        @media (height >= 300px) {
            :root {
                font-size: 14px;
            }

            chart-title {
                color: #00ff00;
            }
        }
    "#;

    let theme = Theme::from_css(css).expect("Failed to parse CSS");

    // Test 1: No height param - should use default 12px root, so 2.0rem * 12px = 24px
    let ctx_no_param = ThemeContext::new("chart-title", IndexMap::new());
    let font_size_default = theme.font_size(&ctx_no_param);
    println!("No param chart-title font-size: {:?}", font_size_default);

    if let Some(size) = font_size_default {
        assert_eq!(
            size, 24.0,
            "Default chart-title should be 24px (2.0rem * 12px root)"
        );
    } else {
        panic!("Expected font size value for chart-title, got None");
    }

    // Test 2: Small height (250px) - root becomes 10px, chart-title = 20px (2.0 * 10)
    let mut params_small = IndexMap::new();
    params_small.insert("height".to_string(), ScalarValue::Float32(Some(250.0)));
    let ctx_small = ThemeContext::new("chart-title", params_small);
    let font_size_small = theme.font_size(&ctx_small);
    let color_small = theme.query(&ctx_small, "color");
    println!("Small (250px) chart-title font-size: {:?}", font_size_small);
    println!("Small (250px) chart-title color: {:?}", color_small);

    // Color should change to red (media query is working)
    if let Some(avenger_chart::theme::ThemeValue::Color(color)) = color_small {
        assert_eq!(color.red, 255, "Small height should have red title");
        assert_eq!(color.green, 0, "Small height should have red title");
        assert_eq!(color.blue, 0, "Small height should have red title");
    } else {
        panic!(
            "Expected color value for small height, got {:?}",
            color_small
        );
    }

    // Font size should cascade from root change: 2.0rem * 10px = 20px
    if let Some(size) = font_size_small {
        assert_eq!(
            size, 20.0,
            "Small height chart-title should be 20px (2.0rem * 10px root)"
        );
    } else {
        panic!("Expected font size value for chart-title, got None");
    }

    // Test 3: Large height (400px) - root becomes 14px, chart-title = 28px (2.0 * 14)
    let mut params_large = IndexMap::new();
    params_large.insert("height".to_string(), ScalarValue::Float32(Some(400.0)));
    let ctx_large = ThemeContext::new("chart-title", params_large);
    let font_size_large = theme.font_size(&ctx_large);
    let color_large = theme.query(&ctx_large, "color");
    println!("Large (400px) chart-title font-size: {:?}", font_size_large);
    println!("Large (400px) chart-title color: {:?}", color_large);

    // Color should change to green (media query is working)
    if let Some(avenger_chart::theme::ThemeValue::Color(color)) = color_large {
        assert_eq!(color.red, 0, "Large height should have green title");
        assert_eq!(color.green, 255, "Large height should have green title");
        assert_eq!(color.blue, 0, "Large height should have green title");
    } else {
        panic!(
            "Expected color value for large height, got {:?}",
            color_large
        );
    }

    // Font size should cascade from root change: 2.0rem * 14px = 28px
    if let Some(size) = font_size_large {
        assert_eq!(
            size, 28.0,
            "Large height chart-title should be 28px (2.0rem * 14px root)"
        );
    } else {
        panic!("Expected font size value for chart-title, got None");
    }

    // Test 4: Test that axis title also cascades properly
    let mut params_axis_small = IndexMap::new();
    params_axis_small.insert("height".to_string(), ScalarValue::Float32(Some(250.0)));
    let ctx_axis = ThemeContext::new("axis", params_axis_small);
    let ctx_axis_title = ctx_axis.child("title");
    let axis_font_size_small = theme.font_size(&ctx_axis_title);
    println!(
        "Small (250px) axis title font-size: {:?}",
        axis_font_size_small
    );

    if let Some(size) = axis_font_size_small {
        assert_eq!(
            size, 10.0,
            "Small height axis title should be 10px (1.0rem * 10px root)"
        );
    } else {
        panic!("Expected font size value for axis title, got None");
    }
}
