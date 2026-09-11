use std::fmt;

use palette::{Hsla, IntoColor, Laba, Mix, Srgba};

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

/// Interpolate over evenly spaced colors based on normalized values.
pub fn interpolate_colors(
    space: ColorInterpolationSpace,
    colors: &[[f32; 4]],
    values: &[f32],
) -> Result<Vec<[f32; 4]>, ColorInterpolationError> {
    if colors.is_empty() {
        return Err(ColorInterpolationError::EmptyColorRange);
    }

    match space {
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
    }
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

fn interpolate_color<C: InterpolationColorSpace>(
    colors: &[C],
    values: &[f32],
) -> Result<Vec<[f32; 4]>, ColorInterpolationError> {
    if colors.is_empty() {
        return Err(ColorInterpolationError::EmptyColorRange);
    }

    let scale_factor = (colors.len() - 1) as f32;
    let mut result = Vec::with_capacity(values.len());
    values.iter().for_each(|v| {
        let continuous_index = (v * scale_factor).clamp(0.0, scale_factor);
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

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_color_approx_eq(actual: [f32; 4], expected: [f32; 4], tolerance: f32) {
        for i in 0..4 {
            assert!(
                (actual[i] - expected[i]).abs() < tolerance,
                "Color component {} differs: actual={}, expected={}, tolerance={}",
                i,
                actual[i],
                expected[i],
                tolerance
            );
        }
    }

    #[test]
    fn test_srgba_interpolation_basic() {
        let colors = vec![[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]];
        let values = [0.0, 0.5, 1.0];

        let colors = interpolate_colors(ColorInterpolationSpace::Srgba, &colors, &values).unwrap();

        assert_color_approx_eq(colors[0], [1.0, 0.0, 0.0, 1.0], 0.001);
        assert_color_approx_eq(colors[2], [0.0, 0.0, 1.0, 1.0], 0.001);
        assert_color_approx_eq(colors[1], [0.5, 0.0, 0.5, 1.0], 0.001);
    }

    #[test]
    fn test_hsla_interpolation_basic() {
        let colors = vec![[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]];
        let values = [0.0, 0.5, 1.0];

        let colors = interpolate_colors(ColorInterpolationSpace::Hsla, &colors, &values).unwrap();

        assert_eq!(colors.len(), 3);
        assert_color_approx_eq(colors[0], [1.0, 0.0, 0.0, 1.0], 0.001);
        assert_color_approx_eq(colors[2], [0.0, 1.0, 0.0, 1.0], 0.001);

        let midpoint = colors[1];
        assert!(midpoint[0] > 0.0 && midpoint[1] > 0.0);
    }

    #[test]
    fn test_laba_interpolation_basic() {
        let colors = vec![[0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]];
        let values = [0.0, 0.5, 1.0];

        let colors = interpolate_colors(ColorInterpolationSpace::Laba, &colors, &values).unwrap();

        assert_eq!(colors.len(), 3);
        assert_color_approx_eq(colors[0], [0.0, 0.0, 0.0, 1.0], 0.001);
        assert_color_approx_eq(colors[2], [1.0, 1.0, 1.0, 1.0], 0.001);

        let midpoint = colors[1];
        assert!(midpoint[0] > 0.4 && midpoint[0] < 0.6);
        assert_color_approx_eq(midpoint, [midpoint[0], midpoint[0], midpoint[0], 1.0], 0.1);
    }

    #[test]
    fn test_interpolation_edge_cases() {
        let colors = vec![[0.5, 0.5, 0.5, 1.0]];
        let values = [0.0, 0.5, 1.0];
        let colors = interpolate_colors(ColorInterpolationSpace::Srgba, &colors, &values).unwrap();

        for color in colors {
            assert_color_approx_eq(color, [0.5, 0.5, 0.5, 1.0], 0.001);
        }
    }

    #[test]
    fn test_interpolation_clamping() {
        let colors = vec![[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]];
        let values = [-0.5, 0.0, 0.5, 1.0, 1.5];
        let colors = interpolate_colors(ColorInterpolationSpace::Srgba, &colors, &values).unwrap();

        assert_color_approx_eq(colors[0], [1.0, 0.0, 0.0, 1.0], 0.001);
        assert_color_approx_eq(colors[4], [0.0, 0.0, 1.0, 1.0], 0.001);
        assert_color_approx_eq(colors[1], [1.0, 0.0, 0.0, 1.0], 0.001);
        assert_color_approx_eq(colors[3], [0.0, 0.0, 1.0, 1.0], 0.001);
    }

    #[test]
    fn test_interpolation_multiple_colors() {
        let colors = vec![
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
        ];

        let values = [0.0, 0.25, 0.5, 0.75, 1.0];
        let colors = interpolate_colors(ColorInterpolationSpace::Srgba, &colors, &values).unwrap();

        assert_color_approx_eq(colors[0], [1.0, 0.0, 0.0, 1.0], 0.001);
        assert_color_approx_eq(colors[2], [0.0, 1.0, 0.0, 1.0], 0.001);
        assert_color_approx_eq(colors[4], [0.0, 0.0, 1.0, 1.0], 0.001);
        assert!(colors[1][0] > 0.0 && colors[1][1] > 0.0);
        assert!(colors[3][1] > 0.0 && colors[3][2] > 0.0);
    }

    #[test]
    fn test_interpolation_with_alpha() {
        let colors = vec![[1.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 1.0]];

        let values = [0.0, 0.5, 1.0];
        let colors = interpolate_colors(ColorInterpolationSpace::Srgba, &colors, &values).unwrap();

        assert_color_approx_eq(colors[0], [1.0, 0.0, 0.0, 0.0], 0.001);
        assert_color_approx_eq(colors[1], [1.0, 0.0, 0.0, 0.5], 0.001);
        assert_color_approx_eq(colors[2], [1.0, 0.0, 0.0, 1.0], 0.001);
    }

    #[test]
    fn test_interpolation_nan_values() {
        let colors = vec![[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]];

        let values = [0.0, f32::NAN, 1.0];
        let colors = interpolate_colors(ColorInterpolationSpace::Srgba, &colors, &values).unwrap();

        assert_color_approx_eq(colors[0], [1.0, 0.0, 0.0, 1.0], 0.001);
        assert_color_approx_eq(colors[1], [1.0, 0.0, 0.0, 1.0], 0.001);
        assert_color_approx_eq(colors[2], [0.0, 0.0, 1.0, 1.0], 0.001);
    }

    #[test]
    fn test_interpolation_infinity_values() {
        let colors = vec![[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]];

        let values = [f32::NEG_INFINITY, 0.5, f32::INFINITY];
        let colors = interpolate_colors(ColorInterpolationSpace::Srgba, &colors, &values).unwrap();

        assert_color_approx_eq(colors[0], [1.0, 0.0, 0.0, 1.0], 0.001);
        assert_color_approx_eq(colors[1], [0.5, 0.0, 0.5, 1.0], 0.001);
        assert_color_approx_eq(colors[2], [0.0, 0.0, 1.0, 1.0], 0.001);
    }

    #[test]
    fn test_empty_colors_array() {
        let values = [0.0, 0.5, 1.0];
        let result = interpolate_colors(ColorInterpolationSpace::Srgba, &[], &values);
        assert_eq!(result, Err(ColorInterpolationError::EmptyColorRange));
    }

    #[test]
    fn test_empty_values_array() {
        let colors = vec![[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]];
        let values = [];

        let colors = interpolate_colors(ColorInterpolationSpace::Srgba, &colors, &values).unwrap();
        assert_eq!(colors.len(), 0);
    }

    #[test]
    fn test_color_space_consistency() {
        let colors = vec![
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
        ];
        let endpoint_values = [0.0, 1.0];

        for space in [
            ColorInterpolationSpace::Srgba,
            ColorInterpolationSpace::Hsla,
            ColorInterpolationSpace::Laba,
        ] {
            let result_colors = interpolate_colors(space, &colors, &endpoint_values).unwrap();
            assert_color_approx_eq(result_colors[0], colors[0], 0.001);
            assert_color_approx_eq(result_colors[1], colors[2], 0.001);
        }
    }

    #[test]
    fn test_large_number_of_values() {
        let colors = vec![[0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]];

        let values: Vec<f32> = (0..1000).map(|i| i as f32 / 999.0).collect();
        let colors = interpolate_colors(ColorInterpolationSpace::Srgba, &colors, &values).unwrap();

        assert_eq!(colors.len(), 1000);
        for i in 1..colors.len() {
            assert!(
                colors[i][0] >= colors[i - 1][0],
                "Red component should be monotonic"
            );
            assert!(
                colors[i][1] >= colors[i - 1][1],
                "Green component should be monotonic"
            );
            assert!(
                colors[i][2] >= colors[i - 1][2],
                "Blue component should be monotonic"
            );
        }
    }

    #[test]
    fn test_interpolation_precision() {
        let colors = vec![[0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]];

        let colors = interpolate_colors(ColorInterpolationSpace::Srgba, &colors, &[0.5]).unwrap();
        assert_color_approx_eq(colors[0], [0.5, 0.5, 0.5, 1.0], 0.0001);
    }

    #[test]
    fn test_color_components_out_of_range() {
        let colors = vec![[-0.5, 0.0, 0.0, 1.0], [1.5, 1.0, 1.0, 1.0]];

        let values = [0.0, 0.5, 1.0];
        let colors = interpolate_colors(ColorInterpolationSpace::Srgba, &colors, &values).unwrap();
        assert_eq!(colors.len(), 3);
    }
}
