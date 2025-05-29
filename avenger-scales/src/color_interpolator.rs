use crate::{
    error::AvengerScaleError,
    scales::{ScaleConfig, ScaleImpl},
};
use arrow::{
    array::{ArrayRef, AsArray, Float32Array, ListArray},
    buffer::OffsetBuffer,
    datatypes::{DataType, Field, Float32Type},
};
use palette::{Hsla, IntoColor, Laba, Mix, Srgba};
use std::{fmt::Debug, sync::Arc};

pub struct ColorInterpolatorConfig {
    pub colors: Vec<[f32; 4]>,
}

pub trait ColorInterpolator: Debug + Send + Sync + 'static {
    /// Interpolate over evenly spaced colors based on normalized values
    fn interpolate(
        &self,
        config: &ColorInterpolatorConfig,
        values: &[f32],
    ) -> Result<ArrayRef, AvengerScaleError>;
}

#[derive(Clone, Debug)]
pub struct SrgbaColorInterpolator;

impl ColorInterpolator for SrgbaColorInterpolator {
    fn interpolate(
        &self,
        config: &ColorInterpolatorConfig,
        values: &[f32],
    ) -> Result<ArrayRef, AvengerScaleError> {
        // Interpolate in Srgba space
        let srgba_colors: Vec<Srgba> = config
            .colors
            .iter()
            .map(|c| Srgba::from_components((c[0], c[1], c[2], c[3])))
            .collect();
        interpolate_color(&srgba_colors, values)
    }
}

#[derive(Clone, Debug)]
pub struct HslaColorInterpolator;

impl ColorInterpolator for HslaColorInterpolator {
    fn interpolate(
        &self,
        config: &ColorInterpolatorConfig,
        values: &[f32],
    ) -> Result<ArrayRef, AvengerScaleError> {
        let hsla_colors: Vec<Hsla> = config
            .colors
            .iter()
            .map(|c| Srgba::from_components((c[0], c[1], c[2], c[3])).into_color())
            .collect();
        interpolate_color(&hsla_colors, values)
    }
}

#[derive(Clone, Debug)]
pub struct LabaColorInterpolator;

impl ColorInterpolator for LabaColorInterpolator {
    fn interpolate(
        &self,
        config: &ColorInterpolatorConfig,
        values: &[f32],
    ) -> Result<ArrayRef, AvengerScaleError> {
        let laba_colors: Vec<Laba> = config
            .colors
            .iter()
            .map(|c| Srgba::from_components((c[0], c[1], c[2], c[3])).into_color())
            .collect();
        interpolate_color(&laba_colors, values)
    }
}

/// A trait for color spaces that can be used with the `NumericColorScale`
pub trait ColorSpace:
    Mix<Scalar = f32> + Copy + IntoColor<Srgba> + Debug + Send + Sync + 'static
{
}

impl<T: Mix<Scalar = f32> + Copy + IntoColor<Srgba> + Debug + Send + Sync + 'static> ColorSpace
    for T
{
}

/// Generic helper function to interpolate colors using palette's `Mix` trait
fn interpolate_color<C: ColorSpace>(
    colors: &[C],
    values: &[f32],
) -> Result<ArrayRef, AvengerScaleError> {
    let scale_factor = (colors.len() - 1) as f32;
    let mut flat_values = Vec::with_capacity(values.len() * 4);
    values.iter().for_each(|v| {
        let continuous_index = (v * scale_factor).clamp(0.0, scale_factor);
        let lower_index = continuous_index.floor() as usize;
        let upper_index = continuous_index.ceil() as usize;

        if lower_index == upper_index {
            let srgba_color: Srgba = colors[lower_index].into_color();
            let (r, g, b, a) = srgba_color.into_components();
            flat_values.extend_from_slice(&[r, g, b, a]);
        } else {
            let lower_color = colors[lower_index];
            let upper_color = colors[upper_index];
            let t = continuous_index - lower_index as f32;
            let srgba_color = lower_color.mix(upper_color, t).into_color();
            let (r, g, b, a) = srgba_color.into_components();
            flat_values.extend_from_slice(&[r, g, b, a]);
        }
    });

    Ok(Arc::new(ListArray::new(
        Arc::new(Field::new_list_field(DataType::Float32, true)),
        OffsetBuffer::from_lengths(vec![4; values.len()]),
        Arc::new(Float32Array::from(flat_values)),
        None,
    )))
}

