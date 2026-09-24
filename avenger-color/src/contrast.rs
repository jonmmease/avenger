//! WCAG 2.1 relative luminance, contrast ratios, and color selection.
//!
//! Ratios and selection use clipped sRGB components and ignore alpha. Composite
//! translucent colors against their background before comparing them.
//!
//! See the WCAG definitions of [contrast ratio](https://www.w3.org/TR/WCAG21/#dfn-contrast-ratio)
//! and [relative luminance](https://www.w3.org/TR/WCAG21/#dfn-relative-luminance).
//!
//! # Examples
//!
//! ```rust
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

/// Decode an sRGB component in [0, 1] to linear light.
#[inline]
fn srgb_to_linear(component: f32) -> f32 {
    if component <= 0.04045 {
        component / 12.92
    } else {
        ((component + 0.055) / 1.055).powf(2.4)
    }
}

/// Calculate WCAG 2.1 relative luminance from sRGB components in [0, 1].
///
/// Returns 0 for black and 1 for white. Inputs are not clipped.
///
/// # Examples
///
/// ```rust
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

/// Calculate the WCAG 2.1 contrast ratio between two colors.
///
/// Uses the clipped sRGB channels returned by [`AbsoluteColor::to_rgba`]. Alpha is
/// ignored. Composite translucent colors against their background first.
///
/// The ratio is `(lighter + 0.05) / (darker + 0.05)`, where the two values are
/// relative luminances. Results range from 1 for equal luminance to 21 for black
/// and white. Swapping the colors does not change the result.
///
/// # Examples
///
/// ```rust
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

    let lighter = l1.max(l2);
    let darker = l1.min(l2);

    (lighter + 0.05) / (darker + 0.05)
}

/// Choose opaque sRGB black or white for the higher [`contrast_ratio`].
///
/// Equal ratios select white. Alpha is ignored. Composite translucent colors
/// against their background first.
///
/// # Examples
///
/// ```rust
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

    let contrast_with_black = contrast_ratio(base_color, &BLACK);
    let contrast_with_white = contrast_ratio(base_color, &WHITE);

    if contrast_with_white >= contrast_with_black {
        WHITE
    } else {
        BLACK
    }
}

/// Select the candidate with the highest [`contrast_ratio`] at or above `min_ratio`.
///
/// Ties retain the first candidate, including its original color space and alpha.
/// An empty list or no qualifying candidate uses [`choose_contrast_color`]. That
/// fallback may have a ratio below `min_ratio`.
///
/// Alpha is ignored when comparing colors. Composite translucent colors against
/// their background first.
///
/// # Examples
///
/// ```rust
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

    for candidate in candidates {
        let ratio = contrast_ratio(base_color, candidate);
        if ratio >= min_ratio && ratio > best_ratio {
            best_ratio = ratio;
            best_candidate = Some(*candidate);
        }
    }

    if let Some(candidate) = best_candidate {
        return candidate;
    }

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
