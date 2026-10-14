use std::sync::Arc;

use arrow::{
    array::{ArrayRef, AsArray, Float32Array, Float64Array, StringArray},
    compute::{kernels::cast, unary},
    datatypes::{DataType, Float64Type},
};
use avenger_common::{
    types::LinearScaleAdjustment,
    value::{ScalarOrArray, ScalarOrArrayValue},
};
use lazy_static::lazy_static;

use super::domain_summary::{fit, DomainOptions, DomainSummary, Extents, NiceStep};
use super::no_domain;
use super::{
    ConfiguredScale, DomainKind, OptionConstraint, OptionDefinition, ScaleConfig, ScaleContext,
    ScaleImpl,
};
use crate::{array, color_interpolator::scale_numeric_to_color, error::AvengerScaleError};

/// Linear scale that maps a continuous numeric domain to a continuous numeric range.
///
/// # Config Options
///
/// - **clamp** (boolean, default: false): When true, values outside the domain are clamped to the domain extent.
///   For scaling, this means values below the domain minimum map to the range minimum, and values above
///   the domain maximum map to the range maximum. For inversion, the clamping is applied to the range.
///
/// - **range_offset** (f64, default: 0.0): An offset applied to the final scaled values. This is added
///   after the linear transformation. When inverting, this offset is subtracted from input values first.
///
/// - **round** (boolean, default: false): When true, output values from scaling are rounded to the nearest
///   integer. This is useful for pixel-perfect rendering. Does not affect inversion.
#[derive(Debug)]
pub struct LinearScale;

impl LinearScale {
    /// A linear scale onto `range` with no domain yet. Scaling needs a domain, which
    /// [`ConfiguredScale::fit_domain`] fits to data.
    pub fn unfitted(range: (f64, f64)) -> ConfiguredScale {
        Self::configured((0.0, 1.0), range).with_domain(no_domain())
    }

    pub fn configured(domain: (f64, f64), range: (f64, f64)) -> ConfiguredScale {
        ConfiguredScale {
            scale_impl: Arc::new(Self),
            config: ScaleConfig {
                domain: Arc::new(Float64Array::from(vec![domain.0, domain.1])),
                range: Arc::new(Float64Array::from(vec![range.0, range.1])),
                options: vec![
                    ("clamp".to_string(), false.into()),
                    ("range_offset".to_string(), 0.0.into()),
                    ("round".to_string(), false.into()),
                ]
                .into_iter()
                .collect(),
                context: ScaleContext::default(),
            },
        }
    }

    pub fn configured_color<I>(domain: (f64, f64), range: I) -> ConfiguredScale
    where
        I: IntoIterator,
        I::Item: Into<String>,
    {
        ConfiguredScale {
            scale_impl: Arc::new(Self),
            config: ScaleConfig {
                domain: Arc::new(Float64Array::from(vec![domain.0, domain.1])),
                range: Arc::new(StringArray::from(
                    range.into_iter().map(Into::into).collect::<Vec<String>>(),
                )),
                options: Default::default(),
                context: ScaleContext::default(),
            },
        }
    }
}

