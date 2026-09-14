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

/// Parse a color string using css-color-parser.
///
/// Supports CSS hex colors, rgb/rgba, hsl/hsla, and CSS named colors. Hex
/// alpha forms are handled here because css-color-parser 0.1.2 accepts
/// `#rgb`/`#rrggbb` but not `#rgba`/`#rrggbbaa`.
pub fn parse_color_string(color_str: &str) -> Option<[f32; 4]> {
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

/// Strict color parser that returns an error if the color string cannot be parsed.
pub fn parse_color_string_strict(color_str: &str) -> Result<[f32; 4], ColorParseError> {
    parse_color_string(color_str).ok_or_else(|| ColorParseError::new(color_str))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_unicode_hex_returns_error() {
        for input in ["#aébcdef", "#abcédef", "#abécdef", "#éab", "#💚abcd"] {
            assert!(parse_color_string_strict(input).is_err(), "{input}");
        }
    }

    #[test]
    fn test_parse_color_string_hex() {
        // Test 3-digit hex
        let [r, g, b, a] = parse_color_string("#f0a").unwrap();
        assert_eq!(r, 1.0);
        assert_eq!(g, 0.0);
        assert!((b - 170.0 / 255.0).abs() < 0.001);
        assert_eq!(a, 1.0);

        // Test 6-digit hex
        let [r, g, b, a] = parse_color_string("#ff00aa").unwrap();
        assert_eq!(r, 1.0);
        assert_eq!(g, 0.0);
        assert!((b - 170.0 / 255.0).abs() < 0.001);
        assert_eq!(a, 1.0);
    }

    #[test]
    fn test_parse_color_string_hex_alpha() {
        let [r, g, b, a] = parse_color_string("#f0a8").unwrap();
        assert_eq!(r, 1.0);
        assert_eq!(g, 0.0);
        assert!((b - 170.0 / 255.0).abs() < 0.001);
        assert!((a - 136.0 / 255.0).abs() < 0.001);

        let [r, g, b, a] = parse_color_string("#ff00aa80").unwrap();
        assert_eq!(r, 1.0);
        assert_eq!(g, 0.0);
        assert!((b - 170.0 / 255.0).abs() < 0.001);
        assert!((a - 128.0 / 255.0).abs() < 0.001);
    }

    #[test]
    fn test_parse_color_string_rgb() {
        // Test rgb function
        let [r, g, b, a] = parse_color_string("rgb(255, 128, 64)").unwrap();
        assert_eq!(r, 1.0);
        assert!((g - 128.0 / 255.0).abs() < 0.001);
        assert!((b - 64.0 / 255.0).abs() < 0.001);
        assert_eq!(a, 1.0);

        // Test rgba function
        let [r, g, b, a] = parse_color_string("rgba(255, 128, 64, 0.5)").unwrap();
        assert_eq!(r, 1.0);
        assert!((g - 128.0 / 255.0).abs() < 0.001);
        assert!((b - 64.0 / 255.0).abs() < 0.001);
        assert!((a - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_parse_color_string_named() {
        // Test common named colors
        let [r, g, b, a] = parse_color_string("red").unwrap();
        assert_eq!(r, 1.0);
        assert_eq!(g, 0.0);
        assert_eq!(b, 0.0);
        assert_eq!(a, 1.0);

        // Test rebeccapurple
        let [r, g, b, a] = parse_color_string("rebeccapurple").unwrap();
        assert!((r - 102.0 / 255.0).abs() < 0.001);
        assert!((g - 51.0 / 255.0).abs() < 0.001);
        assert!((b - 153.0 / 255.0).abs() < 0.001);
        assert_eq!(a, 1.0);
    }

    #[test]
    fn test_parse_invalid_color() {
        assert!(parse_color_string("notacolor").is_none());
        assert!(parse_color_string("#gg00ff").is_none());
    }

    #[test]
    fn test_parse_color_string_strict_error() {
        let err = parse_color_string_strict("notacolor").unwrap_err();
        assert_eq!(err.input(), "notacolor");
        assert_eq!(err.to_string(), "invalid color string 'notacolor'");
    }
}
