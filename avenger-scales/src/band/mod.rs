pub mod opts;
use avenger_common::value::{ScalarOrArray, ScalarOrArrayRef};
use opts::BandScaleOptions;

use crate::error::AvengerScaleError;
use crate::ordinal::OrdinalScale;
use std::fmt::Debug;
use std::hash::Hash;

/// A band scale divides a continuous range into bands and computes positions based on a discrete domain.
///
/// Band scales are like ordinal scales but with continuous numeric output.
/// The continuous range is automatically divided into uniform bands.
/// Commonly used for bar charts with ordinal or categorical dimensions.
#[derive(Debug, Clone)]
pub struct BandScale<D: Debug + Clone + Hash + Eq + Sync + 'static> {
    domain: Vec<D>,
    ordinal_scale: OrdinalScale<D, f32>,
    range: (f32, f32),
    padding_inner: f32,
    padding_outer: f32,
    align: f32,
    round: bool,
}

impl<D: Debug + Clone + Hash + Eq + Sync + 'static> BandScale<D> {
    /// Creates a new band scale with the given domain.
    ///
    /// # Defaults
    /// - range: (0.0, 1.0)
    /// - padding_inner: 0.0
    /// - padding_outer: 0.0
    /// - align: 0.5
    /// - round: false
    pub fn try_new(domain: Vec<D>) -> Result<Self, AvengerScaleError> {
        let mut this = Self {
            domain: domain.clone(),
            // placeholder scale to be updated later
            ordinal_scale: OrdinalScale::new(&domain, &vec![f32::NAN; domain.len()], f32::NAN)?,
            range: (0.0, 1.0),
            padding_inner: 0.0,
            padding_outer: 0.0,
            align: 0.5,
            round: false,
        };

        this.update_ordinal_scale()?;
        Ok(this)
    }

    fn update_ordinal_scale(&mut self) -> Result<(), AvengerScaleError> {
        let n = self.domain.len();
        if n == 0 {
            return Err(AvengerScaleError::EmptyDomain);
        }

        let reverse = self.range.1 < self.range.0;
        let (start, stop) = if reverse {
            (self.range.1, self.range.0)
        } else {
            (self.range.0, self.range.1)
        };

        let step = (stop - start)
            / 1.0_f32.max(bandspace(
                n,
                Some(self.padding_inner),
                Some(self.padding_outer),
            ));
        let step = if self.round { step.floor() } else { step };

        let start = start + (stop - start - step * (n as f32 - self.padding_inner)) * self.align;
        let start = if self.round { start.round() } else { start };

        // Generate range values
        let range_values: Vec<f32> = (0..n).map(|i| start + step * i as f32).collect::<Vec<_>>();

        // Create final range values considering reverse
        let range_values = if reverse {
            range_values.into_iter().rev().collect()
        } else {
            range_values
        };

        // Create ordinal scale and map values
        self.ordinal_scale = OrdinalScale::new(&self.domain, &range_values, f32::NAN)?;
        Ok(())
    }

    /// Sets the output range as (min, max).
    ///
    /// The range may be reversed for inverted scales.
    pub fn range(mut self, range: (f32, f32)) -> Result<Self, AvengerScaleError> {
        self.range = range;
        self.update_ordinal_scale()?;
        Ok(self)
    }

    /// Sets the inner padding between bands to a value between 0 and 1.
    ///
    /// The inner padding determines the ratio of the range that is reserved for blank space
    /// between bands. A value of 0 means no blank space between bands, while a value of 1
    /// means the bands themselves have zero width.
    pub fn padding_inner(mut self, padding: f32) -> Result<Self, AvengerScaleError> {
        self.padding_inner = padding.clamp(0.0, 1.0);
        self.update_ordinal_scale()?;
        Ok(self)
    }

    /// Sets the outer padding to a non-negative value.
    ///
    /// The outer padding determines the ratio of the range that is reserved for blank space
    /// before the first band and after the last band.
    pub fn padding_outer(mut self, padding: f32) -> Result<Self, AvengerScaleError> {
        self.padding_outer = padding.max(0.0);
        self.update_ordinal_scale()?;
        Ok(self)
    }

    /// Sets both inner and outer padding to the same value.
    ///
    /// This is a convenience method equivalent to setting both padding_inner and padding_outer.
    pub fn padding(mut self, padding: f32) -> Result<Self, AvengerScaleError> {
        self.padding_inner = padding.clamp(0.0, 1.0);
        self.padding_outer = padding.max(0.0);
        self.update_ordinal_scale()?;
        Ok(self)
    }

    /// Sets the alignment of the bands within the range to a value between 0 and 1.
    ///
    /// The alignment determines how any leftover unused space is distributed.
    /// A value of 0.0 means the bands are aligned to the left/start,
    /// 0.5 means they are centered (default), and 1.0 means they are aligned to the right/end.
    pub fn align(mut self, align: f32) -> Result<Self, AvengerScaleError> {
        self.align = align.clamp(0.0, 1.0);
        self.update_ordinal_scale()?;
        Ok(self)
    }

    /// Enables or disables rounding of the band positions and widths.
    ///
    /// When rounding is enabled, the start and end positions of each band will be rounded to
    /// the nearest integer. This can be useful when pixel-perfect rendering is desired.
    pub fn round(mut self, round: bool) -> Result<Self, AvengerScaleError> {
        self.round = round;
        self.update_ordinal_scale()?;
        Ok(self)
    }

    /// Returns a reference to the scale's domain.
    pub fn get_domain(&self) -> &Vec<D> {
        &self.domain
    }

    /// Returns the scale's range as a tuple of (start, end).
    pub fn get_range(&self) -> (f32, f32) {
        self.range
    }

    /// Returns the inner padding value.
    pub fn get_padding_inner(&self) -> f32 {
        self.padding_inner
    }

    /// Returns the outer padding value.
    pub fn get_padding_outer(&self) -> f32 {
        self.padding_outer
    }

    /// Returns the alignment value.
    pub fn get_align(&self) -> f32 {
        self.align
    }

    /// Returns whether rounding is enabled.
    pub fn get_round(&self) -> bool {
        self.round
    }

    /// Returns the width of each band.
    ///
    /// Calculated from range, domain size, and padding settings.
    /// Returns 0 for empty domains.
    pub fn bandwidth(&self) -> f32 {
        let n = self.domain.len();
        if n == 0 {
            return 0.0;
        }

        let (start, stop) = if self.range.1 < self.range.0 {
            (self.range.1, self.range.0)
        } else {
            (self.range.0, self.range.1)
        };

        let step = (stop - start)
            / 1.0_f32.max(bandspace(
                n,
                Some(self.padding_inner),
                Some(self.padding_outer),
            ));
        let bandwidth = step * (1.0 - self.padding_inner);

        if self.round {
            bandwidth.round()
        } else {
            bandwidth
        }
    }

    /// Returns the distance between the starts of adjacent bands.
    ///
    /// The step size is calculated based on the range, domain size, and padding settings.
    /// Returns 0 if the domain is empty.
    pub fn step(&self) -> f32 {
        let n = self.domain.len();
        if n == 0 {
            return 0.0;
        }

        let (start, stop) = if self.range.1 < self.range.0 {
            (self.range.1, self.range.0)
        } else {
            (self.range.0, self.range.1)
        };

        let step = (stop - start)
            / 1.0_f32.max(bandspace(
                n,
                Some(self.padding_inner),
                Some(self.padding_outer),
            ));

        if self.round {
            step.floor()
        } else {
            step
        }
    }

    /// Maps input values to their corresponding band positions.
    ///
    /// Returns an error if the domain is empty.
    pub fn scale<'a>(
        &self,
        values: impl Into<ScalarOrArrayRef<'a, D>>,
        opts: &BandScaleOptions,
    ) -> ScalarOrArray<f32> {
        if opts.band.is_some() || opts.range_offset.is_some() {
            let band = opts.band.unwrap_or(0.0);
            let range_offset = opts.range_offset.unwrap_or(0.0);
            let offset = self.bandwidth() * band + range_offset;
            self.ordinal_scale.scale(values).map(|v| v + offset)
        } else {
            self.ordinal_scale.scale(values)
        }
    }

    /// Maps a range value back to the corresponding domain values
    pub fn invert_range(
        &self,
        range_values: (f32, f32),
        opts: &BandScaleOptions,
    ) -> Option<Vec<D>> {
        let (mut lo, mut hi) = range_values;

        // Bail if range values are invalid
        if lo.is_nan() || hi.is_nan() {
            return None;
        }

        // Order range inputs
        if hi < lo {
            std::mem::swap(&mut lo, &mut hi);
        }

        let reverse = self.range.1 < self.range.0;
        let (start, stop) = if reverse {
            (self.range.1, self.range.0)
        } else {
            (self.range.0, self.range.1)
        };

        // Bail if outside scale range
        if hi < start || lo > stop {
            return None;
        }

        // Calculate band positions
        let values = self
            .scale(&self.domain, opts)
            .as_vec(self.domain.len(), None);

        // Binary search for indices
        let mut a = values.partition_point(|&x| x <= lo).saturating_sub(1);
        let b = if (lo - hi).abs() < f32::EPSILON {
            a
        } else {
            values.partition_point(|&x| x <= hi).saturating_sub(1)
        };

        // Increment index if lo is within padding gap
        if lo - values[a] > self.bandwidth() + 1e-10 {
            a += 1;
        }

        // Handle reverse
        let (a, b) = if reverse {
            let n = values.len() - 1;
            (n - b, n - a)
        } else {
            (a, b)
        };

        if a > b {
            return None;
        }

        Some((a..=b).map(|i| self.domain[i].clone()).collect())
    }

    /// Maps a single range value back to the corresponding domain value
    pub fn invert(&self, value: f32, opts: &BandScaleOptions) -> Option<D> {
        self.invert_range((value, value), opts)
            .map(|array| array[0].clone())
    }
}

