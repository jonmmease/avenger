//! Iterator abstraction for band scale position iteration
//!
//! Provides consistent iteration over band positions. The iterator returns
//! `BandPosition` structs with `start()`, `center()`, and `end()` methods
//! for explicitly choosing position within each band.
//!
//! Eliminates duplication of band position calculation logic across faceting system.

use datafusion::common::ScalarValue;
use crate::scales::ConfiguredScaleWithSpec;
use crate::scales::extensions::{ConfiguredScaleLegendExt, DomainValues};
use crate::error::AvengerChartError;
use avenger_scales::scales::ConfiguredScale;

/// Position information for a single band in a band scale
#[derive(Debug, Clone)]
pub struct BandPosition {
    /// The domain value (e.g., "setosa", "versicolor", "virginica")
    pub value: ScalarValue,
    /// Numeric position of the band start (private - use start(), center(), or end())
    position: f32,
    /// Width/height of the band
    pub bandwidth: f32,
}

impl BandPosition {
    /// Create a new BandPosition
    pub fn new(value: ScalarValue, position: f32, bandwidth: f32) -> Self {
        Self {
            value,
            position,
            bandwidth,
        }
    }

    /// Get the start position of this band
    pub fn start(&self) -> f32 {
        self.position
    }

    /// Get the center position of this band
    pub fn center(&self) -> f32 {
        self.position + self.bandwidth / 2.0
    }

    /// Get the end position of this band
    pub fn end(&self) -> f32 {
        self.position + self.bandwidth
    }
}

/// Iterator over band positions from a configured band scale
///
/// Provides consistent iteration over domain values with their corresponding
/// positions and bandwidth for faceted layouts.
///
/// # Example
///
/// ```ignore
/// let iter = BandPositionIterator::from_scale(&row_scale)?;
/// for band_pos in iter {
///     let subplot_y = band_pos.start();  // or .center() or .end()
///     let subplot_height = band_pos.bandwidth;
///     // render subplot at (x, subplot_y) with height subplot_height
/// }
/// ```
pub struct BandPositionIterator {
    domain_vals: Vec<ScalarValue>,
    positions: Vec<f32>,
    bandwidth: f32,
    current_index: usize,
}

impl BandPositionIterator {
    /// Create an iterator from a configured band scale
    ///
    /// Returns positions at the start of each band (band offset = 0.0)
    pub fn from_scale(scale: &ConfiguredScaleWithSpec) -> Result<Self, AvengerChartError> {
        use avenger_scales::scales::band;

        let configured = scale.configured();
        let domain_vals = configured.domain_values()?;

        let positions = match &domain_vals {
            DomainValues::Discrete(vals) => {
                configured.scale_scalars_to_numeric(vals)?
            }
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


    /// Get the number of bands
    pub fn len(&self) -> usize {
        self.domain_vals.len()
    }

    /// Check if iterator is empty
    pub fn is_empty(&self) -> bool {
        self.domain_vals.is_empty()
    }

    /// Get the bandwidth (same for all bands)
    pub fn bandwidth(&self) -> f32 {
        self.bandwidth
    }

    /// Create an iterator from a ConfiguredScale directly
    ///
    /// This is a convenience method for when you already have a ConfiguredScale
    /// instead of a ConfiguredScaleWithSpec.
    pub fn from_configured_scale(scale: &ConfiguredScale) -> Result<Self, AvengerChartError> {
        use avenger_scales::scales::band;

        let domain_vals = scale.domain_values()?;

        let positions = match &domain_vals {
            DomainValues::Discrete(vals) => {
                scale.scale_scalars_to_numeric(vals)?
            }
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

        Some(BandPosition {
            value,
            position,
            bandwidth: self.bandwidth,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.domain_vals.len() - self.current_index;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for BandPositionIterator {}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::common::ScalarValue;

    #[test]
    fn test_band_position_methods() {
        let bp = BandPosition::new(
            ScalarValue::Utf8(Some("test".into())),
            100.0,
            50.0,
        );

        assert_eq!(bp.start(), 100.0);
        assert_eq!(bp.center(), 125.0);
        assert_eq!(bp.end(), 150.0);
        assert_eq!(bp.bandwidth, 50.0);
    }

    #[test]
    fn test_band_position_zero_bandwidth() {
        let bp = BandPosition::new(
            ScalarValue::Utf8(Some("zero".into())),
            100.0,
            0.0,
        );

        assert_eq!(bp.start(), 100.0);
        assert_eq!(bp.center(), 100.0);
        assert_eq!(bp.end(), 100.0);
    }

    #[test]
    fn test_band_position_negative_position() {
        let bp = BandPosition::new(
            ScalarValue::Utf8(Some("negative".into())),
            -50.0,
            20.0,
        );

        assert_eq!(bp.start(), -50.0);
        assert_eq!(bp.center(), -40.0);
        assert_eq!(bp.end(), -30.0);
    }
}
