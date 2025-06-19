use std::{collections::HashMap, sync::Arc};

use crate::error::AvengerScaleError;
use arrow::{
    array::{ArrayRef, Float32Array, UInt32Array},
    compute::kernels::take,
};

use super::point::make_band_config;
use super::ScaleContext;
// use super::point::make_band_config;
use super::{
    ordinal::OrdinalScale, ConfiguredScale, InferDomainFromDataMethod, ScaleConfig, ScaleImpl,
};

#[derive(Debug, Clone)]
pub struct BandScale;

impl BandScale {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(domain: ArrayRef, range: (f32, f32)) -> ConfiguredScale {
        ConfiguredScale {
            scale_impl: Arc::new(Self),
            config: ScaleConfig {
                domain,
                range: Arc::new(Float32Array::from(vec![range.0, range.1])),
                options: vec![
                    ("align".to_string(), 0.5.into()),
                    ("band".to_string(), 0.0.into()),
                    ("padding_inner".to_string(), 0.0.into()),
                    ("padding_outer".to_string(), 0.0.into()),
                    ("round".to_string(), false.into()),
                    ("range_offset".to_string(), 0.0.into()),
                ]
                .into_iter()
                .collect(),
                context: ScaleContext::default(),
            },
        }
    }

    /// Create a band scale from a point scale
    pub fn from_point_scale(point_scale: &ConfiguredScale) -> ConfiguredScale {
        ConfiguredScale {
            scale_impl: Arc::new(Self),
            config: make_band_config(&point_scale.config),
        }
    }
}

impl ScaleImpl for BandScale {
    fn infer_domain_from_data_method(&self) -> InferDomainFromDataMethod {
        InferDomainFromDataMethod::Unique
    }

