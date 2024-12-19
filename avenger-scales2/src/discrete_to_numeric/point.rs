use avenger_common::value::ScalarOrArray;

use crate::{config::ScaleConfigScalarMapUtils, error::AvengerScaleError};

use super::{band::BandScale, DiscreteToNumericScale, DiscreteToNumericScaleConfig};

pub struct PointScale;

impl DiscreteToNumericScale for PointScale {
    fn scale_numbers(
        &self,
        config: &DiscreteToNumericScaleConfig,
        values: &[f32],
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        let band_scale = BandScale;
        let band_config = point_to_band_config(config);
        band_scale.scale_numbers(&band_config, values)
    }

    fn scale_strings(
        &self,
        config: &DiscreteToNumericScaleConfig,
        values: &[String],
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        let band_scale = BandScale;
        let band_config = point_to_band_config(config);
        band_scale.scale_strings(&band_config, values)
    }

    fn scale_indices(
        &self,
        config: &DiscreteToNumericScaleConfig,
        values: &[usize],
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        let band_scale = BandScale;
        let band_config = point_to_band_config(config);
        band_scale.scale_indices(&band_config, values)
    }

    /// Invert from numeric range interval to the discrete domain values that map to that interval
    fn invert(
        &self,
        config: &DiscreteToNumericScaleConfig,
        range: (f32, f32),
    ) -> Result<Vec<usize>, AvengerScaleError> {
        let band_scale = BandScale;
        let band_config = point_to_band_config(config);
        band_scale.invert(&band_config, range)
    }
}

