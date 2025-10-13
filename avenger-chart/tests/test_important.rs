//! Tests for CSS !important flag support

use avenger_chart::theme::{Theme, ThemeContext, ThemeValue};
use indexmap::IndexMap;

#[test]
fn test_important_overrides_higher_specificity() {
    // !important on lower specificity selector should win
    let css = r#"
        /* Lower specificity but with !important */
        mark {
            stroke: #ff0000 !important;
        }

        /* Higher specificity without !important */
        mark[type="symbol"] {
            stroke: #0000ff;
        }
    "#;

    let theme = Theme::from_css(css).expect("Failed to parse CSS");

    // Query for a symbol mark
    let symbol_ctx = ThemeContext::new("mark", IndexMap::new()).with_subtype("symbol");
    let stroke = theme.query(&symbol_ctx, "stroke");

    // Should get red (#ff0000) because !important wins
    if let Some(ThemeValue::Color(color)) = stroke {
        assert_eq!(color.red, 255, "Expected red from !important");
        assert_eq!(color.green, 0);
        assert_eq!(color.blue, 0);
    } else {
        panic!("Expected color value, got {:?}", stroke);
    }
}

#[test]
fn test_important_vs_important() {
    // When both have !important, higher specificity should win
    let css = r#"
        mark {
            fill: #ff0000 !important;
        }

        mark[type="symbol"] {
            fill: #0000ff !important;
        }
    "#;

    let theme = Theme::from_css(css).expect("Failed to parse CSS");

    let symbol_ctx = ThemeContext::new("mark", IndexMap::new()).with_subtype("symbol");
    let fill = theme.query(&symbol_ctx, "fill");

    // Should get blue (#0000ff) because higher specificity wins when both are important
    if let Some(ThemeValue::Color(color)) = fill {
        assert_eq!(color.red, 0);
        assert_eq!(color.green, 0);
        assert_eq!(color.blue, 255, "Expected blue from higher specificity !important");
    } else {
        panic!("Expected color value, got {:?}", fill);
    }
}

#[test]
fn test_important_source_order() {
    // When both have same specificity and both !important, later source order wins
    let css = r#"
        mark {
            opacity: 0.5 !important;
        }

        mark {
            opacity: 0.8 !important;
        }
    "#;

    let theme = Theme::from_css(css).expect("Failed to parse CSS");

    let mark_ctx = ThemeContext::new("mark", IndexMap::new());
    let opacity = theme.query(&mark_ctx, "opacity");

    // Should get 0.8 because later rule wins
    if let Some(ThemeValue::Number(value)) = opacity {
        assert!((value - 0.8).abs() < 0.01, "Expected 0.8 from later !important");
    } else {
        panic!("Expected number value, got {:?}", opacity);
    }
}

#[test]
fn test_important_append_css() {
    // !important in appended CSS should override default theme
    let mut theme = Theme::light();

    theme
        .append_css(
            r#"
        mark {
            stroke: #00ff00 !important;
        }
    "#,
        )
        .expect("Failed to append CSS");

    // The default theme has mark[type="symbol"] { stroke: var(--bg-color); }
    // Our !important should win despite lower specificity
    let symbol_ctx = ThemeContext::new("mark", IndexMap::new()).with_subtype("symbol");
    let stroke = theme.query(&symbol_ctx, "stroke");

    if let Some(ThemeValue::Color(color)) = stroke {
        assert_eq!(color.red, 0);
        assert_eq!(color.green, 255, "Expected green from !important");
        assert_eq!(color.blue, 0);
    } else {
        panic!("Expected color value, got {:?}", stroke);
    }
}

#[test]
fn test_important_parsing_with_space() {
    // Test parsing with space before !important
    let css = r#"
        mark {
            fill: red !important;
        }
    "#;

    let theme = Theme::from_css(css);
    assert!(theme.is_ok(), "Should parse !important with space");
}

#[test]
fn test_important_parsing_without_space() {
    // Test parsing without space before !important
    let css = r#"
        mark {
            fill: red!important;
        }
    "#;

    let theme = Theme::from_css(css);
    assert!(theme.is_ok(), "Should parse !important without space");
}

#[test]
fn test_non_important_does_not_override() {
    // Regular cascade without !important
    let css = r#"
        mark {
            stroke: #ff0000;
        }

        mark[type="symbol"] {
            stroke: #0000ff;
        }
    "#;

    let theme = Theme::from_css(css).expect("Failed to parse CSS");

    let symbol_ctx = ThemeContext::new("mark", IndexMap::new()).with_subtype("symbol");
    let stroke = theme.query(&symbol_ctx, "stroke");

    // Should get blue because higher specificity wins in normal cascade
    if let Some(ThemeValue::Color(color)) = stroke {
        assert_eq!(color.red, 0);
        assert_eq!(color.green, 0);
        assert_eq!(color.blue, 255, "Expected blue from higher specificity");
    } else {
        panic!("Expected color value, got {:?}", stroke);
    }
}

#[test]
fn test_important_with_multiple_properties() {
    // Mix of !important and non-!important properties
    let css = r#"
        mark {
            stroke: #ff0000 !important;
            fill: #ff0000;
        }

        mark[type="symbol"] {
            stroke: #0000ff;
            fill: #0000ff !important;
        }
    "#;

    let theme = Theme::from_css(css).expect("Failed to parse CSS");

    let symbol_ctx = ThemeContext::new("mark", IndexMap::new()).with_subtype("symbol");

    // stroke: red from !important with lower specificity
    let stroke = theme.query(&symbol_ctx, "stroke");
    if let Some(ThemeValue::Color(color)) = stroke {
        assert_eq!(color.red, 255, "stroke should be red from !important");
        assert_eq!(color.green, 0);
        assert_eq!(color.blue, 0);
    } else {
        panic!("Expected stroke color");
    }

    // fill: blue from !important with higher specificity
    let fill = theme.query(&symbol_ctx, "fill");
    if let Some(ThemeValue::Color(color)) = fill {
        assert_eq!(color.red, 0);
        assert_eq!(color.green, 0);
        assert_eq!(color.blue, 255, "fill should be blue from higher specificity !important");
    } else {
        panic!("Expected fill color");
    }
}
