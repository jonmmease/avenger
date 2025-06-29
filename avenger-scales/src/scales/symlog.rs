use std::sync::Arc;

use crate::scalar::Scalar;
use arrow::{
    array::{ArrayRef, AsArray, Float32Array},
    compute::{kernels::cast, unary},
    datatypes::{DataType, Float32Type},
};
use avenger_common::value::ScalarOrArray;

use crate::{array, error::AvengerScaleError};

use super::{
    linear::LinearScale, ConfiguredScale, InferDomainFromDataMethod, ScaleConfig, ScaleContext,
    ScaleImpl,
};

/// Symmetric log scale that provides smooth linear-to-logarithmic transitions for data
/// that includes zero or spans both positive and negative values.
///
/// The symlog transformation is defined as: sign(x) * log(1 + |x|/C) where C is the constant.
/// This provides linear behavior near zero (within [-C, C]) and logarithmic behavior for
/// larger absolute values, making it ideal for data with values near zero or crossing zero.
///
/// # Config Options
///
/// - **constant** (f32, default: 1.0): The linear threshold that controls the transition
///   between linear and logarithmic behavior. Within the range [-constant, constant],
///   the scale behaves approximately linearly. Must be positive.
///
/// - **clamp** (boolean, default: false): When true, values outside the domain are clamped
///   to the domain extent. For scaling, this means values outside domain map to the
///   corresponding range extent. For inversion, values outside range are clamped first.
///
/// - **range_offset** (f32, default: 0.0): An offset applied to the final scaled values.
///   This is added after the transformation. When inverting, this offset is subtracted first.
///
/// - **round** (boolean, default: false): When true, output values from scaling are rounded
///   to the nearest integer. This is useful for pixel-perfect rendering. Does not affect inversion.
///
/// - **nice** (boolean or f32, default: false): When true or a number, extends the domain to
///   nice round values in the transformed space. If true, uses a default count of 10.
///   If a number, uses that as the target tick count for determining nice values.
#[derive(Debug)]
pub struct SymlogScale;

impl SymlogScale {
    pub fn configured(domain: (f32, f32), range: (f32, f32)) -> ConfiguredScale {
        ConfiguredScale {
            scale_impl: Arc::new(Self),
            config: ScaleConfig {
                domain: Arc::new(Float32Array::from(vec![domain.0, domain.1])),
                range: Arc::new(Float32Array::from(vec![range.0, range.1])),
                options: vec![
                    ("constant".to_string(), 1.0.into()),
                    ("clamp".to_string(), false.into()),
                    ("range_offset".to_string(), 0.0.into()),
                    ("round".to_string(), false.into()),
                    ("nice".to_string(), false.into()),
                ]
                .into_iter()
                .collect(),
                context: ScaleContext::default(),
            },
        }
    }

    /// Compute nice domain
    pub fn apply_nice(
        domain: (f32, f32),
        constant: f32,
        count: Option<&Scalar>,
    ) -> Result<(f32, f32), AvengerScaleError> {
        // Create a linear scale to nice the transformed values
        let (domain_start, domain_end) = domain;
        let d0 = symlog_transform(domain_start, constant);
        let d1 = symlog_transform(domain_end, constant);

        let (nice_d0, nice_d1) = LinearScale::apply_nice((d0, d1), count)?;

        // Transform back to original space
        let domain_start = symlog_invert(nice_d0, constant);
        let domain_end = symlog_invert(nice_d1, constant);
        Ok((domain_start, domain_end))
    }
}

