use crate::marks::util::zindex_to_indices;
use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::marks::text::SceneTextMark as RsTextMark;
use avenger_text::types::{FontStyleSpec, FontWeightSpec, TextAlignSpec, TextBaselineSpec};
use gloo_utils::format::JsValueSerdeExt;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::{JsError, JsValue};

#[wasm_bindgen]
pub struct TextMark {
    inner: RsTextMark,
}

impl TextMark {
    pub fn build(self) -> RsTextMark {
        self.inner
    }
}

#[wasm_bindgen]
impl TextMark {
    #[wasm_bindgen(constructor)]
    pub fn new(len: u32, clip: bool, name: Option<String>, zindex: Option<i32>) -> Self {
        Self {
            inner: RsTextMark {
                len,
                clip,
                name: name.unwrap_or_default(),
                zindex,
                ..Default::default()
            },
        }
    }

    pub fn set_zindex(&mut self, zindex: Vec<i32>) {
        self.inner.indices = Some(zindex_to_indices(zindex));
    }

    pub fn set_xy(&mut self, x: Vec<f32>, y: Vec<f32>) {
        self.inner.x = ScalarOrArray::Array(x);
        self.inner.y = ScalarOrArray::Array(y);
    }

    pub fn set_angle(&mut self, angle: Vec<f32>) {
        self.inner.angle = ScalarOrArray::Array(angle);
    }

    pub fn set_font_size(&mut self, font_size: Vec<f32>) {
        self.inner.font_size = ScalarOrArray::Array(font_size);
    }

    pub fn set_font_limit(&mut self, limit: Vec<f32>) {
        self.inner.limit = ScalarOrArray::Array(limit);
    }

    pub fn set_indices(&mut self, indices: Vec<usize>) {
        self.inner.indices = Some(indices);
    }

    /// Set text
    ///
    /// @param {string[]} text
    #[wasm_bindgen(skip_jsdoc)]
    pub fn set_text(&mut self, text: JsValue) -> Result<(), JsError> {
        let text: Vec<String> = text.into_serde()?;
        self.inner.text = ScalarOrArray::Array(text);
        Ok(())
    }

    /// Set font
    ///
    /// @param {string[]} font_values
    /// @param {Uint32Array} indices
    #[wasm_bindgen(skip_jsdoc)]
    pub fn set_font(&mut self, font_values: JsValue, indices: Vec<usize>) -> Result<(), JsError> {
        let font_values: Vec<String> = font_values.into_serde()?;
        let values = indices
            .iter()
            .map(|ind| font_values[*ind].clone())
            .collect::<Vec<_>>();
        self.inner.font = ScalarOrArray::Array(values);
        Ok(())
    }

    /// Set alignment
    ///
    /// @param {("left"|"center"|"right")[]} align_values
    /// @param {Uint32Array} indices
    #[wasm_bindgen(skip_jsdoc)]
    pub fn set_align(&mut self, align_values: JsValue, indices: Vec<usize>) -> Result<(), JsError> {
        let align_values: Vec<TextAlignSpec> = align_values.into_serde()?;
        let values = indices
            .iter()
            .map(|ind| align_values[*ind].clone())
            .collect::<Vec<_>>();
        self.inner.align = ScalarOrArray::Array(values);
        Ok(())
    }

    /// Set alignment
    ///
    /// @param {("alphabetic"|"top"|"middle"|"bottom"|"line-top"|"line-bottom")[]} baseline_values
    /// @param {Uint32Array} indices
    #[wasm_bindgen(skip_jsdoc)]
    pub fn set_baseline(
        &mut self,
        baseline_values: JsValue,
        indices: Vec<usize>,
    ) -> Result<(), JsError> {
        let baseline_values: Vec<TextBaselineSpec> = baseline_values.into_serde()?;
        let values = indices
            .iter()
            .map(|ind| baseline_values[*ind].clone())
            .collect::<Vec<_>>();
        self.inner.baseline = ScalarOrArray::Array(values);
        Ok(())
    }

    /// Set font weight
    ///
    /// @param {(number|"normal"|"bold")[]} weight_values
    /// @param {Uint32Array} indices
    #[wasm_bindgen(skip_jsdoc)]
    pub fn set_font_weight(
        &mut self,
        weight_values: JsValue,
        indices: Vec<usize>,
    ) -> Result<(), JsError> {
        let weight_values: Vec<FontWeightSpec> = weight_values.into_serde()?;
        let values = indices
            .iter()
            .map(|ind| weight_values[*ind].clone())
            .collect::<Vec<_>>();
        self.inner.font_weight = ScalarOrArray::Array(values);
        Ok(())
    }

    /// Set font style
    ///
    /// @param {("normal"|"italic")[]} style_values
    /// @param {Uint32Array} indices
    #[wasm_bindgen(skip_jsdoc)]
    pub fn set_font_style(
        &mut self,
        style_values: JsValue,
        indices: Vec<usize>,
    ) -> Result<(), JsError> {
        let style_values: Vec<FontStyleSpec> = style_values.into_serde()?;
        let values = indices
            .iter()
            .map(|ind| style_values[*ind].clone())
            .collect::<Vec<_>>();
        self.inner.font_style = ScalarOrArray::Array(values);
        Ok(())
    }

    /// Set text color
    ///
    /// @param {string[]} color_values
    /// @param {Uint32Array} indices
    /// @param {Float32Array} opacity
    #[wasm_bindgen(skip_jsdoc)]
    pub fn set_color(
        &mut self,
        color_values: JsValue,
        indices: Vec<usize>,
        opacity: Vec<f32>,
    ) -> Result<(), JsError> {
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
                let [r, g, b, a] = unique_colors[*ind];
                [r as f32, g as f32, b as f32, a as f32 * opacity]
            })
            .collect::<Vec<_>>();

        self.inner.color = ScalarOrArray::Array(colors);
        Ok(())
    }
}