/// Generic helper function to scale numeric values to color values for continuous numeric scales
pub(crate) fn scale_numeric_to_color(
    scale: &impl ScaleImpl,
    config: &ScaleConfig,
    values: &ArrayRef,
) -> Result<ArrayRef, AvengerScaleError> {
    // Create a new config with a range of [0.0, 1.0] and clamp enabled
    let mut numeric_options = config.options.clone();
    numeric_options.insert("clamp".to_string(), true.into());

    // Scale the values to interval [0.0, 1.0]
    let numeric_config = ScaleConfig {
        range: Arc::new(Float32Array::from(vec![0.0, 1.0])),
        domain: config.domain.clone(),
        options: numeric_options,
        context: config.context.clone(),
    };
    let numeric_values = scale.scale(&numeric_config, values)?;
    let numeric_values = numeric_values.as_primitive::<Float32Type>();

    let color_config = ColorInterpolatorConfig {
        colors: config.color_range()?,
    };
    config
        .context
        .color_interpolator
        .interpolate(&color_config, numeric_values.values())
}

#[allow(dead_code)]
struct MakeSureItsObjectSafe {
    interpolator: Box<dyn ColorInterpolator>,
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

    fn extract_colors_from_result(result: ArrayRef) -> Vec<[f32; 4]> {
        let list_array = result.as_list::<i32>();
        list_array
            .iter()
            .map(|color_opt| {
                let color = color_opt.expect("Color should not be null");
                let values = color.as_primitive::<Float32Type>();
                [
                    values.value(0),
                    values.value(1),
                    values.value(2),
                    values.value(3),
                ]
            })
            .collect()
    }

    #[test]
    fn test_srgba_interpolation_basic() {
        let interpolator = SrgbaColorInterpolator;
        let config = ColorInterpolatorConfig {
            colors: vec![
                [1.0, 0.0, 0.0, 1.0], // Red
                [0.0, 0.0, 1.0, 1.0], // Blue
            ],
        };
        let values = [0.0, 0.5, 1.0];

        let result = interpolator.interpolate(&config, &values).unwrap();
        let colors = extract_colors_from_result(result);

        // Check endpoints
        assert_color_approx_eq(colors[0], [1.0, 0.0, 0.0, 1.0], 0.001); // Red
        assert_color_approx_eq(colors[2], [0.0, 0.0, 1.0, 1.0], 0.001); // Blue

        // Check midpoint (should be purple-ish in SRGBA space)
        assert_color_approx_eq(colors[1], [0.5, 0.0, 0.5, 1.0], 0.001);
    }

    #[test]
    fn test_hsla_interpolation_basic() {
        let interpolator = HslaColorInterpolator;
        let config = ColorInterpolatorConfig {
            colors: vec![
                [1.0, 0.0, 0.0, 1.0], // Red
                [0.0, 1.0, 0.0, 1.0], // Green
            ],
        };
        let values = [0.0, 0.5, 1.0];

        let result = interpolator.interpolate(&config, &values).unwrap();
        let colors = extract_colors_from_result(result);

        assert_eq!(colors.len(), 3);
        // Endpoints should be preserved
        assert_color_approx_eq(colors[0], [1.0, 0.0, 0.0, 1.0], 0.001);
        assert_color_approx_eq(colors[2], [0.0, 1.0, 0.0, 1.0], 0.001);
        
        // Midpoint in HSL should be different from SRGBA interpolation
        // (should go through yellow in hue space)
        let midpoint = colors[1];
        assert!(midpoint[0] > 0.0 && midpoint[1] > 0.0); // Should have both red and green
    }

    #[test]
    fn test_laba_interpolation_basic() {
        let interpolator = LabaColorInterpolator;
        let config = ColorInterpolatorConfig {
            colors: vec![
                [0.0, 0.0, 0.0, 1.0], // Black
                [1.0, 1.0, 1.0, 1.0], // White
            ],
        };
        let values = [0.0, 0.5, 1.0];

        let result = interpolator.interpolate(&config, &values).unwrap();
        let colors = extract_colors_from_result(result);

        assert_eq!(colors.len(), 3);
        // Endpoints should be preserved
        assert_color_approx_eq(colors[0], [0.0, 0.0, 0.0, 1.0], 0.001);
        assert_color_approx_eq(colors[2], [1.0, 1.0, 1.0, 1.0], 0.001);
        
        // LAB interpolation should give perceptually uniform gray
        let midpoint = colors[1];
        assert!(midpoint[0] > 0.4 && midpoint[0] < 0.6); // Should be grayish
        assert_color_approx_eq(midpoint, [midpoint[0], midpoint[0], midpoint[0], 1.0], 0.1);
    }

    #[test]
    fn test_interpolation_edge_cases() {
        let interpolator = SrgbaColorInterpolator;
        
        // Single color
        let config = ColorInterpolatorConfig {
            colors: vec![[0.5, 0.5, 0.5, 1.0]],
        };
        let values = [0.0, 0.5, 1.0];
        let result = interpolator.interpolate(&config, &values).unwrap();
        let colors = extract_colors_from_result(result);
        
        // All values should return the same color
        for color in colors {
            assert_color_approx_eq(color, [0.5, 0.5, 0.5, 1.0], 0.001);
        }
    }

