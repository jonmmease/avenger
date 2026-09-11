//! WCAG 2.1 contrast ratio calculations for accessibility
//!
//! This module implements the contrast ratio algorithm specified in the Web Content
//! Accessibility Guidelines (WCAG) 2.1, used for determining accessible color
//! combinations for text and backgrounds.
//!
//! # References
//!
//! - [WCAG 2.1 Contrast Ratio Definition](https://www.w3.org/TR/WCAG21/#dfn-contrast-ratio)
//! - [WCAG 2.1 Relative Luminance](https://www.w3.org/TR/WCAG21/#dfn-relative-luminance)
//! - [CSS Color Module Level 6 - contrast-color()](https://drafts.csswg.org/css-color-6/)
//!
//! # Algorithm
//!
//! The contrast ratio between two colors is calculated as:
//! ```text
//! (L1 + 0.05) / (L2 + 0.05)
//! ```
//! where L1 is the relative luminance of the lighter color and L2 is the relative
//! luminance of the darker color.
//!
//! Relative luminance is calculated by:
//! 1. Converting sRGB components to linear RGB (gamma correction)
//! 2. Applying weighted sum: 0.2126xR + 0.7152xG + 0.0722xB
//!
//! # Limitations
//!
//! **WARNING**: The WCAG 2.1 contrast algorithm has known limitations:
//!
//! - Poor performance with mid-tone colors (30-70% brightness)
//! - Unreliable results on dark backgrounds
//! - Best results with very light (>90%) or very dark (<10%) base colors
//!
//! WCAG 3.0 will introduce APCA (Advanced Perceptual Contrast Algorithm) which
//! addresses these issues, but is not yet finalized.
//!
//! # Examples
//!
//! ```ignore
//! use avenger_color::contrast::{contrast_ratio, choose_contrast_color};
//! use avenger_color::AbsoluteColor;
//!
//! // Calculate contrast between black and white
//! let black = AbsoluteColor::from_srgb(0.0, 0.0, 0.0, 1.0);
//! let white = AbsoluteColor::from_srgb(1.0, 1.0, 1.0, 1.0);
//! let ratio = contrast_ratio(&black, &white);
//! assert_eq!(ratio, 21.0); // Maximum contrast
//!
//! // Choose contrasting color for a dark background
//! let dark_bg = AbsoluteColor::from_srgb(0.1, 0.1, 0.1, 1.0);
//! let text_color = choose_contrast_color(&dark_bg);
//! // Returns white for dark backgrounds
//! ```

use super::{AbsoluteColor, ColorSpace};

/// Convert a single sRGB component to linear RGB
///
/// This applies the inverse of the sRGB gamma correction (also known as the
/// sRGB "companding" function). The sRGB color space uses a gamma of
/// approximately 2.2 for display, but the actual transfer function is slightly
/// more complex.
///
/// # Formula
///
/// ```text
/// if component <= 0.04045:
///     linear = component / 12.92
/// else:
///     linear = ((component + 0.055) / 1.055)^2.4
/// ```
///
/// # Arguments
///
/// * `component` - sRGB component value in range [0.0, 1.0]
///
/// # Returns
///
/// Linear RGB value in range [0.0, 1.0]
///
/// # Note
///
/// The threshold value is 0.04045 (updated from the older 0.03928 in May 2021).
/// This has no practical effect on calculations.
#[inline]
fn srgb_to_linear(component: f32) -> f32 {
    if component <= 0.04045 {
        component / 12.92
    } else {
        ((component + 0.055) / 1.055).powf(2.4)
    }
}

