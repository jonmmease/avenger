use avenger_common::value::{ScalarOrArray, ScalarOrArrayRef};

use crate::error::AvengerScaleError;
use std::fmt::Debug;

/// A threshold scale maps continuous values to discrete values based on explicit threshold boundaries.
///
/// Unlike quantize scales which create uniform segments, threshold scales let you specify
/// the exact boundary points between segments.
#[derive(Debug, Clone)]
pub struct ThresholdScale<R>
where
    R: Clone + Debug + Sync + 'static,
{
    thresholds: Vec<f32>,
    range: Vec<R>,
    default: R,
}

impl<R> ThresholdScale<R>
where
    R: Clone + Debug + Sync + 'static,
{
    pub fn try_new(
        range: Vec<R>,
        thresholds: Vec<f32>,
        default: R,
    ) -> Result<Self, AvengerScaleError> {
        if !thresholds.windows(2).all(|w| w[0] <= w[1]) {
            return Err(AvengerScaleError::ThresholdsNotAscending(thresholds));
        }

        if range.len() != thresholds.len() + 1 {
            return Err(AvengerScaleError::ThresholdDomainMismatch {
                domain_len: thresholds.len(),
                range_len: range.len(),
            });
        }
        Ok(Self {
            thresholds,
            range,
            default,
        })
    }

    pub fn with_domain_and_range(
        mut self,
        domain: Vec<f32>,
        range: Vec<R>,
    ) -> Result<Self, AvengerScaleError> {
        if domain.len() != range.len() + 1 {
            return Err(AvengerScaleError::ThresholdDomainMismatch {
                domain_len: domain.len(),
                range_len: self.range.len(),
            });
        }
        self.thresholds = domain;
        self.range = range;
        Ok(self)
    }

    /// Returns a reference to the threshold values
    pub fn thresholds(&self) -> &[f32] {
        &self.thresholds
    }

    /// Returns a reference to the output range
    pub fn range(&self) -> &Vec<R> {
        &self.range
    }

    /// Returns the default value
    pub fn default(&self) -> &R {
        &self.default
    }

    pub fn scale<'a>(&self, values: impl Into<ScalarOrArrayRef<'a, f32>>) -> ScalarOrArray<R> {
        let thresholds = &self.thresholds;

        values.into().map(|x| {
            if x.is_finite() {
                let idx = match thresholds.binary_search_by(|t| t.partial_cmp(&x).unwrap()) {
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
    use float_cmp::assert_approx_eq;

    use super::*;
    use crate::threshold::ThresholdScale;

    #[test]
    fn test_threshold_scale_basic() -> Result<(), AvengerScaleError> {
        let scale =
            ThresholdScale::try_new(vec!["low", "medium", "high"], vec![30.0, 70.0], "default")?;

        let values = vec![20.0, 50.0, 80.0];
        let result = scale.scale(&values).as_vec(values.len(), None);

        assert_eq!(result[0], "low"); // 20.0 < 30.0
        assert_eq!(result[1], "medium"); // 30.0 <= 50.0 < 70.0
        assert_eq!(result[2], "high"); // 80.0 >= 70.0

        Ok(())
    }

    #[test]
    fn test_threshold_scale_numeric() -> Result<(), AvengerScaleError> {
        let scale = ThresholdScale::try_new(vec![-1.0, 1.0], vec![0.0], f32::NAN)?;

        let values = vec![-0.5, 0.0, 0.5];
        let result = scale.scale(&values).as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], -1.0); // -0.5 < 0.0
        assert_approx_eq!(f32, result[1], 1.0); // 0.0 >= 0.0
        assert_approx_eq!(f32, result[2], 1.0); // 0.5 > 0.0

        Ok(())
    }

    #[test]
    fn test_validate_range_length() -> Result<(), AvengerScaleError> {
        // Tese are fine
        let _ = ThresholdScale::try_new(vec![-1.0, 1.0, 3.0], vec![0.0, 1.0], f32::NAN)?;
        let _ = ThresholdScale::try_new(vec![-1.0, 1.0], vec![0.0], f32::NAN)?;
        let _ = ThresholdScale::try_new(vec![-1.0, 1.0, 3.0, 3.0], vec![0.0, 1.0, 2.0], f32::NAN)?;

        // This is bad
        let err = ThresholdScale::try_new(vec![-1.0, 1.0], vec![0.0, 1.0, 2.0, 3.0], f32::NAN)
            .unwrap_err();
        assert_eq!(
            err,
            AvengerScaleError::ThresholdDomainMismatch {
                domain_len: 4,
                range_len: 2,
            }
        );
        Ok(())
    }
}
