//! The options of an update, which installs a new chart in a `ChartEvaluator` and decides what
//! of the old chart's state carries over: the fitted domains of its scales, the views of pans and
//! zooms, and the contributions of its selections.

use arrow::{
    array::{Array, ArrayRef, BooleanArray},
    compute::{concat, filter, kernels::cmp::eq},
};

use crate::{EvalError, ProjectionIdentity, ScaleIdentity};

/// How an update carries the fitted domain of a scale, the domain that the scale shows without a
/// view and that a double click returns to, and the fit of a projection, what its map shows
/// without a view. A scale carries over when the new chart has a scale with its identity and its
/// scale type, and a projection when the new chart has a projection with its identity and its
/// surface. An explicit domain in the new chart's spec, a domain that reads a selection, and a
/// projection that fits a box or the whole sphere take no option.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fit {
    /// The domain fits the new chart's rows, or its summary when the spec gives one, and the
    /// projection fits the new chart's data.
    #[default]
    Refit,
    /// The domain or the fit of the old chart. A position scale clips the marks outside it, as a
    /// map that fits its data does.
    Keep,
    /// The smallest domain or fit that holds the old one and the new rows: for a continuous
    /// scale, the interval over both, for a discrete scale, the old categories in their order
    /// followed by the new ones, and for a projection, the extent of the old data and the new. A
    /// position scale clips the marks outside it, as a map that fits its data does.
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

/// How an update carries the contributions of a selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Carry {
    /// The contributions of the old chart, without the contributions of a union or an intersect
    /// selection whose owner is not a plot of the new chart. A selection that the new chart does
    /// not define stays.
    #[default]
    Keep,
    /// The initial contributions of the new chart, or none. A selection that the new chart does
    /// not define goes.
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

    /// Whether the scales that carry over keep the views of pans and zooms.
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

/// The categories of `old` in their order, followed by those of `new` that `old` does not hold.
pub(crate) fn union_categories(old: &ArrayRef, new: &ArrayRef) -> Result<ArrayRef, EvalError> {
    let error = |error: arrow::error::ArrowError| {
        EvalError::Evaluation(format!(
            "the union of the categories of two charts: {error}"
        ))
    };
    if old.data_type() != new.data_type() {
        return Ok(old.clone());
    }
    let added = (0..new.len())
        .map(|index| {
            let held = eq(old, &arrow::array::Scalar::new(new.slice(index, 1))).map_err(error)?;
            Ok(Some(held.true_count() == 0))
        })
        .collect::<Result<BooleanArray, EvalError>>()?;
    let added = filter(new, &added).map_err(error)?;
    concat(&[old.as_ref(), added.as_ref()]).map_err(error)
}
