use std::sync::Arc;

use arrow::{
    array::{ArrayRef, AsArray, Float32Array, StringArray},
    compute::{kernels::cast, unary},
    datatypes::{DataType, Float32Type},
};
use avenger_common::{
    types::LinearScaleAdjustment,
    value::{ScalarOrArray, ScalarOrArrayValue},
};
use lazy_static::lazy_static;

use crate::{
    array, color_interpolator::scale_numeric_to_color, error::AvengerScaleError, scalar::Scalar,
};

use super::{
    ConfiguredScale, InferDomainFromDataMethod, OptionConstraint, OptionDefinition, ScaleConfig,
    ScaleContext, ScaleImpl,
};

/// Linear scale that maps a continuous numeric domain to a continuous numeric range.
///
/// # Config Options
///
/// - **clamp** (boolean, default: false): When true, values outside the domain are clamped to the domain extent.
///   For scaling, this means values below the domain minimum map to the range minimum, and values above
///   the domain maximum map to the range maximum. For inversion, the clamping is applied to the range.
///
/// - **range_offset** (f32, default: 0.0): An offset applied to the final scaled values. This is added
///   after the linear transformation. When inverting, this offset is subtracted from input values first.
///
/// - **round** (boolean, default: false): When true, output values from scaling are rounded to the nearest
///   integer. This is useful for pixel-perfect rendering. Does not affect inversion.
///
/// - **nice** (boolean or f32, default: false): When true or a number, extends the domain to nice round values.
///   If true, uses a default count of 10. If a number, uses that as the target tick count for determining
///   nice values. Nice domains are computed to align with human-friendly values (multiples of 1, 2, 5, 10, etc.).
///
/// - **zero** (boolean, default: false): When true, ensures that the domain includes zero. If both min and max
///   are positive, sets min to zero. If both min and max are negative, sets max to zero. If the domain already
///   spans zero, no change is made. Zero extension is applied before nice calculations.
///
/// - **padding** (f32, default: 0.0): Expands the scale domain to accommodate the specified number of pixels
///   on each of the scale range. The scale range must represent pixels for this parameter to function as intended.
///   Padding adjustment is performed prior to all other adjustments, including the effects of the zero and nice properties.
#[derive(Debug)]
pub struct LinearScale;

