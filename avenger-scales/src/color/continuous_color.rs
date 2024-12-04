use avenger_common::value::{ColorOrGradient, ScalarOrArray, ScalarOrArrayRef};
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use palette::{Hsla, IntoColor, Laba, Mix, Srgba};
use std::fmt::Debug;
use std::marker::PhantomData;

use crate::error::AvengerScaleError;
use crate::numeric::linear::LinearNumericScale;
use crate::numeric::log::LogNumericScale;
use crate::numeric::opts::NumericScaleOptions;
use crate::numeric::pow::PowNumericScale;
use crate::numeric::symlog::SymlogNumericScale;
use crate::numeric::ContinuousNumericScale;
use crate::temporal::date::DateScale;
use crate::temporal::timestamp::TimestampScale;
use crate::temporal::timestamptz::TimestampTzScale;

/// A trait for color spaces that can be used with the `NumericColorScale`
pub trait ColorSpace:
    Mix<Scalar = f32> + Copy + IntoColor<Srgba> + Debug + Send + Sync + 'static
{
}

impl<T: Mix<Scalar = f32> + Copy + IntoColor<Srgba> + Debug + Send + Sync + 'static> ColorSpace
    for T
{
}

#[derive(Clone, Debug)]
pub struct ContinuousColorScale<C, S, D>
where
    C: ColorSpace,
    S: ContinuousNumericScale<D>,
    D: 'static + Send + Sync + Clone,
{
    numeric_scale: S,
    range: Vec<C>,
    _marker: PhantomData<D>,
}

impl<C, S, D> ContinuousColorScale<C, S, D>
where
    C: ColorSpace,
    S: ContinuousNumericScale<D>,
    D: 'static + Send + Sync + Clone,
{
    pub fn from_scale(numeric_scale: S, colors: Vec<C>) -> Result<Self, AvengerScaleError> {
        if !numeric_scale.get_clamp() {
            return Err(AvengerScaleError::IncompatibleNumericScaleForColorRange(
                "Clamping must be enabled".to_string(),
            ));
        }
        let expected_range = (0.0, colors.len() as f32 - 1.0);
        if numeric_scale.get_range() != expected_range {
            return Err(AvengerScaleError::IncompatibleNumericScaleForColorRange(
                format!("Range must be ({}, {})", expected_range.0, expected_range.1),
            ));
        }

        Ok(Self {
            numeric_scale,
            range: colors,
            _marker: PhantomData,
        })
    }

    pub fn get_domain(&self) -> (D, D) {
        self.numeric_scale.get_domain()
    }

    pub fn get_range(&self) -> &[C] {
        &self.range
    }

    pub fn scale<'a>(
        &self,
        values: impl Into<ScalarOrArrayRef<'a, D>>,
    ) -> ScalarOrArray<ColorOrGradient> {
        // Normalize the input values to the range [0, number of colors - 1]
        let normalized_values = self
            .numeric_scale
            .scale(values, &NumericScaleOptions::default());

        normalized_values.map(|v| Self::interp_color_to_color_or_gradient(&self.range, *v))
    }

    pub fn ticks(&self, count: Option<f32>) -> Vec<D> {
        self.numeric_scale.ticks(count)
    }

    fn interp_color_to_color_or_gradient(colors: &[C], value: f32) -> ColorOrGradient {
        if !value.is_finite() {
            // Return transparent black if the value is not finite
            ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])
        } else {
            let lower = value.floor() as usize;
            let upper = value.ceil() as usize;
            let interp_factor = value - lower as f32;
            let mixed = colors[lower].mix(colors[upper], interp_factor);
            let c: Srgba = mixed.into_color();
            ColorOrGradient::Color([c.red, c.green, c.blue, c.alpha])
        }
    }
}

