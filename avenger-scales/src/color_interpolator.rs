use crate::{
    error::AvengerScaleError,
    formatter::Formatters,
    scales::{ScaleConfig, ScaleContext, ScaleImpl},
};
use arrow::{
    array::{ArrayRef, AsArray, Float32Array, ListArray},
    buffer::OffsetBuffer,
    datatypes::{DataType, Field, Float32Type},
};
use avenger_common::{
    types::ColorOrGradient,
    value::{ScalarOrArray, ScalarOrArrayValue},
};
use palette::{Hsla, IntoColor, Laba, Mix, Srgba};
use std::{fmt::Debug, sync::Arc};

pub struct ColorInterpolatorConfig {
    pub colors: Vec<[f32; 4]>,
}

pub trait ColorInterpolator: Debug + Send + Sync + 'static {
    /// Interpolate over evenly spaced colors based on normalized values
    fn interpolate(
        &self,
        config: &ColorInterpolatorConfig,
        values: &[f32],
    ) -> Result<ArrayRef, AvengerScaleError>;
}

#[derive(Clone, Debug)]
pub struct SrgbaColorInterpolator;

impl ColorInterpolator for SrgbaColorInterpolator {
    fn interpolate(
        &self,
        config: &ColorInterpolatorConfig,
        values: &[f32],
    ) -> Result<ArrayRef, AvengerScaleError> {
        // Interpolate in Srgba space
        let srgba_colors: Vec<Srgba> = config
            .colors
            .iter()
            .map(|c| Srgba::from_components((c[0], c[1], c[2], c[3])))
            .collect();
        interpolate_color(&srgba_colors, values)
    }
}

#[derive(Clone, Debug)]
pub struct HslaColorInterpolator;

impl ColorInterpolator for HslaColorInterpolator {
    fn interpolate(
        &self,
        config: &ColorInterpolatorConfig,
        values: &[f32],
    ) -> Result<ArrayRef, AvengerScaleError> {
        let hsla_colors: Vec<Hsla> = config
            .colors
            .iter()
            .map(|c| Srgba::from_components((c[0], c[1], c[2], c[3])).into_color())
            .collect();
        interpolate_color(&hsla_colors, values)
    }
}

#[derive(Clone, Debug)]
pub struct LabaColorInterpolator;

impl ColorInterpolator for LabaColorInterpolator {
    fn interpolate(
        &self,
        config: &ColorInterpolatorConfig,
        values: &[f32],
    ) -> Result<ArrayRef, AvengerScaleError> {
        let laba_colors: Vec<Laba> = config
            .colors
            .iter()
            .map(|c| Srgba::from_components((c[0], c[1], c[2], c[3])).into_color())
            .collect();
        interpolate_color(&laba_colors, values)
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
) -> Result<ArrayRef, AvengerScaleError> {
    let scale_factor = (colors.len() - 1) as f32;
    let mut flat_values = Vec::with_capacity(values.len() * 4);
    values.iter().for_each(|v| {
        let continuous_index = (v * scale_factor).clamp(0.0, scale_factor);
        let lower_index = continuous_index.floor() as usize;
        let upper_index = continuous_index.ceil() as usize;

        if lower_index == upper_index {
            let srgba_color: Srgba = colors[lower_index].into_color();
            let (r, g, b, a) = srgba_color.into_components();
            flat_values.extend_from_slice(&[r, g, b, a]);
        } else {
            let lower_color = colors[lower_index];
            let upper_color = colors[upper_index];
            let t = continuous_index - lower_index as f32;
            let srgba_color = lower_color.mix(upper_color, t).into_color();
            let (r, g, b, a) = srgba_color.into_components();
            flat_values.extend_from_slice(&[r, g, b, a]);
        }
    });

    Ok(Arc::new(ListArray::new(
        Arc::new(Field::new_list_field(DataType::Float32, true)),
        OffsetBuffer::from_lengths(vec![4; values.len()]),
        Arc::new(Float32Array::from(flat_values)),
        None,
    )))
}

/// Generic helper function to scale numeric values to color values for continuous numeric scales

/// Generic helper function to scale numeric values to color values for continuous numeric scales
pub(crate) fn scale_numeric_to_color2(
    scale: &impl ScaleImpl,
    config: &ScaleConfig,
    values: &ArrayRef,
) -> Result<ArrayRef, AvengerScaleError> {
    // Create a new config with a range of [0.0, 1.0] and clamp enabled
    let mut numeric_options = config.options.clone();
    numeric_options.insert("clamp".to_string(), true.into());

    // Scale the values to interval [0.0, 1.0]
    let numeric_config = ScaleConfig {
        range: Arc::new(Float32Array::from(vec![0.0, 1.0])),
        domain: config.domain.clone(),
        options: numeric_options,
        context: config.context.clone(),
    };
    let numeric_values = scale.scale(&numeric_config, values)?;
    let numeric_values = numeric_values.as_primitive::<Float32Type>();

    let color_config = ColorInterpolatorConfig {
        colors: config.color_range()?,
    };
    config
        .context
        .color_interpolator
        .interpolate(&color_config, numeric_values.values())
}

struct MakeSureItsObjectSafe {
    interpolator: Box<dyn ColorInterpolator>,
}
