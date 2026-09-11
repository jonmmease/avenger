//! Color mixing and interpolation
//!
//! Ported from Mozilla Stylo's style/color/mix.rs
//! https://searchfox.org/mozilla-central/source/servo/components/style/color/mix.rs

use super::{
    convert::{convert_color_space, normalize_hue},
    types::{AbsoluteColor, ColorSpace},
};

/// Convex K-way mixer over a fixed set of key colors in Oklab.
///
/// Built for categorical raster shading: the key colors (one per category)
/// are converted to Oklab once at construction, then each pixel's weighted
/// mean costs three multiply-adds per category plus a single Oklab → sRGB
/// conversion. The mean is convex, so results stay inside the convex hull
/// of the key colors in Oklab; the final per-channel clamp handles the rare
/// sub-percent gamut excursion after conversion.
///
/// Key-color alpha is ignored — for density-shaded rasters the output alpha
/// is supplied separately by the caller (an opacity scale over the total).
/// Non-finite or negative weights count as zero.
#[derive(Clone, Debug)]
pub struct OklabMixer {
    labs: Vec<[f32; 3]>,
}

impl OklabMixer {
    pub fn new(key_colors: &[[f32; 4]]) -> Self {
        let labs = key_colors
            .iter()
            .map(|color| {
                convert_color_space(
                    &[color[0], color[1], color[2]],
                    ColorSpace::Srgb,
                    ColorSpace::Oklab,
                )
            })
            .collect();
        Self { labs }
    }

    pub fn len(&self) -> usize {
        self.labs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.labs.is_empty()
    }

    /// Weighted convex mix of the key colors; returns sRGB components in
    /// [0, 1]. `None` when the usable weights sum to zero (rasters render
    /// such pixels fully transparent).
    pub fn mix(&self, weights: &[f32]) -> Option<[f32; 3]> {
        debug_assert_eq!(weights.len(), self.labs.len());
        let mut acc = [0.0_f32; 3];
        let mut total = 0.0_f32;
        for (lab, &weight) in self.labs.iter().zip(weights) {
            if !weight.is_finite() || weight <= 0.0 {
                continue;
            }
            total += weight;
            acc[0] += weight * lab[0];
            acc[1] += weight * lab[1];
            acc[2] += weight * lab[2];
        }
        if total <= 0.0 {
            return None;
        }
        let lab = [acc[0] / total, acc[1] / total, acc[2] / total];
        let srgb = convert_color_space(&lab, ColorSpace::Oklab, ColorSpace::Srgb);
        Some([
            srgb[0].clamp(0.0, 1.0),
            srgb[1].clamp(0.0, 1.0),
            srgb[2].clamp(0.0, 1.0),
        ])
    }
}

/// Hue interpolation method for polar color spaces
///
/// See: https://drafts.csswg.org/css-color-4/#typedef-hue-interpolation-method
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum HueInterpolationMethod {
    /// Angles are adjusted so the hue difference is <= 180deg (default)
    Shorter = 0,
    /// Angles are adjusted so the hue difference is >= 180deg
    Longer,
    /// Hue values increase from left to right
    Increasing,
    /// Hue values decrease from left to right
    Decreasing,
    /// Angles are not adjusted (interpolate as-is)
    Specified,
}

