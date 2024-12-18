use avenger_common::types::{ColorOrGradient, GradientStop};
use avenger_common::value::{ScalarOrArray, ScalarOrArrayRef};
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use core::num;
use palette::{Hsla, IntoColor, Laba, Mix, Srgba};
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::Arc;

use crate::error::AvengerScaleError;
use crate::numeric::linear::{LinearNumericScale, LinearNumericScaleConfig};
use crate::numeric::log::{LogNumericScale, LogNumericScaleConfig};
use crate::numeric::pow::{PowNumericScale, PowNumericScaleConfig};
use crate::numeric::symlog::{SymlogNumericScale, SymlogNumericScaleConfig};
use crate::numeric::ContinuousNumericScale;
use crate::temporal::date::{DateScale, DateScaleConfig};
use crate::temporal::timestamp::{TimestampScale, TimestampScaleConfig};
use crate::temporal::timestamptz::{TimestampTzScale, TimestampTzScaleConfig};

/// A trait for color spaces that can be used with the `NumericColorScale`
pub trait ColorSpace:
    Mix<Scalar = f32> + Copy + IntoColor<Srgba> + Debug + Send + Sync + 'static
{
}

impl<T: Mix<Scalar = f32> + Copy + IntoColor<Srgba> + Debug + Send + Sync + 'static> ColorSpace
    for T
{
}

#[derive(Clone)]
pub struct ContinuousColorScale<C, S, D>
where
    C: ColorSpace,
    S: ContinuousNumericScale<Domain = D>,
    D: 'static + Send + Sync + Clone,
{
    numeric_scale: S,
    numeric_scale_cloner: Arc<dyn Fn() -> S + Send + Sync + 'static>,
    range: Vec<C>,
    _marker: PhantomData<D>,
}

impl<C, S, D> ContinuousColorScale<C, S, D>
where
    C: ColorSpace,
    S: ContinuousNumericScale<Domain = D>,
    D: 'static + Send + Sync + Clone,
{
    pub fn from_scale(
        numeric_scale_cloner: Arc<dyn Fn() -> S + Send + Sync + 'static>,
        colors: Vec<C>,
    ) -> Result<Self, AvengerScaleError> {
        let mut numeric_scale = numeric_scale_cloner();
        numeric_scale.set_clamp(true);
        numeric_scale.set_range((0.0, colors.len() as f32 - 1.0));
        Ok(Self {
            numeric_scale,
            numeric_scale_cloner,
            range: colors,
            _marker: PhantomData,
        })
    }

    pub fn domain(&self) -> (S::Domain, S::Domain) {
        self.numeric_scale.domain()
    }

    pub fn range(&self) -> &[C] {
        &self.range
    }

    pub fn with_domain(self, domain: (S::Domain, S::Domain)) -> Self {
        let mut new_scale = self.clone_numeric_scale();
        new_scale.set_domain(domain);
        Self {
            numeric_scale: new_scale,
            ..self
        }
    }

    pub fn with_range(self, range: Vec<C>) -> Self {
        let mut new_scale = self.clone_numeric_scale();
        new_scale.set_range((0.0, range.len() as f32 - 1.0));
        Self {
            numeric_scale: new_scale,
            range,
            ..self
        }
    }

    pub fn get_numeric_scale(&self) -> &S {
        &self.numeric_scale
    }

    /// Clone the numeric scale by calling the builder function and
    /// configuring it with the same settings as the original scale
    pub fn clone_numeric_scale(&self) -> S {
        let mut new_scale = (self.numeric_scale_cloner)();
        new_scale.set_clamp(self.numeric_scale.clamp());
        new_scale.set_range(self.numeric_scale.range());
        new_scale.set_domain(self.numeric_scale.domain());
        new_scale
    }

    pub fn scale(&self, values: &[S::Domain]) -> ScalarOrArray<ColorOrGradient> {
        // Normalize the input values to the range [0, number of colors - 1]
        let normalized_values = self.numeric_scale.scale(values);

        normalized_values.map(|v| Self::interp_color_to_color_or_gradient(&self.range, *v))
    }

    pub fn ticks(&self, count: Option<f32>) -> Vec<S::Domain> {
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
        config: &LinearNumericScaleConfig,
        colors: Vec<C>,
    ) -> ContinuousColorScale<C, LinearNumericScale, f32> {
        let config = config.clone();
        let num_colors = colors.len();
        let numeric_scale_builder = Arc::new(move || {
            LinearNumericScale::new(&config)
                .with_clamp(true)
                .with_range((0.0, num_colors as f32 - 1.0))
        });

        let numeric_scale = numeric_scale_builder();

        Self {
            numeric_scale,
            numeric_scale_cloner: numeric_scale_builder,
            range: colors,
            _marker: PhantomData,
        }
    }
}

impl<C: ColorSpace> ContinuousColorScale<C, LogNumericScale, f32> {
    pub fn new_log(
        config: &LogNumericScaleConfig,
        colors: Vec<C>,
    ) -> ContinuousColorScale<C, LogNumericScale, f32> {
        let config = config.clone();
        let num_colors = colors.len();
        let numeric_scale_builder = Arc::new(move || {
            LogNumericScale::new(&config)
                .with_clamp(true)
                .with_range((0.0, num_colors as f32 - 1.0))
        });

        let numeric_scale = numeric_scale_builder();

        Self {
            numeric_scale,
            numeric_scale_cloner: numeric_scale_builder,
            range: colors,
            _marker: PhantomData,
        }
    }
}

