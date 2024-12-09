use crate::band::{BandScale, BandScaleConfig};
use crate::error::AvengerScaleError;
use avenger_common::value::{ScalarOrArray, ScalarOrArrayRef};
use std::fmt::Debug;
use std::hash::Hash;

#[derive(Debug, Clone)]
pub struct PointScaleConfig {
    pub range: (f32, f32),
    pub align: f32,
    pub padding: f32,
    pub round: bool,
    pub range_offset: f32,
}

impl Default for PointScaleConfig {
    fn default() -> Self {
        Self {
            range: (0.0, 1.0),
            align: 0.5,
            padding: 0.0,
            round: false,
            range_offset: 0.0,
        }
    }
}

impl From<&PointScaleConfig> for BandScaleConfig {
    fn from(config: &PointScaleConfig) -> Self {
        Self {
            range: config.range,
            align: config.align,
            round: config.round,
            range_offset: config.range_offset,
            padding_inner: 1.0,
            padding_outer: config.padding,
            band: 0.0,
        }
    }
}

/// A point scale is a special case of a band scale with padding=1.
/// It places points at uniformly spaced positions along a range.
#[derive(Debug, Clone)]
pub struct PointScale<D: Debug + Clone + Hash + Eq + Sync + 'static> {
    band_scale: BandScale<D>,
}