impl ScaleImpl for SymlogScale {
    fn scale_type(&self) -> &'static str {
        "symlog"
    }

    fn infer_domain_from_data_method(&self) -> InferDomainFromDataMethod {
        InferDomainFromDataMethod::Interval
    }

    fn scale(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError> {
        // Get options
        let constant = config.option_f32("constant", 1.0);
        let range_offset = config.option_f32("range_offset", 0.0);
        let clamp = config.option_boolean("clamp", false);
        let round = config.option_boolean("round", false);

        let (domain_start, domain_end) = SymlogScale::apply_nice(
            config.numeric_interval_domain()?,
            constant,
            config.options.get("nice"),
        )?;
        let (range_start, range_end) = config.numeric_interval_range()?;

        // Handle degenerate domain case
        if domain_start == domain_end
            || range_start == range_end
            || domain_start.is_nan()
            || domain_end.is_nan()
            || range_start.is_nan()
            || range_end.is_nan()
        {
            return Ok(Arc::new(Float32Array::from(vec![range_start; values.len()])) as ArrayRef);
        }

        // Pre-compute transformed domain endpoints
        let d0 = symlog_transform(domain_start, constant);
        let d1 = symlog_transform(domain_end, constant);

        // Pre-compute scale and offset outside the loop
        let scale = (range_end - range_start) / (d1 - d0);
        let offset = range_start - scale * d0 + range_offset;

        // Pre-compute range bounds outside the loop
        let (range_min, range_max) = if range_start <= range_end {
            (range_start, range_end)
        } else {
            (range_end, range_start)
        };

        // Cast to f32 and downcast to f32 array
        let values = cast(values, &DataType::Float32)?;
        let values = values.as_primitive::<Float32Type>();

        match (clamp, round) {
            (true, true) => {
                // clamp, and round
                Ok(Arc::<Float32Array>::new(unary(values, |v| {
                    if v.is_nan() {
                        return f32::NAN;
                    }
                    if v.is_infinite() {
                        if v.is_sign_positive() {
                            return range_end.round();
                        } else {
                            return range_start.round();
                        }
                    }
                    // Apply symlog transform
                    let sign: f32 = if v < 0.0 { -1.0 } else { 1.0 };
                    let transformed = sign * (1.0 + (v.abs() / constant)).ln();

                    // Apply scale and offset, then clamp
                    (scale * transformed + offset)
                        .clamp(range_min, range_max)
                        .round()
                })))
            }
            (true, false) => {
                // clamp, no round
                Ok(Arc::<Float32Array>::new(unary(values, |v| {
                    if v.is_nan() {
                        return f32::NAN;
                    }
                    if v.is_infinite() {
                        if v.is_sign_positive() {
                            return range_end;
                        } else {
                            return range_start;
                        }
                    }
                    // Apply symlog transform
                    let sign: f32 = if v < 0.0 { -1.0 } else { 1.0 };
                    let transformed = sign * (1.0 + (v.abs() / constant)).ln();

                    // Apply scale and offset, then clamp
                    (scale * transformed + offset).clamp(range_min, range_max)
                })))
            }
            (false, true) => {
                // no clamp, round
                Ok(Arc::<Float32Array>::new(unary(values, |v| {
                    if v.is_nan() {
                        return f32::NAN;
                    }
                    if v.is_infinite() {
                        return if v.is_sign_positive() {
                            range_end.round()
                        } else {
                            range_start.round()
                        };
                    }
                    // Apply symlog transform
                    let sign = if v < 0.0 { -1.0 } else { 1.0 };
                    let transformed = sign * (1.0 + (v.abs() / constant)).ln();

                    // Apply scale and offset
                    (scale * transformed + offset).round()
                })))
            }
            (false, false) => {
                // no clamp, no round
                Ok(Arc::<Float32Array>::new(unary(values, |v| {
                    if v.is_nan() {
                        return f32::NAN;
                    }
                    if v.is_infinite() {
                        return if v.is_sign_positive() {
                            range_end
                        } else {
                            range_start
                        };
                    }
                    // Apply symlog transform
                    let sign = if v < 0.0 { -1.0 } else { 1.0 };
                    let transformed = sign * (1.0 + (v.abs() / constant)).ln();

                    // Apply scale and offset
                    scale * transformed + offset
                })))
            }
        }
    }

    fn invert_from_numeric(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        let (domain_start, domain_end) = SymlogScale::apply_nice(
            config.numeric_interval_domain()?,
            config.option_f32("constant", 1.0),
            config.options.get("nice"),
        )?;

        let (range_start, range_end) = config.numeric_interval_range()?;
        let range_offset = config.option_f32("range_offset", 0.0);
        let clamp = config.option_boolean("clamp", false);
        let _round = config.option_boolean("round", false);
        let constant = config.option_f32("constant", 1.0);

        // Handle degenerate domain case
        if domain_start == domain_end
            || range_start == range_end
            || domain_start.is_nan()
            || domain_end.is_nan()
            || range_start.is_nan()
            || range_end.is_nan()
        {
            return Ok(ScalarOrArray::new_array(vec![domain_start; values.len()]));
        }

        // Cast to f32 and downcast to f32 array
        let values = cast(values, &DataType::Float32)?;
        let values = values.as_primitive::<Float32Type>();

        // Pre-compute transformed domain endpoints
        let d0 = symlog_transform(domain_start, constant);
        let d1 = symlog_transform(domain_end, constant);

        // Pre-compute scale and offset outside the loop
        let scale = (d1 - d0) / (range_end - range_start);
        let offset = d0 - scale * range_start;

        if clamp {
            // Pre-compute range bounds outside the loop
            let (range_min, range_max) = if range_start <= range_end {
                (range_start, range_end)
            } else {
                (range_end, range_start)
            };

            Ok(ScalarOrArray::new_array(
                values
                    .values()
                    .iter()
                    .map(|&v| {
                        if v.is_nan() {
                            return f32::NAN;
                        }
                        if v.is_infinite() {
                            if v.is_sign_positive() {
                                return domain_end;
                            } else {
                                return domain_start;
                            }
                        }

                        // Clamp input to range
                        let v = v.clamp(range_min, range_max);

                        // Transform back to original space
                        let normalized = scale * (v - range_offset) + offset;
                        let sign = if normalized < 0.0 { -1.0 } else { 1.0 };

                        // Apply inverse transform
                        sign * (normalized.abs().exp() - 1.0) * constant
                    })
                    .collect(),
            ))
        } else {
            Ok(ScalarOrArray::new_array(
                values
                    .values()
                    .iter()
                    .map(|&v| {
                        if v.is_nan() {
                            return f32::NAN;
                        }
                        if v.is_infinite() {
                            return if v.is_sign_positive() {
                                domain_end
                            } else {
                                domain_start
                            };
                        }

                        // Transform back to original space
                        let normalized = scale * (v - range_offset) + offset;
                        let sign = if normalized < 0.0 { -1.0 } else { 1.0 };

                        // Apply inverse transform
                        sign * (normalized.abs().exp() - 1.0) * constant
                    })
                    .collect(),
            ))
        }
    }

    fn ticks(
        &self,
        config: &ScaleConfig,
        count: Option<f32>,
    ) -> Result<ArrayRef, AvengerScaleError> {
        let (domain_start, domain_end) = SymlogScale::apply_nice(
            config.numeric_interval_domain()?,
            config.option_f32("constant", 1.0),
            config.options.get("nice"),
        )?;
        let count = count.unwrap_or(10.0);
        let ticks_array = Float32Array::from(array::ticks(domain_start, domain_end, count));
        Ok(Arc::new(ticks_array) as ArrayRef)
    }
}

