use avenger_common::value::ScalarOrArray;

use crate::{
    array,
    config::ScaleConfigScalarMapUtils,
    error::AvengerScaleError,
    numeric::{NumericScale, NumericScaleConfig},
};

use super::linear::LinearNumericScale;

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

#[derive(Debug, Clone)]
pub struct PowerNumericScale;

impl NumericScale for PowerNumericScale {
    fn scale(
        &self,
        config: &NumericScaleConfig,
        values: &[f32],
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        // If range start equals end, return constant range value
        if config.range.0 == config.range.1 {
            return Ok(ScalarOrArray::new_array(vec![config.range.0; values.len()]));
        }

        // Get the exponent and build the power function
        let exponent = config.options.try_get_f32("exponent").unwrap_or(1.0);
        let power_fun = PowerFunction::new(exponent);

        let d0 = power_fun.pow(config.domain.0);
        let d1 = power_fun.pow(config.domain.1);

        // If domain start equals end, return constant domain value
        if d0 == d1 {
            return Ok(ScalarOrArray::new_array(vec![
                config.domain.0;
                values.len()
            ]));
        }

        // At this point, we know (d1 - d0) cannot be zero
        let scale = (config.range.1 - config.range.0) / (d1 - d0);
        let offset = config.range.0 - scale * power_fun.pow(config.domain.0) + config.range_offset;

        let (range_min, range_max) = if config.range.0 <= config.range.1 {
            (config.range.0, config.range.1)
        } else {
            (config.range.1, config.range.0)
        };

        match (config.clamp, config.round) {
            (true, false) => match &power_fun {
                // Clamp, no rounding
                PowerFunction::Static { pow_fun, .. } => Ok(ScalarOrArray::new_array(
                    values
                        .iter()
                        .map(|&v| {
                            let abs_v = v.abs();
                            let sign = if v < 0.0 { -1.0 } else { 1.0 };
                            (scale * (sign * pow_fun(abs_v)) + offset).clamp(range_min, range_max)
                        })
                        .collect(),
                )),
                PowerFunction::Custom { exponent } => {
                    let exponent = *exponent;
                    Ok(ScalarOrArray::new_array(
                        values
                            .iter()
                            .map(|&v| {
                                let abs_v = v.abs();
                                let sign = if v < 0.0 { -1.0 } else { 1.0 };
                                (scale * (sign * abs_v.powf(exponent)) + offset)
                                    .clamp(range_min, range_max)
                            })
                            .collect(),
                    ))
                }
            },
            (true, true) => match &power_fun {
                // Clamp and rounding
                PowerFunction::Static { pow_fun, .. } => Ok(ScalarOrArray::new_array(
                    values
                        .iter()
                        .map(|&v| {
                            let abs_v = v.abs();
                            let sign = if v < 0.0 { -1.0 } else { 1.0 };
                            (scale * (sign * pow_fun(abs_v)) + offset)
                                .clamp(range_min, range_max)
                                .round()
                        })
                        .collect(),
                )),
                PowerFunction::Custom { exponent } => {
                    let exponent = *exponent;
                    Ok(ScalarOrArray::new_array(
                        values
                            .iter()
                            .map(|&v| {
                                let abs_v = v.abs();
                                let sign = if v < 0.0 { -1.0 } else { 1.0 };
                                (scale * (sign * abs_v.powf(exponent)) + offset)
                                    .clamp(range_min, range_max)
                                    .round()
                            })
                            .collect(),
                    ))
                }
            },
            (false, false) => match &power_fun {
                // no clamping or rounding
                PowerFunction::Static { pow_fun, .. } => Ok(ScalarOrArray::new_array(
                    values
                        .iter()
                        .map(|&v| {
                            let abs_v = v.abs();
                            let sign = if v < 0.0 { -1.0 } else { 1.0 };
                            scale * (sign * pow_fun(abs_v)) + offset
                        })
                        .collect(),
                )),
                PowerFunction::Custom { exponent } => {
                    let exponent = *exponent;
                    Ok(ScalarOrArray::new_array(
                        values
                            .iter()
                            .map(|&v| {
                                let abs_v = v.abs();
                                let sign = if v < 0.0 { -1.0 } else { 1.0 };
                                scale * (sign * abs_v.powf(exponent)) + offset
                            })
                            .collect(),
                    ))
                }
            },
            (false, true) => match &power_fun {
                // no clamping and rounding
                PowerFunction::Static { pow_fun, .. } => Ok(ScalarOrArray::new_array(
                    values
                        .iter()
                        .map(|&v| {
                            let abs_v = v.abs();
                            let sign = if v < 0.0 { -1.0 } else { 1.0 };
                            (scale * (sign * pow_fun(abs_v)) + offset).round()
                        })
                        .collect(),
                )),
                PowerFunction::Custom { exponent } => {
                    let exponent = *exponent;
                    Ok(ScalarOrArray::new_array(
                        values
                            .iter()
                            .map(|&v| {
                                let abs_v = v.abs();
                                let sign = if v < 0.0 { -1.0 } else { 1.0 };
                                (scale * (sign * abs_v.powf(exponent)) + offset).round()
                            })
                            .collect(),
                    ))
                }
            },
        }
    }

