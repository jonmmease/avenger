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
//! ```
//! use avenger_color::contrast::{contrast_ratio, choose_contrast_color};
//! use avenger_color::AbsoluteColor;
//!
//! // Calculate contrast between black and white
//! let black = AbsoluteColor::from_srgb(0.0, 0.0, 0.0, 1.0);
//! let white = AbsoluteColor::from_srgb(1.0, 1.0, 1.0, 1.0);
//! let ratio = contrast_ratio(&black, &white);
//! assert!((ratio - 21.0).abs() < 1e-5); // Maximum contrast
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
/// ```
/// use avenger_color::contrast::relative_luminance_srgb;
///
/// // Pure white has maximum luminance
/// let white_lum = relative_luminance_srgb(1.0, 1.0, 1.0);
/// assert!((white_lum - 1.0).abs() < 1e-6);
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
/// Uses the clipped sRGB channels returned by [`AbsoluteColor::to_rgba`].
/// Alpha is ignored; composite translucent colors against their background first.
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
/// ```
/// use avenger_color::contrast::contrast_ratio;
/// use avenger_color::AbsoluteColor;
///
/// let white = AbsoluteColor::from_srgb(1.0, 1.0, 1.0, 1.0);
/// let black = AbsoluteColor::from_srgb(0.0, 0.0, 0.0, 1.0);
///
/// // Maximum contrast between black and white
/// let ratio = contrast_ratio(&white, &black);
/// assert!((ratio - 21.0).abs() < 1e-5);
///
/// // Order doesn't matter
/// let ratio_reversed = contrast_ratio(&black, &white);
/// assert!((ratio_reversed - 21.0).abs() < 1e-5);
/// ```
pub fn contrast_ratio(color1: &AbsoluteColor, color2: &AbsoluteColor) -> f32 {
    let [r1, g1, b1, _] = color1.to_rgba();
    let [r2, g2, b2, _] = color2.to_rgba();
    let l1 = relative_luminance_srgb(r1, g1, b1);
    let l2 = relative_luminance_srgb(r2, g2, b2);

    // Apply contrast formula: (lighter + 0.05) / (darker + 0.05)
    let lighter = l1.max(l2);
    let darker = l1.min(l2);

    (lighter + 0.05) / (darker + 0.05)
}

