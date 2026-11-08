//! The state of an evaluation that changes without a new chart, and the options of an update that
//! installs a new chart: the identities that carry the state to the new chart, the views of
//! navigated scales, and how the fitted domains, the views, and the clauses of selections carry.

use std::collections::BTreeMap;

use arrow::{
    array::{Array, ArrayRef, BooleanArray},
    compute::{concat, filter, kernels::cmp::eq},
};
use avenger_chart::{
    proto::{self, domain, Channel},
    resolve::{Identity, Resolved, ScaleGroup},
    Chart, FitKind,
};
use avenger_scales::scales::{
    domain_summary::{CategoryOptions, CategoryOrder, DomainOptions},
    fitting::FittingScale,
    ConfiguredScale,
};

use avenger_selection::SelectionState;

use crate::{
    convert::domain::{domain_options, FitDefaults},
    fit::Position,
    geo, EvalError,
};

type Result<T> = std::result::Result<T, EvalError>;

/// The identity of a scale, which carries its state, such as the view of a pan or a zoom, to a new
/// chart.
#[derive(Clone, Debug, PartialEq)]
pub enum ScaleIdentity {
    /// The share family that the specs of the scale name, with the scale's channel.
    Family { family: String, channel: Channel },
    /// The first plot of the scale with an identity, in tree order, the scale's channel, and the
    /// position of the scale among the plot's scales of the channel: 0, or 1 for the second scale
    /// of a channel in one plot.
    Plot {
        plot: Identity,
        channel: Channel,
        index: usize,
    },
}

/// The identity of a projection, which carries its state, such as the view of a pan or a zoom, to
/// a new chart.
#[derive(Clone, Debug, PartialEq)]
pub enum ProjectionIdentity {
    /// The share family that the geo specs of the projection name.
    Family(String),
    /// The first geo plot of the projection with an identity, in tree order.
    Plot(Identity),
}

/// The identity of each scale group of `resolved`, by its position in `Resolved::scales`, or
/// `None` for a group without a family whose plots have no ids.
pub(crate) fn scale_identities(resolved: &Resolved) -> Vec<Option<ScaleIdentity>> {
    let tree = &resolved.tree;
    let plots_of = |group: &ScaleGroup| {
        let mut plots: Vec<usize> = (group.members.iter())
            .map(|member| tree.layers[member.layer].plot)
            .collect();
        plots.sort_unstable();
        plots.dedup();
        plots
    };
    (resolved.scales.iter())
        .map(|group| {
            if !group.spec.family.is_empty() {
                return Some(ScaleIdentity::Family {
                    family: group.spec.family.clone(),
                    channel: group.channel,
                });
            }
            let plot =
                (plots_of(group).into_iter()).find(|plot| tree.plots[*plot].identity.is_some())?;
            // The position of the group among the plot's groups of its channel.
            let index = (resolved.scales.iter())
                .filter(|other| other.channel == group.channel && plots_of(other).contains(&plot))
                .position(|other| std::ptr::eq(other, group))?;
            Some(ScaleIdentity::Plot {
                plot: tree.plots[plot].identity.clone()?,
                channel: group.channel,
                index,
            })
        })
        .collect()
}

/// The identity of each projection group of `resolved`, by its position in
/// `Resolved::projections`: the share family that its specs name, or else the identity of its
/// first plot with one.
pub(crate) fn projection_identities(resolved: &Resolved) -> Vec<Option<ProjectionIdentity>> {
    let tree = &resolved.tree;
    (resolved.projections.iter())
        .map(|group| {
            if !group.spec.family.is_empty() {
                return Some(ProjectionIdentity::Family(group.spec.family.clone()));
            }
            (group.plots.iter())
                .find_map(|plot| tree.plots[*plot].identity.clone())
                .map(ProjectionIdentity::Plot)
        })
        .collect()
}

