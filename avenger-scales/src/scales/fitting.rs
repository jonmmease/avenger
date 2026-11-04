//! Scales whose domains fit the extent of their range.
//!
//! A [`FittingScale`] holds a configured scale, the [`Direction`] of its range, and, for a domain
//! that follows the data, the summary and the options that the domain fits. [`FittingScale::at`]
//! sets the range across an extent in that direction and fits the domain there, so an axis that a
//! layout measures at several lengths, and the marks that the plot draws at its final length, all
//! see the domain of their own length.
//!
//! ```
//! use avenger_scales::scales::domain_summary::{DomainOptions, Extents, IntervalOptions};
//! use avenger_scales::scales::fitting::{Direction, FittingScale};
//! use avenger_scales::scales::linear::LinearScale;
//! use std::sync::Arc;
//!
//! // Points at 0 and 10 that reach 5 px on each side.
//! let scale = LinearScale::unfitted((0.0, 1.0));
//! let values: arrow::array::ArrayRef = Arc::new(arrow::array::Float64Array::from(vec![0.0, 10.0]));
//! let summary = scale.summarize(&values, Extents::Constant { lower: 5.0, upper: 5.0 }).unwrap();
//! let fitting = FittingScale::new(scale, summary, DomainOptions::Interval(IntervalOptions::default()));
//! let narrow = fitting.at((0.0, 100.0)).unwrap().scale.numeric_interval_domain().unwrap();
//! let wide = fitting.at((0.0, 400.0)).unwrap().scale.numeric_interval_domain().unwrap();
//! // A longer range needs less room in data units for the same pixels.
//! assert!(narrow.0 < wide.0 && wide.1 < narrow.1);
//! // A y scale that runs up the plot starts its domain at the high end of the extent.
//! let up = fitting.with_direction(Direction::Reverse).at((0.0, 100.0)).unwrap().scale;
//! assert_eq!(up.numeric_interval_range().unwrap(), (100.0, 0.0));
//! ```

use crate::error::AvengerScaleError;
use crate::scales::domain_summary::{DomainFitError, DomainOptions, DomainSummary};
use crate::scales::ConfiguredScale;

/// Which end of an extent the start of a fitting scale's domain lies at.
///
/// The direction is relative to the extent, not to the screen. The positions of a Cartesian plot
/// are pixels from its top-left corner, so a y scale that runs up the plot is
/// [`Reverse`](Direction::Reverse).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Direction {
    /// The low end: a rightward x, a downward y, a clockwise theta, an outward radius.
    #[default]
    Forward,
    /// The high end: an upward y, a counterclockwise theta, an inward radius.
    Reverse,
}

/// A scale whose domain fits the extent of its range, or keeps its configured domain.
#[derive(Debug, Clone)]
pub struct FittingScale {
    /// The scale as configured. Only [`FittingScale::at`] sets its range.
    scale: ConfiguredScale,
    direction: Direction,
    /// The summary that the domain fits, with its options, or `None` for a configured domain.
    fit: Option<(DomainSummary, DomainOptions)>,
}

/// A [`FittingScale`] at one extent.
#[derive(Debug, Clone)]
pub struct ScaleAt {
    /// The scale with its range set across the extent and its domain fitted there.
    pub scale: ConfiguredScale,
    /// Whether every mark fits inside the range. When no domain keeps the marks inside, the
    /// domain covers the values without room for the marks, and this is false.
    pub marks_fit: bool,
}

impl FittingScale {
    /// A [`Forward`](Direction::Forward) scale whose domain fits `summary` with `options` at each
    /// extent. The order of `scale`'s range has no effect.
    pub fn new(scale: ConfiguredScale, summary: DomainSummary, options: DomainOptions) -> Self {
        Self {
            scale,
            direction: Direction::Forward,
            fit: Some((summary, options)),
        }
    }

    /// This scale with `direction`.
    pub fn with_direction(self, direction: Direction) -> Self {
        Self { direction, ..self }
    }

    /// Which end of an extent the start of the domain lies at.
    pub fn direction(&self) -> Direction {
        self.direction
    }

    /// The scale as configured, before an extent sets its range. The order of its range does not
    /// orient a fit, which [`direction`](Self::direction) does.
    pub fn scale(&self) -> &ConfiguredScale {
        &self.scale
    }

    /// The summary that the domain fits, or `None` for a scale that keeps its configured domain.
    pub fn summary(&self) -> Option<&DomainSummary> {
        self.fit.as_ref().map(|(summary, _)| summary)
    }

    /// The scale with its range across `extent`, whose ends can come in either order, from the
    /// low end to the high end for a [`Forward`](Direction::Forward) scale and from the high end to
    /// the low end for a [`Reverse`](Direction::Reverse) scale. A scale with a summary fits its
    /// domain, or the clip padding of a band or point scale, to that range.
    pub fn at(&self, extent: (f64, f64)) -> Result<ScaleAt, AvengerScaleError> {
        let (low, high) = (extent.0.min(extent.1), extent.0.max(extent.1));
        let range = match self.direction {
            Direction::Forward => (low, high),
            Direction::Reverse => (high, low),
        };
        let scale = self.scale.clone().with_range_interval(range);
        let Some((summary, options)) = &self.fit else {
            return Ok(ScaleAt {
                scale,
                marks_fit: true,
            });
        };
        match scale.clone().fit_domain(options, summary) {
            Ok(scale) => Ok(ScaleAt {
                scale,
                marks_fit: true,
            }),
            Err(AvengerScaleError::DomainFit(DomainFitError::Infeasible(_))) => Ok(ScaleAt {
                scale: scale.fit_domain(options, &summary.without_extents())?,
                marks_fit: false,
            }),
            Err(error) => Err(error),
        }
    }
}

/// A scale that keeps its configured domain at every extent, in the direction of its range:
/// [`Reverse`](Direction::Reverse) when the range decreases, and
/// [`Forward`](Direction::Forward) otherwise, including for a range that is not numeric.
impl From<ConfiguredScale> for FittingScale {
    fn from(scale: ConfiguredScale) -> Self {
        let direction = match scale.numeric_interval_range() {
            Ok((start, end)) if end < start => Direction::Reverse,
            _ => Direction::Forward,
        };
        Self {
            scale,
            direction,
            fit: None,
        }
    }
}

/// A copy of `scale` that keeps its configured domain at every extent, in the direction of its
/// range.
impl From<&ConfiguredScale> for FittingScale {
    fn from(scale: &ConfiguredScale) -> Self {
        scale.clone().into()
    }
}