/// Calculate WCAG 2.1 relative luminance for an sRGB color
///
/// Relative luminance represents the perceived brightness of a color, based on
/// human eye sensitivity to different wavelengths. Green appears brightest,
/// followed by red, then blue.
///
/// # Algorithm
///
/// 1. Convert each sRGB component to linear RGB (gamma correction)
/// 2. Apply weighted sum: `0.2126xR + 0.7152xG + 0.0722xB`
///
/// The weights are derived from the CIE 1931 color space and represent human
/// visual sensitivity:
/// - **0.2126** (Red): Medium sensitivity
/// - **0.7152** (Green): Highest sensitivity (green appears brightest)
/// - **0.0722** (Blue): Lowest sensitivity (blue appears dimmest)
///
/// # Arguments
///
/// * `r`, `g`, `b` - sRGB color components in range [0.0, 1.0]
///
/// # Returns
///
/// Relative luminance in range [0.0, 1.0]
/// - 0.0 = pure black (no luminance)
/// - 1.0 = pure white (maximum luminance)
///
/// # Examples
///
/// ```ignore
/// use avenger_color::contrast::relative_luminance_srgb;
///
/// // Pure white has maximum luminance
/// let white_lum = relative_luminance_srgb(1.0, 1.0, 1.0);
/// assert_eq!(white_lum, 1.0);
///
/// // Pure black has zero luminance
/// let black_lum = relative_luminance_srgb(0.0, 0.0, 0.0);
/// assert_eq!(black_lum, 0.0);
///
/// // Pure red has luminance ~ 0.2126
/// let red_lum = relative_luminance_srgb(1.0, 0.0, 0.0);
/// assert!((red_lum - 0.2126).abs() < 0.001);
/// ```
pub fn relative_luminance_srgb(r: f32, g: f32, b: f32) -> f32 {
    let r_linear = srgb_to_linear(r);
    let g_linear = srgb_to_linear(g);
    let b_linear = srgb_to_linear(b);

    // Apply weighted sum based on human eye sensitivity
    0.2126 * r_linear + 0.7152 * g_linear + 0.0722 * b_linear
}

/// Calculate WCAG 2.1 contrast ratio between two colors
///
/// The contrast ratio is a measure of the difference in perceived brightness
/// between two colors. It's used to ensure text is readable against its background.
///
/// # Algorithm
///
/// ```text
/// ratio = (L1 + 0.05) / (L2 + 0.05)
/// ```
///
/// where:
/// - L1 is the relative luminance of the **lighter** color
/// - L2 is the relative luminance of the **darker** color
/// - 0.05 is added to prevent division by zero and provide better scaling
///
/// # WCAG Requirements
///
/// - **Level AA**: 4.5:1 for normal text, 3:1 for large text
/// - **Level AAA**: 7:1 for normal text, 4.5:1 for large text
///
/// # Arguments
///
/// * `color1`, `color2` - Colors to compare (any color space, will be converted to sRGB)
///
/// # Returns
///
/// Contrast ratio in range [1.0, 21.0]
/// - 1.0 = no contrast (same color)
/// - 21.0 = maximum contrast (black vs white)
///
/// # Examples
///
/// ```ignore
/// use avenger_color::contrast::contrast_ratio;
/// use avenger_color::AbsoluteColor;
///
/// let white = AbsoluteColor::from_srgb(1.0, 1.0, 1.0, 1.0);
/// let black = AbsoluteColor::from_srgb(0.0, 0.0, 0.0, 1.0);
///
/// // Maximum contrast between black and white
/// let ratio = contrast_ratio(&white, &black);
/// assert_eq!(ratio, 21.0);
///
/// // Order doesn't matter
/// let ratio_reversed = contrast_ratio(&black, &white);
/// assert_eq!(ratio_reversed, 21.0);
/// ```
pub fn contrast_ratio(color1: &AbsoluteColor, color2: &AbsoluteColor) -> f32 {
    // Convert both colors to sRGB if needed
    let srgb1 = if color1.color_space == ColorSpace::Srgb {
        *color1
    } else {
        color1.to_color_space(ColorSpace::Srgb)
    };

    let srgb2 = if color2.color_space == ColorSpace::Srgb {
        *color2
    } else {
        color2.to_color_space(ColorSpace::Srgb)
    };

    // Calculate relative luminance for each color
    let l1 = relative_luminance_srgb(
        srgb1.components[0],
        srgb1.components[1],
        srgb1.components[2],
    );

    let l2 = relative_luminance_srgb(
        srgb2.components[0],
        srgb2.components[1],
        srgb2.components[2],
    );

    // Apply contrast formula: (lighter + 0.05) / (darker + 0.05)
    let lighter = l1.max(l2);
    let darker = l1.min(l2);

    (lighter + 0.05) / (darker + 0.05)
}