    #[test]
    fn test_interpolation_clamping() {
        let interpolator = SrgbaColorInterpolator;
        let config = ColorInterpolatorConfig {
            colors: vec![
                [1.0, 0.0, 0.0, 1.0], // Red
                [0.0, 0.0, 1.0, 1.0], // Blue
            ],
        };
        
        // Test values outside [0, 1] range
        let values = [-0.5, 0.0, 0.5, 1.0, 1.5];
        let result = interpolator.interpolate(&config, &values).unwrap();
        let colors = extract_colors_from_result(result);

        // Values < 0 should clamp to first color
        assert_color_approx_eq(colors[0], [1.0, 0.0, 0.0, 1.0], 0.001);
        
        // Values > 1 should clamp to last color
        assert_color_approx_eq(colors[4], [0.0, 0.0, 1.0, 1.0], 0.001);
        
        // Normal values should interpolate correctly
        assert_color_approx_eq(colors[1], [1.0, 0.0, 0.0, 1.0], 0.001); // 0.0 -> red
        assert_color_approx_eq(colors[3], [0.0, 0.0, 1.0, 1.0], 0.001); // 1.0 -> blue
    }

    #[test]
    fn test_interpolation_multiple_colors() {
        let interpolator = SrgbaColorInterpolator;
        let config = ColorInterpolatorConfig {
            colors: vec![
                [1.0, 0.0, 0.0, 1.0], // Red
                [0.0, 1.0, 0.0, 1.0], // Green
                [0.0, 0.0, 1.0, 1.0], // Blue
            ],
        };
        
        let values = [0.0, 0.25, 0.5, 0.75, 1.0];
        let result = interpolator.interpolate(&config, &values).unwrap();
        let colors = extract_colors_from_result(result);

        // Check endpoints
        assert_color_approx_eq(colors[0], [1.0, 0.0, 0.0, 1.0], 0.001); // Red
        assert_color_approx_eq(colors[2], [0.0, 1.0, 0.0, 1.0], 0.001); // Green  
        assert_color_approx_eq(colors[4], [0.0, 0.0, 1.0, 1.0], 0.001); // Blue

        // Check intermediate points
        assert!(colors[1][0] > 0.0 && colors[1][1] > 0.0); // Between red and green
        assert!(colors[3][1] > 0.0 && colors[3][2] > 0.0); // Between green and blue
    }

    #[test]
    fn test_interpolation_with_alpha() {
        let interpolator = SrgbaColorInterpolator;
        let config = ColorInterpolatorConfig {
            colors: vec![
                [1.0, 0.0, 0.0, 0.0], // Transparent red
                [1.0, 0.0, 0.0, 1.0], // Opaque red
            ],
        };
        
        let values = [0.0, 0.5, 1.0];
        let result = interpolator.interpolate(&config, &values).unwrap();
        let colors = extract_colors_from_result(result);

        // Color should stay red, alpha should interpolate
        assert_color_approx_eq(colors[0], [1.0, 0.0, 0.0, 0.0], 0.001);
        assert_color_approx_eq(colors[1], [1.0, 0.0, 0.0, 0.5], 0.001);
        assert_color_approx_eq(colors[2], [1.0, 0.0, 0.0, 1.0], 0.001);
    }

    #[test]
    fn test_interpolation_nan_values() {
        let interpolator = SrgbaColorInterpolator;
        let config = ColorInterpolatorConfig {
            colors: vec![
                [1.0, 0.0, 0.0, 1.0], // Red
                [0.0, 0.0, 1.0, 1.0], // Blue
            ],
        };
        
        let values = [0.0, f32::NAN, 1.0];
        let result = interpolator.interpolate(&config, &values).unwrap();
        let colors = extract_colors_from_result(result);

        // NaN should clamp to 0 (first color)
        assert_color_approx_eq(colors[0], [1.0, 0.0, 0.0, 1.0], 0.001);
        assert_color_approx_eq(colors[1], [1.0, 0.0, 0.0, 1.0], 0.001); // NaN -> first color
        assert_color_approx_eq(colors[2], [0.0, 0.0, 1.0, 1.0], 0.001);
    }

    #[test]
    fn test_interpolation_infinity_values() {
        let interpolator = SrgbaColorInterpolator;
        let config = ColorInterpolatorConfig {
            colors: vec![
                [1.0, 0.0, 0.0, 1.0], // Red
                [0.0, 0.0, 1.0, 1.0], // Blue
            ],
        };
        
        let values = [f32::NEG_INFINITY, 0.5, f32::INFINITY];
        let result = interpolator.interpolate(&config, &values).unwrap();
        let colors = extract_colors_from_result(result);

        // -Infinity should clamp to first color, +Infinity to last color
        assert_color_approx_eq(colors[0], [1.0, 0.0, 0.0, 1.0], 0.001);
        assert_color_approx_eq(colors[1], [0.5, 0.0, 0.5, 1.0], 0.001); // Normal interpolation
        assert_color_approx_eq(colors[2], [0.0, 0.0, 1.0, 1.0], 0.001);
    }