impl ScaleImpl for LinearScale {
    fn scale_type(&self) -> &'static str {
        "linear"
    }

    fn summarize(
        &self,
        _config: &ScaleConfig,
        values: &ArrayRef,
        extents: Extents,
    ) -> Result<DomainSummary, AvengerScaleError> {
        let values = cast(values, &DataType::Float64)?;
        fit::interval_summary(values.as_primitive::<Float64Type>(), extents)
    }

    fn fit_domain(
        &self,
        config: &ScaleConfig,
        options: &DomainOptions,
        summary: &DomainSummary,
    ) -> Result<ScaleConfig, AvengerScaleError> {
        let options = options.interval()?;
        fit::fit_continuous(
            config,
            options,
            summary,
            fit::IntervalFit {
                nice: options.nice.map(NiceStep::Auto),
                zero: options.zero.then_some(0.0),
                single_value_span: fit::linear_single_value_span,
            },
            |value| value,
            (0.0, 1.0),
        )
    }

    fn domain_kind(&self) -> DomainKind {
        DomainKind::Numeric
    }

    fn option_definitions(&self) -> &[OptionDefinition] {
        lazy_static! {
            static ref DEFINITIONS: Vec<OptionDefinition> = vec![
                OptionDefinition::optional("clamp", OptionConstraint::Boolean),
                OptionDefinition::optional("range_offset", OptionConstraint::Float),
                OptionDefinition::optional("round", OptionConstraint::Boolean),
                OptionDefinition::optional("default", OptionConstraint::Float),
            ];
        }

        &DEFINITIONS
    }

    fn invert(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError> {
        // Cast input to Float64 if needed
        let float_values = cast(values, &DataType::Float64)?;

        // Call existing invert_from_numeric
        let result = self.invert_from_numeric(config, &float_values)?;

        // Convert ScalarOrArray<f64> to ArrayRef
        match result.value() {
            ScalarOrArrayValue::Scalar(s) => {
                // If scalar, create array with single value repeated for input length
                Ok(Arc::new(Float64Array::from(vec![*s; values.len()])) as ArrayRef)
            }
            ScalarOrArrayValue::Array(arr) => {
                // If array, convert to ArrayRef
                Ok(Arc::new(Float64Array::from(arr.as_ref().clone())) as ArrayRef)
            }
        }
    }

    fn scale(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError> {
        // Check if color interpolation is needed FIRST
        if config.color_range().is_ok() {
            return scale_numeric_to_color(self, config, values);
        }

        // For numeric ranges, get the actual range values
        let (range_start, range_end) = config.numeric_interval_range()?;
        let (domain_start, domain_end) = config.numeric_interval_domain()?;

        // Handle degenerate domain/range cases
        if domain_start == domain_end
            || range_start == range_end
            || domain_start.is_nan()
            || domain_end.is_nan()
            || range_start.is_nan()
            || range_end.is_nan()
        {
            return Ok(Arc::new(Float32Array::from(vec![
                range_start as f32;
                values.len()
            ])));
        }

        // Get options
        let range_offset = config.option_f64("range_offset", 0.0);
        let clamp = config.option_boolean("clamp", false);
        let round = config.option_boolean("round", false);

        // Cast to f64 and downcast to f64 array
        let array = cast(values, &DataType::Float64)?;
        let array = array.as_primitive::<Float64Type>();

        // Measure each value from the domain start, so domains far from zero, such as epoch
        // milliseconds, keep their precision.
        let scale = (range_end - range_start) / (domain_end - domain_start);
        let origin = range_start + range_offset;

        let (range_min, range_max) = if range_start <= range_end {
            (range_start, range_end)
        } else {
            (range_end, range_start)
        };

        // Positions are computed in f64 and stored as the f32 values that marks draw.
        let scaled_vec: Float32Array = match (clamp, round) {
            (true, true) => {
                // clamp and round
                unary(array, |v| {
                    (origin + scale * (v - domain_start))
                        .clamp(range_min, range_max)
                        .round() as f32
                })
            }
            (true, false) => {
                // clamp, no round
                unary(array, |v| {
                    (origin + scale * (v - domain_start)).clamp(range_min, range_max) as f32
                })
            }
            (false, true) => {
                // no clamp, round
                unary(array, |v| {
                    (origin + scale * (v - domain_start)).round() as f32
                })
            }
            (false, false) => {
                // no clamp, no round
                unary(array, |v| (origin + scale * (v - domain_start)) as f32)
            }
        };

        Ok(Arc::new(scaled_vec))
    }

    fn invert_from_numeric(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ScalarOrArray<f64>, AvengerScaleError> {
        let (range_start, range_end) = config.numeric_interval_range()?;
        let (domain_start, domain_end) = config.numeric_interval_domain()?;
        let range_offset = config.option_f64("range_offset", 0.0);
        let clamp = config.option_boolean("clamp", false);

        // Handle degenerate domain case
        if domain_start == domain_end
            || range_start == range_end
            || domain_start.is_nan()
            || domain_end.is_nan()
            || range_start.is_nan()
            || range_end.is_nan()
        {
            return Ok(ScalarOrArray::new_array(vec![domain_start; values.len()]));
        }

        // Cast to f64 and downcast to f64 array
        let array = cast(values, &DataType::Float64)?;
        let array = array.as_primitive::<Float64Type>();

        let scale = (domain_end - domain_start) / (range_end - range_start);
        let offset = domain_start - scale * range_start;

        if clamp {
            let (range_min, range_max) = if range_start <= range_end {
                (range_start, range_end)
            } else {
                (range_end, range_start)
            };

            Ok(ScalarOrArray::new_array(
                array
                    .values()
                    .iter()
                    .map(|v| {
                        let v = (v - range_offset).clamp(range_min, range_max);
                        scale * v + offset
                    })
                    .collect(),
            ))
        } else {
            Ok(ScalarOrArray::new_array(
                array
                    .values()
                    .iter()
                    .map(|v| scale * (v - range_offset) + offset)
                    .collect(),
            ))
        }
    }

    fn ticks(
        &self,
        config: &ScaleConfig,
        count: Option<f64>,
    ) -> Result<ArrayRef, AvengerScaleError> {
        let (domain_start, domain_end) = config.numeric_interval_domain()?;
        let count = count.unwrap_or(10.0);
        Ok(array::tick_array(array::ticks(
            domain_start,
            domain_end,
            count,
        )))
    }

    /// Pans the domain by the given delta
    ///
    /// The delta value represents fractional units of the scale range; for example,
    /// 0.5 indicates panning the scale domain to the right by half the scale range.
    fn pan(&self, config: &ScaleConfig, delta: f64) -> Result<ScaleConfig, AvengerScaleError> {
        let (domain_start, domain_end) = config.numeric_interval_domain()?;
        let domain_delta = (domain_end - domain_start) * delta;
        Ok(ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![
                domain_start - domain_delta,
                domain_end - domain_delta,
            ])),
            ..config.clone()
        })
    }

    /// Zooms the domain by the given scale factor
    ///
    /// The anchor value represents the zoom position in terms of fractional units of the
    /// scale range; for example, 0.5 indicates a zoom centered on the mid-point of the
    /// scale range.
    ///
    /// The scale factor represents the amount to scale the domain by; for example,
    /// 2.0 indicates zooming the scale domain to be twice as large.
    fn zoom(
        &self,
        config: &ScaleConfig,
        anchor: f64,
        scale_factor: f64,
    ) -> Result<ScaleConfig, AvengerScaleError> {
        let (domain_start, domain_end) = config.numeric_interval_domain()?;
        let domain_anchor = domain_start + anchor * (domain_end - domain_start);

        let new_start = domain_anchor + (domain_start - domain_anchor) * scale_factor;
        let new_end = domain_anchor + (domain_end - domain_anchor) * scale_factor;

        Ok(ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![new_start, new_end])),
            ..config.clone()
        })
    }

    /// Compute adjustment for data that was originally scaled by `from_config` to be scaled
    /// by `to_config`
    fn adjust(
        &self,
        from_config: &ScaleConfig,
        to_config: &ScaleConfig,
    ) -> Result<LinearScaleAdjustment, AvengerScaleError> {
        let (from_domain_start, from_domain_end) = from_config.numeric_interval_domain()?;
        let (from_range_start, from_range_end) = from_config.numeric_interval_range()?;
        let (to_domain_start, to_domain_end) = to_config.numeric_interval_domain()?;
        let (to_range_start, to_range_end) = to_config.numeric_interval_range()?;

        // A from-position p shows the domain value d = (p - from_range_start) / from_slope +
        // from_domain_start, which the to-scale draws at to_range_start + to_slope *
        // (d - to_domain_start). Only the difference of the domain starts involves large values,
        // so domains far from zero, such as epoch milliseconds, keep their precision.
        let from_slope =
            (from_range_end - from_range_start) / (from_domain_end - from_domain_start);
        let to_slope = (to_range_end - to_range_start) / (to_domain_end - to_domain_start);
        let scale = to_slope / from_slope;
        let offset = to_range_start + to_slope * (from_domain_start - to_domain_start)
            - scale * from_range_start;
        // Marks store the adjustment as f32.
        Ok(LinearScaleAdjustment {
            scale: scale as f32,
            offset: offset as f32,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::datatypes::{Float32Type, Float64Type};
    use float_cmp::assert_approx_eq;

    #[test]
    fn test_scale() -> Result<(), AvengerScaleError> {
        // Test scaling with edge cases: out-of-bounds, nulls, and interpolation
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![10.0, 30.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 100.0])),
            options: vec![("clamp".to_string(), true.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let scale = LinearScale;
        let values = Arc::new(Float64Array::from(vec![
            0.0,  // < domain
            10.0, // domain start
            15.0, 20.0, 25.0, 30.0, // in domain
            40.0, // > domain
        ])) as ArrayRef;

        let result = scale
            .scale_to_numeric(&config, &values)?
            .as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 0.0); // clamped
        assert_approx_eq!(f32, result[1], 0.0); // domain start
        assert_approx_eq!(f32, result[2], 25.0); // interpolated
        assert_approx_eq!(f32, result[3], 50.0); // interpolated
        assert_approx_eq!(f32, result[4], 75.0); // interpolated
        assert_approx_eq!(f32, result[5], 100.0); // domain end
        assert_approx_eq!(f32, result[6], 100.0); // clamped

        Ok(())
    }

    /// An epoch-millisecond domain, where f32 holds values only about 131 s apart, scales and
    /// inverts to the millisecond. Positions are the f32 values nearest the exact positions, and
    /// after a 1,000x zoom adjustment they stay within 0.01 px.
    #[test]
    fn test_epoch_millisecond_domain_keeps_millisecond_precision() -> Result<(), AvengerScaleError>
    {
        let start = 1.7e12_f64;
        let scale = LinearScale::configured((start, start + 60_000.0), (0.0, 600.0));
        let values = Arc::new(Float64Array::from(vec![start + 1.0, start + 30_012.0])) as ArrayRef;
        let positions = scale.scale(&values)?;
        let positions = positions.as_primitive::<Float32Type>().values();
        assert_approx_eq!(f32, positions[0], 0.01);
        assert_approx_eq!(f32, positions[1], 300.12);
        assert_approx_eq!(f64, scale.invert_scalar(0.01)?, start + 1.0, epsilon = 1e-3);

        let zoomed = LinearScale::configured((start + 29_970.0, start + 30_030.0), (0.0, 600.0));
        let adjustment = scale.adjust(&zoomed)?;
        let adjusted =
            f64::from(positions[1]) * f64::from(adjustment.scale) + f64::from(adjustment.offset);
        assert_approx_eq!(f64, adjusted, 420.0, epsilon = 0.01);
        Ok(())
    }

    #[test]
    fn test_adjust_uses_source_and_target_ranges() -> Result<(), AvengerScaleError> {
        let source = LinearScale::configured((0.0, 10.0), (0.0, 100.0));
        let target = LinearScale::configured((0.0, 10.0), (10.0, 210.0));

        let adjustment = source.adjust(&target)?;

        assert_approx_eq!(f32, adjustment.scale, 2.0);
        assert_approx_eq!(f32, adjustment.offset, 10.0);

        Ok(())
    }

    #[test]
    fn test_scale_with_range_offset() -> Result<(), AvengerScaleError> {
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![10.0, 30.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 100.0])),
            options: vec![
                ("range_offset".to_string(), 3.0.into()),
                ("clamp".to_string(), true.into()),
            ]
            .into_iter()
            .collect(),
            context: ScaleContext::default(),
        };

        let scale = LinearScale;
        let values = Arc::new(Float64Array::from(vec![
            0.0,  // < domain
            10.0, // domain start
            15.0, 20.0, 25.0, 30.0, // in domain
            40.0, // > domain
        ])) as ArrayRef;

        let result = scale
            .scale_to_numeric(&config, &values)?
            .as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 0.0); // clamped
        assert_approx_eq!(f32, result[1], 3.0); // domain start
        assert_approx_eq!(f32, result[2], 28.0); // interpolated
        assert_approx_eq!(f32, result[3], 53.0); // interpolated
        assert_approx_eq!(f32, result[4], 78.0); // interpolated
        assert_approx_eq!(f32, result[5], 100.0); // domain end
        assert_approx_eq!(f32, result[6], 100.0); // clamped

        Ok(())
    }

    #[test]
    fn test_scale_degenerate() -> Result<(), AvengerScaleError> {
        // Tests behavior with zero-width domain (matches d3 behavior)
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![10.0, 10.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 100.0])),
            options: vec![("clamp".to_string(), true.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let scale = LinearScale;
        let values = Arc::new(Float64Array::from(vec![0.0, 10.0, 20.0])) as ArrayRef;
        let result = scale
            .scale_to_numeric(&config, &values)?
            .as_vec(values.len(), None);

        // All values should map to range_start (d3 behavior)
        for &value in &result {
            assert_approx_eq!(f32, value, 0.0);
        }

        Ok(())
    }

    #[test]
    fn test_degenerate_cases() -> Result<(), AvengerScaleError> {
        let scale = LinearScale;

        // Test degenerate domain
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![1.0, 1.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 0.0])),
            options: vec![("clamp".to_string(), false.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let values = Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0])) as ArrayRef;
        let result = scale
            .scale_to_numeric(&config, &values)?
            .as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 0.0);
        assert_approx_eq!(f32, result[1], 0.0);
        assert_approx_eq!(f32, result[2], 0.0);

        // Test degenerate range
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![0.0, 10.0])),
            range: Arc::new(Float64Array::from(vec![1.0, 1.0])),
            options: vec![("clamp".to_string(), false.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let result = scale
            .scale_to_numeric(&config, &values)?
            .as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 1.0);
        assert_approx_eq!(f32, result[1], 1.0);
        assert_approx_eq!(f32, result[2], 1.0);

        Ok(())
    }

    #[test]
    fn test_invert_clamped() -> Result<(), AvengerScaleError> {
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![10.0, 30.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 100.0])),
            options: vec![("clamp".to_string(), true.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let scale = LinearScale;
        let values = Arc::new(Float64Array::from(vec![-25.0, 0.0, 50.0, 100.0, 125.0])) as ArrayRef;
        let result = scale
            .invert_from_numeric(&config, &values)?
            .as_vec(values.len(), None);

        assert_approx_eq!(f64, result[0], 10.0); // clamped below
        assert_approx_eq!(f64, result[1], 10.0); // range start
        assert_approx_eq!(f64, result[2], 20.0); // interpolated
        assert_approx_eq!(f64, result[3], 30.0); // range end
        assert_approx_eq!(f64, result[4], 30.0); // clamped above

        Ok(())
    }

    #[test]
    fn test_invert_unclamped() -> Result<(), AvengerScaleError> {
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![10.0, 30.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 100.0])),
            options: vec![("clamp".to_string(), false.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let scale = LinearScale;
        let values = Arc::new(Float64Array::from(vec![-25.0, 0.0, 50.0, 100.0, 125.0])) as ArrayRef;
        let result = scale
            .invert_from_numeric(&config, &values)?
            .as_vec(values.len(), None);

        assert_approx_eq!(f64, result[0], 5.0); // below range
        assert_approx_eq!(f64, result[1], 10.0); // range start
        assert_approx_eq!(f64, result[2], 20.0); // interpolated
        assert_approx_eq!(f64, result[3], 30.0); // range end
        assert_approx_eq!(f64, result[4], 35.0); // above range

        Ok(())
    }

    #[test]
    fn test_invert_with_range_offset() -> Result<(), AvengerScaleError> {
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![10.0, 30.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 100.0])),
            options: vec![("range_offset".to_string(), 3.0.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let scale = LinearScale;
        let values = Arc::new(Float64Array::from(vec![-22.0, 3.0, 53.0, 103.0, 128.0])) as ArrayRef;
        let result = scale
            .invert_from_numeric(&config, &values)?
            .as_vec(values.len(), None);

        assert_approx_eq!(f64, result[0], 5.0); // below range
        assert_approx_eq!(f64, result[1], 10.0); // range start
        assert_approx_eq!(f64, result[2], 20.0); // interpolated
        assert_approx_eq!(f64, result[3], 30.0); // range end
        assert_approx_eq!(f64, result[4], 35.0); // above range

        Ok(())
    }

    #[test]
    fn test_invert_reversed_range() -> Result<(), AvengerScaleError> {
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![10.0, 30.0])),
            range: Arc::new(Float64Array::from(vec![100.0, 0.0])),
            options: vec![("clamp".to_string(), true.into())]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };

        let scale = LinearScale;
        let values = Arc::new(Float64Array::from(vec![125.0, 100.0, 50.0, 0.0, -25.0])) as ArrayRef;
        let result = scale
            .invert_from_numeric(&config, &values)?
            .as_vec(values.len(), None);

        assert_approx_eq!(f64, result[0], 10.0); // clamped
        assert_approx_eq!(f64, result[1], 10.0); // range start
        assert_approx_eq!(f64, result[2], 20.0); // interpolated
        assert_approx_eq!(f64, result[3], 30.0); // range end
        assert_approx_eq!(f64, result[4], 30.0); // clamped

        Ok(())
    }

    #[test]
    fn test_ticks() -> Result<(), AvengerScaleError> {
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![0.0, 10.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 100.0])),
            options: vec![].into_iter().collect(),
            context: ScaleContext::default(),
        };

        let scale = LinearScale;

        let expected = vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0];
        let ticks_array = scale.ticks(&config, Some(5.0))?;
        let ticks_array = ticks_array.as_primitive::<Float64Type>();
        assert_eq!(ticks_array.values().to_vec(), expected);

        let expected = vec![0.0, 5.0, 10.0];
        let ticks_array = scale.ticks(&config, Some(2.0))?;
        let ticks_array = ticks_array.as_primitive::<Float64Type>();
        assert_eq!(ticks_array.values().to_vec(), expected);

        let expected = vec![0.0, 10.0];
        let ticks_array = scale.ticks(&config, Some(1.0))?;
        let ticks_array = ticks_array.as_primitive::<Float64Type>();
        assert_eq!(ticks_array.values().to_vec(), expected);

        Ok(())
    }

    #[test]
    fn test_ticks_span_zero() -> Result<(), AvengerScaleError> {
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![-100.0, 100.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 100.0])),
            options: vec![].into_iter().collect(),
            context: ScaleContext::default(),
        };

        let scale = LinearScale;
        let ticks_array = scale.ticks(&config, Some(10.0))?;
        let ticks_array = ticks_array.as_primitive::<Float64Type>();
        assert_eq!(
            ticks_array.values().to_vec(),
            vec![-100.0, -80.0, -60.0, -40.0, -20.0, 0.0, 20.0, 40.0, 60.0, 80.0, 100.0]
        );

        let ticks_array = scale.ticks(&config, Some(5.0))?;
        let ticks_array = ticks_array.as_primitive::<Float64Type>();
        assert_eq!(
            ticks_array.values().to_vec(),
            vec![-100.0, -50.0, 0.0, 50.0, 100.0]
        );

        let ticks_array = scale.ticks(&config, Some(2.0))?;
        let ticks_array = ticks_array.as_primitive::<Float64Type>();
        assert_eq!(ticks_array.values().to_vec(), vec![-100.0, 0.0, 100.0]);

        let ticks_array = scale.ticks(&config, Some(1.0))?;
        let ticks_array = ticks_array.as_primitive::<Float64Type>();
        assert_eq!(ticks_array.values().to_vec(), vec![0.0]);

        Ok(())
    }

    #[test]
    fn test_ticks_are_exact_decimals() -> Result<(), AvengerScaleError> {
        let scale = LinearScale::configured((0.0, 1.0), (0.0, 100.0));
        let ticks = scale.ticks(Some(10.0))?;
        let expected: Vec<f64> = (0..=10).map(|i| f64::from(i) / 10.0).collect();
        assert_eq!(
            ticks.as_primitive::<Float64Type>().values().to_vec(),
            expected
        );
        Ok(())
    }

    #[test]
    fn test_configured_scale_invert() -> Result<(), AvengerScaleError> {
        use arrow::array::Int32Array;
        // Test the new invert() method that returns ArrayRef
        let scale = LinearScale::configured((10.0, 30.0), (0.0, 100.0));

        // Test with Float32 values
        let values = Arc::new(Float64Array::from(vec![0.0, 25.0, 50.0, 75.0, 100.0])) as ArrayRef;
        let result = scale.invert(&values)?;
        let result_array = result.as_primitive::<Float64Type>();

        assert_approx_eq!(f64, result_array.value(0), 10.0); // range start -> domain start
        assert_approx_eq!(f64, result_array.value(1), 15.0); // 25% -> 15
        assert_approx_eq!(f64, result_array.value(2), 20.0); // 50% -> 20
        assert_approx_eq!(f64, result_array.value(3), 25.0); // 75% -> 25
        assert_approx_eq!(f64, result_array.value(4), 30.0); // range end -> domain end

        // Test with Int32 values (should be cast to Float64 internally)
        let values = Arc::new(Int32Array::from(vec![0, 50, 100])) as ArrayRef;
        let result = scale.invert(&values)?;
        let result_array = result.as_primitive::<Float64Type>();

        assert_eq!(result.len(), 3);
        assert_approx_eq!(f64, result_array.value(0), 10.0);
        assert_approx_eq!(f64, result_array.value(1), 20.0);
        assert_approx_eq!(f64, result_array.value(2), 30.0);

        // Test with clamping
        let scale = scale.with_option("clamp", true);
        let values = Arc::new(Float64Array::from(vec![-50.0, 150.0])) as ArrayRef;
        let result = scale.invert(&values)?;
        let result_array = result.as_primitive::<Float64Type>();

        assert_approx_eq!(f64, result_array.value(0), 10.0); // clamped to domain start
        assert_approx_eq!(f64, result_array.value(1), 30.0); // clamped to domain end

        Ok(())
    }
}
