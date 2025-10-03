//! Color space conversion algorithms
//!
//! Ported from Mozilla Stylo:
//! https://searchfox.org/mozilla-central/source/servo/components/style/color/convert.rs
//!
//! Algorithms and constants are from the CSS Color Module Level 4:
//! https://drafts.csswg.org/css-color-4/#color-conversion-code

/// Normalize hue into the range [0, 360).
///
/// This handles wraparound for hue values, ensuring they're always in the standard range.
#[inline]
pub fn normalize_hue(hue: f32) -> f32 {
    hue - 360.0 * (hue / 360.0).floor()
}

/// Calculate the hue from RGB components and return it along with the min and max RGB values.
///
/// This is a helper function used by both rgb_to_hsl and rgb_to_hwb.
///
/// # Returns
///
/// (hue, min, max) where:
/// - hue is in degrees [0, 360) or NaN if undefined (for achromatic colors)
/// - min is the minimum of (red, green, blue)
/// - max is the maximum of (red, green, blue)
#[inline]
fn rgb_to_hue_min_max(red: f32, green: f32, blue: f32) -> (f32, f32, f32) {
    let max = red.max(green).max(blue);
    let min = red.min(green).min(blue);

    let delta = max - min;

    let hue = if delta != 0.0 {
        60.0 * if max == red {
            (green - blue) / delta + if green < blue { 6.0 } else { 0.0 }
        } else if max == green {
            (blue - red) / delta + 2.0
        } else {
            (red - green) / delta + 4.0
        }
    } else {
        // Achromatic color - hue is undefined, represented as NaN
        f32::NAN
    };

    (hue, min, max)
}

/// Convert from HSL notation to RGB notation.
///
/// # Arguments
///
/// * `hue` - Hue in degrees [0, 360]. Values outside this range will be normalized.
/// * `saturation` - Saturation as a percentage [0, 100]
/// * `lightness` - Lightness as a percentage [0, 100]
///
/// # Returns
///
/// (red, green, blue) where each component is in the range [0.0, 1.0]
///
/// # Reference
///
/// https://drafts.csswg.org/css-color-4/#hsl-to-rgb
#[inline]
pub fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> (f32, f32, f32) {
    /// Helper function for HSL to RGB conversion
    fn hue_to_rgb(t1: f32, t2: f32, hue: f32) -> f32 {
        let hue = normalize_hue(hue);

        if hue * 6.0 < 360.0 {
            t1 + (t2 - t1) * hue / 60.0
        } else if hue * 2.0 < 360.0 {
            t2
        } else if hue * 3.0 < 720.0 {
            t1 + (t2 - t1) * (240.0 - hue) / 60.0
        } else {
            t1
        }
    }

    // Normalize hue (handle NaN by converting to 0.0)
    let hue = if hue.is_nan() { 0.0 } else { hue };

    // Convert saturation and lightness from percentage to [0, 1]
    let saturation = saturation / 100.0;
    let lightness = lightness / 100.0;

    let t2 = if lightness <= 0.5 {
        lightness * (saturation + 1.0)
    } else {
        lightness + saturation - lightness * saturation
    };
    let t1 = lightness * 2.0 - t2;

    (
        hue_to_rgb(t1, t2, hue + 120.0),
        hue_to_rgb(t1, t2, hue),
        hue_to_rgb(t1, t2, hue - 120.0),
    )
}

