//! Building and fitting summaries, shared by the scales' `summarize` and `fit_domain`.

use std::collections::HashMap;
use std::sync::Arc;

use arrow::array::{Array, ArrayRef, Float64Array};

use super::{
    check_extents, CategoryOptions, CategorySummary, DomainFitError, DomainSummary, Extents,
    IntervalOptions, IntervalSummary, NiceStep,
};
use crate::error::AvengerScaleError;
use crate::scalar::Scalar;
use crate::scales::ScaleConfig;

/// How an interval scale fits a summary in its coordinate, from its `IntervalOptions`.
pub struct IntervalFit {
    /// The step of a nice domain, when the options ask for one.
    pub nice: Option<NiceStep>,
    /// Zero in the coordinate, when the options ask for a domain that contains it.
    pub zero: Option<f64>,
    /// The span of a domain whose values coincide at a coordinate value.
    pub single_value_span: fn(f64) -> f64,
}

/// A scale's range as pixels, when it is a numeric interval.
pub(crate) fn pixel_range(config: &ScaleConfig) -> Option<(f64, f64)> {
    if config.range.len() == 2 && config.range.data_type().is_numeric() {
        config.numeric_interval_range().ok()
    } else {
        None
    }
}

/// An interval summary of `values`, already in the scale's coordinate. Missing and non-finite
/// values are skipped with their extents.
pub fn interval_summary(
    values: &Float64Array,
    extents: Extents,
) -> Result<DomainSummary, AvengerScaleError> {
    let finite = |i: usize| values.is_valid(i) && values.value(i).is_finite();
    let summary = match extents {
        Extents::PerRow { lower, upper } => {
            if lower.len() != values.len() || upper.len() != values.len() {
                return Err(DomainFitError::InvalidInput(
                    "values and extents must have the same length".into(),
                )
                .into());
            }
            let rows: Vec<usize> = (0..values.len()).filter(|&i| finite(i)).collect();
            let pick = |items: &[f64]| rows.iter().map(|&i| items[i]).collect::<Vec<_>>();
            IntervalSummary::new(&pick(values.values()), &pick(lower), &pick(upper))?
        }
        Extents::None | Extents::Constant { .. } => {
            // With equal extents only the smallest and largest values can bind.
            let (lower, upper) = match extents {
                Extents::Constant { lower, upper } => (lower, upper),
                _ => (0.0, 0.0),
            };
            check_extents([&lower, &upper])?;
            let extremes = (0..values.len())
                .filter(|&i| finite(i))
                .map(|i| values.value(i))
                .fold(None, |extremes, value| {
                    let (min, max) = extremes.unwrap_or((value, value));
                    Some((f64::min(min, value), f64::max(max, value)))
                });
            match extremes {
                Some((min, max)) => IntervalSummary::new(&[min, max], &[lower; 2], &[upper; 2])?,
                None => IntervalSummary::new(&[], &[], &[])?,
            }
        }
    };
    Ok(DomainSummary::Interval(summary))
}

/// A summary's interval summary, or an error for other kinds of summary.
pub(crate) fn interval_parts(
    summary: &DomainSummary,
) -> Result<&IntervalSummary, AvengerScaleError> {
    match summary {
        DomainSummary::Interval(summary) => Ok(summary),
        _ => Err(
            DomainFitError::InvalidInput("an interval scale fits an interval summary".into())
                .into(),
        ),
    }
}