/// Applies the symlog transform to a single value
fn symlog_transform(x: f32, constant: f32) -> f32 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    sign * (1.0 + (x.abs() / constant)).ln()
}

/// Applies the inverse symlog transform to a single value
fn symlog_invert(x: f32, constant: f32) -> f32 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    sign * ((x.abs()).exp() - 1.0) * constant
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use float_cmp::{assert_approx_eq, F32Margin};

    #[test]
    fn test_basic_scale() {
        let scale = SymlogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![-100.0, 100.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 1.0])),
            options: HashMap::new(),
            context: ScaleContext::default(),
        };

        let values = Arc::new(Float32Array::from(vec![-100.0, 0.0, 100.0])) as ArrayRef;
        let result = scale
            .scale_to_numeric(&config, &values)
            .unwrap()
            .as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 0.0);
        assert_approx_eq!(f32, result[1], 0.5);
        assert_approx_eq!(f32, result[2], 1.0);
    }

    #[test]
    fn test_basic_scale_range_offset() {
        let scale = SymlogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![-100.0, 100.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 1.0])),
            options: vec![("range_offset".to_string(), 0.5.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let values = Arc::new(Float32Array::from(vec![-100.0, 0.0, 100.0])) as ArrayRef;
        let result = scale
            .scale_to_numeric(&config, &values)
            .unwrap()
            .as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 0.5);
        assert_approx_eq!(f32, result[1], 1.0);
        assert_approx_eq!(f32, result[2], 1.5);
    }

    #[test]
    fn test_constant() {
        let scale = SymlogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![-100.0, 100.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 1.0])),
            options: vec![("constant".to_string(), 5.0.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let values = Arc::new(Float32Array::from(vec![-100.0, 0.0, 100.0])) as ArrayRef;
        let result = scale
            .scale_to_numeric(&config, &values)
            .unwrap()
            .as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 0.0);
        assert_approx_eq!(f32, result[1], 0.5);
        assert_approx_eq!(f32, result[2], 1.0);
    }

    #[test]
    fn test_clamp() {
        let scale = SymlogScale;
        let values = Arc::new(Float32Array::from(vec![3.0, -1.0])) as ArrayRef;

        let config_no_clamp = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![0.0, 1.0])),
            range: Arc::new(Float32Array::from(vec![10.0, 20.0])),
            options: vec![("clamp".to_string(), false.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let result = scale
            .scale_to_numeric(&config_no_clamp, &values)
            .unwrap()
            .as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 30.0);
        assert_approx_eq!(f32, result[1], 0.0);

        let config_with_clamp = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![0.0, 1.0])),
            range: Arc::new(Float32Array::from(vec![10.0, 20.0])),
            options: vec![("clamp".to_string(), true.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let result = scale
            .scale_to_numeric(&config_with_clamp, &values)
            .unwrap()
            .as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 20.0);
        assert_approx_eq!(f32, result[1], 10.0);
    }

    #[test]
    fn test_edge_cases() {
        let scale = SymlogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![-100.0, 100.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 1.0])),
            options: HashMap::new(),
            context: ScaleContext::default(),
        };

        // Test NaN
        let values = Arc::new(Float32Array::from(vec![f32::NAN])) as ArrayRef;
        let result = scale
            .scale_to_numeric(&config, &values)
            .unwrap()
            .as_vec(values.len(), None);
        assert!(result[0].is_nan());

        // Test infinity
        let values =
            Arc::new(Float32Array::from(vec![f32::INFINITY, f32::NEG_INFINITY])) as ArrayRef;
        let result = scale
            .scale_to_numeric(&config, &values)
            .unwrap()
            .as_vec(values.len(), None);
        assert!(result[0].is_finite()); // Should be clamped if clamp is true
        assert!(result[1].is_finite()); // Should be clamped if clamp is true
    }

    #[test]
    fn test_invert() {
        let scale = SymlogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![-100.0, 100.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 1.0])),
            options: HashMap::new(),
            context: ScaleContext::default(),
        };

        // Test that invert(scale(x)) ≈ x
        let values = Arc::new(Float32Array::from(vec![
            -100.0, -10.0, -1.0, 0.0, 1.0, 10.0, 100.0,
        ])) as ArrayRef;
        let scaled = scale.scale_to_numeric(&config, &values).unwrap();
        let scaled_array =
            Arc::new(Float32Array::from(scaled.as_vec(values.len(), None))) as ArrayRef;

        let inverted = scale.invert_from_numeric(&config, &scaled_array).unwrap();
        let inverted_array =
            Arc::new(Float32Array::from(inverted.as_vec(values.len(), None))) as ArrayRef;

        let values_f32 = values.as_primitive::<Float32Type>();
        let inverted_f32 = inverted_array.as_primitive::<Float32Type>();
        for i in 0..values.len() {
            assert_approx_eq!(f32, inverted_f32.value(i), values_f32.value(i));
        }
    }

    #[test]
    fn test_invert_range_offset() {
        let scale = SymlogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![-100.0, 100.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 1.0])),
            options: vec![("range_offset".to_string(), 0.5.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        // Test that invert(scale(x)) ≈ x
        let values = Arc::new(Float32Array::from(vec![
            -100.0, -10.0, -1.0, 0.0, 1.0, 10.0, 100.0,
        ])) as ArrayRef;
        let scaled = scale.scale_to_numeric(&config, &values).unwrap();
        let scaled_array =
            Arc::new(Float32Array::from(scaled.as_vec(values.len(), None))) as ArrayRef;
        let inverted = scale.invert_from_numeric(&config, &scaled_array).unwrap();
        let inverted_array =
            Arc::new(Float32Array::from(inverted.as_vec(values.len(), None))) as ArrayRef;

        let values_f32 = values.as_primitive::<Float32Type>();
        let inverted_f32 = inverted_array.as_primitive::<Float32Type>();

        for i in 0..values.len() {
            assert_approx_eq!(
                f32,
                inverted_f32.value(i),
                values_f32.value(i),
                F32Margin {
                    epsilon: 0.0001,
                    ..Default::default()
                }
            );
        }
    }

    #[test]
    fn test_invert_clamped() {
        let scale = SymlogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![-100.0, 100.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 1.0])),
            options: vec![("clamp".to_string(), true.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        // Test values outside the range
        let values = Arc::new(Float32Array::from(vec![-0.5, 1.5])) as ArrayRef;
        let result = scale.invert_from_numeric(&config, &values).unwrap();
        let result_vec = result.as_vec(values.len(), None);
        assert_approx_eq!(f32, result_vec[0], -100.0);
        assert_approx_eq!(f32, result_vec[1], 100.0);
    }

    #[test]
    fn test_invert_constant() {
        let scale = SymlogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![-100.0, 100.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 1.0])),
            options: vec![("constant".to_string(), 2.0.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        // Test that invert(scale(x)) ≈ x with different constant
        let values = Arc::new(Float32Array::from(vec![-50.0, 0.0, 50.0])) as ArrayRef;
        let scaled = scale.scale_to_numeric(&config, &values).unwrap();
        let scaled_array =
            Arc::new(Float32Array::from(scaled.as_vec(values.len(), None))) as ArrayRef;

        let inverted = scale.invert_from_numeric(&config, &scaled_array).unwrap();
        let inverted_array =
            Arc::new(Float32Array::from(inverted.as_vec(values.len(), None))) as ArrayRef;

        let values_f32 = values.as_primitive::<Float32Type>();
        let inverted_f32 = inverted_array.as_primitive::<Float32Type>();
        for i in 0..values.len() {
            assert_approx_eq!(f32, inverted_f32.value(i), values_f32.value(i));
        }
    }

    #[test]
    fn test_invert_edge_cases() {
        let scale = SymlogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![-100.0, 100.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 1.0])),
            options: HashMap::new(),
            context: ScaleContext::default(),
        };

        // Test NaN
        let values = Arc::new(Float32Array::from(vec![f32::NAN])) as ArrayRef;
        let result = scale.invert_from_numeric(&config, &values).unwrap();
        let result_vec = result.as_vec(values.len(), None);
        assert!(result_vec[0].is_nan());

        // Test infinity
        let values =
            Arc::new(Float32Array::from(vec![f32::INFINITY, f32::NEG_INFINITY])) as ArrayRef;
        let result = scale.invert_from_numeric(&config, &values).unwrap();
        let result_vec = result.as_vec(values.len(), None);
        assert_approx_eq!(f32, result_vec[0], 100.0); // maps to domain end
        assert_approx_eq!(f32, result_vec[1], -100.0); // maps to domain start
    }

    #[test]
    fn test_invert_degenerate() {
        // Test degenerate domain
        let scale = SymlogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![1.0, 1.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 1.0])),
            options: HashMap::new(),
            context: ScaleContext::default(),
        };
        let values = Arc::new(Float32Array::from(vec![0.0, 0.5, 1.0])) as ArrayRef;
        let result = scale.invert_from_numeric(&config, &values).unwrap();
        let result_vec = result.as_vec(values.len(), None);
        for &value in &result_vec {
            assert_approx_eq!(f32, value, 1.0);
        }

        // Test degenerate range
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![-100.0, 100.0])),
            range: Arc::new(Float32Array::from(vec![1.0, 1.0])),
            options: HashMap::new(),
            context: ScaleContext::default(),
        };
        let result = scale.invert_from_numeric(&config, &values).unwrap();
        let result_vec = result.as_vec(values.len(), None);
        for &value in &result_vec {
            assert_approx_eq!(f32, value, -100.0);
        }
    }

    #[test]
    fn test_nice() {
        let constant = 2.0;
        let nice_domain =
            SymlogScale::apply_nice((0.1, 0.9), constant, Some(&10.0.into())).unwrap();

        // Invert the known nice domain in linear space.
        let transformed_domain = (symlog_invert(0.0, constant), symlog_invert(0.4, constant));

        // The domain should NOT change after nice() with these values
        assert_approx_eq!(f32, transformed_domain.0, nice_domain.0);
        assert_approx_eq!(f32, transformed_domain.1, nice_domain.1);
    }

    #[test]
    fn test_ticks() {
        let scale = SymlogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![-1.0, 1.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 1.0])),
            options: HashMap::new(),
            context: ScaleContext::default(),
        };
        let ticks = scale.ticks(&config, Some(10.0)).unwrap();
        let ticks_array = ticks.as_primitive::<Float32Type>();
        let expected = [
            -1.0f32, -0.8, -0.6, -0.4, -0.2, 0.0, 0.2, 0.4, 0.6, 0.8, 1.0,
        ];

        assert_eq!(ticks.len(), expected.len());
        for (a, b) in ticks_array.values().iter().zip(expected.iter()) {
            assert_approx_eq!(f32, *a, *b);
        }
    }

    #[test]
    fn test_ticks_with_constant() {
        let constant = 2.0;
        let scale = SymlogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![-10.0, 10.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 1.0])),
            options: vec![("constant".to_string(), constant.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let ticks = scale.ticks(&config, Some(5.0)).unwrap();
        let ticks_array = ticks.as_primitive::<Float32Type>();
        assert!(!ticks.is_empty());

        // Ticks should be symmetric around zero
        let mid_idx = ticks.len() / 2;
        if ticks.len() % 2 == 1 {
            assert_approx_eq!(f32, ticks_array.value(mid_idx), 0.0);
        }

        // Test symmetry of positive/negative ticks
        for i in 0..mid_idx {
            assert_approx_eq!(
                f32,
                ticks_array.value(i).abs(),
                ticks_array.value(ticks.len() - 1 - i).abs()
            );
        }
    }
}
