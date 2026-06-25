//! Test that media queries work when params are explicitly provided

use avenger_chart::theme::{Theme, ThemeContext};
use datafusion_common::ScalarValue;
use indexmap::IndexMap;

#[test]
fn test_media_query_with_explicit_params() {
    let css = r#"
        /* Default */
        guide {
            background-color: transparent;
        }

        /* Small screens */
        @media (width < 600px) {
            guide {
                background-color: rgba(33, 150, 243, 0.5);
            }
        }

        /* Large screens */
        @media (width >= 600px) {
            guide {
                background-color: rgba(244, 67, 54, 0.5);
            }
        }
    "#;

    let theme = Theme::from_css(css).expect("Failed to parse CSS");

    // Test 1: No width param - should get transparent (default)
    let ctx_no_param = ThemeContext::new("guide", IndexMap::new());
    let bg_no_param = theme.query(&ctx_no_param, "background-color");
    println!("No param: {:?}", bg_no_param);

    // Test 2: Small width (400px) - should get blue
    let mut params_small = IndexMap::new();
    params_small.insert("width".to_string(), ScalarValue::Float32(Some(400.0)));
    let ctx_small = ThemeContext::new("guide", params_small);
    let bg_small = theme.query(&ctx_small, "background-color");
    println!("Small (400px): {:?}", bg_small);

    if let Some(avenger_chart::theme::ThemeValue::Color(color)) = bg_small {
        // Should be blue: rgba(33, 150, 243, 0.5)
        assert_eq!(
            color.red, 33,
            "Small screen should have blue background (red=33)"
        );
        assert_eq!(
            color.green, 150,
            "Small screen should have blue background (green=150)"
        );
        assert_eq!(
            color.blue, 243,
            "Small screen should have blue background (blue=243)"
        );
        assert_eq!(
            color.alpha, 127,
            "Small screen should have alpha=127 (0.5*255)"
        );
    } else {
        panic!("Expected color value for small screen, got {:?}", bg_small);
    }

    // Test 3: Large width (800px) - should get red
    let mut params_large = IndexMap::new();
    params_large.insert("width".to_string(), ScalarValue::Float32(Some(800.0)));
    let ctx_large = ThemeContext::new("guide", params_large);
    let bg_large = theme.query(&ctx_large, "background-color");
    println!("Large (800px): {:?}", bg_large);

    if let Some(avenger_chart::theme::ThemeValue::Color(color)) = bg_large {
        // Should be red: rgba(244, 67, 54, 0.5)
        assert_eq!(
            color.red, 244,
            "Large screen should have red background (red=244)"
        );
        assert_eq!(
            color.green, 67,
            "Large screen should have red background (green=67)"
        );
        assert_eq!(
            color.blue, 54,
            "Large screen should have red background (blue=54)"
        );
        assert_eq!(
            color.alpha, 127,
            "Large screen should have alpha=127 (0.5*255)"
        );
    } else {
        panic!("Expected color value for large screen, got {:?}", bg_large);
    }
}
