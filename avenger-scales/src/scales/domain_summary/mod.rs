//! Domain inference: summaries of data that merge, and from which a scale's domain is fitted at any
//! plot size.
//!
//! [`ScaleImpl::summarize`](crate::scales::ScaleImpl::summarize) summarizes values, with
//! the pixel extents of the marks drawn at them, into a [`DomainSummary`]. A summary depends only
//! on the data and the scale's coordinate, so summaries of several tables, or of appended data,
//! merge. [`ScaleImpl::fit_domain`](crate::scales::ScaleImpl::fit_domain) then
//! fits the scale's domain for its range, following [`DomainOptions`]. A summary's Arrow form,
//! from [`DomainSummary::to_arrow`], is how queries return summaries.
//!
//! An [`IntervalSummary`] fits the narrowest linear domain that keeps every marker inside a plot of
//! a given width. Only the markers on two convex hull chains can decide a fit, and which markers
//! those are does not depend on the width. Values are in the scale's coordinate, so log, pow, and
//! symlog scales fit transformed values and invert the endpoints. `including` and `fit_nice`
//! constrain the fit, because widening a fitted domain to a nice one can push a marker at the edge
//! out of the plot.
//!
//! A [`CategorySummary`] holds distinct categories with the largest extents of each. Band and point
//! scales fit their clip padding from it, so that marks at the outer categories stay in the range.
//!
//! Scales defined in other crates use the same helpers as the built-in scales: [`interval_summary`]
//! with [`fit_continuous`] or [`fit_interval`] for interval domains, and [`CategorySummary::new`]
//! with [`fit_discrete`] or [`fit_categories`] for categories.
//!
//! ```
//! use avenger_scales::scales::domain_summary::{IntervalSummary, NiceStep};
//! // Markers at 5 and 10 that reach 5 px on each side, prepared once and fitted at two widths.
//! let markers = IntervalSummary::new(&[5.0, 10.0], &[5.0, 5.0], &[5.0, 5.0]).unwrap();
//! let (start, end) = markers.fit(100.0).unwrap();
//! let (wide_start, wide_end) = markers.fit(400.0).unwrap();
//! assert!(start < wide_start && wide_end < end);
//! // A domain that includes zero and ends on round values.
//! let nice = markers.including(0.0).unwrap().fit_nice(100.0, NiceStep::Auto(10.0)).unwrap();
//! assert_eq!(nice, (0.0, 11.0));
//! ```

mod category;
pub(crate) mod encoding;
pub(crate) mod fit;
mod interval;
mod sample;

use arrow::array::ArrayRef;
use thiserror::Error;

pub use category::CategorySummary;
pub use fit::{
    fit_categories, fit_continuous, fit_discrete, fit_interval, interval_summary,
    linear_single_value_span, IntervalFit,
};
pub use interval::{IntervalSummary, NiceStep};
pub use sample::SampleSummary;

/// Why data has no summary or no fitted domain.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum DomainFitError {
    /// Input that cannot be summarized or fitted: a value, extent, or width that is not finite, a
    /// negative extent, a non-positive width, mismatched lengths, values that all coincide, values
    /// whose span exceeds the `f64` range, or summaries that do not merge.
    #[error("invalid domain fit input: {0}")]
    InvalidInput(String),
    /// Markers that do not fit in the width at any domain.
    #[error("marker extents cannot fit: {0}")]
    Infeasible(String),
}

/// How a scale fits its domain to a summary, by kind of summary. Only the fit reads these options,
/// so summaries do not depend on them.
#[derive(Debug, Clone)]
pub enum DomainOptions {
    /// For linear, log, pow, symlog, time, and quantize scales.
    Interval(IntervalOptions),
    /// For band, point, nested band, and ordinal scales.
    Categories(CategoryOptions),
    /// For quantile scales, which take no options.
    Sample,
}

impl DomainOptions {
    /// The options of an interval scale, or an error for other kinds of options.
    pub fn interval(&self) -> Result<&IntervalOptions, DomainFitError> {
        match self {
            DomainOptions::Interval(options) => Ok(options),
            _ => Err(DomainFitError::InvalidInput(
                "an interval scale fits with interval options".into(),
            )),
        }
    }

