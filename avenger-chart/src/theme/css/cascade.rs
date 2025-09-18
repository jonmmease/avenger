//! CSS cascade and property resolution utilities

use crate::theme::{LengthUnit, Rgba, ThemeValue};

/// List of CSS properties that inherit by default
const INHERITED_PROPERTIES: &[&str] = &[
    "color",
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "line-height",
    "text-align",
    "visibility",
    "letter-spacing",
    "word-spacing",
    "direction",
];

/// Check if a property inherits by default
fn is_inherited(property: &str) -> bool {
    INHERITED_PROPERTIES.contains(&property)
}

/// Get the initial value for a property
fn get_initial_value(property: &str) -> ThemeValue {
    match property {
        // Text properties
        "color" => ThemeValue::Color(Rgba {
            red: 0,
            green: 0,
            blue: 0,
            alpha: 255,
        }),
        "font-family" => ThemeValue::String("sans-serif".to_string()),
        "font-size" => ThemeValue::Length(12.0, LengthUnit::Px),
        "font-weight" => ThemeValue::Double(400.0),
        "font-style" => ThemeValue::Keyword("normal".to_string()),

        // Mark properties
        "fill" => ThemeValue::Color(Rgba {
            red: 0,
            green: 0,
            blue: 0,
            alpha: 255,
        }),
        "stroke" => ThemeValue::None,
        "stroke-width" => ThemeValue::Double(1.0),
        "opacity" => ThemeValue::Double(1.0),
        "size" => ThemeValue::Double(60.0),

        // Layout properties
        "padding" => ThemeValue::Double(0.0),
        "margin" => ThemeValue::Double(0.0),
        "width" => ThemeValue::Keyword("auto".to_string()),
        "height" => ThemeValue::Keyword("auto".to_string()),

        // Grid properties
        "grid-color" => ThemeValue::Color(Rgba {
            red: 200,
            green: 200,
            blue: 200,
            alpha: 255,
        }),
        "grid-opacity" => ThemeValue::Double(1.0),
        "grid-width" => ThemeValue::Double(1.0),

        // Background
        "background-color" => ThemeValue::Keyword("transparent".to_string()),

        // Default
        _ => ThemeValue::Initial,
    }
}