// Helper to create a linear color scale
impl<C: ColorSpace> ContinuousColorScale<C, LinearNumericScale, f32> {
    pub fn new_linear(
        domain: (f32, f32),
        colors: Vec<C>,
        nice: Option<usize>,
    ) -> ContinuousColorScale<C, LinearNumericScale, f32> {
        let mut numeric_scale = LinearNumericScale::new()
            .domain(domain)
            .clamp(true)
            .range((0.0, colors.len() as f32 - 1.0));

        if let Some(count) = nice {
            numeric_scale = numeric_scale.nice(Some(count));
        }

        Self {
            numeric_scale,
            range: colors,
            _marker: PhantomData,
        }
    }
}

impl<C: ColorSpace> ContinuousColorScale<C, LogNumericScale, f32> {
    pub fn new_log(domain: (f32, f32), colors: Vec<C>, base: Option<f32>, nice: bool) -> Self {
        let mut numeric_scale = LogNumericScale::new(base)
            .domain(domain)
            .clamp(true)
            .range((0.0, colors.len() as f32 - 1.0));

        if nice {
            numeric_scale = numeric_scale.nice();
        }

        Self {
            numeric_scale,
            range: colors,
            _marker: PhantomData,
        }
    }
}

impl<C: ColorSpace> ContinuousColorScale<C, PowNumericScale, f32> {
    pub fn new_pow(
        domain: (f32, f32),
        colors: Vec<C>,
        exponent: Option<f32>,
        nice: Option<usize>,
    ) -> Self {
        let mut numeric_scale = PowNumericScale::new()
            .exponent(exponent.unwrap_or(1.0))
            .domain(domain)
            .clamp(true)
            .range((0.0, colors.len() as f32 - 1.0));

        if let Some(count) = nice {
            numeric_scale = numeric_scale.nice(Some(count));
        }

        Self {
            numeric_scale,
            range: colors,
            _marker: PhantomData,
        }
    }
}

impl<C: ColorSpace> ContinuousColorScale<C, SymlogNumericScale, f32> {
    pub fn new_symlog(
        domain: (f32, f32),
        colors: Vec<C>,
        constant: Option<f32>,
        nice: Option<usize>,
    ) -> Self {
        let mut numeric_scale = SymlogNumericScale::new(constant)
            .domain(domain)
            .clamp(true)
            .range((0.0, colors.len() as f32 - 1.0));

        if let Some(count) = nice {
            numeric_scale = numeric_scale.nice(Some(count));
        }

        Self {
            numeric_scale,
            range: colors,
            _marker: PhantomData,
        }
    }
}

impl<C: ColorSpace> ContinuousColorScale<C, DateScale, NaiveDate> {
    pub fn new_date(domain: (NaiveDate, NaiveDate), colors: Vec<C>) -> Self {
        let numeric_scale = DateScale::new(domain)
            .clamp(true)
            .range((0.0, colors.len() as f32 - 1.0));

        Self {
            numeric_scale,
            range: colors,
            _marker: PhantomData,
        }
    }
}

impl<C: ColorSpace> ContinuousColorScale<C, TimestampScale, NaiveDateTime> {
    pub fn new_timestamp(domain: (NaiveDateTime, NaiveDateTime), colors: Vec<C>) -> Self {
        let numeric_scale = TimestampScale::new(domain)
            .clamp(true)
            .range((0.0, colors.len() as f32 - 1.0));

        Self {
            numeric_scale,
            range: colors,
            _marker: PhantomData,
        }
    }
}

impl<C: ColorSpace, Tz: TimeZone + Copy>
    ContinuousColorScale<C, TimestampTzScale<Tz>, DateTime<Utc>>
{
    pub fn new_timestamp_tz(
        domain: (DateTime<Utc>, DateTime<Utc>),
        colors: Vec<C>,
        display_tz: Tz,
    ) -> Self {
        let numeric_scale = TimestampTzScale::new(domain, display_tz)
            .clamp(true)
            .range((0.0, colors.len() as f32 - 1.0));

        Self {
            numeric_scale,
            range: colors,
            _marker: PhantomData,
        }
    }
}

// Predefine aliases
pub type LinearSrgbaScale = ContinuousColorScale<Srgba, LinearNumericScale, f32>;
pub type LogSrgbaScale = ContinuousColorScale<Srgba, LogNumericScale, f32>;
pub type PowSrgbaScale = ContinuousColorScale<Srgba, PowNumericScale, f32>;
pub type SymlogSrgbaScale = ContinuousColorScale<Srgba, SymlogNumericScale, f32>;

