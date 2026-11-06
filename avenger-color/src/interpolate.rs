use std::fmt;

use crate::{mix_colors, AbsoluteColor, ColorSpace, HueInterpolationMethod};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorInterpolationError {
    EmptyColorRange,
}

impl fmt::Display for ColorInterpolationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ColorInterpolationError::EmptyColorRange => {
                write!(f, "color interpolation requires at least one color")
            }
        }
    }
}

impl std::error::Error for ColorInterpolationError {}

/// Sample evenly spaced color stops at positions in `[0, 1]`.
///
/// Input and output colors are normalized sRGB RGBA with straight alpha. `space` and `hue` select
/// the color space and the hue interpolation of [`mix_colors`], which mixes the opaque colors of
/// adjacent stops. Alpha is interpolated independently, without premultiplying the color
/// components.
///
/// Positions outside `[0, 1]`, including infinities, select the nearest endpoint.
/// NaN selects the first color. A single stop produces a constant color, and
/// empty positions produce an empty result. An empty color range returns an error.
pub fn interpolate_colors(
    space: ColorSpace,
    hue: HueInterpolationMethod,
    colors: &[[f32; 4]],
    values: &[f32],
) -> Result<Vec<[f32; 4]>, ColorInterpolationError> {
    if colors.is_empty() {
        return Err(ColorInterpolationError::EmptyColorRange);
    }
    // The opaque stops convert to the interpolation space once, so mixing them converts nothing.
    let opaque: Vec<AbsoluteColor> = (colors.iter())
        .map(|[r, g, b, _]| AbsoluteColor::from_srgb(*r, *g, *b, 1.0).to_color_space(space))
        .collect();
    let scale_factor = (colors.len() - 1) as f32;
    Ok(values
        .iter()
        .map(|value| {
            let continuous_index = if value.is_nan() {
                0.0
            } else {
                (value * scale_factor).clamp(0.0, scale_factor)
            };
            let lower = continuous_index.floor() as usize;
            let upper = continuous_index.ceil() as usize;
            if lower == upper {
                return colors[lower];
            }
            let t = continuous_index - lower as f32;
            // Opaque colors without a hue mix as their components do, without premultiplying.
            let mixed = match space.is_polar() {
                true => mix_colors(space, &opaque[lower], 1.0 - t, &opaque[upper], t, hue),
                false => {
                    let [a, b] = [opaque[lower].components, opaque[upper].components];
                    let mix = |index: usize| a[index] + (b[index] - a[index]) * t;
                    AbsoluteColor::new(space, mix(0), mix(1), mix(2), 1.0)
                }
            };
            let [r, g, b, _] = mixed.to_rgba();
            let alpha = colors[lower][3] + (colors[upper][3] - colors[lower][3]) * t;
            [r, g, b, alpha]
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPACES: [ColorSpace; 8] = [
        ColorSpace::Srgb,
        ColorSpace::SrgbLinear,
        ColorSpace::Hsl,
        ColorSpace::Hwb,
        ColorSpace::Lab,
        ColorSpace::Lch,
        ColorSpace::Oklab,
        ColorSpace::Oklch,
    ];

    fn interpolate_colors(
        space: ColorSpace,
        colors: &[[f32; 4]],
        values: &[f32],
    ) -> Result<Vec<[f32; 4]>, ColorInterpolationError> {
        super::interpolate_colors(space, HueInterpolationMethod::Shorter, colors, values)
    }

    fn assert_color_close(actual: [f32; 4], expected: [f32; 4]) {
        for (actual, expected) in actual.into_iter().zip(expected) {
            assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
        }
    }

    #[test]
    fn spaces_preserve_endpoints_and_mix_straight_alpha() {
        let red_green = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 1.0]];
        let black_white = [[0.0, 0.0, 0.0, 0.0], [1.0, 1.0, 1.0, 1.0]];
        for (space, colors, midpoint) in [
            (ColorSpace::Srgb, red_green, [0.5, 0.5, 0.0, 0.5]),
            (ColorSpace::Hsl, red_green, [1.0, 1.0, 0.0, 0.5]),
            // CIELAB L*=50 converts to approximately 0.4663266 in sRGB.
            (
                ColorSpace::Lab,
                black_white,
                [0.4663266, 0.4663266, 0.4663266, 0.5],
            ),
            // Hues at 350 and 10 degrees interpolate through red.
            (
                ColorSpace::Hsl,
                [[1.0, 0.0, 1.0 / 6.0, 1.0], [1.0, 1.0 / 6.0, 0.0, 1.0]],
                [1.0, 0.0, 0.0, 1.0],
            ),
        ] {
            let result = interpolate_colors(space, &colors, &[0.0, 0.5, 1.0]).unwrap();
            assert_eq!(result.len(), 3);
            assert_color_close(result[0], colors[0]);
            assert_color_close(result[1], midpoint);
            assert_color_close(result[2], colors[1]);
        }
    }

    #[test]
    fn every_space_matches_mix_colors_of_its_stops() {
        let colors = [
            [0.1, 0.2, 0.8, 1.0],
            [0.9, 0.9, 0.2, 0.5],
            [0.8, 0.1, 0.1, 1.0],
        ];
        let values = [0.0, 0.1, 0.3, 0.5, 0.75, 1.0];
        for space in SPACES {
            let result = interpolate_colors(space, &colors, &values).unwrap();
            for (value, actual) in values.iter().zip(result) {
                let position = value * 2.0;
                let lower = (position.floor() as usize).min(1);
                let t = position - lower as f32;
                let opaque = |[r, g, b, _]: [f32; 4]| AbsoluteColor::from_srgb(r, g, b, 1.0);
                let [r, g, b, _] = mix_colors(
                    space,
                    &opaque(colors[lower]),
                    1.0 - t,
                    &opaque(colors[lower + 1]),
                    t,
                    HueInterpolationMethod::Shorter,
                )
                .to_rgba();
                let alpha = colors[lower][3] + (colors[lower + 1][3] - colors[lower][3]) * t;
                assert_color_close(actual, [r, g, b, alpha]);
            }
        }
    }

    #[test]
    fn linear_srgb_mixes_light_linearly() {
        let black_white = [[0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]];
        let result = interpolate_colors(ColorSpace::SrgbLinear, &black_white, &[0.5]).unwrap();
        // Half of the light of white is 0.5 in linear light, which sRGB encodes as about 0.7354.
        assert_color_close(result[0], [0.735_357, 0.735_357, 0.735_357, 1.0]);
        // Linear sRGB converts through the other spaces and back.
        let back = AbsoluteColor::from_srgb(0.2, 0.4, 0.6, 1.0)
            .to_color_space(ColorSpace::SrgbLinear)
            .to_color_space(ColorSpace::Oklab)
            .to_color_space(ColorSpace::SrgbLinear)
            .to_rgba();
        assert_color_close(back, [0.2, 0.4, 0.6, 1.0]);
    }

    #[test]
    fn multiple_stops_use_local_segment_positions() {
        let colors = [
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
        ];
        let result = interpolate_colors(ColorSpace::Srgb, &colors, &[0.125, 0.5, 0.875]).unwrap();
        assert_eq!(
            result,
            vec![[0.75, 0.25, 0.0, 1.0], colors[1], [0.0, 0.25, 0.75, 1.0]]
        );
    }

    #[test]
    fn values_are_clamped_and_nan_selects_first_color() {
        let colors = [[1.0, 0.0, 0.0, 0.25], [0.0, 0.0, 1.0, 0.75]];
        let values = [
            f32::NEG_INFINITY,
            -0.5,
            0.0,
            1.0,
            1.5,
            f32::INFINITY,
            f32::NAN,
        ];
        for space in SPACES {
            let result = interpolate_colors(space, &colors, &values).unwrap();
            assert_eq!(result.len(), values.len());
            for (actual, index) in result.into_iter().zip([0, 0, 0, 1, 1, 1, 0]) {
                assert_color_close(actual, colors[index]);
            }
        }
    }

    #[test]
    fn single_color_is_constant() {
        let color = [0.2, 0.4, 0.6, 0.7];
        let values = [f32::NEG_INFINITY, 0.0, 0.5, 1.0, f32::INFINITY, f32::NAN];
        for space in SPACES {
            let result = interpolate_colors(space, &[color], &values).unwrap();
            assert_eq!(result.len(), values.len());
            for actual in result {
                assert_color_close(actual, color);
            }
        }
    }

    #[test]
    fn empty_color_range_is_rejected() {
        for space in SPACES {
            assert_eq!(
                interpolate_colors(space, &[], &[0.5]),
                Err(ColorInterpolationError::EmptyColorRange)
            );
        }
    }

    #[test]
    fn empty_values_produce_no_colors() {
        for space in SPACES {
            assert!(interpolate_colors(space, &[[0.2, 0.4, 0.6, 1.0]], &[])
                .unwrap()
                .is_empty());
        }
    }
}
