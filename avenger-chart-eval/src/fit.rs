//! The fit of scales: each position scale group's fitting scale, whose domain fits the plot's
//! length with room for the reach of its marks, and each non-position scale, whose domain needs
//! no length.

use std::collections::BTreeMap;

use arrow::{array::ArrayRef, datatypes::DataType};
use avenger_chart::{proto::Channel, resolve::ScaleGroup, Chart, FitKind};
use avenger_scales::scales::{
    domain_summary::{CategoryOptions, CategoryOrder, DomainOptions, DomainSummary, Extents},
    fitting::{Direction, FittingScale, ScaleAt},
    ConfiguredScale,
};
use avenger_selection::SelectionState;
use avenger_style::{ComputedStyle, ComputedStyles};

use crate::{
    convert::{
        domain::{domain_options, source, with_ends, DomainSource, Fit},
        scale::{range, range_shape, unfitted},
    },
    EvalError, EvalWarning, Registry,
};

type Result<T> = std::result::Result<T, EvalError>;

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
    /// The ends of a selection's ranges that bound the domain, low then high, when the domain
    /// takes its other end from the fallback that `fitting` fits.
    pub(crate) ends: Option<(Option<ArrayRef>, Option<ArrayRef>)>,
    /// Whether the plot clips its marks, because the domain can exclude data.
    pub(crate) clip: bool,
}

impl Position {
    /// The scale with its range across `extent` and its domain fitted there.
    pub(crate) fn at(&self, extent: (f64, f64)) -> Result<ScaleAt> {
        let at = self.fitting.at(extent)?;
        let Some((low, high)) = &self.ends else {
            return Ok(at);
        };
        let domain = with_ends(low.as_ref(), high.as_ref(), &at.scale.config.domain)?;
        Ok(ScaleAt {
            scale: at.scale.with_domain(domain),
            marks_fit: at.marks_fit,
        })
    }
}

/// Whether the range of a position group runs in its reverse direction, as its spec's range says.
fn reversed(group: &ScaleGroup) -> bool {
    group.spec.range.as_ref().is_some_and(|range| range.reverse)
}

/// The fitting scale of a position group, from its spec, its `scale` node's computed style, the
/// selection states, and the values of its members with the reach of their marks.
pub(crate) fn position(
    chart: &Chart,
    group: &ScaleGroup,
    style: &ComputedStyle,
    registry: &Registry,
    states: &BTreeMap<String, SelectionState>,
    values: &[Option<MemberValues>],
) -> Result<Position> {
    let spec = &group.spec;
    let unfitted = unfitted(spec, style, registry, &value_type(values))?;
    // A continuous `y` runs up the plot, and a discrete one down it, so its first category is at
    // the top.
    let kind = spec.r#type.as_ref().and_then(FitKind::of);
    let up = group.channel == Channel::Y && kind != Some(FitKind::Categories);
    let direction = match up != reversed(group) {
        true => Direction::Reverse,
        false => Direction::Forward,
    };
    let fit = domain_options(chart, spec);
    let position = match source(chart, group, states)? {
        DomainSource::Explicit(domain) => explicit(&unfitted, domain, kind, values)?,
        DomainSource::Inferred { summaries, data } => Position {
            fitting: match inferred_summary(&unfitted, &summaries, &data, values)? {
                Some(summary) => fitting_of(unfitted.clone(), summary, &fit)?,
                None => FittingScale::from(unfitted.clone()),
            },
            ends: None,
            clip: false,
        },
        DomainSource::Ends {
            low,
            high,
            fallback,
        } => {
            let fitting = match *fallback {
                DomainSource::Explicit(domain) => {
                    explicit(&unfitted, domain, kind, values)?.fitting
                }
                DomainSource::Inferred { summaries, data } => {
                    match inferred_summary(&unfitted, &summaries, &data, values)? {
                        Some(summary) => fitting_of(unfitted.clone(), summary, &fit)?,
                        None => FittingScale::from(unfitted.clone()),
                    }
                }
                DomainSource::Ends { .. } => unreachable!("a fallback is not a selection domain"),
            };
            Position {
                fitting,
                ends: Some((low, high)),
                clip: true,
            }
        }
    };
    Ok(Position {
        fitting: position.fitting.with_direction(direction),
        ..position
    })
}

/// The position scale of an explicit domain, which the plot clips. A band or a point scale keeps
/// its categories and fits its clip padding to the reach of its marks.
fn explicit(
    unfitted: &ConfiguredScale,
    domain: ArrayRef,
    kind: Option<FitKind>,
    values: &[Option<MemberValues>],
) -> Result<Position> {
    let scale = unfitted.clone().with_domain(domain.clone());
    let ordinal = unfitted.scale_impl.scale_type() == "ordinal";
    let fitting = match (kind, summarize(unfitted, values.iter().flatten())?) {
        (Some(FitKind::Categories), Some(summary)) if !ordinal => FittingScale::new(
            scale,
            summary,
            DomainOptions::Categories(CategoryOptions {
                order: CategoryOrder::Explicit(domain),
            }),
        ),
        _ => FittingScale::from(scale),
    };
    Ok(Position {
        fitting,
        ends: None,
        clip: true,
    })
}

/// A non-position scale with its domain and its range: the domain from its source, fitted without
/// a length, and the range from the computed `scheme` or `range` of its `scale` node. A scale whose
/// styles give no range keeps the range from 0 to 1, with a warning.
pub(crate) fn non_position(
    chart: &Chart,
    group: &ScaleGroup,
    style: &ComputedStyle,
    styles: &ComputedStyles,
    registry: &Registry,
    states: &BTreeMap<String, SelectionState>,
    values: &[Option<ArrayRef>],
    warnings: &mut Vec<EvalWarning>,
) -> Result<(ConfiguredScale, bool)> {
    let spec = &group.spec;
    let values: Vec<Option<MemberValues>> = (values.iter())
        .map(|values| values.clone().map(|values| (values, None)))
        .collect();
    let unfitted = unfitted(spec, style, registry, &value_type(&values))?;
    let fit = domain_options(chart, spec);
    let fitted = |source: DomainSource| -> Result<ArrayRef> {
        Ok(match source {
            DomainSource::Explicit(domain) => domain,
            DomainSource::Inferred { summaries, data } => {
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
            DomainSource::Ends { .. } => unreachable!("the ends resolve below"),
        })
    };
    let source = source(chart, group, states)?;
    let clip = source.clips();
    let domain = match source {
        DomainSource::Ends {
            low,
            high,
            fallback,
        } => with_ends(low.as_ref(), high.as_ref(), &fitted(*fallback)?)?,
        source => fitted(source)?,
    };
    let scale = unfitted.with_domain(domain.clone());
    let shape = range_shape(spec, domain.len());
    let scale = match range(style, styles, shape) {
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
    Ok((scale, clip))
}