pub type LinearHslaScale = ContinuousColorScale<Hsla, LinearNumericScale, f32>;
pub type LogHslaScale = ContinuousColorScale<Hsla, LogNumericScale, f32>;
pub type PowHslaScale = ContinuousColorScale<Hsla, PowNumericScale, f32>;
pub type SymlogHslaScale = ContinuousColorScale<Hsla, SymlogNumericScale, f32>;

pub type LinearLabaScale = ContinuousColorScale<Laba, LinearNumericScale, f32>;
pub type LogLabaScale = ContinuousColorScale<Laba, LogNumericScale, f32>;
pub type PowLabaScale = ContinuousColorScale<Laba, PowNumericScale, f32>;
pub type SymlogLabaScale = ContinuousColorScale<Laba, SymlogNumericScale, f32>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AvengerScaleError;
    use float_cmp::assert_approx_eq;

    fn assert_srgba_approx_eq(actual: &ColorOrGradient, expected: Srgba) {
        let c = actual.color_or_transparent();
        let actual_srgba = Srgba::new(c[0], c[1], c[2], c[3]);

        assert_approx_eq!(f32, actual_srgba.red, expected.red);
        assert_approx_eq!(f32, actual_srgba.green, expected.green);
        assert_approx_eq!(f32, actual_srgba.blue, expected.blue);
        assert_approx_eq!(f32, actual_srgba.alpha, expected.alpha);
    }

    fn assert_hsla_approx_eq(actual: &ColorOrGradient, expected: Hsla) {
        let c = actual.color_or_transparent();
        let actual_srgba = Srgba::new(c[0], c[1], c[2], c[3]);
        let actual_hsla: Hsla = actual_srgba.into_color();

        assert_approx_eq!(
            f32,
            actual_hsla.hue.into_degrees(),
            expected.hue.into_degrees()
        );
        assert_approx_eq!(f32, actual_hsla.saturation, expected.saturation);
        assert_approx_eq!(f32, actual_hsla.lightness, expected.lightness);
    }

    // Basic scale configuration and defaults
    #[test]
    fn test_defaults() {
        let colors = vec![Srgba::new(0.0, 0.0, 0.0, 1.0)];
        let scale = ContinuousColorScale::new_linear((0.0, 1.0), colors.clone(), None);
        assert_eq!(scale.get_domain(), (0.0, 1.0));
        assert_eq!(scale.get_range(), colors);
    }

    // Core scaling functionality
    #[test]
    fn test_scale_srgb() -> Result<(), AvengerScaleError> {
        // Tests basic scaling with nulls and clamping
        let scale = ContinuousColorScale::new_linear(
            (10.0, 30.0),
            vec![
                Srgba::new(0.0, 0.0, 0.0, 1.0), // black
                Srgba::new(1.0, 0.0, 0.0, 1.0), // red
            ],
            None,
        );

        let values = vec![
            0.0,      // below domain
            f32::NAN, // null value
            10.0,     // domain start
            15.0,     // interpolated
            20.0,     // interpolated
            25.0,     // interpolated
            30.0,     // domain end
            40.0,     // above domain
        ];

        let result = scale.scale(&values).as_vec(values.len(), None);

        // Below domain - should clamp to black
        assert_srgba_approx_eq(&result[0], Srgba::new(0.0, 0.0, 0.0, 1.0));

        // Non-finite value - should be transparent black
        assert_srgba_approx_eq(&result[1], Srgba::new(0.0, 0.0, 0.0, 0.0));

        // Domain start - should be black
        assert_srgba_approx_eq(&result[2], Srgba::new(0.0, 0.0, 0.0, 1.0));

        // Interpolated values - should linearly increase in red
        assert_srgba_approx_eq(&result[3], Srgba::new(0.25, 0.0, 0.0, 1.0));
        assert_srgba_approx_eq(&result[4], Srgba::new(0.5, 0.0, 0.0, 1.0));
        assert_srgba_approx_eq(&result[5], Srgba::new(0.75, 0.0, 0.0, 1.0));

        // Domain end - should be red
        assert_srgba_approx_eq(&result[6], Srgba::new(1.0, 0.0, 0.0, 1.0));

        // Above domain - should clamp to red
        assert_srgba_approx_eq(&result[7], Srgba::new(1.0, 0.0, 0.0, 1.0));

        Ok(())
    }

    #[test]
    fn test_scale_hsl() -> Result<(), AvengerScaleError> {
        // Test HSL color interpolation with nulls and clamping
        let scale = ContinuousColorScale::new_linear(
            (10.0, 30.0),
            vec![
                Hsla::new(0.0, 0.5, 0.5, 1.0),  // red
                Hsla::new(60.0, 0.5, 0.5, 1.0), // yellow
            ],
            None,
        );

        let values = vec![
            0.0,      // below domain
            f32::NAN, // null value
            10.0,     // domain start
            15.0,     // interpolated
            20.0,     // interpolated
            25.0,     // interpolated
            30.0,     // domain end
            40.0,     // above domain
        ];

        let result = scale.scale(&values).as_vec(values.len(), None);

        // Below domain - should clamp to red
        assert_hsla_approx_eq(&result[0], Hsla::new(0.0, 0.5, 0.5, 1.0));

        // Non-finite value - should be transparent black
        assert_hsla_approx_eq(&result[1], Hsla::new(0.0, 0.0, 0.0, 0.0));

        // Domain start - should be red
        assert_hsla_approx_eq(&result[2], Hsla::new(0.0, 0.5, 0.5, 1.0));

        // Interpolated values - should show gradual transition from red to yellow
        assert_hsla_approx_eq(&result[3], Hsla::new(15.0, 0.5, 0.5, 1.0)); // 25% between red and yellow
        assert_hsla_approx_eq(&result[4], Hsla::new(30.0, 0.5, 0.5, 1.0)); // 50% between red and yellow
        assert_hsla_approx_eq(&result[5], Hsla::new(45.0, 0.5, 0.5, 1.0)); // 75% between red and yellow

        // Domain end - should be yellow
        assert_hsla_approx_eq(&result[6], Hsla::new(60.0, 0.5, 0.5, 1.0));

        // Above domain - should clamp to yellow
        assert_hsla_approx_eq(&result[7], Hsla::new(60.0, 0.5, 0.5, 1.0));

        Ok(())
    }

    #[test]
    fn test_timestamp_scale() {
        let start = DateTime::from_timestamp(0, 0).unwrap().naive_utc();
        let end = start + chrono::Duration::days(10);
        let mid = start + chrono::Duration::days(5);

        let colors = vec![
            Srgba::new(0.0, 0.0, 0.0, 1.0), // black
            Srgba::new(1.0, 0.0, 0.0, 1.0), // red
        ];

        let scale = ContinuousColorScale::new_timestamp((start, end), colors);

        let values = vec![
            start - chrono::Duration::days(1), // < domain
            start,                             // domain start
            mid,                               // middle
            end,                               // domain end
            end + chrono::Duration::days(1),   // > domain
        ];

        let result = scale.scale(&values).as_vec(values.len(), None);
        // Below domain - should clamp to black
        assert_srgba_approx_eq(&result[0], Srgba::new(0.0, 0.0, 0.0, 1.0));
        // Domain start - should be black
        assert_srgba_approx_eq(&result[1], Srgba::new(0.0, 0.0, 0.0, 1.0));
        // Middle - should be halfway between black and red
        assert_srgba_approx_eq(&result[2], Srgba::new(0.5, 0.0, 0.0, 1.0));
        // Domain end - should be red
        assert_srgba_approx_eq(&result[3], Srgba::new(1.0, 0.0, 0.0, 1.0));
        // Above domain - should clamp to red
        assert_srgba_approx_eq(&result[4], Srgba::new(1.0, 0.0, 0.0, 1.0));
    }
}
