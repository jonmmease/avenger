use avenger_common::value::ScalarOrArray;

use crate::{
    config::{DiscreteRangeConfig, ScaleConfigScalarMapUtils},
    discrete_to_discrete::{
        ordinal::OrdinalScale, DiscreteToDiscreteScale, DiscreteToDiscreteScaleConfig,
    },
    error::AvengerScaleError,
};

use super::{DiscreteToNumericScale, DiscreteToNumericScaleConfig};

pub struct BandScale;

impl DiscreteToNumericScale for BandScale {
    fn scale_numbers(
        &self,
        config: &DiscreteToNumericScaleConfig,
        values: &[f32],
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        let ordinal_scale = OrdinalScale;
        let range_values = build_range_values(config)?;

        // create ordinal scale config. Range is not used because we'll use the resulting indices
        let ordinal_scale_config = DiscreteToDiscreteScaleConfig {
            domain: config.domain.clone(),
            range: DiscreteRangeConfig::Numbers(vec![0.0; range_values.len()]),
            default_value: None,
        };

        let range_indices = ordinal_scale.scale_numbers(&ordinal_scale_config, values)?;
        let range_values = range_indices
            .iter()
            .map(|i| i.map(|i| range_values[i]).unwrap_or(f32::NAN))
            .collect::<Vec<_>>();

        Ok(ScalarOrArray::new_array(range_values))
    }

    fn scale_strings(
        &self,
        config: &DiscreteToNumericScaleConfig,
        values: &[String],
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        let ordinal_scale = OrdinalScale;
        let range_values = build_range_values(config)?;

        // create ordinal scale config. Range is not used because we'll use the resulting indices
        let ordinal_scale_config = DiscreteToDiscreteScaleConfig {
            domain: config.domain.clone(),
            range: DiscreteRangeConfig::Numbers(vec![0.0; range_values.len()]),
            default_value: None,
        };

        let range_indices = ordinal_scale.scale_strings(&ordinal_scale_config, values)?;
        let range_values = range_indices
            .iter()
            .map(|i| i.map(|i| range_values[i]).unwrap_or(f32::NAN))
            .collect::<Vec<_>>();

        Ok(ScalarOrArray::new_array(range_values))
    }

    fn scale_indices(
        &self,
        config: &DiscreteToNumericScaleConfig,
        values: &[usize],
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        let ordinal_scale = OrdinalScale;
        let range_values = build_range_values(config)?;

        // create ordinal scale config. Range is not used because we'll use the resulting indices
        let ordinal_scale_config = DiscreteToDiscreteScaleConfig {
            domain: config.domain.clone(),
            range: DiscreteRangeConfig::Numbers(vec![0.0; range_values.len()]),
            default_value: None,
        };

        let range_indices = ordinal_scale.scale_indices(&ordinal_scale_config, values)?;
        let range_values = range_indices
            .iter()
            .map(|i| i.map(|i| range_values[i]).unwrap_or(f32::NAN))
            .collect::<Vec<_>>();

        Ok(ScalarOrArray::new_array(range_values))
    }

    fn invert(
        &self,
        config: &DiscreteToNumericScaleConfig,
        range: (f32, f32),
    ) -> Result<Vec<usize>, AvengerScaleError> {
        let (mut lo, mut hi) = range;

        // Bail if range values are invalid
        if lo.is_nan() || hi.is_nan() {
            return Ok(vec![]);
        }

        // Order range inputs
        if hi < lo {
            std::mem::swap(&mut lo, &mut hi);
        }

        let reverse = config.range.1 < config.range.0;
        let (start, stop) = if reverse {
            (config.range.1, config.range.0)
        } else {
            (config.range.0, config.range.1)
        };

        // Bail if outside scale range
        if hi < start || lo > stop {
            return Ok(vec![]);
        }

        // Calculate band positions
        let values = build_range_values(config)?;

        // Binary search for indices
        let mut a = values.partition_point(|&x| x <= lo).saturating_sub(1);
        let b = if (lo - hi).abs() < f32::EPSILON {
            a
        } else {
            values.partition_point(|&x| x <= hi).saturating_sub(1)
        };

        // Increment index if lo is within padding gap
        if lo - values[a] > bandwidth(config) + 1e-10 {
            a += 1;
        }

        // Handle reverse
        let (a, b) = if reverse {
            let n = values.len() - 1;
            (n - b, n - a)
        } else {
            (a, b)
        };

        if a > b {
            return Ok(vec![]);
        }

        Ok((a..=b).map(|i| i).collect())
    }
}