    #[test]
    fn test_empty_colors_array() {
        let interpolator = SrgbaColorInterpolator;
        let config = ColorInterpolatorConfig { colors: vec![] };
        let values = [0.0, 0.5, 1.0];
        
        // This should panic or return an error - empty colors array is invalid
        let result = std::panic::catch_unwind(|| {
            interpolator.interpolate(&config, &values)
        });
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_values_array() {
        let interpolator = SrgbaColorInterpolator;
        let config = ColorInterpolatorConfig {
            colors: vec![
                [1.0, 0.0, 0.0, 1.0],
                [0.0, 0.0, 1.0, 1.0],
            ],
        };
        let values = [];
        
        let result = interpolator.interpolate(&config, &values).unwrap();
        let colors = extract_colors_from_result(result);
        assert_eq!(colors.len(), 0);
    }

    #[test]
    fn test_color_space_consistency() {
        // Test that all interpolators preserve endpoints exactly
        let colors = vec![
            [1.0, 0.0, 0.0, 1.0], // Red
            [0.0, 1.0, 0.0, 1.0], // Green
            [0.0, 0.0, 1.0, 1.0], // Blue
        ];
        let config = ColorInterpolatorConfig { colors: colors.clone() };
        let endpoint_values = [0.0, 1.0];
        
        let interpolators: Vec<Box<dyn ColorInterpolator>> = vec![
            Box::new(SrgbaColorInterpolator),
            Box::new(HslaColorInterpolator),
            Box::new(LabaColorInterpolator),
        ];
        
        for interpolator in interpolators {
            let result = interpolator.interpolate(&config, &endpoint_values).unwrap();
            let result_colors = extract_colors_from_result(result);
            
            // All interpolators should preserve endpoints exactly
            assert_color_approx_eq(result_colors[0], colors[0], 0.001);
            assert_color_approx_eq(result_colors[1], colors[2], 0.001);
        }
    }

    #[test]
    fn test_large_number_of_values() {
        let interpolator = SrgbaColorInterpolator;
        let config = ColorInterpolatorConfig {
            colors: vec![
                [0.0, 0.0, 0.0, 1.0], // Black
                [1.0, 1.0, 1.0, 1.0], // White
            ],
        };
        
        // Test with 1000 values
        let values: Vec<f32> = (0..1000).map(|i| i as f32 / 999.0).collect();
        let result = interpolator.interpolate(&config, &values).unwrap();
        let colors = extract_colors_from_result(result);
        
        assert_eq!(colors.len(), 1000);
        
        // Check that interpolation is monotonic for grayscale
        for i in 1..colors.len() {
            assert!(colors[i][0] >= colors[i-1][0], "Red component should be monotonic");
            assert!(colors[i][1] >= colors[i-1][1], "Green component should be monotonic");
            assert!(colors[i][2] >= colors[i-1][2], "Blue component should be monotonic");
        }
    }

    #[test]
    fn test_interpolation_precision() {
        let interpolator = SrgbaColorInterpolator;
        let config = ColorInterpolatorConfig {
            colors: vec![
                [0.0, 0.0, 0.0, 1.0], // Black
                [1.0, 1.0, 1.0, 1.0], // White
            ],
        };
        
        // Test interpolation at exact midpoint
        let result = interpolator.interpolate(&config, &[0.5]).unwrap();
        let colors = extract_colors_from_result(result);
        
        // Midpoint should be exactly gray
        assert_color_approx_eq(colors[0], [0.5, 0.5, 0.5, 1.0], 0.0001);
    }

    #[test]
    fn test_color_components_out_of_range() {
        let interpolator = SrgbaColorInterpolator;
        
        // Test with color components outside [0,1] range
        let config = ColorInterpolatorConfig {
            colors: vec![
                [-0.5, 0.0, 0.0, 1.0], // Invalid red component
                [1.5, 1.0, 1.0, 1.0],  // Invalid red component
            ],
        };
        
        let values = [0.0, 0.5, 1.0];
        let result = interpolator.interpolate(&config, &values);
        
        // Should handle gracefully (either clamp or interpolate as-is)
        assert!(result.is_ok(), "Should handle out-of-range color components gracefully");
        
        let colors = extract_colors_from_result(result.unwrap());
        assert_eq!(colors.len(), 3);
    }
}
