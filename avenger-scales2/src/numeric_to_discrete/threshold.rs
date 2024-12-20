use crate::error::AvengerScaleError;

use super::{NumericToDiscreteScale, NumericToDiscreteScaleConfig};

#[derive(Debug, Clone)]
pub struct ThresholdScale;

impl NumericToDiscreteScale for ThresholdScale {
    fn scale(
        &self,
        config: &NumericToDiscreteScaleConfig,
        values: &[f32],
    ) -> Result<Vec<Option<usize>>, AvengerScaleError> {
        validate_threshold_config(config)?;
        // The domain are the thresholds
        let thresholds = &config.domain;

        Ok(values
            .iter()
            .map(|x| {
                if x.is_finite() {
                    let idx = match thresholds.binary_search_by(|t| t.partial_cmp(&x).unwrap()) {
                        Ok(i) => (i + 1) as usize,
                        Err(i) => i as usize,
                    };
                    Some(idx)
                } else {
                    None
                }
            })
            .collect())
    }
}

fn validate_threshold_config(
    config: &NumericToDiscreteScaleConfig,
) -> Result<(), AvengerScaleError> {
    // The domain are the thresholds
    let thresholds = &config.domain;

    // Validate the thresholds are in ascending order
    if !thresholds.windows(2).all(|w| w[0] <= w[1]) {
        return Err(AvengerScaleError::ThresholdsNotAscending(
            thresholds.clone(),
        ));
    }

    // Validate the range has the correct number of elements
    if config.range.len() != thresholds.len() + 1 {
        return Err(AvengerScaleError::ThresholdDomainMismatch {
            domain_len: thresholds.len(),
            range_len: config.range.len(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use float_cmp::assert_approx_eq;

    use crate::config::DiscreteRangeConfig;

    use super::*;

    #[test]
    fn test_threshold_scale_basic() -> Result<(), AvengerScaleError> {
        let config = NumericToDiscreteScaleConfig {
            domain: vec![30.0, 70.0],
            range: DiscreteRangeConfig::Numbers(vec![0.0, 1.0, 2.0]),
            options: HashMap::new(),
        };
        let scale = ThresholdScale;

        let values = vec![20.0, 50.0, 80.0];
        let result = scale.scale(&config, &values)?;

        assert_eq!(result[0], Some(0)); // 20.0 < 30.0
        assert_eq!(result[1], Some(1)); // 30.0 <= 50.0 < 70.0
        assert_eq!(result[2], Some(2)); // 80.0 >= 70.0

        Ok(())
    }

    #[test]
    fn test_validate_range_length() -> Result<(), AvengerScaleError> {
        // Tese are fine
        validate_threshold_config(&NumericToDiscreteScaleConfig {
            domain: vec![-1.0, 1.0],
            range: DiscreteRangeConfig::Numbers(vec![0.0, 1.0, 2.0]),
            options: HashMap::new(),
        })?;
        validate_threshold_config(&NumericToDiscreteScaleConfig {
            domain: vec![-1.0, 1.0, 3.0, 3.0],
            range: DiscreteRangeConfig::Indices(vec![0, 1, 2, 3, 4]),
            options: HashMap::new(),
        })?;

        // Invalid number of elements between thresholds and range
        let err = validate_threshold_config(&NumericToDiscreteScaleConfig {
            domain: vec![-1.0, 1.0, 3.0, 3.0],
            range: DiscreteRangeConfig::Numbers(vec![0.0, 1.0, 2.0, 3.0]),
            options: HashMap::new(),
        })
        .unwrap_err();
        assert_eq!(
            err,
            AvengerScaleError::ThresholdDomainMismatch {
                domain_len: 4,
                range_len: 4,
            }
        );

        // Non-ascending thresholds
        let err = validate_threshold_config(&NumericToDiscreteScaleConfig {
            domain: vec![-1.0, 1.0, 4.0, 3.0],
            range: DiscreteRangeConfig::Indices(vec![0, 1, 2, 3, 4]),
            options: HashMap::new(),
        })
        .unwrap_err();
        assert_eq!(
            err,
            AvengerScaleError::ThresholdsNotAscending(vec![-1.0, 1.0, 4.0, 3.0])
        );

        Ok(())
    }
}
