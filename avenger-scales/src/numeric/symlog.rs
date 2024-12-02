use crate::error::AvengerScaleError;

use super::linear::LinearNumericScale;

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
}

impl SymlogNumericScale {
    /// Creates a new symlog scale with default domain [0, 1], range [0, 1], and constant 1
    pub fn new(constant: Option<f32>) -> Self {
        Self {
            domain_start: 0.0,
            domain_end: 1.0,
            range_start: 0.0,
            range_end: 1.0,
            constant: constant.unwrap_or(1.0),
            clamp: false,
        }
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

    /// Sets the constant that determines the size of the linear region around zero
    pub fn constant(mut self, constant: f32) -> Self {
        self.constant = constant;
        self
    }

    /// Returns the current constant value
    pub fn get_constant(&self) -> f32 {
        self.constant
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

    /// Maps input values from domain to range using symlog transform
    pub fn scale(&self, values: &[f32]) -> Result<Vec<f32>, AvengerScaleError> {
        // Handle degenerate domain case
        if self.domain_start == self.domain_end {
            return Ok(vec![self.range_start; values.len()]);
        }

        // Handle degenerate range case
        if self.range_start == self.range_end {
            return Ok(vec![self.range_start; values.len()]);
        }

        // Pre-compute transformed domain endpoints
        let d0 = self.transform(self.domain_start);
        let d1 = self.transform(self.domain_end);

        // Pre-compute scale and offset outside the loop
        let scale = (self.range_end - self.range_start) / (d1 - d0);
        let offset = self.range_start - scale * d0;
        let constant = self.constant;

        if self.clamp {
            // Pre-compute range bounds outside the loop
            let (range_min, range_max) = if self.range_start <= self.range_end {
                (self.range_start, self.range_end)
            } else {
                (self.range_end, self.range_start)
            };

            Ok(values
                .iter()
                .map(|&v| {
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
                    let sign = if v < 0.0 { -1.0 } else { 1.0 };
                    let transformed = sign * (1.0 + (v.abs() / constant)).ln();

                    // Apply scale and offset, then clamp
                    (scale * transformed + offset).clamp(range_min, range_max)
                })
                .collect())
        } else {
            Ok(values
                .iter()
                .map(|&v| {
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
                .collect())
        }
    }

    /// Maps output values from range back to domain using inverse symlog transform
    pub fn invert(&self, values: &[f32]) -> Result<Vec<f32>, AvengerScaleError> {
        // Handle degenerate domain case
        if self.domain_start == self.domain_end {
            return Ok(vec![self.domain_start; values.len()]);
        }

        // Handle degenerate range case
        if self.range_start == self.range_end {
            return Ok(vec![self.domain_start; values.len()]);
        }

        // Pre-compute transformed domain endpoints
        let d0 = self.transform(self.domain_start);
        let d1 = self.transform(self.domain_end);

        // Pre-compute scale and offset outside the loop
        let scale = (d1 - d0) / (self.range_end - self.range_start);
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

            Ok(values
                .iter()
                .map(|&v| {
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
                    let normalized = scale * v + offset;
                    let sign = if normalized < 0.0 { -1.0 } else { 1.0 };

                    // Apply inverse transform
                    sign * (normalized.abs().exp() - 1.0) * constant
                })
                .collect())
        } else {
            // Pre-compute constant for efficiency
            let constant = self.constant;

            Ok(values
                .iter()
                .map(|&v| {
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
                    let normalized = scale * v + offset;
                    let sign = if normalized < 0.0 { -1.0 } else { 1.0 };

                    // Apply inverse transform
                    sign * (normalized.abs().exp() - 1.0) * constant
                })
                .collect())
        }
    }

    /// Extends the domain to nice round numbers in transformed space
    pub fn nice(self, count: Option<usize>) -> Self {
        // Create a linear scale to nice the transformed values
        let d0 = self.transform(self.domain_start);
        let d1 = self.transform(self.domain_end);

        let linear = LinearNumericScale::new().domain((d0, d1)).nice(count);

        let (nice_d0, nice_d1) = linear.get_domain();

        // Transform back to original space
        let domain_start = self.transform_inv(nice_d0);
        let domain_end = self.transform_inv(nice_d1);
        self.domain((domain_start, domain_end))
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
        let scale = SymlogNumericScale::new(None);
        assert_eq!(scale.get_domain(), (0.0, 1.0));
        assert_eq!(scale.get_range(), (0.0, 1.0));
        assert_eq!(scale.get_clamp(), false);
        assert_eq!(scale.get_constant(), 1.0);
    }

    #[test]
    fn test_basic_scale() {
        let scale = SymlogNumericScale::new(None)
            .domain((-100.0, 100.0))
            .range((0.0, 1.0));

        let values = vec![-100.0, 0.0, 100.0];
        let result = scale.scale(&values).unwrap();

        assert_approx_eq!(f32, result[0], 0.0);
        assert_approx_eq!(f32, result[1], 0.5);
        assert_approx_eq!(f32, result[2], 1.0);
    }

    #[test]
    fn test_constant() {
        let scale = SymlogNumericScale::new(None).constant(5.0);
        assert_eq!(scale.get_constant(), 5.0);

        // Test that changing constant doesn't affect domain or range
        assert_eq!(scale.get_domain(), (0.0, 1.0));
        assert_eq!(scale.get_range(), (0.0, 1.0));
    }

    #[test]
    fn test_clamp() {
        let scale = SymlogNumericScale::new(None).range((10.0, 20.0));

        // Default no clamping
        let values = vec![3.0, -1.0];
        let result = scale.scale(&values).unwrap();
        assert_approx_eq!(f32, result[0], 30.0);
        assert_approx_eq!(f32, result[1], 0.0);

        // With clamping
        let scale = scale.clamp(true);
        let result = scale.scale(&values).unwrap();
        assert_approx_eq!(f32, result[0], 20.0);
        assert_approx_eq!(f32, result[1], 10.0);
    }

    #[test]
    fn test_edge_cases() {
        let scale = SymlogNumericScale::new(None)
            .domain((-100.0, 100.0))
            .range((0.0, 1.0));

        // Test NaN
        let values = vec![f32::NAN];
        let result = scale.scale(&values).unwrap();
        assert!(result[0].is_nan());

        // Test infinity
        let values = vec![f32::INFINITY, f32::NEG_INFINITY];
        let result = scale.scale(&values).unwrap();
        assert!(result[0].is_finite()); // Should be clamped if clamp is true
        assert!(result[1].is_finite()); // Should be clamped if clamp is true
    }

    #[test]
    fn test_invert() {
        let scale = SymlogNumericScale::new(None)
            .domain((-100.0, 100.0))
            .range((0.0, 1.0));

        // Test that invert(scale(x)) ≈ x
        let values = vec![-100.0, -10.0, -1.0, 0.0, 1.0, 10.0, 100.0];
        let scaled = scale.scale(&values).unwrap();
        let inverted = scale.invert(&scaled).unwrap();

        for i in 0..values.len() {
            assert_approx_eq!(f32, inverted[i], values[i]);
        }
    }

    #[test]
    fn test_invert_clamped() {
        let scale = SymlogNumericScale::new(None)
            .domain((-100.0, 100.0))
            .range((0.0, 1.0))
            .clamp(true);

        // Test values outside the range
        let values = vec![-0.5, 1.5];
        let result = scale.invert(&values).unwrap();
        assert_approx_eq!(f32, result[0], -100.0);
        assert_approx_eq!(f32, result[1], 100.0);
    }

    #[test]
    fn test_invert_constant() {
        let scale = SymlogNumericScale::new(Some(2.0)) // different constant
            .domain((-100.0, 100.0))
            .range((0.0, 1.0));

        // Test that invert(scale(x)) ≈ x with different constant
        let values = vec![-50.0, 0.0, 50.0];
        let scaled = scale.scale(&values).unwrap();
        let inverted = scale.invert(&scaled).unwrap();

        for i in 0..values.len() {
            assert_approx_eq!(f32, inverted[i], values[i]);
        }
    }

    #[test]
    fn test_invert_edge_cases() {
        let scale = SymlogNumericScale::new(None)
            .domain((-100.0, 100.0))
            .range((0.0, 1.0));

        // Test NaN
        let values = vec![f32::NAN];
        let result = scale.invert(&values).unwrap();
        assert!(result[0].is_nan());

        // Test infinity
        let values = vec![f32::INFINITY, f32::NEG_INFINITY];
        let result = scale.invert(&values).unwrap();
        assert_approx_eq!(f32, result[0], 100.0); // maps to domain end
        assert_approx_eq!(f32, result[1], -100.0); // maps to domain start
    }

    #[test]
    fn test_invert_degenerate() {
        // Test degenerate domain
        let scale = SymlogNumericScale::new(None)
            .domain((1.0, 1.0))
            .range((0.0, 1.0));
        let values = vec![0.0, 0.5, 1.0];
        let result = scale.invert(&values).unwrap();
        for i in 0..values.len() {
            assert_approx_eq!(f32, result[i], 1.0);
        }

        // Test degenerate range
        let scale = SymlogNumericScale::new(None)
            .domain((-100.0, 100.0))
            .range((1.0, 1.0));
        let result = scale.invert(&values).unwrap();
        for i in 0..values.len() {
            assert_approx_eq!(f32, result[i], -100.0);
        }
    }

    #[test]
    fn test_nice() {
        let scale = SymlogNumericScale::new(Some(2.0))
            .domain((0.1, 0.9))
            .nice(None);

        let nice_domain = scale.get_domain();
        let transformed_domain = (scale.transform_inv(0.0), scale.transform_inv(0.4));

        // The domain should NOT change after nice() with these values
        assert_approx_eq!(f32, transformed_domain.0, nice_domain.0);
        assert_approx_eq!(f32, transformed_domain.1, nice_domain.1);
        assert_eq!(scale.get_constant(), 2.0);
    }

    #[test]
    fn test_ticks() {
        let scale = SymlogNumericScale::new(None).domain((-1.0, 1.0));
        let ticks = scale.ticks(Some(10.0));
        let expected = vec![
            -0.6f32, -0.5, -0.4, -0.3, -0.2, -0.1, 0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6,
        ];

        assert_eq!(ticks.len(), expected.len());
        for (a, b) in ticks.iter().zip(expected.iter()) {
            assert_approx_eq!(f32, *a, scale.transform_inv(*b));
        }
    }

    #[test]
    fn test_ticks_with_constant() {
        let scale = SymlogNumericScale::new(Some(2.0)).domain((-10.0, 10.0));

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
