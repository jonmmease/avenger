use std::sync::Arc;

use crate::scalar::Scalar;
use arrow::{
    array::{ArrayRef, AsArray, Float32Array},
    compute::{kernels::cast, unary},
    datatypes::{DataType, Float32Type},
};
use avenger_common::{
    types::ColorOrGradient,
    value::{ScalarOrArray, ScalarOrArrayValue},
};

use crate::{array, color_interpolator::scale_numeric_to_color, error::AvengerScaleError};

use super::{
    linear::LinearScale, ConfiguredScale, InferDomainFromDataMethod, ScaleConfig, ScaleContext,
    ScaleImpl,
};

/// Power scale that maps a continuous numeric domain to a continuous numeric range
/// using power (exponential) transformation.
///
/// The scale applies x^exponent transformation to input values. It supports negative
/// values by preserving the sign: sign(x) * |x|^exponent. Common exponents include
/// 0.5 (square root) for area-based encodings and 2 (square) for emphasizing differences.
///
/// # Config Options
///
/// - **exponent** (f32, default: 1.0): The power exponent. When 1.0, behaves like a
///   linear scale. Values < 1 compress large values and expand small values.
///   Values > 1 expand large values and compress small values. Must not be 0.
///
/// - **clamp** (boolean, default: false): When true, values outside the domain are
///   clamped to the domain extent before transformation. For inversion, values
///   outside the range are clamped first.
///
/// - **range_offset** (f32, default: 0.0): An offset applied to the final scaled
///   values. This is added after the power transformation and linear mapping.
///   Note: In inversion, this is subtracted from the output, not the input.
///
/// - **round** (boolean, default: false): When true, output values from scaling
///   are rounded to the nearest integer. Useful for pixel-perfect rendering.
///   Does not affect inversion.
///
/// - **nice** (boolean or f32, default: false): When true or a number, extends
///   the domain to nice round values in the transformed space. If true, uses
///   a default count of 10. If a number, uses that as the target tick count.
#[derive(Debug)]
pub struct PowScale;

