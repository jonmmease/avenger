//! Building and fitting supports, shared by the scales' `domain_support` and `fit_support`.

use std::collections::HashMap;
use std::sync::Arc;

use arrow::array::{Array, ArrayRef, AsArray, Float64Array};
use arrow::compute::cast;
use arrow::datatypes::{DataType, Float64Type};

use super::{
    check_extents, Coordinate, DomainFitError, DomainOptions, DomainSupport, Extents,
    IntervalSupport, SampleSupport, UniqueSupport,
};
use crate::error::AvengerScaleError;
use crate::scalar::Scalar;
use crate::scales::ScaleConfig;

/// How an interval scale rounds a fitted domain when `nice` is set.
pub(crate) enum Rounding {
    /// `LinearScale`'s nice rounding, in the scale's coordinate.
    Nice,
    /// Whole multiples of a step, in the scale's coordinate.
    Grid(f64),
    /// The scale cannot round its domain.
    Unsupported,
}

/// How an interval scale fits a support in its coordinate.
pub(crate) struct IntervalFit<F: Fn(f64) -> f64> {
    pub scale_type: &'static str,
    pub rounding: Rounding,
    /// Zero in the coordinate, or `None` when the domain cannot contain zero.
    pub zero: Option<f64>,
    /// The span of a domain whose values coincide at a coordinate value.
    pub single_value_span: F,
}

/// The values of a numeric array as `f64`, with `None` for nulls.
pub(crate) fn numbers(values: &ArrayRef) -> Result<Vec<Option<f64>>, AvengerScaleError> {
    let values = cast(values, &DataType::Float64)?;
    Ok(values.as_primitive::<Float64Type>().iter().collect())
}

/// A scale's range as pixels, when it is a numeric interval.
pub(crate) fn pixel_range(config: &ScaleConfig) -> Option<(f64, f64)> {
    if config.range.len() == 2 && config.range.data_type().is_numeric() {
        config.numeric_interval_range().ok()
    } else {
        None
    }
}

/// Fails when marks reach past their positions on a scale whose range is not in pixels.
fn check_pixel_extents(
    config: &ScaleConfig,
    extents: &Extents,
    positional: bool,
) -> Result<(), AvengerScaleError> {
    if !extents.is_none() && (!positional || pixel_range(config).is_none()) {
        return Err(DomainFitError::InvalidInput(
            "extents need a scale whose range is an interval in pixels".into(),
        )
        .into());
    }
    Ok(())
}

/// An interval support of `values`, already in `coordinate`. Missing and non-finite values are
/// skipped with their extents.
pub(crate) fn interval_support(
    config: &ScaleConfig,
    coordinate: Coordinate,
    values: &[Option<f64>],
    extents: Extents,
) -> Result<DomainSupport, AvengerScaleError> {
    check_pixel_extents(config, &extents, true)?;
    let finite = |value: &Option<f64>| value.filter(|value| value.is_finite());
    let support = match extents {
        Extents::PerRow { lower, upper } => {
            if lower.len() != values.len() || upper.len() != values.len() {
                return Err(DomainFitError::InvalidInput(
                    "values and extents must have the same length".into(),
                )
                .into());
            }
            let rows: Vec<usize> = (0..values.len())
                .filter(|&i| finite(&values[i]).is_some())
                .collect();
            let pick = |items: &[f64]| rows.iter().map(|&i| items[i]).collect::<Vec<_>>();
            let kept = rows.iter().map(|&i| values[i].unwrap()).collect::<Vec<_>>();
            IntervalSupport::new(&kept, &pick(lower), &pick(upper))?
        }
        Extents::None | Extents::Constant { .. } => {
            // With equal extents only the smallest and largest values can bind.
            let (lower, upper) = match extents {
                Extents::Constant { lower, upper } => (lower, upper),
                _ => (0.0, 0.0),
            };
            check_extents([&lower, &upper])?;
            let extremes = values
                .iter()
                .filter_map(finite)
                .fold(None, |extremes, value| {
                    let (min, max) = extremes.unwrap_or((value, value));
                    Some((f64::min(min, value), f64::max(max, value)))
                });
            match extremes {
                Some((min, max)) => IntervalSupport::new(&[min, max], &[lower; 2], &[upper; 2])?,
                None => IntervalSupport::new(&[], &[], &[])?,
            }
        }
    };
    Ok(DomainSupport::Interval {
        coordinate,
        support,
    })
}

/// A support's coordinate and interval support, or an error for other kinds of support.
pub(crate) fn interval_parts<'a>(
    support: &'a DomainSupport,
    scale_type: &str,
) -> Result<(&'a Coordinate, &'a IntervalSupport), AvengerScaleError> {
    match support {
        DomainSupport::Interval {
            coordinate,
            support,
        } => Ok((coordinate, support)),
        _ => Err(DomainFitError::InvalidInput(format!(
            "a {scale_type} scale fits an interval support"
        ))
        .into()),
    }
}