    /// Invert numeric values from continuous range to continuous domain
    fn invert(
        &self,
        config: &NumericScaleConfig,
        values: &[f32],
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        let exponent = f32::try_from(
            config
                .options
                .get("exponent")
                .cloned()
                .unwrap_or(1.0.into()),
        )?;

        let power_fun = PowerFunction::new(exponent);
        let d0 = power_fun.pow(config.domain.0);
        let d1 = power_fun.pow(config.domain.1);

        // If domain start equals end, return constant
        if d0 == d1 {
            return Ok(ScalarOrArray::new_array(vec![
                config.domain.0;
                values.len()
            ]));
        }

        let scale = (config.range.1 - config.range.0) / (d1 - d0);
        let range_offset = config.range_offset;
        let offset = config.range.0 - scale * d0;

        if config.clamp {
            let (range_min, range_max) = if config.range.0 <= config.range.1 {
                (config.range.0, config.range.1)
            } else {
                (config.range.1, config.range.0)
            };

            match &power_fun {
                PowerFunction::Static { pow_inv_fun, .. } => Ok(ScalarOrArray::new_array(
                    values
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
                        values
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
                    values
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
                        values
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

    /// Nice scale domain
    fn nice(
        &self,
        mut config: NumericScaleConfig,
        count: Option<usize>,
    ) -> Result<NumericScaleConfig, AvengerScaleError> {
        // Transform domain to linear space using power function
        let exponent = config.options.try_get_f32("exponent").unwrap_or(1.0);
        let power_fun = PowerFunction::new(exponent);
        let d0 = power_fun.pow(config.domain.0);
        let d1 = power_fun.pow(config.domain.1);

        // Use linear scale to nice the transformed values
        let linear = LinearNumericScale;
        let linear_config = NumericScaleConfig {
            domain: (d0, d1),
            ..Default::default()
        };
        let niced_linear_config = linear.nice(linear_config, count)?;

        // Update config by transforming back to original domain
        let (nice_d0, nice_d1) = niced_linear_config.domain;
        config.domain.0 = power_fun.pow_inv(nice_d0);
        config.domain.1 = power_fun.pow_inv(nice_d1);
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
    use std::collections::HashMap;

    #[test]
    fn test_square() -> Result<(), AvengerScaleError> {
        let scale = PowerNumericScale;

        let config = NumericScaleConfig {
            options: HashMap::from([("exponent".to_string(), 2.0.into())]),
            ..Default::default()
        };

        let values = vec![2.0, -2.0];
        let result = scale.scale(&config, &values)?.as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 4.0);
        assert_approx_eq!(f32, result[1], -4.0);

        Ok(())
    }

    #[test]
    fn test_sqrt() -> Result<(), AvengerScaleError> {
        let scale = PowerNumericScale;

        let config = NumericScaleConfig {
            options: HashMap::from([("exponent".to_string(), 0.5.into())]),
            ..Default::default()
        };

        let values = vec![4.0, -4.0];
        let result = scale.scale(&config, &values)?.as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 2.0);
        assert_approx_eq!(f32, result[1], -2.0);

        Ok(())
    }

    #[test]
    fn test_sqrt_range_offset() -> Result<(), AvengerScaleError> {
        let scale = PowerNumericScale;

        let config = NumericScaleConfig {
            options: HashMap::from([("exponent".to_string(), 0.5.into())]),
            range_offset: 1.0,
            ..Default::default()
        };

        let values = vec![4.0, -4.0];
        let result = scale.scale(&config, &values)?.as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 3.0);
        assert_approx_eq!(f32, result[1], -1.0);

        Ok(())
    }

    #[test]
    fn test_custom_exponent() -> Result<(), AvengerScaleError> {
        let scale = PowerNumericScale;

        let config = NumericScaleConfig {
            options: HashMap::from([("exponent".to_string(), 3.0.into())]),
            ..Default::default()
        };

        let values = vec![2.0, -2.0];
        let result = scale.scale(&config, &values)?.as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 8.0);
        assert_approx_eq!(f32, result[1], -8.0);

        Ok(())
    }

    #[test]
    fn test_custom_exponent_range_offset() -> Result<(), AvengerScaleError> {
        let scale = PowerNumericScale;

        let config = NumericScaleConfig {
            options: HashMap::from([("exponent".to_string(), 3.0.into())]),
            range_offset: 1.0,
            ..Default::default()
        };

        let values = vec![2.0, -2.0];
        let result = scale.scale(&config, &values)?.as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 9.0);
        assert_approx_eq!(f32, result[1], -7.0);

        Ok(())
    }

    #[test]
    fn test_domain_coercion() -> Result<(), AvengerScaleError> {
        let scale = PowerNumericScale;

        let config = NumericScaleConfig {
            domain: (-1.0, 2.0),
            options: HashMap::from([("exponent".to_string(), 2.0.into())]),
            ..Default::default()
        };

        let values = vec![-1.0, 0.0, 1.0, 2.0];
        let result = scale.scale(&config, &values)?.as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 0.0);
        assert_approx_eq!(f32, result[1], 0.2);
        assert_approx_eq!(f32, result[2], 0.4);
        assert_approx_eq!(f32, result[3], 1.0);

        Ok(())
    }

