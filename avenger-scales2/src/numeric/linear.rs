use avenger_common::value::ScalarOrArray;

use crate::{
    array,
    error::AvengerScaleError,
    numeric::{NumericScale, NumericScaleConfig},
};

pub struct LinearNumericScale;

impl NumericScale for LinearNumericScale {
    fn scale(
        &self,
        config: &NumericScaleConfig,
        values: &[f32],
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        let (domain_start, domain_end) = config.domain;
        let (range_start, range_end) = config.range;

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

        let domain_span = domain_end - domain_start;
        let scale = (range_end - range_start) / domain_span;
        let range_offset = config.range_offset;
        let offset = range_start - scale * domain_start + range_offset;

        let (range_min, range_max) = if range_start <= range_end {
            (range_start, range_end)
        } else {
            (range_end, range_start)
        };

        let clamp = config.clamp;
        let round = config.round;

        match (clamp, round) {
            (true, true) => {
                // clamp and round
                Ok(ScalarOrArray::new_array(
                    values
                        .iter()
                        .map(|v| (scale * v + offset).clamp(range_min, range_max).round())
                        .collect(),
                ))
            }
            (true, false) => {
                // clamp, no round
                Ok(ScalarOrArray::new_array(
                    values
                        .iter()
                        .map(|v| (scale * v + offset).clamp(range_min, range_max))
                        .collect(),
                ))
            }
            (false, true) => {
                // no clamp, round
                Ok(ScalarOrArray::new_array(
                    values
                        .iter()
                        .map(|v| (scale * v + offset).round())
                        .collect(),
                ))
            }
            (false, false) => {
                // no clamp, no round
                Ok(ScalarOrArray::new_array(
                    values.iter().map(|v| scale * v + offset).collect(),
                ))
            }
        }
    }

    /// Invert numeric values from continuous range to continuous domain
    fn invert(
        &self,
        config: &NumericScaleConfig,
        values: &[f32],
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        let (domain_start, domain_end) = config.domain;
        let (range_start, range_end) = config.range;

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

        let scale = (domain_end - domain_start) / (range_end - range_start);
        let range_offset = config.range_offset;
        let offset = domain_start - scale * range_start;

        let clamp = config.clamp;

        if clamp {
            let (range_min, range_max) = if range_start <= range_end {
                (range_start, range_end)
            } else {
                (range_end, range_start)
            };

            Ok(ScalarOrArray::new_array(
                values
                    .iter()
                    .map(|v| {
                        let v = (v - range_offset).clamp(range_min, range_max);
                        scale * v + offset
                    })
                    .collect(),
            ))
        } else {
            Ok(ScalarOrArray::new_array(
                values
                    .iter()
                    .map(|v| scale * (v - range_offset) + offset)
                    .collect(),
            ))
        }
    }

