use crate::marks::util::{decode_colors, decode_gradients, zindex_to_indices};
use avenger_scenegraph::marks::rect::SceneRectMark as RsRectMark;
use avenger_scenegraph::marks::value::EncodingValue;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::{JsError, JsValue};

#[wasm_bindgen]
pub struct RectMark {
    inner: RsRectMark,
}

impl RectMark {
    pub fn build(self) -> RsRectMark {
        self.inner
    }
}

#[wasm_bindgen]
impl RectMark {
    #[wasm_bindgen(constructor)]
    pub fn new(len: u32, clip: bool, name: Option<String>, zindex: Option<i32>) -> Self {
        Self {
            inner: RsRectMark {
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
        self.inner.x = EncodingValue::Array { values: x };
        self.inner.y = EncodingValue::Array { values: y };
    }

    pub fn set_width(&mut self, width: Vec<f32>) {
        self.inner.width = EncodingValue::Array { values: width };
    }

    pub fn set_height(&mut self, height: Vec<f32>) {
        self.inner.height = EncodingValue::Array { values: height };
    }

    pub fn set_corner_radius(&mut self, corner_radius: Vec<f32>) {
        self.inner.corner_radius = EncodingValue::Array {
            values: corner_radius,
        };
    }

    pub fn set_stroke_width(&mut self, width: Vec<f32>) {
        self.inner.stroke_width = EncodingValue::Array { values: width }
    }

    /// Set stroke color.
    ///
    /// @param {string[]} color_values
    /// @param {Uint32Array} indices
    /// @param {Float32Array} opacity
    #[wasm_bindgen(skip_jsdoc)]
    pub fn set_stroke(
        &mut self,
        color_values: JsValue,
        indices: Vec<usize>,
        opacity: Vec<f32>,
    ) -> Result<(), JsError> {
        self.inner.stroke = EncodingValue::Array {
            values: decode_colors(color_values, indices, opacity)?,
        };
        Ok(())
    }

    /// Set stroke gradient
    ///
    /// @param {(string|object)[]} values
    /// @param {Float32Array} opacity
    #[wasm_bindgen(skip_jsdoc)]
    pub fn set_stroke_gradient(
        &mut self,
        values: JsValue,
        opacity: Vec<f32>,
    ) -> Result<(), JsError> {
        self.inner.stroke = EncodingValue::Array {
            values: decode_gradients(values, opacity, &mut self.inner.gradients)?,
        };
        Ok(())
    }

    /// Set fill color
    ///
    /// @param {string[]} color_values
    /// @param {Uint32Array} indices
    /// @param {Float32Array} opacity
    #[wasm_bindgen(skip_jsdoc)]
    pub fn set_fill(
        &mut self,
        color_values: JsValue,
        indices: Vec<usize>,
        opacity: Vec<f32>,
    ) -> Result<(), JsError> {
        self.inner.fill = EncodingValue::Array {
            values: decode_colors(color_values, indices, opacity)?,
        };
        Ok(())
    }

    /// Set fill gradient
    ///
    /// @param {(string|object)[]} values
    /// @param {Float32Array} opacity
    #[wasm_bindgen(skip_jsdoc)]
    pub fn set_fill_gradient(&mut self, values: JsValue, opacity: Vec<f32>) -> Result<(), JsError> {
        self.inner.fill = EncodingValue::Array {
            values: decode_gradients(values, opacity, &mut self.inner.gradients)?,
        };
        Ok(())
    }
}
