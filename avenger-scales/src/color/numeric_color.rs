use avenger_common::value::{ColorOrGradient, ScalarOrArray, ScalarOrArrayRef};
use palette::{Hsla, IntoColor, Laba, Mix, Srgba};
use std::fmt::Debug;

use crate::numeric::linear::LinearNumericScale;
use crate::numeric::log::LogNumericScale;
use crate::numeric::opts::NumericScaleOptions;
use crate::numeric::pow::PowNumericScale;
use crate::numeric::symlog::SymlogNumericScale;
use crate::numeric::NumericScale;

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
pub struct NumericColorScale<C: ColorSpace> {
    numeric_scale: NumericScale,
    range: Vec<C>,
}

impl<C: ColorSpace> NumericColorScale<C> {
    pub fn new(numeric_scale: impl Into<NumericScale>, colors: Vec<C>) -> Self {
        let numeric_scale = numeric_scale
            .into()
            .clamp(true)
            .range((0.0, colors.len() as f32 - 1.0));

        Self {
            numeric_scale,
            range: colors,
        }
    }

    pub fn new_linear(colors: Vec<C>) -> Self {
        Self::new(LinearNumericScale::new(), colors)
    }

    pub fn new_log(colors: Vec<C>, base: Option<f32>) -> Self {
        Self::new(LogNumericScale::new(base), colors)
    }

    pub fn new_pow(colors: Vec<C>, exponent: Option<f32>) -> Self {
        let mut scale = PowNumericScale::new();
        if let Some(exp) = exponent {
            scale = scale.exponent(exp);
        }
        Self::new(scale, colors)
    }

    pub fn new_symlog(colors: Vec<C>, constant: Option<f32>) -> Self {
        Self::new(SymlogNumericScale::new(constant), colors)
    }

    pub fn nice(mut self, count: usize) -> Self {
        self.numeric_scale = self.numeric_scale.nice(Some(count));
        self
    }

    pub fn domain(mut self, (start, end): (f32, f32)) -> Self {
        self.numeric_scale = self.numeric_scale.domain((start, end));
        self
    }

    pub fn get_domain(&self) -> (f32, f32) {
        self.numeric_scale.get_domain()
    }

    pub fn range(mut self, colors: Vec<C>) -> Self {
        // Update numeric scale range to produce values from 0 to the number of colors - 1
        self.numeric_scale = self.numeric_scale.range((0.0, colors.len() as f32 - 1.0));
        self.range = colors;
        self
    }

    pub fn get_range(&self) -> &[C] {
        &self.range
    }

    pub fn scale<'a>(
        &self,
        values: impl Into<ScalarOrArrayRef<'a, f32>>,
    ) -> ScalarOrArray<ColorOrGradient> {
        // Normalize the input values to the range [0, number of colors - 1]
        let normalized_values = self
            .numeric_scale
            .scale(values, &NumericScaleOptions::default());

        normalized_values.map(|v| Self::interp_color_to_color_or_gradient(&self.range, *v))
    }

    pub fn ticks(&self, count: Option<f32>) -> Vec<f32> {
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

pub type NumericSrgbaScale = NumericColorScale<Srgba>;
pub type NumericHslaScale = NumericColorScale<Hsla>;
pub type NumericLabaScale = NumericColorScale<Laba>;

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
        let scale = NumericSrgbaScale::new_linear(colors.clone());
        assert_eq!(scale.get_domain(), (0.0, 1.0));
        assert_eq!(scale.get_range(), colors);
    }

    // Core scaling functionality
    #[test]
    fn test_scale_srgb() -> Result<(), AvengerScaleError> {
        // Tests basic scaling with nulls and clamping
        let scale = NumericSrgbaScale::new_linear(vec![
            Srgba::new(0.0, 0.0, 0.0, 1.0), // black
            Srgba::new(1.0, 0.0, 0.0, 1.0), // red
        ])
        .domain((10.0, 30.0));

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
        let scale = NumericHslaScale::new_linear(vec![
            Hsla::new(0.0, 0.5, 0.5, 1.0),  // red
            Hsla::new(60.0, 0.5, 0.5, 1.0), // yellow
        ])
        .domain((10.0, 30.0));

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
}
