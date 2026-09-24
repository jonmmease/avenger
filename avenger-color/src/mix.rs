//! Weighted color mixing with premultiplied alpha and polar hue interpolation.

// Ported from Mozilla Stylo's style/color/mix.rs
// https://searchfox.org/mozilla-central/source/servo/components/style/color/mix.rs

use super::{
    convert::{convert_color_space, normalize_hue},
    types::{AbsoluteColor, ColorSpace},
};

/// Weighted Oklab mixing over a fixed palette of sRGB colors.
///
/// Palette colors are converted to Oklab once at construction and reused by each
/// call to [`Self::mix`]. Alpha is ignored. Mixing returns RGB only.
#[derive(Clone, Debug)]
pub struct OklabMixer {
    labs: Vec<[f32; 3]>,
}

impl OklabMixer {
    /// Cache Oklab components for sRGB `[r, g, b, a]` palette entries.
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

    /// Number of palette entries, and the required number of weights.
    pub fn len(&self) -> usize {
        self.labs.len()
    }

    /// Whether the palette contains no colors.
    pub fn is_empty(&self) -> bool {
        self.labs.is_empty()
    }

    /// Return the weighted Oklab mean as sRGB components clipped to [0, 1].
    ///
    /// Weights correspond to palette entries in order and are normalized by their
    /// sum. Non-finite and nonpositive weights are ignored. Returns `None` when
    /// there are no positive finite weights, including for an empty palette.
    ///
    /// # Panics
    ///
    /// Panics unless there is exactly one weight per palette entry.
    pub fn mix(&self, weights: &[f32]) -> Option<[f32; 3]> {
        assert_eq!(weights.len(), self.labs.len(), "one weight per key color");
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

/// Hue interpolation for polar color spaces in [`mix_colors`].
///
/// `Shorter`, `Longer`, `Increasing`, and `Decreasing` follow
/// [CSS hue interpolation](https://drafts.csswg.org/css-color-4/#hue-interpolation).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum HueInterpolationMethod {
    /// Follow the shorter arc, with an angular difference of at most 180 degrees.
    Shorter = 0,
    /// Follow the longer arc, with an angular difference of at least 180 degrees.
    Longer,
    /// Interpolate toward increasing hue angles.
    Increasing,
    /// Interpolate toward decreasing hue angles.
    Decreasing,
    /// Interpolate the supplied angles directly, without choosing an arc.
    Specified,
}

/// Convert two colors to `interpolation_space` and mix them with premultiplied alpha.
///
/// Weights must be finite and nonnegative, with a finite sum. They are normalized
/// by their sum. A zero sum uses equal weights. Input alpha is clipped to [0, 1].
///
/// Hue uses `hue_method` without alpha premultiplication and is normalized to
/// [0, 360) degrees. An undefined hue (`NaN`) takes the other color's hue. Two
/// undefined hues are treated as zero before applying the interpolation method.
/// The result retains `interpolation_space`.
pub fn mix_colors(
    interpolation_space: ColorSpace,
    left_color: &AbsoluteColor,
    left_weight: f32,
    right_color: &AbsoluteColor,
    right_weight: f32,
    hue_method: HueInterpolationMethod,
) -> AbsoluteColor {
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

    let left = left_color.to_color_space(interpolation_space);
    let right = right_color.to_color_space(interpolation_space);

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

    let result = interpolate_premultiplied(
        &left_components,
        lw,
        &right_components,
        rw,
        interpolation_space.hue_index(),
        hue_method,
    );

    AbsoluteColor::new(
        interpolation_space,
        result[0],
        result[1],
        result[2],
        result[3],
    )
}

/// Resolve undefined hues and select the interpolation arc.
///
/// <https://drafts.csswg.org/css-color-4/#hue-interpolation>
fn adjust_hue(left: &mut f32, right: &mut f32, method: HueInterpolationMethod) {
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

    if method == HueInterpolationMethod::Specified {
        return;
    }

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
/// <https://drafts.csswg.org/css-color-4/#interpolation-alpha>
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

/// Interpolate components with premultiplied alpha, except for hue.
///
/// <https://drafts.csswg.org/css-color-4/#interpolation-alpha>
fn interpolate_premultiplied(
    left: &[f32; 4], // [c0, c1, c2, alpha]
    left_weight: f32,
    right: &[f32; 4], // [c0, c1, c2, alpha]
    right_weight: f32,
    hue_index: Option<usize>,
    hue_method: HueInterpolationMethod,
) -> [f32; 4] {
    let left_alpha = left[3].clamp(0.0, 1.0);
    let right_alpha = right[3].clamp(0.0, 1.0);
    let alpha = (left_alpha * left_weight + right_alpha * right_weight).clamp(0.0, 1.0);

    let mut result = [0.0; 4];

    for i in 0..3 {
        let is_hue = hue_index == Some(i);

        result[i] = if is_hue {
            normalize_hue(interpolate_hue(
                left[i],
                left_weight,
                right[i],
                right_weight,
                hue_method,
            ))
        } else {
            let interpolated = interpolate_premultiplied_component(
                left[i],
                left_weight,
                left_alpha,
                right[i],
                right_weight,
                right_alpha,
            );

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
    fn lch_mixing_with_neutral_colors_preserves_the_chromatic_hue() {
        let red = AbsoluteColor::from_srgb(1.0, 0.0, 0.0, 1.0);
        // CSS D50 LCH hue of sRGB red.
        let expected_hue = 40.85767;
        for gray in [0.0, 0.5, 1.0] {
            let neutral = AbsoluteColor::from_srgb(gray, gray, gray, 1.0);
            assert!(neutral.to_color_space(ColorSpace::Lch).components[2].is_nan());
            for (left, right) in [(neutral, red), (red, neutral)] {
                let mixed = mix_colors(
                    ColorSpace::Lch,
                    &left,
                    0.5,
                    &right,
                    0.5,
                    HueInterpolationMethod::Shorter,
                );
                assert!(
                    (mixed.components[2] - expected_hue).abs() < 1e-3,
                    "{mixed:?}"
                );
            }
        }
    }

    #[test]
    fn mixing_identical_colors_preserves_alpha() {
        for alpha in [0.0004, 0.123456, 128.0 / 255.0, 0.999995] {
            let color = AbsoluteColor::from_srgb(1.0, 0.0, 0.0, alpha);
            let mixed = mix_colors(
                ColorSpace::Srgb,
                &color,
                0.5,
                &color,
                0.5,
                HueInterpolationMethod::Shorter,
            );
            assert_eq!(mixed.alpha, alpha);
        }
    }

    #[test]
    fn mixing_normalizes_weights_and_premultiplies_alpha() {
        for (left_alpha, right_alpha, weights, expected) in [
            (1.0, 1.0, [1.0, 3.0], [0.25, 0.0, 0.75, 1.0]),
            (1.0, 1.0, [0.0, 0.0], [0.5, 0.0, 0.5, 1.0]),
            (0.5, 1.0, [1.0, 1.0], [1.0 / 3.0, 0.0, 2.0 / 3.0, 0.75]),
            (0.0, 1.0, [1.0, 1.0], [0.0, 0.0, 1.0, 0.5]),
            (1.0, 0.0, [1.0, 1.0], [1.0, 0.0, 0.0, 0.5]),
            (0.0, 0.0, [1.0, 1.0], [0.0, 0.0, 0.0, 0.0]),
        ] {
            let left = AbsoluteColor::from_srgb(1.0, 0.0, 0.0, left_alpha);
            let right = AbsoluteColor::from_srgb(0.0, 0.0, 1.0, right_alpha);
            let actual = mix_colors(
                ColorSpace::Srgb,
                &left,
                weights[0],
                &right,
                weights[1],
                HueInterpolationMethod::Shorter,
            );
            assert_eq!(actual, AbsoluteColor::from_rgba(expected));
        }
    }

    #[test]
    fn public_mixing_obeys_hue_methods_and_missing_hues() {
        use HueInterpolationMethod::*;
        for (space, index, components) in [
            (ColorSpace::Hsl, 0, [0.0, 80.0, 50.0]),
            (ColorSpace::Hwb, 0, [0.0, 10.0, 20.0]),
            (ColorSpace::Lch, 2, [50.0, 30.0, 0.0]),
            (ColorSpace::Oklch, 2, [0.5, 0.1, 0.0]),
        ] {
            for (method, left_hue, right_hue, expected) in [
                (Shorter, 10.0, 350.0, 0.0),
                (Shorter, 350.0, 10.0, 0.0),
                (Longer, 10.0, 20.0, 195.0),
                (Longer, 20.0, 10.0, 195.0),
                (Increasing, 350.0, 10.0, 0.0),
                (Increasing, 10.0, 350.0, 180.0),
                (Decreasing, 10.0, 350.0, 0.0),
                (Decreasing, 350.0, 10.0, 180.0),
                (Specified, 10.0, 350.0, 180.0),
                (Specified, -90.0, 450.0, 180.0),
                (Shorter, -90.0, 450.0, 180.0),
                (Shorter, f32::NAN, 40.0, 40.0),
                (Shorter, 40.0, f32::NAN, 40.0),
                (Shorter, f32::NAN, f32::NAN, 0.0),
            ] {
                let mut left = AbsoluteColor {
                    components,
                    alpha: 0.25,
                    color_space: space,
                };
                let mut right = AbsoluteColor {
                    alpha: 0.75,
                    ..left
                };
                left.components[index] = left_hue;
                right.components[index] = right_hue;
                let actual = mix_colors(space, &left, 1.0, &right, 1.0, method);
                let mut expected_color = AbsoluteColor {
                    components,
                    alpha: 0.5,
                    color_space: space,
                };
                expected_color.components[index] = expected;
                assert_eq!(
                    actual, expected_color,
                    "{space:?} {method:?}: {left_hue} -> {right_hue}"
                );
            }
        }
    }

    #[test]
    fn mixing_converts_inputs_to_the_requested_space() {
        let red = AbsoluteColor::from_srgb(1.0, 0.0, 0.0, 1.0);
        let blue = AbsoluteColor::from_srgb(0.0, 0.0, 1.0, 1.0);
        let actual = mix_colors(
            ColorSpace::Oklab,
            &red,
            1.0,
            &blue,
            1.0,
            HueInterpolationMethod::Shorter,
        );
        // CSS Color 4 sample conversions, averaged in Oklab outside this crate.
        assert_eq!(actual.color_space, ColorSpace::Oklab);
        assert_eq!(actual.alpha, 1.0);
        for (actual, expected) in
            actual
                .components
                .into_iter()
                .zip([0.5399845, 0.09620305, -0.09284094])
        {
            assert!((actual - expected).abs() < 1e-6);
        }
    }

    #[test]
    fn oklab_mixer_matches_reference_weighted_means() {
        // Expected sRGB values from CSS Color 4's f64 conversion sample code:
        // https://www.w3.org/TR/css-color-4/#color-conversion-code
        // Key alpha is deliberately varied: OklabMixer ignores it.
        let keys = [
            [0.9, 0.2, 0.1, 0.0],
            [0.1, 0.5, 0.8, 0.5],
            [0.2, 0.7, 0.3, 1.0],
        ];
        for (palette, weights, expected) in [
            (
                &keys[..],
                &[1.0, 2.0, 4.0][..],
                [0.3791198, 0.6063569, 0.4623209],
            ),
            (&keys[..], &[1.0, 0.0, 0.0][..], [0.9, 0.2, 0.1]),
            (&keys[..], &[0.0, 1.0, 0.0][..], [0.1, 0.5, 0.8]),
            (&keys[..], &[0.0, 0.0, 1.0][..], [0.2, 0.7, 0.3]),
            (
                &[[0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]][..],
                &[1.0, 1.0][..],
                [0.3885729; 3],
            ),
            (
                &[[0.0, 0.0, 1.0, 1.0], [1.0, 1.0, 0.0, 1.0]][..],
                &[1.0, 1.0][..],
                [0.4225514, 0.6723662, 0.7805431],
            ),
        ] {
            let mixer = OklabMixer::new(palette);
            assert_eq!(mixer.len(), palette.len());
            assert!(!mixer.is_empty());
            for scale in [1.0, 7.5] {
                let scaled: Vec<_> = weights.iter().map(|w| w * scale).collect();
                let actual = mixer.mix(&scaled).unwrap();
                for (actual, expected) in actual.into_iter().zip(expected) {
                    assert!(
                        (actual - expected).abs() < 1e-5,
                        "{weights:?}: {actual} != {expected}"
                    );
                }
            }
        }
    }

    #[test]
    fn oklab_mixer_ignores_unusable_weights_and_handles_empty_palettes() {
        let empty = OklabMixer::new(&[]);
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);
        assert_eq!(empty.mix(&[]), None);
        let mixer = OklabMixer::new(&[[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]]);
        for invalid in [0.0, -3.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(mixer.mix(&[invalid, 0.0]), None);
            let actual = mixer.mix(&[invalid, 2.0]).unwrap();
            for (actual, expected) in actual.into_iter().zip([0.0, 1.0, 0.0]) {
                assert!((actual - expected).abs() < 1e-5, "{invalid}: {actual}");
            }
        }
    }

    #[test]
    #[should_panic(expected = "one weight per key color")]
    fn oklab_mixer_rejects_mismatched_weights() {
        let mixer = OklabMixer::new(&[[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]]);
        mixer.mix(&[1.0]);
    }
}
