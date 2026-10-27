//! Scales whose domains fit the length of their range.
//!
//! A [`FittingScale`] holds a configured scale and, for a domain that follows the data, the
//! summary and the options that the domain fits. [`FittingScale::at`] sets the range to a length
//! and fits the domain there, so an axis that a layout measures at several lengths, and the marks
//! that the plot draws at its final length, all see the domain of their own length.
//!
//! ```
//! use avenger_scales::scales::domain_summary::{DomainOptions, Extents, IntervalOptions};
//! use avenger_scales::scales::fitting::FittingScale;
//! use avenger_scales::scales::linear::LinearScale;
//! use std::sync::Arc;
//!
//! // Points at 0 and 10 that reach 5 px on each side.
//! let scale = LinearScale::unfitted((0.0, 1.0));
//! let values: arrow::array::ArrayRef = Arc::new(arrow::array::Float64Array::from(vec![0.0, 10.0]));
//! let summary = scale.summarize(&values, Extents::Constant { lower: 5.0, upper: 5.0 }).unwrap();
//! let fitting = FittingScale::new(scale, summary, DomainOptions::Interval(IntervalOptions::default()));
//! let narrow = fitting.at(100.0).unwrap().scale.numeric_interval_domain().unwrap();
//! let wide = fitting.at(400.0).unwrap().scale.numeric_interval_domain().unwrap();
//! // A longer range needs less room in data units for the same pixels.
//! assert!(narrow.0 < wide.0 && wide.1 < narrow.1);
//! ```

use crate::error::AvengerScaleError;
use crate::scales::domain_summary::{DomainFitError, DomainOptions, DomainSummary};
use crate::scales::ConfiguredScale;

/// A scale whose domain fits the length of its range, or keeps its configured domain.
#[derive(Debug, Clone)]
pub struct FittingScale {
    /// The scale as configured. Its range gives the direction of the range at every length.
    scale: ConfiguredScale,
    /// The summary that the domain fits, with its options, or `None` for a configured domain.
    fit: Option<(DomainSummary, DomainOptions)>,
}

/// A [`FittingScale`] at one length.
#[derive(Debug, Clone)]
pub struct ScaleAt {
    /// The scale with its range set to the length and its domain fitted there.
    pub scale: ConfiguredScale,
    /// Whether every mark fits inside the range. When no domain keeps the marks inside, the
    /// domain covers the values without room for the marks, and this is false.
    pub marks_fit: bool,
}

impl FittingScale {
    /// A scale whose domain fits `summary` with `options` at each length.
    pub fn new(scale: ConfiguredScale, summary: DomainSummary, options: DomainOptions) -> Self {
        Self {
            scale,
            fit: Some((summary, options)),
        }
    }

    /// The scale as configured, before any length sets its range.
    pub fn scale(&self) -> &ConfiguredScale {
        &self.scale
    }

    /// The summary that the domain fits, or `None` for a scale that keeps its configured domain.
    pub fn summary(&self) -> Option<&DomainSummary> {
        self.fit.as_ref().map(|(summary, _)| summary)
    }

    /// The scale at a range of `length`, from 0 to `length`, or from `length` to 0 for a scale
    /// whose configured range decreases, as a y scale's does. A scale with a summary fits its
    /// domain, or the clip padding of a band or point scale, to that range.
    pub fn at(&self, length: f32) -> Result<ScaleAt, AvengerScaleError> {
        let (start, end) = self.scale.numeric_interval_range()?;
        let length = f64::from(length);
        let range = if end < start {
            (length, 0.0)
        } else {
            (0.0, length)
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

/// A scale that keeps its configured domain at every length.
impl From<ConfiguredScale> for FittingScale {
    fn from(scale: ConfiguredScale) -> Self {
        Self { scale, fit: None }
    }
}

/// A copy of `scale` that keeps its configured domain at every length.
impl From<&ConfiguredScale> for FittingScale {
    fn from(scale: &ConfiguredScale) -> Self {
        scale.clone().into()
    }
}