/// Mix two colors in a specified color space
///
/// # Arguments
///
/// * `interpolation_space` - The color space to perform mixing in
/// * `left_color` - The first color to mix
/// * `left_weight` - Weight for the first color (0.0-1.0)
/// * `right_color` - The second color to mix
/// * `right_weight` - Weight for the second color (0.0-1.0)
/// * `hue_method` - How to interpolate hue in polar color spaces
///
/// # Returns
///
/// A new color representing the mix of the two input colors
pub fn mix_colors(
    interpolation_space: ColorSpace,
    left_color: &AbsoluteColor,
    left_weight: f32,
    right_color: &AbsoluteColor,
    right_weight: f32,
    hue_method: HueInterpolationMethod,
) -> AbsoluteColor {
    // Normalize weights to sum to 1.0
    let total = left_weight + right_weight;
    let lw = if total != 0.0 {
        left_weight / total
    } else {
        0.5
    };
    let rw = if total != 0.0 {
        right_weight / total
    } else {
        0.5
    };

    // Convert both colors to the interpolation space
    let left = left_color.to_color_space(interpolation_space);
    let right = right_color.to_color_space(interpolation_space);

    // Prepare components with alpha for premultiplied interpolation
    let left_components = [
        left.components[0],
        left.components[1],
        left.components[2],
        left.alpha,
    ];
    let right_components = [
        right.components[0],
        right.components[1],
        right.components[2],
        right.alpha,
    ];

    // Interpolate with premultiplied alpha
    let result = interpolate_premultiplied(
        &left_components,
        lw,
        &right_components,
        rw,
        interpolation_space.hue_index(),
        hue_method,
    );

    // Round alpha to avoid precision issues (0.999995 -> 1.0)
    let alpha = (result[3] * 1000.0).round() / 1000.0;

    AbsoluteColor::new(interpolation_space, result[0], result[1], result[2], alpha)
}

/// Adjust hue angles for interpolation according to the specified method
///
/// See: https://drafts.csswg.org/css-color-4/#hue-interpolation
fn adjust_hue(left: &mut f32, right: &mut f32, method: HueInterpolationMethod) {
    // If both hues are NaN, set to 0
    if left.is_nan() {
        if right.is_nan() {
            *left = 0.0;
            *right = 0.0;
        } else {
            *left = *right;
        }
    } else if right.is_nan() {
        *right = *left;
    }

    // For "specified" method, no adjustment needed
    if method == HueInterpolationMethod::Specified {
        return;
    }

    // Normalize to [0, 360)
    *left = normalize_hue(*left);
    *right = normalize_hue(*right);

    match method {
        // https://drafts.csswg.org/css-color-4/#shorter
        HueInterpolationMethod::Shorter => {
            let delta = *right - *left;
            if delta > 180.0 {
                *left += 360.0;
            } else if delta < -180.0 {
                *right += 360.0;
            }
        }
        // https://drafts.csswg.org/css-color-4/#longer
        HueInterpolationMethod::Longer => {
            let delta = *right - *left;
            if 0.0 < delta && delta < 180.0 {
                *left += 360.0;
            } else if -180.0 < delta && delta <= 0.0 {
                *right += 360.0;
            }
        }
        // https://drafts.csswg.org/css-color-4/#increasing
        HueInterpolationMethod::Increasing => {
            if *right < *left {
                *right += 360.0;
            }
        }
        // https://drafts.csswg.org/css-color-4/#decreasing
        HueInterpolationMethod::Decreasing => {
            if *left < *right {
                *left += 360.0;
            }
        }
        HueInterpolationMethod::Specified => unreachable!("Handled above"),
    }
}

/// Interpolate hue component with the specified method
fn interpolate_hue(
    mut left: f32,
    left_weight: f32,
    mut right: f32,
    right_weight: f32,
    method: HueInterpolationMethod,
) -> f32 {
    adjust_hue(&mut left, &mut right, method);
    left * left_weight + right * right_weight
}

/// Interpolate a component using premultiplied alpha
///
/// See: https://drafts.csswg.org/css-color-4/#interpolation-alpha
fn interpolate_premultiplied_component(
    left: f32,
    left_weight: f32,
    left_alpha: f32,
    right: f32,
    right_weight: f32,
    right_alpha: f32,
) -> f32 {
    left * left_weight * left_alpha + right * right_weight * right_alpha
}