/// Choose contrasting color (black or white) using WCAG 2.1 algorithm
///
/// Selects either black or white based on which provides the higher
/// [`contrast_ratio`] against the base color. Alpha is ignored; composite
/// translucent colors before calling this function.
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
/// ```
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
/// Selects the highest WCAG contrast ratio that meets `min_ratio`.
/// Ties retain the first candidate.
/// Alpha is ignored; composite translucent colors before calling this function.
///
/// # Algorithm
///
/// 1. Calculate contrast ratio of base color with each candidate
/// 2. Filter candidates that meet the minimum contrast threshold
/// 3. Return the candidate with the **highest** contrast ratio
/// 4. If no candidate meets threshold, fall back to black or white (whichever is better)
///
/// # Arguments
///
/// * `base_color` - The background color to contrast against
/// * `candidates` - List of candidate colors to choose from
/// * `min_ratio` - Minimum acceptable contrast ratio (for example, 4.5 for WCAG AA normal text)
///
/// # Returns
///
/// The candidate color with the best contrast ratio, or black/white if none meet threshold
///
/// # Examples
///
/// ```
/// use avenger_color::contrast::choose_best_contrast;
/// use avenger_color::AbsoluteColor;
///
/// let bg = AbsoluteColor::from_srgb(0.2, 0.2, 0.2, 1.0);
/// let candidates = vec![
///     AbsoluteColor::from_srgb(0.13, 0.13, 0.13, 1.0), // #222
///     AbsoluteColor::from_srgb(0.93, 0.93, 0.93, 1.0), // #eee
/// ];
///
/// // Choose best contrast with AA threshold (4.5:1)
/// let best = choose_best_contrast(&bg, &candidates, 4.5);
/// assert_eq!(best, candidates[1]);
/// ```
pub fn choose_best_contrast(
    base_color: &AbsoluteColor,
    candidates: &[AbsoluteColor],
    min_ratio: f32,
) -> AbsoluteColor {
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

    const BLACK: AbsoluteColor = AbsoluteColor {
        components: [0.0; 3],
        alpha: 1.0,
        color_space: ColorSpace::Srgb,
    };
    const WHITE: AbsoluteColor = AbsoluteColor {
        components: [1.0; 3],
        ..BLACK
    };

    #[test]
    fn relative_luminance_matches_reference_values() {
        for (rgb, expected) in [
            ([0.0, 0.0, 0.0], 0.0),
            ([1.0, 1.0, 1.0], 1.0),
            ([1.0, 0.0, 0.0], 0.2126),
            ([0.0, 1.0, 0.0], 0.7152),
            ([0.0, 0.0, 1.0], 0.0722),
            ([0.03, 0.03, 0.03], 0.0023219814),
            ([0.5, 0.5, 0.5], 0.21404114),
        ] {
            let actual = relative_luminance_srgb(rgb[0], rgb[1], rgb[2]);
            assert!(
                (actual - expected).abs() < 1e-6,
                "{rgb:?}: {actual} != {expected}"
            );
        }
    }

    #[test]
    fn contrast_ratios_match_reference_values() {
        // WCAG relative luminance and (lighter + 0.05) / (darker + 0.05).
        for (left, right, expected) in [
            (WHITE, BLACK, 21.0),
            (BLACK, BLACK, 1.0),
            (
                WHITE,
                AbsoluteColor::from_rgba8([89, 89, 89, 255]),
                7.004729,
            ),
            (
                WHITE,
                AbsoluteColor::from_rgba8([64, 64, 64, 255]),
                10.368378,
            ),
            (
                BLACK,
                AbsoluteColor::from_srgb(0.5, 0.5, 0.5, 1.0),
                5.280823,
            ),
            (
                WHITE,
                AbsoluteColor::new(ColorSpace::Oklch, 0.0, 0.0, 0.0, 1.0),
                21.0,
            ),
        ] {
            let actual = contrast_ratio(&left, &right);
            assert!(
                (actual - expected).abs() < 1e-5,
                "{left:?}, {right:?}: {actual}"
            );
            assert_eq!(actual, contrast_ratio(&right, &left));
        }
    }

    #[test]
    fn black_or_white_selection_matches_background() {
        for (rgb, expected) in [
            ([0.1; 3], WHITE),
            ([0.9; 3], BLACK),
            ([0.5; 3], BLACK),
            ([0.0, 0.0, 1.0], WHITE),
            ([1.0, 1.0, 0.0], BLACK),
        ] {
            let background = AbsoluteColor::from_srgb(rgb[0], rgb[1], rgb[2], 1.0);
            assert_eq!(choose_contrast_color(&background), expected, "{rgb:?}");
        }
    }

    #[test]
    fn candidates_use_highest_ratio_inclusive_threshold_and_first_tie() {
        let navy = AbsoluteColor::from_srgb(0.0, 0.0, 0.5, 0.25);
        let teal = AbsoluteColor::from_srgb(0.0, 0.5, 0.5, 1.0);
        let purple = AbsoluteColor::from_srgb(0.5, 0.0, 0.5, 1.0);
        for candidates in [
            [teal, navy, purple],
            [navy, purple, teal],
            [purple, teal, navy],
        ] {
            assert_eq!(choose_best_contrast(&WHITE, &candidates, 3.0), navy);
        }
        // Use the computed ratio so float rounding cannot move this boundary.
        let threshold = contrast_ratio(&WHITE, &navy);
        assert_eq!(choose_best_contrast(&WHITE, &[navy], threshold), navy);
        assert_eq!(
            choose_best_contrast(&WHITE, &[navy], threshold + 0.01),
            BLACK
        );

        // Alpha does not affect ranking, but the original candidate is returned.
        let opaque_navy = AbsoluteColor { alpha: 1.0, ..navy };
        for candidates in [[navy, opaque_navy], [opaque_navy, navy]] {
            assert_eq!(
                choose_best_contrast(&WHITE, &candidates, 3.0),
                candidates[0]
            );
        }
    }

    #[test]
    fn fallback_returns_the_better_black_or_white() {
        for (gray, expected) in [(0.1, WHITE), (0.5, BLACK)] {
            let background = AbsoluteColor::from_srgb(gray, gray, gray, 1.0);
            for candidates in [&[][..], &[background][..]] {
                assert_eq!(choose_best_contrast(&background, candidates, 4.5), expected);
            }
        }
    }

    #[test]
    fn contrast_uses_clipped_srgb_for_ratios_and_selection() {
        let black = AbsoluteColor::from_srgb(0.0, 0.0, 0.0, 1.0);
        let white = AbsoluteColor::from_srgb(1.0, 1.0, 1.0, 1.0);
        let background = AbsoluteColor::new(ColorSpace::Oklch, 0.5, 0.4, 0.0, 1.0);
        let displayed = AbsoluteColor::from_rgba(background.to_rgba());
        for (text, expected) in [(black, 4.84695), (white, 4.33262)] {
            let ratio = contrast_ratio(&background, &text);
            assert!((ratio - expected).abs() < 1e-4, "{ratio}");
            assert_eq!(ratio, contrast_ratio(&displayed, &text));
            assert_eq!(ratio, contrast_ratio(&text, &background));
        }
        assert_eq!(choose_contrast_color(&background), black);
        assert_eq!(
            choose_best_contrast(&background, &[white, black], 4.5),
            black
        );

        // Direct sRGB inputs use the same clipping and ignore alpha.
        let overbright = AbsoluteColor::from_srgb(2.0, 2.0, 2.0, 0.25);
        let negative = AbsoluteColor::from_srgb(-1.0, -1.0, -1.0, 0.75);
        assert_eq!(
            contrast_ratio(&overbright, &negative),
            contrast_ratio(&white, &black)
        );
    }
}
