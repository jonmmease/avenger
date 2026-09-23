//! Relative luminance for sRGB colors.
//!
//! https://www.w3.org/TR/WCAG21/#dfn-relative-luminance

#[inline]
fn srgb_to_linear(component: f32) -> f32 {
    if component <= 0.04045 {
        component / 12.92
    } else {
        ((component + 0.055) / 1.055).powf(2.4)
    }
}

/// Calculate relative luminance from normalized sRGB components.
pub fn relative_luminance_srgb(r: f32, g: f32, b: f32) -> f32 {
    let r_linear = srgb_to_linear(r);
    let g_linear = srgb_to_linear(g);
    let b_linear = srgb_to_linear(b);

    // Apply weighted sum based on human eye sensitivity
    0.2126 * r_linear + 0.7152 * g_linear + 0.0722 * b_linear
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(a: f32, b: f32, epsilon: f32) -> bool {
        (a - b).abs() < epsilon
    }

    #[test]
    fn test_srgb_to_linear_threshold() {
        // Below threshold (0.04045), should use linear division
        let low = 0.03_f32;
        let low_linear = srgb_to_linear(low);
        assert!(approx_eq(low_linear, low / 12.92, 0.0001));

        // Above threshold, should use power function
        let high = 0.5_f32;
        let high_linear = srgb_to_linear(high);
        let expected = ((high + 0.055) / 1.055).powf(2.4);
        assert!(approx_eq(high_linear, expected, 0.0001));
    }

    #[test]
    fn test_srgb_to_linear_extremes() {
        // Pure black
        assert_eq!(srgb_to_linear(0.0), 0.0);

        // Pure white
        assert!(approx_eq(srgb_to_linear(1.0), 1.0, 0.0001));

        // Mid-tone (should be approximately 0.214)
        let mid_linear = srgb_to_linear(0.5);
        assert!(approx_eq(mid_linear, 0.2140, 0.001));
    }

    #[test]
    fn test_relative_luminance_pure_colors() {
        // Pure white should have luminance 1.0
        let white_lum = relative_luminance_srgb(1.0, 1.0, 1.0);
        assert!(approx_eq(white_lum, 1.0, 0.0001));

        // Pure black should have luminance 0.0
        let black_lum = relative_luminance_srgb(0.0, 0.0, 0.0);
        assert!(approx_eq(black_lum, 0.0, 0.0001));

        // Pure red: ~0.2126 (the red coefficient)
        let red_lum = relative_luminance_srgb(1.0, 0.0, 0.0);
        assert!(approx_eq(red_lum, 0.2126, 0.001));

        // Pure green: ~0.7152 (the green coefficient)
        let green_lum = relative_luminance_srgb(0.0, 1.0, 0.0);
        assert!(approx_eq(green_lum, 0.7152, 0.001));

        // Pure blue: ~0.0722 (the blue coefficient)
        let blue_lum = relative_luminance_srgb(0.0, 0.0, 1.0);
        assert!(approx_eq(blue_lum, 0.0722, 0.001));
    }
}
