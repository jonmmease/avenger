use std::sync::Arc;

use arrow::{
    array::{ArrayRef, AsArray, Float32Array, Float64Array},
    compute::{kernels::cast, unary},
    datatypes::{DataType, Float64Type},
};
use avenger_color::ColorOrGradient;
use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};

use crate::{array, color_interpolator::scale_numeric_to_color, error::AvengerScaleError};

use super::domain_summary::{fit, DomainOptions, DomainSummary, Extents, NiceStep};
use super::no_domain;
use super::{
    ConfiguredScale, DomainKind, OptionConstraint, OptionDefinition, ScaleConfig, ScaleContext,
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
/// - **exponent** (f64, default: 1.0): The power exponent. When 1.0, behaves like a
///   linear scale. Values < 1 compress large values and expand small values.
///   Values > 1 expand large values and compress small values. Must not be 0.
///
/// - **clamp** (boolean, default: false): When true, values outside the domain are
///   clamped to the domain extent before transformation. For inversion, values
///   outside the range are clamped first.
///
/// - **range_offset** (f64, default: 0.0): An offset applied to the final scaled
///   values. This is added after the power transformation and linear mapping.
///   Note: In inversion, this is subtracted from the output, not the input.
///
/// - **round** (boolean, default: false): When true, output values from scaling
///   are rounded to the nearest integer. Useful for pixel-perfect rendering.
///   Does not affect inversion.
#[derive(Debug, Serialize, Deserialize)]
pub struct PowScale;

impl PowScale {
    /// A power scale onto `range` with no domain yet. Scaling needs a domain, which
    /// [`ConfiguredScale::fit_domain`] fits to data.
    pub fn unfitted(range: (f64, f64)) -> ConfiguredScale {
        Self::configured((0.0, 1.0), range).with_domain(no_domain())
    }

    pub fn configured(domain: (f64, f64), range: (f64, f64)) -> ConfiguredScale {
        ConfiguredScale {
            scale_impl: Arc::new(Self),
            config: ScaleConfig {
                domain: Arc::new(Float64Array::from(vec![domain.0, domain.1])),
                range: Arc::new(Float64Array::from(vec![range.0, range.1])),
                options: vec![
                    ("exponent".to_string(), 1.0.into()),
                    ("clamp".to_string(), false.into()),
                    ("range_offset".to_string(), 0.0.into()),
                    ("round".to_string(), false.into()),
                ]
                .into_iter()
                .collect(),
                context: ScaleContext::default(),
            },
        }
    }
}

#[typetag::serde(name = "pow")]
impl ScaleImpl for PowScale {
    fn scale_type(&self) -> &'static str {
        "pow"
    }

    fn summarize(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
        extents: Extents,
    ) -> Result<DomainSummary, AvengerScaleError> {
        let exponent = config.option_f64("exponent", 1.0);
        let power = PowerFunction::new(exponent);
        let values = cast(values, &DataType::Float64)?;
        let values = values
            .as_primitive::<Float64Type>()
            .unary::<_, Float64Type>(|value| power.pow(value));
        fit::interval_summary(&values, extents)
    }

    fn fit_domain(
        &self,
        config: &ScaleConfig,
        options: &DomainOptions,
        summary: &DomainSummary,
    ) -> Result<ScaleConfig, AvengerScaleError> {
        let options = options.interval()?;
        let exponent = config.option_f64("exponent", 1.0);
        let power = PowerFunction::new(exponent);
        fit::fit_continuous(
            config,
            options,
            summary,
            fit::IntervalFit {
                nice: options.nice.map(NiceStep::Auto),
                zero: options.zero.then_some(0.0),
                single_value_span: fit::linear_single_value_span,
            },
            |value| power.pow_inv(value),
            (0.0, 1.0),
        )
    }

    fn domain_kind(&self) -> DomainKind {
        DomainKind::Numeric
    }

    fn option_definitions(&self) -> &[OptionDefinition] {
        lazy_static! {
            static ref DEFINITIONS: Vec<OptionDefinition> = vec![
                OptionDefinition::optional("exponent", OptionConstraint::Float),
                OptionDefinition::optional("clamp", OptionConstraint::Boolean),
                OptionDefinition::optional("range_offset", OptionConstraint::Float),
                OptionDefinition::optional("round", OptionConstraint::Boolean),
                OptionDefinition::optional("default", OptionConstraint::Float),
            ];
        }

        &DEFINITIONS
    }

    fn invert(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError> {
        // Cast input to Float64 if needed
        let float_values = cast(values, &DataType::Float64)?;

        // Call existing invert_from_numeric
        let result = self.invert_from_numeric(config, &float_values)?;

        // Convert ScalarOrArray<f64> to ArrayRef
        match result.value() {
            ScalarOrArrayValue::Scalar(s) => {
                // If scalar, create array with single value repeated for input length
                Ok(Arc::new(Float64Array::from(vec![*s; values.len()])) as ArrayRef)
            }
            ScalarOrArrayValue::Array(arr) => {
                // If array, convert to ArrayRef
                Ok(Arc::new(Float64Array::from(arr.as_ref().clone())) as ArrayRef)
            }
        }
    }

    fn scale(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError> {
        // Get options
        let exponent = config.option_f64("exponent", 1.0);
        let range_offset = config.option_f64("range_offset", 0.0);
        let clamp = config.option_boolean("clamp", false);
        let round = config.option_boolean("round", false);

        let (domain_start, domain_end) = config.numeric_interval_domain()?;

        // Check if color interpolation is needed
        if config.color_range().is_ok() {
            return scale_numeric_to_color(self, config, values);
        }

        let (range_start, range_end) = config.numeric_interval_range()?;

        // If range start equals end, return constant range value
        if range_start == range_end {
            return Ok(Arc::new(Float32Array::from(vec![
                range_start as f32;
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
                domain_start as f32;
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

        // Cast to f64 and downcast to f64 array
        let values = cast(values, &DataType::Float64)?;
        let values = values.as_primitive::<Float64Type>();

        match (clamp, round) {
            (true, false) => match &power_fun {
                // Clamp, no rounding
                PowerFunction::Static { pow_fun, .. } => {
                    // Predefined power function
                    Ok(Arc::<Float32Array>::new(unary(values, |v| {
                        let abs_v = v.abs();
                        let sign = if v < 0.0 { -1.0 } else { 1.0 };
                        (scale * (sign * pow_fun(abs_v)) + offset).clamp(range_min, range_max)
                    } as f32)) as ArrayRef)
                }
                PowerFunction::Custom { exponent } => {
                    // Custom power function
                    let exponent = *exponent;
                    Ok(Arc::<Float32Array>::new(unary(values, |v| {
                        let abs_v = v.abs();
                        let sign = if v < 0.0 { -1.0 } else { 1.0 };
                        (scale * (sign * abs_v.powf(exponent)) + offset).clamp(range_min, range_max)
                    } as f32)) as ArrayRef)
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
                    } as f32)) as ArrayRef)
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
                    } as f32)) as ArrayRef)
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
                    } as f32)) as ArrayRef)
                }
                PowerFunction::Custom { exponent } => {
                    // custom power function
                    let exponent = *exponent;
                    Ok(Arc::<Float32Array>::new(unary(values, |v| {
                        let abs_v = v.abs();
                        let sign = if v < 0.0 { -1.0 } else { 1.0 };
                        scale * (sign * abs_v.powf(exponent)) + offset
                    } as f32)) as ArrayRef)
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
                    } as f32)) as ArrayRef)
                }
                PowerFunction::Custom { exponent } => {
                    // custom power function
                    let exponent = *exponent;
                    Ok(Arc::<Float32Array>::new(unary(values, |v| {
                        let abs_v = v.abs();
                        let sign = if v < 0.0 { -1.0 } else { 1.0 };
                        (scale * (sign * abs_v.powf(exponent)) + offset).round()
                    } as f32)) as ArrayRef)
                }
            },
        }
    }

    fn invert_from_numeric(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ScalarOrArray<f64>, AvengerScaleError> {
        // Get options
        let exponent = config.option_f64("exponent", 1.0);
        let range_offset = config.option_f64("range_offset", 0.0);
        let clamp = config.option_boolean("clamp", false);

        let (range_start, range_end) = config.numeric_interval_range()?;
        let (domain_start, domain_end) = config.numeric_interval_domain()?;

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

        // Cast to f64 and downcast to f64 array
        let array = cast(values, &DataType::Float64)?;
        let array = array.as_primitive::<Float64Type>();

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
        count: Option<f64>,
    ) -> Result<ArrayRef, AvengerScaleError> {
        let (domain_start, domain_end) = config.numeric_interval_domain()?;
        let count = count.unwrap_or(10.0);
        Ok(array::tick_array(array::ticks(
            domain_start,
            domain_end,
            count,
        )))
    }
}

