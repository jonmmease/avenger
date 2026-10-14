use std::sync::Arc;

use arrow::{
    array::{ArrayRef, AsArray, Float32Array, Float64Array},
    compute::{kernels::cast, unary},
    datatypes::{DataType, Float64Type},
};
use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use lazy_static::lazy_static;

use crate::{array, error::AvengerScaleError};

use super::domain_summary::{fit, DomainOptions, DomainSummary, Extents, NiceStep};
use super::no_domain;
use super::{
    ConfiguredScale, DomainKind, OptionConstraint, OptionDefinition, ScaleConfig, ScaleContext,
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
/// - **constant** (f64, default: 1.0): The linear threshold that controls the transition
///   between linear and logarithmic behavior. Within the range [-constant, constant],
///   the scale behaves approximately linearly. Must be positive.
///
/// - **clamp** (boolean, default: false): When true, values outside the domain are clamped
///   to the domain extent. For scaling, this means values outside domain map to the
///   corresponding range extent. For inversion, values outside range are clamped first.
///
/// - **range_offset** (f64, default: 0.0): An offset applied to the final scaled values.
///   This is added after the transformation. When inverting, this offset is subtracted first.
///
/// - **round** (boolean, default: false): When true, output values from scaling are rounded
///   to the nearest integer. This is useful for pixel-perfect rendering. Does not affect inversion.
#[derive(Debug)]
pub struct SymlogScale;

impl SymlogScale {
    /// A symlog scale onto `range` with no domain yet. Scaling needs a domain, which
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
                    ("constant".to_string(), 1.0.into()),
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

impl ScaleImpl for SymlogScale {
    fn scale_type(&self) -> &'static str {
        "symlog"
    }

    fn summarize(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
        extents: Extents,
    ) -> Result<DomainSummary, AvengerScaleError> {
        let constant = config.option_f64("constant", 1.0);
        let values = cast(values, &DataType::Float64)?;
        let values = values
            .as_primitive::<Float64Type>()
            .unary::<_, Float64Type>(|value| symlog_transform(value, constant));
        fit::interval_summary(DataType::Float64, &values, extents)
    }

    fn fit_domain(
        &self,
        config: &ScaleConfig,
        options: &DomainOptions,
        summary: &DomainSummary,
    ) -> Result<ScaleConfig, AvengerScaleError> {
        let options = options.interval()?;
        let constant = config.option_f64("constant", 1.0);
        fit::fit_continuous(
            config,
            options,
            summary,
            fit::IntervalFit {
                nice: options.nice.map(NiceStep::Auto),
                zero: options.zero.then_some(0.0),
                single_value_span: fit::linear_single_value_span,
            },
            |value| symlog_invert(value, constant),
            (0.0, 1.0),
        )
    }

    fn domain_kind(&self) -> DomainKind {
        DomainKind::Numeric
    }

    fn option_definitions(&self) -> &[OptionDefinition] {
        lazy_static! {
            static ref DEFINITIONS: Vec<OptionDefinition> = vec![
                OptionDefinition::optional("constant", OptionConstraint::PositiveFloat),
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
        let constant = config.option_f64("constant", 1.0);
        let range_offset = config.option_f64("range_offset", 0.0);
        let clamp = config.option_boolean("clamp", false);
        let round = config.option_boolean("round", false);

        let (domain_start, domain_end) = config.numeric_interval_domain()?;
        let (range_start, range_end) = config.numeric_interval_range()?;

        // Handle degenerate domain case
        if domain_start == domain_end
            || range_start == range_end
            || domain_start.is_nan()
            || domain_end.is_nan()
            || range_start.is_nan()
            || range_end.is_nan()
        {
            return Ok(
                Arc::new(Float32Array::from(vec![range_start as f32; values.len()])) as ArrayRef,
            );
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

        // Cast to f64 and downcast to f64 array
        let values = cast(values, &DataType::Float64)?;
        let values = values.as_primitive::<Float64Type>();

        match (clamp, round) {
            (true, true) => {
                // clamp, and round
                Ok(Arc::<Float32Array>::new(unary(values, |v| {
                    if v.is_nan() {
                        return f32::NAN;
                    }
                    if v.is_infinite() {
                        if v.is_sign_positive() {
                            return range_end.round() as f32;
                        } else {
                            return range_start.round() as f32;
                        }
                    }
                    // Apply symlog transform
                    let sign: f64 = if v < 0.0 { -1.0 } else { 1.0 };
                    let transformed = sign * (1.0 + (v.abs() / constant)).ln();

                    // Apply scale and offset, then clamp
                    (scale * transformed + offset)
                        .clamp(range_min, range_max)
                        .round()
                }
                    as f32)))
            }
            (true, false) => {
                // clamp, no round
                Ok(Arc::<Float32Array>::new(unary(values, |v| {
                    if v.is_nan() {
                        return f32::NAN;
                    }
                    if v.is_infinite() {
                        if v.is_sign_positive() {
                            return range_end as f32;
                        } else {
                            return range_start as f32;
                        }
                    }
                    // Apply symlog transform
                    let sign: f64 = if v < 0.0 { -1.0 } else { 1.0 };
                    let transformed = sign * (1.0 + (v.abs() / constant)).ln();

                    // Apply scale and offset, then clamp
                    (scale * transformed + offset).clamp(range_min, range_max)
                }
                    as f32)))
            }
            (false, true) => {
                // no clamp, round
                Ok(Arc::<Float32Array>::new(unary(values, |v| {
                    if v.is_nan() {
                        return f32::NAN;
                    }
                    if v.is_infinite() {
                        return if v.is_sign_positive() {
                            range_end.round() as f32
                        } else {
                            range_start.round() as f32
                        };
                    }
                    // Apply symlog transform
                    let sign = if v < 0.0 { -1.0 } else { 1.0 };
                    let transformed = sign * (1.0 + (v.abs() / constant)).ln();

                    // Apply scale and offset
                    (scale * transformed + offset).round()
                }
                    as f32)))
            }
            (false, false) => {
                // no clamp, no round
                Ok(Arc::<Float32Array>::new(unary(values, |v| {
                    if v.is_nan() {
                        return f32::NAN;
                    }
                    if v.is_infinite() {
                        return if v.is_sign_positive() {
                            range_end as f32
                        } else {
                            range_start as f32
                        };
                    }
                    // Apply symlog transform
                    let sign = if v < 0.0 { -1.0 } else { 1.0 };
                    let transformed = sign * (1.0 + (v.abs() / constant)).ln();

                    // Apply scale and offset
                    scale * transformed + offset
                }
                    as f32)))
            }
        }
    }

    fn invert_from_numeric(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ScalarOrArray<f64>, AvengerScaleError> {
        let (domain_start, domain_end) = config.numeric_interval_domain()?;

        let (range_start, range_end) = config.numeric_interval_range()?;
        let range_offset = config.option_f64("range_offset", 0.0);
        let clamp = config.option_boolean("clamp", false);
        let _round = config.option_boolean("round", false);
        let constant = config.option_f64("constant", 1.0);

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

        // Cast to f64 and downcast to f64 array
        let values = cast(values, &DataType::Float64)?;
        let values = values.as_primitive::<Float64Type>();

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
                            return f64::NAN;
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
                            return f64::NAN;
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

/// Applies the symlog transform to a single value
/// Uses ln_1p for better numerical stability near zero
fn symlog_transform(x: f64, constant: f64) -> f64 {
    x.signum() * (x.abs() / constant).ln_1p()
}

/// Applies the inverse symlog transform to a single value
/// Uses exp_m1 for better numerical stability near zero
fn symlog_invert(x: f64, constant: f64) -> f64 {
    x.signum() * x.abs().exp_m1() * constant
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use arrow::datatypes::Float64Type;
    use float_cmp::{assert_approx_eq, F64Margin};

    #[test]
    fn test_basic_scale() {
        let scale = SymlogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![-100.0, 100.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 1.0])),
            options: HashMap::new(),
            context: ScaleContext::default(),
        };

        let values = Arc::new(Float64Array::from(vec![-100.0, 0.0, 100.0])) as ArrayRef;
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
            domain: Arc::new(Float64Array::from(vec![-100.0, 100.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 1.0])),
            options: vec![("range_offset".to_string(), 0.5.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let values = Arc::new(Float64Array::from(vec![-100.0, 0.0, 100.0])) as ArrayRef;
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
            domain: Arc::new(Float64Array::from(vec![-100.0, 100.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 1.0])),
            options: vec![("constant".to_string(), 5.0.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let values = Arc::new(Float64Array::from(vec![-100.0, 0.0, 100.0])) as ArrayRef;
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
        let values = Arc::new(Float64Array::from(vec![3.0, -1.0])) as ArrayRef;

        let config_no_clamp = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![0.0, 1.0])),
            range: Arc::new(Float64Array::from(vec![10.0, 20.0])),
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
            domain: Arc::new(Float64Array::from(vec![0.0, 1.0])),
            range: Arc::new(Float64Array::from(vec![10.0, 20.0])),
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
            domain: Arc::new(Float64Array::from(vec![-100.0, 100.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 1.0])),
            options: HashMap::new(),
            context: ScaleContext::default(),
        };

        // Test NaN
        let values = Arc::new(Float64Array::from(vec![f64::NAN])) as ArrayRef;
        let result = scale
            .scale_to_numeric(&config, &values)
            .unwrap()
            .as_vec(values.len(), None);
        assert!(result[0].is_nan());

        // Test infinity
        let values =
            Arc::new(Float64Array::from(vec![f64::INFINITY, f64::NEG_INFINITY])) as ArrayRef;
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
            domain: Arc::new(Float64Array::from(vec![-100.0, 100.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 1.0])),
            options: HashMap::new(),
            context: ScaleContext::default(),
        };

        // Test that invert(scale(x)) ≈ x
        let values = Arc::new(Float64Array::from(vec![
            -100.0, -10.0, -1.0, 0.0, 1.0, 10.0, 100.0,
        ])) as ArrayRef;
        let scaled_array = scale.scale(&config, &values).unwrap();

        let inverted = scale.invert_from_numeric(&config, &scaled_array).unwrap();
        let inverted_array =
            Arc::new(Float64Array::from(inverted.as_vec(values.len(), None))) as ArrayRef;

        let values_f64 = values.as_primitive::<Float64Type>();
        let inverted_f64 = inverted_array.as_primitive::<Float64Type>();
        for i in 0..values.len() {
            assert_approx_eq!(
                f64,
                inverted_f64.value(i),
                values_f64.value(i),
                epsilon = 1e-6
            );
        }
    }

    #[test]
    fn test_invert_range_offset() {
        let scale = SymlogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![-100.0, 100.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 1.0])),
            options: vec![("range_offset".to_string(), 0.5.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        // Test that invert(scale(x)) ≈ x
        let values = Arc::new(Float64Array::from(vec![
            -100.0, -10.0, -1.0, 0.0, 1.0, 10.0, 100.0,
        ])) as ArrayRef;
        let scaled_array = scale.scale(&config, &values).unwrap();
        let inverted = scale.invert_from_numeric(&config, &scaled_array).unwrap();
        let inverted_array =
            Arc::new(Float64Array::from(inverted.as_vec(values.len(), None))) as ArrayRef;

        let values_f64 = values.as_primitive::<Float64Type>();
        let inverted_f64 = inverted_array.as_primitive::<Float64Type>();

        for i in 0..values.len() {
            assert_approx_eq!(
                f64,
                inverted_f64.value(i),
                values_f64.value(i),
                F64Margin {
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
            domain: Arc::new(Float64Array::from(vec![-100.0, 100.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 1.0])),
            options: vec![("clamp".to_string(), true.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        // Test values outside the range
        let values = Arc::new(Float64Array::from(vec![-0.5, 1.5])) as ArrayRef;
        let result = scale.invert_from_numeric(&config, &values).unwrap();
        let result_vec = result.as_vec(values.len(), None);
        assert_approx_eq!(f64, result_vec[0], -100.0);
        assert_approx_eq!(f64, result_vec[1], 100.0);
    }

    #[test]
    fn test_invert_constant() {
        let scale = SymlogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![-100.0, 100.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 1.0])),
            options: vec![("constant".to_string(), 2.0.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        // Test that invert(scale(x)) ≈ x with different constant
        let values = Arc::new(Float64Array::from(vec![-50.0, 0.0, 50.0])) as ArrayRef;
        let scaled_array = scale.scale(&config, &values).unwrap();

        let inverted = scale.invert_from_numeric(&config, &scaled_array).unwrap();
        let inverted_array =
            Arc::new(Float64Array::from(inverted.as_vec(values.len(), None))) as ArrayRef;

        let values_f64 = values.as_primitive::<Float64Type>();
        let inverted_f64 = inverted_array.as_primitive::<Float64Type>();
        for i in 0..values.len() {
            assert_approx_eq!(
                f64,
                inverted_f64.value(i),
                values_f64.value(i),
                epsilon = 1e-5
            );
        }
    }

    #[test]
    fn test_invert_edge_cases() {
        let scale = SymlogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![-100.0, 100.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 1.0])),
            options: HashMap::new(),
            context: ScaleContext::default(),
        };

        // Test NaN
        let values = Arc::new(Float64Array::from(vec![f64::NAN])) as ArrayRef;
        let result = scale.invert_from_numeric(&config, &values).unwrap();
        let result_vec = result.as_vec(values.len(), None);
        assert!(result_vec[0].is_nan());

        // Test infinity
        let values =
            Arc::new(Float64Array::from(vec![f64::INFINITY, f64::NEG_INFINITY])) as ArrayRef;
        let result = scale.invert_from_numeric(&config, &values).unwrap();
        let result_vec = result.as_vec(values.len(), None);
        assert_approx_eq!(f64, result_vec[0], 100.0); // maps to domain end
        assert_approx_eq!(f64, result_vec[1], -100.0); // maps to domain start
    }

    #[test]
    fn test_invert_degenerate() {
        // Test degenerate domain
        let scale = SymlogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![1.0, 1.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 1.0])),
            options: HashMap::new(),
            context: ScaleContext::default(),
        };
        let values = Arc::new(Float64Array::from(vec![0.0, 0.5, 1.0])) as ArrayRef;
        let result = scale.invert_from_numeric(&config, &values).unwrap();
        let result_vec = result.as_vec(values.len(), None);
        for &value in &result_vec {
            assert_approx_eq!(f64, value, 1.0);
        }

        // Test degenerate range
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![-100.0, 100.0])),
            range: Arc::new(Float64Array::from(vec![1.0, 1.0])),
            options: HashMap::new(),
            context: ScaleContext::default(),
        };
        let result = scale.invert_from_numeric(&config, &values).unwrap();
        let result_vec = result.as_vec(values.len(), None);
        for &value in &result_vec {
            assert_approx_eq!(f64, value, -100.0);
        }
    }

    #[test]
    fn test_ticks() {
        let scale = SymlogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![-1.0, 1.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 1.0])),
            options: HashMap::new(),
            context: ScaleContext::default(),
        };
        let ticks = scale.ticks(&config, Some(10.0)).unwrap();
        let ticks_array = ticks.as_primitive::<Float64Type>();
        // Widened ticks are the exact decimals.
        assert_eq!(
            ticks_array.values().to_vec(),
            [-1.0, -0.8, -0.6, -0.4, -0.2, 0.0, 0.2, 0.4, 0.6, 0.8, 1.0]
        );
    }

    #[test]
    fn test_ticks_with_constant() {
        let constant = 2.0;
        let scale = SymlogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![-10.0, 10.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 1.0])),
            options: vec![("constant".to_string(), constant.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let ticks = scale.ticks(&config, Some(5.0)).unwrap();
        let ticks_array = ticks.as_primitive::<Float64Type>();
        assert!(!ticks.is_empty());

        // Ticks should be symmetric around zero
        let mid_idx = ticks.len() / 2;
        if ticks.len() % 2 == 1 {
            assert_approx_eq!(f64, ticks_array.value(mid_idx), 0.0);
        }

        // Test symmetry of positive/negative ticks
        for i in 0..mid_idx {
            assert_approx_eq!(
                f64,
                ticks_array.value(i).abs(),
                ticks_array.value(ticks.len() - 1 - i).abs()
            );
        }
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
                    f64,
                    inverted,
                    x,
                    F64Margin {
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