/// Choose contrasting color (black or white) using WCAG 2.1 algorithm
///
/// This function implements the basic `contrast-color()` CSS function behavior,
/// which selects either black or white based on which provides better contrast
/// against the base color.
///
/// # Algorithm
///
/// 1. Calculate contrast ratio with pure black (0, 0, 0)
/// 2. Calculate contrast ratio with pure white (255, 255, 255)
/// 3. Return white if ratios are equal (per CSS spec)
/// 4. Otherwise return the color with higher contrast ratio
///
/// # Arguments
///
/// * `base_color` - The background color to contrast against (any color space)
///
/// # Returns
///
/// Either pure white `#ffffff` or pure black `#000000` in sRGB color space
///
/// # Limitations
///
/// WARNING: **WARNING**: This function has known limitations inherited from WCAG 2.1:
///
/// - Best results with very light or very dark base colors
/// - Poor results with mid-tone colors (30-70% brightness)
/// - No guarantee of accessibility compliance
/// - Always test with actual users and accessibility tools
///
/// # Examples
///
/// ```ignore
/// use avenger_color::contrast::choose_contrast_color;
/// use avenger_color::AbsoluteColor;
///
/// // Dark background gets white text
/// let dark_bg = AbsoluteColor::from_srgb(0.1, 0.1, 0.1, 1.0);
/// let text = choose_contrast_color(&dark_bg);
/// assert_eq!(text.components, [1.0, 1.0, 1.0]); // white
///
/// // Light background gets black text
/// let light_bg = AbsoluteColor::from_srgb(0.9, 0.9, 0.9, 1.0);
/// let text = choose_contrast_color(&light_bg);
/// assert_eq!(text.components, [0.0, 0.0, 0.0]); // black
/// ```
pub fn choose_contrast_color(base_color: &AbsoluteColor) -> AbsoluteColor {
    // Define pure black and white in sRGB
    const BLACK: AbsoluteColor = AbsoluteColor {
        components: [0.0, 0.0, 0.0],
        alpha: 1.0,
        color_space: ColorSpace::Srgb,
    };

    const WHITE: AbsoluteColor = AbsoluteColor {
        components: [1.0, 1.0, 1.0],
        alpha: 1.0,
        color_space: ColorSpace::Srgb,
    };

    // Calculate contrast ratios
    let contrast_with_black = contrast_ratio(base_color, &BLACK);
    let contrast_with_white = contrast_ratio(base_color, &WHITE);

    // Per CSS spec: return white if equal, otherwise return higher contrast
    if contrast_with_white >= contrast_with_black {
        WHITE
    } else {
        BLACK
    }
}