impl PowScale {
    pub fn configured(domain: (f32, f32), range: (f32, f32)) -> ConfiguredScale {
        ConfiguredScale {
            scale_impl: Arc::new(Self),
            config: ScaleConfig {
                domain: Arc::new(Float32Array::from(vec![domain.0, domain.1])),
                range: Arc::new(Float32Array::from(vec![range.0, range.1])),
                options: vec![
                    ("exponent".to_string(), 1.0.into()),
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
        exponent: f32,
        count: Option<&Scalar>,
    ) -> Result<(f32, f32), AvengerScaleError> {
        // Transform domain to linear space using power function
        let power_fun = PowerFunction::new(exponent);
        let d0 = power_fun.pow(domain.0);
        let d1 = power_fun.pow(domain.1);

        // Use linear scale to nice the transformed values
        let (nice_d0, nice_d1) = LinearScale::apply_nice((d0, d1), count)?;

        // Transforming back to original domain
        let domain_start = power_fun.pow_inv(nice_d0);
        let domain_end = power_fun.pow_inv(nice_d1);
        Ok((domain_start, domain_end))
    }
}

impl ScaleImpl for PowScale {
    fn scale_type(&self) -> &'static str {
        "pow"
    }

    fn infer_domain_from_data_method(&self) -> InferDomainFromDataMethod {
        InferDomainFromDataMethod::Interval
    }

    fn invert(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError> {
        // Cast input to Float32 if needed
        let float_values = cast(values, &DataType::Float32)?;

        // Call existing invert_from_numeric
        let result = self.invert_from_numeric(config, &float_values)?;

        // Convert ScalarOrArray<f32> to ArrayRef
        match result.value() {
            ScalarOrArrayValue::Scalar(s) => {
                // If scalar, create array with single value repeated for input length
                Ok(Arc::new(Float32Array::from(vec![*s; values.len()])) as ArrayRef)
            }
            ScalarOrArrayValue::Array(arr) => {
                // If array, convert to ArrayRef
                Ok(Arc::new(Float32Array::from(arr.as_ref().clone())) as ArrayRef)
            }
        }
    }

    fn scale(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError> {
        // Get options
        let exponent = config.option_f32("exponent", 1.0);
        let range_offset = config.option_f32("range_offset", 0.0);
        let clamp = config.option_boolean("clamp", false);
        let round = config.option_boolean("round", false);

        let (range_start, range_end) = config.numeric_interval_range()?;
        let (domain_start, domain_end) = PowScale::apply_nice(
            config.numeric_interval_domain()?,
            exponent,
            config.options.get("nice"),
        )?;

        // Check if color interpolation is needed
        if config.color_range().is_ok() {
            // Create new config with niced domain
            let config = ScaleConfig {
                domain: Arc::new(Float32Array::from(vec![domain_start, domain_end])),
                ..config.clone()
            };
            return scale_numeric_to_color(self, &config, values);
        }

        // If range start equals end, return constant range value
        if range_start == range_end {
            return Ok(Arc::new(Float32Array::from(vec![
                range_start;
                values.len()
            ])));
        }

        // Build the power function
        let power_fun = PowerFunction::new(exponent);

        let d0 = power_fun.pow(domain_start);
        let d1 = power_fun.pow(domain_end);

        // If domain start equals end, return constant domain value
        if d0 == d1 {
            return Ok(Arc::new(Float32Array::from(vec![
                domain_start;
                values.len()
            ])));
        }

        // At this point, we know (d1 - d0) cannot be zero
        let scale = (range_end - range_start) / (d1 - d0);
        let offset = range_start - scale * power_fun.pow(domain_start) + range_offset;

        let (range_min, range_max) = if range_start <= range_end {
            (range_start, range_end)
        } else {
            (range_end, range_start)
        };

        // Cast to f32 and downcast to f32 array
        let values = cast(values, &DataType::Float32)?;
        let values = values.as_primitive::<Float32Type>();

        match (clamp, round) {
            (true, false) => match &power_fun {
                // Clamp, no rounding
                PowerFunction::Static { pow_fun, .. } => {
                    // Predefined power function
                    Ok(Arc::<Float32Array>::new(unary(values, |v| {
                        let abs_v = v.abs();
                        let sign = if v < 0.0 { -1.0 } else { 1.0 };
                        (scale * (sign * pow_fun(abs_v)) + offset).clamp(range_min, range_max)
                    })) as ArrayRef)
                }
                PowerFunction::Custom { exponent } => {
                    // Custom power function
                    let exponent = *exponent;
                    Ok(Arc::<Float32Array>::new(unary(values, |v| {
                        let abs_v = v.abs();
                        let sign = if v < 0.0 { -1.0 } else { 1.0 };
                        (scale * (sign * abs_v.powf(exponent)) + offset).clamp(range_min, range_max)
                    })) as ArrayRef)
                }
            },
            (true, true) => match &power_fun {
                // Clamp and rounding
                PowerFunction::Static { pow_fun, .. } => {
                    // Predefined power function
                    Ok(Arc::<Float32Array>::new(unary(values, |v| {
                        let abs_v = v.abs();
                        let sign = if v < 0.0 { -1.0 } else { 1.0 };
                        (scale * (sign * pow_fun(abs_v)) + offset)
                            .clamp(range_min, range_max)
                            .round()
                    })) as ArrayRef)
                }
                PowerFunction::Custom { exponent } => {
                    // Custom power function
                    let exponent = *exponent;
                    Ok(Arc::<Float32Array>::new(unary(values, |v| {
                        let abs_v = v.abs();
                        let sign = if v < 0.0 { -1.0 } else { 1.0 };
                        (scale * (sign * abs_v.powf(exponent)) + offset)
                            .clamp(range_min, range_max)
                            .round()
                    })) as ArrayRef)
                }
            },
            (false, false) => match &power_fun {
                // no clamping or rounding
                PowerFunction::Static { pow_fun, .. } => {
                    // predefined power function
                    Ok(Arc::<Float32Array>::new(unary(values, |v| {
                        let abs_v = v.abs();
                        let sign = if v < 0.0 { -1.0 } else { 1.0 };
                        scale * (sign * pow_fun(abs_v)) + offset
                    })) as ArrayRef)
                }
                PowerFunction::Custom { exponent } => {
                    // custom power function
                    let exponent = *exponent;
                    Ok(Arc::<Float32Array>::new(unary(values, |v| {
                        let abs_v = v.abs();
                        let sign = if v < 0.0 { -1.0 } else { 1.0 };
                        scale * (sign * abs_v.powf(exponent)) + offset
                    })) as ArrayRef)
                }
            },
            (false, true) => match &power_fun {
                // no clamping and rounding
                PowerFunction::Static { pow_fun, .. } => {
                    // predefined power function
                    Ok(Arc::<Float32Array>::new(unary(values, |v| {
                        let abs_v = v.abs();
                        let sign = if v < 0.0 { -1.0 } else { 1.0 };
                        (scale * (sign * pow_fun(abs_v)) + offset).round()
                    })) as ArrayRef)
                }
                PowerFunction::Custom { exponent } => {
                    // custom power function
                    let exponent = *exponent;
                    Ok(Arc::<Float32Array>::new(unary(values, |v| {
                        let abs_v = v.abs();
                        let sign = if v < 0.0 { -1.0 } else { 1.0 };
                        (scale * (sign * abs_v.powf(exponent)) + offset).round()
                    })) as ArrayRef)
                }
            },
        }
    }

    fn invert_from_numeric(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        // Get options
        let exponent = config.option_f32("exponent", 1.0);
        let range_offset = config.option_f32("range_offset", 0.0);
        let clamp = config.option_boolean("clamp", false);

        let (range_start, range_end) = config.numeric_interval_range()?;
        let (domain_start, domain_end) = PowScale::apply_nice(
            config.numeric_interval_domain()?,
            exponent,
            config.options.get("nice"),
        )?;

        // If domain start equals end, return constant domain value
        if domain_start == domain_end {
            return Ok(ScalarOrArray::new_array(vec![domain_start; values.len()]));
        }

        // Build the power function
        let power_fun = PowerFunction::new(exponent);
        let d0 = power_fun.pow(domain_start);
        let d1 = power_fun.pow(domain_end);

        let scale = (range_end - range_start) / (d1 - d0);
        let offset = range_start - scale * d0;

        // Cast to f32 and downcast to f32 array
        let array = cast(values, &DataType::Float32)?;
        let array = array.as_primitive::<Float32Type>();

        if clamp {
            let (range_min, range_max) = if range_start <= range_end {
                (range_start, range_end)
            } else {
                (range_end, range_start)
            };

            match &power_fun {
                PowerFunction::Static { pow_inv_fun, .. } => Ok(ScalarOrArray::new_array(
                    array
                        .values()
                        .iter()
                        .map(|&v| {
                            let v = v.clamp(range_min, range_max);
                            let normalized = (v - offset) / scale;
                            let abs_norm = normalized.abs();
                            let sign = if normalized < 0.0 { -1.0 } else { 1.0 };
                            sign * pow_inv_fun(abs_norm) - range_offset
                        })
                        .collect(),
                )),
                PowerFunction::Custom { exponent } => {
                    let inv_exponent = 1.0 / exponent;
                    Ok(ScalarOrArray::new_array(
                        array
                            .values()
                            .iter()
                            .map(|&v| {
                                let v = v.clamp(range_min, range_max);
                                let normalized = (v - offset) / scale;
                                let abs_norm = normalized.abs();
                                let sign = if normalized < 0.0 { -1.0 } else { 1.0 };
                                sign * abs_norm.powf(inv_exponent) - range_offset
                            })
                            .collect(),
                    ))
                }
            }
        } else {
            match &power_fun {
                PowerFunction::Static { pow_inv_fun, .. } => Ok(ScalarOrArray::new_array(
                    array
                        .values()
                        .iter()
                        .map(|&v| {
                            let normalized = (v - offset) / scale;
                            let abs_norm = normalized.abs();
                            let sign = if normalized < 0.0 { -1.0 } else { 1.0 };
                            sign * pow_inv_fun(abs_norm) - range_offset
                        })
                        .collect(),
                )),
                PowerFunction::Custom { exponent } => {
                    let inv_exponent = 1.0 / exponent;
                    Ok(ScalarOrArray::new_array(
                        array
                            .values()
                            .iter()
                            .map(|&v| {
                                let normalized = (v - offset) / scale;
                                let abs_norm = normalized.abs();
                                let sign = if normalized < 0.0 { -1.0 } else { 1.0 };
                                sign * abs_norm.powf(inv_exponent) - range_offset
                            })
                            .collect(),
                    ))
                }
            }
        }
    }

    /// Scale to color values
    fn scale_to_color(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ScalarOrArray<ColorOrGradient>, AvengerScaleError> {
        let scaled = self.scale(config, values)?;
        config.context.color_coercer.coerce(&scaled, None)
    }

    fn ticks(
        &self,
        config: &ScaleConfig,
        count: Option<f32>,
    ) -> Result<ArrayRef, AvengerScaleError> {
        let exponent = config.option_f32("exponent", 1.0);
        let (domain_start, domain_end) = PowScale::apply_nice(
            config.numeric_interval_domain()?,
            exponent,
            config.options.get("nice"),
        )?;
        let count = count.unwrap_or(10.0);
        let ticks_array = Float32Array::from(array::ticks(domain_start, domain_end, count));
        Ok(Arc::new(ticks_array) as ArrayRef)
    }
}

/// Handles power transformations with different exponents
#[derive(Clone, Debug)]
enum PowerFunction {
    Static {
        pow_fun: fn(f32) -> f32,
        pow_inv_fun: fn(f32) -> f32,
    },
    Custom {
        exponent: f32,
    },
}

impl PowerFunction {
    /// Creates a new PowerFunction with optimized implementations for common exponents
    pub fn new(exponent: f32) -> Self {
        if exponent == 2.0 {
            PowerFunction::Static {
                pow_fun: |x| x * x,
                pow_inv_fun: f32::sqrt,
            }
        } else if exponent == 0.5 {
            PowerFunction::Static {
                pow_fun: f32::sqrt,
                pow_inv_fun: |x| x * x,
            }
        } else {
            PowerFunction::Custom { exponent }
        }
    }

    /// Raises the absolute value of x to the power, preserving sign
    pub fn pow(&self, x: f32) -> f32 {
        let abs_x = x.abs();
        let sign = if x < 0.0 { -1.0 } else { 1.0 };
        match self {
            PowerFunction::Static { pow_fun, .. } => sign * pow_fun(abs_x),
            PowerFunction::Custom { exponent } => sign * abs_x.powf(*exponent),
        }
    }

    /// Computes the inverse power transform, preserving sign
    pub fn pow_inv(&self, x: f32) -> f32 {
        let abs_x = x.abs();
        let sign = if x < 0.0 { -1.0 } else { 1.0 };
        match self {
            PowerFunction::Static { pow_inv_fun, .. } => sign * pow_inv_fun(abs_x),
            PowerFunction::Custom { exponent } => sign * abs_x.powf(1.0 / *exponent),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use float_cmp::assert_approx_eq;

    #[test]
    fn test_scale() -> Result<(), AvengerScaleError> {
        // Test scaling with edge cases: out-of-bounds, nulls, and interpolation
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![10.0, 30.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 100.0])),
            options: vec![("clamp".to_string(), true.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let scale = LinearScale;
        let values = Arc::new(Float32Array::from(vec![
            0.0,  // < domain
            10.0, // domain start
            15.0, 20.0, 25.0, 30.0, // in domain
            40.0, // > domain
        ])) as ArrayRef;

        let result = scale
            .scale_to_numeric(&config, &values)?
            .as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 0.0); // clamped
        assert_approx_eq!(f32, result[1], 0.0); // domain start
        assert_approx_eq!(f32, result[2], 25.0); // interpolated
        assert_approx_eq!(f32, result[3], 50.0); // interpolated
        assert_approx_eq!(f32, result[4], 75.0); // interpolated
        assert_approx_eq!(f32, result[5], 100.0); // domain end
        assert_approx_eq!(f32, result[6], 100.0); // clamped

        Ok(())
    }

    #[test]
    fn test_scale_with_range_offset() -> Result<(), AvengerScaleError> {
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![10.0, 30.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 100.0])),
            options: vec![
                ("range_offset".to_string(), 3.0.into()),
                ("clamp".to_string(), true.into()),
            ]
            .into_iter()
            .collect(),
            context: ScaleContext::default(),
        };

        let scale = LinearScale;
        let values = Arc::new(Float32Array::from(vec![
            0.0,  // < domain
            10.0, // domain start
            15.0, 20.0, 25.0, 30.0, // in domain
            40.0, // > domain
        ])) as ArrayRef;

        let result = scale
            .scale_to_numeric(&config, &values)?
            .as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 0.0); // clamped
        assert_approx_eq!(f32, result[1], 3.0); // domain start
        assert_approx_eq!(f32, result[2], 28.0); // interpolated
        assert_approx_eq!(f32, result[3], 53.0); // interpolated
        assert_approx_eq!(f32, result[4], 78.0); // interpolated
        assert_approx_eq!(f32, result[5], 100.0); // domain end
        assert_approx_eq!(f32, result[6], 100.0); // clamped

        Ok(())
    }

    #[test]
    fn test_scale_degenerate() -> Result<(), AvengerScaleError> {
        // Tests behavior with zero-width domain (matches d3 behavior)
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![10.0, 10.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 100.0])),
            options: vec![("clamp".to_string(), true.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let scale = LinearScale;
        let values = Arc::new(Float32Array::from(vec![0.0, 10.0, 20.0])) as ArrayRef;
        let result = scale
            .scale_to_numeric(&config, &values)?
            .as_vec(values.len(), None);

        // All values should map to range_start (d3 behavior)
        for &value in &result {
            assert_approx_eq!(f32, value, 0.0);
        }

        Ok(())
    }

    #[test]
    fn test_degenerate_cases() -> Result<(), AvengerScaleError> {
        let scale = LinearScale;

        // Test degenerate domain
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![1.0, 1.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 0.0])),
            options: vec![("clamp".to_string(), false.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let values = Arc::new(Float32Array::from(vec![0.0, 1.0, 2.0])) as ArrayRef;
        let result = scale
            .scale_to_numeric(&config, &values)?
            .as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 0.0);
        assert_approx_eq!(f32, result[1], 0.0);
        assert_approx_eq!(f32, result[2], 0.0);

        // Test degenerate range
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![0.0, 10.0])),
            range: Arc::new(Float32Array::from(vec![1.0, 1.0])),
            options: vec![("clamp".to_string(), false.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let result = scale
            .scale_to_numeric(&config, &values)?
            .as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 1.0);
        assert_approx_eq!(f32, result[1], 1.0);
        assert_approx_eq!(f32, result[2], 1.0);

        Ok(())
    }

