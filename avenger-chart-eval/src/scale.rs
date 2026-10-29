//! The configured scales of scale specs: their types, options, domains, and ranges.

use std::{collections::HashMap, sync::Arc};

use arrow::{
    array::{new_empty_array, ArrayRef, Float64Array, ListArray, StringArray, UInt32Array},
    compute::take,
    datatypes::{DataType, Float64Type},
};
use avenger_color::parse_color_string;
use avenger_scales::{
    scalar::Scalar,
    scales::{
        domain_summary::{DomainOptions, DomainSummary, Extents},
        ConfiguredScale, ScaleConfig, ScaleImpl,
    },
};

use avenger_chart::{
    payload,
    proto::{self, domain, mark, range, Channel},
    Chart, FitKind,
};

use crate::{select::Selections, EvalError, Theme};

type Result<T> = std::result::Result<T, EvalError>;

pub(crate) fn eval_error(context: &str, error: impl std::fmt::Display) -> EvalError {
    EvalError::Evaluation(format!("{context}: {error}"))
}

/// The scale type of a typetag name of avenger-scales.
pub(crate) fn scale_impl(name: &str) -> Result<Arc<dyn ScaleImpl>> {
    let scale: Box<dyn ScaleImpl> =
        serde_json::from_value(serde_json::json!({ "type": name, "fields": null }))
            .map_err(|error| eval_error(&format!("the scale type `{name}`"), error))?;
    Ok(Arc::from(scale))
}

/// The options of a spec, as avenger-scales takes them.
pub(crate) fn options(spec: &proto::ScaleSpec) -> Result<HashMap<String, Scalar>> {
    Ok(payload::options(&spec.options)?.into_iter().collect())
}