impl<C: ColorSpace> ContinuousColorScale<C, PowNumericScale, f32> {
    pub fn new_pow(
        config: &PowNumericScaleConfig,
        colors: Vec<C>,
    ) -> ContinuousColorScale<C, PowNumericScale, f32> {
        let config = config.clone();
        let num_colors = colors.len();
        let numeric_scale_builder = Arc::new(move || {
            PowNumericScale::new(&config)
                .with_clamp(true)
                .with_range((0.0, num_colors as f32 - 1.0))
        });

        let numeric_scale = numeric_scale_builder();

        Self {
            numeric_scale,
            numeric_scale_cloner: numeric_scale_builder,
            range: colors,
            _marker: PhantomData,
        }
    }
}

impl<C: ColorSpace> ContinuousColorScale<C, SymlogNumericScale, f32> {
    pub fn new_symlog(
        config: &SymlogNumericScaleConfig,
        colors: Vec<C>,
    ) -> ContinuousColorScale<C, SymlogNumericScale, f32> {
        let config = config.clone();
        let num_colors = colors.len();
        let numeric_scale_builder = Arc::new(move || {
            SymlogNumericScale::new(&config)
                .with_clamp(true)
                .with_range((0.0, num_colors as f32 - 1.0))
        });

        let numeric_scale = numeric_scale_builder();

        Self {
            numeric_scale,
            numeric_scale_cloner: numeric_scale_builder,
            range: colors,
            _marker: PhantomData,
        }
    }
}

impl<C: ColorSpace> ContinuousColorScale<C, DateScale, NaiveDate> {
    pub fn new_date(config: &DateScaleConfig, colors: Vec<C>) -> Self {
        let config = config.clone();
        let num_colors = colors.len();
        let numeric_scale_builder = Arc::new(move || {
            DateScale::new(&config)
                .with_clamp(true)
                .with_range((0.0, num_colors as f32 - 1.0))
        });

        let numeric_scale = numeric_scale_builder();

        Self {
            numeric_scale,
            numeric_scale_cloner: numeric_scale_builder,
            range: colors,
            _marker: PhantomData,
        }
    }
}

impl<C: ColorSpace> ContinuousColorScale<C, TimestampScale, NaiveDateTime> {
    pub fn new_timestamp(config: &TimestampScaleConfig, colors: Vec<C>) -> Self {
        let config = config.clone();
        let num_colors = colors.len();
        let numeric_scale_builder = Arc::new(move || {
            TimestampScale::new(&config)
                .with_clamp(true)
                .with_range((0.0, num_colors as f32 - 1.0))
        });

        let numeric_scale = numeric_scale_builder();

        Self {
            numeric_scale,
            numeric_scale_cloner: numeric_scale_builder,
            range: colors,
            _marker: PhantomData,
        }
    }
}

impl<C: ColorSpace> ContinuousColorScale<C, TimestampTzScale, DateTime<Utc>> {
    pub fn new_timestamp_tz(
        config: &TimestampTzScaleConfig,
        colors: Vec<C>,
        display_tz: Tz,
    ) -> Self {
        let config = config.clone();
        let num_colors = colors.len();
        let numeric_scale_builder = Arc::new(move || {
            TimestampTzScale::new(&config, display_tz)
                .with_clamp(true)
                .with_range((0.0, num_colors as f32 - 1.0))
        });

        let numeric_scale = numeric_scale_builder();

        Self {
            numeric_scale,
            numeric_scale_cloner: numeric_scale_builder,
            range: colors,
            _marker: PhantomData,
        }
    }
}

impl<C, S> ContinuousColorScale<C, S, f32>
where
    C: ColorSpace,
    S: ContinuousNumericScale<Domain = f32>,
{
    pub fn to_gradient_stops(&self) -> Vec<GradientStop> {
        let domain_start: f32 = self.domain().0.into();
        let domain_end: f32 = self.domain().1.into();

        let num_segments = 10;
        let step = (domain_end - domain_start) / num_segments as f32;
        let fractions: Vec<f32> = (0..=num_segments)
            .map(|i| i as f32 / num_segments as f32)
            .collect();
        let domain_values: Vec<f32> = (0..=num_segments)
            .map(|i| domain_start + i as f32 * step)
            .collect();
        let colors = self.scale(&domain_values).as_vec(domain_values.len(), None);

        fractions
            .iter()
            .zip(colors)
            .map(|(f, c)| GradientStop {
                offset: *f,
                color: c.color_or_transparent(),
            })
            .collect()
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
        let scale = ContinuousColorScale::new_linear(
            &LinearNumericScaleConfig {
                domain: (0.0, 1.0),
                ..Default::default()
            },
            colors.clone(),
        );
        assert_eq!(scale.domain(), (0.0, 1.0));
        assert_eq!(scale.range(), colors);
    }

    // Core scaling functionality
    #[test]
    fn test_scale_srgb() -> Result<(), AvengerScaleError> {
        // Tests basic scaling with nulls and clamping
        let scale = ContinuousColorScale::new_linear(
            &LinearNumericScaleConfig {
                domain: (10.0, 30.0),
                ..Default::default()
            },
            vec![
                Srgba::new(0.0, 0.0, 0.0, 1.0), // black
                Srgba::new(1.0, 0.0, 0.0, 1.0), // red
            ],
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
            &LinearNumericScaleConfig {
                domain: (10.0, 30.0),
                ..Default::default()
            },
            vec![
                Hsla::new(0.0, 0.5, 0.5, 1.0),  // red
                Hsla::new(60.0, 0.5, 0.5, 1.0), // yellow
            ],
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

        let scale = ContinuousColorScale::new_timestamp(
            &TimestampScaleConfig {
                domain: (start, end),
                ..Default::default()
            },
            colors,
        );

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
