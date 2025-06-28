use std::sync::Arc;

use arrow::{
    array::{ArrayRef, AsArray, Float32Array, StringArray},
    compute::{kernels::cast, unary},
    datatypes::{DataType, Float32Type},
};
use avenger_common::{types::LinearScaleAdjustment, value::ScalarOrArray};

use crate::{
    array, color_interpolator::scale_numeric_to_color, error::AvengerScaleError, scalar::Scalar,
};

use super::{ConfiguredScale, InferDomainFromDataMethod, ScaleConfig, ScaleContext, ScaleImpl};

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

    /// Compute nice domain
    pub fn apply_nice(
        domain: (f32, f32),
        count: Option<&Scalar>,
    ) -> Result<(f32, f32), AvengerScaleError> {
        // Extract count, or return raw domain if no nice option
        let count = if let Some(count) = count {
            if count.array().data_type().is_numeric() {
                count.as_f32()?
            } else if let Ok(true) = count.as_boolean() {
                10.0
            } else {
                return Ok(domain);
            }
        } else {
            return Ok(domain);
        };

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
}

impl ScaleImpl for LinearScale {
    fn infer_domain_from_data_method(&self) -> InferDomainFromDataMethod {
        InferDomainFromDataMethod::Interval
    }

    fn scale(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError> {
        let (domain_start, domain_end) = LinearScale::apply_nice(
            config.numeric_interval_domain()?,
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

        let (range_start, range_end) = config.numeric_interval_range()?;

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
        let (domain_start, domain_end) = LinearScale::apply_nice(
            config.numeric_interval_domain()?,
            config.options.get("nice"),
        )?;

        let (range_start, range_end) = config.numeric_interval_range()?;
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
        let (domain_start, domain_end) = LinearScale::apply_nice(
            config.numeric_interval_domain()?,
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
}
