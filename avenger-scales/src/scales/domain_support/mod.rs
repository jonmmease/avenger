//! Domain inference: summaries of data, called supports, that combine with `union` and fit a
//! scale's domain at any plot size.
//!
//! [`ScaleImpl::domain_support`](crate::scales::ScaleImpl::domain_support) summarizes values, with
//! the pixel extents of the marks drawn at them, into a [`DomainSupport`]. A support depends only
//! on the data and the scale's coordinate, so shared scales, layers, and appended data take the
//! union of their supports. [`ScaleImpl::fit_support`](crate::scales::ScaleImpl::fit_support) then
//! fits the scale's domain for its range, following [`DomainOptions`].
//!
//! An [`IntervalSupport`] fits the narrowest linear domain that keeps every marker inside a plot of
//! a given width. Only the markers on two convex hull chains can decide a fit, and which markers
//! those are does not depend on the width. Values are in the scale's coordinate, so log, pow, and
//! symlog scales fit transformed values and invert the endpoints. `including` and the rounded fits
//! `fit_nice` and `fit_grid` constrain the fit, because rounding a fitted domain outward can push a
//! marker at the edge out of the plot.
//!
//! A [`UniqueSupport`] holds distinct categories with the largest extents of each. Band and point
//! scales fit their clip padding from it, so that marks at the outer categories stay in the range.
//!
//! ```
//! use avenger_scales::scales::domain_support::IntervalSupport;
//! // Markers at 5 and 10 that reach 5 px on each side, prepared once and fitted at two widths.
//! let markers = IntervalSupport::new(&[5.0, 10.0], &[5.0, 5.0], &[5.0, 5.0]).unwrap();
//! let (start, end) = markers.fit(100.0).unwrap();
//! let (wide_start, wide_end) = markers.fit(400.0).unwrap();
//! assert!(start < wide_start && wide_end < end);
//! // A domain that includes zero and ends on round values.
//! assert_eq!(markers.including(0.0).unwrap().fit_nice(100.0, 10.0).unwrap(), (0.0, 11.0));
//! ```

pub(crate) mod fit;
mod interval;
mod sample;
mod unique;

use arrow::array::ArrayRef;
use arrow::datatypes::DataType;
use thiserror::Error;

pub use interval::IntervalSupport;
pub use sample::SampleSupport;
pub use unique::UniqueSupport;

/// Why data has no support or no fitted domain.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum DomainFitError {
    /// Input that cannot be summarized or fitted: a value, extent, or width that is not finite, a
    /// negative extent, a non-positive width, mismatched lengths, values whose span exceeds the
    /// `f64` range, or supports that do not combine.
    #[error("invalid domain fit input: {0}")]
    InvalidInput(String),
    /// Markers that do not fit in the width at any domain.
    #[error("marker extents cannot fit: {0}")]
    Infeasible(String),
}

/// How a scale fits its domain to a support. Only the fit reads these options, so supports do not
/// depend on them.
#[derive(Debug, Clone, Default)]
pub struct DomainOptions {
    /// The order of a discrete scale's categories.
    pub order: CategoryOrder,
    /// Whether an interval domain contains zero.
    pub zero: bool,
    /// The tick count for rounding an interval domain to nice values. Log scales round to powers of
    /// the base, whatever the count.
    pub nice: Option<f64>,
    /// The domain when there is no data. Each scale has its own default.
    pub empty: Option<ArrayRef>,
    /// An interval domain's span when every value is the same, in the scale's coordinate. Each
    /// scale has its own default.
    pub single_value_span: Option<f64>,
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
    /// Whether no mark reaches past its position.
    pub(crate) fn is_none(&self) -> bool {
        matches!(self, Extents::None)
    }

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

/// The coordinate in which an interval support's values are linear.
#[derive(Debug, Clone, PartialEq)]
pub enum Coordinate {
    /// The values themselves.
    Linear,
    /// `log_base(d)`, or `−log_base(−d)` for a domain of negative values.
    Log { base: f64, negative: bool },
    /// `sign(d)·|d|^exponent`.
    Pow { exponent: f64 },
    /// The symmetric log transform with `constant`.
    Symlog { constant: f64 },
    /// Milliseconds since the epoch, for a domain of `data_type`.
    Time { data_type: DataType },
}

/// The support of a scale's domain.
#[derive(Debug, Clone, PartialEq)]
pub enum DomainSupport {
    /// Values in `coordinate`, for linear, log, pow, symlog, time, and quantize scales.
    Interval {
        coordinate: Coordinate,
        support: IntervalSupport,
    },
    /// Categories, for band, point, nested band, and ordinal scales.
    Unique(UniqueSupport),
    /// A sample of values, for quantile scales.
    Sample(SampleSupport),
}

impl DomainSupport {
    /// Both supports as one. A support without data changes nothing. Supports of different kinds,
    /// coordinates, or category types do not combine.
    pub fn union(&self, other: &Self) -> Result<Self, DomainFitError> {
        match (self, other) {
            (
                DomainSupport::Interval {
                    coordinate,
                    support,
                },
                DomainSupport::Interval {
                    coordinate: other_coordinate,
                    support: other_support,
                },
            ) => {
                if support.extent().is_none() {
                    return Ok(other.clone());
                }
                if other_support.extent().is_none() {
                    return Ok(self.clone());
                }
                if coordinate != other_coordinate {
                    return Err(DomainFitError::InvalidInput(format!(
                        "supports in {coordinate:?} and {other_coordinate:?} do not combine"
                    )));
                }
                Ok(DomainSupport::Interval {
                    coordinate: coordinate.clone(),
                    support: support.union(other_support),
                })
            }
            (DomainSupport::Unique(support), DomainSupport::Unique(other)) => {
                Ok(DomainSupport::Unique(support.union(other)?))
            }
            (DomainSupport::Sample(support), DomainSupport::Sample(other)) => {
                Ok(DomainSupport::Sample(support.union(other)))
            }
            _ => Err(DomainFitError::InvalidInput(
                "supports of different kinds do not combine".into(),
            )),
        }
    }
}