    /// The options of a category scale, or an error for other kinds of options.
    pub fn categories(&self) -> Result<&CategoryOptions, DomainFitError> {
        match self {
            DomainOptions::Categories(options) => Ok(options),
            _ => Err(DomainFitError::InvalidInput(
                "a category scale fits with category options".into(),
            )),
        }
    }
}

/// How an interval scale fits its domain.
#[derive(Debug, Clone, Default)]
pub struct IntervalOptions {
    /// Whether the domain contains zero.
    pub zero: bool,
    /// The tick count for a nice domain, whose ends are round values. A log scale's nice domain
    /// ends on powers of the base, whatever the count.
    pub nice: Option<f64>,
    /// The domain when there is no data. Each scale has its own default.
    pub empty: Option<ArrayRef>,
}

/// How a category scale orders its domain.
#[derive(Debug, Clone, Default)]
pub struct CategoryOptions {
    /// The order of the categories.
    pub order: CategoryOrder,
}

/// The order of a discrete scale's categories.
#[derive(Debug, Clone, Default)]
pub enum CategoryOrder {
    /// Ascending. Nested paths sort level by level, so each level sorts within its parent.
    #[default]
    Ascending,
    /// Descending, level by level for nested paths.
    Descending,
    /// Exactly these categories, in this order. Categories outside the list are not in the domain,
    /// and listed categories without data still have positions. Orders by first appearance or by
    /// an aggregate come from a query over the data and arrive here.
    Explicit(ArrayRef),
}

/// The pixel extents of the marks drawn at each value: how far a mark reaches from the value's
/// position toward smaller values or earlier categories (`lower`), and toward larger values or
/// later categories (`upper`).
#[derive(Debug, Clone, Copy, Default)]
pub enum Extents<'a> {
    /// Marks that do not reach past their positions.
    #[default]
    None,
    /// The same extents for every value.
    Constant { lower: f64, upper: f64 },
    /// One pair of extents for each value.
    PerRow { lower: &'a [f64], upper: &'a [f64] },
}

impl Extents<'_> {
    /// The extents for each of `len` values, checked.
    fn per_row(&self, len: usize) -> Result<(Vec<f64>, Vec<f64>), DomainFitError> {
        let (lower, upper) = match *self {
            Extents::None => (vec![0.0; len], vec![0.0; len]),
            Extents::Constant { lower, upper } => (vec![lower; len], vec![upper; len]),
            Extents::PerRow { lower, upper } => {
                if lower.len() != len || upper.len() != len {
                    return Err(DomainFitError::InvalidInput(
                        "values and extents must have the same length".into(),
                    ));
                }
                (lower.to_vec(), upper.to_vec())
            }
        };
        check_extents(lower.iter().chain(&upper))?;
        Ok((lower, upper))
    }
}

/// Fails unless every extent is finite and nonnegative.
fn check_extents<'a>(extents: impl IntoIterator<Item = &'a f64>) -> Result<(), DomainFitError> {
    if extents
        .into_iter()
        .any(|extent| !extent.is_finite() || *extent < 0.0)
    {
        return Err(DomainFitError::InvalidInput(
            "extents must be finite and nonnegative".into(),
        ));
    }
    Ok(())
}

/// A summary of data for fitting a scale's domain.
#[derive(Debug, Clone, PartialEq)]
pub enum DomainSummary {
    /// Values in the scale's coordinate, for linear, log, pow, symlog, time, and quantize scales.
    Interval(IntervalSummary),
    /// Categories, for band, point, nested band, and ordinal scales.
    Categories(CategorySummary),
    /// A sample of values, for quantile scales.
    Sample(SampleSummary),
}

impl DomainSummary {
    /// Both summaries as one. A summary without data changes nothing. Summaries of different kinds
    /// or category types do not merge.
    pub fn merge(&self, other: &Self) -> Result<Self, DomainFitError> {
        match (self, other) {
            (DomainSummary::Interval(summary), DomainSummary::Interval(other)) => {
                Ok(DomainSummary::Interval(summary.merge(other)))
            }
            (DomainSummary::Categories(summary), DomainSummary::Categories(other)) => {
                Ok(DomainSummary::Categories(summary.merge(other)?))
            }
            (DomainSummary::Sample(summary), DomainSummary::Sample(other)) => {
                Ok(DomainSummary::Sample(summary.merge(other)))
            }
            _ => Err(DomainFitError::InvalidInput(
                "summaries of different kinds do not merge".into(),
            )),
        }
    }
}
