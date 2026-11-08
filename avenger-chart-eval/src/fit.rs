//! The fit of scales: each position scale group's fitting scale, whose domain fits the plot's
//! length with room for the reach of its marks, and each non-position scale, whose domain needs
//! no length.

use std::collections::BTreeMap;

use arrow::{array::ArrayRef, datatypes::DataType};
use avenger_chart::{proto::Channel, resolve::ScaleGroup, Chart, FitKind};
use avenger_scales::scales::{
    domain_summary::{CategoryOptions, CategoryOrder, DomainOptions, DomainSummary, Extents},
    fitting::{Direction, FittingScale},
    ConfiguredScale,
};
use avenger_selection::SelectionState;
use avenger_style::{ComputedStyle, ComputedStyles};

use crate::{
    convert::{
        domain::{domain_options, source, DomainSource, Fit, FitDefaults},
        scale::{fit_kind, range, range_shape, unfitted},
    },
    EvalError, EvalWarning, Registry,
};

type Result<T> = std::result::Result<T, EvalError>;

/// What every fit reads: the chart, its custom scales, and the state of each selection by name.
#[derive(Clone, Copy)]
pub(crate) struct FitInputs<'a> {
    pub(crate) chart: &'a Chart,
    pub(crate) registry: &'a Registry,
    pub(crate) states: &'a BTreeMap<String, SelectionState>,
}

/// The values of one member of a scale group, with the reach of each value's mark toward the lower
/// and the higher values of its domain, or `None` for marks that reach no farther than their
/// values.
pub(crate) type MemberValues = (ArrayRef, Option<(Vec<f64>, Vec<f64>)>);

/// The summary of `values` for `scale`, with the reach of each value's mark.
fn summarize<'a>(
    scale: &ConfiguredScale,
    values: impl IntoIterator<Item = &'a MemberValues>,
) -> Result<Option<DomainSummary>> {
    let mut merged: Option<DomainSummary> = None;
    for (values, reach) in values {
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
        let summary = scale.summarize(values, extents)?;
        merged = Some(match merged {
            Some(merged) => merged
                .merge(&summary)
                .map_err(avenger_scales::error::AvengerScaleError::from)?,
            None => summary,
        });
    }
    Ok(merged)
}

/// The summary of an inferred domain: the producer's summaries, merged with the summary of the
/// values of the members that `data` marks.
fn inferred_summary(
    scale: &ConfiguredScale,
    summaries: &[DomainSummary],
    data: &[bool],
    values: &[Option<MemberValues>],
) -> Result<Option<DomainSummary>> {
    let mut merged = summarize(
        scale,
        (values.iter().zip(data))
            .filter(|(_, data)| **data)
            .filter_map(|(values, _)| values.as_ref()),
    )?;
    for summary in summaries {
        merged = Some(match merged {
            Some(merged) => merged
                .merge(summary)
                .map_err(avenger_scales::error::AvengerScaleError::from)?,
            None => summary.clone(),
        });
    }
    Ok(merged)
}

/// The type of the values of a group: of its first member with values, or numbers.
fn value_type(values: &[Option<MemberValues>]) -> DataType {
    (values.iter().flatten().next())
        .map(|(values, _)| values.data_type().clone())
        .unwrap_or(DataType::Float64)
}

/// A fitting scale of `summary` with the fit's options, whose categories come in the fit's
/// preferred order when it has one. The categories that a summary holds do not depend on the
/// length, so the order applies once, as an explicit order.
fn fitting_of(scale: ConfiguredScale, summary: DomainSummary, fit: &Fit) -> Result<FittingScale> {
    if fit.preferred.is_some() {
        if let DomainOptions::Categories(_) = &fit.options {
            let ascending = scale.clone().with_range_interval((0.0, 1.0)).fit_domain(
                &DomainOptions::Categories(CategoryOptions::default()),
                &summary.without_extents(),
            )?;
            let order = fit.order(ascending.config.domain.clone())?;
            return Ok(FittingScale::new(
                scale,
                summary,
                DomainOptions::Categories(CategoryOptions {
                    order: CategoryOrder::Explicit(order),
                }),
            ));
        }
    }
    Ok(FittingScale::new(scale, summary, fit.options.clone()))
}

/// A position scale of a plot, which fits its domain at each length.
#[derive(Clone, Debug)]
pub(crate) struct Position {
    pub(crate) fitting: FittingScale,
    /// Whether the plot clips its marks, because the domain can exclude data: an explicit domain,
    /// a selection's domain, or a pinned fit.
    pub(crate) clip: bool,
}

/// Which end of the plot's extent the start of a position group's domain lies at: a continuous `y`
/// runs up the plot, and a discrete one down it, so its first category is at the top, and a range
/// that the spec reverses runs the other way.
pub(crate) fn direction(group: &ScaleGroup, kind: FitKind) -> Direction {
    let up = group.channel == Channel::Y && kind != FitKind::Categories;
    let reversed = (group.spec.range.as_ref()).is_some_and(|range| range.reverse);
    match up != reversed {
        true => Direction::Reverse,
        false => Direction::Forward,
    }
}

