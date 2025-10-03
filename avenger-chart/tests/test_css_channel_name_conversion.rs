//! Test that underscore channel names are automatically converted to hyphens in CSS

use avenger_chart::theme::Theme;
use avenger_scales::scales::RangeKind;

#[test]
fn test_underscore_to_hyphen_conversion() {
    // Define CSS theme with hyphenated property names
    let css = r#"
        mark[type="glow"] {
            glow-color-continuous: #ff0000, #00ff00;
            pulse-speed-discrete: 1, 2, 3, 4, 5;
        }
    "#;

    let theme = Theme::from_css(css).expect("Failed to parse CSS");

    // Query with underscore channel names - should automatically convert to hyphens
    let glow_color_range = theme.get_range_for_channel(
        "glow",
        "glow_color", // underscore in Rust
        RangeKind::Continuous,
        None,
    );

    let pulse_speed_range = theme.get_range_for_channel(
        "glow",
        "pulse_speed", // underscore in Rust
        RangeKind::Discrete,
        Some(3),
    );

    // Verify we got color range (not empty/default)
    // Color ranges return ScaleRange::Color variant
    match glow_color_range {
        avenger_chart::scales::ScaleRange::Color(_) => {
            // Success - got a color range
        }
        _ => panic!("Expected color range for glow_color, got something else"),
    }

    // Verify we got discrete numeric range
    match pulse_speed_range {
        avenger_chart::scales::ScaleRange::Discrete(values) => {
            assert_eq!(values.len(), 3, "Expected 3 discrete values");
        }
        _ => panic!("Expected discrete range for pulse_speed"),
    }
}

#[test]
fn test_stroke_dash_defaults() {
    // Test that default dash patterns are available
    use avenger_chart::theme::DEFAULT_DASH_NAMES;

    assert!(!DEFAULT_DASH_NAMES.is_empty(), "Should have dash patterns");
    assert!(
        DEFAULT_DASH_NAMES.contains(&"solid"),
        "Should contain 'solid' dash pattern"
    );
    assert!(
        DEFAULT_DASH_NAMES.contains(&"dashed"),
        "Should contain 'dashed' pattern"
    );
}

#[test]
fn test_stroke_width_conversion() {
    let css = r#"
        mark {
            stroke-width-discrete: 1, 3, 5, 7;
            stroke-width-continuous: 0.5, 10;
        }
    "#;

    let theme = Theme::from_css(css).expect("Failed to parse CSS");

    // Query with stroke_width (underscore)
    let discrete_range = theme.get_range_for_channel(
        "line",
        "stroke_width", // underscore in Rust
        RangeKind::Discrete,
        Some(4),
    );

    let continuous_range = theme.get_range_for_channel(
        "line",
        "stroke_width", // underscore in Rust
        RangeKind::Continuous,
        None,
    );

    // Verify we got the values from CSS
    match discrete_range {
        avenger_chart::scales::ScaleRange::Discrete(values) => {
            assert_eq!(values.len(), 4);
        }
        _ => panic!("Expected discrete range"),
    }

    match continuous_range {
        avenger_chart::scales::ScaleRange::Numeric(_, _) => {
            // Success - got numeric range
        }
        _ => panic!("Expected continuous numeric range"),
    }
}
