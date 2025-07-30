use std::{collections::HashMap, sync::Arc};

use crate::error::AvengerScaleError;
use arrow::{
    array::{ArrayRef, Float32Array, UInt32Array},
    compute::kernels::take,
};
use lazy_static::lazy_static;

use super::point::make_band_config;
use super::ScaleContext;
use super::{
    ordinal::OrdinalScale, ConfiguredScale, InferDomainFromDataMethod, OptionConstraint,
    OptionDefinition, ScaleConfig, ScaleImpl,
};

/// Band scale that maps discrete domain values to continuous numeric bands with optional padding.
///
/// This scale is useful for bar charts and other visualizations where each discrete value
/// needs a dedicated band of space. Each domain value is assigned a band of equal width,
/// with configurable padding between and around the bands.
///
/// # Config Options
///
/// - **align** (f32, default: 0.5): Alignment of the band layout within the range [0, 1].
///   0 aligns to the start, 0.5 centers, and 1 aligns to the end.
///
/// - **band** (f32, default: 0.0): Position within each band where the value is placed [0, 1].
///   0 places at band start, 0.5 at band center, and 1 at band end. This effectively
///   controls where within its allocated band each mark is positioned.
///
/// - **padding** (f32, default: 0.0): Padding before the first and after
///   the last band as a multiple of the step size. Must be non-negative. A value of 0.5 adds half
///   a step of padding on each end.
///
/// - **padding_inner** (f32, default: 0.0): Padding between adjacent bands
///   as a fraction [0, 1] of the step size. A value of 0.2 means 20% of the step is used for padding.
///
/// - **padding_outer** (f32, default: 0.0): Alias for `padding`. Kept for backward compatibility.
///
/// - **round** (boolean, default: false): When true, band positions and widths are rounded
///   to integer pixel values for crisp rendering.
///
/// - **range_offset** (f32, default: 0.0): Additional offset applied to all band positions
///   after computing their base positions. Useful for fine-tuning placement.
#[derive(Debug, Clone)]
pub struct BandScale;

