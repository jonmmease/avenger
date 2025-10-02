//! CSS value parsing utilities

use crate::theme::{CssRgba, ThemeValue};

/// Parse an rgb() or rgba() function from parsed CSS arguments
pub fn parse_rgb_function(args: &[ThemeValue]) -> Option<CssRgba> {
    if args.len() < 3 {
        return None;
    }

    let red = match &args[0] {
        ThemeValue::Number(n) => (*n as u8).min(255),
        _ => return None,
    };

    let green = match &args[1] {
        ThemeValue::Number(n) => (*n as u8).min(255),
        _ => return None,
    };

    let blue = match &args[2] {
        ThemeValue::Number(n) => (*n as u8).min(255),
        _ => return None,
    };

    let alpha = if args.len() > 3 {
        match &args[3] {
            ThemeValue::Number(n) => ((*n * 255.0) as u8).min(255),
            _ => 255,
        }
    } else {
        255
    };

    Some(CssRgba {
        red,
        green,
        blue,
        alpha,
    })
}
