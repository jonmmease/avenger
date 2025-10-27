//! Iterator abstraction for band scale position iteration
//!
//! Provides consistent iteration over band positions with support for:
//! - Standard band positions (start of each band)
//! - Centered band positions (for label placement)
//! - Custom band offset positions
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
    /// Numeric position of the band start
    pub position: f32,
    /// Width/height of the band
    pub bandwidth: f32,
}

impl BandPosition {
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
///     let subplot_y = band_pos.position;
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

    /// Create an iterator with custom band parameter (0.0 = start, 0.5 = center, 1.0 = end)
    ///
    /// Useful for positioning labels at band centers:
    /// ```ignore
    /// let iter = BandPositionIterator::from_scale_with_band(&row_scale, 0.5)?;
    /// for band_pos in iter {
    ///     let label_y = band_pos.position;  // Already at center
    /// }
    /// ```
    pub fn from_scale_with_band(
        scale: &ConfiguredScaleWithSpec,
        band_offset: f32,
    ) -> Result<Self, AvengerChartError> {
        use avenger_scales::scales::band;

        let configured = scale.configured();
        let domain_vals = configured.domain_values()?;

        // Create modified config with custom band offset
        let mut modified_config = configured.config.clone();
        modified_config.options.insert(
            "band".to_string(),
            avenger_scales::scalar::Scalar::from_f32(band_offset),
        );

        let modified_scale = avenger_scales::scales::ConfiguredScale {
            scale_impl: configured.scale_impl.clone(),
            config: modified_config,
        };

        let positions = match &domain_vals {
            DomainValues::Discrete(vals) => {
                modified_scale.scale_scalars_to_numeric(vals)?
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

    /// Create an iterator from a ConfiguredScale with custom band parameter
    pub fn from_configured_scale_with_band(
        scale: &ConfiguredScale,
        band_offset: f32,
    ) -> Result<Self, AvengerChartError> {
        use avenger_scales::scales::band;

        let domain_vals = scale.domain_values()?;

        // Create modified config with custom band offset
        let mut modified_config = scale.config.clone();
        modified_config.options.insert(
            "band".to_string(),
            avenger_scales::scalar::Scalar::from_f32(band_offset),
        );

        let modified_scale = ConfiguredScale {
            scale_impl: scale.scale_impl.clone(),
            config: modified_config,
        };

        let positions = match &domain_vals {
            DomainValues::Discrete(vals) => {
                modified_scale.scale_scalars_to_numeric(vals)?
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
