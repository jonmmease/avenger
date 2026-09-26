use std::{fmt::Debug, sync::Arc};

use arrow::{
    array::{ArrayRef, AsArray, Float32Array, ListArray},
    buffer::OffsetBuffer,
    datatypes::{DataType, Field, Float32Type},
};
use avenger_color::{interpolate_colors, ColorInterpolationSpace};

use crate::{
    error::AvengerScaleError,
    scales::{ScaleConfig, ScaleImpl},
};

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
        interpolate_to_arrow(ColorInterpolationSpace::Srgba, &config.colors, values)
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
        interpolate_to_arrow(ColorInterpolationSpace::Hsla, &config.colors, values)
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
        interpolate_to_arrow(ColorInterpolationSpace::Laba, &config.colors, values)
    }
}

fn interpolate_to_arrow(
    space: ColorInterpolationSpace,
    colors: &[[f32; 4]],
    values: &[f32],
) -> Result<ArrayRef, AvengerScaleError> {
    let colors = interpolate_colors(space, colors, values)
        .map_err(|e| AvengerScaleError::InternalError(e.to_string()))?;
    let mut flat_values = Vec::with_capacity(values.len() * 4);
    colors
        .iter()
        .for_each(|color| flat_values.extend_from_slice(color));

    Ok(Arc::new(ListArray::new(
        Arc::new(Field::new_list_field(DataType::Float32, true)),
        OffsetBuffer::from_lengths(vec![4; values.len()]),
        Arc::new(Float32Array::from(flat_values)),
        None,
    )))
}

/// Generic helper function to scale numeric values to color values for continuous numeric scales
pub(crate) fn scale_numeric_to_color(
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

#[allow(dead_code)]
struct MakeSureItsObjectSafe {
    interpolator: Box<dyn ColorInterpolator>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::Array;

    fn adapters() -> [(&'static dyn ColorInterpolator, ColorInterpolationSpace); 3] {
        [
            (&SrgbaColorInterpolator, ColorInterpolationSpace::Srgba),
            (&HslaColorInterpolator, ColorInterpolationSpace::Hsla),
            (&LabaColorInterpolator, ColorInterpolationSpace::Laba),
        ]
    }

    #[test]
    fn adapters_preserve_color_space_and_arrow_layout() {
        let config = ColorInterpolatorConfig {
            colors: vec![[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 1.0]],
        };
        let values = [0.25, 0.75];
        for (adapter, space) in adapters() {
            let result = adapter.interpolate(&config, &values).unwrap();
            assert_eq!(
                result.data_type(),
                &DataType::List(Arc::new(Field::new_list_field(DataType::Float32, true)))
            );
            let lists = result.as_list::<i32>();
            assert_eq!(lists.len(), values.len());
            assert_eq!(lists.null_count(), 0);
            assert_eq!(lists.value_offsets(), &[0, 4, 8]);
            let expected: Vec<f32> = interpolate_colors(space, &config.colors, &values)
                .unwrap()
                .into_iter()
                .flatten()
                .collect();
            assert_eq!(
                lists
                    .values()
                    .as_primitive::<Float32Type>()
                    .values()
                    .as_ref(),
                expected.as_slice()
            );
        }
    }

    #[test]
    fn empty_values_preserve_arrow_type() {
        let config = ColorInterpolatorConfig {
            colors: vec![[0.2, 0.4, 0.6, 1.0]],
        };
        for (adapter, _) in adapters() {
            let result = adapter.interpolate(&config, &[]).unwrap();
            let lists = result.as_list::<i32>();
            assert_eq!(lists.len(), 0);
            assert_eq!(lists.value_offsets(), &[0]);
            assert_eq!(lists.values().data_type(), &DataType::Float32);
            assert_eq!(lists.values().len(), 0);
        }
    }

    #[test]
    fn empty_color_errors_are_propagated() {
        let config = ColorInterpolatorConfig { colors: vec![] };
        for (adapter, _) in adapters() {
            let error = adapter.interpolate(&config, &[0.5]).unwrap_err();
            let AvengerScaleError::InternalError(message) = error else {
                panic!("unexpected error: {error}");
            };
            assert_eq!(
                message,
                avenger_color::ColorInterpolationError::EmptyColorRange.to_string()
            );
        }
    }
}
