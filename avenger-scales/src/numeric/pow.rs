use avenger_common::value::{ScalarOrArray, ScalarOrArrayRef};

use crate::error::AvengerScaleError;
use std::sync::Arc;

use super::{linear::LinearNumericScale, opts::NumericScaleOptions};

/// Handles power transformations with different exponents
#[derive(Clone, Debug)]
enum PowerFunction {
    Static {
        pow_fun: fn(f32) -> f32,
        pow_inv_fun: fn(f32) -> f32,
        exponent: f32,
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
                exponent,
            }
        } else if exponent == 0.5 {
            PowerFunction::Static {
                pow_fun: f32::sqrt,
                pow_inv_fun: |x| x * x,
                exponent,
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

    /// Returns the current exponent
    pub fn exponent(&self) -> f32 {
        match self {
            PowerFunction::Static { exponent, .. } => *exponent,
            PowerFunction::Custom { exponent } => *exponent,
        }
    }
}

/// A power scale that maps numeric input values using a power transform.
/// Supports different exponents, clamping, domain niceing, and tick generation.
#[derive(Clone, Debug)]
pub struct PowNumericScale {
    domain_start: f32,
    domain_end: f32,
    range_start: f32,
    range_end: f32,
    clamp: bool,
    power_fun: Arc<PowerFunction>,
}

impl PowNumericScale {
    /// Creates a new power scale with default domain [0, 1], range [0, 1], and exponent 1
    pub fn new() -> Self {
        Self {
            domain_start: 0.0,
            domain_end: 1.0,
            range_start: 0.0,
            range_end: 1.0,
            clamp: false,
            power_fun: Arc::new(PowerFunction::new(1.0)),
        }
    }

    /// Sets the power exponent
    pub fn exponent(mut self, exp: f32) -> Self {
        self.power_fun = Arc::new(PowerFunction::new(exp));
        self
    }

    /// Returns the current exponent
    pub fn get_exponent(&self) -> f32 {
        self.power_fun.exponent()
    }

    /// Sets the input domain of the scale
    pub fn domain(mut self, (start, end): (f32, f32)) -> Self {
        self.domain_start = start;
        self.domain_end = end;
        self
    }

    /// Returns the current domain as (start, end)
    pub fn get_domain(&self) -> (f32, f32) {
        (self.domain_start, self.domain_end)
    }

    /// Sets the output range of the scale
    pub fn range(mut self, (start, end): (f32, f32)) -> Self {
        self.range_start = start;
        self.range_end = end;
        self
    }

    /// Returns the current range as (start, end)
    pub fn get_range(&self) -> (f32, f32) {
        (self.range_start, self.range_end)
    }

    /// Enables or disables clamping of output values to the range
    pub fn clamp(mut self, clamp: bool) -> Self {
        self.clamp = clamp;
        self
    }

    /// Returns whether output clamping is enabled
    pub fn get_clamp(&self) -> bool {
        self.clamp
    }

    /// Applies the power transform to a single value
    pub fn transform(&self, x: f32) -> f32 {
        self.power_fun.pow(x)
    }

    /// Applies the inverse power transform to a single value
    pub fn transform_inv(&self, x: f32) -> f32 {
        self.power_fun.pow_inv(x)
    }

    /// Maps input values from domain to range using power transform
    pub fn scale<'a>(
        &self,
        values: impl Into<ScalarOrArrayRef<'a, f32>>,
        opts: &NumericScaleOptions,
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        // If range start equals end, return constant range value
        if self.range_start == self.range_end {
            return Ok(values.into().map(|_| self.range_start));
        }

        let d0 = self.power_fun.pow(self.domain_start);
        let d1 = self.power_fun.pow(self.domain_end);

        // If domain start equals end, return constant domain value
        if d0 == d1 {
            return Ok(values.into().map(|_| self.domain_start));
        }

        // At this point, we know (d1 - d0) cannot be zero
        let scale = (self.range_end - self.range_start) / (d1 - d0);
        let range_offset = opts.range_offset.unwrap_or(0.0);
        let offset =
            self.range_start - scale * self.power_fun.pow(self.domain_start) + range_offset;

        if self.clamp {
            let (range_min, range_max) = if self.range_start <= self.range_end {
                (self.range_start, self.range_end)
            } else {
                (self.range_end, self.range_start)
            };

            match self.power_fun.as_ref() {
                PowerFunction::Static { pow_fun, .. } => Ok(values.into().map(|&v| {
                    let abs_v = v.abs();
                    let sign = if v < 0.0 { -1.0 } else { 1.0 };
                    (scale * (sign * pow_fun(abs_v)) + offset).clamp(range_min, range_max)
                })),
                PowerFunction::Custom { exponent } => {
                    let exponent = *exponent;
                    Ok(values.into().map(|&v| {
                        let abs_v = v.abs();
                        let sign = if v < 0.0 { -1.0 } else { 1.0 };
                        (scale * (sign * abs_v.powf(exponent)) + offset).clamp(range_min, range_max)
                    }))
                }
            }
        } else {
            match self.power_fun.as_ref() {
                PowerFunction::Static { pow_fun, .. } => Ok(values.into().map(|&v| {
                    let abs_v = v.abs();
                    let sign = if v < 0.0 { -1.0 } else { 1.0 };
                    scale * (sign * pow_fun(abs_v)) + offset
                })),
                PowerFunction::Custom { exponent } => {
                    let exponent = *exponent;
                    Ok(values.into().map(|&v| {
                        let abs_v = v.abs();
                        let sign = if v < 0.0 { -1.0 } else { 1.0 };
                        scale * (sign * abs_v.powf(exponent)) + offset
                    }))
                }
            }
        }
    }

    /// Maps output values from range back to domain using inverse power transform
    pub fn invert<'a>(
        &self,
        values: impl Into<ScalarOrArrayRef<'a, f32>>,
        opts: &NumericScaleOptions,
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        let d0 = self.power_fun.pow(self.domain_start);
        let d1 = self.power_fun.pow(self.domain_end);

        // If domain start equals end, return constant
        if d0 == d1 {
            return Ok(values.into().map(|_| self.domain_start));
        }

        let scale = (self.range_end - self.range_start) / (d1 - d0);
        let range_offset = opts.range_offset.unwrap_or(0.0);
        let offset = self.range_start - scale * d0;

        if self.clamp {
            let (range_min, range_max) = if self.range_start <= self.range_end {
                (self.range_start, self.range_end)
            } else {
                (self.range_end, self.range_start)
            };

            match self.power_fun.as_ref() {
                PowerFunction::Static { pow_inv_fun, .. } => Ok(values.into().map(|&v| {
                    let v = v.clamp(range_min, range_max);
                    let normalized = (v - offset) / scale;
                    let abs_norm = normalized.abs();
                    let sign = if normalized < 0.0 { -1.0 } else { 1.0 };
                    sign * pow_inv_fun(abs_norm) - range_offset
                })),
                PowerFunction::Custom { exponent } => {
                    let inv_exponent = 1.0 / exponent;
                    Ok(values.into().map(|&v| {
                        let v = v.clamp(range_min, range_max);
                        let normalized = (v - offset) / scale;
                        let abs_norm = normalized.abs();
                        let sign = if normalized < 0.0 { -1.0 } else { 1.0 };
                        sign * abs_norm.powf(inv_exponent) - range_offset
                    }))
                }
            }
        } else {
            match self.power_fun.as_ref() {
                PowerFunction::Static { pow_inv_fun, .. } => Ok(values.into().map(|&v| {
                    let normalized = (v - offset) / scale;
                    let abs_norm = normalized.abs();
                    let sign = if normalized < 0.0 { -1.0 } else { 1.0 };
                    sign * pow_inv_fun(abs_norm) - range_offset
                })),
                PowerFunction::Custom { exponent } => {
                    let inv_exponent = 1.0 / exponent;
                    Ok(values.into().map(|&v| {
                        let normalized = (v - offset) / scale;
                        let abs_norm = normalized.abs();
                        let sign = if normalized < 0.0 { -1.0 } else { 1.0 };
                        sign * abs_norm.powf(inv_exponent) - range_offset
                    }))
                }
            }
        }
    }

    /// Extends the domain to nice round numbers in transformed space
    pub fn nice(mut self, count: Option<usize>) -> Self {
        // Transform domain to linear space using power function
        let d0 = self.power_fun.pow(self.domain_start);
        let d1 = self.power_fun.pow(self.domain_end);

        // Use linear scale to nice the transformed values
        let linear = LinearNumericScale::new().domain((d0, d1)).nice(count);

        let (nice_d0, nice_d1) = linear.get_domain();
        self.domain_start = self.transform_inv(nice_d0);
        self.domain_end = self.transform_inv(nice_d1);
        self
    }

    /// Generates evenly spaced tick values within the domain
    pub fn ticks(&self, count: Option<f32>) -> Vec<f32> {
        // Transform domain to log space
        let d0 = self.transform(self.domain_start);
        let d1 = self.transform(self.domain_end);

        // Use linear scale to generate ticks in transformed space
        let linear = LinearNumericScale::new().domain((d0, d1));

        let log_ticks = linear.ticks(count);

        // Transform ticks back to original space
        log_ticks.iter().map(|&x| self.transform_inv(x)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use float_cmp::assert_approx_eq;

    #[test]
    fn test_defaults() {
        let scale = PowNumericScale::new();
        assert_eq!(scale.domain_start, 0.0);
        assert_eq!(scale.domain_end, 1.0);
        assert_eq!(scale.range_start, 0.0);
        assert_eq!(scale.range_end, 1.0);
        assert_eq!(scale.clamp, false);
        assert_eq!(scale.get_exponent(), 1.0);
    }

    #[test]
    fn test_square() {
        let scale = PowNumericScale::new().exponent(2.0);
        let values = vec![2.0, -2.0];
        let result = scale
            .scale(&values, &Default::default())
            .unwrap()
            .as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 4.0);
        assert_approx_eq!(f32, result[1], -4.0);
    }

    #[test]
    fn test_sqrt() {
        let scale = PowNumericScale::new().exponent(0.5);
        let values = vec![4.0, -4.0];
        let result = scale
            .scale(&values, &Default::default())
            .unwrap()
            .as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 2.0);
        assert_approx_eq!(f32, result[1], -2.0);
    }

    #[test]
    fn test_sqrt_range_offset() {
        let scale = PowNumericScale::new().exponent(0.5);
        let values = vec![4.0, -4.0];
        let result = scale
            .scale(
                &values,
                &NumericScaleOptions {
                    range_offset: Some(1.0),
                },
            )
            .unwrap()
            .as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 3.0);
        assert_approx_eq!(f32, result[1], -1.0);
    }

    #[test]
    fn test_custom_exponent() {
        let scale = PowNumericScale::new().exponent(3.0);
        let values = vec![2.0, -2.0];
        let result = scale
            .scale(&values, &Default::default())
            .unwrap()
            .as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 8.0);
        assert_approx_eq!(f32, result[1], -8.0);
    }

    #[test]
    fn test_custom_exponent_range_offset() {
        let scale = PowNumericScale::new().exponent(3.0);
        let values = vec![2.0, -2.0];
        let result = scale
            .scale(
                &values,
                &NumericScaleOptions {
                    range_offset: Some(1.0),
                },
            )
            .unwrap()
            .as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 9.0);
        assert_approx_eq!(f32, result[1], -7.0);
    }

    #[test]
    fn test_domain_coercion() {
        let scale = PowNumericScale::new().domain((-1.0, 2.0)).exponent(2.0);
        let values = vec![-1.0, 0.0, 1.0, 2.0];
        let result = scale
            .scale(&values, &Default::default())
            .unwrap()
            .as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 0.0);
        assert_approx_eq!(f32, result[1], 0.2);
        assert_approx_eq!(f32, result[2], 0.4);
        assert_approx_eq!(f32, result[3], 1.0);
    }

    #[test]
    fn test_clamping() {
        let scale = PowNumericScale::new()
            .domain((0.0, 1.0))
            .clamp(true)
            .exponent(2.0);
        let values = vec![-0.5, 0.0, 0.5, 1.0, 1.5];
        let result = scale
            .scale(&values, &Default::default())
            .unwrap()
            .as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 0.0);
        assert_approx_eq!(f32, result[1], 0.0);
        assert_approx_eq!(f32, result[2], 0.25);
        assert_approx_eq!(f32, result[3], 1.0);
        assert_approx_eq!(f32, result[4], 1.0);
    }

    #[test]
    fn test_invert_exp_1() {
        let scale = PowNumericScale::new().domain((1.0, 2.0)).range((0.0, 1.0));
        let values = vec![-0.5, 0.0, 0.5, 1.0, 1.5];
        let result = scale
            .invert(&values, &Default::default())
            .unwrap()
            .as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 0.5);
        assert_approx_eq!(f32, result[1], 1.0);
        assert_approx_eq!(f32, result[2], 1.5);
        assert_approx_eq!(f32, result[3], 2.0);
        assert_approx_eq!(f32, result[4], 2.5);
    }

    #[test]
    fn test_invert_exp_1_range_offset() {
        let scale = PowNumericScale::new().domain((1.0, 2.0)).range((0.0, 1.0));
        let values = vec![0.5, 1.0, 1.5, 2.0, 2.5];
        let result = scale
            .invert(
                &values,
                &NumericScaleOptions {
                    range_offset: Some(1.0),
                },
            )
            .unwrap()
            .as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 0.5);
        assert_approx_eq!(f32, result[1], 1.0);
        assert_approx_eq!(f32, result[2], 1.5);
        assert_approx_eq!(f32, result[3], 2.0);
        assert_approx_eq!(f32, result[4], 2.5);
    }

    #[test]
    fn test_negative_numbers_exp_2() {
        let scale = PowNumericScale::new().exponent(2.0);
        let values = vec![-2.0, -1.0, 0.0, 1.0, 2.0];
        let result = scale
            .scale(&values, &Default::default())
            .unwrap()
            .as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], -4.0);
        assert_approx_eq!(f32, result[1], -1.0);
        assert_approx_eq!(f32, result[2], 0.0);
        assert_approx_eq!(f32, result[3], 1.0);
        assert_approx_eq!(f32, result[4], 4.0);
    }

    #[test]
    fn test_negative_domain_exp_1() {
        let scale = PowNumericScale::new().domain((-2.0, 2.0)).range((0.0, 1.0));
        let values = vec![-2.0, -1.0, 0.0, 1.0, 2.0];
        let result = scale
            .scale(&values, &Default::default())
            .unwrap()
            .as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 0.0);
        assert_approx_eq!(f32, result[1], 0.25);
        assert_approx_eq!(f32, result[2], 0.5);
        assert_approx_eq!(f32, result[3], 0.75);
        assert_approx_eq!(f32, result[4], 1.0);
    }

    #[test]
    fn test_invert_negative_exp_2() {
        let scale = PowNumericScale::new()
            .domain((-2.0, 2.0))
            .range((-4.0, 4.0))
            .exponent(2.0);
        let values = vec![-4.0, -2.0, 0.0, 2.0, 4.0];
        let result = scale
            .invert(&values, &Default::default())
            .unwrap()
            .as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], -2.0);
        assert_approx_eq!(f32, result[1], -1.414214);
        assert_approx_eq!(f32, result[2], 0.0);
        assert_approx_eq!(f32, result[3], 1.414214);
        assert_approx_eq!(f32, result[4], 2.0);
    }

    #[test]
    fn test_nice() {
        // Test with exponent 2 (square)
        let scale = PowNumericScale::new()
            .domain((-10.0, 10.0))
            .exponent(2.0)
            .nice(None);
        let (d0, d1) = scale.get_domain();
        assert_approx_eq!(f32, d0, -10.0);
        assert_approx_eq!(f32, d1, 10.0);

        // Test with exponent 0.5 (sqrt)
        let scale = PowNumericScale::new()
            .domain((0.1, 0.9))
            .exponent(3.0)
            .nice(None);
        let (d0, d1) = scale.get_domain();
        assert_approx_eq!(f32, d0, scale.transform_inv(0.0));
        assert_approx_eq!(f32, d1, scale.transform_inv(0.8));

        // Test with custom exponent
        let scale = PowNumericScale::new()
            .domain((0.1, 0.9))
            .exponent(3.0)
            .nice(None);
        let (d0, d1) = scale.get_domain();
        assert_approx_eq!(f32, d0, scale.transform_inv(0.0));
        assert_approx_eq!(f32, d1, scale.transform_inv(0.8));
    }
}
