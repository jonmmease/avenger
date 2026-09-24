//! Color parsing utilities

use std::fmt;

use css_color_parser::Color as CssColor;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorParseError {
    input: String,
}

impl ColorParseError {
    pub fn new(input: impl Into<String>) -> Self {
        Self {
            input: input.into(),
        }
    }

    pub fn input(&self) -> &str {
        &self.input
    }
}

impl fmt::Display for ColorParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid color string '{}'", self.input)
    }
}

impl std::error::Error for ColorParseError {}

/// Parse a CSS color into normalized sRGB RGBA with straight alpha.
///
/// Supports CSS hex colors, rgb/rgba, hsl/hsla, and CSS named colors. Hex
/// alpha forms are handled here because css-color-parser 0.1.2 accepts
/// `#rgb`/`#rrggbb` but not `#rgba`/`#rrggbbaa`.
/// Surrounding whitespace is ignored. Invalid input returns `None`.
pub fn parse_color_string(color_str: &str) -> Option<[f32; 4]> {
    let color_str = color_str.trim();

    // Special case for rebeccapurple which isn't in css-color-parser 0.1.2
    // but is an official CSS color (added in CSS Color Module Level 4)
    if color_str.eq_ignore_ascii_case("rebeccapurple") {
        return Some([
            102.0 / 255.0, // red
            51.0 / 255.0,  // green
            153.0 / 255.0, // blue
            1.0,           // alpha
        ]);
    }

    if let Some(rgba) = parse_hex_alpha_color(color_str) {
        return Some(rgba);
    }

    // Try parsing with css-color-parser which supports:
    // - Hex colors (#fff, #ffffff)
    // - rgb/rgba functions
    // - hsl/hsla functions
    // - Most CSS named colors
    color_str.parse::<CssColor>().ok().map(|css_color| {
        [
            css_color.r as f32 / 255.0,
            css_color.g as f32 / 255.0,
            css_color.b as f32 / 255.0,
            css_color.a,
        ]
    })
}

fn parse_hex_alpha_color(color_str: &str) -> Option<[f32; 4]> {
    let hex = color_str.strip_prefix('#')?;
    if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    match hex.len() {
        4 => {
            let mut chars = hex.chars();
            Some([
                repeated_hex_digit(chars.next()?)? as f32 / 255.0,
                repeated_hex_digit(chars.next()?)? as f32 / 255.0,
                repeated_hex_digit(chars.next()?)? as f32 / 255.0,
                repeated_hex_digit(chars.next()?)? as f32 / 255.0,
            ])
        }
        8 => Some([
            hex_byte(&hex[0..2])? as f32 / 255.0,
            hex_byte(&hex[2..4])? as f32 / 255.0,
            hex_byte(&hex[4..6])? as f32 / 255.0,
            hex_byte(&hex[6..8])? as f32 / 255.0,
        ]),
        _ => None,
    }
}

fn repeated_hex_digit(ch: char) -> Option<u8> {
    let digit = ch.to_digit(16)? as u8;
    Some(digit * 17)
}

fn hex_byte(hex: &str) -> Option<u8> {
    u8::from_str_radix(hex, 16).ok()
}

/// Parse the same syntax as [`parse_color_string`], returning an error on failure.
///
/// The error preserves the original input, including surrounding whitespace.
/// This function adds error details without changing which inputs are accepted.
pub fn parse_color_string_strict(color_str: &str) -> Result<[f32; 4], ColorParseError> {
    parse_color_string(color_str).ok_or_else(|| ColorParseError::new(color_str))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_syntax_produces_normalized_rgba() {
        for (input, expected) in [
            ("#f0a", [1.0, 0.0, 170.0 / 255.0, 1.0]),
            ("#Ff00Aa", [1.0, 0.0, 170.0 / 255.0, 1.0]),
            ("#f0a8", [1.0, 0.0, 170.0 / 255.0, 136.0 / 255.0]),
            ("#ff00aa80", [1.0, 0.0, 170.0 / 255.0, 128.0 / 255.0]),
            ("rgb(255, 128, 64)", [1.0, 128.0 / 255.0, 64.0 / 255.0, 1.0]),
            (
                "rgba(255, 128, 64, 0.5)",
                [1.0, 128.0 / 255.0, 64.0 / 255.0, 0.5],
            ),
            ("hsl(120, 100%, 50%)", [0.0, 1.0, 0.0, 1.0]),
            ("hsla(120, 100%, 50%, 0.5)", [0.0, 1.0, 0.0, 0.5]),
            ("red", [1.0, 0.0, 0.0, 1.0]),
            (
                "rebeccapurple",
                [102.0 / 255.0, 51.0 / 255.0, 153.0 / 255.0, 1.0],
            ),
            ("transparent", [0.0, 0.0, 0.0, 0.0]),
        ] {
            assert_eq!(parse_color_string(input), Some(expected), "{input:?}");
        }
    }

    #[test]
    fn surrounding_whitespace_is_accepted_for_extended_colors() {
        for (input, expected) in [
            (" \t#f0a8\n", [1.0, 0.0, 170.0 / 255.0, 136.0 / 255.0]),
            ("\n#ff00aa80 \t", [1.0, 0.0, 170.0 / 255.0, 128.0 / 255.0]),
            (
                "\tRebeccaPurple \n",
                [102.0 / 255.0, 51.0 / 255.0, 153.0 / 255.0, 1.0],
            ),
        ] {
            assert_eq!(parse_color_string_strict(input), Ok(expected), "{input:?}");
        }
    }

    #[test]
    fn invalid_unicode_hex_returns_error() {
        for input in ["#aébcdef", "#abcédef", "#abécdef", "#éab", "#💚abcd"] {
            assert!(parse_color_string_strict(input).is_err(), "{input}");
        }
    }

    #[test]
    fn malformed_colors_return_none() {
        for input in ["", "notacolor", "#gg00ff", "#f0ag", "#ff00aa8g", "#12345"] {
            assert!(parse_color_string(input).is_none(), "{input:?}");
        }
    }

    #[test]
    fn strict_error_preserves_original_input() {
        let input = "  notacolor  ";
        let error = parse_color_string_strict(input).unwrap_err();
        assert_eq!(error.input(), input);
        assert!(error.to_string().contains(input));
    }
}