/// Calculates required steps for a band scale based on domain count and padding.
///
/// # Arguments
/// * `count` - Number of domain elements
/// * `padding_inner` - Inner padding [0.0, 1.0], defaults to 0.0
/// * `padding_outer` - Outer padding ≥ 0.0, defaults to 0.0
pub fn bandspace(count: usize, padding_inner: Option<f32>, padding_outer: Option<f32>) -> f32 {
    let padding_inner = padding_inner.unwrap_or(0.0).clamp(0.0, 1.0);
    let padding_outer = padding_outer.unwrap_or(0.0).max(0.0);

    let count = count as f32;
    count - padding_inner + padding_outer * 2.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use float_cmp::{assert_approx_eq, F32Margin};

    #[test]
    fn test_band_scale_defaults() -> Result<(), AvengerScaleError> {
        let domain = vec!["a", "b", "c"];
        let scale = BandScale::try_new(domain)?;

        assert_eq!(scale.get_range(), (0.0, 1.0));
        assert_eq!(scale.get_padding_inner(), 0.0);
        assert_eq!(scale.get_padding_outer(), 0.0);
        assert_eq!(scale.get_align(), 0.5);
        assert_eq!(scale.get_round(), false);
        Ok(())
    }

    #[test]
    fn test_band_scale_basic() -> Result<(), AvengerScaleError> {
        let domain = vec!["a", "b", "c"];
        let scale = BandScale::try_new(domain)?;

        let values = vec!["a", "b", "b", "c", "f"];
        let result = scale
            .scale(&values, &BandScaleOptions::default())
            .as_vec(values.len(), None);

        // With 3 bands in [0,1] and no padding, expect bands at 0.0, 0.333, 0.667
        assert_approx_eq!(f32, result[0], 0.0); // "a"
        assert_approx_eq!(f32, result[1], 0.3333333); // "b"
        assert_approx_eq!(f32, result[2], 0.3333333); // "b"
        assert_approx_eq!(f32, result[3], 0.6666667); // "c"
        assert!(result[4].is_nan()); // "f"
        assert_approx_eq!(f32, scale.bandwidth(), 0.3333333);
        assert_approx_eq!(f32, scale.step(), 0.3333333);

        Ok(())
    }

    #[test]
    fn test_band_scale_padding() -> Result<(), AvengerScaleError> {
        let domain = vec!["a", "b", "c"];
        let scale = BandScale::try_new(domain)?
            .range((0.0, 120.0))?
            .padding(0.2)?;

        let values = vec!["a", "b", "b", "c", "f"];
        let result = scale
            .scale(&values, &BandScaleOptions::default())
            .as_vec(values.len(), None);

        // With padding of 0.2, points should be inset
        assert_approx_eq!(f32, result[0], 7.5); // "a"
        assert_approx_eq!(f32, result[1], 45.0); // "b"
        assert_approx_eq!(f32, result[2], 45.0); // "b"
        assert_approx_eq!(f32, result[3], 82.5); // "c"
        assert!(result[4].is_nan()); // "f"

        assert_approx_eq!(f32, scale.bandwidth(), 30.0);

        Ok(())
    }

    #[test]
    fn test_band_scale_round() -> Result<(), AvengerScaleError> {
        let domain = vec!["a", "b", "c"];
        let scale = BandScale::try_new(domain)?
            .range((0.0, 100.0))?
            .round(true)?;

        let values = vec!["a", "b", "b", "c", "f"];
        let result = scale
            .scale(&values, &BandScaleOptions::default())
            .as_vec(values.len(), None);

        // With rounding, values should be integers
        assert_eq!(result[0], 1.0); // "a"
        assert_eq!(result[1], 34.0); // "b"
        assert_eq!(result[2], 34.0); // "b"
        assert_eq!(result[3], 67.0); // "c"
        assert!(result[4].is_nan()); // "f"
        assert_eq!(scale.bandwidth(), 33.0);

        Ok(())
    }

    #[test]
    fn test_bandspace() {
        // Test default padding (0, 0)
        assert_eq!(bandspace(3, None, None), 3.0);

        // Test with only inner padding
        assert_eq!(bandspace(3, Some(0.2), None), 2.8);

        // Test with only outer padding
        assert_eq!(bandspace(3, None, Some(0.5)), 4.0);

        // Test with both inner and outer padding
        assert_eq!(bandspace(3, Some(0.2), Some(0.5)), 3.8);

        // Test with invalid paddings (should clamp)
        assert_eq!(bandspace(3, Some(1.5), Some(-0.5)), 2.0); // inner clamped to 1.0, outer clamped to 0.0
    }

    #[test]
    fn test_band_scale_invert() -> Result<(), AvengerScaleError> {
        let domain = vec!["a", "b", "c"];
        let scale = BandScale::try_new(domain)?
            .range((0.0, 120.0))?
            .padding(0.2)?;

        // Test exact band positions
        let result = scale.invert(7.5, &BandScaleOptions::default()).unwrap();
        assert_eq!(result, "a");

        let result = scale.invert(45.0, &BandScaleOptions::default()).unwrap();
        assert_eq!(result, "b");

        // Test position within band
        let result = scale.invert(15.0, &BandScaleOptions::default()).unwrap();
        assert_eq!(result, "a");

        // Test position in padding (should return None)
        assert!(scale.invert(40.0, &BandScaleOptions::default()).is_none());

        // Test out of range
        assert!(scale.invert(-10.0, &BandScaleOptions::default()).is_none());
        assert!(scale.invert(130.0, &BandScaleOptions::default()).is_none());

        Ok(())
    }

    #[test]
    fn test_band_scale_invert_range() -> Result<(), AvengerScaleError> {
        let domain = vec!["a", "b", "c"];
        let scale = BandScale::try_new(domain)?
            .range((0.0, 120.0))?
            .padding(0.2)?;

        // Test range covering multiple bands
        let result = scale
            .invert_range((7.5, 82.5), &BandScaleOptions::default())
            .unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "a");
        assert_eq!(result[1], "b");
        assert_eq!(result[2], "c");

        // Test partial range
        let result = scale
            .invert_range((45.0, 82.5), &BandScaleOptions::default())
            .unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "b");
        assert_eq!(result[1], "c");

        // Test reversed range (should handle automatically)
        let result = scale
            .invert_range((82.5, 45.0), &BandScaleOptions::default())
            .unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "b");
        assert_eq!(result[1], "c");

        // Test out of range
        assert!(scale
            .invert_range((-10.0, -5.0), &BandScaleOptions::default())
            .is_none());
        assert!(scale
            .invert_range((130.0, 140.0), &BandScaleOptions::default())
            .is_none());

        // Test invalid range (NaN)
        assert!(scale
            .invert_range((f32::NAN, 50.0), &BandScaleOptions::default())
            .is_none());

        Ok(())
    }

    #[test]
    fn test_basic_band() -> Result<(), AvengerScaleError> {
        let scale: BandScale<String> =
            BandScale::try_new(vec!["A".into(), "B".into(), "C".into()])?;
        let values = vec!["A".into(), "B".into(), "C".into()];
        let result = scale
            .scale(&values, &BandScaleOptions::default())
            .as_vec(values.len(), None);

        let margin = F32Margin {
            epsilon: 0.0001,
            ..Default::default()
        };

        assert_approx_eq!(f32, result[0], 0.0, margin);
        assert_approx_eq!(f32, result[1], 0.333333, margin);
        assert_approx_eq!(f32, result[2], 0.666667, margin);
        Ok(())
    }

    #[test]
    fn test_band_position() -> Result<(), AvengerScaleError> {
        let scale: BandScale<String> =
            BandScale::try_new(vec!["A".into(), "B".into(), "C".into()])?;
        let values = vec!["A".into(), "B".into(), "C".into()];
        let result = scale
            .scale(
                &values,
                &BandScaleOptions {
                    band: Some(0.5),
                    ..Default::default()
                },
            )
            .as_vec(values.len(), None);

        let margin = F32Margin {
            epsilon: 0.0001,
            ..Default::default()
        };
        assert_approx_eq!(f32, result[0], 0.166667, margin);
        assert_approx_eq!(f32, result[1], 0.5, margin);
        assert_approx_eq!(f32, result[2], 0.833333, margin);
        Ok(())
    }

    #[test]
    fn test_band_position_offset() -> Result<(), AvengerScaleError> {
        let scale: BandScale<String> =
            BandScale::try_new(vec!["A".into(), "B".into(), "C".into()])?;
        let values = vec!["A".into(), "B".into(), "C".into()];
        let result = scale
            .scale(
                &values,
                &BandScaleOptions {
                    band: Some(0.5),
                    range_offset: Some(1.0),
                    ..Default::default()
                },
            )
            .as_vec(values.len(), None);

        let margin = F32Margin {
            epsilon: 0.0001,
            ..Default::default()
        };
        assert_approx_eq!(f32, result[0], 1.166667, margin);
        assert_approx_eq!(f32, result[1], 1.5, margin);
        assert_approx_eq!(f32, result[2], 1.833333, margin);
        Ok(())
    }
}
