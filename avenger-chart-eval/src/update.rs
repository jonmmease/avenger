//! The options of an update, which installs a new chart in a `ChartEvaluator` and decides what
//! of the old chart's state carries over: the fitted domains of its scales, the views of pans and
//! zooms, and the contributions of its selections.

use arrow::{
    array::{Array, ArrayRef, BooleanArray},
    compute::{concat, filter, kernels::cmp::eq},
};

use crate::{EvalError, ScaleIdentity};

/// How an update carries the fitted domain of a scale, the domain that the scale shows without a
/// view and that a double click returns to. A scale carries over when the new chart has a scale
/// with its identity and its scale type. An explicit domain in the new chart's spec, and a domain
/// that reads a selection, take no option.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fit {
    /// The domain fits the new chart's rows, or its summary when the spec gives one.
    #[default]
    Refit,
    /// The domain of the old chart. A position scale clips the marks outside it.
    Keep,
    /// The smallest domain that holds the old domain and the new rows: for a continuous scale,
    /// the interval over both, and for a discrete scale, the old categories in their order
    /// followed by the new ones. A position scale clips the marks outside it.
    Extend,
}

/// Whether an update keeps the views of pans and zooms, for the scales that carry over.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Views {
    #[default]
    Keep,
    /// Every scale shows its fitted domain.
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

/// What an update carries from the old chart to the new one: a [`Fit`] for every scale, with
/// overrides by scale identity, a [`Views`], and a [`Carry`] for every selection, with overrides
/// by name.
#[derive(Clone, Debug, Default)]
pub struct UpdateOptions {
    fit: Fit,
    fits: Vec<(ScaleIdentity, Fit)>,
    views: Views,
    carry: Carry,
    carries: Vec<(String, Carry)>,
}

impl UpdateOptions {
    /// The defaults: `Fit::Refit`, `Views::Keep`, and `Carry::Keep`.
    pub fn new() -> Self {
        Self::default()
    }

    /// The fit of every scale without an override.
    pub fn fit(mut self, fit: Fit) -> Self {
        self.fit = fit;
        self
    }

    /// The fit of the scale with `identity`, in place of the fit of every scale.
    pub fn fit_of(mut self, identity: ScaleIdentity, fit: Fit) -> Self {
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
    pub(crate) fn fit_for(&self, identity: &ScaleIdentity) -> Fit {
        (self.fits.iter())
            .find(|(own, _)| own == identity)
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