/// Convert from RGB notation to HSL notation.
///
/// # Arguments
///
/// * `red` - Red component in the range [0.0, 1.0]
/// * `green` - Green component in the range [0.0, 1.0]
/// * `blue` - Blue component in the range [0.0, 1.0]
///
/// # Returns
///
/// (hue, saturation, lightness) where:
/// - hue is in degrees [0, 360) or NaN for achromatic colors
/// - saturation is a percentage [0, 100]
/// - lightness is a percentage [0, 100]
///
/// # Reference
///
/// https://drafts.csswg.org/css-color-4/#rgb-to-hsl
#[inline]
pub fn rgb_to_hsl(red: f32, green: f32, blue: f32) -> (f32, f32, f32) {
    let (hue, min, max) = rgb_to_hue_min_max(red, green, blue);

    let lightness = (min + max) / 2.0;
    let delta = max - min;

    let saturation = if delta != 0.0 {
        if lightness == 0.0 || lightness == 1.0 {
            0.0
        } else {
            (max - lightness) / lightness.min(1.0 - lightness)
        }
    } else {
        0.0
    };

    (hue, saturation * 100.0, lightness * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to compare floats with tolerance
    fn assert_float_eq(a: f32, b: f32, epsilon: f32, msg: &str) {
        assert!(
            (a - b).abs() < epsilon,
            "{}: expected {}, got {} (diff: {})",
            msg,
            b,
            a,
            (a - b).abs()
        );
    }

    /// Helper to compare RGB tuples with tolerance
    fn assert_rgb_eq(actual: (f32, f32, f32), expected: (f32, f32, f32), epsilon: f32) {
        assert_float_eq(actual.0, expected.0, epsilon, "red");
        assert_float_eq(actual.1, expected.1, epsilon, "green");
        assert_float_eq(actual.2, expected.2, epsilon, "blue");
    }

    /// Helper to compare HSL tuples with tolerance, handling NaN hue
    fn assert_hsl_eq(actual: (f32, f32, f32), expected: (f32, f32, f32), epsilon: f32) {
        // For achromatic colors, hue can be NaN or any value
        if expected.1 == 0.0 || expected.2 == 0.0 || expected.2 == 100.0 {
            // Saturation is 0 or lightness is 0 or 100 - hue is powerless
            // Don't check hue, it can be anything
        } else if actual.0.is_nan() && expected.0.is_nan() {
            // Both NaN is okay
        } else {
            let hue_diff = (actual.0 - expected.0).abs();
            // Handle wraparound: 359 and 1 are close
            let hue_diff = hue_diff.min(360.0 - hue_diff);
            assert!(
                hue_diff < epsilon,
                "hue: expected {}, got {} (diff: {})",
                expected.0,
                actual.0,
                hue_diff
            );
        }
        assert_float_eq(actual.1, expected.1, epsilon, "saturation");
        assert_float_eq(actual.2, expected.2, epsilon, "lightness");
    }

    #[test]
    fn test_normalize_hue() {
        assert_float_eq(normalize_hue(0.0), 0.0, 0.01, "0 degrees");
        assert_float_eq(normalize_hue(360.0), 0.0, 0.01, "360 degrees wraps to 0");
        assert_float_eq(normalize_hue(720.0), 0.0, 0.01, "720 degrees wraps to 0");
        assert_float_eq(
            normalize_hue(-90.0),
            270.0,
            0.01,
            "-90 degrees wraps to 270",
        );
        assert_float_eq(normalize_hue(450.0), 90.0, 0.01, "450 degrees wraps to 90");
    }

    #[test]
    fn test_hsl_to_rgb_primary_colors() {
        // Red: hsl(0, 100%, 50%)
        assert_rgb_eq(hsl_to_rgb(0.0, 100.0, 50.0), (1.0, 0.0, 0.0), 0.01);

        // Green: hsl(120, 100%, 50%)
        assert_rgb_eq(hsl_to_rgb(120.0, 100.0, 50.0), (0.0, 1.0, 0.0), 0.01);

        // Blue: hsl(240, 100%, 50%)
        assert_rgb_eq(hsl_to_rgb(240.0, 100.0, 50.0), (0.0, 0.0, 1.0), 0.01);
    }

    #[test]
    fn test_hsl_to_rgb_secondary_colors() {
        // Yellow: hsl(60, 100%, 50%)
        assert_rgb_eq(hsl_to_rgb(60.0, 100.0, 50.0), (1.0, 1.0, 0.0), 0.01);

        // Cyan: hsl(180, 100%, 50%)
        assert_rgb_eq(hsl_to_rgb(180.0, 100.0, 50.0), (0.0, 1.0, 1.0), 0.01);

        // Magenta: hsl(300, 100%, 50%)
        assert_rgb_eq(hsl_to_rgb(300.0, 100.0, 50.0), (1.0, 0.0, 1.0), 0.01);
    }

    #[test]
    fn test_hsl_to_rgb_achromatic() {
        // Black: hsl(any, any%, 0%)
        assert_rgb_eq(hsl_to_rgb(0.0, 0.0, 0.0), (0.0, 0.0, 0.0), 0.01);
        assert_rgb_eq(hsl_to_rgb(180.0, 50.0, 0.0), (0.0, 0.0, 0.0), 0.01);

        // White: hsl(any, any%, 100%)
        assert_rgb_eq(hsl_to_rgb(0.0, 0.0, 100.0), (1.0, 1.0, 1.0), 0.01);
        assert_rgb_eq(hsl_to_rgb(240.0, 75.0, 100.0), (1.0, 1.0, 1.0), 0.01);

        // Gray: hsl(any, 0%, 50%)
        assert_rgb_eq(hsl_to_rgb(0.0, 0.0, 50.0), (0.5, 0.5, 0.5), 0.01);
        assert_rgb_eq(hsl_to_rgb(120.0, 0.0, 50.0), (0.5, 0.5, 0.5), 0.01);
    }

    #[test]
    fn test_hsl_to_rgb_various() {
        // Light pink: hsl(0, 100%, 75%)
        assert_rgb_eq(hsl_to_rgb(0.0, 100.0, 75.0), (1.0, 0.5, 0.5), 0.01);

        // Dark green: hsl(120, 100%, 25%)
        assert_rgb_eq(hsl_to_rgb(120.0, 100.0, 25.0), (0.0, 0.5, 0.0), 0.01);

        // Desaturated blue: hsl(240, 50%, 50%)
        assert_rgb_eq(hsl_to_rgb(240.0, 50.0, 50.0), (0.25, 0.25, 0.75), 0.01);
    }

    #[test]
    fn test_rgb_to_hsl_primary_colors() {
        // Red
        assert_hsl_eq(rgb_to_hsl(1.0, 0.0, 0.0), (0.0, 100.0, 50.0), 0.1);

        // Green
        assert_hsl_eq(rgb_to_hsl(0.0, 1.0, 0.0), (120.0, 100.0, 50.0), 0.1);

        // Blue
        assert_hsl_eq(rgb_to_hsl(0.0, 0.0, 1.0), (240.0, 100.0, 50.0), 0.1);
    }

    #[test]
    fn test_rgb_to_hsl_secondary_colors() {
        // Yellow
        assert_hsl_eq(rgb_to_hsl(1.0, 1.0, 0.0), (60.0, 100.0, 50.0), 0.1);

        // Cyan
        assert_hsl_eq(rgb_to_hsl(0.0, 1.0, 1.0), (180.0, 100.0, 50.0), 0.1);

        // Magenta
        assert_hsl_eq(rgb_to_hsl(1.0, 0.0, 1.0), (300.0, 100.0, 50.0), 0.1);
    }

    #[test]
    fn test_rgb_to_hsl_achromatic() {
        // Black
        let (_h, s, l) = rgb_to_hsl(0.0, 0.0, 0.0);
        assert_float_eq(s, 0.0, 0.1, "black saturation");
        assert_float_eq(l, 0.0, 0.1, "black lightness");
        // Hue is undefined for black (can be NaN)

        // White
        let (_h, s, l) = rgb_to_hsl(1.0, 1.0, 1.0);
        assert_float_eq(s, 0.0, 0.1, "white saturation");
        assert_float_eq(l, 100.0, 0.1, "white lightness");
        // Hue is undefined for white (can be NaN)

        // Gray
        let (_h, s, l) = rgb_to_hsl(0.5, 0.5, 0.5);
        assert_float_eq(s, 0.0, 0.1, "gray saturation");
        assert_float_eq(l, 50.0, 0.1, "gray lightness");
        // Hue is undefined for gray (can be NaN)
    }

    #[test]
    fn test_hsl_roundtrip() {
        // Test that converting HSL -> RGB -> HSL gives back the same values
        let test_cases = vec![
            (0.0, 100.0, 50.0),   // Red
            (120.0, 100.0, 50.0), // Green
            (240.0, 100.0, 50.0), // Blue
            (60.0, 75.0, 60.0),   // Light yellowish
            (180.0, 50.0, 40.0),  // Dark cyan
            (300.0, 80.0, 70.0),  // Light magenta
            (45.0, 60.0, 55.0),   // Orange-ish
            (200.0, 40.0, 45.0),  // Blue-ish
        ];

        for (h, s, l) in test_cases {
            let (r, g, b) = hsl_to_rgb(h, s, l);
            let (h2, s2, l2) = rgb_to_hsl(r, g, b);

            assert_hsl_eq((h2, s2, l2), (h, s, l), 0.1);
        }
    }

    #[test]
    fn test_rgb_roundtrip() {
        // Test that converting RGB -> HSL -> RGB gives back the same values
        let test_cases = vec![
            (1.0, 0.0, 0.0),   // Red
            (0.0, 1.0, 0.0),   // Green
            (0.0, 0.0, 1.0),   // Blue
            (1.0, 1.0, 0.0),   // Yellow
            (0.0, 1.0, 1.0),   // Cyan
            (1.0, 0.0, 1.0),   // Magenta
            (0.5, 0.5, 0.5),   // Gray
            (0.75, 0.25, 0.5), // Some color
            (0.2, 0.6, 0.8),   // Another color
        ];

        for (r, g, b) in test_cases {
            let (h, s, l) = rgb_to_hsl(r, g, b);
            let (r2, g2, b2) = hsl_to_rgb(h, s, l);

            assert_rgb_eq((r2, g2, b2), (r, g, b), 0.01);
        }
    }

    #[test]
    fn test_hue_wraparound() {
        // Test that hue values outside [0, 360) are handled correctly
        assert_rgb_eq(
            hsl_to_rgb(0.0, 100.0, 50.0),
            hsl_to_rgb(360.0, 100.0, 50.0),
            0.01,
        );
        assert_rgb_eq(
            hsl_to_rgb(120.0, 100.0, 50.0),
            hsl_to_rgb(480.0, 100.0, 50.0),
            0.01,
        );
        assert_rgb_eq(
            hsl_to_rgb(240.0, 100.0, 50.0),
            hsl_to_rgb(-120.0, 100.0, 50.0),
            0.01,
        );
    }

    #[test]
    fn test_nan_hue_handling() {
        // NaN hue should be treated as 0
        let (r1, g1, b1) = hsl_to_rgb(f32::NAN, 50.0, 50.0);
        let (r2, g2, b2) = hsl_to_rgb(0.0, 50.0, 50.0);
        assert_rgb_eq((r1, g1, b1), (r2, g2, b2), 0.01);
    }
}
