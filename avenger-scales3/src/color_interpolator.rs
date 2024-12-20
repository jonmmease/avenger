use crate::{
    error::AvengerScaleError,
    scales::{ArrowScale, ScaleConfig},
};
use arrow::array::{ArrayRef, Float32Array};
use avenger_common::{
    types::ColorOrGradient,
    value::{ScalarOrArray, ScalarOrArrayValue},
};
use palette::{Hsla, IntoColor, Laba, Mix, Srgba};
use std::{fmt::Debug, sync::Arc};

pub struct ColorInterpolatorConfig {
    pub colors: Vec<[f32; 4]>,
}

pub trait ColorInterpolator: Send + Sync + 'static {
    /// Interpolate over evenly spaced colors based on normalized values
    fn interpolate(
        &self,
        config: &ColorInterpolatorConfig,
        values: &[f32],
    ) -> Result<ScalarOrArray<ColorOrGradient>, AvengerScaleError>;
}

#[derive(Clone)]
pub struct SrgbaColorInterpolator;

impl ColorInterpolator for SrgbaColorInterpolator {
    fn interpolate(
        &self,
        config: &ColorInterpolatorConfig,
        values: &[f32],
    ) -> Result<ScalarOrArray<ColorOrGradient>, AvengerScaleError> {
        // Interpolate in Srgba space
        let srgba_colors: Vec<Srgba> = config
            .colors
            .iter()
            .map(|c| Srgba::from_components((c[0], c[1], c[2], c[3])))
            .collect();
        Ok(interpolate_color(&srgba_colors, values))
    }
}

#[derive(Clone)]
pub struct HslaColorInterpolator;

impl ColorInterpolator for HslaColorInterpolator {
    fn interpolate(
        &self,
        config: &ColorInterpolatorConfig,
        values: &[f32],
    ) -> Result<ScalarOrArray<ColorOrGradient>, AvengerScaleError> {
        let hsla_colors: Vec<Hsla> = config
            .colors
            .iter()
            .map(|c| Srgba::from_components((c[0], c[1], c[2], c[3])).into_color())
            .collect();
        Ok(interpolate_color(&hsla_colors, values))
    }
}

#[derive(Clone)]
pub struct LabaColorInterpolator;

impl ColorInterpolator for LabaColorInterpolator {
    fn interpolate(
        &self,
        config: &ColorInterpolatorConfig,
        values: &[f32],
    ) -> Result<ScalarOrArray<ColorOrGradient>, AvengerScaleError> {
        let laba_colors: Vec<Laba> = config
            .colors
            .iter()
            .map(|c| Srgba::from_components((c[0], c[1], c[2], c[3])).into_color())
            .collect();
        Ok(interpolate_color(&laba_colors, values))
    }
}

/// A trait for color spaces that can be used with the `NumericColorScale`
pub trait ColorSpace:
    Mix<Scalar = f32> + Copy + IntoColor<Srgba> + Debug + Send + Sync + 'static
{
}

impl<T: Mix<Scalar = f32> + Copy + IntoColor<Srgba> + Debug + Send + Sync + 'static> ColorSpace
    for T
{
}

/// Generic helper function to interpolate colors using palette's `Mix` trait
fn interpolate_color<C: ColorSpace>(
    colors: &[C],
    values: &[f32],
) -> ScalarOrArray<ColorOrGradient> {
    let scale_factor = (colors.len() - 1) as f32;
    ScalarOrArray::new_array(
        values
            .iter()
            .map(|v| {
                let continuous_index = (v * scale_factor).clamp(0.0, scale_factor);

                let lower_index = continuous_index.floor() as usize;
                let upper_index = continuous_index.ceil() as usize;

                if lower_index == upper_index {
                    let srgba_color: Srgba = colors[lower_index].into_color();
                    let (r, g, b, a) = srgba_color.into_components();
                    ColorOrGradient::Color([r, g, b, a])
                } else {
                    let lower_color = colors[lower_index];
                    let upper_color = colors[upper_index];
                    let t = continuous_index - lower_index as f32;
                    let srgba_color = lower_color.mix(upper_color, t).into_color();
                    let (r, g, b, a) = srgba_color.into_components();
                    ColorOrGradient::Color([r, g, b, a])
                }
            })
            .collect(),
    )
}

/// Generic helper function to scale numeric values to color values for continuous numeric scales
pub(crate) fn scale_numeric_to_color(
    scale: &impl ArrowScale,
    config: &ScaleConfig,
    values: &ArrayRef,
    interpolator: &dyn ColorInterpolator,
) -> Result<ScalarOrArray<ColorOrGradient>, AvengerScaleError> {
    // Create a new config with a range of [0.0, 1.0] and clamp enabled
    let mut numeric_options = config.options.clone();
    numeric_options.insert("clamp".to_string(), true.into());
    let numeric_config = ScaleConfig {
        range: Arc::new(Float32Array::from(vec![0.0, 1.0])),
        domain: config.domain.clone(),
        options: numeric_options,
    };

    // Scale the values to the numeric range
    let numeric_values = scale.scale_to_numeric(&numeric_config, values)?;

    // Interpolate the values to the color range
    let color_config = ColorInterpolatorConfig {
        colors: config.color_range()?,
    };

    match numeric_values.value() {
        ScalarOrArrayValue::Array(values) => interpolator.interpolate(&color_config, values),
        ScalarOrArrayValue::Scalar(value) => {
            let result = interpolator.interpolate(&color_config, &[*value])?;
            Ok(result.to_scalar_if_len_one())
        }
    }
}

struct MakeSureItsObjectSafe {
    interpolator: Box<dyn ColorInterpolator>,
}