/// Fits an interval summary, returning the domain in the scale's coordinate. A summary without
/// markers fits `(0, 1)`.
pub fn fit_interval(
    config: &ScaleConfig,
    summary: &IntervalSummary,
    fit: IntervalFit,
) -> Result<(f64, f64), AvengerScaleError> {
    let mut summary = summary.clone();
    if let Some(zero) = fit.zero {
        summary = summary.including(zero)?;
    }
    if let Some((value, max)) = summary.extent() {
        if value == max {
            let span = (fit.single_value_span)(value);
            summary = summary
                .including(value - span / 2.0)?
                .including(value + span / 2.0)?;
        }
    }
    // Without extents, any width gives the same fit.
    let width = match pixel_range(config) {
        Some((start, end)) => (end - start).abs(),
        None if summary.has_extents() => {
            return Err(DomainFitError::InvalidInput(
                "extents need a scale whose range is an interval in pixels".into(),
            )
            .into())
        }
        None => 1.0,
    };
    let fitted = summary.fit(width)?;
    let Some(step) = fit.nice else {
        return Ok(fitted);
    };
    match summary.fit_nice(width, step) {
        Ok(nice_domain) => Ok(nice_domain),
        // A nice domain would crush the data, so the domain stays as fitted.
        Err(DomainFitError::Infeasible(_)) => Ok(fitted),
        Err(error) => Err(error.into()),
    }
}

/// Fits a continuous numeric scale, inverting the fitted endpoints from the scale's coordinate with
/// `invert`. No data gives the options' empty domain, or `empty`.
pub fn fit_continuous(
    config: &ScaleConfig,
    options: &IntervalOptions,
    summary: &DomainSummary,
    fit: IntervalFit,
    invert: impl Fn(f64) -> f64,
    empty: (f64, f64),
) -> Result<ScaleConfig, AvengerScaleError> {
    let summary = interval_parts(summary)?;
    if summary.extent().is_none() {
        let default = interval_domain(empty.0, empty.1);
        return Ok(with_domain(config, empty_domain(options, default)));
    }
    let (start, end) = fit_interval(config, summary, fit)?;
    Ok(with_domain(
        config,
        interval_domain(invert(start), invert(end)),
    ))
}

/// The span of a single value in a linear coordinate: 10% of its magnitude, and at least 0.1.
pub fn linear_single_value_span(value: f64) -> f64 {
    0.1 * value.abs().max(1.0)
}

/// The domain for no data: the options' empty domain, or the scale's default.
pub(crate) fn empty_domain(options: &IntervalOptions, default: ArrayRef) -> ArrayRef {
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

/// A summary's category summary, or an error for other kinds of summary.
pub(crate) fn category_parts<'a>(
    summary: &'a DomainSummary,
    scale_type: &str,
) -> Result<&'a CategorySummary, AvengerScaleError> {
    match summary {
        DomainSummary::Categories(summary) => Ok(summary),
        _ => Err(DomainFitError::InvalidInput(format!(
            "a {scale_type} scale fits a category summary"
        ))
        .into()),
    }
}

/// Fits a band, point, or nested band scale: its categories in order, and the clip padding that
/// keeps every mark inside the range. `unit_positions` gives each domain category's position for
/// a configuration whose range is `(0, 1)`. Positions are linear in the range left after clip
/// padding, so one interval fit over the unit positions gives both paddings exactly. Virtual
/// markers at 0 and 1 carry the configured clip paddings, which stay as minimums.
pub fn fit_discrete(
    config: &ScaleConfig,
    options: &CategoryOptions,
    summary: &DomainSummary,
    scale_type: &str,
    unit_positions: impl Fn(&ScaleConfig) -> Result<Vec<f64>, AvengerScaleError>,
) -> Result<ScaleConfig, AvengerScaleError> {
    let categories = category_parts(summary, scale_type)?;
    let domain = categories.domain(&options.order)?;
    let (mut lower, mut upper) = categories.extents_at(&domain)?;
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
    let (start, end) = IntervalSummary::new(&positions, &lower, &upper)?.fit(width)?;
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
pub fn fit_categories(
    config: &ScaleConfig,
    options: &CategoryOptions,
    summary: &DomainSummary,
    scale_type: &str,
) -> Result<ScaleConfig, AvengerScaleError> {
    let domain = category_parts(summary, scale_type)?.domain(&options.order)?;
    Ok(with_domain(config, domain))
}
