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
use lazy_static::lazy_static;

use crate::{array, color_interpolator::scale_numeric_to_color, error::AvengerScaleError};

use super::{
    linear::{LinearScale, NormalizationConfig},
    ConfiguredScale, InferDomainFromDataMethod, OptionConstraint, OptionDefinition, ScaleConfig,
    ScaleContext, ScaleImpl,
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
///
/// - **zero** (boolean, default: false): When true, ensures that the domain includes zero. If both min and max
///   are positive, sets min to zero. If both min and max are negative, sets max to zero. If the domain already
///   spans zero, no change is made. Zero extension is applied before nice calculations.
///
/// - **padding** (f32, default: 0.0): Expands the scale domain to accommodate the specified number of pixels
///   on each of the scale range. The scale range must represent pixels for this parameter to function as intended.
///   Padding is applied in pow space before all other adjustments, including zero and nice properties.
///
/// - **padding_lower** (f32, default: value of padding): Expands the scale domain at the lower end to accommodate
///   the specified number of pixels. Takes precedence over the general padding value for the lower end.
///
/// - **padding_upper** (f32, default: value of padding): Expands the scale domain at the upper end to accommodate
///   the specified number of pixels. Takes precedence over the general padding value for the upper end.
#[derive(Debug)]
pub struct PowScale;

/// Configuration for pow scale normalization operations
#[derive(Debug, Clone)]
pub struct PowNormalizationConfig<'a> {
    pub domain: (f32, f32),
    pub range: (f32, f32),
    pub padding: Option<&'a Scalar>,
    pub padding_lower: Option<&'a Scalar>,
    pub padding_upper: Option<&'a Scalar>,
    pub exponent: f32,
    pub zero: Option<&'a Scalar>,
    pub nice: Option<&'a Scalar>,
}

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
                    ("zero".to_string(), false.into()),
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

    /// Apply padding to the domain
    pub fn apply_padding(
        domain: (f32, f32),
        range: (f32, f32),
        padding_lower: f32,
        padding_upper: f32,
        exponent: f32,
    ) -> Result<(f32, f32), AvengerScaleError> {
        let (domain_start, domain_end) = domain;
        let (range_start, range_end) = range;

        // Early return for degenerate cases
        if domain_start == domain_end
            || range_start == range_end
            || (padding_lower <= 0.0 && padding_upper <= 0.0)
        {
            return Ok(domain);
        }

        // Transform to pow space using the power function
        let power_fun = PowerFunction::new(exponent);
        let pow_start = power_fun.pow(domain_start);
        let pow_end = power_fun.pow(domain_end);

        // Calculate how many pow-space units per pixel
        let range_span = (range_end - range_start).abs();
        let pow_span = pow_end - pow_start;
        let pow_per_pixel = pow_span / range_span;

        // Apply padding in pow space
        let new_pow_start = pow_start - padding_lower * pow_per_pixel;
        let new_pow_end = pow_end + padding_upper * pow_per_pixel;

        // Transform back to linear space
        let new_start = power_fun.pow_inv(new_pow_start);
        let new_end = power_fun.pow_inv(new_pow_end);

        Ok((new_start, new_end))
    }

    /// Apply normalization (padding, zero and nice) to domain
    pub fn apply_normalization(
        config: PowNormalizationConfig,
    ) -> Result<(f32, f32), AvengerScaleError> {
        // Apply padding first if specified
        let padding_value = config.padding.and_then(|p| p.as_f32().ok()).unwrap_or(0.0);
        let padding_lower_value = config
            .padding_lower
            .and_then(|p| p.as_f32().ok())
            .unwrap_or(padding_value);
        let padding_upper_value = config
            .padding_upper
            .and_then(|p| p.as_f32().ok())
            .unwrap_or(padding_value);

        let domain = if padding_lower_value > 0.0 || padding_upper_value > 0.0 {
            PowScale::apply_padding(
                config.domain,
                config.range,
                padding_lower_value,
                padding_upper_value,
                config.exponent,
            )?
        } else {
            config.domain
        };

        // Use LinearScale normalization for zero and nice since power transformation preserves zero
        let linear_config = NormalizationConfig {
            domain,
            range: config.range,
            padding: None,
            padding_lower: None,
            padding_upper: None,
            zero: config.zero,
            nice: config.nice,
        };
        let (normalized_start, normalized_end) = LinearScale::apply_normalization(linear_config)?;
        Ok((normalized_start, normalized_end))
    }
}