/// Handles power transformations with different exponents
#[derive(Clone, Debug)]
enum PowerFunction {
    Static {
        pow_fun: fn(f64) -> f64,
        pow_inv_fun: fn(f64) -> f64,
    },
    Custom {
        exponent: f64,
    },
}

impl PowerFunction {
    /// Creates a new PowerFunction with optimized implementations for common exponents
    pub fn new(exponent: f64) -> Self {
        if exponent == 2.0 {
            PowerFunction::Static {
                pow_fun: |x| x * x,
                pow_inv_fun: f64::sqrt,
            }
        } else if exponent == 0.5 {
            PowerFunction::Static {
                pow_fun: f64::sqrt,
                pow_inv_fun: |x| x * x,
            }
        } else {
            PowerFunction::Custom { exponent }
        }
    }

    /// Raises the absolute value of x to the power, preserving sign
    pub fn pow(&self, x: f64) -> f64 {
        let abs_x = x.abs();
        let sign = if x < 0.0 { -1.0 } else { 1.0 };
        match self {
            PowerFunction::Static { pow_fun, .. } => sign * pow_fun(abs_x),
            PowerFunction::Custom { exponent } => sign * abs_x.powf(*exponent),
        }
    }

    /// Computes the inverse power transform, preserving sign
    pub fn pow_inv(&self, x: f64) -> f64 {
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
    use crate::scales::linear::LinearScale;
    use arrow::datatypes::Float64Type;
    use float_cmp::assert_approx_eq;

    #[test]
    fn test_scale() -> Result<(), AvengerScaleError> {
        // Test scaling with edge cases: out-of-bounds, nulls, and interpolation
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![10.0, 30.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 100.0])),
            options: vec![("clamp".to_string(), true.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let scale = LinearScale;
        let values = Arc::new(Float64Array::from(vec![
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
            domain: Arc::new(Float64Array::from(vec![10.0, 30.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 100.0])),
            options: vec![
                ("range_offset".to_string(), 3.0.into()),
                ("clamp".to_string(), true.into()),
            ]
            .into_iter()
            .collect(),
            context: ScaleContext::default(),
        };

        let scale = LinearScale;
        let values = Arc::new(Float64Array::from(vec![
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
            domain: Arc::new(Float64Array::from(vec![10.0, 10.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 100.0])),
            options: vec![("clamp".to_string(), true.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let scale = LinearScale;
        let values = Arc::new(Float64Array::from(vec![0.0, 10.0, 20.0])) as ArrayRef;
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
            domain: Arc::new(Float64Array::from(vec![1.0, 1.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 0.0])),
            options: vec![("clamp".to_string(), false.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let values = Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0])) as ArrayRef;
        let result = scale
            .scale_to_numeric(&config, &values)?
            .as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 0.0);
        assert_approx_eq!(f32, result[1], 0.0);
        assert_approx_eq!(f32, result[2], 0.0);

        // Test degenerate range
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![0.0, 10.0])),
            range: Arc::new(Float64Array::from(vec![1.0, 1.0])),
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
            domain: Arc::new(Float64Array::from(vec![10.0, 30.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 100.0])),
            options: vec![("clamp".to_string(), true.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let scale = LinearScale;
        let values = Arc::new(Float64Array::from(vec![-25.0, 0.0, 50.0, 100.0, 125.0])) as ArrayRef;
        let result = scale
            .invert_from_numeric(&config, &values)?
            .as_vec(values.len(), None);

        assert_approx_eq!(f64, result[0], 10.0); // clamped below
        assert_approx_eq!(f64, result[1], 10.0); // range start
        assert_approx_eq!(f64, result[2], 20.0); // interpolated
        assert_approx_eq!(f64, result[3], 30.0); // range end
        assert_approx_eq!(f64, result[4], 30.0); // clamped above

        Ok(())
    }

    #[test]
    fn test_invert_unclamped() -> Result<(), AvengerScaleError> {
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![10.0, 30.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 100.0])),
            options: vec![("clamp".to_string(), false.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let scale = LinearScale;
        let values = Arc::new(Float64Array::from(vec![-25.0, 0.0, 50.0, 100.0, 125.0])) as ArrayRef;
        let result = scale
            .invert_from_numeric(&config, &values)?
            .as_vec(values.len(), None);

        assert_approx_eq!(f64, result[0], 5.0); // below range
        assert_approx_eq!(f64, result[1], 10.0); // range start
        assert_approx_eq!(f64, result[2], 20.0); // interpolated
        assert_approx_eq!(f64, result[3], 30.0); // range end
        assert_approx_eq!(f64, result[4], 35.0); // above range

        Ok(())
    }

    #[test]
    fn test_invert_with_range_offset() -> Result<(), AvengerScaleError> {
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![10.0, 30.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 100.0])),
            options: vec![("range_offset".to_string(), 3.0.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let scale = LinearScale;
        let values = Arc::new(Float64Array::from(vec![-22.0, 3.0, 53.0, 103.0, 128.0])) as ArrayRef;
        let result = scale
            .invert_from_numeric(&config, &values)?
            .as_vec(values.len(), None);

        assert_approx_eq!(f64, result[0], 5.0); // below range
        assert_approx_eq!(f64, result[1], 10.0); // range start
        assert_approx_eq!(f64, result[2], 20.0); // interpolated
        assert_approx_eq!(f64, result[3], 30.0); // range end
        assert_approx_eq!(f64, result[4], 35.0); // above range

        Ok(())
    }

    #[test]
    fn test_invert_reversed_range() -> Result<(), AvengerScaleError> {
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![10.0, 30.0])),
            range: Arc::new(Float64Array::from(vec![100.0, 0.0])),
            options: vec![("clamp".to_string(), true.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let scale = LinearScale;
        let values = Arc::new(Float64Array::from(vec![125.0, 100.0, 50.0, 0.0, -25.0])) as ArrayRef;
        let result = scale
            .invert_from_numeric(&config, &values)?
            .as_vec(values.len(), None);

        assert_approx_eq!(f64, result[0], 10.0); // clamped
        assert_approx_eq!(f64, result[1], 10.0); // range start
        assert_approx_eq!(f64, result[2], 20.0); // interpolated
        assert_approx_eq!(f64, result[3], 30.0); // range end
        assert_approx_eq!(f64, result[4], 30.0); // clamped

        Ok(())
    }

    #[test]
    fn test_ticks() -> Result<(), AvengerScaleError> {
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![0.0, 10.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 100.0])),
            options: vec![].into_iter().collect(),
            context: ScaleContext::default(),
        };

        let scale = LinearScale;

        let expected = vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0];
        let ticks_array = scale.ticks(&config, Some(5.0))?;
        let ticks_array = ticks_array.as_primitive::<Float64Type>();
        assert_eq!(ticks_array.values().to_vec(), expected);

        let expected = vec![0.0, 5.0, 10.0];
        let ticks_array = scale.ticks(&config, Some(2.0))?;
        let ticks_array = ticks_array.as_primitive::<Float64Type>();
        assert_eq!(ticks_array.values().to_vec(), expected);

        let expected = vec![0.0, 10.0];
        let ticks_array = scale.ticks(&config, Some(1.0))?;
        let ticks_array = ticks_array.as_primitive::<Float64Type>();
        assert_eq!(ticks_array.values().to_vec(), expected);

        Ok(())
    }

    #[test]
    fn test_ticks_span_zero() -> Result<(), AvengerScaleError> {
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![-100.0, 100.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 100.0])),
            options: vec![].into_iter().collect(),
            context: ScaleContext::default(),
        };