/// How a summary or data domain of a spec fits: its fit, or a sample for a quantile scale.
pub(crate) fn domain_options(chart: &Chart, spec: &proto::ScaleSpec) -> Result<DomainOptions> {
    match &spec.fit {
        Some(fit) => Ok(chart.fit(fit)?),
        None if FitKind::of(&spec.r#type) == FitKind::Sample => Ok(DomainOptions::Sample),
        None => Ok(DomainOptions::Interval(Default::default())),
    }
}

/// Where the domain of a scale comes from at an evaluation: the domain of a selection
/// domain's selection, or its fallback.
pub(crate) enum DomainSource {
    /// A domain that the scale keeps at every size, from the spec or a selection, whose plot
    /// clips its marks.
    Explicit(ArrayRef),
    /// A summary of the producer.
    Summary(DomainSummary),
    /// The unit's data.
    Data,
}

/// The domain source of `domain`, with the domains of selections from `selections`.
pub(crate) fn domain_source(
    chart: &Chart,
    domain: Option<&proto::Domain>,
    selections: &Selections,
) -> Result<DomainSource> {
    match domain.and_then(|domain| domain.domain.as_ref()) {
        Some(domain::Domain::Explicit(index)) => Ok(DomainSource::Explicit(chart.list(*index))),
        Some(domain::Domain::Summary(index)) => Ok(DomainSource::Summary(chart.summary(*index))),
        Some(domain::Domain::Selection(selection)) => {
            let channel = Channel::try_from(selection.channel).unwrap_or(Channel::Unspecified);
            match selections.domain(&selection.selection, channel)? {
                Some(values) => Ok(DomainSource::Explicit(values)),
                None => domain_source(chart, selection.fallback.as_deref(), selections),
            }
        }
        Some(domain::Domain::Data(_)) | None => Ok(DomainSource::Data),
    }
}

/// A scale of `spec` without its domain: its type, its options, an empty domain of
/// `value_type`, and the range `(0, 1)`.
pub(crate) fn unfitted(spec: &proto::ScaleSpec, value_type: &DataType) -> Result<ConfiguredScale> {
    Ok(ConfiguredScale {
        scale_impl: scale_impl(&spec.r#type)?,
        config: ScaleConfig {
            domain: new_empty_array(value_type),
            range: Arc::new(Float64Array::from(vec![0.0, 1.0])),
            options: options(spec)?,
            context: Default::default(),
        },
    })
}

/// Values of an encoding with the pixel extents of each value's mark toward the lower and the
/// higher values of its domain, or `None` for marks without extents.
pub(crate) type ExtendedValues = (ArrayRef, Option<(Vec<f64>, Vec<f64>)>);

/// The summary of `values` for `scale`, with the pixel extents of each value's mark.
pub(crate) fn summarize(
    scale: &ConfiguredScale,
    values: &[ExtendedValues],
) -> Result<Option<DomainSummary>> {
    let mut merged: Option<DomainSummary> = None;
    for (values, reach) in values {
        // Marks of one reach give one constant extent.
        let constant = |values: &[f64]| values.windows(2).all(|pair| pair[0] == pair[1]);
        let extents = match reach {
            Some((lower, upper)) if !lower.is_empty() && constant(lower) && constant(upper) => {
                Extents::Constant {
                    lower: lower[0],
                    upper: upper[0],
                }
            }
            Some((lower, upper)) => Extents::PerRow { lower, upper },
            None => Extents::None,
        };
        let summary = scale
            .summarize(values, extents)
            .map_err(|error| eval_error("a summary of a domain", error))?;
        merged = Some(match merged {
            Some(merged) => merged
                .merge(&summary)
                .map_err(|error| eval_error("the summaries of a scale", error))?,
            None => summary,
        });
    }
    Ok(merged)
}

/// The configured scale of a non-position channel, whose fit needs no length: its domain from
/// the spec, a selection, the producer's summary, or `values`, the values of its encodings, and
/// its range from the spec or the theme.
pub(crate) fn non_position(
    chart: &Chart,
    spec: &proto::ScaleSpec,
    channel: Channel,
    mark: &mark::Mark,
    values: &[ArrayRef],
    selections: &Selections,
    theme: &Theme,
) -> Result<ConfiguredScale> {
    let value_type = values
        .first()
        .map(|values| values.data_type().clone())
        .unwrap_or(DataType::Float64);
    let unfitted = unfitted(spec, &value_type)?;
    let domain = match domain_source(chart, spec.domain.as_ref(), selections)? {
        DomainSource::Explicit(domain) => domain,
        source => {
            let summary = match source {
                DomainSource::Summary(summary) => Some(summary),
                _ => {
                    let values: Vec<_> =
                        values.iter().map(|values| (values.clone(), None)).collect();
                    summarize(&unfitted, &values)?
                }
            };
            match summary {
                Some(summary) => {
                    unfitted
                        .clone()
                        .fit_domain(&domain_options(chart, spec)?, &summary)
                        .map_err(|error| eval_error("the domain of a scale", error))?
                        .config
                        .domain
                }
                None => unfitted.config.domain.clone(),
            }
        }
    };
    let range = non_position_range(chart, spec, channel, mark, &domain, theme)?;
    let mut scale = unfitted.with_domain(domain);
    scale.config.range = range;
    Ok(scale)
}

/// The range of a non-position scale: the spec's values or palette, or the theme's default for
/// the channel, the mark, and the kind of the scale, reversed when the spec says so.
fn non_position_range(
    chart: &Chart,
    spec: &proto::ScaleSpec,
    channel: Channel,
    mark: &mark::Mark,
    domain: &ArrayRef,
    theme: &Theme,
) -> Result<ArrayRef> {
    let palette = |name: &str| -> Result<ArrayRef> {
        let colors = theme
            .palette(name)
            .ok_or_else(|| EvalError::Evaluation(format!("the theme has no palette `{name}`")))?;
        Ok(Arc::new(StringArray::from(colors.to_vec())))
    };
    let kind = FitKind::of(&spec.r#type);
    // The count of values that a discrete or binned scale maps to.
    let count = match spec.r#type.as_str() {
        "threshold" => domain.len() + 1,
        "quantize" | "quantile" => theme.bins,
        _ => domain.len().max(1),
    };
    let explicit = spec.range.as_ref().and_then(|range| range.range.as_ref());
    let range = match explicit {
        Some(range::Range::Values(index)) => chart.list(*index),
        Some(range::Range::Palette(name)) => match kind {
            FitKind::Categories | FitKind::Interval if spec.r#type != "quantize" => palette(name)?,
            _ => sample_colors(theme.palette(name).unwrap_or_default(), count),
        },
        None => match channel {
            Channel::Color | Channel::Fill | Channel::Stroke => match spec.r#type.as_str() {
                "quantize" | "quantile" | "threshold" => sample_colors(
                    theme.palette(&theme.sequential_palette).unwrap_or_default(),
                    count,
                ),
                _ if kind == FitKind::Categories => palette(&theme.category_palette)?,
                _ => palette(&theme.sequential_palette)?,
            },
            Channel::Shape => Arc::new(StringArray::from(theme.shapes.clone())),
            Channel::Pattern => Arc::new(StringArray::from(theme.patterns.clone())),
            Channel::StrokeDash => Arc::new(ListArray::from_iter_primitive::<Float64Type, _, _>(
                theme.stroke_dashes.iter().map(|dash| {
                    Some(
                        dash.iter()
                            .map(|length| Some(*length as f64))
                            .collect::<Vec<_>>(),
                    )
                }),
            )),
            _ => {
                let [low, high] = match channel {
                    Channel::Opacity | Channel::FillOpacity | Channel::StrokeOpacity => {
                        theme.opacity_range
                    }
                    Channel::StrokeWidth => theme.stroke_width_range,
                    Channel::Angle => theme.angle_range,
                    Channel::Size => match mark {
                        mark::Mark::Text(_) => theme.font_size_range,
                        mark::Mark::Trail(_) => theme.trail_size_range,
                        _ => theme.size_range,
                    },
                    _ => [0.0, 1.0],
                };
                match kind {
                    FitKind::Interval if spec.r#type != "quantize" => {
                        Arc::new(Float64Array::from(vec![low as f64, high as f64]))
                    }
                    _ => {
                        let steps = (count.max(2) - 1) as f64;
                        Arc::new(Float64Array::from(
                            (0..count)
                                .map(|step| low as f64 + (high - low) as f64 * step as f64 / steps)
                                .collect::<Vec<_>>(),
                        ))
                    }
                }
            }
        },
    };
    let reverse = spec.range.as_ref().is_some_and(|range| range.reverse);
    Ok(if reverse { reversed(&range)? } else { range })
}