fn build_range_values(
    config: &DiscreteToNumericScaleConfig,
) -> Result<Vec<f32>, AvengerScaleError> {
    let n = config.domain.len();

    if n == 0 {
        return Err(AvengerScaleError::EmptyDomain);
    }

    let align = config.options.try_get_f32("align").unwrap_or(0.5);
    let band = config.options.try_get_f32("band").unwrap_or(0.0);
    let padding_inner = config.options.try_get_f32("padding_inner").unwrap_or(0.0);
    let padding_outer = config.options.try_get_f32("padding_outer").unwrap_or(0.0);
    let round = config.round;
    let range_offset = config.range_offset;
    let range = config.range;

    if align < 0.0 || align > 1.0 || !align.is_finite() {
        return Err(AvengerScaleError::InvalidScalePropertyValue(format!(
            "align is {} but must be between 0 and 1",
            align
        )));
    }

    if band < 0.0 || band > 1.0 || !band.is_finite() {
        return Err(AvengerScaleError::InvalidScalePropertyValue(format!(
            "band is {} but must be between 0 and 1",
            band
        )));
    }

    if padding_inner < 0.0 || padding_inner > 1.0 || !padding_inner.is_finite() {
        return Err(AvengerScaleError::InvalidScalePropertyValue(format!(
            "padding_inner is {} but must be between 0 and 1",
            padding_inner
        )));
    }

    if padding_outer < 0.0 || !padding_outer.is_finite() {
        return Err(AvengerScaleError::InvalidScalePropertyValue(format!(
            "padding_outer is {} but must be non-negative",
            padding_outer
        )));
    }

    if !range.0.is_finite() || !range.1.is_finite() {
        return Err(AvengerScaleError::InvalidScalePropertyValue(format!(
            "range is ({}, {}) but both ends must be finite",
            range.0, range.1
        )));
    }

    let reverse = range.1 < range.0;
    let (start, stop) = if reverse {
        (range.1, range.0)
    } else {
        (range.0, range.1)
    };

    let step = (stop - start) / 1.0_f32.max(bandspace(n, Some(padding_inner), Some(padding_outer)));
    let step = if round { step.floor() } else { step };

    let start = start + (stop - start - step * (n as f32 - padding_inner)) * align;
    let start = if round { start.round() } else { start };

    // Generate range values
    let range_values: Vec<f32> = (0..n).map(|i| start + step * i as f32).collect::<Vec<_>>();

    // compute band offset
    let band_offset = if band != 0.0 || range_offset != 0.0 {
        bandwidth(config) * band + range_offset
    } else {
        0.0
    };

    // Create final range values considering reverse and band offset
    let range_values = if reverse {
        range_values.into_iter().rev().collect()
    } else {
        range_values
    }
    .iter()
    .map(|v| v + band_offset)
    .collect::<Vec<_>>();

    // Create ordinal scale and map values
    Ok(range_values)
}