    #[test]
    fn test_invert_clamped() -> Result<(), AvengerScaleError> {
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![10.0, 30.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 100.0])),
            options: vec![("clamp".to_string(), true.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let scale = LinearScale;
        let values = Arc::new(Float32Array::from(vec![-25.0, 0.0, 50.0, 100.0, 125.0])) as ArrayRef;
        let result = scale
            .invert_from_numeric(&config, &values)?
            .as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 10.0); // clamped below
        assert_approx_eq!(f32, result[1], 10.0); // range start
        assert_approx_eq!(f32, result[2], 20.0); // interpolated
        assert_approx_eq!(f32, result[3], 30.0); // range end
        assert_approx_eq!(f32, result[4], 30.0); // clamped above

        Ok(())
    }

    #[test]
    fn test_invert_unclamped() -> Result<(), AvengerScaleError> {
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![10.0, 30.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 100.0])),
            options: vec![("clamp".to_string(), false.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let scale = LinearScale;
        let values = Arc::new(Float32Array::from(vec![-25.0, 0.0, 50.0, 100.0, 125.0])) as ArrayRef;
        let result = scale
            .invert_from_numeric(&config, &values)?
            .as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 5.0); // below range
        assert_approx_eq!(f32, result[1], 10.0); // range start
        assert_approx_eq!(f32, result[2], 20.0); // interpolated
        assert_approx_eq!(f32, result[3], 30.0); // range end
        assert_approx_eq!(f32, result[4], 35.0); // above range

        Ok(())
    }

    #[test]
    fn test_invert_with_range_offset() -> Result<(), AvengerScaleError> {
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![10.0, 30.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 100.0])),
            options: vec![("range_offset".to_string(), 3.0.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let scale = LinearScale;
        let values = Arc::new(Float32Array::from(vec![-22.0, 3.0, 53.0, 103.0, 128.0])) as ArrayRef;
        let result = scale
            .invert_from_numeric(&config, &values)?
            .as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 5.0); // below range
        assert_approx_eq!(f32, result[1], 10.0); // range start
        assert_approx_eq!(f32, result[2], 20.0); // interpolated
        assert_approx_eq!(f32, result[3], 30.0); // range end
        assert_approx_eq!(f32, result[4], 35.0); // above range

        Ok(())
    }

    #[test]
    fn test_invert_reversed_range() -> Result<(), AvengerScaleError> {
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![10.0, 30.0])),
            range: Arc::new(Float32Array::from(vec![100.0, 0.0])),
            options: vec![("clamp".to_string(), true.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let scale = LinearScale;
        let values = Arc::new(Float32Array::from(vec![125.0, 100.0, 50.0, 0.0, -25.0])) as ArrayRef;
        let result = scale
            .invert_from_numeric(&config, &values)?
            .as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 10.0); // clamped
        assert_approx_eq!(f32, result[1], 10.0); // range start
        assert_approx_eq!(f32, result[2], 20.0); // interpolated
        assert_approx_eq!(f32, result[3], 30.0); // range end
        assert_approx_eq!(f32, result[4], 30.0); // clamped

        Ok(())
    }

    #[test]
    fn test_ticks() -> Result<(), AvengerScaleError> {
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![0.0, 10.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 100.0])),
            options: vec![].into_iter().collect(),
            context: ScaleContext::default(),
        };

        let scale = LinearScale;

        let expected = vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0];
        let ticks_array = scale.ticks(&config, Some(5.0))?;
        let ticks_array = ticks_array.as_primitive::<Float32Type>();
        assert_eq!(ticks_array.values().to_vec(), expected);

        let expected = vec![0.0, 5.0, 10.0];
        let ticks_array = scale.ticks(&config, Some(2.0))?;
        let ticks_array = ticks_array.as_primitive::<Float32Type>();
        assert_eq!(ticks_array.values().to_vec(), expected);

        let expected = vec![0.0, 10.0];
        let ticks_array = scale.ticks(&config, Some(1.0))?;
        let ticks_array = ticks_array.as_primitive::<Float32Type>();
        assert_eq!(ticks_array.values().to_vec(), expected);

        Ok(())
    }

    #[test]
    fn test_ticks_span_zero() -> Result<(), AvengerScaleError> {
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![-100.0, 100.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 100.0])),
            options: vec![].into_iter().collect(),
            context: ScaleContext::default(),
        };

        let scale = LinearScale;
        let ticks_array = scale.ticks(&config, Some(10.0))?;
        let ticks_array = ticks_array.as_primitive::<Float32Type>();
        assert_eq!(
            ticks_array.values().to_vec(),
            vec![-100.0, -80.0, -60.0, -40.0, -20.0, 0.0, 20.0, 40.0, 60.0, 80.0, 100.0]
        );

        let ticks_array = scale.ticks(&config, Some(5.0))?;
        let ticks_array = ticks_array.as_primitive::<Float32Type>();
        assert_eq!(
            ticks_array.values().to_vec(),
            vec![-100.0, -50.0, 0.0, 50.0, 100.0]
        );

        let ticks_array = scale.ticks(&config, Some(2.0))?;
        let ticks_array = ticks_array.as_primitive::<Float32Type>();
        assert_eq!(ticks_array.values().to_vec(), vec![-100.0, 0.0, 100.0]);

        let ticks_array = scale.ticks(&config, Some(1.0))?;
        let ticks_array = ticks_array.as_primitive::<Float32Type>();
        assert_eq!(ticks_array.values().to_vec(), vec![0.0]);

        Ok(())
    }

    #[test]
    fn test_nice_convergence() -> Result<(), AvengerScaleError> {
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![1.1, 10.9])),
            range: Arc::new(Float32Array::from(vec![0.0, 100.0])),
            options: vec![].into_iter().collect(),
            context: ScaleContext::default(),
        };

        let niced_domain =
            PowScale::apply_nice(config.numeric_interval_domain()?, 1.0, Some(&10.0.into()))?;
        assert_eq!(niced_domain, (1.0, 11.0));

        Ok(())
    }

    #[test]
    fn test_configured_scale_invert_pow() -> Result<(), AvengerScaleError> {
        // Test the new invert() method with pow scale
        // Using exponent=2, domain [0,4], range [0,16]
        let scale = PowScale::configured((0.0, 4.0), (0.0, 16.0)).with_option("exponent", 2.0);

        // Test with Float32 values
        let values = Arc::new(Float32Array::from(vec![0.0, 4.0, 9.0, 16.0])) as ArrayRef;
        let result = scale.invert(&values)?;
        let result_array = result.as_primitive::<Float32Type>();

        assert_eq!(result.len(), 4);
        assert_approx_eq!(f32, result_array.value(0), 0.0); // 0^2 = 0 -> 0
        assert_approx_eq!(f32, result_array.value(1), 2.0); // 2^2 = 4 -> 2
        assert_approx_eq!(f32, result_array.value(2), 3.0); // 3^2 = 9 -> 3
        assert_approx_eq!(f32, result_array.value(3), 4.0); // 4^2 = 16 -> 4

        Ok(())
    }
}