impl<D: Debug + Clone + Hash + Eq + Sync + 'static> PointScale<D> {
    /// Creates a new point scale with the given domain.
    ///
    /// # Defaults
    /// - range: (0.0, 1.0)
    /// - align: 0.5
    /// - padding: 0.0
    /// - round: false
    pub fn try_new(domain: Vec<D>, config: &PointScaleConfig) -> Result<Self, AvengerScaleError> {
        Ok(Self {
            band_scale: BandScale::try_new(domain, &config.into())?,
        })
    }

    /// Sets the range offset to a value between 0 and 1.
    pub fn with_range_offset(mut self, range_offset: f32) -> Result<Self, AvengerScaleError> {
        self.band_scale = self.band_scale.with_range_offset(range_offset)?;
        Ok(self)
    }

    /// Sets the output range as (min, max).
    pub fn with_range(mut self, range: (f32, f32)) -> Result<Self, AvengerScaleError> {
        self.band_scale = self.band_scale.with_range(range)?;
        Ok(self)
    }

    /// Sets the domain of the scale.
    pub fn with_domain(mut self, domain: Vec<D>) -> Result<Self, AvengerScaleError> {
        self.band_scale = self.band_scale.with_domain(domain)?;
        Ok(self)
    }

    /// Sets the alignment of points within the range to a value between 0 and 1.
    pub fn with_align(mut self, align: f32) -> Result<Self, AvengerScaleError> {
        self.band_scale = self.band_scale.with_align(align)?;
        Ok(self)
    }

    /// Enables or disables rounding of the point positions.
    pub fn with_round(mut self, round: bool) -> Result<Self, AvengerScaleError> {
        self.band_scale = self.band_scale.with_round(round)?;
        Ok(self)
    }

    /// Sets the padding between points to a value between 0 and 1.
    pub fn with_padding(mut self, padding: f32) -> Result<Self, AvengerScaleError> {
        self.band_scale = self.band_scale.with_padding_outer(padding)?;
        Ok(self)
    }

    /// Returns a reference to the scale's domain.
    pub fn domain(&self) -> &Vec<D> {
        self.band_scale.domain()
    }

    /// Returns the scale's range as a tuple of (start, end).
    pub fn range(&self) -> (f32, f32) {
        self.band_scale.range()
    }

    /// Returns the alignment value.
    pub fn align(&self) -> f32 {
        self.band_scale.align()
    }

    /// Returns whether rounding is enabled.
    pub fn round(&self) -> bool {
        self.band_scale.round()
    }

    /// Returns the step size between adjacent points.
    pub fn step(&self) -> f32 {
        self.band_scale.step()
    }

    /// Returns the padding between points.
    pub fn padding(&self) -> f32 {
        self.band_scale.padding_outer()
    }

    /// Maps input values to their corresponding point positions.
    pub fn scale<'a>(&self, values: impl Into<ScalarOrArrayRef<'a, D>>) -> ScalarOrArray<f32> {
        self.band_scale.scale(values)
    }

    /// Maps a range value back to the corresponding domain values.
    pub fn invert_range(&self, range_values: (f32, f32)) -> Option<Vec<D>> {
        self.band_scale.invert_range(range_values)
    }

    /// Maps a single range value back to the corresponding domain value.
    pub fn invert(&self, value: f32) -> Option<D> {
        self.band_scale.invert(value)
    }

    /// Returns the underlying band scale.
    pub fn to_band(self) -> BandScale<D> {
        self.band_scale
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use float_cmp::assert_approx_eq;

    #[test]
    fn test_point_scale_basic() -> Result<(), AvengerScaleError> {
        let domain = vec!["a", "b", "c"];
        let scale = PointScale::try_new(domain.clone(), &PointScaleConfig::default())?;

        let values = vec!["a", "b", "b", "c", "f"];
        let result = scale.scale(&values).as_vec(values.len(), None);

        // With 3 points in [0,1], expect points at 0.0, 0.5, 1.0
        assert_approx_eq!(f32, result[0], 0.0); // "a"
        assert_approx_eq!(f32, result[1], 0.5); // "b"
        assert_approx_eq!(f32, result[2], 0.5); // "b"
        assert_approx_eq!(f32, result[3], 1.0); // "c"
        assert!(result[4].is_nan()); // "f"
        assert_approx_eq!(f32, scale.step(), 0.5);

        Ok(())
    }

    #[test]
    fn test_point_scale_custom_range() -> Result<(), AvengerScaleError> {
        let domain = vec!["a", "b", "c"];
        let scale = PointScale::try_new(domain.clone(), &PointScaleConfig::default())?
            .with_range((0.0, 100.0))?;

        let values = vec!["a", "b", "b", "c", "f"];
        let result = scale.scale(&values).as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 0.0); // "a"
        assert_approx_eq!(f32, result[1], 50.0); // "b"
        assert_approx_eq!(f32, result[2], 50.0); // "b"
        assert_approx_eq!(f32, result[3], 100.0); // "c"
        assert!(result[4].is_nan()); // "f"
        Ok(())
    }

    #[test]
    fn test_point_scale_with_padding() -> Result<(), AvengerScaleError> {
        let domain = vec!["a", "b", "c"];
        let scale = PointScale::try_new(domain.clone(), &PointScaleConfig::default())?
            .with_range((0.0, 100.0))?
            .with_padding(0.5)?;

        let values = vec!["a", "b", "b", "c", "f"];
        let result = scale.scale(&values).as_vec(values.len(), None);

        // With padding of 0.5, points should be at specific positions
        assert_approx_eq!(f32, result[0], 16.666667); // "a"
        assert_approx_eq!(f32, result[1], 50.0); // "b"
        assert_approx_eq!(f32, result[2], 50.0); // "b"
        assert_approx_eq!(f32, result[3], 83.333333); // "c"
        assert!(result[4].is_nan()); // "f"

        Ok(())
    }

    #[test]
    fn test_point_scale_round() -> Result<(), AvengerScaleError> {
        let domain = vec!["a", "b", "c", "d"];
        let scale = PointScale::try_new(domain.clone(), &PointScaleConfig::default())?
            .with_range((0.0, 100.0))?
            .with_round(true)?;

        let values = vec!["a", "b", "b", "c", "d", "f"];
        let result = scale.scale(&values).as_vec(values.len(), None);

        // With 4 points in [0,100] and rounding, expect points at 0, 34, 67, 100
        // Confirmed with d3-scale
        assert_approx_eq!(f32, result[0], 1.0); // "a"
        assert_approx_eq!(f32, result[1], 34.0); // "b"
        assert_approx_eq!(f32, result[2], 34.0); // "b"
        assert_approx_eq!(f32, result[3], 67.0); // "c"
        assert_approx_eq!(f32, result[4], 100.0); // "d"
        assert!(result[5].is_nan()); // "f"
        Ok(())
    }

    #[test]
    fn test_point_scale_range_offset() -> Result<(), AvengerScaleError> {
        let domain = vec!["a", "b", "c", "d"];
        let scale = PointScale::try_new(domain.clone(), &PointScaleConfig::default())?
            .with_range((0.0, 100.0))?
            .with_range_offset(1.0)?
            .with_round(true)?;

        let values = vec!["a", "b", "b", "c", "d", "f"];
        let result = scale.scale(&values).as_vec(values.len(), None);

        // With 4 points in [0,100] and rounding, expect points at 0, 34, 67, 100
        // Confirmed with d3-scale
        assert_approx_eq!(f32, result[0], 2.0); // "a"
        assert_approx_eq!(f32, result[1], 35.0); // "b"
        assert_approx_eq!(f32, result[2], 35.0); // "b"
        assert_approx_eq!(f32, result[3], 68.0); // "c"
        assert_approx_eq!(f32, result[4], 101.0); // "d"
        assert!(result[5].is_nan()); // "f"
        Ok(())
    }

    #[test]
    fn test_point_scale_invert() -> Result<(), AvengerScaleError> {
        let domain = vec!["a", "b", "c"];
        let scale = PointScale::try_new(
            domain.clone(),
            &PointScaleConfig {
                range: (0.0, 100.0),
                ..Default::default()
            },
        )?;

        // Test exact point positions
        let result = scale.invert(0.0).unwrap();
        assert_eq!(result, "a");

        let result = scale.invert(50.0).unwrap();
        assert_eq!(result, "b");

        let result = scale.invert(100.0).unwrap();
        assert_eq!(result, "c");

        // Test positions between points (should return None)
        assert!(scale.invert(25.0).is_none());
        assert!(scale.invert(75.0).is_none());

        // Test out of range
        assert!(scale.invert(-10.0).is_none());
        assert!(scale.invert(110.0).is_none());

        Ok(())
    }

    #[test]
    fn test_point_scale_invert_range() -> Result<(), AvengerScaleError> {
        let domain = vec!["a", "b", "c"];
        let scale = PointScale::try_new(
            domain.clone(),
            &PointScaleConfig {
                range: (0.0, 100.0),
                ..Default::default()
            },
        )?;

        // Test range covering multiple points
        let result = scale.invert_range((0.0, 100.0)).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "a");
        assert_eq!(result[1], "b");
        assert_eq!(result[2], "c");

        // Test partial range
        let result = scale.invert_range((50.0, 100.0)).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "b");
        assert_eq!(result[1], "c");

        // Test reversed range
        let result = scale.invert_range((100.0, 50.0)).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "b");
        assert_eq!(result[1], "c");

        Ok(())
    }
}