/// The first scale group of each channel of each plot of `resolved`, by plot.
pub(crate) fn plot_groups(resolved: &Resolved) -> Vec<BTreeMap<Channel, usize>> {
    let mut plots = vec![BTreeMap::new(); resolved.tree.plots.len()];
    for (index, group) in resolved.scales.iter().enumerate() {
        for member in &group.members {
            let plot = resolved.tree.layers[member.layer].plot;
            plots[plot].entry(group.channel).or_insert(index);
        }
    }
    plots
}

/// Whether the domain of `spec` reads a selection.
pub(crate) fn selection_domain(spec: &proto::Scale) -> bool {
    matches!(
        spec.domain
            .as_ref()
            .and_then(|domain| domain.domain.as_ref()),
        Some(domain::Domain::Selection(_))
    )
}

/// Whether the domain of `spec` is explicit.
pub(crate) fn explicit_domain(spec: &proto::Scale) -> bool {
    matches!(
        spec.domain
            .as_ref()
            .and_then(|domain| domain.domain.as_ref()),
        Some(domain::Domain::Explicit(_))
    )
}

/// The navigation of a scale at the size of its plot: the bounds that its domain stays inside, and
/// its smallest width, in the numeric values of the scale, such as epoch milliseconds for a time
/// scale.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScaleNavigation {
    pub bounds: Option<[f64; 2]>,
    pub min_span: Option<f64>,
}

/// The view of a position group: the scale that it shows, whose domain replaces the fitted domain
/// at every length, and the domain of the fitted scale that it started from.
#[derive(Clone, Debug)]
pub(crate) struct ScaleView {
    pub(crate) scale: ConfiguredScale,
    pub(crate) start: ArrayRef,
}

impl ScaleView {
    /// Whether a navigation moved the view from the domain that it started from. An evaluation
    /// drops a view that did not move, such as the view of a press without a drag, so the scale
    /// fits again.
    pub(crate) fn moved(&self) -> bool {
        self.scale.domain().as_ref() != self.start.as_ref()
    }
}

/// How an update carries the fitted domain of a scale, the domain that the scale shows without a
/// view and that a double click returns to, and the fit of a projection, what its geo plot shows
/// without a view. A scale carries over when the new chart has a scale with its identity and its
/// kind of fit, and a projection when the new chart has a projection with its identity. An explicit
/// domain in the new chart's spec, a domain that reads a selection, and a projection that fits a
/// box or the whole sphere take no option.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fit {
    /// The domain fits the new chart's rows, or its summary when the spec gives one, and the
    /// projection fits the new chart's data.
    #[default]
    Refit,
    /// The domain or the fit of the old chart. A position scale clips the marks outside it, as a
    /// geo plot that fits its data does.
    Keep,
    /// The smallest domain or fit that holds the old one and the new rows: for a continuous
    /// scale, the interval over both, for a discrete scale, the old categories in their order
    /// followed by the new ones, and for a projection, the extent of the old data and the new. A
    /// position scale clips the marks outside it.
    Extend,
}

/// What a fit option names: a scale or a projection, by its identity.
#[derive(Clone, Debug, PartialEq)]
pub enum Fitted {
    Scale(ScaleIdentity),
    Projection(ProjectionIdentity),
}

impl From<ScaleIdentity> for Fitted {
    fn from(identity: ScaleIdentity) -> Self {
        Self::Scale(identity)
    }
}

impl From<ProjectionIdentity> for Fitted {
    fn from(identity: ProjectionIdentity) -> Self {
        Self::Projection(identity)
    }
}

/// Whether an update keeps the views of pans and zooms, for the scales and the projections that
/// carry over.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Views {
    #[default]
    Keep,
    /// Every scale shows its fitted domain, and every projection its fit.
    Drop,
}

/// How an update carries the clauses of a selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Carry {
    /// The clauses of the old chart, without the clauses of a selection that is not global whose
    /// owner is not a plot or a layer of the new chart. A selection that the new chart does not
    /// define stays.
    #[default]
    Keep,
    /// The initial clauses of the new chart, or none. A selection that the new chart does not
    /// define goes.
    Reset,
}

