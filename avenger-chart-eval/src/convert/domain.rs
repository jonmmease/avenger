//! Where the domains of scales come from: explicit values, the producer's summaries, the data of
//! the layers, or the values of a selection, and how a summary or a data domain fits.

use std::collections::BTreeMap;

use arrow::{
    array::ArrayRef,
    compute::{cast, concat, kernels::cmp::lt},
};
use avenger_chart::{
    proto::{
        self, category_fit, domain, domain_fit, interval_fit, selection_domain, CategorySort,
        Channel,
    },
    resolve::{scale_channel, ScaleGroup},
    Chart, FitKind,
};
use avenger_scales::scales::domain_summary::{
    CategoryOptions, CategoryOrder, DomainOptions, DomainPin, DomainSummary, IntervalOptions,
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
    /// that `data` marks, one flag for each member, and the end of a selection's ranges that the
    /// fit pins when the ranges leave the other end open.
    Inferred {
        summaries: Vec<DomainSummary>,
        data: Vec<bool>,
        pin: Option<DomainPin<ArrayRef>>,
    },
}

/// The explicit domain `explicit` with the end that `pin` gives, in the type of `explicit`. An end
/// past the other end closes the domain at the pin, so the domain never runs backward.
fn pinned_explicit(explicit: &ArrayRef, pin: &DomainPin<ArrayRef>) -> Result<ArrayRef> {
    let (first, last) = (explicit.slice(0, 1), explicit.slice(explicit.len() - 1, 1));
    let typed = |end: &ArrayRef| cast(end, explicit.data_type());
    let (low, high) = match pin {
        DomainPin::Low(low) => {
            let low = typed(low)?;
            match lt(&last, &low)?.value(0) {
                true => (low.clone(), low),
                false => (low, last),
            }
        }
        DomainPin::High(high) => {
            let high = typed(high)?;
            match lt(&high, &first)?.value(0) {
                true => (high.clone(), high),
                false => (first, high),
            }
        }
        DomainPin::Mid(_) => unreachable!("a selection pins an end"),
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
        pin: None,
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
                        pin: None,
                    },
                    Some(selection_domain::Fallback::Data(_)) | None => DomainSource::Inferred {
                        summaries: Vec::new(),
                        data: vec![true; members],
                        pin: None,
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
                    // Ranges that leave one end open pin the other, and the fallback gives the
                    // open end.
                    Some(SelectionValues::Range { low, high }) => {
                        let pin = match (low, high) {
                            (Some(low), _) => DomainPin::Low(low),
                            (None, Some(high)) => DomainPin::High(high),
                            (None, None) => unreachable!("a range bounds an end"),
                        };
                        match fallback {
                            DomainSource::Explicit(explicit) => {
                                DomainSource::Explicit(pinned_explicit(&explicit, &pin)?)
                            }
                            DomainSource::Inferred {
                                summaries, data, ..
                            } => DomainSource::Inferred {
                                summaries,
                                data,
                                pin: Some(pin),
                            },
                        }
                    }
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
    /// This fit pinned at `pin`, a selection's end, in place of a pin of the spec: an interval fit
    /// pins, and a category fit keeps its options.
    pub(crate) fn pinned(mut self, pin: Option<DomainPin<ArrayRef>>) -> Self {
        if let (Some(pin), DomainOptions::Interval(options)) = (pin, &mut self.options) {
            options.pin = Some(pin);
        }
        self
    }

    /// Whether the fit pins a value, so the domain can exclude data and its plot clips.
    pub(crate) fn pins(&self) -> bool {
        matches!(&self.options, DomainOptions::Interval(options) if options.pin.is_some())
    }

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

/// The fit of a summary or a data domain of `spec`: the options of `kind`, the kind of fit of its
/// type, with the fields of the spec's fit, or of a selection domain's fallback.
pub(crate) fn domain_options(chart: &Chart, spec: &proto::ScaleSpec, kind: FitKind) -> Fit {
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
    let mut preferred = None;
    let options = match (kind, fit.and_then(|fit| fit.fit.as_ref())) {
        (FitKind::Sample, _) => DomainOptions::Sample,
        (_, Some(domain_fit::Fit::Categories(categories))) => {
            let (options, listed) = category_options(chart, categories);
            preferred = listed;
            DomainOptions::Categories(options)
        }
        (FitKind::Categories, None) => DomainOptions::Categories(CategoryOptions::default()),
        (_, Some(domain_fit::Fit::Interval(interval))) => {
            DomainOptions::Interval(interval_options(chart, interval))
        }
        _ => DomainOptions::Interval(IntervalOptions::default()),
    };
    Fit { options, preferred }
}

fn interval_options(chart: &Chart, fit: &proto::IntervalFit) -> IntervalOptions {
    let value = |value: &proto::ValueRef| chart.value(value).clone();
    IntervalOptions {
        zero: fit.zero.unwrap_or(false),
        nice: fit.nice,
        empty: fit.empty.as_ref().map(|empty| chart.list_elements(empty)),
        pin: fit.domain_pin.as_ref().map(|pin| match pin {
            interval_fit::DomainPin::Low(low) => DomainPin::Low(value(low)),
            interval_fit::DomainPin::Mid(mid) => DomainPin::Mid(value(mid)),
            interval_fit::DomainPin::High(high) => DomainPin::High(value(high)),
        }),
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
