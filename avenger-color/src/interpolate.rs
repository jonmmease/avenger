use std::fmt;

use palette::{Hsla, IntoColor, Laba, Mix, Srgba};

/// Space used to mix colors whose inputs and outputs are sRGB RGBA.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColorInterpolationSpace {
    Srgba,
    Hsla,
    Laba,
}

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
/// Input and output colors are normalized sRGB RGBA with straight alpha.
/// `space` selects the intermediate color space. Alpha is interpolated
/// independently, without premultiplying the color components.
///
/// Positions outside `[0, 1]`, including infinities, select the nearest endpoint.
/// NaN selects the first color. A single stop produces a constant color, and
/// empty positions produce an empty result. An empty color range returns an error.
pub fn interpolate_colors(
    space: ColorInterpolationSpace,
    colors: &[[f32; 4]],
    values: &[f32],
) -> Result<Vec<[f32; 4]>, ColorInterpolationError> {
    if colors.is_empty() {
        return Err(ColorInterpolationError::EmptyColorRange);
    }

    Ok(match space {
        ColorInterpolationSpace::Srgba => {
            let colors: Vec<Srgba> = colors
                .iter()
                .map(|c| Srgba::from_components((c[0], c[1], c[2], c[3])))
                .collect();
            interpolate_color(&colors, values)
        }
        ColorInterpolationSpace::Hsla => {
            let colors: Vec<Hsla> = colors
                .iter()
                .map(|c| Srgba::from_components((c[0], c[1], c[2], c[3])).into_color())
                .collect();
            interpolate_color(&colors, values)
        }
        ColorInterpolationSpace::Laba => {
            let colors: Vec<Laba> = colors
                .iter()
                .map(|c| Srgba::from_components((c[0], c[1], c[2], c[3])).into_color())
                .collect();
            interpolate_color(&colors, values)
        }
    })
}

/// A trait for color spaces that can be interpolated with palette's `Mix`.
trait InterpolationColorSpace:
    Mix<Scalar = f32> + Copy + IntoColor<Srgba> + std::fmt::Debug + Send + Sync + 'static
{
}

impl<T: Mix<Scalar = f32> + Copy + IntoColor<Srgba> + std::fmt::Debug + Send + Sync + 'static>
    InterpolationColorSpace for T
{
}

fn interpolate_color<C: InterpolationColorSpace>(colors: &[C], values: &[f32]) -> Vec<[f32; 4]> {
    let scale_factor = (colors.len() - 1) as f32;
    let mut result = Vec::with_capacity(values.len());
    values.iter().for_each(|v| {
        let continuous_index = if v.is_nan() {
            0.0
        } else {
            (v * scale_factor).clamp(0.0, scale_factor)
        };
        let lower_index = continuous_index.floor() as usize;
        let upper_index = continuous_index.ceil() as usize;

        let srgba_color: Srgba = if lower_index == upper_index {
            colors[lower_index].into_color()
        } else {
            let lower_color = colors[lower_index];
            let upper_color = colors[upper_index];
            let t = continuous_index - lower_index as f32;
            lower_color.mix(upper_color, t).into_color()
        };

        let (r, g, b, a) = srgba_color.into_components();
        result.push([r, g, b, a]);
    });

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPACES: [ColorInterpolationSpace; 3] = [
        ColorInterpolationSpace::Srgba,
        ColorInterpolationSpace::Hsla,
        ColorInterpolationSpace::Laba,
    ];

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
            (
                ColorInterpolationSpace::Srgba,
                red_green,
                [0.5, 0.5, 0.0, 0.5],
            ),
            (
                ColorInterpolationSpace::Hsla,
                red_green,
                [1.0, 1.0, 0.0, 0.5],
            ),
            // CIELAB L*=50 converts to approximately 0.4663266 in sRGB.
            (
                ColorInterpolationSpace::Laba,
                black_white,
                [0.4663266, 0.4663266, 0.4663266, 0.5],
            ),
            // Hues at 350 and 10 degrees interpolate through red.
            (
                ColorInterpolationSpace::Hsla,
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
    fn multiple_stops_use_local_segment_positions() {
        let colors = [
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
        ];
        let result = interpolate_colors(
            ColorInterpolationSpace::Srgba,
            &colors,
            &[0.125, 0.5, 0.875],
        )
        .unwrap();
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