fn point_to_band_config(
    point_config: &DiscreteToNumericScaleConfig,
) -> DiscreteToNumericScaleConfig {
    // Map padding to both padding
    let padding_outer = point_config.options.try_get_f32("padding").unwrap_or(0.0);
    let align = point_config.options.try_get_f32("align").unwrap_or(0.5);

    DiscreteToNumericScaleConfig {
        domain: point_config.domain.clone(),
        range: point_config.range,
        round: point_config.round,
        range_offset: point_config.range_offset,
        options: vec![
            // padding_inner is always 1.0 for point scale
            ("padding_inner".to_string(), 1.0.into()),
            // padding becomes padding_outer
            ("padding_outer".to_string(), padding_outer.into()),
            // band is always 0.0 (though it doesn't matter when padding_inner is 1.0)
            ("band".to_string(), 0.0.into()),
            // align passes through
            ("align".to_string(), align.into()),
        ]
        .into_iter()
        .collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use float_cmp::assert_approx_eq;

    #[test]
    fn test_point_scale_basic() -> Result<(), AvengerScaleError> {
        let config = DiscreteToNumericScaleConfig {
            domain: vec!["a", "b", "c"].into(),
            range: (0.0, 1.0),
            round: false,
            range_offset: 0.0,
            options: HashMap::new(),
        };
        let scale = PointScale;

        let values = vec!["a", "b", "b", "c", "f"]
            .into_iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();

        let result = scale
            .scale_strings(&config, &values)?
            .as_vec(values.len(), None);

        // With 3 points in [0,1], expect points at 0.0, 0.5, 1.0
        assert_approx_eq!(f32, result[0], 0.0); // "a"
        assert_approx_eq!(f32, result[1], 0.5); // "b"
        assert_approx_eq!(f32, result[2], 0.5); // "b"
        assert_approx_eq!(f32, result[3], 1.0); // "c"
        assert!(result[4].is_nan()); // "f"

        Ok(())
    }

    #[test]
    fn test_point_scale_custom_range() -> Result<(), AvengerScaleError> {
        let config = DiscreteToNumericScaleConfig {
            domain: vec!["a", "b", "c"].into(),
            range: (0.0, 100.0),
            round: false,
            range_offset: 0.0,
            options: HashMap::new(),
        };
        let scale = PointScale;

        let values = vec!["a", "b", "b", "c", "f"]
            .into_iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        let result = scale
            .scale_strings(&config, &values)?
            .as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 0.0); // "a"
        assert_approx_eq!(f32, result[1], 50.0); // "b"
        assert_approx_eq!(f32, result[2], 50.0); // "b"
        assert_approx_eq!(f32, result[3], 100.0); // "c"
        assert!(result[4].is_nan()); // "f"
        Ok(())
    }

    #[test]
    fn test_point_scale_with_padding() -> Result<(), AvengerScaleError> {
        let config = DiscreteToNumericScaleConfig {
            domain: vec!["a", "b", "c"].into(),
            range: (0.0, 100.0),
            round: false,
            range_offset: 0.0,
            options: HashMap::from([("padding".to_string(), 0.5.into())]),
        };
        let scale = PointScale;

        let values = vec!["a", "b", "b", "c", "f"]
            .into_iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        let result = scale
            .scale_strings(&config, &values)?
            .as_vec(values.len(), None);

        // With padding of 0.5, points should be at specific positions
        assert_approx_eq!(f32, result[0], 16.666667); // "a"
        assert_approx_eq!(f32, result[1], 50.0); // "b"
        assert_approx_eq!(f32, result[2], 50.0); // "b"
        assert_approx_eq!(f32, result[3], 83.333333); // "c"
        assert!(result[4].is_nan()); // "f"

        Ok(())
    }

    #[test]
    fn test_point_scale_round() -> Result<(), AvengerScaleError> {
        let config = DiscreteToNumericScaleConfig {
            domain: vec!["a", "b", "c", "d"].into(),
            range: (0.0, 100.0),
            round: true,
            range_offset: 0.0,
            options: HashMap::new(),
        };
        let scale = PointScale;

        let values = vec!["a", "b", "b", "c", "d", "f"]
            .into_iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        let result = scale
            .scale_strings(&config, &values)?
            .as_vec(values.len(), None);

        // With 4 points in [0,100] and rounding, expect points at 0, 34, 67, 100
        // Confirmed with d3-scale
        assert_approx_eq!(f32, result[0], 1.0); // "a"
        assert_approx_eq!(f32, result[1], 34.0); // "b"
        assert_approx_eq!(f32, result[2], 34.0); // "b"
        assert_approx_eq!(f32, result[3], 67.0); // "c"
        assert_approx_eq!(f32, result[4], 100.0); // "d"
        assert!(result[5].is_nan()); // "f"
        Ok(())
    }

    #[test]
    fn test_point_scale_range_offset() -> Result<(), AvengerScaleError> {
        let config = DiscreteToNumericScaleConfig {
            domain: vec!["a", "b", "c", "d"].into(),
            range: (0.0, 100.0),
            round: true,
            range_offset: 1.0,
            options: HashMap::new(),
        };
        let scale = PointScale;

        let values = vec!["a", "b", "b", "c", "d", "f"]
            .into_iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        let result = scale
            .scale_strings(&config, &values)?
            .as_vec(values.len(), None);

        // With 4 points in [0,100] and rounding, expect points at 0, 34, 67, 100
        // Confirmed with d3-scale
        assert_approx_eq!(f32, result[0], 2.0); // "a"
        assert_approx_eq!(f32, result[1], 35.0); // "b"
        assert_approx_eq!(f32, result[2], 35.0); // "b"
        assert_approx_eq!(f32, result[3], 68.0); // "c"
        assert_approx_eq!(f32, result[4], 101.0); // "d"
        assert!(result[5].is_nan()); // "f"
        Ok(())
    }

    #[test]
    fn test_point_scale_invert() -> Result<(), AvengerScaleError> {
        let scale = PointScale;
        let config = DiscreteToNumericScaleConfig {
            domain: vec!["a", "b", "c"].into(),
            range: (0.0, 100.0),
            round: false,
            range_offset: 0.0,
            options: HashMap::new(),
        };

        // Test exact point positions
        let result = scale.invert(&config, (0.0, 0.0))?;
        assert_eq!(result, vec![0]);

        let result = scale.invert(&config, (50.0, 50.0))?;
        assert_eq!(result, vec![1]);

        let result = scale.invert(&config, (100.0, 100.0))?;
        assert_eq!(result, vec![2]);

        // Test positions between points (should return None)
        assert!(scale.invert(&config, (25.0, 25.0))?.is_empty());
        assert!(scale.invert(&config, (75.0, 75.0))?.is_empty());

        // Test out of range
        assert!(scale.invert(&config, (-10.0, -10.0))?.is_empty());
        assert!(scale.invert(&config, (110.0, 110.0))?.is_empty());

        Ok(())
    }

    #[test]
    fn test_point_scale_invert_range() -> Result<(), AvengerScaleError> {
        let scale = PointScale;
        let config = DiscreteToNumericScaleConfig {
            domain: vec!["a", "b", "c"].into(),
            range: (0.0, 100.0),
            round: false,
            range_offset: 0.0,
            options: HashMap::new(),
        };

        // Test range covering multiple points
        let result = scale.invert(&config, (0.0, 100.0))?;
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], 0);
        assert_eq!(result[1], 1);
        assert_eq!(result[2], 2);

        // Test partial range
        let result = scale.invert(&config, (50.0, 100.0))?;
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], 1);
        assert_eq!(result[1], 2);

        // Test reversed range
        let result = scale.invert(&config, (100.0, 50.0))?;
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], 1);
        assert_eq!(result[1], 2);

        Ok(())
    }
}
