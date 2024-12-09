use avenger_common::value::{ScalarOrArray, ScalarOrArrayRef};

use super::{
    linear::{LinearNumericScale, LinearNumericScaleConfig},
    ContinuousNumericScale,
};

/// Configuration for a symlog scale
#[derive(Debug, Clone)]
pub struct SymlogNumericScaleConfig {
    pub domain: (f32, f32),
    pub range: (f32, f32),
    pub constant: f32,
    pub clamp: bool,
    pub range_offset: f32,
    pub nice: Option<usize>,
}

impl Default for SymlogNumericScaleConfig {
    fn default() -> Self {
        Self {
            domain: (0.0, 1.0),
            range: (0.0, 1.0),
            constant: 1.0,
            clamp: false,
            range_offset: 0.0,
            nice: None,
        }
    }
}

/// A symmetric log scale that maps numeric input values using a log transform that handles zero and negative values.
/// The transform is linear near zero (controlled by the constant parameter) and logarithmic further out.
#[derive(Debug, Clone)]
pub struct SymlogNumericScale {
    domain_start: f32,
    domain_end: f32,
    range_start: f32,
    range_end: f32,
    constant: f32,
    clamp: bool,
    range_offset: f32,
}

impl SymlogNumericScale {
    /// Creates a new symlog scale with default domain [0, 1], range [0, 1], and constant 1
    pub fn new(config: &SymlogNumericScaleConfig) -> Self {
        let mut this = Self {
            domain_start: config.domain.0,
            domain_end: config.domain.1,
            range_start: config.range.0,
            range_end: config.range.1,
            constant: config.constant,
            clamp: config.clamp,
            range_offset: config.range_offset,
        };
        if let Some(count) = config.nice {
            this = this.nice(Some(count));
        }
        this
    }

    /// Returns the current constant value
    pub fn get_constant(&self) -> f32 {
        self.constant
    }

    /// Applies the symlog transform to a single value
    fn transform(&self, x: f32) -> f32 {
        let sign = if x < 0.0 { -1.0 } else { 1.0 };
        sign * (1.0 + (x.abs() / self.constant)).ln()
    }

    /// Applies the inverse symlog transform to a single value
    fn transform_inv(&self, x: f32) -> f32 {
        let sign = if x < 0.0 { -1.0 } else { 1.0 };
        sign * ((x.abs()).exp() - 1.0) * self.constant
    }

    /// Extends the domain to nice round numbers in transformed space
    pub fn nice(self, count: Option<usize>) -> Self {
        // Create a linear scale to nice the transformed values
        let d0 = self.transform(self.domain_start);
        let d1 = self.transform(self.domain_end);

        let linear = LinearNumericScale::new(&LinearNumericScaleConfig {
            domain: (d0, d1),
            ..Default::default()
        })
        .nice(count);

        let (nice_d0, nice_d1) = linear.domain();

        // Transform back to original space
        let domain_start = self.transform_inv(nice_d0);
        let domain_end = self.transform_inv(nice_d1);
        Self {
            domain_start,
            domain_end,
            ..self
        }
    }

    /// Sets the domain
    pub fn with_domain(self, (domain_start, domain_end): (f32, f32)) -> Self {
        Self {
            domain_start,
            domain_end,
            ..self
        }
    }

    /// Sets the range
    pub fn with_range(self, (range_start, range_end): (f32, f32)) -> Self {
        Self {
            range_start,
            range_end,
            ..self
        }
    }

    /// Sets the clamp flag
    pub fn with_clamp(self, clamp: bool) -> Self {
        Self { clamp, ..self }
    }

    /// Sets the range offset
    pub fn with_range_offset(self, range_offset: f32) -> Self {
        Self {
            range_offset,
            ..self
        }
    }

    /// Sets the constant
    pub fn with_constant(self, constant: f32) -> Self {
        Self { constant, ..self }
    }
}

impl ContinuousNumericScale<f32> for SymlogNumericScale {
    fn domain(&self) -> (f32, f32) {
        (self.domain_start, self.domain_end)
    }

    fn range(&self) -> (f32, f32) {
        (self.range_start, self.range_end)
    }

    fn clamp(&self) -> bool {
        self.clamp
    }

    fn set_domain(&mut self, domain: (f32, f32)) {
        self.domain_start = domain.0;
        self.domain_end = domain.1;
    }

    fn set_range(&mut self, range: (f32, f32)) {
        self.range_start = range.0;
        self.range_end = range.1;
    }

    fn set_clamp(&mut self, clamp: bool) {
        self.clamp = clamp;
    }

