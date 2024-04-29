use avenger::marks::value::{ColorOrGradient, Gradient};
use avenger_vega::error::AvengerVegaError;
use avenger_vega::marks::values::CssColorOrGradient;
use gloo_utils::format::JsValueSerdeExt;
use wasm_bindgen::{JsError, JsValue};

pub fn decode_gradients(
    values: JsValue,
    opacity: Vec<f32>,
    gradients: &mut Vec<Gradient>,
) -> Result<Vec<ColorOrGradient>, JsError> {
    let values: Vec<CssColorOrGradient> = values.into_serde()?;
    values
        .iter()
        .zip(opacity)
        .map(|(grad, opacity)| grad.to_color_or_grad(opacity, gradients))
        .collect::<Result<Vec<_>, AvengerVegaError>>()
        .map_err(|_| JsError::new("Failed to parse gradients"))
}

pub fn decode_gradient(
    value: JsValue,
    opacity: f32,
    gradients: &mut Vec<Gradient>,
) -> Result<ColorOrGradient, JsError> {
    let grad: CssColorOrGradient = value.into_serde()?;
    grad.to_color_or_grad(opacity, gradients)
        .map_err(|_| JsError::new("Failed to parse gradient"))
}

pub fn decode_colors(
    color_values: JsValue,
    indices: Vec<usize>,
    opacity: Vec<f32>,
) -> Result<Vec<ColorOrGradient>, JsError> {
    // Parse unique colors
    let color_values: Vec<String> = color_values.into_serde()?;
    let unique_colors = color_values
        .iter()
        .map(|c| {
            let Ok(c) = csscolorparser::parse(c) else {
                return [0.0, 0.0, 0.0, 0.0];
            };
            [c.r as f32, c.g as f32, c.b as f32, c.a as f32]
        })
        .collect::<Vec<_>>();

    // Combine with opacity to build
    let colors = indices
        .iter()
        .zip(opacity)
        .map(|(ind, opacity)| {
            let [r, g, b, a] = unique_colors[*ind as usize];
            ColorOrGradient::Color([r as f32, g as f32, b as f32, a as f32 * opacity])
        })
        .collect::<Vec<_>>();
    Ok(colors)
}

pub fn decode_color(color_value: &str, opacity: f32) -> Result<ColorOrGradient, JsError> {
    Ok(match csscolorparser::parse(color_value) {
        Ok(c) => ColorOrGradient::Color([c.r as f32, c.g as f32, c.b as f32, c.a as f32 * opacity]),
        Err(_) => ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]),
    })
}

pub fn zindex_to_indices(zindex: Vec<i32>) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..zindex.len()).collect();
    indices.sort_by_key(|i| zindex[*i]);
    indices
}
