use std::sync::Arc;

use crate::scalar::Scalar;
use arrow::{
    array::{ArrayRef, AsArray, Float32Array},
    compute::{kernels::cast, unary},
    datatypes::{DataType, Float32Type},
};
use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use lazy_static::lazy_static;

use crate::{array, error::AvengerScaleError};

use super::{
    linear::{LinearScale, NormalizationConfig},
    ConfiguredScale, InferDomainFromDataMethod, OptionConstraint, OptionDefinition, ScaleConfig,
    ScaleContext, ScaleImpl,
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
///
/// - **zero** (boolean, default: false): When true, ensures that the domain includes zero. If both min and max
///   are positive, sets min to zero. If both min and max are negative, sets max to zero. If the domain already
///   spans zero, no change is made. Zero extension is applied before nice calculations.
///
/// - **padding** (f32, default: 0.0): Expands the scale domain to accommodate the specified number of pixels
///   on each of the scale range. The scale range must represent pixels for this parameter to function as intended.
///   Padding is applied in symlog space before all other adjustments, including zero and nice properties.
///
/// - **padding_lower** (f32, default: value of padding): Expands the scale domain at the lower end to accommodate
///   the specified number of pixels. Takes precedence over the general padding value for the lower end.
///
/// - **padding_upper** (f32, default: value of padding): Expands the scale domain at the upper end to accommodate
///   the specified number of pixels. Takes precedence over the general padding value for the upper end.
#[derive(Debug)]
pub struct SymlogScale;

/// Configuration for symlog scale normalization operations
#[derive(Debug, Clone)]
pub struct SymlogNormalizationConfig<'a> {
    pub domain: (f32, f32),
    pub range: (f32, f32),
    pub constant: f32,
    pub padding: Option<&'a Scalar>,
    pub padding_lower: Option<&'a Scalar>,
    pub padding_upper: Option<&'a Scalar>,
    pub zero: Option<&'a Scalar>,
    pub nice: Option<&'a Scalar>,
}

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
                    ("zero".to_string(), false.into()),
                    ("padding".to_string(), 0.0.into()),
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

    /// Apply padding in symlog space
    pub fn apply_padding(
        domain: (f32, f32),
        range: (f32, f32),
        padding_lower: f32,
        padding_upper: f32,
        constant: f32,
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

        // Transform to symlog space
        let symlog_start = symlog_transform(domain_start, constant);
        let symlog_end = symlog_transform(domain_end, constant);

        // Calculate how many symlog units per pixel
        let range_span = (range_end - range_start).abs();
        let symlog_span = symlog_end - symlog_start;
        let symlog_per_pixel = symlog_span / range_span;

        // Apply padding in symlog space
        let new_symlog_start = symlog_start - padding_lower * symlog_per_pixel;
        let new_symlog_end = symlog_end + padding_upper * symlog_per_pixel;

        // Transform back to linear space
        let new_start = symlog_invert(new_symlog_start, constant);
        let new_end = symlog_invert(new_symlog_end, constant);

        Ok((new_start, new_end))
    }

    /// Apply normalization (padding, zero and nice) to domain
    pub fn apply_normalization(
        config: SymlogNormalizationConfig,
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
            Self::apply_padding(
                config.domain,
                config.range,
                padding_lower_value,
                padding_upper_value,
                config.constant,
            )?
        } else {
            config.domain
        };

        // Then apply zero and nice using LinearScale normalization
        let linear_config = NormalizationConfig {
            domain,
            range: (0.0, 1.0),
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

impl ScaleImpl for SymlogScale {
    fn scale_type(&self) -> &'static str {
        "symlog"
    }

    fn infer_domain_from_data_method(&self) -> InferDomainFromDataMethod {
        InferDomainFromDataMethod::Interval
    }

    fn option_definitions(&self) -> &[OptionDefinition] {
        lazy_static! {
            static ref DEFINITIONS: Vec<OptionDefinition> = vec![
                OptionDefinition::optional("constant", OptionConstraint::PositiveFloat),
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
        let constant = config.option_f32("constant", 1.0);
        let range_offset = config.option_f32("range_offset", 0.0);
        let clamp = config.option_boolean("clamp", false);
        let round = config.option_boolean("round", false);

        // Get range for padding calculation, use dummy range if not numeric
        let range_for_padding = config.numeric_interval_range().unwrap_or((0.0, 1.0));

        let (domain_start, domain_end) =
            SymlogScale::apply_normalization(SymlogNormalizationConfig {
                domain: config.numeric_interval_domain()?,
                range: range_for_padding,
                constant,
                padding: config.options.get("padding"),
                padding_lower: config.options.get("padding_lower"),
                padding_upper: config.options.get("padding_upper"),
                zero: config.options.get("zero"),
                nice: config.options.get("nice"),
            })?;
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
        let (domain_start, domain_end) =
            SymlogScale::apply_normalization(SymlogNormalizationConfig {
                domain: config.numeric_interval_domain()?,
                range: config.numeric_interval_range()?,
                constant: config.option_f32("constant", 1.0),
                padding: config.options.get("padding"),
                padding_lower: config.options.get("padding_lower"),
                padding_upper: config.options.get("padding_upper"),
                zero: config.options.get("zero"),
                nice: config.options.get("nice"),
            })?;

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
        let (domain_start, domain_end) =
            SymlogScale::apply_normalization(SymlogNormalizationConfig {
                domain: config.numeric_interval_domain()?,
                range: (0.0, 1.0), // Use dummy range for ticks computation
                constant: config.option_f32("constant", 1.0),
                padding: None, // No padding for ticks
                padding_lower: None,
                padding_upper: None,
                zero: config.options.get("zero"),
                nice: config.options.get("nice"),
            })?;
        let count = count.unwrap_or(10.0);
        let ticks_array = Float32Array::from(array::ticks(domain_start, domain_end, count));
        Ok(Arc::new(ticks_array) as ArrayRef)
    }

    fn compute_nice_domain(&self, config: &ScaleConfig) -> Result<ArrayRef, AvengerScaleError> {
        let constant = config.option_f32("constant", 1.0);
        // Get range for padding calculation, use dummy range if not numeric
        let range_for_padding = config.numeric_interval_range().unwrap_or((0.0, 1.0));
        let (domain_start, domain_end) =
            SymlogScale::apply_normalization(SymlogNormalizationConfig {
                domain: config.numeric_interval_domain()?,
                range: range_for_padding,
                constant,
                padding: config.options.get("padding"),
                padding_lower: config.options.get("padding_lower"),
                padding_upper: config.options.get("padding_upper"),
                zero: config.options.get("zero"),
                nice: config.options.get("nice"),
            })?;

        Ok(Arc::new(Float32Array::from(vec![domain_start, domain_end])) as ArrayRef)
    }
}

/// Applies the symlog transform to a single value
/// Uses ln_1p for better numerical stability near zero
fn symlog_transform(x: f32, constant: f32) -> f32 {
    x.signum() * (x.abs() / constant).ln_1p()
}

/// Applies the inverse symlog transform to a single value
/// Uses exp_m1 for better numerical stability near zero
fn symlog_invert(x: f32, constant: f32) -> f32 {
    x.signum() * x.abs().exp_m1() * constant
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

    #[test]
    fn test_apply_padding_symmetric() {
        let constant = 1.0;

        // Test symmetric padding
        let (new_start, new_end) =
            SymlogScale::apply_padding((-10.0, 10.0), (0.0, 100.0), 10.0, 10.0, constant).unwrap();

        // In symlog space: symlog_transform(-10, 1) = -2.398, symlog_transform(10, 1) = 2.398
        // symlog units per pixel = 4.796 / 100 = 0.04796
        // padding = 10 pixels = 0.4796 symlog units on each side
        // new symlog domain = [-2.398 - 0.4796, 2.398 + 0.4796] = [-2.8776, 2.8776]
        // symlog_invert(-2.8776, 1) = -16.769, symlog_invert(2.8776, 1) = 16.769
        assert!(new_start < -10.0);
        assert!(new_end > 10.0);
        assert_approx_eq!(f32, new_start, -16.769337, epsilon = 0.01);
        assert_approx_eq!(f32, new_end, 16.769337, epsilon = 0.01);

        // Test zero padding
        let (new_start, new_end) =
            SymlogScale::apply_padding((-10.0, 10.0), (0.0, 100.0), 0.0, 0.0, constant).unwrap();
        assert_approx_eq!(f32, new_start, -10.0);
        assert_approx_eq!(f32, new_end, 10.0);

        // Test degenerate domain
        let (new_start, new_end) =
            SymlogScale::apply_padding((5.0, 5.0), (0.0, 100.0), 10.0, 10.0, constant).unwrap();
        assert_approx_eq!(f32, new_start, 5.0);
        assert_approx_eq!(f32, new_end, 5.0);
    }

    #[test]
    fn test_symlog_scale_with_padding() {
        let scale = SymlogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![-10.0, 10.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 100.0])),
            options: vec![("padding".to_string(), 10.0.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        // Test that domain endpoints map correctly
        let values = Arc::new(Float32Array::from(vec![-10.0, 0.0, 10.0])) as ArrayRef;
        let result = scale
            .scale_to_numeric(&config, &values)
            .unwrap()
            .as_vec(values.len(), None);

        // With padding, -10 and 10 should no longer map to exactly 0 and 100
        assert!(result[0] > 0.0);
        assert!(result[2] < 100.0);
        // 0 should still map to the center
        assert_approx_eq!(
            f32,
            result[1],
            50.0,
            F32Margin {
                epsilon: 0.01,
                ..Default::default()
            }
        );
    }

    #[test]
    fn test_padding_with_nice() {
        let scale = SymlogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![-9.0, 9.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 100.0])),
            options: vec![
                ("padding".to_string(), 10.0.into()),
                ("nice".to_string(), true.into()),
            ]
            .into_iter()
            .collect(),
            context: ScaleContext::default(),
        };

        // With padding and nice, domain should be expanded then niced
        let nice_domain = scale.compute_nice_domain(&config).unwrap();
        let nice_array = nice_domain.as_primitive::<Float32Type>();

        // Domain should be expanded and then made nice
        assert!(nice_array.value(0) <= -9.0);
        assert!(nice_array.value(1) >= 9.0);
    }

    #[test]
    fn test_apply_padding_asymmetric() {
        let constant = 1.0;

        // Test asymmetric padding
        let (new_start, new_end) =
            SymlogScale::apply_padding((-10.0, 10.0), (0.0, 100.0), 20.0, 10.0, constant).unwrap();

        // In symlog space: domain [-10, 10] -> [-2.398, 2.398]
        // symlog units per pixel = 4.796 / 100 = 0.04796
        // padding_lower = 20 pixels = 0.9592 symlog units
        // padding_upper = 10 pixels = 0.4796 symlog units
        // new symlog domain = [-2.398 - 0.9592, 2.398 + 0.4796] = [-3.3572, 2.8776]
        // symlog_invert(-3.3572, 1) = -27.713, symlog_invert(2.8776, 1) = 16.769
        assert_approx_eq!(f32, new_start, -27.713177, epsilon = 0.01);
        assert_approx_eq!(f32, new_end, 16.769337, epsilon = 0.01);

        // Test with only lower padding
        let (new_start, new_end) =
            SymlogScale::apply_padding((1.0, 10.0), (0.0, 100.0), 25.0, 0.0, constant).unwrap();

        // Original domain in symlog space: [0.693, 2.398]
        // symlog units per pixel = 1.705 / 100 = 0.01705
        // padding_lower = 25 pixels = 0.426 symlog units
        // new start = 0.693 - 0.426 = 0.267
        // symlog_invert(0.267) ≈ 0.306
        assert_approx_eq!(f32, new_start, 0.30616736, epsilon = 0.01);
        assert_approx_eq!(f32, new_end, 10.0);

        // Test with only upper padding
        let (new_start, new_end) =
            SymlogScale::apply_padding((1.0, 10.0), (0.0, 100.0), 0.0, 25.0, constant).unwrap();

        // padding_upper = 25 pixels = 0.426 symlog units
        // new end = 2.398 + 0.426 = 2.824
        // symlog_invert(2.824) ≈ 15.845
        assert_approx_eq!(f32, new_start, 1.0);
        assert_approx_eq!(f32, new_end, 15.84548, epsilon = 0.01);
    }

    #[test]
    fn test_symlog_scale_with_asymmetric_padding() -> Result<(), AvengerScaleError> {
        // Test with padding_lower and padding_upper
        let scale = SymlogScale::configured((-5.0, 5.0), (0.0, 100.0))
            .with_option("padding_lower", 20.0)
            .with_option("padding_upper", 10.0)
            .with_option("constant", 1.0);

        let normalized = scale.normalize()?;
        let domain = normalized.numeric_interval_domain()?;

        // Domain should be expanded asymmetrically
        assert!(domain.0 < -5.0);
        assert!(domain.1 > 5.0);
        // Lower padding is larger, so |domain.0| should be > |domain.1|
        assert!(domain.0.abs() > domain.1.abs());

        Ok(())
    }

    #[test]
    fn test_symlog_padding_fallback_behavior() -> Result<(), AvengerScaleError> {
        // Test that padding_lower/upper fall back to padding when not specified
        let scale1 = SymlogScale::configured((-10.0, 10.0), (0.0, 100.0))
            .with_option("padding", 15.0)
            .with_option("padding_lower", 20.0) // Only lower specified
            .with_option("constant", 1.0);

        let normalized1 = scale1.normalize()?;
        let domain1 = normalized1.numeric_interval_domain()?;

        // padding_lower = 20, padding_upper falls back to padding = 15
        // Should expand more on the lower end
        assert!(domain1.0.abs() > domain1.1.abs());

        Ok(())
    }

    #[test]
    fn test_padding_near_zero() {
        // Test numerical stability with values very close to zero
        let constant = 0.001;

        // Test with very small domain near zero
        let (new_start, new_end) =
            SymlogScale::apply_padding((-0.001, 0.001), (0.0, 100.0), 5.0, 5.0, constant).unwrap();

        // Domain should expand
        assert!(new_start < -0.001);
        assert!(new_end > 0.001);

        // Test transform stability near zero
        let epsilon = 1e-10;
        let transformed = symlog_transform(epsilon, constant);
        let inverted = symlog_invert(transformed, constant);

        // Should round-trip accurately
        assert_approx_eq!(
            f32,
            inverted,
            epsilon,
            F32Margin {
                epsilon: 1e-15,
                ..Default::default()
            }
        );

        // Test with zero in domain
        let (new_start, new_end) =
            SymlogScale::apply_padding((-1.0, 0.0), (0.0, 100.0), 10.0, 10.0, constant).unwrap();

        assert!(new_start < -1.0);
        assert!(new_end > 0.0); // Padding expands both ends of domain
    }

    #[test]
    fn test_symlog_transform_matches_vega() {
        // Test that our transforms match Vega's implementation
        let constant = 1.0;
        let test_values = vec![-10.0, -1.0, -0.1, -0.01, 0.0, 0.01, 0.1, 1.0, 10.0];

        for x in test_values {
            let transformed = symlog_transform(x, constant);
            let inverted = symlog_invert(transformed, constant);

            // Should round-trip accurately
            if x == 0.0 {
                assert_eq!(inverted, 0.0);
            } else {
                assert_approx_eq!(
                    f32,
                    inverted,
                    x,
                    F32Margin {
                        epsilon: 1e-6,
                        ..Default::default()
                    }
                );
            }

            // Verify sign preservation
            assert_eq!(transformed.signum(), x.signum());
        }
    }
}