/// Fails unless a support's coordinate is the scale's.
pub(crate) fn check_coordinate(
    support: &Coordinate,
    scale: &Coordinate,
) -> Result<(), AvengerScaleError> {
    if support != scale {
        return Err(DomainFitError::InvalidInput(format!(
            "a support in {support:?} does not fit a scale in {scale:?}"
        ))
        .into());
    }
    Ok(())
}

/// Fits a support with markers, returning the domain in its coordinate.
pub(crate) fn fit_interval<F: Fn(f64) -> f64>(
    config: &ScaleConfig,
    options: &DomainOptions,
    support: &IntervalSupport,
    fit: IntervalFit<F>,
) -> Result<(f64, f64), AvengerScaleError> {
    let unsupported = |option: &str| {
        AvengerScaleError::InvalidScalePropertyValue(format!(
            "{} scales do not support {option}",
            fit.scale_type
        ))
    };
    let mut support = support.clone();
    if options.zero {
        support = support.including(fit.zero.ok_or_else(|| unsupported("zero"))?)?;
    }
    if let Some((value, max)) = support.extent() {
        if value == max {
            let span = options
                .single_value_span
                .unwrap_or_else(|| (fit.single_value_span)(value));
            if !span.is_finite() || span <= 0.0 {
                return Err(DomainFitError::InvalidInput(
                    "the single-value span must be finite and positive".into(),
                )
                .into());
            }
            support = support
                .including(value - span / 2.0)?
                .including(value + span / 2.0)?;
        }
    }
    // A range that is not in pixels has no extents, so any width gives the same fit.
    let width = pixel_range(config).map_or(1.0, |(start, end)| (end - start).abs());
    let fitted = support.fit(width)?;
    let Some(count) = options.nice else {
        return Ok(fitted);
    };
    let rounded = match fit.rounding {
        Rounding::Nice => support.fit_nice(width, count),
        Rounding::Grid(step) => support.fit_grid(width, step),
        Rounding::Unsupported => return Err(unsupported("nice")),
    };
    match rounded {
        Ok(domain) => Ok(domain),
        // Rounding would crush the data, so the domain stays unrounded.
        Err(DomainFitError::Infeasible(_)) => Ok(fitted),
        Err(error) => Err(error.into()),
    }
}

/// Fits a continuous scale whose support is in `coordinate`, inverting the fitted endpoints with
/// `invert`. No data gives the options' empty domain, or `empty`.
pub(crate) fn fit_continuous<F: Fn(f64) -> f64>(
    config: &ScaleConfig,
    options: &DomainOptions,
    support: &DomainSupport,
    coordinate: &Coordinate,
    fit: IntervalFit<F>,
    invert: impl Fn(f64) -> f64,
    empty: (f64, f64),
) -> Result<ScaleConfig, AvengerScaleError> {
    let (support_coordinate, support) = interval_parts(support, fit.scale_type)?;
    if support.extent().is_none() {
        let default = interval_domain(empty.0, empty.1);
        return Ok(with_domain(config, empty_domain(options, default)));
    }
    check_coordinate(support_coordinate, coordinate)?;
    let (start, end) = fit_interval(config, options, support, fit)?;
    Ok(with_domain(
        config,
        interval_domain(invert(start), invert(end)),
    ))
}

/// The span of a single value in a linear coordinate: 10% of its magnitude, and at least 0.1.
pub(crate) fn linear_single_value_span(value: f64) -> f64 {
    0.1 * value.abs().max(1.0)
}

/// The domain for no data: the options' empty domain, or the scale's default.
pub(crate) fn empty_domain(options: &DomainOptions, default: ArrayRef) -> ArrayRef {
    options.empty.clone().unwrap_or(default)
}

/// `config` with `domain`, other options unchanged.
pub(crate) fn with_domain(config: &ScaleConfig, domain: ArrayRef) -> ScaleConfig {
    ScaleConfig {
        domain,
        ..config.clone()
    }
}

/// A two-value `f64` domain.
pub(crate) fn interval_domain(start: f64, end: f64) -> ArrayRef {
    Arc::new(Float64Array::from(vec![start, end]))
}

/// A unique support of categories. Extents need a positional scale with a pixel range.
pub(crate) fn unique_support(
    config: &ScaleConfig,
    values: &ArrayRef,
    extents: Extents,
    positional: bool,
) -> Result<DomainSupport, AvengerScaleError> {
    check_pixel_extents(config, &extents, positional)?;
    Ok(DomainSupport::Unique(UniqueSupport::new(values, extents)?))
}