/// What an update carries from the old chart to the new one: a [`Fit`] for every scale and every
/// projection, with overrides by identity, a [`Views`], and a [`Carry`] for every selection, with
/// overrides by name.
#[derive(Clone, Debug, Default)]
pub struct UpdateOptions {
    fit: Fit,
    fits: Vec<(Fitted, Fit)>,
    views: Views,
    carry: Carry,
    carries: Vec<(String, Carry)>,
}

impl UpdateOptions {
    /// The defaults: `Fit::Refit`, `Views::Keep`, and `Carry::Keep`.
    pub fn new() -> Self {
        Self::default()
    }

    /// The fit of every scale and every projection without an override.
    pub fn fit(mut self, fit: Fit) -> Self {
        self.fit = fit;
        self
    }

    /// The fit of the scale or the projection with `identity`, in place of the fit of every
    /// scale and every projection.
    pub fn fit_of(mut self, identity: impl Into<Fitted>, fit: Fit) -> Self {
        let identity = identity.into();
        self.fits.retain(|(own, _)| *own != identity);
        self.fits.push((identity, fit));
        self
    }

    /// Whether the scales and the projections that carry over keep the views of pans and zooms.
    pub fn views(mut self, views: Views) -> Self {
        self.views = views;
        self
    }

    /// The carry of every selection without an override.
    pub fn selections(mut self, carry: Carry) -> Self {
        self.carry = carry;
        self
    }

    /// The carry of the selection `name`, in place of the carry of every selection.
    pub fn selection(mut self, name: impl Into<String>, carry: Carry) -> Self {
        let name = name.into();
        self.carries.retain(|(own, _)| *own != name);
        self.carries.push((name, carry));
        self
    }

    /// The fit of the scale with `identity`.
    pub(crate) fn scale_fit(&self, identity: &ScaleIdentity) -> Fit {
        self.fit_where(|own| matches!(own, Fitted::Scale(own) if own == identity))
    }

    /// The fit of the projection with `identity`.
    #[cfg_attr(not(feature = "geo"), allow(dead_code))]
    pub(crate) fn projection_fit(&self, identity: &ProjectionIdentity) -> Fit {
        self.fit_where(|own| matches!(own, Fitted::Projection(own) if own == identity))
    }

    /// The fit of the first override that `named` accepts, or the fit of every scale and every
    /// projection.
    fn fit_where(&self, named: impl Fn(&Fitted) -> bool) -> Fit {
        (self.fits.iter())
            .find(|(own, _)| named(own))
            .map_or(self.fit, |(_, fit)| *fit)
    }

    pub(crate) fn keeps_views(&self) -> bool {
        self.views == Views::Keep
    }

    /// The carry of the selection `name`.
    pub(crate) fn carry_for(&self, name: &str) -> Carry {
        (self.carries.iter())
            .find(|(own, _)| own == name)
            .map_or(self.carry, |(_, carry)| *carry)
    }
}

/// What an evaluator keeps between the evaluations of one chart, and carries across an update to a
/// new chart as its `UpdateOptions` say: the clauses of the selections, the views of the position
/// scales and of the projections, and the fitted domains that an update carried from the old chart.
#[derive(Default)]
pub(crate) struct EvaluationState {
    /// The state of each selection by name, with its clauses.
    pub(crate) selections: BTreeMap<String, SelectionState>,
    /// The view of each position group that has one, by its position in `Resolved::scales`.
    pub(crate) views: BTreeMap<usize, ScaleView>,
    /// The fitted domains that an update carried from the old chart, by scale group.
    pub(crate) carried: BTreeMap<usize, Carried>,
    /// The views and the carried fits of the projections of geo plots.
    pub(crate) projections: geo::ProjectionState,
    /// The modes that a mode widget or the host turned on or off, by name, in place of their
    /// initial states while the chart declares them.
    pub(crate) modes: BTreeMap<String, bool>,
}

/// A fitted domain that an update carries from the old chart to a scale group of the new one,
/// which the fit of the group applies each time it prepares.
#[derive(Clone, Debug)]
pub(crate) enum Carried {
    /// A position scale of the old chart, under `Keep` or `Extend`.
    Position(Fit, Position),
    /// A non-position scale of the old chart, under `Keep` or `Extend`.
    Scale(Fit, ConfiguredScale),
}