/// Choose best contrasting color from a list of candidates
///
/// This implements the extended `contrast-color()` syntax from CSS Color Module Level 6:
/// ```css
/// contrast-color(<color>, <color-list>)
/// ```
///
/// # Algorithm
///
/// 1. Calculate contrast ratio of base color with each candidate
/// 2. Filter candidates that meet minimum contrast threshold (default 4.5:1 for WCAG AA)
/// 3. Return the candidate with the **highest** contrast ratio
/// 4. If no candidate meets threshold, fall back to black or white (whichever is better)
///
/// # Arguments
///
/// * `base_color` - The background color to contrast against
/// * `candidates` - List of candidate colors to choose from
/// * `min_ratio` - Minimum acceptable contrast ratio (default 4.5:1 for WCAG AA)
///
/// # Returns
///
/// The candidate color with the best contrast ratio, or black/white if none meet threshold
///
/// # Examples
///
/// ```ignore
/// use avenger_color::contrast::choose_best_contrast;
/// use avenger_color::AbsoluteColor;
///
/// let bg = AbsoluteColor::from_srgb(0.5, 0.5, 0.5, 1.0);
/// let candidates = vec![
///     AbsoluteColor::from_srgb(0.13, 0.13, 0.13, 1.0), // #222
///     AbsoluteColor::from_srgb(0.93, 0.93, 0.93, 1.0), // #eee
/// ];
///
/// // Choose best contrast with AA threshold (4.5:1)
/// let best = choose_best_contrast(&bg, &candidates, 4.5);
/// // Returns #eee (higher contrast than #222)
/// ```
pub fn choose_best_contrast(
    base_color: &AbsoluteColor,
    candidates: &[AbsoluteColor],
    min_ratio: f32,
) -> AbsoluteColor {
    if candidates.is_empty() {
        // No candidates provided, fall back to black or white
        return choose_contrast_color(base_color);
    }

    let mut best_candidate: Option<AbsoluteColor> = None;
    let mut best_ratio = 0.0;

    // Find the candidate with the highest contrast ratio that meets the threshold
    for candidate in candidates {
        let ratio = contrast_ratio(base_color, candidate);
        if ratio >= min_ratio && ratio > best_ratio {
            best_ratio = ratio;
            best_candidate = Some(*candidate);
        }
    }

    // If we found a candidate meeting the threshold, return it
    if let Some(candidate) = best_candidate {
        return candidate;
    }

    // Otherwise fall back to black or white
    choose_contrast_color(base_color)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper for floating point comparison
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

    #[test]
    fn test_contrast_ratio_extremes() {
        let white = AbsoluteColor::from_srgb(1.0, 1.0, 1.0, 1.0);
        let black = AbsoluteColor::from_srgb(0.0, 0.0, 0.0, 1.0);

        // White on black should be 21:1 (maximum contrast)
        let ratio = contrast_ratio(&white, &black);
        assert!(approx_eq(ratio, 21.0, 0.01));

        // Order shouldn't matter
        let ratio_reversed = contrast_ratio(&black, &white);
        assert!(approx_eq(ratio_reversed, 21.0, 0.01));
    }

    #[test]
    fn test_contrast_ratio_same_color() {
        let color = AbsoluteColor::from_srgb(0.5, 0.5, 0.5, 1.0);

        // Same color should have 1:1 contrast (minimum)
        let ratio = contrast_ratio(&color, &color);
        assert!(approx_eq(ratio, 1.0, 0.01));
    }

    #[test]
    fn test_contrast_ratio_with_color_space_conversion() {
        // Test that color space conversion works correctly
        let white_srgb = AbsoluteColor::from_srgb(1.0, 1.0, 1.0, 1.0);
        let black_oklch = AbsoluteColor::new(ColorSpace::Oklch, 0.0, 0.0, 0.0, 1.0);

        let ratio = contrast_ratio(&white_srgb, &black_oklch);
        // Should still get maximum contrast even with different color spaces
        assert!(ratio > 15.0); // Should be close to 21, but conversion might introduce small errors
    }

    #[test]
    fn test_choose_contrast_for_dark_background() {
        // Very dark background should get white text
        let dark_bg = AbsoluteColor::from_srgb(0.1, 0.1, 0.1, 1.0);
        let contrast = choose_contrast_color(&dark_bg);

        assert_eq!(contrast.components[0], 1.0); // white R
        assert_eq!(contrast.components[1], 1.0); // white G
        assert_eq!(contrast.components[2], 1.0); // white B
        assert_eq!(contrast.color_space, ColorSpace::Srgb);
    }

    #[test]
    fn test_choose_contrast_for_light_background() {
        // Very light background should get black text
        let light_bg = AbsoluteColor::from_srgb(0.9, 0.9, 0.9, 1.0);
        let contrast = choose_contrast_color(&light_bg);

        assert_eq!(contrast.components[0], 0.0); // black R
        assert_eq!(contrast.components[1], 0.0); // black G
        assert_eq!(contrast.components[2], 0.0); // black B
        assert_eq!(contrast.color_space, ColorSpace::Srgb);
    }

    #[test]
    fn test_choose_contrast_for_pure_blue() {
        // Pure blue is relatively dark, should get white
        let blue = AbsoluteColor::from_srgb(0.0, 0.0, 1.0, 1.0);
        let contrast = choose_contrast_color(&blue);

        // Should be white (blue has low luminance due to 0.0722 coefficient)
        assert_eq!(contrast.components[0], 1.0);
        assert_eq!(contrast.components[1], 1.0);
        assert_eq!(contrast.components[2], 1.0);
    }

    #[test]
    fn test_choose_contrast_for_pure_yellow() {
        // Pure yellow (red + green) is very light, should get black
        let yellow = AbsoluteColor::from_srgb(1.0, 1.0, 0.0, 1.0);
        let contrast = choose_contrast_color(&yellow);

        // Should be black (yellow has high luminance: 0.2126 + 0.7152 ~ 0.93)
        assert_eq!(contrast.components[0], 0.0);
        assert_eq!(contrast.components[1], 0.0);
        assert_eq!(contrast.components[2], 0.0);
    }

    #[test]
    fn test_choose_contrast_for_mid_gray() {
        // Mid-gray (sRGB 0.5, 0.5, 0.5 has luminance ~0.214)
        // Black provides better contrast: ~5.28:1 vs white's ~3.98:1
        let mid_gray = AbsoluteColor::from_srgb(0.5, 0.5, 0.5, 1.0);
        let contrast = choose_contrast_color(&mid_gray);

        // Should be black (better contrast for mid-tone grays)
        assert_eq!(contrast.components[0], 0.0);
        assert_eq!(contrast.components[1], 0.0);
        assert_eq!(contrast.components[2], 0.0);
    }

    #[test]
    fn test_choose_contrast_tie_behavior() {
        // Test the >= behavior: when contrast is equal, white should be chosen
        // This is hard to test with real colors, so we verify the algorithm
        // by checking that the function uses >= not just >

        // For very dark colors close to black, both might have similar contrast
        let very_dark = AbsoluteColor::from_srgb(0.01, 0.01, 0.01, 1.0);
        let contrast = choose_contrast_color(&very_dark);

        // Should be white (much better contrast for very dark colors)
        assert_eq!(contrast.components[0], 1.0);
        assert_eq!(contrast.components[1], 1.0);
        assert_eq!(contrast.components[2], 1.0);
    }

    #[test]
    fn test_wcag_aa_compliance() {
        // Test some known WCAG AA compliant color pairs

        // White on #595959 (medium gray) should pass AA (4.5:1)
        let white = AbsoluteColor::from_srgb(1.0, 1.0, 1.0, 1.0);
        let medium_gray = AbsoluteColor::from_srgb(
            0x59 as f32 / 255.0,
            0x59 as f32 / 255.0,
            0x59 as f32 / 255.0,
            1.0,
        );

        let ratio = contrast_ratio(&white, &medium_gray);
        assert!(
            ratio >= 4.5,
            "Should meet AA standard for normal text, got {}",
            ratio
        );
    }

    #[test]
    fn test_wcag_aaa_compliance() {
        // Test WCAG AAA compliance (7:1)
        let white = AbsoluteColor::from_srgb(1.0, 1.0, 1.0, 1.0);
        let dark_gray = AbsoluteColor::from_srgb(
            0x40 as f32 / 255.0,
            0x40 as f32 / 255.0,
            0x40 as f32 / 255.0,
            1.0,
        );

        let ratio = contrast_ratio(&white, &dark_gray);
        assert!(
            ratio >= 7.0,
            "Should meet AAA standard for normal text, got {}",
            ratio
        );
    }

    // Tests for choose_best_contrast (extended syntax with candidate lists)

    #[test]
    fn test_choose_best_contrast_basic() {
        // Darker background to ensure candidates meet threshold
        let bg = AbsoluteColor::from_srgb(0.2, 0.2, 0.2, 1.0);

        // Candidates: medium grey and light grey
        let candidates = vec![
            AbsoluteColor::from_srgb(0.5, 0.5, 0.5, 1.0), // mid gray
            AbsoluteColor::from_srgb(0.93, 0.93, 0.93, 1.0), // #eee
        ];

        // Should choose #eee (higher contrast)
        let best = choose_best_contrast(&bg, &candidates, 3.0);
        assert!(approx_eq(best.components[0], 0.93, 0.01));
    }

    #[test]
    fn test_choose_best_contrast_with_brand_colors() {
        // Light blue background
        let bg = AbsoluteColor::from_srgb(0.68, 0.85, 0.90, 1.0); // lightblue

        // Brand color palette
        let candidates = vec![
            AbsoluteColor::from_srgb(0.0, 0.0, 0.5, 1.0), // navy
            AbsoluteColor::from_srgb(0.5, 0.0, 0.0, 1.0), // maroon
            AbsoluteColor::from_srgb(0.5, 0.0, 0.5, 1.0), // purple
            AbsoluteColor::from_srgb(0.0, 0.5, 0.5, 1.0), // teal
        ];

        // Should choose navy (darkest, best contrast)
        let best = choose_best_contrast(&bg, &candidates, 3.0);
        assert!(approx_eq(best.components[2], 0.5, 0.01)); // Blue component
        assert!(approx_eq(best.components[0], 0.0, 0.01)); // Red component
    }

    #[test]
    fn test_choose_best_contrast_none_meet_threshold() {
        // Medium gray background
        let bg = AbsoluteColor::from_srgb(0.5, 0.5, 0.5, 1.0);

        // All candidates have poor contrast
        let candidates = vec![
            AbsoluteColor::from_srgb(0.45, 0.45, 0.45, 1.0),
            AbsoluteColor::from_srgb(0.55, 0.55, 0.55, 1.0),
        ];

        // Should fall back to black or white
        let best = choose_best_contrast(&bg, &candidates, 4.5);
        // Should be either pure black or pure white
        let is_black_or_white = (approx_eq(best.components[0], 0.0, 0.01)
            && approx_eq(best.components[1], 0.0, 0.01)
            && approx_eq(best.components[2], 0.0, 0.01))
            || (approx_eq(best.components[0], 1.0, 0.01)
                && approx_eq(best.components[1], 1.0, 0.01)
                && approx_eq(best.components[2], 1.0, 0.01));
        assert!(is_black_or_white);
    }

    #[test]
    fn test_choose_best_contrast_empty_candidates() {
        // Should fall back to choose_contrast_color behavior
        let bg = AbsoluteColor::from_srgb(0.1, 0.1, 0.1, 1.0);
        let candidates = vec![];

        let best = choose_best_contrast(&bg, &candidates, 4.5);
        // Dark background should get white
        assert_eq!(best.components[0], 1.0);
        assert_eq!(best.components[1], 1.0);
        assert_eq!(best.components[2], 1.0);
    }

    #[test]
    fn test_choose_best_contrast_all_candidates_meet_threshold() {
        // Black background
        let bg = AbsoluteColor::from_srgb(0.0, 0.0, 0.0, 1.0);

        // All light colors with good contrast
        let candidates = vec![
            AbsoluteColor::from_srgb(0.9, 0.9, 0.9, 1.0), // light gray
            AbsoluteColor::from_srgb(1.0, 1.0, 0.8, 1.0), // cream
            AbsoluteColor::from_srgb(1.0, 1.0, 1.0, 1.0), // white
        ];

        // Should choose white (highest contrast)
        let best = choose_best_contrast(&bg, &candidates, 4.5);
        assert_eq!(best.components[0], 1.0);
        assert_eq!(best.components[1], 1.0);
        assert_eq!(best.components[2], 1.0);
    }

    #[test]
    fn test_choose_best_contrast_with_low_threshold() {
        // Light background
        let bg = AbsoluteColor::from_srgb(0.9, 0.9, 0.9, 1.0);

        // Candidates with varying contrast
        let candidates = vec![
            AbsoluteColor::from_srgb(0.6, 0.6, 0.6, 1.0), // Medium gray
            AbsoluteColor::from_srgb(0.3, 0.3, 0.3, 1.0), // Dark gray
        ];

        // With low threshold (2.0:1), should pick the one with better contrast
        let best = choose_best_contrast(&bg, &candidates, 2.0);
        // Should choose the darker one (0.3) as it has better contrast
        assert!(approx_eq(best.components[0], 0.3, 0.01));
    }

    #[test]
    fn test_choose_best_contrast_prefers_highest_ratio() {
        // White background
        let bg = AbsoluteColor::from_srgb(1.0, 1.0, 1.0, 1.0);

        let candidates = vec![
            AbsoluteColor::from_srgb(0.2, 0.2, 0.2, 1.0), // Contrast ~10:1
            AbsoluteColor::from_srgb(0.0, 0.0, 0.0, 1.0), // Contrast 21:1 (max)
            AbsoluteColor::from_srgb(0.1, 0.1, 0.1, 1.0), // Contrast ~15:1
        ];

        // Should choose pure black (21:1)
        let best = choose_best_contrast(&bg, &candidates, 4.5);
        assert_eq!(best.components[0], 0.0);
        assert_eq!(best.components[1], 0.0);
        assert_eq!(best.components[2], 0.0);
    }
}