    /// Nice scale domain
    fn nice(
        &self,
        mut config: NumericScaleConfig,
        count: Option<usize>,
    ) -> Result<NumericScaleConfig, AvengerScaleError> {
        let (domain_start, domain_end) = config.domain;

        if domain_start == domain_end || domain_start.is_nan() || domain_end.is_nan() {
            return Ok(config);
        }

        let (mut start, mut stop) = if domain_start <= domain_end {
            (domain_start, domain_end)
        } else {
            (domain_end, domain_start)
        };

        let mut prestep = 0.0;
        let mut max_iter = 10;

        let count = count.unwrap_or(10);
        while max_iter > 0 {
            let step = array::tick_increment(start as f32, stop as f32, count as f32);

            if step == prestep {
                if domain_start <= domain_end {
                    config.domain = (start, stop);
                } else {
                    config.domain = (stop, start);
                }
                return Ok(config);
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
            config.domain = (start, stop);
        } else {
            config.domain = (stop, start);
        }

        Ok(config)
    }

    /// Compute ticks for a scale with numeric domain
    /// Ticks are in the domain space of the scale
    fn ticks(
        &self,
        config: NumericScaleConfig,
        count: Option<f32>,
    ) -> Result<Vec<f32>, AvengerScaleError> {
        let count = count.unwrap_or(10.0);
        Ok(array::ticks(
            config.domain.0 as f32,
            config.domain.1 as f32,
            count,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use float_cmp::assert_approx_eq;

    #[test]
    fn test_defaults() {
        let config = NumericScaleConfig::default();
        assert_eq!(config.domain, (0.0, 1.0));
        assert_eq!(config.range, (0.0, 1.0));
        assert!(!config.clamp);
    }

    #[test]
    fn test_scale() -> Result<(), AvengerScaleError> {
        // Test scaling with edge cases: out-of-bounds, nulls, and interpolation
        let config = NumericScaleConfig {
            domain: (10.0, 30.0),
            range: (0.0, 100.0),
            clamp: true,
            ..Default::default()
        };

        let scale = LinearNumericScale;
        let values = vec![
            0.0,  // < domain
            10.0, // domain start
            15.0, 20.0, 25.0, 30.0, // in domain
            40.0, // > domain
        ];

        let result = scale.scale(&config, &values)?.as_vec(values.len(), None);

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
        let config = NumericScaleConfig {
            domain: (10.0, 30.0),
            range: (0.0, 100.0),
            range_offset: 3.0,
            clamp: true,
            ..Default::default()
        };

        let scale = LinearNumericScale;
        let values = vec![
            0.0,  // < domain
            10.0, // domain start
            15.0, 20.0, 25.0, 30.0, // in domain
            40.0, // > domain
        ];

        let result = scale.scale(&config, &values)?.as_vec(values.len(), None);

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
        let config = NumericScaleConfig {
            domain: (10.0, 10.0),
            range: (0.0, 100.0),
            clamp: true,
            ..Default::default()
        };

        let scale = LinearNumericScale;
        let values = vec![0.0, 10.0, 20.0];
        let result = scale.scale(&config, &values)?.as_vec(values.len(), None);

        // All values should map to range_start (d3 behavior)
        for i in 0..result.len() {
            assert_approx_eq!(f32, result[i], 0.0);
        }

        Ok(())
    }

    #[test]
    fn test_degenerate_cases() -> Result<(), AvengerScaleError> {
        let scale = LinearNumericScale;

        // Test degenerate domain
        let config = NumericScaleConfig {
            domain: (1.0, 1.0),
            range: (0.0, 0.0),
            clamp: false,
            ..Default::default()
        };

        let values = vec![0.0, 1.0, 2.0];
        let result = scale.scale(&config, &values)?.as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 0.0);
        assert_approx_eq!(f32, result[1], 0.0);
        assert_approx_eq!(f32, result[2], 0.0);

        // Test degenerate range
        let config = NumericScaleConfig {
            domain: (0.0, 10.0),
            range: (1.0, 1.0),
            clamp: false,
            ..Default::default()
        };

        let result = scale.scale(&config, &values)?.as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 1.0);
        assert_approx_eq!(f32, result[1], 1.0);
        assert_approx_eq!(f32, result[2], 1.0);

        Ok(())
    }

    #[test]
    fn test_invert_clamped() -> Result<(), AvengerScaleError> {
        let config = NumericScaleConfig {
            domain: (10.0, 30.0),
            range: (0.0, 100.0),
            clamp: true,
            ..Default::default()
        };

        let scale = LinearNumericScale;
        let values = vec![-25.0, 0.0, 50.0, 100.0, 125.0];
        let result = scale.invert(&config, &values)?.as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 10.0); // clamped below
        assert_approx_eq!(f32, result[1], 10.0); // range start
        assert_approx_eq!(f32, result[2], 20.0); // interpolated
        assert_approx_eq!(f32, result[3], 30.0); // range end
        assert_approx_eq!(f32, result[4], 30.0); // clamped above

        Ok(())
    }

    #[test]
    fn test_invert_unclamped() -> Result<(), AvengerScaleError> {
        let config = NumericScaleConfig {
            domain: (10.0, 30.0),
            range: (0.0, 100.0),
            clamp: false,
            ..Default::default()
        };

        let scale = LinearNumericScale;
        let values = vec![-25.0, 0.0, 50.0, 100.0, 125.0];
        let result = scale.invert(&config, &values)?.as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 5.0); // below range
        assert_approx_eq!(f32, result[1], 10.0); // range start
        assert_approx_eq!(f32, result[2], 20.0); // interpolated
        assert_approx_eq!(f32, result[3], 30.0); // range end
        assert_approx_eq!(f32, result[4], 35.0); // above range

        Ok(())
    }