/// A support's unique support, or an error for other kinds of support.
pub(crate) fn unique_parts<'a>(
    support: &'a DomainSupport,
    scale_type: &str,
) -> Result<&'a UniqueSupport, AvengerScaleError> {
    match support {
        DomainSupport::Unique(support) => Ok(support),
        _ => Err(DomainFitError::InvalidInput(format!(
            "a {scale_type} scale fits a unique support"
        ))
        .into()),
    }
}

/// Fits a band, point, or nested band scale: its categories in order, and the clip padding that
/// keeps every mark inside the range. `unit_positions` gives each domain category's position for
/// a configuration whose range is `(0, 1)`. Positions are linear in the range left after clip
/// padding, so one interval fit over the unit positions gives both paddings exactly. Virtual
/// markers at 0 and 1 carry the configured clip paddings, which stay as minimums.
pub(crate) fn fit_discrete(
    config: &ScaleConfig,
    options: &DomainOptions,
    support: &DomainSupport,
    scale_type: &str,
    unit_positions: impl Fn(&ScaleConfig) -> Result<Vec<f64>, AvengerScaleError>,
) -> Result<ScaleConfig, AvengerScaleError> {
    let unique = unique_parts(support, scale_type)?;
    let domain = unique.domain(&options.order)?;
    let (mut lower, mut upper) = unique.extents_at(&domain)?;
    let (range_start, range_end) = pixel_range(config).ok_or_else(|| {
        AvengerScaleError::InvalidScalePropertyValue(format!(
            "a {scale_type} scale fits its domain to a numeric range"
        ))
    })?;
    let minimum_lower = config.option_f64("clip_padding_lower", 0.0);
    let minimum_upper = config.option_f64("clip_padding_upper", 0.0);
    let mut positions = if domain.is_empty() {
        Vec::new()
    } else {
        let mut unit_options: HashMap<String, Scalar> = config.options.clone();
        unit_options.remove("clip_padding_lower");
        unit_options.remove("clip_padding_upper");
        unit_options.insert("round".to_string(), false.into());
        unit_options.insert("range_offset".to_string(), 0.0.into());
        unit_positions(&ScaleConfig {
            domain: domain.clone(),
            range: interval_domain(0.0, 1.0),
            options: unit_options,
            context: config.context.clone(),
        })?
    };
    positions.extend([0.0, 1.0]);
    lower.extend([minimum_lower, 0.0]);
    upper.extend([0.0, minimum_upper]);
    let width = (range_end - range_start).abs();
    let (start, end) = IntervalSupport::new(&positions, &lower, &upper)?.fit(width)?;
    let slope = width / (end - start);
    let padding_lower = (-slope * start).max(minimum_lower);
    let padding_upper = (width - slope * (1.0 - start)).max(minimum_upper);
    let mut fitted = with_domain(config, domain);
    fitted
        .options
        .insert("clip_padding_lower".to_string(), padding_lower.into());
    fitted
        .options
        .insert("clip_padding_upper".to_string(), padding_upper.into());
    Ok(fitted)
}

/// Fits an ordinal scale: its categories in order.
pub(crate) fn fit_categories(
    config: &ScaleConfig,
    options: &DomainOptions,
    support: &DomainSupport,
    scale_type: &str,
) -> Result<ScaleConfig, AvengerScaleError> {
    let domain = unique_parts(support, scale_type)?.domain(&options.order)?;
    Ok(with_domain(config, domain))
}

/// A sample support of the finite numbers in `values`.
pub(crate) fn sample_support(
    config: &ScaleConfig,
    values: &ArrayRef,
    extents: Extents,
) -> Result<DomainSupport, AvengerScaleError> {
    check_pixel_extents(config, &extents, false)?;
    let values: Vec<f64> = numbers(values)?
        .into_iter()
        .flatten()
        .filter(|value| value.is_finite())
        .collect();
    Ok(DomainSupport::Sample(SampleSupport::new(&values)?))
}

/// Fits a quantile scale: its domain is the whole sample.
pub(crate) fn fit_sample(
    config: &ScaleConfig,
    options: &DomainOptions,
    support: &DomainSupport,
) -> Result<ScaleConfig, AvengerScaleError> {
    let DomainSupport::Sample(sample) = support else {
        return Err(
            DomainFitError::InvalidInput("a quantile scale fits a sample support".into()).into(),
        );
    };
    let domain = if sample.values().is_empty() {
        empty_domain(options, Arc::new(Float64Array::from(Vec::<f64>::new())))
    } else {
        Arc::new(Float64Array::from(sample.values().to_vec()))
    };
    Ok(with_domain(config, domain))
}