impl BandScale {
    pub fn configured(domain: ArrayRef, range: (f32, f32)) -> ConfiguredScale {
        ConfiguredScale {
            scale_impl: Arc::new(Self),
            config: ScaleConfig {
                domain,
                range: Arc::new(Float32Array::from(vec![range.0, range.1])),
                options: vec![
                    ("align".to_string(), 0.5.into()),
                    ("band".to_string(), 0.0.into()),
                    ("padding".to_string(), 0.0.into()),
                    ("padding_inner".to_string(), 0.0.into()),
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
    fn scale_type(&self) -> &'static str {
        "band"
    }

    fn infer_domain_from_data_method(&self) -> InferDomainFromDataMethod {
        InferDomainFromDataMethod::Unique
    }

    fn option_definitions(&self) -> &[OptionDefinition] {
        lazy_static! {
            static ref DEFINITIONS: Vec<OptionDefinition> = vec![
                OptionDefinition::optional(
                    "align",
                    OptionConstraint::FloatRange { min: 0.0, max: 1.0 }
                ),
                OptionDefinition::optional(
                    "band",
                    OptionConstraint::FloatRange { min: 0.0, max: 1.0 }
                ),
                OptionDefinition::optional("padding", OptionConstraint::NonNegativeFloat),
                OptionDefinition::optional(
                    "padding_inner",
                    OptionConstraint::FloatRange { min: 0.0, max: 1.0 }
                ),
                OptionDefinition::optional("padding_outer", OptionConstraint::NonNegativeFloat),
                OptionDefinition::optional("round", OptionConstraint::Boolean),
                OptionDefinition::optional("range_offset", OptionConstraint::Float),
            ];
        }

        &DEFINITIONS
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

        let scaled = ordinal_scale.scale(&ordinal_config, values)?;

        // Band scale always returns Float32, so cast the dictionary array
        use arrow::compute::kernels::cast;
        use arrow::datatypes::DataType;
        Ok(cast(&scaled, &DataType::Float32)?)
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

    // Handle padding options: 'padding' is primary for outer, 'padding_outer' is kept for compatibility
    let padding_inner = config.option_f32("padding_inner", 0.0);
    let padding_outer = config
        .options
        .get("padding_outer")
        .and_then(|v| v.as_f32().ok())
        .unwrap_or_else(|| config.option_f32("padding", 0.0));
    let round = config.option_boolean("round", false);
    let range_offset = config.option_f32("range_offset", 0.0);
    let (range_start, range_stop) = config.numeric_interval_range()?;

    if !(0.0..=1.0).contains(&align) || !align.is_finite() {
        return Err(AvengerScaleError::InvalidScalePropertyValue(format!(
            "align is {align} but must be between 0 and 1"
        )));
    }

    if !(0.0..=1.0).contains(&band) || !band.is_finite() {
        return Err(AvengerScaleError::InvalidScalePropertyValue(format!(
            "band is {band} but must be between 0 and 1"
        )));
    }

    if !(0.0..=1.0).contains(&padding_inner) || !padding_inner.is_finite() {
        return Err(AvengerScaleError::InvalidScalePropertyValue(format!(
            "padding_inner is {padding_inner} but must be between 0 and 1"
        )));
    }

    if padding_outer < 0.0 || !padding_outer.is_finite() {
        return Err(AvengerScaleError::InvalidScalePropertyValue(format!(
            "padding_outer is {padding_outer} but must be non-negative"
        )));
    }

    if !range_start.is_finite() || !range_stop.is_finite() {
        return Err(AvengerScaleError::InvalidScalePropertyValue(format!(
            "range is ({range_start}, {range_stop}) but both ends must be finite"
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

    // Handle padding options: 'padding' is primary for outer, 'padding_outer' is kept for compatibility
    let padding_inner = config.option_f32("padding_inner", 0.0);
    let padding_outer = config
        .options
        .get("padding_outer")
        .and_then(|v| v.as_f32().ok())
        .unwrap_or_else(|| config.option_f32("padding", 0.0));

    let (start, stop) = if range_stop < range_start {
        (range_stop, range_start)
    } else {
        (range_start, range_stop)
    };

    let step = (stop - start) / 1.0_f32.max(bandspace(n, Some(padding_inner), Some(padding_outer)));
    let step = if config.option_boolean("round", false) {
        step.floor()
    } else {
        step
    };
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
    let padding_outer = config
        .options
        .get("padding_outer")
        .and_then(|v| v.as_f32().ok())
        .unwrap_or_else(|| config.option_f32("padding", 0.0));

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
                ("padding".to_string(), 0.2.into()),
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

        // With rounding, positions are offset by alignment
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
                ("padding".to_string(), 0.2.into()),
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
                ("padding".to_string(), 0.2.into()),
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

    #[test]
    fn test_band_scale_round_various_ranges() -> Result<(), AvengerScaleError> {
        // Test rounding behavior across various ranges to ensure consistency with Vega
        let domain = Arc::new(StringArray::from(vec!["A", "B", "C", "D", "E"]));
        let scale = BandScale;

        // Test with various ranges to check rounding behavior
        let test_cases = vec![
            // (range_start, range_stop, expected_positions, expected_bandwidth)
            (0.0, 300.0, vec![0.0, 60.0, 120.0, 180.0, 240.0], 60.0),
            (0.0, 295.0, vec![0.0, 59.0, 118.0, 177.0, 236.0], 59.0),
            (10.0, 290.0, vec![10.0, 66.0, 122.0, 178.0, 234.0], 56.0),
            (0.0, 100.0, vec![0.0, 20.0, 40.0, 60.0, 80.0], 20.0),
            // With remainder of 2, offset = round(2 * 0.5) = 1
            (0.0, 97.0, vec![1.0, 20.0, 39.0, 58.0, 77.0], 19.0),
        ];

        for (start, stop, expected_positions, expected_bandwidth) in test_cases {
            let config = ScaleConfig {
                domain: domain.clone(),
                range: Arc::new(Float32Array::from(vec![start, stop])),
                options: vec![
                    ("round".to_string(), true.into()),
                    ("align".to_string(), 0.5.into()),
                ]
                .into_iter()
                .collect(),
                context: ScaleContext::default(),
            };

            let values = Arc::new(StringArray::from(vec!["A", "B", "C", "D", "E"])) as ArrayRef;
            let result = scale
                .scale_to_numeric(&config, &values)?
                .as_vec(values.len(), None);

            // Check positions
            for (i, (actual, expected)) in result.iter().zip(expected_positions.iter()).enumerate()
            {
                assert_eq!(
                    *actual, *expected,
                    "Position mismatch at index {} for range [{}, {}]: expected {}, got {}",
                    i, start, stop, expected, actual
                );
            }

            // Check bandwidth
            assert_eq!(
                bandwidth(&config)?,
                expected_bandwidth,
                "Bandwidth mismatch for range [{}, {}]",
                start,
                stop
            );
        }

        Ok(())
    }

    #[test]
    fn test_band_scale_round_with_padding_detailed() -> Result<(), AvengerScaleError> {
        // Test rounding with padding to ensure proper calculation
        let domain = Arc::new(StringArray::from(vec!["A", "B", "C"]));
        let scale = BandScale;

        let config = ScaleConfig {
            domain: domain.clone(),
            range: Arc::new(Float32Array::from(vec![0.0, 300.0])),
            options: vec![
                ("round".to_string(), true.into()),
                ("padding_inner".to_string(), 0.1.into()),
                ("padding".to_string(), 0.1.into()),
                ("align".to_string(), 0.5.into()),
            ]
            .into_iter()
            .collect(),
            context: ScaleContext::default(),
        };

        let values = Arc::new(StringArray::from(vec!["A", "B", "C"])) as ArrayRef;
        let result = scale
            .scale_to_numeric(&config, &values)?
            .as_vec(values.len(), None);

        // With padding, the calculation follows:
        // step = floor(300 / (3 - 0.1 + 0.2)) = floor(96.774) = 96
        // bandwidth = round(96 * 0.9) = round(86.4) = 86
        // start = round(0 + (300 - 96 * 2.9) * 0.5) = round(10.8) = 11

        // Expected positions: [11, 107, 203]
        assert_eq!(result[0], 11.0, "First position should be 11");
        assert_eq!(result[1], 107.0, "Second position should be 107");
        assert_eq!(result[2], 203.0, "Third position should be 203");
        assert_eq!(bandwidth(&config)?, 86.0, "Bandwidth should be 86");
        assert_eq!(step(&config)?, 96.0, "Step should be 96");

        Ok(())
    }

    #[test]
    fn test_band_scale_round_fractional_step() -> Result<(), AvengerScaleError> {
        // Test edge case where step calculation results in fractional value
        let domain = Arc::new(StringArray::from(vec!["A", "B"]));
        let scale = BandScale;

        // Case where step = 101 / 2 = 50.5, which floors to 50
        let config = ScaleConfig {
            domain: domain.clone(),
            range: Arc::new(Float32Array::from(vec![0.0, 101.0])),
            options: vec![("round".to_string(), true.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let values = Arc::new(StringArray::from(vec!["A", "B"])) as ArrayRef;
        let result = scale
            .scale_to_numeric(&config, &values)?
            .as_vec(values.len(), None);

        // With rounding, positions include alignment offset
        assert_eq!(result[0], 1.0, "First position should be 1");
        assert_eq!(result[1], 51.0, "Second position should be 51");
        assert_eq!(bandwidth(&config)?, 50.0, "Bandwidth should be 50");

        Ok(())
    }

    #[test]
    fn test_band_scale_matches_vega_positions() -> Result<(), AvengerScaleError> {
        // Test that positions match Vega exactly for a typical scenario
        let domain = Arc::new(StringArray::from(vec![
            "A", "B", "C", "D", "E", "F", "G", "H",
        ]));
        let scale = BandScale;

        let config = ScaleConfig {
            domain: domain.clone(),
            range: Arc::new(Float32Array::from(vec![0.0, 300.0])),
            options: vec![
                ("round".to_string(), true.into()),
                ("align".to_string(), 0.5.into()),
            ]
            .into_iter()
            .collect(),
            context: ScaleContext::default(),
        };

        let values = Arc::new(StringArray::from(vec![
            "A", "B", "C", "D", "E", "F", "G", "H",
        ])) as ArrayRef;
        let result = scale
            .scale_to_numeric(&config, &values)?
            .as_vec(values.len(), None);

        // Vega positions: with step=37, remainder=4, offset=2
        let expected = [2.0, 39.0, 76.0, 113.0, 150.0, 187.0, 224.0, 261.0];

        for i in 0..8 {
            assert_eq!(result[i], expected[i], "Position mismatch at index {}", i);
        }

        assert_eq!(bandwidth(&config)?, 37.0, "Bandwidth should be 37");
        assert_eq!(step(&config)?, 37.0, "Step should be 37");

        Ok(())
    }
}