    fn scale<'a>(&self, values: impl Into<ScalarOrArrayRef<'a, f32>>) -> ScalarOrArray<f32> {
        // Handle degenerate domain case
        if self.domain_start == self.domain_end
            || self.range_start == self.range_end
            || self.domain_start.is_nan()
            || self.domain_end.is_nan()
            || self.range_start.is_nan()
            || self.range_end.is_nan()
        {
            return values.into().map(|_| self.range_start);
        }

        // Pre-compute transformed domain endpoints
        let d0 = self.transform(self.domain_start);
        let d1 = self.transform(self.domain_end);

        // Pre-compute scale and offset outside the loop
        let scale = (self.range_end - self.range_start) / (d1 - d0);
        let offset = self.range_start - scale * d0 + self.range_offset;
        let constant = self.constant;

        if self.clamp {
            // Pre-compute range bounds outside the loop
            let (range_min, range_max) = if self.range_start <= self.range_end {
                (self.range_start, self.range_end)
            } else {
                (self.range_end, self.range_start)
            };

            values.into().map(|&v| {
                if v.is_nan() {
                    return f32::NAN;
                }
                if v.is_infinite() {
                    if v.is_sign_positive() {
                        return range_max;
                    } else {
                        return range_min;
                    }
                }
                // Apply symlog transform
                let sign: f32 = if v < 0.0 { -1.0 } else { 1.0 };
                let transformed = sign * (1.0 + (v.abs() / constant)).ln();

                // Apply scale and offset, then clamp
                (scale * transformed + offset).clamp(range_min, range_max)
            })
        } else {
            values.into().map(|&v| {
                if v.is_nan() {
                    return f32::NAN;
                }
                if v.is_infinite() {
                    return if v.is_sign_positive() {
                        self.range_end
                    } else {
                        self.range_start
                    };
                }
                // Apply symlog transform
                let sign = if v < 0.0 { -1.0 } else { 1.0 };
                let transformed = sign * (1.0 + (v.abs() / constant)).ln();

                // Apply scale and offset
                scale * transformed + offset
            })
        }
    }

    /// Maps output values from range back to domain using inverse symlog transform
    fn invert<'a>(&self, values: impl Into<ScalarOrArrayRef<'a, f32>>) -> ScalarOrArray<f32> {
        // Handle degenerate domain case
        if self.domain_start == self.domain_end
            || self.range_start == self.range_end
            || self.domain_start.is_nan()
            || self.domain_end.is_nan()
            || self.range_start.is_nan()
            || self.range_end.is_nan()
        {
            return values.into().map(|_| self.domain_start);
        }

        // Pre-compute transformed domain endpoints
        let d0 = self.transform(self.domain_start);
        let d1 = self.transform(self.domain_end);

        // Pre-compute scale and offset outside the loop
        let scale = (d1 - d0) / (self.range_end - self.range_start);
        let range_offset = self.range_offset;
        let offset = d0 - scale * self.range_start;

        if self.clamp {
            // Pre-compute range bounds outside the loop
            let (range_min, range_max) = if self.range_start <= self.range_end {
                (self.range_start, self.range_end)
            } else {
                (self.range_end, self.range_start)
            };

            // Pre-compute constant for efficiency
            let constant = self.constant;

            values.into().map(|&v| {
                if v.is_nan() {
                    return f32::NAN;
                }
                if v.is_infinite() {
                    if v.is_sign_positive() {
                        return self.domain_end;
                    } else {
                        return self.domain_start;
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
        } else {
            // Pre-compute constant for efficiency
            let constant = self.constant;

            values.into().map(|&v| {
                if v.is_nan() {
                    return f32::NAN;
                }
                if v.is_infinite() {
                    return if v.is_sign_positive() {
                        self.domain_end
                    } else {
                        self.domain_start
                    };
                }

                // Transform back to original space
                let normalized = scale * (v - range_offset) + offset;
                let sign = if normalized < 0.0 { -1.0 } else { 1.0 };

                // Apply inverse transform
                sign * (normalized.abs().exp() - 1.0) * constant
            })
        }
    }

    /// Generates evenly spaced tick values within the domain
    fn ticks(&self, count: Option<f32>) -> Vec<f32> {
        // Use linear scale to generate ticks in transformed space
        let linear = LinearNumericScale::new(&LinearNumericScaleConfig {
            domain: self.domain(),
            ..Default::default()
        });

        return linear.ticks(count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use float_cmp::{assert_approx_eq, F32Margin};

    #[test]
    fn test_defaults() {
        let scale = SymlogNumericScale::new(&SymlogNumericScaleConfig::default());
        assert_eq!(scale.domain(), (0.0, 1.0));
        assert_eq!(scale.range(), (0.0, 1.0));
        assert_eq!(scale.clamp(), false);
        assert_eq!(scale.get_constant(), 1.0);
    }

    #[test]
    fn test_basic_scale() {
        let scale = SymlogNumericScale::new(&SymlogNumericScaleConfig {
            domain: (-100.0, 100.0),
            range: (0.0, 1.0),
            ..Default::default()
        });

        let values = vec![-100.0, 0.0, 100.0];
        let result = scale.scale(&values).as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 0.0);
        assert_approx_eq!(f32, result[1], 0.5);
        assert_approx_eq!(f32, result[2], 1.0);
    }

    #[test]
    fn test_basic_scale_range_offset() {
        let scale = SymlogNumericScale::new(&SymlogNumericScaleConfig {
            domain: (-100.0, 100.0),
            range: (0.0, 1.0),
            range_offset: 0.5,
            ..Default::default()
        });

        let values = vec![-100.0, 0.0, 100.0];
        let result = scale.scale(&values).as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 0.5);
        assert_approx_eq!(f32, result[1], 1.0);
        assert_approx_eq!(f32, result[2], 1.5);
    }

    #[test]
    fn test_constant() {
        let scale = SymlogNumericScale::new(&SymlogNumericScaleConfig {
            constant: 5.0,
            ..Default::default()
        });
        assert_eq!(scale.get_constant(), 5.0);

        // Test that changing constant doesn't affect domain or range
        assert_eq!(scale.domain(), (0.0, 1.0));
        assert_eq!(scale.range(), (0.0, 1.0));
    }

    #[test]
    fn test_clamp() {
        let scale = SymlogNumericScale::new(&SymlogNumericScaleConfig {
            range: (10.0, 20.0),
            ..Default::default()
        });

        // Default no clamping
        let values = vec![3.0, -1.0];
        let result = scale.scale(&values).as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 30.0);
        assert_approx_eq!(f32, result[1], 0.0);

        // With clamping
        let scale = scale.with_clamp(true);
        let result = scale.scale(&values).as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 20.0);
        assert_approx_eq!(f32, result[1], 10.0);
    }

    #[test]
    fn test_edge_cases() {
        let scale = SymlogNumericScale::new(&SymlogNumericScaleConfig {
            domain: (-100.0, 100.0),
            range: (0.0, 1.0),
            ..Default::default()
        });

        // Test NaN
        let values = vec![f32::NAN];
        let result = scale.scale(&values).as_vec(values.len(), None);
        assert!(result[0].is_nan());

        // Test infinity
        let values = vec![f32::INFINITY, f32::NEG_INFINITY];
        let result = scale.scale(&values).as_vec(values.len(), None);
        assert!(result[0].is_finite()); // Should be clamped if clamp is true
        assert!(result[1].is_finite()); // Should be clamped if clamp is true
    }

    #[test]
    fn test_invert() {
        let scale = SymlogNumericScale::new(&SymlogNumericScaleConfig {
            domain: (-100.0, 100.0),
            range: (0.0, 1.0),
            ..Default::default()
        });

        // Test that invert(scale(x)) ≈ x
        let values = vec![-100.0, -10.0, -1.0, 0.0, 1.0, 10.0, 100.0];
        let scaled = scale.scale(&values).as_vec(values.len(), None);
        let inverted = scale.invert(&scaled).as_vec(values.len(), None);

        for i in 0..values.len() {
            assert_approx_eq!(f32, inverted[i], values[i]);
        }
    }

    #[test]
    fn test_invert_range_offset() {
        let scale = SymlogNumericScale::new(&SymlogNumericScaleConfig {
            domain: (-100.0, 100.0),
            range: (0.0, 1.0),
            range_offset: 0.5,
            ..Default::default()
        });

        // Test that invert(scale(x)) ≈ x
        let values = vec![-100.0, -10.0, -1.0, 0.0, 1.0, 10.0, 100.0];
        let scaled = scale.scale(&values).as_vec(values.len(), None);
        let inverted = scale.invert(&scaled).as_vec(values.len(), None);

        for i in 0..values.len() {
            assert_approx_eq!(
                f32,
                inverted[i],
                values[i],
                F32Margin {
                    epsilon: 0.0001,
                    ..Default::default()
                }
            );
        }
    }

    #[test]
    fn test_invert_clamped() {
        let scale = SymlogNumericScale::new(&SymlogNumericScaleConfig {
            domain: (-100.0, 100.0),
            range: (0.0, 1.0),
            clamp: true,
            ..Default::default()
        });

        // Test values outside the range
        let values = vec![-0.5, 1.5];
        let result = scale.invert(&values).as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], -100.0);
        assert_approx_eq!(f32, result[1], 100.0);
    }

    #[test]
    fn test_invert_constant() {
        let scale = SymlogNumericScale::new(&SymlogNumericScaleConfig {
            constant: 2.0,
            domain: (-100.0, 100.0),
            range: (0.0, 1.0),
            ..Default::default()
        });

        // Test that invert(scale(x)) ≈ x with different constant
        let values = vec![-50.0, 0.0, 50.0];
        let scaled = scale.scale(&values).as_vec(values.len(), None);
        let inverted = scale.invert(&scaled).as_vec(values.len(), None);

        for i in 0..values.len() {
            assert_approx_eq!(f32, inverted[i], values[i]);
        }
    }

    #[test]
    fn test_invert_edge_cases() {
        let scale = SymlogNumericScale::new(&SymlogNumericScaleConfig {
            domain: (-100.0, 100.0),
            range: (0.0, 1.0),
            ..Default::default()
        });

        // Test NaN
        let values = vec![f32::NAN];
        let result = scale.invert(&values).as_vec(values.len(), None);
        assert!(result[0].is_nan());

        // Test infinity
        let values = vec![f32::INFINITY, f32::NEG_INFINITY];
        let result = scale.invert(&values).as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 100.0); // maps to domain end
        assert_approx_eq!(f32, result[1], -100.0); // maps to domain start
    }

    #[test]
    fn test_invert_degenerate() {
        // Test degenerate domain
        let scale = SymlogNumericScale::new(&Default::default())
            .with_domain((1.0, 1.0))
            .with_range((0.0, 1.0));
        let values = vec![0.0, 0.5, 1.0];
        let result = scale.invert(&values).as_vec(values.len(), None);
        for i in 0..values.len() {
            assert_approx_eq!(f32, result[i], 1.0);
        }

        // Test degenerate range
        let scale = SymlogNumericScale::new(&Default::default())
            .with_domain((-100.0, 100.0))
            .with_range((1.0, 1.0));
        let result = scale.invert(&values).as_vec(values.len(), None);
        for i in 0..values.len() {
            assert_approx_eq!(f32, result[i], -100.0);
        }
    }

    #[test]
    fn test_nice() {
        let scale = SymlogNumericScale::new(&SymlogNumericScaleConfig {
            constant: 2.0,
            domain: (0.1, 0.9),
            ..Default::default()
        })
        .nice(None);

        let nice_domain = scale.domain();
        let transformed_domain = (scale.transform_inv(0.0), scale.transform_inv(0.4));

        // The domain should NOT change after nice() with these values
        assert_approx_eq!(f32, transformed_domain.0, nice_domain.0);
        assert_approx_eq!(f32, transformed_domain.1, nice_domain.1);
        assert_eq!(scale.get_constant(), 2.0);
    }

    #[test]
    fn test_ticks() {
        let scale = SymlogNumericScale::new(&SymlogNumericScaleConfig {
            domain: (-1.0, 1.0),
            ..Default::default()
        });
        let ticks = scale.ticks(Some(10.0));
        let expected = vec![
            -1.0f32, -0.8, -0.6, -0.4, -0.2, 0.0, 0.2, 0.4, 0.6, 0.8, 1.0,
        ];

        assert_eq!(ticks.len(), expected.len());
        for (a, b) in ticks.iter().zip(expected.iter()) {
            assert_approx_eq!(f32, *a, *b);
        }
    }

    #[test]
    fn test_ticks_with_constant() {
        let scale = SymlogNumericScale::new(&SymlogNumericScaleConfig {
            constant: 2.0,
            domain: (-10.0, 10.0),
            ..Default::default()
        });

        let ticks = scale.ticks(Some(5.0));
        assert!(ticks.len() > 0);

        // Ticks should be symmetric around zero
        let mid_idx = ticks.len() / 2;
        if ticks.len() % 2 == 1 {
            assert_approx_eq!(f32, ticks[mid_idx], 0.0);
        }

        // Test symmetry of positive/negative ticks
        for i in 0..mid_idx {
            assert_approx_eq!(f32, ticks[i].abs(), ticks[ticks.len() - 1 - i].abs());
        }
    }
}