    #[test]
    fn test_clamping() -> Result<(), AvengerScaleError> {
        let scale = PowerNumericScale;

        let config = NumericScaleConfig {
            options: HashMap::from([("exponent".to_string(), 2.0.into())]),
            clamp: true,
            ..Default::default()
        };

        let values = vec![-0.5, 0.0, 0.5, 1.0, 1.5];
        let result = scale.scale(&config, &values)?.as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 0.0);
        assert_approx_eq!(f32, result[1], 0.0);
        assert_approx_eq!(f32, result[2], 0.25);
        assert_approx_eq!(f32, result[3], 1.0);
        assert_approx_eq!(f32, result[4], 1.0);

        Ok(())
    }

    #[test]
    fn test_invert_exp_1() -> Result<(), AvengerScaleError> {
        let scale = PowerNumericScale;

        let config = NumericScaleConfig {
            domain: (1.0, 2.0),
            range: (0.0, 1.0),
            ..Default::default()
        };

        let values = vec![-0.5, 0.0, 0.5, 1.0, 1.5];
        let result = scale.invert(&config, &values)?.as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 0.5);
        assert_approx_eq!(f32, result[1], 1.0);
        assert_approx_eq!(f32, result[2], 1.5);
        assert_approx_eq!(f32, result[3], 2.0);
        assert_approx_eq!(f32, result[4], 2.5);

        Ok(())
    }

    #[test]
    fn test_invert_exp_1_range_offset() -> Result<(), AvengerScaleError> {
        let scale = PowerNumericScale;

        let config = NumericScaleConfig {
            domain: (1.0, 2.0),
            range: (0.0, 1.0),
            range_offset: 1.0,
            ..Default::default()
        };

        let values = vec![0.5, 1.0, 1.5, 2.0, 2.5];
        let result = scale.invert(&config, &values)?.as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 0.5);
        assert_approx_eq!(f32, result[1], 1.0);
        assert_approx_eq!(f32, result[2], 1.5);
        assert_approx_eq!(f32, result[3], 2.0);
        assert_approx_eq!(f32, result[4], 2.5);

        Ok(())
    }

    #[test]
    fn test_negative_numbers_exp_2() -> Result<(), AvengerScaleError> {
        let scale = PowerNumericScale;

        let config = NumericScaleConfig {
            options: HashMap::from([("exponent".to_string(), 2.0.into())]),
            ..Default::default()
        };

        let values = vec![-2.0, -1.0, 0.0, 1.0, 2.0];
        let result = scale.scale(&config, &values)?.as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], -4.0);
        assert_approx_eq!(f32, result[1], -1.0);
        assert_approx_eq!(f32, result[2], 0.0);
        assert_approx_eq!(f32, result[3], 1.0);
        assert_approx_eq!(f32, result[4], 4.0);

        Ok(())
    }

    #[test]
    fn test_negative_domain_exp_1() -> Result<(), AvengerScaleError> {
        let scale = PowerNumericScale;

        let config = NumericScaleConfig {
            domain: (-2.0, 2.0),
            range: (0.0, 1.0),
            ..Default::default()
        };

        let values = vec![-2.0, -1.0, 0.0, 1.0, 2.0];
        let result = scale.scale(&config, &values)?.as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 0.0);
        assert_approx_eq!(f32, result[1], 0.25);
        assert_approx_eq!(f32, result[2], 0.5);
        assert_approx_eq!(f32, result[3], 0.75);
        assert_approx_eq!(f32, result[4], 1.0);

        Ok(())
    }

    #[test]
    fn test_invert_negative_exp_2() -> Result<(), AvengerScaleError> {
        let scale = PowerNumericScale;

        let config = NumericScaleConfig {
            domain: (-2.0, 2.0),
            range: (-4.0, 4.0),
            options: HashMap::from([("exponent".to_string(), 2.0.into())]),
            ..Default::default()
        };

        let values = vec![-4.0, -2.0, 0.0, 2.0, 4.0];
        let result = scale.invert(&config, &values)?.as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], -2.0);
        assert_approx_eq!(f32, result[1], -1.414214);
        assert_approx_eq!(f32, result[2], 0.0);
        assert_approx_eq!(f32, result[3], 1.414214);
        assert_approx_eq!(f32, result[4], 2.0);

        Ok(())
    }

    #[test]
    fn test_nice() -> Result<(), AvengerScaleError> {
        // Test with exponent 2 (square)
        let scale = PowerNumericScale;

        let config = NumericScaleConfig {
            domain: (-10.0, 10.0),
            options: HashMap::from([("exponent".to_string(), 2.0.into())]),
            ..Default::default()
        };

        let result = scale.nice(config, None)?;
        let (d0, d1) = result.domain;
        assert_approx_eq!(f32, d0, -10.0);
        assert_approx_eq!(f32, d1, 10.0);

        // Test with exponent 0.5 (sqrt)
        let scale = PowerNumericScale;

        let config = NumericScaleConfig {
            domain: (0.1, 0.9),
            options: HashMap::from([("exponent".to_string(), 3.0.into())]),
            ..Default::default()
        };

        let result = scale.nice(config.clone(), None)?;

        let power_fun = PowerFunction::new(config.options.try_get_f32("exponent").unwrap_or(1.0));

        let (d0, d1) = result.domain;
        assert_approx_eq!(f32, d0, power_fun.pow_inv(0.0));
        assert_approx_eq!(f32, d1, power_fun.pow_inv(0.8));

        // Test with custom exponent
        let scale = PowerNumericScale;

        let config = NumericScaleConfig {
            domain: (0.1, 0.9),
            options: HashMap::from([("exponent".to_string(), 3.0.into())]),
            ..Default::default()
        };

        let result = scale.nice(config.clone(), None)?;
        let (d0, d1) = result.domain;
        assert_approx_eq!(f32, d0, power_fun.pow_inv(0.0));
        assert_approx_eq!(f32, d1, power_fun.pow_inv(0.8));

        Ok(())
    }
}