/// Returns the width of each band.
///
/// Calculated from range, domain size, and padding settings.
/// Returns 0 for empty domains.
pub fn bandwidth(config: &DiscreteToNumericScaleConfig) -> f32 {
    let n = config.domain.len();
    if n == 0 {
        return 0.0;
    }

    let range = config.range;
    let padding_inner = config.options.try_get_f32("padding_inner").unwrap_or(0.0);
    let padding_outer = config.options.try_get_f32("padding_outer").unwrap_or(0.0);

    let (start, stop) = if range.1 < range.0 {
        (range.1, range.0)
    } else {
        (range.0, range.1)
    };

    let step = (stop - start) / 1.0_f32.max(bandspace(n, Some(padding_inner), Some(padding_outer)));
    let bandwidth = step * (1.0 - padding_inner);

    if config.round {
        bandwidth.round()
    } else {
        bandwidth
    }
}

/// Returns the distance between the starts of adjacent bands.
///
/// The step size is calculated based on the range, domain size, and padding settings.
/// Returns 0 if the domain is empty.
pub fn step(config: &DiscreteToNumericScaleConfig) -> f32 {
    let n = config.domain.len();
    if n == 0 {
        return 0.0;
    }

    let (start, stop) = if config.range.1 < config.range.0 {
        (config.range.1, config.range.0)
    } else {
        (config.range.0, config.range.1)
    };

    let padding_inner = config.options.try_get_f32("padding_inner").unwrap_or(0.0);
    let padding_outer = config.options.try_get_f32("padding_outer").unwrap_or(0.0);

    let step = (stop - start) / 1.0_f32.max(bandspace(n, Some(padding_inner), Some(padding_outer)));

    if config.round {
        step.floor()
    } else {
        step
    }
}

/// Calculates required steps for a band scale based on domain count and padding.
///
/// # Arguments
/// * `count` - Number of domain elements
/// * `padding_inner` - Inner padding [0.0, 1.0], defaults to 0.0
/// * `padding_outer` - Outer padding ≥ 0.0, defaults to 0.0
pub fn bandspace(count: usize, padding_inner: Option<f32>, padding_outer: Option<f32>) -> f32 {
    let padding_inner = padding_inner.unwrap_or(0.0).clamp(0.0, 1.0);
    let padding_outer = padding_outer.unwrap_or(0.0).max(0.0);

    let count = count as f32;
    count - padding_inner + padding_outer * 2.0
}

mod tests {
    use std::collections::HashMap;

    // use float_cmp::{assert_approx_eq, F32Margin};

    use float_cmp::assert_approx_eq;

    use crate::config::ScaleConfigScalar;

    use super::*;
    // use float_cmp::{assert_approx_eq, F32Margin};

    #[test]
    fn test_band_scale_basic() -> Result<(), AvengerScaleError> {
        let domain = vec!["a", "b", "c"];
        let scale = BandScale;
        let config = DiscreteToNumericScaleConfig {
            domain: domain.into(),
            range: (0.0, 1.0),
            options: vec![("align".to_string(), 0.5.into())]
                .into_iter()
                .collect(),

            round: false,
            range_offset: 0.0,
        };

        let values: Vec<_> = vec!["a", "b", "b", "c", "f"]
            .into_iter()
            .map(|s| s.to_string())
            .collect();
        let result = scale
            .scale_strings(&config, &values)?
            .as_vec(values.len(), None);

        // With 3 bands in [0,1] and no padding, expect bands at 0.0, 0.333, 0.667
        assert_approx_eq!(f32, result[0], 0.0); // "a"
        assert_approx_eq!(f32, result[1], 0.3333333); // "b"
        assert_approx_eq!(f32, result[2], 0.3333333); // "b"
        assert_approx_eq!(f32, result[3], 0.6666667); // "c"
        assert!(result[4].is_nan()); // "f"
        assert_approx_eq!(f32, bandwidth(&config), 0.3333333);
        assert_approx_eq!(f32, step(&config), 0.3333333);

        Ok(())
    }