/// The fitting scale of a position group, from its spec, its `scale` node's computed style, the
/// selection states, and the values of its members with the reach of their marks. A group that
/// `wraps` around whole circles, whose range ends where it starts, leaves the space between two
/// categories after its last, so each category takes one equal step.
pub(crate) fn position(
    inputs: &FitInputs,
    group: &ScaleGroup,
    defaults: FitDefaults,
    style: &ComputedStyle,
    values: &[Option<MemberValues>],
    wraps: bool,
) -> Result<Position> {
    let FitInputs {
        chart,
        registry,
        states,
    } = *inputs;
    let spec = &group.spec;
    let mut unfitted = unfitted(spec, style, registry, &value_type(values))?;
    if wraps {
        let wrapped = match unfitted.scale_impl.scale_type() {
            "band" => Some((
                "padding_outer",
                unfitted.config.option_f64("padding_inner", 0.0) / 2.0,
            )),
            "point" => Some(("padding", 0.5)),
            _ => None,
        };
        if let Some((name, padding)) = wrapped {
            (unfitted.config.options).insert(name.to_string(), padding.into());
        }
    }
    let kind = fit_kind(spec, registry)?;
    let fit = domain_options(chart, spec, kind, defaults);
    let (fitting, clip) = match source(chart, group, states)? {
        DomainSource::Explicit(domain) => (explicit(&unfitted, domain, kind, values)?, true),
        DomainSource::Inferred {
            summaries,
            data,
            pin,
        } => {
            let fit = fit.pinned(pin);
            let fitting = match inferred_summary(&unfitted, &summaries, &data, values)? {
                Some(summary) => fitting_of(unfitted, summary, &fit)?,
                None => FittingScale::from(unfitted),
            };
            (fitting, fit.pins())
        }
    };
    Ok(Position {
        fitting: fitting.with_direction(direction(group, kind)),
        clip,
    })
}

/// The fitting scale of an explicit domain. A band or a point scale keeps its categories and fits
/// its clip padding to the reach of its marks.
fn explicit(
    unfitted: &ConfiguredScale,
    domain: ArrayRef,
    kind: FitKind,
    values: &[Option<MemberValues>],
) -> Result<FittingScale> {
    let scale = unfitted.clone().with_domain(domain.clone());
    let ordinal = unfitted.scale_impl.scale_type() == "ordinal";
    Ok(
        match (kind, summarize(unfitted, values.iter().flatten())?) {
            (FitKind::Categories, Some(summary)) if !ordinal => FittingScale::new(
                scale,
                summary,
                DomainOptions::Categories(CategoryOptions {
                    order: CategoryOrder::Explicit(domain),
                }),
            ),
            _ => FittingScale::from(scale),
        },
    )
}

/// A non-position scale with its domain and its range: the domain from its source, fitted without
/// a length, and the range from the computed `scheme` or `range` of its `scale` node. A scale whose
/// styles give no range keeps the range from 0 to 1, with a warning.
pub(crate) fn non_position(
    inputs: &FitInputs,
    group: &ScaleGroup,
    defaults: FitDefaults,
    style: &ComputedStyle,
    styles: &ComputedStyles,
    values: &[Option<ArrayRef>],
    warnings: &mut Vec<EvalWarning>,
) -> Result<ConfiguredScale> {
    let FitInputs {
        chart,
        registry,
        states,
    } = *inputs;
    let spec = &group.spec;
    let values: Vec<Option<MemberValues>> = (values.iter())
        .map(|values| values.clone().map(|values| (values, None)))
        .collect();
    let unfitted = unfitted(spec, style, registry, &value_type(&values))?;
    let domain = match source(chart, group, states)? {
        DomainSource::Explicit(domain) => domain,
        DomainSource::Inferred {
            summaries,
            data,
            pin,
        } => {
            let fit = domain_options(chart, spec, fit_kind(spec, registry)?, defaults).pinned(pin);
            match inferred_summary(&unfitted, &summaries, &data, &values)? {
                Some(summary) => {
                    let domain = (unfitted.clone().with_range_interval((0.0, 1.0)))
                        .fit_domain(&fit.options, &summary)?
                        .config
                        .domain;
                    fit.order(domain)?
                }
                None => unfitted.config.domain.clone(),
            }
        }
    };
    let scale = unfitted.with_domain(domain.clone());
    let shape = range_shape(spec, domain.len());
    // A format range that only reverses reverses the range of the styles, and one with values or a
    // palette is reversed in the styles already.
    let reverse = spec
        .range
        .as_ref()
        .is_some_and(|range| range.reverse && range.range.is_none());
    let range = match (range(style, styles, shape), reverse) {
        (Some(range), true) => Some(range.reversed()?),
        (range, _) => range,
    };
    let scale = match range {
        Some(range) => range.apply(scale)?,
        None => {
            warnings.push(EvalWarning::NotDrawn {
                path: format!("the {} scale", group.channel.field_name()),
                reason:
                    "its styles give no scheme and no range, so it maps to the range from 0 to 1"
                        .to_string(),
            });
            scale
        }
    };
    Ok(scale)
}
