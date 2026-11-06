//! Where the domains of scales come from: explicit values, the producer's summaries, the data of
//! the layers, or the values of a selection, and how a summary or a data domain fits.

use std::collections::BTreeMap;

use arrow::{
    array::ArrayRef,
    compute::{cast, concat, kernels::cmp::lt},
};
use avenger_chart::{
    proto::{self, category_fit, domain, domain_fit, selection_domain, CategorySort, Channel},
    resolve::{scale_channel, ScaleGroup},
    Chart, FitKind,
};
use avenger_scales::scales::domain_summary::{
    CategoryOptions, CategoryOrder, DomainOptions, DomainSummary, IntervalOptions,
};
use avenger_selection::SelectionState;

use super::selection::{domain_values, SelectionValues};
use crate::EvalError;

type Result<T> = std::result::Result<T, EvalError>;

/// Where a scale's domain comes from at one evaluation.
#[derive(Clone, Debug)]
pub(crate) enum DomainSource {
    /// A domain that the scale keeps at every size, from the spec or a selection, whose plot clips
    /// its marks.
    Explicit(ArrayRef),
    /// The producer's summaries, merged with the summaries of the data of the members of the group
    /// that `data` marks, one flag for each member.
    Inferred {
        summaries: Vec<DomainSummary>,
        data: Vec<bool>,
    },
    /// The ranges of a selection with an end that no range bounds: the domain takes that end from
    /// its fallback, once the fallback is known.
    Ends {
        low: Option<ArrayRef>,
        high: Option<ArrayRef>,
        fallback: Box<DomainSource>,
    },
}

/// The two ends of a selection domain: its own ends, and an end that it leaves open from `fallback`,
/// the domain that its fallback gave, low then high, in the type of the fallback, which is the type
/// that the scale's fit gives. An end that passes the fallback's other end closes the domain at
/// that end, so the domain never runs backward.
pub(crate) fn with_ends(
    low: Option<&ArrayRef>,
    high: Option<&ArrayRef>,
    fallback: &ArrayRef,
) -> Result<ArrayRef> {
    let typed = |end: &ArrayRef| cast(end, fallback.data_type());
    let (low, high) = match (low, high) {
        (Some(low), Some(high)) => (typed(low)?, typed(high)?),
        (Some(low), None) => {
            let (low, high) = (typed(low)?, fallback.slice(fallback.len() - 1, 1));
            match lt(&high, &low)?.value(0) {
                true => (low.clone(), low),
                false => (low, high),
            }
        }
        (None, Some(high)) => {
            let (low, high) = (fallback.slice(0, 1), typed(high)?);
            match lt(&high, &low)?.value(0) {
                true => (high.clone(), high),
                false => (low, high),
            }
        }
        (None, None) => return Ok(fallback.clone()),
    };
    Ok(concat(&[low.as_ref(), high.as_ref()])?)
}

/// The source of the domain of `group` at the selection states `states`.
pub(crate) fn source(
    chart: &Chart,
    group: &ScaleGroup,
    states: &BTreeMap<String, SelectionState>,
) -> Result<DomainSource> {
    let members = group.members.len();
    let inferred = || DomainSource::Inferred {
        summaries: (group.members.iter())
            // The second channel of a pair shares its first's summary.
            .filter(|member| scale_channel(member.channel) == member.channel)
            .filter_map(|member| member.summary.as_ref())
            .map(|summary| summary_of(chart, summary))
            .collect(),
        data: group
            .members
            .iter()
            .map(|member| member.summary.is_none())
            .collect(),
    };
    Ok(
        match group
            .spec
            .domain
            .as_ref()
            .and_then(|domain| domain.domain.as_ref())
        {
            Some(domain::Domain::Explicit(values)) => {
                DomainSource::Explicit(chart.list_elements(values))
            }
            Some(domain::Domain::Selection(selection)) => {
                let fallback = match &selection.fallback {
                    Some(selection_domain::Fallback::Explicit(values)) => {
                        DomainSource::Explicit(chart.list_elements(values))
                    }
                    Some(selection_domain::Fallback::Summary(summary)) => DomainSource::Inferred {
                        summaries: summary
                            .value
                            .as_ref()
                            .map(|value| summary_of(chart, value))
                            .into_iter()
                            .collect(),
                        data: vec![false; members],
                    },
                    Some(selection_domain::Fallback::Data(_)) | None => DomainSource::Inferred {
                        summaries: Vec::new(),
                        data: vec![true; members],
                    },
                };
                let channel = Channel::try_from(selection.channel).unwrap_or_default();
                let values = match states.get(&selection.selection) {
                    Some(state) => domain_values(state, channel)?,
                    None => None,
                };
                match values {
                    Some(SelectionValues::Set(values)) => DomainSource::Explicit(values),
                    Some(SelectionValues::Range {
                        low: Some(low),
                        high: Some(high),
                    }) => DomainSource::Explicit(concat(&[low.as_ref(), high.as_ref()])?),
                    Some(SelectionValues::Range { low, high }) => DomainSource::Ends {
                        low,
                        high,
                        fallback: Box::new(fallback),
                    },
                    None => fallback,
                }
            }
            Some(domain::Domain::Summary(_) | domain::Domain::Data(_)) | None => inferred(),
        },
    )
}