    #[test]
    fn test_band_scale_padding() -> Result<(), AvengerScaleError> {
        let domain = vec!["a", "b", "c"];
        let scale = BandScale;
        let config = DiscreteToNumericScaleConfig {
            domain: domain.into(),
            range: (0.0, 120.0),
            options: vec![
                ("padding_inner".to_string(), 0.2.into()),
                ("padding_outer".to_string(), 0.2.into()),
            ]
            .into_iter()
            .collect(),

            round: false,
            range_offset: 0.0,
        };

        let values: Vec<_> = vec!["a", "b", "b", "c", "f"]
            .into_iter()
            .map(|s| s.to_string())
            .collect();
        let result = scale
            .scale_strings(&config, &values)?
            .as_vec(values.len(), None);

        // With padding of 0.2, points should be inset
        assert_approx_eq!(f32, result[0], 7.5); // "a"
        assert_approx_eq!(f32, result[1], 45.0); // "b"
        assert_approx_eq!(f32, result[2], 45.0); // "b"
        assert_approx_eq!(f32, result[3], 82.5); // "c"
        assert!(result[4].is_nan()); // "f"

        assert_approx_eq!(f32, bandwidth(&config), 30.0);

        Ok(())
    }

    #[test]
    fn test_band_scale_round() -> Result<(), AvengerScaleError> {
        let domain = vec!["a", "b", "c"];
        let scale = BandScale;
        let config = DiscreteToNumericScaleConfig {
            domain: domain.into(),
            range: (0.0, 100.0),
            round: true,
            range_offset: 0.0,
            options: HashMap::new(),
        };

        let values: Vec<_> = vec!["a", "b", "b", "c", "f"]
            .into_iter()
            .map(|s| s.to_string())
            .collect();
        let result = scale
            .scale_strings(&config, &values)?
            .as_vec(values.len(), None);

        // With rounding, values should be integers
        assert_eq!(result[0], 1.0); // "a"
        assert_eq!(result[1], 34.0); // "b"
        assert_eq!(result[2], 34.0); // "b"
        assert_eq!(result[3], 67.0); // "c"
        assert!(result[4].is_nan()); // "f"
        assert_eq!(bandwidth(&config), 33.0);

        Ok(())
    }

    #[test]
    fn test_bandspace() {
        // Test default padding (0, 0)
        assert_eq!(bandspace(3, None, None), 3.0);

        // Test with only inner padding
        assert_eq!(bandspace(3, Some(0.2), None), 2.8);

        // Test with only outer padding
        assert_eq!(bandspace(3, None, Some(0.5)), 4.0);

        // Test with both inner and outer padding
        assert_eq!(bandspace(3, Some(0.2), Some(0.5)), 3.8);

        // Test with invalid paddings (should clamp)
        assert_eq!(bandspace(3, Some(1.5), Some(-0.5)), 2.0); // inner clamped to 1.0, outer clamped to 0.0
    }

    #[test]
    fn test_band_scale_invert() -> Result<(), AvengerScaleError> {
        let domain = vec!["a", "b", "c"];
        let scale = BandScale;
        let config = DiscreteToNumericScaleConfig {
            domain: domain.into(),
            range: (0.0, 120.0),
            round: false,
            range_offset: 0.0,
            options: vec![
                ("padding_inner".to_string(), 0.2.into()),
                ("padding_outer".to_string(), 0.2.into()),
            ]
            .into_iter()
            .collect(),
        };

        // Test exact band positions
        let result = scale.invert(&config, (7.5, 7.5)).unwrap();
        assert_eq!(result, vec![0]);

        let result = scale.invert(&config, (45.0, 45.0)).unwrap();
        assert_eq!(result, vec![1]);

        // Test position within band
        let result = scale.invert(&config, (15.0, 15.0)).unwrap();
        assert_eq!(result, vec![0]);

        // Test position in padding (should return None)
        assert!(scale.invert(&config, (40.0, 40.0))?.is_empty());

        // Test out of range
        assert!(scale.invert(&config, (-10.0, -10.0))?.is_empty());
        assert!(scale.invert(&config, (130.0, 130.0))?.is_empty());

        Ok(())
    }