    #[test]
    fn test_invert_with_range_offset() -> Result<(), AvengerScaleError> {
        let config = NumericScaleConfig {
            domain: (10.0, 30.0),
            range: (0.0, 100.0),
            range_offset: 3.0,
            clamp: false,
            ..Default::default()
        };

        let scale = LinearNumericScale;
        let values = vec![-22.0, 3.0, 53.0, 103.0, 128.0];
        let result = scale.invert(&config, &values)?.as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 5.0); // below range
        assert_approx_eq!(f32, result[1], 10.0); // range start
        assert_approx_eq!(f32, result[2], 20.0); // interpolated
        assert_approx_eq!(f32, result[3], 30.0); // range end
        assert_approx_eq!(f32, result[4], 35.0); // above range

        Ok(())
    }

    #[test]
    fn test_invert_reversed_range() -> Result<(), AvengerScaleError> {
        let config = NumericScaleConfig {
            domain: (10.0, 30.0),
            range: (100.0, 0.0),
            clamp: true,
            ..Default::default()
        };

        let scale = LinearNumericScale;
        let values = vec![125.0, 100.0, 50.0, 0.0, -25.0];
        let result = scale.invert(&config, &values)?.as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 10.0); // clamped
        assert_approx_eq!(f32, result[1], 10.0); // range start
        assert_approx_eq!(f32, result[2], 20.0); // interpolated
        assert_approx_eq!(f32, result[3], 30.0); // range end
        assert_approx_eq!(f32, result[4], 30.0); // clamped

        Ok(())
    }

    #[test]
    fn test_ticks() -> Result<(), AvengerScaleError> {
        let config = NumericScaleConfig {
            domain: (0.0, 10.0),
            range: (0.0, 100.0),
            ..Default::default()
        };

        let scale = LinearNumericScale;
        assert_eq!(
            scale.ticks(config.clone(), Some(5.0))?,
            vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0]
        );
        assert_eq!(
            scale.ticks(config.clone(), Some(2.0))?,
            vec![0.0, 5.0, 10.0]
        );
        assert_eq!(scale.ticks(config, Some(1.0))?, vec![0.0, 10.0]);

        Ok(())
    }

    #[test]
    fn test_ticks_span_zero() -> Result<(), AvengerScaleError> {
        let config = NumericScaleConfig {
            domain: (-100.0, 100.0),
            ..Default::default()
        };

        let scale = LinearNumericScale;
        assert_eq!(
            scale.ticks(config.clone(), Some(10.0))?,
            vec![-100.0, -80.0, -60.0, -40.0, -20.0, 0.0, 20.0, 40.0, 60.0, 80.0, 100.0]
        );
        assert_eq!(
            scale.ticks(config.clone(), Some(5.0))?,
            vec![-100.0, -50.0, 0.0, 50.0, 100.0]
        );
        assert_eq!(
            scale.ticks(config.clone(), Some(2.0))?,
            vec![-100.0, 0.0, 100.0]
        );
        assert_eq!(scale.ticks(config, Some(1.0))?, vec![0.0]);

        Ok(())
    }

    #[test]
    fn test_nice_convergence() -> Result<(), AvengerScaleError> {
        let config = NumericScaleConfig {
            domain: (1.1, 10.9),
            ..Default::default()
        };

        let scale = LinearNumericScale;
        let niced_config = scale.nice(config, Some(10))?;
        assert_eq!(niced_config.domain, (1.0, 11.0));

        Ok(())
    }

    #[test]
    fn test_nice_negative_step() -> Result<(), AvengerScaleError> {
        let config = NumericScaleConfig {
            domain: (-1.1, -10.9),
            ..Default::default()
        };

        let scale = LinearNumericScale;
        let niced_config = scale.nice(config, Some(10))?;
        assert_eq!(niced_config.domain, (-1.0, -11.0));

        Ok(())
    }
}