/// Interpolate all components with premultiplied alpha
///
/// This implements the color mixing algorithm from CSS Color 4:
/// https://drafts.csswg.org/css-color-4/#interpolation
fn interpolate_premultiplied(
    left: &[f32; 4], // [c0, c1, c2, alpha]
    left_weight: f32,
    right: &[f32; 4], // [c0, c1, c2, alpha]
    right_weight: f32,
    hue_index: Option<usize>,
    hue_method: HueInterpolationMethod,
) -> [f32; 4] {
    // Interpolate alpha first
    let left_alpha = left[3].clamp(0.0, 1.0);
    let right_alpha = right[3].clamp(0.0, 1.0);
    let alpha = (left_alpha * left_weight + right_alpha * right_weight).clamp(0.0, 1.0);

    let mut result = [0.0; 4];

    // Interpolate each color component
    for i in 0..3 {
        let is_hue = hue_index == Some(i);

        result[i] = if is_hue {
            // Hue components use special interpolation
            normalize_hue(interpolate_hue(
                left[i],
                left_weight,
                right[i],
                right_weight,
                hue_method,
            ))
        } else {
            // Non-hue components use premultiplied alpha
            let interpolated = interpolate_premultiplied_component(
                left[i],
                left_weight,
                left_alpha,
                right[i],
                right_weight,
                right_alpha,
            );

            // Un-premultiply
            if alpha == 0.0 {
                interpolated
            } else {
                interpolated / alpha
            }
        };
    }

    result[3] = alpha;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oklab_mixer_pure_weight_reproduces_key_color() {
        let keys = [
            [0.9, 0.2, 0.1, 1.0_f32],
            [0.1, 0.5, 0.8, 1.0],
            [0.2, 0.7, 0.3, 1.0],
        ];
        let mixer = OklabMixer::new(&keys);
        for (index, key) in keys.iter().enumerate() {
            let mut weights = [0.0_f32; 3];
            weights[index] = 7.5; // un-normalized on purpose
            let mixed = mixer.mix(&weights).expect("nonzero weight");
            for channel in 0..3 {
                assert!(
                    (mixed[channel] - key[channel]).abs() <= 1.5 / 255.0,
                    "key {index} channel {channel}: {} vs {}",
                    mixed[channel],
                    key[channel]
                );
            }
        }
    }

    #[test]
    fn test_oklab_mixer_matches_independent_oklab_average() {
        let black = [0.0_f32, 0.0, 0.0, 1.0];
        let white = [1.0_f32, 1.0, 1.0, 1.0];
        let mixer = OklabMixer::new(&[black, white]);
        let mixed = mixer.mix(&[1.0, 1.0]).expect("nonzero weights");

        // Independent computation through the public conversion API.
        let lab_black = convert_color_space(&[0.0, 0.0, 0.0], ColorSpace::Srgb, ColorSpace::Oklab);
        let lab_white = convert_color_space(&[1.0, 1.0, 1.0], ColorSpace::Srgb, ColorSpace::Oklab);
        let lab_mid = [
            (lab_black[0] + lab_white[0]) / 2.0,
            (lab_black[1] + lab_white[1]) / 2.0,
            (lab_black[2] + lab_white[2]) / 2.0,
        ];
        let expected = convert_color_space(&lab_mid, ColorSpace::Oklab, ColorSpace::Srgb);
        for channel in 0..3 {
            assert!((mixed[channel] - expected[channel]).abs() < 1e-4);
        }
        // And it is measurably NOT the naive gamma-sRGB average (0.5): the
        // Oklab half-lightness gray encodes near 0.389 in sRGB.
        assert!(
            (mixed[0] - 0.5).abs() > 0.05,
            "Oklab mid-gray should differ from the naive sRGB average, got {}",
            mixed[0]
        );
    }

    #[test]
    fn test_oklab_mixer_blue_yellow_not_naive_gray() {
        let blue = [0.0_f32, 0.0, 1.0, 1.0];
        let yellow = [1.0_f32, 1.0, 0.0, 1.0];
        let mixer = OklabMixer::new(&[blue, yellow]);
        let mixed = mixer.mix(&[0.5, 0.5]).expect("nonzero weights");
        // Naive gamma-sRGB averaging collapses to exactly (0.5, 0.5, 0.5).
        let max_dev_from_gray = mixed
            .iter()
            .map(|c| (c - 0.5_f32).abs())
            .fold(0.0_f32, f32::max);
        assert!(
            max_dev_from_gray > 0.05,
            "Oklab blue/yellow mix should not collapse to the muddy sRGB gray, got {mixed:?}"
        );
        // Convexity + clamp: all channels in range.
        assert!(mixed.iter().all(|c| (0.0..=1.0).contains(c)));
    }

    #[test]
    fn test_oklab_mixer_degenerate_weights() {
        let mixer = OklabMixer::new(&[[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]]);
        assert!(mixer.mix(&[0.0, 0.0]).is_none());
        assert!(mixer.mix(&[f32::NAN, 0.0]).is_none());
        // Negative and NaN weights are ignored, not propagated.
        let mixed = mixer.mix(&[-3.0, 2.0]).expect("one usable weight");
        assert!((mixed[1] - 1.0).abs() <= 1.5 / 255.0);
        assert!(mixed[0].abs() <= 1.5 / 255.0);
    }

    #[test]
    fn test_adjust_hue_shorter() {
        let mut left = 10.0;
        let mut right = 350.0;
        adjust_hue(&mut left, &mut right, HueInterpolationMethod::Shorter);
        // Should adjust right to -10 (350 -> 350, but effective as -10 for shorter path)
        // Actually normalizes first, so right stays 350, left becomes 370
        assert_eq!(left, 370.0);
        assert_eq!(right, 350.0);
    }

    #[test]
    fn test_adjust_hue_longer() {
        let mut left = 10.0;
        let mut right = 20.0;
        adjust_hue(&mut left, &mut right, HueInterpolationMethod::Longer);
        // Delta is 10 (< 180), so left should be increased by 360
        assert_eq!(left, 370.0);
        assert_eq!(right, 20.0);
    }

    #[test]
    fn test_mix_srgb() {
        let red = AbsoluteColor::from_srgb(1.0, 0.0, 0.0, 1.0);
        let blue = AbsoluteColor::from_srgb(0.0, 0.0, 1.0, 1.0);

        let purple = mix_colors(
            ColorSpace::Srgb,
            &red,
            0.5,
            &blue,
            0.5,
            HueInterpolationMethod::Shorter,
        );

        // Should be roughly purple (0.5, 0, 0.5)
        assert!((purple.components[0] - 0.5).abs() < 0.01);
        assert!((purple.components[1] - 0.0).abs() < 0.01);
        assert!((purple.components[2] - 0.5).abs() < 0.01);
        assert_eq!(purple.alpha, 1.0);
    }

    #[test]
    fn test_mix_with_alpha() {
        let red_half = AbsoluteColor::from_srgb(1.0, 0.0, 0.0, 0.5);
        let blue_full = AbsoluteColor::from_srgb(0.0, 0.0, 1.0, 1.0);

        let mix = mix_colors(
            ColorSpace::Srgb,
            &red_half,
            0.5,
            &blue_full,
            0.5,
            HueInterpolationMethod::Shorter,
        );

        // Alpha should be (0.5 * 0.5 + 1.0 * 0.5) = 0.75
        assert!((mix.alpha - 0.75).abs() < 0.01);
    }

    #[test]
    fn test_mix_in_oklab() {
        let red = AbsoluteColor::from_srgb(1.0, 0.0, 0.0, 1.0);
        let blue = AbsoluteColor::from_srgb(0.0, 0.0, 1.0, 1.0);

        // Mix in Oklab space (perceptually uniform)
        let mix = mix_colors(
            ColorSpace::Oklab,
            &red,
            0.5,
            &blue,
            0.5,
            HueInterpolationMethod::Shorter,
        );

        // Result should be in Oklab space
        assert_eq!(mix.color_space, ColorSpace::Oklab);

        // Convert back to sRGB to verify it's different from sRGB mixing
        let srgb = mix.to_color_space(ColorSpace::Srgb);

        // In Oklab, the mix should be different from simple RGB average
        // This is a perceptual color space, so the result will differ
        assert!(srgb.components[0] > 0.0 && srgb.components[0] < 1.0);
        assert!(srgb.components[2] > 0.0 && srgb.components[2] < 1.0);
    }
}
