use avenger_common::value::{ScalarOrArray, ScalarOrArrayRef};

use crate::array;

use super::ContinuousNumericScale;

#[derive(Clone, Debug)]
pub struct LinearNumericScaleConfig {
    pub domain: (f32, f32),
    pub range: (f32, f32),
    pub clamp: bool,
    pub range_offset: Option<f32>,
    pub nice: Option<usize>,
    pub round: bool,
}

impl Default for LinearNumericScaleConfig {
    fn default() -> Self {
        Self {
            domain: (0.0, 1.0),
            range: (0.0, 1.0),
            clamp: false,
            range_offset: None,
            nice: None,
            round: false,
        }
    }
}

/// A linear scale that maps numeric input values from a domain to a range.
/// Supports clamping, domain niceing, and tick generation.
#[derive(Clone, Debug)]
pub struct LinearNumericScale {
    domain_start: f32,
    domain_end: f32,
    range_start: f32,
    range_end: f32,
    clamp: bool,
    range_offset: Option<f32>,
    round: bool,
}

impl LinearNumericScale {
    /// Creates a new linear scale with default domain [0, 1] and range [0, 1]
    pub fn new(config: &LinearNumericScaleConfig) -> Self {
        let mut this = Self {
            domain_start: config.domain.0,
            domain_end: config.domain.1,
            range_start: config.range.0,
            range_end: config.range.1,
            clamp: config.clamp,
            range_offset: config.range_offset,
            round: config.round,
        };

        if let Some(nice) = config.nice {
            this = this.nice(Some(nice));
        }

        this
    }

    /// Extends the domain to nice round numbers for better tick selection
    pub fn nice(mut self, count: Option<usize>) -> Self {
        if self.domain_start == self.domain_end
            || self.domain_start.is_nan()
            || self.domain_end.is_nan()
        {
            return self;
        }

        let (mut start, mut stop) = if self.domain_start <= self.domain_end {
            (self.domain_start, self.domain_end)
        } else {
            (self.domain_end, self.domain_start)
        };

        let mut prestep = 0.0;
        let mut max_iter = 10;

        let count = count.unwrap_or(10);
        while max_iter > 0 {
            let step = array::tick_increment(start as f32, stop as f32, count as f32);

            if step == prestep {
                if self.domain_start <= self.domain_end {
                    self.domain_start = start;
                    self.domain_end = stop;
                } else {
                    self.domain_start = stop;
                    self.domain_end = start;
                }
                return self;
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

        if self.domain_start <= self.domain_end {
            self.domain_start = start;
            self.domain_end = stop;
        } else {
            self.domain_start = stop;
            self.domain_end = start;
        }

        self
    }

    pub fn with_domain(mut self, domain: (f32, f32)) -> Self {
        self.domain_start = domain.0;
        self.domain_end = domain.1;
        self
    }

    pub fn with_range(mut self, range: (f32, f32)) -> Self {
        self.range_start = range.0;
        self.range_end = range.1;
        self
    }

    pub fn with_clamp(mut self, clamp: bool) -> Self {
        self.clamp = clamp;
        self
    }

    pub fn with_range_offset(mut self, range_offset: Option<f32>) -> Self {
        self.range_offset = range_offset;
        self
    }

    pub fn with_round(mut self, round: bool) -> Self {
        self.round = round;
        self
    }
}

impl ContinuousNumericScale<f32> for LinearNumericScale {
    fn domain(&self) -> (f32, f32) {
        (self.domain_start, self.domain_end)
    }

    /// Returns the current range as (start, end)
    fn range(&self) -> (f32, f32) {
        (self.range_start, self.range_end)
    }

    /// Returns whether output clamping is enabled
    fn clamp(&self) -> bool {
        self.clamp
    }

    /// Maps input values from domain to range
    fn scale<'a>(&self, values: impl Into<ScalarOrArrayRef<'a, f32>>) -> ScalarOrArray<f32> {
        // Handle degenerate domain/range cases
        if self.domain_start == self.domain_end
            || self.range_start == self.range_end
            || self.domain_start.is_nan()
            || self.domain_end.is_nan()
            || self.range_start.is_nan()
            || self.range_end.is_nan()
        {
            return values.into().map(|_| self.range_start);
        }

        let domain_span = self.domain_end - self.domain_start;
        let scale = (self.range_end - self.range_start) / domain_span;
        let range_offset = self.range_offset.unwrap_or(0.0);
        let offset = self.range_start - scale * self.domain_start + range_offset;

        let (range_min, range_max) = if self.range_start <= self.range_end {
            (self.range_start, self.range_end)
        } else {
            (self.range_end, self.range_start)
        };

        match (self.clamp, self.round) {
            (true, true) => {
                // clamp and round
                values
                    .into()
                    .map(|v| (scale * v + offset).clamp(range_min, range_max).round())
            }
            (true, false) => {
                // clamp, no round
                values
                    .into()
                    .map(|v| (scale * v + offset).clamp(range_min, range_max))
            }
            (false, true) => {
                // no clamp, round
                values.into().map(|v| (scale * v + offset).round())
            }
            (false, false) => {
                // no clamp, no round
                values.into().map(|v| scale * v + offset)
            }
        }
    }

    /// Maps output values from range back to domain
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

        let scale = (self.domain_end - self.domain_start) / (self.range_end - self.range_start);
        let range_offset = self.range_offset.unwrap_or(0.0);
        let offset = self.domain_start - scale * self.range_start;

        if self.clamp {
            let (range_min, range_max) = if self.range_start <= self.range_end {
                (self.range_start, self.range_end)
            } else {
                (self.range_end, self.range_start)
            };

            values.into().map(|v| {
                let v = (v - range_offset).clamp(range_min, range_max);
                scale * v + offset
            })
        } else {
            values.into().map(|v| scale * (v - range_offset) + offset)
        }
    }

