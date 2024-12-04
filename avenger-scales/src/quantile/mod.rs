use avenger_common::value::{ScalarOrArray, ScalarOrArrayRef};

use crate::error::AvengerScaleError;
use std::fmt::Debug;

#[derive(Debug, Clone)]
pub struct QuantileScaleConfig {
    /// The domain as a sample population
    pub domain: Vec<f32>,
}

impl Default for QuantileScaleConfig {
    fn default() -> Self {
        Self {
            domain: vec![0.0, 1.0],
        }
    }
}

/// A quantile scale maps a continuous domain to discrete values based on sample quantiles.
///
/// Unlike quantize scales which create uniform segments, quantile scales create segments
/// based on the distribution of values in the domain. The domain is specified as a sample
/// population, and thresholds are computed to create groups of equal size.
#[derive(Debug, Clone)]
pub struct QuantileScale<R>
where
    R: Clone + Debug + Sync + 'static,
{
    domain: Vec<f32>,
    range: Vec<R>,
    default: R,
    thresholds: Vec<f32>,
}

impl<R> QuantileScale<R>
where
    R: Clone + Debug + Sync + 'static,
{
    pub fn try_new(
        range: Vec<R>,
        default: R,
        config: &QuantileScaleConfig,
    ) -> Result<Self, AvengerScaleError> {
        if range.is_empty() {
            return Err(AvengerScaleError::EmptyRange);
        }
        let mut this = Self {
            domain: config.domain.clone(),
            range,
            default,
            thresholds: vec![],
        };
        this.update_thresholds();
        Ok(this)
    }

    /// Sets the domain from a sample population
    pub fn with_domain(mut self, domain: Vec<f32>) -> Result<Self, AvengerScaleError> {
        if domain.is_empty() {
            return Err(AvengerScaleError::EmptyDomain);
        }
        self.domain = domain;
        self.update_thresholds();
        Ok(self)
    }

    /// Sets the output range as an Arrow array
    pub fn with_range(mut self, range: Vec<R>) -> Result<Self, AvengerScaleError> {
        if range.is_empty() {
            return Err(AvengerScaleError::EmptyRange);
        }
        self.range = range;
        self.update_thresholds();
        Ok(self)
    }

    /// Returns the sample population domain
    pub fn get_domain(&self) -> &[f32] {
        &self.domain
    }

    /// Returns a reference to the output range
    pub fn get_range(&self) -> &Vec<R> {
        &self.range
    }

    /// Returns the computed quantile thresholds
    pub fn quantiles(&self) -> &[f32] {
        &self.thresholds
    }

    fn update_thresholds(&mut self) {
        let n = self.range.len();
        if n <= 1 || self.domain.is_empty() {
            self.thresholds = vec![];
            return;
        }

        // Sort domain values
        let mut sorted_domain = self.domain.clone();
        sorted_domain.sort_by(|a, b| a.partial_cmp(b).unwrap());

        // Compute n-1 quantile thresholds
        self.thresholds = (1..n)
            .map(|i| {
                let k = (sorted_domain.len() * i) / n;
                sorted_domain[k]
            })
            .collect();
    }

    pub fn scale<'a>(&self, values: impl Into<ScalarOrArrayRef<'a, f32>>) -> ScalarOrArray<R> {
        let n = self.range.len();

        if n == 1 {
            let r = self.range[0].clone();
            return values.into().map(|_| r.clone());
        }

        values.into().map(|x| {
            if x.is_finite() {
                // Find index using binary search on thresholds
                let idx = match self
                    .thresholds
                    .binary_search_by(|t| t.partial_cmp(&x).unwrap())
                {
                    Ok(i) => (i + 1) as usize,
                    Err(i) => i as usize,
                };
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
    use float_cmp::assert_approx_eq;

    #[test]
    fn test_quantile_scale_basic() -> Result<(), AvengerScaleError> {
        // Create sample population with skewed distribution
        let domain = vec![1.0, 1.0, 2.0, 3.0, 3.0, 3.0, 4.0, 4.0, 5.0];
        let scale = QuantileScale::try_new(
            vec!["small", "medium", "large"],
            "default",
            &QuantileScaleConfig { domain },
        )?;

        // Check quantile thresholds
        let thresholds = scale.quantiles();
        assert_approx_eq!(f32, thresholds[0], 3.0); // First third of values: [1,1,2]
        assert_approx_eq!(f32, thresholds[1], 4.0); // Second third: [3,3,3]
                                                    // Last third: [4,4,5]

        // Test mapping values
        let values = vec![1.5, 3.0, 4.5, f32::NAN];
        let result = scale.scale(&values).as_vec(values.len(), None);

        assert_eq!(result[0], "small"); // 1.5 < 3.0
        assert_eq!(result[1], "medium"); // 3.0 < 4.0
        assert_eq!(result[2], "large"); // 4.5 >= 4.0
        assert_eq!(result[3], "default"); // non-finite

        Ok(())
    }
}
