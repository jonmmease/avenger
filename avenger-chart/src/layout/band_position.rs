//! Band-scale position iteration shared by child-frame container layouts.
//!
//! The iterator returns `BandPosition` structs with `start()`, `center()`, and
//! `end()` methods for explicitly choosing placement within each band.

use avenger_chart_core::{BandPosition, ConfiguredScaleLegendExt, DomainValues};
use avenger_scales::scales::{ConfiguredScale, band};
use datafusion::common::ScalarValue;

use crate::{error::AvengerChartError, scales::ConfiguredScaleWithSpec};

/// Iterator over band positions from a configured band scale.
pub struct BandPositionIterator {
    domain_vals: Vec<ScalarValue>,
    positions: Vec<f32>,
    bandwidth: f32,
    current_index: usize,
}

impl BandPositionIterator {
    /// Create an iterator from a configured band scale.
    pub fn from_scale(scale: &ConfiguredScaleWithSpec) -> Result<Self, AvengerChartError> {
        let configured = scale.configured();
        let domain_vals = configured.domain_values()?;

        let positions = match &domain_vals {
            DomainValues::Discrete(vals) => configured.scale_scalars_to_numeric(vals)?,
            _ => Vec::new(),
        };

        let bandwidth = band::bandwidth(&configured.config)?;

        let domain_vals = match domain_vals {
            DomainValues::Discrete(vals) => vals,
            _ => Vec::new(),
        };

        Ok(Self {
            domain_vals,
            positions,
            bandwidth,
            current_index: 0,
        })
    }

    pub fn len(&self) -> usize {
        self.domain_vals.len()
    }

    pub fn is_empty(&self) -> bool {
        self.domain_vals.is_empty()
    }

    pub fn bandwidth(&self) -> f32 {
        self.bandwidth
    }

    /// Create an iterator from a configured scale directly.
    pub fn from_configured_scale(scale: &ConfiguredScale) -> Result<Self, AvengerChartError> {
        let domain_vals = scale.domain_values()?;

        let positions = match &domain_vals {
            DomainValues::Discrete(vals) => scale.scale_scalars_to_numeric(vals)?,
            _ => Vec::new(),
        };

        let bandwidth = band::bandwidth(&scale.config)?;

        let domain_vals = match domain_vals {
            DomainValues::Discrete(vals) => vals,
            _ => Vec::new(),
        };

        Ok(Self {
            domain_vals,
            positions,
            bandwidth,
            current_index: 0,
        })
    }
}

impl Iterator for BandPositionIterator {
    type Item = BandPosition;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current_index >= self.domain_vals.len() {
            return None;
        }

        let index = self.current_index;
        let value = self.domain_vals[index].clone();
        let position = self.positions.get(index).copied().unwrap_or(0.0);

        self.current_index += 1;

        Some(BandPosition::new(value, position, self.bandwidth))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.domain_vals.len() - self.current_index;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for BandPositionIterator {}