impl ScaleImpl for PowScale {
    fn scale_type(&self) -> &'static str {
        "pow"
    }

    fn infer_domain_from_data_method(&self) -> InferDomainFromDataMethod {
        InferDomainFromDataMethod::Interval
    }

    fn option_definitions(&self) -> &[OptionDefinition] {
        lazy_static! {
            static ref DEFINITIONS: Vec<OptionDefinition> = vec![
                OptionDefinition::optional("exponent", OptionConstraint::Float),
                OptionDefinition::optional("clamp", OptionConstraint::Boolean),
                OptionDefinition::optional("range_offset", OptionConstraint::Float),
                OptionDefinition::optional("round", OptionConstraint::Boolean),
                OptionDefinition::optional("nice", OptionConstraint::nice()),
                OptionDefinition::optional("zero", OptionConstraint::Boolean),
                OptionDefinition::optional("padding", OptionConstraint::NonNegativeFloat),
                OptionDefinition::optional("padding_lower", OptionConstraint::NonNegativeFloat),
                OptionDefinition::optional("padding_upper", OptionConstraint::NonNegativeFloat),
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

        // Get range for padding calculation, use dummy range if not numeric
        let range_for_padding = config.numeric_interval_range().unwrap_or((0.0, 1.0));

        let (domain_start, domain_end) = PowScale::apply_normalization(PowNormalizationConfig {
            domain: config.numeric_interval_domain()?,
            range: range_for_padding,
            padding: config.options.get("padding"),
            padding_lower: config.options.get("padding_lower"),
            padding_upper: config.options.get("padding_upper"),
            exponent,
            zero: config.options.get("zero"),
            nice: config.options.get("nice"),
        })?;

        // Check if color interpolation is needed
        if config.color_range().is_ok() {
            // Create new config with normalized domain
            let config = ScaleConfig {
                domain: Arc::new(Float32Array::from(vec![domain_start, domain_end])),
                ..config.clone()
            };
            return scale_numeric_to_color(self, &config, values);
        }

        let (range_start, range_end) = config.numeric_interval_range()?;

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

        // Get range for padding calculation, use dummy range if not numeric
        let range_for_padding = config.numeric_interval_range().unwrap_or((0.0, 1.0));
        let (range_start, range_end) = config.numeric_interval_range()?;

        let (domain_start, domain_end) = PowScale::apply_normalization(PowNormalizationConfig {
            domain: config.numeric_interval_domain()?,
            range: range_for_padding,
            padding: config.options.get("padding"),
            padding_lower: config.options.get("padding_lower"),
            padding_upper: config.options.get("padding_upper"),
            exponent,
            zero: config.options.get("zero"),
            nice: config.options.get("nice"),
        })?;

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

        // Get range for padding calculation, use dummy range if not numeric
        let range_for_padding = config.numeric_interval_range().unwrap_or((0.0, 1.0));

        let (domain_start, domain_end) = PowScale::apply_normalization(PowNormalizationConfig {
            domain: config.numeric_interval_domain()?,
            range: range_for_padding,
            padding: config.options.get("padding"),
            padding_lower: config.options.get("padding_lower"),
            padding_upper: config.options.get("padding_upper"),
            exponent,
            zero: config.options.get("zero"),
            nice: config.options.get("nice"),
        })?;
        let count = count.unwrap_or(10.0);
        let ticks_array = Float32Array::from(array::ticks(domain_start, domain_end, count));
        Ok(Arc::new(ticks_array) as ArrayRef)
    }

    fn compute_nice_domain(&self, config: &ScaleConfig) -> Result<ArrayRef, AvengerScaleError> {
        let exponent = config.option_f32("exponent", 1.0);

        // Get range for padding calculation, use dummy range if not numeric
        let range_for_padding = config.numeric_interval_range().unwrap_or((0.0, 1.0));

        let (domain_start, domain_end) = PowScale::apply_normalization(PowNormalizationConfig {
            domain: config.numeric_interval_domain()?,
            range: range_for_padding,
            padding: config.options.get("padding"),
            padding_lower: config.options.get("padding_lower"),
            padding_upper: config.options.get("padding_upper"),
            exponent,
            zero: config.options.get("zero"),
            nice: config.options.get("nice"),
        })?;

        Ok(Arc::new(Float32Array::from(vec![domain_start, domain_end])) as ArrayRef)
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

    #[test]
    fn test_apply_padding_symmetric() -> Result<(), AvengerScaleError> {
        // Test symmetric padding
        let result = PowScale::apply_padding((2.0, 4.0), (0.0, 500.0), 10.0, 10.0, 2.0)?;

        // In pow space: [4, 16] with range 500
        // pow units per pixel = (16-4)/500 = 0.024
        // padding = 10 pixels = 0.24 pow units on each side
        // new pow domain = [3.76, 16.24]
        // Back to linear: [1.939, 4.030]
        assert!(result.0 < 2.0);
        assert!(result.1 > 4.0);
        assert_approx_eq!(f32, result.0, 1.9391448, epsilon = 0.01);
        assert_approx_eq!(f32, result.1, 4.029875, epsilon = 0.01);

        // Test with no padding
        let result = PowScale::apply_padding((2.0, 4.0), (0.0, 500.0), 0.0, 0.0, 2.0)?;
        assert_eq!(result, (2.0, 4.0));

        // Test with degenerate domain
        let result = PowScale::apply_padding((5.0, 5.0), (0.0, 500.0), 10.0, 10.0, 2.0)?;
        assert_eq!(result, (5.0, 5.0));

        // Test with degenerate range
        let result = PowScale::apply_padding((2.0, 4.0), (100.0, 100.0), 10.0, 10.0, 2.0)?;
        assert_eq!(result, (2.0, 4.0));

        Ok(())
    }

    #[test]
    fn test_apply_padding_asymmetric() -> Result<(), AvengerScaleError> {
        // Test asymmetric padding
        let result = PowScale::apply_padding((2.0, 4.0), (0.0, 500.0), 20.0, 10.0, 2.0)?;

        // In pow space: [4, 16] with range 500
        // pow units per pixel = (16-4)/500 = 0.024
        // padding_lower = 20 pixels = 0.48 pow units
        // padding_upper = 10 pixels = 0.24 pow units
        // new pow domain = [3.52, 16.24]
        // Back to linear: [1.876, 4.030]
        assert_approx_eq!(f32, result.0, 1.8761664, epsilon = 0.01);
        assert_approx_eq!(f32, result.1, 4.029875, epsilon = 0.01);

        // Test with only lower padding
        let result = PowScale::apply_padding((2.0, 4.0), (0.0, 500.0), 25.0, 0.0, 2.0)?;

        // padding_lower = 25 pixels = 0.6 pow units
        // new pow domain = [3.4, 16.0]
        assert_approx_eq!(f32, result.0, 1.8439089, epsilon = 0.01);
        assert_eq!(result.1, 4.0);

        // Test with only upper padding
        let result = PowScale::apply_padding((2.0, 4.0), (0.0, 500.0), 0.0, 25.0, 2.0)?;

        // padding_upper = 25 pixels = 0.6 pow units
        // new pow domain = [4.0, 16.6]
        assert_eq!(result.0, 2.0);
        assert_approx_eq!(f32, result.1, 4.0743666, epsilon = 0.01);

        Ok(())
    }

    #[test]
    fn test_pow_scale_with_padding() -> Result<(), AvengerScaleError> {
        let scale = PowScale::configured((2.0, 10.0), (0.0, 100.0))
            .with_option("padding", 5.0)
            .with_option("exponent", 2.0);

        // The padding should expand the domain
        let values = Arc::new(Float32Array::from(vec![2.0, 10.0])) as ArrayRef;
        let result = scale.scale(&values)?;
        let result_array = result.as_primitive::<Float32Type>();

        // With padding, the domain endpoints should map to values inside the range
        assert!(result_array.value(0) > 0.0);
        assert!(result_array.value(1) < 100.0);

        Ok(())
    }

    #[test]
    fn test_padding_with_nice() -> Result<(), AvengerScaleError> {
        // Test padding applied before nice
        let domain = (2.0, 10.0);
        let range = (0.0, 100.0);
        let padding_value = Scalar::from(5.0);
        let padding = Some(&padding_value);
        let exponent = 2.0;
        let nice_value: Scalar = true.into();
        let nice = Some(&nice_value);

        let result = PowScale::apply_normalization(PowNormalizationConfig {
            domain,
            range,
            padding,
            padding_lower: None,
            padding_upper: None,
            exponent,
            zero: None,
            nice,
        })?;

        // Nice should round to nice values after padding is applied
        // The exact values depend on the nice algorithm
        assert!(result.0 <= 2.0);
        assert!(result.1 >= 10.0);

        Ok(())
    }

    #[test]
    fn test_pow_scale_padding_with_color_range() -> Result<(), AvengerScaleError> {
        // Test that padding with non-numeric range doesn't crash
        // We test this by verifying that apply_normalization works with a dummy range
        let domain = (2.0, 8.0);
        let range = (0.0, 500.0); // larger range for padding calculation
        let padding_value = Scalar::from(10.0);
        let padding = Some(&padding_value);
        let exponent = 2.0;

        let result = PowScale::apply_normalization(PowNormalizationConfig {
            domain,
            range,
            padding,
            padding_lower: None,
            padding_upper: None,
            exponent,
            zero: None,
            nice: None,
        })?;

        // Should have expanded the domain
        assert!(result.0 < domain.0);
        assert!(result.1 > domain.1);

        Ok(())
    }

    #[test]
    fn test_pow_scale_with_asymmetric_padding() -> Result<(), AvengerScaleError> {
        // Test with padding_lower and padding_upper
        let scale = PowScale::configured((2.0, 8.0), (0.0, 100.0))
            .with_option("padding_lower", 20.0)
            .with_option("padding_upper", 10.0)
            .with_option("exponent", 2.0);

        let normalized = scale.normalize()?;
        let domain = normalized.numeric_interval_domain()?;

        // With exponent=2, domain [2,8] becomes [4,64] in pow space
        // pow units per pixel = (64-4)/100 = 0.6
        // padding_lower = 20 pixels = 12 pow units, so pow_start = 4-12 = -8
        // But pow_inv(-8) with sign preservation = -sqrt(8) = -2.828
        // padding_upper = 10 pixels = 6 pow units, so pow_end = 64+6 = 70
        // pow_inv(70) = sqrt(70) = 8.366
        assert!(domain.0 < 0.0); // Due to sign preservation in pow space
        assert_approx_eq!(f32, domain.0, -2.828427, epsilon = 0.01);
        assert_approx_eq!(f32, domain.1, 8.366601, epsilon = 0.01);

        Ok(())
    }

    #[test]
    fn test_pow_padding_fallback_behavior() -> Result<(), AvengerScaleError> {
        // Test that padding_lower/upper fall back to padding when not specified
        let scale1 = PowScale::configured((1.0, 9.0), (0.0, 100.0))
            .with_option("padding", 15.0)
            .with_option("padding_lower", 20.0) // Only lower specified
            .with_option("exponent", 2.0);

        let normalized1 = scale1.normalize()?;
        let domain1 = normalized1.numeric_interval_domain()?;

        // padding_lower = 20, padding_upper falls back to padding = 15
        // In pow space: [1, 81], pow units per pixel = 80/100 = 0.8
        // padding_lower = 20 pixels = 16 pow units, new start = 1-16 = -15
        // pow_inv(-15) with sign = -sqrt(15) = -3.873
        // padding_upper = 15 pixels = 12 pow units, new end = 81+12 = 93
        // pow_inv(93) = sqrt(93) = 9.644
        assert!(domain1.0 < 0.0);
        assert_approx_eq!(f32, domain1.0, -3.8729835, epsilon = 0.01);
        assert_approx_eq!(f32, domain1.1, 9.643651, epsilon = 0.01);

        Ok(())
    }
}