/// `values` in reverse order.
pub(crate) fn reversed(values: &ArrayRef) -> Result<ArrayRef> {
    let indices = UInt32Array::from((0..values.len() as u32).rev().collect::<Vec<_>>());
    Ok(take(values, &indices, None)?)
}

/// `count` colors at even steps along `palette`, interpolated in sRGB between its colors, as CSS
/// colors.
fn sample_colors(palette: &[String], count: usize) -> ArrayRef {
    let stops: Vec<[f32; 4]> = palette
        .iter()
        .filter_map(|color| parse_color_string(color))
        .collect();
    let colors: Vec<String> = (0..count)
        .map(|step| {
            let t = if count < 2 {
                1.0
            } else {
                step as f32 / (count - 1) as f32
            };
            let color = match stops.len() {
                0 => [0.0, 0.0, 0.0, 1.0],
                1 => stops[0],
                len => {
                    let position = t * (len - 1) as f32;
                    let index = (position.floor() as usize).min(len - 2);
                    let local = position - index as f32;
                    let (a, b) = (stops[index], stops[index + 1]);
                    [
                        a[0] + (b[0] - a[0]) * local,
                        a[1] + (b[1] - a[1]) * local,
                        a[2] + (b[2] - a[2]) * local,
                        a[3] + (b[3] - a[3]) * local,
                    ]
                }
            };
            format!(
                "rgba({}, {}, {}, {})",
                (color[0] * 255.0).round(),
                (color[1] * 255.0).round(),
                (color[2] * 255.0).round(),
                color[3]
            )
        })
        .collect();
    Arc::new(StringArray::from(colors))
}