    /// Generates evenly spaced tick values within the domain
    fn ticks(&self, count: Option<f32>) -> Vec<f32> {
        let count = count.unwrap_or(10.0);
        array::ticks(self.domain_start as f32, self.domain_end as f32, count)
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use float_cmp::assert_approx_eq;

    #[test]
    fn test_defaults() {
        let scale = LinearNumericScale::new(&Default::default());
        assert_eq!(scale.domain_start, 0.0);
        assert_eq!(scale.domain_end, 1.0);
        assert_eq!(scale.range_start, 0.0);
        assert_eq!(scale.range_end, 1.0);
        assert_eq!(scale.clamp, false);
    }

    #[test]
    fn test_scale() {
        // Test scaling with edge cases: out-of-bounds, nulls, and interpolation
        let scale = LinearNumericScale::new(&LinearNumericScaleConfig {
            domain: (10.0, 30.0),
            range: (0.0, 100.0),
            clamp: true,
            ..Default::default()
        });

        let values = vec![
            0.0,  // < domain
            10.0, // domain start
            15.0, 20.0, 25.0, 30.0, // in domain
            40.0, // > domain
        ];

        let result = scale.scale(&values).as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 0.0); // clamped
        assert_approx_eq!(f32, result[1], 0.0); // domain start
        assert_approx_eq!(f32, result[2], 25.0); // interpolated
        assert_approx_eq!(f32, result[3], 50.0); // interpolated
        assert_approx_eq!(f32, result[4], 75.0); // interpolated
        assert_approx_eq!(f32, result[5], 100.0); // domain end
        assert_approx_eq!(f32, result[6], 100.0); // clamped
    }

