//! CSS value parsing utilities

use crate::theme::{Rgba, ThemeValue};

/// Parse a named color
pub fn parse_named_color(name: &str) -> Option<Rgba> {
    let color = match name.to_lowercase().as_str() {
        // Basic colors
        "black" => (0, 0, 0),
        "white" => (255, 255, 255),
        "red" => (255, 0, 0),
        "green" => (0, 128, 0),
        "blue" => (0, 0, 255),
        "yellow" => (255, 255, 0),
        "cyan" => (0, 255, 255),
        "magenta" => (255, 0, 255),

        // Grays
        "gray" | "grey" => (128, 128, 128),
        "darkgray" | "darkgrey" => (169, 169, 169),
        "lightgray" | "lightgrey" => (211, 211, 211),
        "dimgray" | "dimgrey" => (105, 105, 105),

        // Extended colors
        "orange" => (255, 165, 0),
        "purple" => (128, 0, 128),
        "brown" => (165, 42, 42),
        "pink" => (255, 192, 203),
        "lime" => (0, 255, 0),
        "navy" => (0, 0, 128),
        "teal" => (0, 128, 128),
        "olive" => (128, 128, 0),
        "maroon" => (128, 0, 0),

        // Common web colors
        "steelblue" => (70, 130, 180),
        "cornflowerblue" => (100, 149, 237),
        "dodgerblue" => (30, 144, 255),
        "lightblue" => (173, 216, 230),
        "skyblue" => (135, 206, 235),

        _ => return None,
    };

    Some(Rgba {
        red: color.0,
        green: color.1,
        blue: color.2,
        alpha: 255,
    })
}

/// Parse a hex color
pub fn parse_hex_color(hex: &str) -> Option<Rgba> {
    let hex = hex.trim_start_matches('#');

    let (r, g, b) = match hex.len() {
        3 => {
            // Short form: #RGB
            let r = u8::from_str_radix(&hex[0..1], 16).ok()?;
            let g = u8::from_str_radix(&hex[1..2], 16).ok()?;
            let b = u8::from_str_radix(&hex[2..3], 16).ok()?;
            (r * 17, g * 17, b * 17)
        }
        6 => {
            // Long form: #RRGGBB
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            (r, g, b)
        }
        _ => return None,
    };

    Some(Rgba {
        red: r,
        green: g,
        blue: b,
        alpha: 255,
    })
}

/// Parse an rgb() or rgba() function
pub fn parse_rgb_function(args: &[ThemeValue]) -> Option<Rgba> {
    if args.len() < 3 {
        return None;
    }

    let red = match &args[0] {
        ThemeValue::Double(n) => (*n as u8).min(255),
        _ => return None,
    };

    let green = match &args[1] {
        ThemeValue::Double(n) => (*n as u8).min(255),
        _ => return None,
    };

    let blue = match &args[2] {
        ThemeValue::Double(n) => (*n as u8).min(255),
        _ => return None,
    };

    let alpha = if args.len() > 3 {
        match &args[3] {
            ThemeValue::Double(n) => ((*n * 255.0) as u8).min(255),
            _ => 255,
        }
    } else {
        255
    };

    Some(Rgba {
        red,
        green,
        blue,
        alpha,
    })
}