        let scale = LinearScale;
        let ticks_array = scale.ticks(&config, Some(10.0))?;
        let ticks_array = ticks_array.as_primitive::<Float64Type>();
        assert_eq!(
            ticks_array.values().to_vec(),
            vec![-100.0, -80.0, -60.0, -40.0, -20.0, 0.0, 20.0, 40.0, 60.0, 80.0, 100.0]
        );

        let ticks_array = scale.ticks(&config, Some(5.0))?;
        let ticks_array = ticks_array.as_primitive::<Float64Type>();
        assert_eq!(
            ticks_array.values().to_vec(),
            vec![-100.0, -50.0, 0.0, 50.0, 100.0]
        );

        let ticks_array = scale.ticks(&config, Some(2.0))?;
        let ticks_array = ticks_array.as_primitive::<Float64Type>();
        assert_eq!(ticks_array.values().to_vec(), vec![-100.0, 0.0, 100.0]);

        let ticks_array = scale.ticks(&config, Some(1.0))?;
        let ticks_array = ticks_array.as_primitive::<Float64Type>();
        assert_eq!(ticks_array.values().to_vec(), vec![0.0]);

        Ok(())
    }

    #[test]
    fn test_configured_scale_invert_pow() -> Result<(), AvengerScaleError> {
        // Test the new invert() method with pow scale
        // Using exponent=2, domain [0,4], range [0,16]
        let scale = PowScale::configured((0.0, 4.0), (0.0, 16.0)).with_option("exponent", 2.0);

        // Test with Float32 values
        let values = Arc::new(Float64Array::from(vec![0.0, 4.0, 9.0, 16.0])) as ArrayRef;
        let result = scale.invert(&values)?;
        let result_array = result.as_primitive::<Float64Type>();

        assert_eq!(result.len(), 4);
        assert_approx_eq!(f64, result_array.value(0), 0.0); // 0^2 = 0 -> 0
        assert_approx_eq!(f64, result_array.value(1), 2.0); // 2^2 = 4 -> 2
        assert_approx_eq!(f64, result_array.value(2), 3.0); // 3^2 = 9 -> 3
        assert_approx_eq!(f64, result_array.value(3), 4.0); // 4^2 = 16 -> 4

        Ok(())
    }
}