impl LinearScale {
    pub fn configured(domain: (f32, f32), range: (f32, f32)) -> ConfiguredScale {
        ConfiguredScale {
            scale_impl: Arc::new(Self),
            config: ScaleConfig {
                domain: Arc::new(Float32Array::from(vec![domain.0, domain.1])),
                range: Arc::new(Float32Array::from(vec![range.0, range.1])),
                options: vec![
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

    pub fn configured_color<I>(domain: (f32, f32), range: I) -> ConfiguredScale
    where
        I: IntoIterator,
        I::Item: Into<String>,
    {
        ConfiguredScale {
            scale_impl: Arc::new(Self),
            config: ScaleConfig {
                domain: Arc::new(Float32Array::from(vec![domain.0, domain.1])),
                range: Arc::new(StringArray::from(
                    range.into_iter().map(Into::into).collect::<Vec<String>>(),
                )),
                options: Default::default(),
                context: ScaleContext::default(),
            },
        }
    }

    /// Apply normalization (padding, zero and nice) to domain
    pub fn apply_normalization(
        domain: (f32, f32),
        range: (f32, f32),
        padding: Option<&Scalar>,
        zero: Option<&Scalar>,
        nice: Option<&Scalar>,
    ) -> Result<(f32, f32), AvengerScaleError> {
        let (mut domain_start, mut domain_end) = domain;

        // Early return for degenerate cases
        if domain_start == domain_end || domain_start.is_nan() || domain_end.is_nan() {
            return Ok(domain);
        }

        // Step 1: Apply padding if requested (before all other adjustments)
        if let Some(padding_option) = padding {
            let padding_value = padding_option.as_f32()?;
            if padding_value > 0.0 {
                let (padded_start, padded_end) =
                    Self::apply_padding((domain_start, domain_end), range, padding_value)?;
                domain_start = padded_start;
                domain_end = padded_end;
            }
        }

        // Step 2: Apply zero extension if requested
        if let Some(zero_option) = zero {
            if let Ok(true) = zero_option.as_boolean() {
                if domain_start > 0.0 && domain_end > 0.0 {
                    // Both positive, extend to include zero at start
                    domain_start = 0.0;
                } else if domain_start < 0.0 && domain_end < 0.0 {
                    // Both negative, extend to include zero at end
                    domain_end = 0.0;
                }
                // If domain spans zero, no change needed
            }
        }

        // Step 3: Apply nice transformation if requested
        let nice_count = if let Some(count) = nice {
            if count.array().data_type().is_numeric() {
                Some(count.as_f32()?)
            } else if let Ok(true) = count.as_boolean() {
                Some(10.0)
            } else {
                None
            }
        } else {
            None
        };

        if let Some(count) = nice_count {
            // Apply nice transformation to the zero-extended domain
            let (nice_start, nice_end) =
                Self::apply_nice_internal((domain_start, domain_end), count)?;
            domain_start = nice_start;
            domain_end = nice_end;
        }

        Ok((domain_start, domain_end))
    }

    /// Internal nice calculation (kept for backward compatibility and internal use)
    fn apply_nice_internal(
        domain: (f32, f32),
        count: f32,
    ) -> Result<(f32, f32), AvengerScaleError> {
        let (domain_start, domain_end) = domain;

        if domain_start == domain_end || domain_start.is_nan() || domain_end.is_nan() {
            return Ok(domain);
        }

        let (mut start, mut stop) = if domain_start <= domain_end {
            (domain_start, domain_end)
        } else {
            (domain_end, domain_start)
        };

        let mut prestep = 0.0;
        let mut max_iter = 10;

        while max_iter > 0 {
            let step = array::tick_increment(start, stop, count);

            if step == prestep {
                if domain_start <= domain_end {
                    return Ok((start, stop));
                } else {
                    return Ok((stop, start));
                }
            } else if step > 0.0 {
                start = (start / step).floor() * step;
                stop = (stop / step).ceil() * step;
            } else if step < 0.0 {
                start = (start * step).ceil() / step;
                stop = (stop * step).floor() / step;
            } else {
                break;
            }

            prestep = step;
            max_iter -= 1;
        }

        if domain_start <= domain_end {
            Ok((start, stop))
        } else {
            Ok((stop, start))
        }
    }

    /// Compute nice domain (backward compatibility wrapper)
    pub fn apply_nice(
        domain: (f32, f32),
        count: Option<&Scalar>,
    ) -> Result<(f32, f32), AvengerScaleError> {
        Self::apply_normalization(domain, (0.0, 1.0), None, None, count)
    }

    /// Apply padding to domain based on pixel values
    pub fn apply_padding(
        domain: (f32, f32),
        range: (f32, f32),
        padding: f32,
    ) -> Result<(f32, f32), AvengerScaleError> {
        let (domain_start, domain_end) = domain;
        let (range_start, range_end) = range;

        // Early return for degenerate cases
        if domain_start == domain_end || range_start == range_end || padding <= 0.0 {
            return Ok(domain);
        }

        // Calculate the span of the range in pixels
        let span = (range_end - range_start).abs();

        // Calculate scale factor: frac = span / (span - 2 * pad)
        // This represents how much to expand the domain
        let frac = span / (span - 2.0 * padding);

        // For linear scale, zoom from center (anchor = 0.5)
        let domain_center = (domain_start + domain_end) / 2.0;

        // Expand domain by scale factor
        let new_start = domain_center + (domain_start - domain_center) * frac;
        let new_end = domain_center + (domain_end - domain_center) * frac;

        Ok((new_start, new_end))
    }
}

impl ScaleImpl for LinearScale {
    fn scale_type(&self) -> &'static str {
        "linear"
    }

    fn infer_domain_from_data_method(&self) -> InferDomainFromDataMethod {
        InferDomainFromDataMethod::Interval
    }

    fn option_definitions(&self) -> &[OptionDefinition] {
        lazy_static! {
            static ref DEFINITIONS: Vec<OptionDefinition> = vec![
                OptionDefinition::optional("clamp", OptionConstraint::Boolean),
                OptionDefinition::optional("range_offset", OptionConstraint::Float),
                OptionDefinition::optional("round", OptionConstraint::Boolean),
                OptionDefinition::optional("nice", OptionConstraint::nice()),
                OptionDefinition::optional("zero", OptionConstraint::Boolean),
                OptionDefinition::optional("default", OptionConstraint::Float),
                OptionDefinition::optional("padding", OptionConstraint::NonNegativeFloat),
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
        // Check if color interpolation is needed FIRST
        if config.color_range().is_ok() {
            // Get domain normalization without needing numeric range
            let (domain_start, domain_end) = LinearScale::apply_normalization(
                config.numeric_interval_domain()?,
                (0.0, 1.0), // dummy range for padding calculation
                config.options.get("padding"),
                config.options.get("zero"),
                config.options.get("nice"),
            )?;

            // Create new config with niced domain
            let config = ScaleConfig {
                domain: Arc::new(Float32Array::from(vec![domain_start, domain_end])),
                ..config.clone()
            };
            return scale_numeric_to_color(self, &config, values);
        }

        // For numeric ranges, get the actual range values
        let (range_start, range_end) = config.numeric_interval_range()?;
        let (domain_start, domain_end) = LinearScale::apply_normalization(
            config.numeric_interval_domain()?,
            (range_start, range_end),
            config.options.get("padding"),
            config.options.get("zero"),
            config.options.get("nice"),
        )?;

        // Handle degenerate domain/range cases
        if domain_start == domain_end
            || range_start == range_end
            || domain_start.is_nan()
            || domain_end.is_nan()
            || range_start.is_nan()
            || range_end.is_nan()
        {
            return Ok(Arc::new(Float32Array::from(vec![
                range_start;
                values.len()
            ])));
        }

        // Cast to f32 and downcast to f32 array
        let array = cast(values, &DataType::Float32)?;
        let array = array.as_primitive::<Float32Type>();

        // Get options
        let range_offset = config.option_f32("range_offset", 0.0);
        let clamp = config.option_boolean("clamp", false);
        let round = config.option_boolean("round", false);

        // Extract domain and range
        let domain_span = domain_end - domain_start;
        let scale = (range_end - range_start) / domain_span;
        let offset = range_start - scale * domain_start + range_offset;

        let (range_min, range_max) = if range_start <= range_end {
            (range_start, range_end)
        } else {
            (range_end, range_start)
        };

        let scaled_vec: Float32Array = match (clamp, round) {
            (true, true) => {
                // clamp and round
                unary(array, |v| {
                    (scale * v + offset).clamp(range_min, range_max).round()
                })
            }
            (true, false) => {
                // clamp, no round
                unary(array, |v| (scale * v + offset).clamp(range_min, range_max))
            }
            (false, true) => {
                // no clamp, round
                unary(array, |v| (scale * v + offset).round())
            }
            (false, false) => {
                // no clamp, no round
                unary(array, |v| scale * v + offset)
            }
        };

        Ok(Arc::new(scaled_vec))
    }

    fn invert_from_numeric(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        let (range_start, range_end) = config.numeric_interval_range()?;
        let (domain_start, domain_end) = LinearScale::apply_normalization(
            config.numeric_interval_domain()?,
            (range_start, range_end),
            config.options.get("padding"),
            config.options.get("zero"),
            config.options.get("nice"),
        )?;
        let range_offset = config.option_f32("range_offset", 0.0);
        let clamp = config.option_boolean("clamp", false);

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
        let array = cast(values, &DataType::Float32)?;
        let array = array.as_primitive::<Float32Type>();

        let scale = (domain_end - domain_start) / (range_end - range_start);
        let offset = domain_start - scale * range_start;

        if clamp {
            let (range_min, range_max) = if range_start <= range_end {
                (range_start, range_end)
            } else {
                (range_end, range_start)
            };

            Ok(ScalarOrArray::new_array(
                array
                    .values()
                    .iter()
                    .map(|v| {
                        let v = (v - range_offset).clamp(range_min, range_max);
                        scale * v + offset
                    })
                    .collect(),
            ))
        } else {
            Ok(ScalarOrArray::new_array(
                array
                    .values()
                    .iter()
                    .map(|v| scale * (v - range_offset) + offset)
                    .collect(),
            ))
        }
    }

    fn ticks(
        &self,
        config: &ScaleConfig,
        count: Option<f32>,
    ) -> Result<ArrayRef, AvengerScaleError> {
        let (range_start, range_end) = config.numeric_interval_range()?;
        let (domain_start, domain_end) = LinearScale::apply_normalization(
            config.numeric_interval_domain()?,
            (range_start, range_end),
            config.options.get("padding"),
            config.options.get("zero"),
            config.options.get("nice"),
        )?;

        let count = count.unwrap_or(10.0);
        let ticks_array = Float32Array::from(array::ticks(domain_start, domain_end, count));
        Ok(Arc::new(ticks_array) as ArrayRef)
    }

    /// Pans the domain by the given delta
    ///
    /// The delta value represents fractional units of the scale range; for example,
    /// 0.5 indicates panning the scale domain to the right by half the scale range.
    fn pan(&self, config: &ScaleConfig, delta: f32) -> Result<ScaleConfig, AvengerScaleError> {
        let (domain_start, domain_end) = config.numeric_interval_domain()?;
        let domain_delta = (domain_end - domain_start) * delta;
        Ok(ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![
                domain_start - domain_delta,
                domain_end - domain_delta,
            ])),
            ..config.clone()
        })
    }

    /// Zooms the domain by the given scale factor
    ///
    /// The anchor value represents the zoom position in terms of fractional units of the
    /// scale range; for example, 0.5 indicates a zoom centered on the mid-point of the
    /// scale range.
    ///
    /// The scale factor represents the amount to scale the domain by; for example,
    /// 2.0 indicates zooming the scale domain to be twice as large.
    fn zoom(
        &self,
        config: &ScaleConfig,
        anchor: f32,
        scale_factor: f32,
    ) -> Result<ScaleConfig, AvengerScaleError> {
        let (domain_start, domain_end) = config.numeric_interval_domain()?;
        let domain_anchor = domain_start + anchor * (domain_end - domain_start);

        let new_start = domain_anchor + (domain_start - domain_anchor) * scale_factor;
        let new_end = domain_anchor + (domain_end - domain_anchor) * scale_factor;

        Ok(ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![new_start, new_end])),
            ..config.clone()
        })
    }

    /// Compute adjustment for data that was originally scaled by `from_config` to be scaled
    /// by `to_config`
    fn adjust(
        &self,
        from_config: &ScaleConfig,
        to_config: &ScaleConfig,
    ) -> Result<LinearScaleAdjustment, AvengerScaleError> {
        // Solve with sympy
        // -----------------
        // ```python
        // from sympy import symbols, solve, factor
        // # Define variables
        // adj_scale, adj_offset = symbols('adj_scale adj_offset', real=True)
        // domain_a_start, domain_a_end = symbols('from_domain_start from_domain_end', real=True)
        // range_a_start, range_a_end = symbols('from_range_start from_range_end', real=True)
        // domain_b_start, domain_b_end = symbols('to_domain_start to_domain_end', real=True)
        // range_b_start, range_b_end = symbols('to_range_start to_range_end', real=True)
        //
        // # Define scale and offset for scale A:
        // scale_a = (range_a_end - range_a_start)/(domain_a_end - domain_a_start)
        // offset_a = range_a_start - scale_a * domain_a_start
        //
        // # Define scale and offset for scale B:
        // scale_b = (range_b_end - range_b_start)/(domain_b_end - domain_b_start)
        // offset_b = range_b_start - scale_b * domain_b_start
        //
        // # Map domain_a_start with both scales:
        // range_value_a1 = scale_a * domain_a_start + offset_a
        // range_value_b1 = scale_b * domain_a_start + offset_b
        //
        // # Map domain_a_end with both scales:
        // range_value_a2 = scale_a * domain_a_end + offset_a
        // range_value_b2 = scale_b * domain_a_end + offset_b
        //
        // # Solve for adjustment factors:
        // eq1 = adj_scale * range_value_a1 + adj_offset - range_value_b1
        // eq2 = adj_scale * range_value_a2 + adj_offset - range_value_b2
        //
        // solution = solve((eq1, eq2), (adj_scale, adj_offset))
        // print("let scale =", factor(solution[adj_scale].simplify()))
        // print("let offset =", factor(solution[adj_offset].simplify()))
        // ```
        let (from_domain_start, from_domain_end) = from_config.numeric_interval_domain()?;
        let (from_range_start, from_range_end) = to_config.numeric_interval_range()?;
        let (to_domain_start, to_domain_end) = to_config.numeric_interval_domain()?;
        let (to_range_start, to_range_end) = to_config.numeric_interval_range()?;

        let scale = (from_domain_end - from_domain_start) * (to_range_end - to_range_start)
            / ((from_range_end - from_range_start) * (to_domain_end - to_domain_start));

        let offset = -(from_domain_end * from_range_start * to_range_end
            - from_domain_end * from_range_start * to_range_start
            - from_domain_start * from_range_end * to_range_end
            + from_domain_start * from_range_end * to_range_start
            - from_range_end * to_domain_end * to_range_start
            + from_range_end * to_domain_start * to_range_end
            + from_range_start * to_domain_end * to_range_start
            - from_range_start * to_domain_start * to_range_end)
            / ((from_range_end - from_range_start) * (to_domain_end - to_domain_start));
        Ok(LinearScaleAdjustment { scale, offset })
    }

    fn compute_nice_domain(&self, config: &ScaleConfig) -> Result<ArrayRef, AvengerScaleError> {
        let (range_start, range_end) = config.numeric_interval_range()?;
        let (domain_start, domain_end) = LinearScale::apply_normalization(
            config.numeric_interval_domain()?,
            (range_start, range_end),
            config.options.get("padding"),
            config.options.get("zero"),
            config.options.get("nice"),
        )?;

        Ok(Arc::new(Float32Array::from(vec![domain_start, domain_end])) as ArrayRef)
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
            LinearScale::apply_nice(config.numeric_interval_domain()?, Some(&Scalar::from(10.0)))?;
        assert_eq!(niced_domain, (1.0, 11.0));

        Ok(())
    }

    #[test]
    fn test_configured_scale_invert() -> Result<(), AvengerScaleError> {
        use arrow::array::Int32Array;
        // Test the new invert() method that returns ArrayRef
        let scale = LinearScale::configured((10.0, 30.0), (0.0, 100.0));

        // Test with Float32 values
        let values = Arc::new(Float32Array::from(vec![0.0, 25.0, 50.0, 75.0, 100.0])) as ArrayRef;
        let result = scale.invert(&values)?;
        let result_array = result.as_primitive::<Float32Type>();

        assert_approx_eq!(f32, result_array.value(0), 10.0); // range start -> domain start
        assert_approx_eq!(f32, result_array.value(1), 15.0); // 25% -> 15
        assert_approx_eq!(f32, result_array.value(2), 20.0); // 50% -> 20
        assert_approx_eq!(f32, result_array.value(3), 25.0); // 75% -> 25
        assert_approx_eq!(f32, result_array.value(4), 30.0); // range end -> domain end

        // Test with Int32 values (should be cast to Float32 internally)
        let values = Arc::new(Int32Array::from(vec![0, 50, 100])) as ArrayRef;
        let result = scale.invert(&values)?;
        let result_array = result.as_primitive::<Float32Type>();

        assert_eq!(result.len(), 3);
        assert_approx_eq!(f32, result_array.value(0), 10.0);
        assert_approx_eq!(f32, result_array.value(1), 20.0);
        assert_approx_eq!(f32, result_array.value(2), 30.0);

        // Test with clamping
        let scale = scale.with_option("clamp", true);
        let values = Arc::new(Float32Array::from(vec![-50.0, 150.0])) as ArrayRef;
        let result = scale.invert(&values)?;
        let result_array = result.as_primitive::<Float32Type>();

        assert_approx_eq!(f32, result_array.value(0), 10.0); // clamped to domain start
        assert_approx_eq!(f32, result_array.value(1), 30.0); // clamped to domain end

        Ok(())
    }

    #[test]
    fn test_normalize_with_nice_true() -> Result<(), AvengerScaleError> {
        let scale = LinearScale::configured((1.1, 10.9), (0.0, 100.0)).with_option("nice", true);

        let normalized = scale.normalize()?;

        // Check that the domain has been niced
        let domain = normalized.numeric_interval_domain()?;
        assert_approx_eq!(f32, domain.0, 1.0);
        assert_approx_eq!(f32, domain.1, 11.0);

        // Check that the nice option is disabled
        assert!(!normalized.option_boolean("nice", true));

        Ok(())
    }

    #[test]
    fn test_normalize_with_nice_false() -> Result<(), AvengerScaleError> {
        let scale = LinearScale::configured((1.1, 10.9), (0.0, 100.0)).with_option("nice", false);

        let normalized = scale.normalize()?;

        // Check that the domain is unchanged
        let domain = normalized.numeric_interval_domain()?;
        assert_approx_eq!(f32, domain.0, 1.1);
        assert_approx_eq!(f32, domain.1, 10.9);

        // Check that the nice option is still false
        assert!(!normalized.option_boolean("nice", true));

        Ok(())
    }

    #[test]
    fn test_normalize_with_nice_count() -> Result<(), AvengerScaleError> {
        let scale = LinearScale::configured((1.1, 10.9), (0.0, 100.0)).with_option("nice", 5.0);

        let normalized = scale.normalize()?;

        // Check that the domain has been niced
        let domain = normalized.numeric_interval_domain()?;
        let expected_nice = LinearScale::apply_nice((1.1, 10.9), Some(&5.0.into()))?;

        assert_approx_eq!(f32, domain.0, expected_nice.0);
        assert_approx_eq!(f32, domain.1, expected_nice.1);

        // Check that the nice option is disabled
        assert!(!normalized.option_boolean("nice", true));

        Ok(())
    }

    #[test]
    fn test_normalize_with_zero_both_positive() -> Result<(), AvengerScaleError> {
        let scale = LinearScale::configured((2.0, 10.0), (0.0, 100.0)).with_option("zero", true);

        let normalized = scale.normalize()?;

        // Check that zero is included (min should be 0)
        let domain = normalized.numeric_interval_domain()?;
        assert_approx_eq!(f32, domain.0, 0.0);
        assert_approx_eq!(f32, domain.1, 10.0);

        // Check that the zero option is disabled
        assert!(!normalized.option_boolean("zero", true));

        Ok(())
    }

    #[test]
    fn test_normalize_with_zero_both_negative() -> Result<(), AvengerScaleError> {
        let scale = LinearScale::configured((-10.0, -2.0), (0.0, 100.0)).with_option("zero", true);

        let normalized = scale.normalize()?;

        // Check that zero is included (max should be 0)
        let domain = normalized.numeric_interval_domain()?;
        assert_approx_eq!(f32, domain.0, -10.0);
        assert_approx_eq!(f32, domain.1, 0.0);

        // Check that the zero option is disabled
        assert!(!normalized.option_boolean("zero", true));

        Ok(())
    }

    #[test]
    fn test_normalize_with_zero_spans_zero() -> Result<(), AvengerScaleError> {
        let scale = LinearScale::configured((-5.0, 5.0), (0.0, 100.0)).with_option("zero", true);

        let normalized = scale.normalize()?;

        // Check that domain is unchanged (already spans zero)
        let domain = normalized.numeric_interval_domain()?;
        assert_approx_eq!(f32, domain.0, -5.0);
        assert_approx_eq!(f32, domain.1, 5.0);

        // Check that the zero option is disabled
        assert!(!normalized.option_boolean("zero", true));

        Ok(())
    }

    #[test]
    fn test_normalize_with_zero_and_nice() -> Result<(), AvengerScaleError> {
        let scale = LinearScale::configured((2.1, 9.9), (0.0, 100.0))
            .with_option("zero", true)
            .with_option("nice", true);

        let normalized = scale.normalize()?;

        // Check that zero is applied first, then nice
        let domain = normalized.numeric_interval_domain()?;
        // Should be (0.0, 9.9) after zero, then (0.0, 10.0) after nice
        assert_approx_eq!(f32, domain.0, 0.0);
        assert_approx_eq!(f32, domain.1, 10.0);

        // Check that both options are disabled
        assert!(!normalized.option_boolean("zero", true));
        assert!(!normalized.option_boolean("nice", true));

        Ok(())
    }

    #[test]
    fn test_normalize_with_zero_false() -> Result<(), AvengerScaleError> {
        let scale = LinearScale::configured((2.0, 10.0), (0.0, 100.0)).with_option("zero", false);

        let normalized = scale.normalize()?;

        // Check that domain is unchanged
        let domain = normalized.numeric_interval_domain()?;
        assert_approx_eq!(f32, domain.0, 2.0);
        assert_approx_eq!(f32, domain.1, 10.0);

        // Check that the zero option is disabled
        assert!(!normalized.option_boolean("zero", true));

        Ok(())
    }

    #[test]
    fn test_apply_normalization_zero_only() -> Result<(), AvengerScaleError> {
        // Both positive
        let result = LinearScale::apply_normalization(
            (2.0, 10.0),
            (0.0, 1.0),
            None,
            Some(&true.into()),
            None,
        )?;
        assert_approx_eq!(f32, result.0, 0.0);
        assert_approx_eq!(f32, result.1, 10.0);

        // Both negative
        let result = LinearScale::apply_normalization(
            (-10.0, -2.0),
            (0.0, 1.0),
            None,
            Some(&true.into()),
            None,
        )?;
        assert_approx_eq!(f32, result.0, -10.0);
        assert_approx_eq!(f32, result.1, 0.0);

        // Spans zero (no change)
        let result = LinearScale::apply_normalization(
            (-5.0, 5.0),
            (0.0, 1.0),
            None,
            Some(&true.into()),
            None,
        )?;
        assert_approx_eq!(f32, result.0, -5.0);
        assert_approx_eq!(f32, result.1, 5.0);

        // Zero false (no change)
        let result = LinearScale::apply_normalization(
            (2.0, 10.0),
            (0.0, 1.0),
            None,
            Some(&false.into()),
            None,
        )?;
        assert_approx_eq!(f32, result.0, 2.0);
        assert_approx_eq!(f32, result.1, 10.0);

        Ok(())
    }

    #[test]
    fn test_apply_padding() -> Result<(), AvengerScaleError> {
        // Test basic padding
        let result = LinearScale::apply_padding((0.0, 10.0), (0.0, 100.0), 10.0)?;
        // With padding of 10 pixels on each side and range of 100, scale factor = 100 / 80 = 1.25
        // Domain expands from center (5.0) by factor 1.25
        assert_approx_eq!(f32, result.0, -1.25); // 5 + (0 - 5) * 1.25 = -1.25
        assert_approx_eq!(f32, result.1, 11.25); // 5 + (10 - 5) * 1.25 = 11.25

        // Test with negative domain
        let result = LinearScale::apply_padding((-20.0, -10.0), (0.0, 100.0), 10.0)?;
        // Center is -15, scale factor = 1.25
        assert_approx_eq!(f32, result.0, -21.25); // -15 + (-20 - -15) * 1.25 = -21.25
        assert_approx_eq!(f32, result.1, -8.75); // -15 + (-10 - -15) * 1.25 = -8.75

        // Test with reversed range
        let result = LinearScale::apply_padding((0.0, 10.0), (100.0, 0.0), 10.0)?;
        // Range span is still 100, so same scale factor
        assert_approx_eq!(f32, result.0, -1.25);
        assert_approx_eq!(f32, result.1, 11.25);

        // Test with zero padding (no change)
        let result = LinearScale::apply_padding((0.0, 10.0), (0.0, 100.0), 0.0)?;
        assert_approx_eq!(f32, result.0, 0.0);
        assert_approx_eq!(f32, result.1, 10.0);

        // Test with degenerate domain (no change)
        let result = LinearScale::apply_padding((5.0, 5.0), (0.0, 100.0), 10.0)?;
        assert_approx_eq!(f32, result.0, 5.0);
        assert_approx_eq!(f32, result.1, 5.0);

        // Test with degenerate range (no change)
        let result = LinearScale::apply_padding((0.0, 10.0), (50.0, 50.0), 10.0)?;
        assert_approx_eq!(f32, result.0, 0.0);
        assert_approx_eq!(f32, result.1, 10.0);

        Ok(())
    }

    #[test]
    fn test_linear_scale_with_padding() -> Result<(), AvengerScaleError> {
        // Create a linear scale with padding
        let scale = LinearScale::configured((0.0, 10.0), (0.0, 100.0)).with_option("padding", 10.0);

        // Normalize the scale to apply padding
        let normalized = scale.normalize()?;

        // Check that domain has been expanded
        let domain = normalized.numeric_interval_domain()?;
        assert_approx_eq!(f32, domain.0, -1.25);
        assert_approx_eq!(f32, domain.1, 11.25);

        // Test scaling values
        let values = Arc::new(Float32Array::from(vec![0.0, 5.0, 10.0])) as ArrayRef;
        let result = normalized.scale(&values)?;
        let result_array = result.as_primitive::<Float32Type>();

        // With expanded domain, the original values should map differently
        // Domain is [-1.25, 11.25], range is [0, 100]
        // Scale factor = 100 / 12.5 = 8
        // 0.0 -> (0 - -1.25) * 8 = 10
        // 5.0 -> (5 - -1.25) * 8 = 50
        // 10.0 -> (10 - -1.25) * 8 = 90

        // The scale operation with expanded domain:
        // Domain is [-1.25, 11.25], range is [0, 100]
        // Scale factor = 100 / 12.5 = 8
        // 0.0 -> (0 - -1.25) * 8 = 10
        // 5.0 -> (5 - -1.25) * 8 = 50
        // 10.0 -> (10 - -1.25) * 8 = 90
        assert_approx_eq!(f32, result_array.value(0), 10.0);
        assert_approx_eq!(f32, result_array.value(1), 50.0);
        assert_approx_eq!(f32, result_array.value(2), 90.0);

        Ok(())
    }

    #[test]
    fn test_padding_with_zero_and_nice() -> Result<(), AvengerScaleError> {
        // Test that transformations are applied in order: padding -> zero -> nice
        let scale = LinearScale::configured((2.0, 10.0), (0.0, 100.0))
            .with_option("padding", 9.0)
            .with_option("zero", true)
            .with_option("nice", true);

        let normalized = scale.normalize()?;
        let domain = normalized.numeric_interval_domain()?;

        // Expected transformations:
        assert_approx_eq!(f32, domain.0, 0.0);
        assert_approx_eq!(f32, domain.1, 11.0);

        // Verify all normalization options are disabled
        assert_eq!(normalized.option_f32("padding", -1.0), 0.0);
        assert!(!normalized.option_boolean("zero", true));
        assert!(!normalized.option_boolean("nice", true));

        Ok(())
    }
}
