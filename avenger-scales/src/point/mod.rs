pub mod opts;
use opts::PointScaleOptions;

use crate::band::opts::BandScaleOptions;
use crate::band::BandScale;
use crate::error::AvengerScaleError;
use std::fmt::Debug;
use std::hash::Hash;

/// A point scale is a special case of a band scale with padding=1.
/// It places points at uniformly spaced positions along a range.
#[derive(Debug, Clone)]
pub struct PointScale<D: Debug + Clone + Hash + Eq> {
    band_scale: BandScale<D>,
}

impl<D: Debug + Clone + Hash + Eq> PointScale<D> {
    /// Creates a new point scale with the given domain.
    ///
    /// # Defaults
    /// - range: (0.0, 1.0)
    /// - align: 0.5
    /// - padding: 0.0
    /// - round: false
    pub fn try_new(domain: Vec<D>) -> Result<Self, AvengerScaleError> {
        Ok(Self {
            band_scale: BandScale::try_new(domain)?
                .padding_inner(1.0)?
                .padding_outer(0.0)?,
        })
    }

    /// Sets the output range as (min, max).
    pub fn range(mut self, range: (f32, f32)) -> Result<Self, AvengerScaleError> {
        self.band_scale = self.band_scale.range(range)?;
        Ok(self)
    }

    /// Sets the alignment of points within the range to a value between 0 and 1.
    pub fn align(mut self, align: f32) -> Result<Self, AvengerScaleError> {
        self.band_scale = self.band_scale.align(align)?;
        Ok(self)
    }

    /// Sets the padding between points to a value between 0 and 1.
    pub fn padding(mut self, padding: f32) -> Result<Self, AvengerScaleError> {
        self.band_scale = self.band_scale.padding_outer(padding)?;
        Ok(self)
    }

    /// Enables or disables rounding of the point positions.
    pub fn round(mut self, round: bool) -> Result<Self, AvengerScaleError> {
        self.band_scale = self.band_scale.round(round)?;
        Ok(self)
    }

    /// Returns a reference to the scale's domain.
    pub fn get_domain(&self) -> &Vec<D> {
        self.band_scale.get_domain()
    }

    /// Returns the scale's range as a tuple of (start, end).
    pub fn get_range(&self) -> (f32, f32) {
        self.band_scale.get_range()
    }

    /// Returns the alignment value.
    pub fn get_align(&self) -> f32 {
        self.band_scale.get_align()
    }

    /// Returns whether rounding is enabled.
    pub fn get_round(&self) -> bool {
        self.band_scale.get_round()
    }

    /// Returns the step size between adjacent points.
    pub fn step(&self) -> f32 {
        self.band_scale.step()
    }

    /// Maps input values to their corresponding point positions.
    pub fn scale(
        &self,
        values: &Vec<D>,
        opts: &PointScaleOptions,
    ) -> Result<Vec<Option<f32>>, AvengerScaleError> {
        self.band_scale.scale(values, &BandScaleOptions::from(opts))
    }

    /// Maps a range value back to the corresponding domain values.
    pub fn invert_range(
        &self,
        range_values: (f32, f32),
        opts: &PointScaleOptions,
    ) -> Option<Vec<D>> {
        self.band_scale
            .invert_range(range_values, &BandScaleOptions::from(opts))
    }