    #[test]
    fn test_scale_with_range_offset() {
        // Test scaling with edge cases: out-of-bounds, nulls, and interpolation
        let scale = LinearNumericScale::new(&LinearNumericScaleConfig {
            domain: (10.0, 30.0),
            range: (0.0, 100.0),
            range_offset: Some(3.0),
            clamp: true,
            ..Default::default()
        });

        let values = vec![
            0.0,  // < domain
            10.0, // domain start
            15.0, 20.0, 25.0, 30.0, // in domain
            40.0, // > domain
        ];

        let result = scale.scale(&values).as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 0.0); // clamped
        assert_approx_eq!(f32, result[1], 3.0); // domain start
        assert_approx_eq!(f32, result[2], 28.0); // interpolated
        assert_approx_eq!(f32, result[3], 53.0); // interpolated
        assert_approx_eq!(f32, result[4], 78.0); // interpolated
        assert_approx_eq!(f32, result[5], 100.0); // domain end
        assert_approx_eq!(f32, result[6], 100.0); // clamped
    }

    // Degenerate domain tests
    #[test]
    fn test_scale_degenerate() {
        // Tests behavior with zero-width domain (matches d3 behavior)
        let scale = LinearNumericScale::new(&LinearNumericScaleConfig {
            domain: (10.0, 10.0),
            range: (0.0, 100.0),
            clamp: true,
            ..Default::default()
        });

        let values = vec![0.0, 10.0, 20.0];
        let result = scale.scale(&values).as_vec(values.len(), None);

        // All values should map to range_start (d3 behavior)
        for i in 0..result.len() {
            assert_approx_eq!(f32, result[i], 0.0);
        }
    }

    #[test]
    fn test_degenerate_cases() {
        // Test degenerate domain
        let scale = LinearNumericScale::new(&LinearNumericScaleConfig {
            domain: (1.0, 1.0),
            range: (0.0, 0.0),
            clamp: false,
            ..Default::default()
        });
        let values = vec![0.0, 1.0, 2.0];
        let result = scale.scale(&values).as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 0.0);
        assert_approx_eq!(f32, result[1], 0.0);
        assert_approx_eq!(f32, result[2], 0.0);

        // Test degenerate range
        let scale = LinearNumericScale::new(&LinearNumericScaleConfig {
            domain: (0.0, 10.0),
            range: (1.0, 1.0),
            clamp: false,
            ..Default::default()
        });
        let result = scale.scale(&values).as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 1.0);
        assert_approx_eq!(f32, result[1], 1.0);
        assert_approx_eq!(f32, result[2], 1.0);
    }

    #[test]
    fn test_invert_clamped() {
        let scale = LinearNumericScale::new(&LinearNumericScaleConfig {
            domain: (10.0, 30.0),
            range: (0.0, 100.0),
            clamp: true,
            ..Default::default()
        });

        let values = vec![-25.0, 0.0, 50.0, 100.0, 125.0];
        let result = scale.invert(&values).as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 10.0); // clamped below
        assert_approx_eq!(f32, result[1], 10.0); // range start
        assert_approx_eq!(f32, result[2], 20.0); // interpolated
        assert_approx_eq!(f32, result[3], 30.0); // range end
        assert_approx_eq!(f32, result[4], 30.0); // clamped above
    }

    #[test]
    fn test_invert_unclamped() {
        // Tests invert with clamping disabled (extrapolation)
        let scale = LinearNumericScale::new(&LinearNumericScaleConfig {
            domain: (10.0, 30.0),
            range: (0.0, 100.0),
            clamp: false,
            ..Default::default()
        });

        let values = vec![-25.0, 0.0, 50.0, 100.0, 125.0];
        let result = scale.invert(&values).as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 5.0); // below range
        assert_approx_eq!(f32, result[1], 10.0); // range start
        assert_approx_eq!(f32, result[2], 20.0); // interpolated
        assert_approx_eq!(f32, result[3], 30.0); // range end
        assert_approx_eq!(f32, result[4], 35.0); // above range
    }

    #[test]
    fn test_invert_with_range_offset() {
        // Tests invert with clamping disabled (extrapolation)
        let scale = LinearNumericScale::new(&LinearNumericScaleConfig {
            domain: (10.0, 30.0),
            range: (0.0, 100.0),
            range_offset: Some(3.0),
            clamp: false,
            ..Default::default()
        });

        let values = vec![-22.0, 3.0, 53.0, 103.0, 128.0];
        let result = scale.invert(&values).as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 5.0); // below range
        assert_approx_eq!(f32, result[1], 10.0); // range start
        assert_approx_eq!(f32, result[2], 20.0); // interpolated
        assert_approx_eq!(f32, result[3], 30.0); // range end
        assert_approx_eq!(f32, result[4], 35.0); // above range
    }

    #[test]
    fn test_invert_reversed_range() {
        // Tests invert with reversed range (d3.scaleLinear.invert with reversed range)
        let scale = LinearNumericScale::new(&LinearNumericScaleConfig {
            domain: (10.0, 30.0),
            range: (100.0, 0.0),
            clamp: true,
            ..Default::default()
        });

        let values = vec![125.0, 100.0, 50.0, 0.0, -25.0];
        let result = scale.invert(&values).as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 10.0); // clamped
        assert_approx_eq!(f32, result[1], 10.0); // range start
        assert_approx_eq!(f32, result[2], 20.0); // interpolated
        assert_approx_eq!(f32, result[3], 30.0); // range end
        assert_approx_eq!(f32, result[4], 30.0); // clamped
    }

    #[test]
    fn test_ticks() {
        // Tests basic tick generation (d3.scaleLinear.ticks)
        let scale = LinearNumericScale::new(&LinearNumericScaleConfig {
            domain: (0.0, 10.0),
            range: (0.0, 100.0),
            ..Default::default()
        });

        assert_eq!(scale.ticks(Some(5.0)), vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0]);
        assert_eq!(scale.ticks(Some(2.0)), vec![0.0, 5.0, 10.0]);
        assert_eq!(scale.ticks(Some(1.0)), vec![0.0, 10.0]);
    }

    #[test]
    fn test_ticks_span_zero() {
        // Tests tick generation across zero (d3.scaleLinear.ticks with domain crossing zero)
        let scale = LinearNumericScale::new(&LinearNumericScaleConfig {
            domain: (-100.0, 100.0),
            ..Default::default()
        });

        assert_eq!(
            scale.ticks(Some(10.0)),
            vec![-100.0, -80.0, -60.0, -40.0, -20.0, 0.0, 20.0, 40.0, 60.0, 80.0, 100.0]
        );
        assert_eq!(
            scale.ticks(Some(5.0)),
            vec![-100.0, -50.0, 0.0, 50.0, 100.0]
        );
        assert_eq!(scale.ticks(Some(2.0)), vec![-100.0, 0.0, 100.0]);
        assert_eq!(scale.ticks(Some(1.0)), vec![0.0]);
    }

    // Nice domain tests
    #[test]
    fn test_nice_convergence() {
        // Tests nice() with typical domain (d3.scaleLinear.nice)
        let scale = LinearNumericScale::new(&LinearNumericScaleConfig {
            domain: (1.1, 10.9),
            ..Default::default()
        })
        .nice(Some(10));

        assert_eq!(scale.domain_start, 1.0);
        assert_eq!(scale.domain_end, 11.0);
    }

    #[test]
    fn test_nice_negative_step() {
        // Tests nice() with reversed domain
        let scale = LinearNumericScale::new(&LinearNumericScaleConfig {
            domain: (-1.1, -10.9),
            ..Default::default()
        })
        .nice(Some(10));

        assert_eq!(scale.domain_start, -1.0);
        assert_eq!(scale.domain_end, -11.0);
    }
}
