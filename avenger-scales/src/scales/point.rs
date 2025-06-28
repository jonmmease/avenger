use std::sync::Arc;

use arrow::array::{ArrayRef, Float32Array};
use avenger_common::value::ScalarOrArray;

use crate::error::AvengerScaleError;

use super::{
    band::BandScale, ConfiguredScale, InferDomainFromDataMethod, ScaleConfig, ScaleContext,
    ScaleImpl,
};

#[derive(Debug, Clone)]
pub struct PointScale;

impl PointScale {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(domain: ArrayRef, range: (f32, f32)) -> ConfiguredScale {
        ConfiguredScale {
            scale_impl: Arc::new(Self),
            config: ScaleConfig {
                domain,
                range: Arc::new(Float32Array::from(vec![range.0, range.1])),
                options: vec![
                    ("align".to_string(), 0.5.into()),
                    ("padding".to_string(), 0.0.into()),
                    ("round".to_string(), false.into()),
                    ("range_offset".to_string(), 0.0.into()),
                ]
                .into_iter()
                .collect(),
                context: ScaleContext::default(),
            },
        }
    }
}

impl ScaleImpl for PointScale {
    fn infer_domain_from_data_method(&self) -> InferDomainFromDataMethod {
        InferDomainFromDataMethod::Unique
    }