    #[test]
    fn test_band_scale_invert_range() -> Result<(), AvengerScaleError> {
        let domain = vec!["a", "b", "c"];
        let scale = BandScale;
        let config = DiscreteToNumericScaleConfig {
            domain: domain.into(),
            range: (0.0, 120.0),
            round: false,
            range_offset: 0.0,
            options: vec![
                ("padding_inner".to_string(), 0.2.into()),
                ("padding_outer".to_string(), 0.2.into()),
            ]
            .into_iter()
            .collect(),
        };

        // Test range covering multiple bands
        let result = scale.invert(&config, (7.5, 82.5)).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], 0);
        assert_eq!(result[1], 1);
        assert_eq!(result[2], 2);

        // Test partial range
        let result = scale.invert(&config, (45.0, 82.5)).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], 1);
        assert_eq!(result[1], 2);

        // Test reversed range (should handle automatically)
        let result = scale.invert(&config, (82.5, 45.0)).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], 1);
        assert_eq!(result[1], 2);

        // Test out of range
        assert!(scale.invert(&config, (-10.0, -5.0))?.is_empty());
        assert!(scale.invert(&config, (130.0, 140.0))?.is_empty());

        // Test invalid range (NaN)
        assert!(scale.invert(&config, (f32::NAN, 50.0))?.is_empty());

        Ok(())
    }

    #[test]
    fn test_basic_band() -> Result<(), AvengerScaleError> {
        let scale = BandScale;
        let config = DiscreteToNumericScaleConfig {
            domain: vec!["A", "B", "C"].into(),
            range: (0.0, 1.0),
            round: false,
            range_offset: 0.0,
            options: HashMap::new(),
        };
        let values = vec!["A".into(), "B".into(), "C".into()];
        let result = scale
            .scale_strings(&config, &values)?
            .as_vec(values.len(), None);

        let margin = F32Margin {
            epsilon: 0.0001,
            ..Default::default()
        };

        assert_approx_eq!(f32, result[0], 0.0, margin);
        assert_approx_eq!(f32, result[1], 0.333333, margin);
        assert_approx_eq!(f32, result[2], 0.666667, margin);
        Ok(())
    }

    #[test]
    fn test_band_position() -> Result<(), AvengerScaleError> {
        let scale = BandScale;
        let config = DiscreteToNumericScaleConfig {
            domain: vec!["A", "B", "C"].into(),
            range: (0.0, 1.0),
            round: false,
            range_offset: 0.0,
            options: vec![("band".to_string(), 0.5.into())].into_iter().collect(),
        };
        let values = vec!["A".into(), "B".into(), "C".into()];
        let result = scale
            .scale_strings(&config, &values)?
            .as_vec(values.len(), None);
        let margin = F32Margin {
            epsilon: 0.0001,
            ..Default::default()
        };
        assert_approx_eq!(f32, result[0], 0.166667, margin);
        assert_approx_eq!(f32, result[1], 0.5, margin);
        assert_approx_eq!(f32, result[2], 0.833333, margin);
        Ok(())
    }

    #[test]
    fn test_band_position_offset() -> Result<(), AvengerScaleError> {
        let scale = BandScale;
        let config = DiscreteToNumericScaleConfig {
            domain: vec!["A", "B", "C"].into(),
            range: (0.0, 1.0),
            round: false,
            range_offset: 1.0,
            options: vec![("band".to_string(), 0.5.into())].into_iter().collect(),
        };
        let values = vec!["A".into(), "B".into(), "C".into()];
        let result = scale
            .scale_strings(&config, &values)?
            .as_vec(values.len(), None);

        let margin = F32Margin {
            epsilon: 0.0001,
            ..Default::default()
        };
        assert_approx_eq!(f32, result[0], 1.166667, margin);
        assert_approx_eq!(f32, result[1], 1.5, margin);
        assert_approx_eq!(f32, result[2], 1.833333, margin);
        Ok(())
    }
}