    /// Maps a single range value back to the corresponding domain value.
    pub fn invert(&self, value: f32, opts: &PointScaleOptions) -> Option<D> {
        self.band_scale.invert(value, &BandScaleOptions::from(opts))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use float_cmp::assert_approx_eq;

    #[test]
    fn test_point_scale_basic() -> Result<(), AvengerScaleError> {
        let domain = vec!["a", "b", "c"];
        let scale = PointScale::try_new(domain.clone())?;

        let values = vec!["a", "b", "b", "c", "f"];
        let result = scale.scale(&values, &PointScaleOptions::default())?;

        // With 3 points in [0,1], expect points at 0.0, 0.5, 1.0
        assert_approx_eq!(f32, result[0].unwrap(), 0.0); // "a"
        assert_approx_eq!(f32, result[1].unwrap(), 0.5); // "b"
        assert_approx_eq!(f32, result[2].unwrap(), 0.5); // "b"
        assert_approx_eq!(f32, result[3].unwrap(), 1.0); // "c"
        assert!(result[4].is_none()); // "f"
        assert_approx_eq!(f32, scale.step(), 0.5);

        Ok(())
    }

    #[test]
    fn test_point_scale_custom_range() -> Result<(), AvengerScaleError> {
        let domain = vec!["a", "b", "c"];
        let scale = PointScale::try_new(domain.clone())?.range((0.0, 100.0))?;

        let values = vec!["a", "b", "b", "c", "f"];
        let result = scale.scale(&values, &PointScaleOptions::default())?;

        assert_approx_eq!(f32, result[0].unwrap(), 0.0); // "a"
        assert_approx_eq!(f32, result[1].unwrap(), 50.0); // "b"
        assert_approx_eq!(f32, result[2].unwrap(), 50.0); // "b"
        assert_approx_eq!(f32, result[3].unwrap(), 100.0); // "c"
        assert!(result[4].is_none()); // "f"
        Ok(())
    }

    #[test]
    fn test_point_scale_with_padding() -> Result<(), AvengerScaleError> {
        let domain = vec!["a", "b", "c"];
        let scale = PointScale::try_new(domain.clone())?
            .range((0.0, 100.0))?
            .padding(0.5)?;

        let values = vec!["a", "b", "b", "c", "f"];
        let result = scale.scale(&values, &PointScaleOptions::default())?;

        // With padding of 0.5, points should be at specific positions
        assert_approx_eq!(f32, result[0].unwrap(), 16.666667); // "a"
        assert_approx_eq!(f32, result[1].unwrap(), 50.0); // "b"
        assert_approx_eq!(f32, result[2].unwrap(), 50.0); // "b"
        assert_approx_eq!(f32, result[3].unwrap(), 83.333333); // "c"
        assert!(result[4].is_none()); // "f"

        Ok(())
    }

    #[test]
    fn test_point_scale_round() -> Result<(), AvengerScaleError> {
        let domain = vec!["a", "b", "c", "d"];
        let scale = PointScale::try_new(domain.clone())?
            .range((0.0, 100.0))?
            .round(true)?;

        let values = vec!["a", "b", "b", "c", "d", "f"];
        let result = scale.scale(&values, &PointScaleOptions::default())?;

        // With 4 points in [0,100] and rounding, expect points at 0, 34, 67, 100
        // Confirmed with d3-scale
        assert_approx_eq!(f32, result[0].unwrap(), 1.0); // "a"
        assert_approx_eq!(f32, result[1].unwrap(), 34.0); // "b"
        assert_approx_eq!(f32, result[2].unwrap(), 34.0); // "b"
        assert_approx_eq!(f32, result[3].unwrap(), 67.0); // "c"
        assert_approx_eq!(f32, result[4].unwrap(), 100.0); // "d"
        assert!(result[5].is_none()); // "f"
        Ok(())
    }

    #[test]
    fn test_point_scale_range_offset() -> Result<(), AvengerScaleError> {
        let domain = vec!["a", "b", "c", "d"];
        let scale = PointScale::try_new(domain.clone())?
            .range((0.0, 100.0))?
            .round(true)?;

        let values = vec!["a", "b", "b", "c", "d", "f"];
        let result = scale.scale(
            &values,
            &PointScaleOptions {
                range_offset: Some(1.0),
            },
        )?;

        // With 4 points in [0,100] and rounding, expect points at 0, 34, 67, 100
        // Confirmed with d3-scale
        assert_approx_eq!(f32, result[0].unwrap(), 2.0); // "a"
        assert_approx_eq!(f32, result[1].unwrap(), 35.0); // "b"
        assert_approx_eq!(f32, result[2].unwrap(), 35.0); // "b"
        assert_approx_eq!(f32, result[3].unwrap(), 68.0); // "c"
        assert_approx_eq!(f32, result[4].unwrap(), 101.0); // "d"
        assert!(result[5].is_none()); // "f"
        Ok(())
    }

    #[test]
    fn test_point_scale_invert() -> Result<(), AvengerScaleError> {
        let domain = vec!["a", "b", "c"];
        let scale = PointScale::try_new(domain.clone())?.range((0.0, 100.0))?;

        // Test exact point positions
        let result = scale.invert(0.0, &PointScaleOptions::default()).unwrap();
        assert_eq!(result, "a");

        let result = scale.invert(50.0, &PointScaleOptions::default()).unwrap();
        assert_eq!(result, "b");

        let result = scale.invert(100.0, &PointScaleOptions::default()).unwrap();
        assert_eq!(result, "c");

        // Test positions between points (should return None)
        assert!(scale.invert(25.0, &PointScaleOptions::default()).is_none());
        assert!(scale.invert(75.0, &PointScaleOptions::default()).is_none());

        // Test out of range
        assert!(scale.invert(-10.0, &PointScaleOptions::default()).is_none());
        assert!(scale.invert(110.0, &PointScaleOptions::default()).is_none());

        Ok(())
    }

    #[test]
    fn test_point_scale_invert_range() -> Result<(), AvengerScaleError> {
        let domain = vec!["a", "b", "c"];
        let scale = PointScale::try_new(domain.clone())?.range((0.0, 100.0))?;

        // Test range covering multiple points
        let result = scale
            .invert_range((0.0, 100.0), &PointScaleOptions::default())
            .unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "a");
        assert_eq!(result[1], "b");
        assert_eq!(result[2], "c");

        // Test partial range
        let result = scale
            .invert_range((50.0, 100.0), &PointScaleOptions::default())
            .unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "b");
        assert_eq!(result[1], "c");

        // Test reversed range
        let result = scale
            .invert_range((100.0, 50.0), &PointScaleOptions::default())
            .unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "b");
        assert_eq!(result[1], "c");

        Ok(())
    }
}
