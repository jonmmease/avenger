use std::sync::Arc;

use arrow::{
    array::{ArrayRef, AsArray, Float32Array, StringArray},
    compute::kernels::cast,
    datatypes::{DataType, Float32Type},
};
use avenger_common::{types::ColorOrGradient, value::ScalarOrArray};
use datafusion_common::ScalarValue;

use crate::{
    array,
    color_interpolator::{scale_numeric_to_color, ColorInterpolator, SrgbaColorInterpolator},
    error::AvengerScaleError,
    formatter::Formatters,
    utils::ScalarValueUtils,
};

use super::{ConfiguredScale, InferDomainFromDataMethod, ScaleConfig, ScaleImpl};

#[derive(Debug)]
pub struct LinearScale;

impl LinearScale {
    pub fn new(domain: (f32, f32), range: (f32, f32)) -> ConfiguredScale {
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
            },
            color_interpolator: Arc::new(SrgbaColorInterpolator),
            formatters: Formatters::default(),
        }
    }

    pub fn new_color<I>(domain: (f32, f32), range: I) -> ConfiguredScale
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
            },
            color_interpolator: Arc::new(SrgbaColorInterpolator),
            formatters: Formatters::default(),
        }
    }

    /// Compute nice domain
    pub fn apply_nice(
        domain: (f32, f32),
        count: Option<&ScalarValue>,
    ) -> Result<(f32, f32), AvengerScaleError> {
        // Extract count, or return raw domain if no nice option
        let count = if let Some(count) = count {
            if count.data_type().is_numeric() {
                count.as_f32()?
            } else if count == &ScalarValue::from(true) {
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
            let step = array::tick_increment(start as f32, stop as f32, count as f32);

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

    fn scale_to_numeric(
        &self,
        config: &ScaleConfig,
        values: &arrow::array::ArrayRef,
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        let (domain_start, domain_end) = LinearScale::apply_nice(
            config.numeric_interval_domain()?,
            config.options.get("nice"),
        )?;

        let (range_start, range_end) = config.numeric_interval_range()?;

        // Handle degenerate domain/range cases
        if domain_start == domain_end
            || range_start == range_end
            || domain_start.is_nan()
            || domain_end.is_nan()
            || range_start.is_nan()
            || range_end.is_nan()
        {
            return Ok(ScalarOrArray::new_array(vec![range_start; values.len()]));
        }

        // Cast to f32 and downcast to f32 array
        let array = cast(values, &DataType::Float32)?;
        let array = array.as_primitive::<Float32Type>();

        // Get options
        let range_offset = config.f32_option("range_offset", 0.0);
        let clamp = config.boolean_option("clamp", false);
        let round = config.boolean_option("round", false);

        // Extract domain and range
        let domain_span = domain_end - domain_start;
        let scale = (range_end - range_start) / domain_span;
        let offset = range_start - scale * domain_start + range_offset;

        let (range_min, range_max) = if range_start <= range_end {
            (range_start, range_end)
        } else {
            (range_end, range_start)
        };

        match (clamp, round) {
            (true, true) => {
                // clamp and round
                Ok(ScalarOrArray::new_array(
                    array
                        .values()
                        .iter()
                        .map(|v| (scale * v + offset).clamp(range_min, range_max).round())
                        .collect(),
                ))
            }
            (true, false) => {
                // clamp, no round
                Ok(ScalarOrArray::new_array(
                    array
                        .values()
                        .iter()
                        .map(|v| (scale * v + offset).clamp(range_min, range_max))
                        .collect(),
                ))
            }
            (false, true) => {
                // no clamp, round
                Ok(ScalarOrArray::new_array(
                    array
                        .values()
                        .iter()
                        .map(|v| (scale * v + offset).round())
                        .collect(),
                ))
            }
            (false, false) => {
                // no clamp, no round
                Ok(ScalarOrArray::new_array(
                    array.values().iter().map(|v| scale * v + offset).collect(),
                ))
            }
        }
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
        let range_offset = config.f32_option("range_offset", 0.0);
        let clamp = config.boolean_option("clamp", false);

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

    /// Scale to color values
    fn scale_to_color(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
        interpolator: &dyn ColorInterpolator,
    ) -> Result<ScalarOrArray<ColorOrGradient>, AvengerScaleError> {
        scale_numeric_to_color(self, config, values, interpolator)
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
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

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
        };

        let scale = LinearScale;
        let values = Arc::new(Float32Array::from(vec![0.0, 10.0, 20.0])) as ArrayRef;
        let result = scale
            .scale_to_numeric(&config, &values)?
            .as_vec(values.len(), None);

        // All values should map to range_start (d3 behavior)
        for i in 0..result.len() {
            assert_approx_eq!(f32, result[i], 0.0);
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
        };

        let niced_domain =
            LinearScale::apply_nice(config.numeric_interval_domain()?, Some(&10.0.into()))?;
        assert_eq!(niced_domain, (1.0, 11.0));

        Ok(())
    }
}