    fn scale(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError> {
        let band_config = make_band_config(config);
        let band_scale = BandScale;
        band_scale.scale(&band_config, values)
    }

    /// Scale to numeric values
    fn scale_to_numeric(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        // Implement point scale in terms of a band scale.
        let band_config = make_band_config(config);
        let band_scale = BandScale;
        band_scale.scale_to_numeric(&band_config, values)
    }

    fn invert_range_interval(
        &self,
        config: &ScaleConfig,
        range: (f32, f32),
    ) -> Result<ArrayRef, AvengerScaleError> {
        let band_config = make_band_config(config);
        let band_scale = BandScale;
        band_scale.invert_range_interval(&band_config, range)
    }
}

pub(crate) fn make_band_config(point_config: &ScaleConfig) -> ScaleConfig {
    let padding = point_config.option_f32("padding", 0.0);
    let align = point_config.option_f32("align", 0.5);
    let range_offset = point_config.option_f32("range_offset", 0.0);
    let round = point_config.option_boolean("round", false);

    ScaleConfig {
        domain: point_config.domain.clone(),
        range: point_config.range.clone(),
        options: vec![
            ("padding_inner".to_string(), 1.0.into()),
            ("padding_outer".to_string(), padding.into()),
            ("band".to_string(), 0.0.into()),
            ("align".to_string(), align.into()),
            ("range_offset".to_string(), range_offset.into()),
            ("round".to_string(), round.into()),
        ]
        .into_iter()
        .collect(),
        context: point_config.context.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use arrow::array::{AsArray, StringArray};
    use float_cmp::assert_approx_eq;

    #[test]
    fn test_point_scale_basic() -> Result<(), AvengerScaleError> {
        let config = ScaleConfig {
            domain: Arc::new(StringArray::from(vec!["a", "b", "c"])),
            range: Arc::new(Float32Array::from(vec![0.0, 1.0])),
            options: HashMap::new(),
            context: ScaleContext::default(),
        };
        let scale = PointScale;

        let values = Arc::new(StringArray::from(vec!["a", "b", "b", "c", "f"])) as ArrayRef;

        let result = scale
            .scale_to_numeric(&config, &values)?
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
        let config = ScaleConfig {
            domain: Arc::new(StringArray::from(vec!["a", "b", "c"])),
            range: Arc::new(Float32Array::from(vec![0.0, 100.0])),
            options: HashMap::new(),
            context: ScaleContext::default(),
        };
        let scale = PointScale;

        let values = Arc::new(StringArray::from(vec!["a", "b", "b", "c", "f"])) as ArrayRef;
        let result = scale
            .scale_to_numeric(&config, &values)?
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
        let config = ScaleConfig {
            domain: Arc::new(StringArray::from(vec!["a", "b", "c"])),
            range: Arc::new(Float32Array::from(vec![0.0, 100.0])),
            options: HashMap::from([("padding".to_string(), 0.5.into())]),
            context: ScaleContext::default(),
        };
        let scale = PointScale;

        let values = Arc::new(StringArray::from(vec!["a", "b", "b", "c", "f"])) as ArrayRef;
        let result = scale
            .scale_to_numeric(&config, &values)?
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
        let config = ScaleConfig {
            domain: Arc::new(StringArray::from(vec!["a", "b", "c", "d"])),
            range: Arc::new(Float32Array::from(vec![0.0, 100.0])),
            options: vec![("round".to_string(), true.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };
        let scale = PointScale;

        let values = Arc::new(StringArray::from(vec!["a", "b", "b", "c", "d", "f"])) as ArrayRef;
        let result = scale
            .scale_to_numeric(&config, &values)?
            .as_vec(values.len(), None);

        // With 4 points in [0,100] and rounding, expect points at 0, 34, 67, 100
        // Confirmed with d3-scale
        println!("{:?}", result);
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
        let config = ScaleConfig {
            domain: Arc::new(StringArray::from(vec!["a", "b", "c", "d"])),
            range: Arc::new(Float32Array::from(vec![0.0, 100.0])),
            options: vec![
                ("range_offset".to_string(), 1.0.into()),
                ("round".to_string(), true.into()),
            ]
            .into_iter()
            .collect(),
            context: ScaleContext::default(),
        };
        let scale = PointScale;

        let values = Arc::new(StringArray::from(vec!["a", "b", "b", "c", "d", "f"])) as ArrayRef;
        let result = scale
            .scale_to_numeric(&config, &values)?
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
        let config = ScaleConfig {
            domain: Arc::new(StringArray::from(vec!["a", "b", "c"])),
            range: Arc::new(Float32Array::from(vec![0.0, 100.0])),
            options: HashMap::new(),
            context: ScaleContext::default(),
        };

        // Test exact point positions
        let result = scale.invert_range_interval(&config, (0.0, 0.0))?;
        let result = result.as_string::<i32>().iter().collect::<Vec<_>>();
        let expected = vec![Some("a")];
        assert_eq!(result, expected);

        let result = scale.invert_range_interval(&config, (50.0, 50.0))?;
        let result = result.as_string::<i32>().iter().collect::<Vec<_>>();
        let expected = vec![Some("b")];
        assert_eq!(result, expected);

        let result = scale.invert_range_interval(&config, (100.0, 100.0))?;
        let result = result.as_string::<i32>().iter().collect::<Vec<_>>();
        let expected = vec![Some("c")];
        assert_eq!(result, expected);

        // Test positions between points (should return None)
        assert!(scale
            .invert_range_interval(&config, (25.0, 25.0))?
            .is_empty());
        assert!(scale
            .invert_range_interval(&config, (75.0, 75.0))?
            .is_empty());

        // Test out of range
        assert!(scale
            .invert_range_interval(&config, (-10.0, -10.0))?
            .is_empty());
        assert!(scale
            .invert_range_interval(&config, (110.0, 110.0))?
            .is_empty());

        Ok(())
    }

    #[test]
    fn test_point_scale_invert_range() -> Result<(), AvengerScaleError> {
        let scale = PointScale;
        let config = ScaleConfig {
            domain: Arc::new(StringArray::from(vec!["a", "b", "c"])),
            range: Arc::new(Float32Array::from(vec![0.0, 100.0])),
            options: HashMap::new(),
            context: ScaleContext::default(),
        };

        // Test range covering multiple points
        let result = scale.invert_range_interval(&config, (0.0, 100.0))?;
        let result = result.as_string::<i32>().iter().collect::<Vec<_>>();
        let expected = vec![Some("a"), Some("b"), Some("c")];
        assert_eq!(result, expected);

        // Test partial range
        let result = scale.invert_range_interval(&config, (50.0, 100.0))?;
        let result = result.as_string::<i32>().iter().collect::<Vec<_>>();
        let expected = vec![Some("b"), Some("c")];
        assert_eq!(result, expected);

        // Test reversed range
        let result = scale.invert_range_interval(&config, (100.0, 50.0))?;
        let result = result.as_string::<i32>().iter().collect::<Vec<_>>();
        let expected = vec![Some("b"), Some("c")];
        assert_eq!(result, expected);

        Ok(())
    }
}
