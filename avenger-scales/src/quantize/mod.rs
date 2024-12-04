use avenger_common::value::{ScalarOrArray, ScalarOrArrayRef};

use crate::numeric::{
    linear::{LinearNumericScale, LinearNumericScaleConfig},
    ContinuousNumericScale,
};
use std::fmt::Debug;

pub struct QuantizeScaleConfig {
    pub domain: (f32, f32),
    pub nice: Option<usize>,
}

impl Default for QuantizeScaleConfig {
    fn default() -> Self {
        Self {
            domain: (0.0, 1.0),
            nice: None,
        }
    }
}

/// A quantize scale divides a continuous domain into uniform segments and maps values to a discrete range.
///
/// The quantize scale is like a linear scale, except it divides the domain into uniform segments
/// based on the number of values in the range. Each segment is then mapped to a corresponding
/// discrete value in the range.
#[derive(Debug, Clone)]
pub struct QuantizeScale<R>
where
    R: Clone + Debug + Sync + 'static,
{
    domain: (f32, f32),
    range: Vec<R>,
    default: R,
}

impl<R> QuantizeScale<R>
where
    R: Clone + Debug + Sync + 'static,
{
    /// Creates a new quantize scale with default domain [0,1] and range [0,1]
    pub fn new(range: Vec<R>, default: R, config: &QuantizeScaleConfig) -> Self {
        let mut this = Self {
            domain: config.domain,
            range,
            default,
        };

        if let Some(nice) = config.nice {
            this = this.nice(Some(nice));
        }
        this
    }

    /// Sets the input domain as a tuple of (min, max)
    pub fn with_domain(mut self, domain: (f32, f32)) -> Self {
        self.domain = domain;
        self
    }

    /// Sets the output range as an Arrow array
    pub fn with_range(mut self, range: Vec<R>) -> Self {
        self.range = range;
        self
    }

    pub fn with_default(mut self, default: R) -> Self {
        self.default = default;
        self
    }

    /// Extends the domain to nice round numbers for better quantization boundaries
    pub fn nice(mut self, count: Option<usize>) -> Self {
        // Use linear scale for the nice calculation
        self.domain = LinearNumericScale::new(&LinearNumericScaleConfig {
            domain: self.domain,
            ..Default::default()
        })
        .nice(count)
        .domain();
        self
    }

    /// Returns the input domain
    pub fn domain(&self) -> (f32, f32) {
        self.domain
    }

    /// Returns a reference to the output range
    pub fn range(&self) -> &Vec<R> {
        &self.range
    }

    /// Returns the default value
    pub fn default(&self) -> &R {
        &self.default
    }

    /// Returns the threshold values that divide the domain
    pub fn thresholds(&self) -> Vec<f32> {
        let n = self.range.len();
        if n <= 1 {
            return vec![];
        }

        // Calculate n-1 threshold values that divide the domain into n segments
        (1..n)
            .map(|i| {
                let t = (i as f32) / (n as f32);
                self.domain.0 * (1.0 - t) + self.domain.1 * t
            })
            .collect()
    }

    pub fn scale<'a>(&self, values: impl Into<ScalarOrArrayRef<'a, f32>>) -> ScalarOrArray<R> {
        let n = self.range.len();

        // If there is only one range value, return it for all values
        if n == 1 {
            let r = self.range[0].clone();
            return values.into().map(|_| r.clone());
        }

        // Pre-compute scaling factors
        let domain_span = self.domain.1 - self.domain.0;
        let segments = n as f32;

        values.into().map(|x| {
            if x.is_finite() {
                let normalized = (x - self.domain.0) / domain_span;
                let idx = ((normalized * segments).floor() as usize).clamp(0, n - 1);
                self.range[idx].clone()
            } else {
                self.default.clone()
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AvengerScaleError;
    use float_cmp::assert_approx_eq;

    #[test]
    fn test_quantize_scale_basic() -> Result<(), AvengerScaleError> {
        let scale = QuantizeScale::new(
            vec![0.0, 0.5, 1.0],
            f32::NAN,
            &QuantizeScaleConfig {
                domain: (0.0, 1.0),
                ..Default::default()
            },
        );

        // Test array scaling with all test cases
        let values = vec![0.3, 0.5, 0.8];
        let result = scale.scale(&values).as_vec(values.len(), None);
        assert_eq!(result[0], 0.0);
        assert_eq!(result[1], 0.5);
        assert_eq!(result[2], 1.0);

        Ok(())
    }

    #[test]
    fn test_quantize_thresholds() {
        let scale = QuantizeScale::new(
            vec![0.0, 0.5, 1.0],
            f32::NAN,
            &QuantizeScaleConfig {
                domain: (0.0, 100.0),
                ..Default::default()
            },
        );

        let thresholds = scale.thresholds();
        assert_approx_eq!(f32, thresholds[0], 33.333332);
        assert_approx_eq!(f32, thresholds[1], 66.66667);
    }

    #[test]
    fn test_quantize_string_range() -> Result<(), AvengerScaleError> {
        let scale = QuantizeScale::new(
            vec!["small", "medium", "large"],
            "default",
            &QuantizeScaleConfig {
                domain: (0.0, 1.0),
                ..Default::default()
            },
        );

        let values = vec![0.3, 0.5, 0.8];
        let result = scale.scale(&values).as_vec(values.len(), None);

        assert_eq!(result[0], "small");
        assert_eq!(result[1], "medium");
        assert_eq!(result[2], "large");

        Ok(())
    }

    #[test]
    fn test_quantize_scale_nice() -> Result<(), AvengerScaleError> {
        let scale = QuantizeScale::new(
            vec![0.0, 25.0, 50.0, 75.0, 100.0],
            f32::NAN,
            &QuantizeScaleConfig {
                domain: (1.1, 10.9),
                ..Default::default()
            },
        )
        .nice(Some(5));

        // Domain should be extended to nice numbers
        let (start, end) = scale.domain();
        assert_approx_eq!(f32, start, 0.0);
        assert_approx_eq!(f32, end, 12.0);

        let values = vec![1.0, 6.0, 11.0];
        let result = scale.scale(&values).as_vec(values.len(), None);

        assert_eq!(result[0], 0.0); // Near start of domain
        assert_eq!(result[1], 50.0); // Middle of domain
        assert_eq!(result[2], 100.0); // Near end of domain

        Ok(())
    }
}