    fn scale(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError> {
        let range_values = build_range_values(config)?;
        let range_array = Arc::new(Float32Array::from(range_values));
        let ordinal_scale = OrdinalScale;
        let ordinal_config = ScaleConfig {
            domain: config.domain.clone(),
            range: range_array,
            options: HashMap::new(),
            context: config.context.clone(),
        };

        ordinal_scale.scale(&ordinal_config, values)
    }

    fn invert_range_interval(
        &self,
        config: &ScaleConfig,
        range: (f32, f32),
    ) -> Result<ArrayRef, AvengerScaleError> {
        let (mut lo, mut hi) = range;

        // Bail if range values are invalid
        if lo.is_nan() || hi.is_nan() {
            return Ok(Arc::new(Float32Array::from(Vec::<f32>::new())));
        }

        // Order range inputs
        if hi < lo {
            std::mem::swap(&mut lo, &mut hi);
        }

        let (range_start, range_stop) = config.numeric_interval_range()?;

        let reverse = range_stop < range_start;
        let (start, stop) = if reverse {
            (range_stop, range_start)
        } else {
            (range_start, range_stop)
        };

        // Bail if outside scale range
        if hi < start || lo > stop {
            return Ok(Arc::new(Float32Array::from(Vec::<f32>::new())));
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
        if lo - values[a] > bandwidth(config)? + 1e-10 {
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
            return Ok(Arc::new(Float32Array::from(Vec::<f32>::new())));
        }

        let indices = Arc::new(UInt32Array::from(
            (a..=b).map(|i| i as u32).collect::<Vec<_>>(),
        )) as ArrayRef;

        let domain_values = take::take(&config.domain, &indices, None)?;
        Ok(domain_values)
    }
}

fn build_range_values(config: &ScaleConfig) -> Result<Vec<f32>, AvengerScaleError> {
    let n = config.domain.len();

    if n == 0 {
        return Err(AvengerScaleError::EmptyDomain);
    }

    let align = config.option_f32("align", 0.5);
    let band = config.option_f32("band", 0.0);
    let padding_inner = config.option_f32("padding_inner", 0.0);
    let padding_outer = config.option_f32("padding_outer", 0.0);
    let round = config.option_boolean("round", false);
    let range_offset = config.option_f32("range_offset", 0.0);
    let (range_start, range_stop) = config.numeric_interval_range()?;

    if !(0.0..=1.0).contains(&align) || !align.is_finite() {
        return Err(AvengerScaleError::InvalidScalePropertyValue(format!(
            "align is {} but must be between 0 and 1",
            align
        )));
    }

    if !(0.0..=1.0).contains(&band) || !band.is_finite() {
        return Err(AvengerScaleError::InvalidScalePropertyValue(format!(
            "band is {} but must be between 0 and 1",
            band
        )));
    }

    if !(0.0..=1.0).contains(&padding_inner) || !padding_inner.is_finite() {
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

    if !range_start.is_finite() || !range_stop.is_finite() {
        return Err(AvengerScaleError::InvalidScalePropertyValue(format!(
            "range is ({}, {}) but both ends must be finite",
            range_start, range_stop
        )));
    }

    let reverse = range_stop < range_start;
    let (start, stop) = if reverse {
        (range_stop, range_start)
    } else {
        (range_start, range_stop)
    };

    let step = (stop - start) / 1.0_f32.max(bandspace(n, Some(padding_inner), Some(padding_outer)));
    let step = if round { step.floor() } else { step };

    let start = start + (stop - start - step * (n as f32 - padding_inner)) * align;
    let start = if round { start.round() } else { start };

    // Generate range values
    let range_values: Vec<f32> = (0..n).map(|i| start + step * i as f32).collect::<Vec<_>>();

    // compute band offset
    let band_offset = if band != 0.0 || range_offset != 0.0 {
        bandwidth(config)? * band + range_offset
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
pub fn bandwidth(config: &ScaleConfig) -> Result<f32, AvengerScaleError> {
    let n = config.domain.len();
    if n == 0 {
        return Ok(0.0);
    }

    let (range_start, range_stop) = config.numeric_interval_range()?;
    let padding_inner = config.option_f32("padding_inner", 0.0);
    let padding_outer = config.option_f32("padding_outer", 0.0);

    let (start, stop) = if range_stop < range_start {
        (range_stop, range_start)
    } else {
        (range_start, range_stop)
    };

    let step = (stop - start) / 1.0_f32.max(bandspace(n, Some(padding_inner), Some(padding_outer)));
    let bandwidth = step * (1.0 - padding_inner);

    if config.option_boolean("round", false) {
        Ok(bandwidth.round())
    } else {
        Ok(bandwidth)
    }
}

/// Returns the distance between the starts of adjacent bands.
///
/// The step size is calculated based on the range, domain size, and padding settings.
/// Returns 0 if the domain is empty.
pub fn step(config: &ScaleConfig) -> Result<f32, AvengerScaleError> {
    let n = config.domain.len();
    if n == 0 {
        return Ok(0.0);
    }

    let (range_start, range_stop) = config.numeric_interval_range()?;
    let (start, stop) = if range_stop < range_start {
        (range_stop, range_start)
    } else {
        (range_start, range_stop)
    };

    let padding_inner = config.option_f32("padding_inner", 0.0);
    let padding_outer = config.option_f32("padding_outer", 0.0);

    let step = (stop - start) / 1.0_f32.max(bandspace(n, Some(padding_inner), Some(padding_outer)));

    if config.option_boolean("round", false) {
        Ok(step.floor())
    } else {
        Ok(step)
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

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{ArrayRef, AsArray, Float32Array, StringArray};
    use float_cmp::{assert_approx_eq, F32Margin};

    #[test]
    fn test_band_scale_basic() -> Result<(), AvengerScaleError> {
        let domain = StringArray::from(vec!["a", "b", "c"]);
        let scale = BandScale;
        let config = ScaleConfig {
            domain: Arc::new(domain),
            range: Arc::new(Float32Array::from(vec![0.0, 1.0])),
            options: vec![("align".to_string(), 0.5.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let values = Arc::new(StringArray::from(vec!["a", "b", "b", "c", "f"])) as ArrayRef;
        let result = scale
            .scale_to_numeric(&config, &values)?
            .as_vec(values.len(), None);

        // With 3 bands in [0,1] and no padding, expect bands at 0.0, 0.333, 0.667
        assert_approx_eq!(f32, result[0], 0.0); // "a"
        assert_approx_eq!(f32, result[1], 0.3333333); // "b"
        assert_approx_eq!(f32, result[2], 0.3333333); // "b"
        assert_approx_eq!(f32, result[3], 0.6666667); // "c"
        assert!(result[4].is_nan()); // "f"
        assert_approx_eq!(f32, bandwidth(&config)?, 0.3333333);
        assert_approx_eq!(f32, step(&config)?, 0.3333333);

        Ok(())
    }

    #[test]
    fn test_band_scale_padding() -> Result<(), AvengerScaleError> {
        let scale = BandScale;
        let config = ScaleConfig {
            domain: Arc::new(StringArray::from(vec!["a", "b", "c"])),
            range: Arc::new(Float32Array::from(vec![0.0, 120.0])),
            options: vec![
                ("padding_inner".to_string(), 0.2.into()),
                ("padding_outer".to_string(), 0.2.into()),
            ]
            .into_iter()
            .collect(),
            context: ScaleContext::default(),
        };

        let values = Arc::new(StringArray::from(vec!["a", "b", "b", "c", "f"])) as ArrayRef;
        let result = scale
            .scale_to_numeric(&config, &values)?
            .as_vec(values.len(), None);

        // With padding of 0.2, points should be inset
        assert_approx_eq!(f32, result[0], 7.5); // "a"
        assert_approx_eq!(f32, result[1], 45.0); // "b"
        assert_approx_eq!(f32, result[2], 45.0); // "b"
        assert_approx_eq!(f32, result[3], 82.5); // "c"
        assert!(result[4].is_nan()); // "f"

        assert_approx_eq!(f32, bandwidth(&config)?, 30.0);

        Ok(())
    }

    #[test]
    fn test_band_scale_round() -> Result<(), AvengerScaleError> {
        let domain = StringArray::from(vec!["a", "b", "c"]);
        let scale = BandScale;
        let config = ScaleConfig {
            domain: Arc::new(domain),
            range: Arc::new(Float32Array::from(vec![0.0, 100.0])),
            options: vec![("round".to_string(), true.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let values = Arc::new(StringArray::from(vec!["a", "b", "b", "c", "f"])) as ArrayRef;
        let result = scale
            .scale_to_numeric(&config, &values)?
            .as_vec(values.len(), None);

        // With rounding, values should be integers
        assert_eq!(result[0], 1.0); // "a"
        assert_eq!(result[1], 34.0); // "b"
        assert_eq!(result[2], 34.0); // "b"
        assert_eq!(result[3], 67.0); // "c"
        assert!(result[4].is_nan()); // "f"
        assert_eq!(bandwidth(&config)?, 33.0);

        Ok(())
    }

    #[test]
    fn test_band_position() -> Result<(), AvengerScaleError> {
        let scale = BandScale;
        let config = ScaleConfig {
            domain: Arc::new(StringArray::from(vec!["A", "B", "C"])),
            range: Arc::new(Float32Array::from(vec![0.0, 1.0])),
            options: vec![("band".to_string(), 0.5.into())].into_iter().collect(),
            context: ScaleContext::default(),
        };
        let values = Arc::new(StringArray::from(vec!["A", "B", "C"])) as ArrayRef;
        let result = scale
            .scale_to_numeric(&config, &values)?
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
        let config = ScaleConfig {
            domain: Arc::new(StringArray::from(vec!["A", "B", "C"])),
            range: Arc::new(Float32Array::from(vec![0.0, 1.0])),
            options: vec![
                ("band".to_string(), 0.5.into()),
                ("range_offset".to_string(), 1.0.into()),
            ]
            .into_iter()
            .collect(),
            context: ScaleContext::default(),
        };
        let values = Arc::new(StringArray::from(vec!["A", "B", "C"])) as ArrayRef;
        let result = scale
            .scale_to_numeric(&config, &values)?
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

    #[test]
    fn test_band_scale_invert() -> Result<(), AvengerScaleError> {
        let scale = BandScale;
        let config = ScaleConfig {
            domain: Arc::new(StringArray::from(vec!["a", "b", "c"])),
            range: Arc::new(Float32Array::from(vec![0.0, 120.0])),
            options: vec![
                ("padding_inner".to_string(), 0.2.into()),
                ("padding_outer".to_string(), 0.2.into()),
            ]
            .into_iter()
            .collect(),
            context: ScaleContext::default(),
        };

        // Test exact band positions
        let result = scale.invert_range_interval(&config, (7.5, 7.5)).unwrap();
        let result = result.as_string::<i32>().iter().collect::<Vec<_>>();
        assert_eq!(result, vec![Some("a")]);

        let result = scale.invert_range_interval(&config, (45.0, 45.0)).unwrap();
        let result = result.as_string::<i32>().iter().collect::<Vec<_>>();
        assert_eq!(result, vec![Some("b")]);

        // Test position within band
        let result = scale.invert_range_interval(&config, (15.0, 15.0)).unwrap();
        let result = result.as_string::<i32>().iter().collect::<Vec<_>>();
        assert_eq!(result, vec![Some("a")]);

        // Test position in padding (should return None)
        let result = scale.invert_range_interval(&config, (40.0, 40.0)).unwrap();
        assert_eq!(result.len(), 0);

        // Test out of range
        assert!(scale
            .invert_range_interval(&config, (-10.0, -10.0))?
            .is_empty());
        assert!(scale
            .invert_range_interval(&config, (130.0, 130.0))?
            .is_empty());

        Ok(())
    }

    #[test]
    fn test_band_scale_invert_range() -> Result<(), AvengerScaleError> {
        let scale = BandScale;
        let config = ScaleConfig {
            domain: Arc::new(StringArray::from(vec!["a", "b", "c"])),
            range: Arc::new(Float32Array::from(vec![0.0, 120.0])),
            options: vec![
                ("padding_inner".to_string(), 0.2.into()),
                ("padding_outer".to_string(), 0.2.into()),
            ]
            .into_iter()
            .collect(),
            context: ScaleContext::default(),
        };

        // Test range covering multiple bands
        let result = scale.invert_range_interval(&config, (7.5, 82.5)).unwrap();
        let result = result.as_string::<i32>().iter().collect::<Vec<_>>();
        assert_eq!(result, vec![Some("a"), Some("b"), Some("c")]);

        // Test partial range
        let result = scale.invert_range_interval(&config, (45.0, 82.5)).unwrap();
        let result = result.as_string::<i32>().iter().collect::<Vec<_>>();
        assert_eq!(result, vec![Some("b"), Some("c")]);

        // Test reversed range (should handle automatically)
        let result = scale.invert_range_interval(&config, (82.5, 45.0)).unwrap();
        let result = result.as_string::<i32>().iter().collect::<Vec<_>>();
        assert_eq!(result, vec![Some("b"), Some("c")]);

        // Test out of range
        assert!(scale
            .invert_range_interval(&config, (-10.0, -5.0))?
            .is_empty());
        assert!(scale
            .invert_range_interval(&config, (130.0, 140.0))?
            .is_empty());

        // Test invalid range (NaN)
        assert!(scale
            .invert_range_interval(&config, (f32::NAN, 50.0))?
            .is_empty());

        Ok(())
    }
}