/// The producer's summary that `value` refers to.
fn summary_of(chart: &Chart, value: &proto::ValueRef) -> DomainSummary {
    DomainSummary::from_arrow(chart.value(value).as_ref(), 0)
        .expect("a valid chart's summary has the layout of its scale")
}

/// How a summary or a data domain of a scale fits: the options of avenger-scales, and the
/// categories that come first in a discrete domain.
#[derive(Clone, Debug)]
pub(crate) struct Fit {
    pub(crate) options: DomainOptions,
    /// A preferred order: the listed categories that the domain holds come first, in the list's
    /// order, before the other categories, ascending. Listed categories without data are not in the
    /// domain, so the fit sorts ascending and then moves them.
    pub(crate) preferred: Option<ArrayRef>,
}

impl Fit {
    /// A fitted discrete domain in the preferred order, or `domain` without one.
    pub(crate) fn order(&self, domain: ArrayRef) -> Result<ArrayRef> {
        let Some(preferred) = &self.preferred else {
            return Ok(domain);
        };
        let keys = |values: &ArrayRef| -> Result<Vec<Vec<u8>>> {
            let converter = arrow::row::RowConverter::new(vec![arrow::row::SortField::new(
                values.data_type().clone(),
            )])?;
            let rows = converter.convert_columns(std::slice::from_ref(values))?;
            Ok(rows.iter().map(|row| row.as_ref().to_vec()).collect())
        };
        let preferred = arrow::compute::cast(preferred, domain.data_type())?;
        let (domain_keys, preferred_keys) = (keys(&domain)?, keys(&preferred)?);
        let mut indices: Vec<u32> = Vec::with_capacity(domain.len());
        for key in &preferred_keys {
            if let Some(index) = domain_keys.iter().position(|known| known == key) {
                if !indices.contains(&(index as u32)) {
                    indices.push(index as u32);
                }
            }
        }
        let first = indices.clone();
        indices.extend((0..domain.len() as u32).filter(|index| !first.contains(index)));
        Ok(arrow::compute::take(
            &domain,
            &arrow::array::UInt32Array::from(indices),
            None,
        )?)
    }
}

/// The fit of a summary or a data domain of `spec`: the options of the kind of its type, with the
/// fields of the spec's fit, or of a selection domain's fallback.
pub(crate) fn domain_options(chart: &Chart, spec: &proto::ScaleSpec) -> Fit {
    let fit = match spec
        .domain
        .as_ref()
        .and_then(|domain| domain.domain.as_ref())
    {
        Some(domain::Domain::Summary(summary)) => summary.fit.as_ref(),
        Some(domain::Domain::Data(data)) => data.fit.as_ref(),
        Some(domain::Domain::Selection(selection)) => match &selection.fallback {
            Some(selection_domain::Fallback::Summary(summary)) => summary.fit.as_ref(),
            Some(selection_domain::Fallback::Data(data)) => data.fit.as_ref(),
            _ => None,
        },
        _ => None,
    };
    let kind = spec.r#type.as_ref().and_then(FitKind::of);
    let mut preferred = None;
    let options = match (kind, fit.and_then(|fit| fit.fit.as_ref())) {
        (Some(FitKind::Sample), _) => DomainOptions::Sample,
        (_, Some(domain_fit::Fit::Categories(categories))) => {
            let (options, listed) = category_options(chart, categories);
            preferred = listed;
            DomainOptions::Categories(options)
        }
        (Some(FitKind::Categories), None) => DomainOptions::Categories(CategoryOptions::default()),
        (_, Some(domain_fit::Fit::Interval(interval))) => {
            DomainOptions::Interval(interval_options(chart, interval))
        }
        _ => DomainOptions::Interval(IntervalOptions::default()),
    };
    Fit { options, preferred }
}

fn interval_options(chart: &Chart, fit: &proto::IntervalFit) -> IntervalOptions {
    IntervalOptions {
        zero: fit.zero.unwrap_or(false),
        nice: fit.nice,
        empty: fit.empty.as_ref().map(|empty| chart.list_elements(empty)),
    }
}

/// The category options of a fit, and its preferred categories, which the fit moves first after
/// it sorts the categories ascending.
fn category_options(
    chart: &Chart,
    fit: &proto::CategoryFit,
) -> (CategoryOptions, Option<ArrayRef>) {
    let (order, preferred) = match &fit.order {
        Some(category_fit::Order::Sort(sort)) => {
            let order = match CategorySort::try_from(*sort).unwrap_or_default() {
                CategorySort::Descending => CategoryOrder::Descending,
                _ => CategoryOrder::Ascending,
            };
            (order, None)
        }
        Some(category_fit::Order::Preferred(preferred)) => (
            CategoryOrder::Ascending,
            Some(chart.list_elements(preferred)),
        ),
        None => (CategoryOrder::Ascending, None),
    };
    (CategoryOptions { order }, preferred)
}