/// The position scale of a group of the new chart, whose kind of fit is `kind`, under `fit`,
/// `Keep` or `Extend`, from its scale `old` in the old chart and its fitted scale `new`, where
/// `defaults` are the fit's defaults for the group.
pub(crate) fn carry_position(
    chart: &Chart,
    group: &ScaleGroup,
    kind: FitKind,
    fit: Fit,
    old: &Position,
    new: &Position,
    defaults: FitDefaults,
) -> Result<Position> {
    // A carried fit keeps the direction of the new fit.
    let direction = new.fitting.direction();
    let fitting = match (fit, kind) {
        (Fit::Keep, _) => old.fitting.clone().with_direction(direction),
        (Fit::Extend, FitKind::Interval) => match (old.fitting.summary(), new.fitting.summary()) {
            (Some(old_summary), Some(new_summary)) => FittingScale::new(
                new.fitting.scale().clone(),
                (old_summary.merge(new_summary))
                    .map_err(avenger_scales::error::AvengerScaleError::from)?,
                domain_options(chart, &group.spec, kind, defaults).options,
            )
            .with_direction(direction),
            _ => return Ok(new.clone()),
        },
        (Fit::Extend, FitKind::Categories) => {
            let categories = |position: &Position| -> Result<ArrayRef> {
                Ok(position.fitting.at((0.0, 1.0))?.scale.domain().clone())
            };
            let order = union_categories(&categories(old)?, &categories(new)?)?;
            match (old.fitting.summary(), new.fitting.summary()) {
                (Some(old_summary), Some(new_summary)) => FittingScale::new(
                    new.fitting.scale().clone(),
                    (old_summary.merge(new_summary))
                        .map_err(avenger_scales::error::AvengerScaleError::from)?,
                    DomainOptions::Categories(CategoryOptions {
                        order: CategoryOrder::Explicit(order),
                    }),
                )
                .with_direction(direction),
                _ => FittingScale::from(new.fitting.scale().clone().with_domain(order))
                    .with_direction(direction),
            }
        }
        _ => return Ok(new.clone()),
    };
    Ok(Position {
        fitting,
        clip: true,
    })
}

/// The non-position scale of a group of the new chart, whose kind of fit is `kind`, under `fit`,
/// `Keep` or `Extend`, from its scale `old` in the old chart and its scale `new`.
pub(crate) fn carry_scale(
    kind: FitKind,
    fit: Fit,
    old: &ConfiguredScale,
    new: &ConfiguredScale,
) -> Result<ConfiguredScale> {
    Ok(match (fit, kind) {
        (Fit::Keep, _) => new.clone().with_domain(old.domain().clone()),
        (Fit::Extend, FitKind::Interval) if old.domain().len() == 2 && new.domain().len() == 2 => {
            match (old.numeric_interval_domain(), new.numeric_interval_domain()) {
                (Ok((a, b)), Ok((c, d))) => {
                    let (low, high) = (a.min(b).min(c.min(d)), a.max(b).max(c.max(d)));
                    let ends = if c <= d { (low, high) } else { (high, low) };
                    new.clone().with_domain_interval(ends)
                }
                _ => new.clone(),
            }
        }
        (Fit::Extend, FitKind::Categories) => new
            .clone()
            .with_domain(union_categories(old.domain(), new.domain())?),
        _ => new.clone(),
    })
}

/// The categories of `old` in their order, followed by those of `new` that `old` does not hold.
pub(crate) fn union_categories(old: &ArrayRef, new: &ArrayRef) -> Result<ArrayRef> {
    if old.data_type() != new.data_type() {
        return Ok(old.clone());
    }
    let added = (0..new.len())
        .map(|index| {
            let held = eq(old, &arrow::array::Scalar::new(new.slice(index, 1)))?;
            Ok(Some(held.true_count() == 0))
        })
        .collect::<Result<BooleanArray>>()?;
    let added = filter(new, &added)?;
    Ok(concat(&[old.as_ref(), added.as_ref()])?)
}
