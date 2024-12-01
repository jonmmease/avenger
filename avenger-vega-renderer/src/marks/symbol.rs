use crate::marks::util::{decode_colors, decode_gradients, zindex_to_indices};
use avenger_scenegraph::error::AvengerError;
use avenger_scenegraph::marks::symbol::{SymbolMark as RsSymbolMark, SymbolShape};
use avenger_scenegraph::marks::value::EncodingValue;
use gloo_utils::format::JsValueSerdeExt;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::{JsError, JsValue};

#[wasm_bindgen]
pub struct SymbolMark {
    inner: RsSymbolMark,
}

impl SymbolMark {
    pub fn build(self) -> RsSymbolMark {
        self.inner
    }
}

#[wasm_bindgen]
impl SymbolMark {
    #[wasm_bindgen(constructor)]
    pub fn new(len: u32, clip: bool, name: Option<String>, zindex: Option<i32>) -> Self {
        Self {
            inner: RsSymbolMark {
                len,
                clip,
                zindex,
                name: name.unwrap_or_default(),
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

    pub fn set_size(&mut self, size: Vec<f32>) {
        self.inner.size = EncodingValue::Array { values: size };
    }

    pub fn set_angle(&mut self, angle: Vec<f32>) {
        self.inner.angle = EncodingValue::Array { values: angle };
    }

    pub fn set_stroke_width(&mut self, width: Option<f32>) {
        self.inner.stroke_width = width;
    }

    /// Set stroke color
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

    /// Set symbol shape
    ///
    /// @param {string[]} shape_values
    /// @param {Uint32Array} indices
    #[wasm_bindgen(skip_jsdoc)]
    pub fn set_shape(&mut self, shape_values: JsValue, indices: Vec<usize>) -> Result<(), JsError> {
        let shapes: Vec<String> = shape_values.into_serde()?;
        let shapes = shapes
            .iter()
            .map(|s| SymbolShape::from_vega_str(s))
            .collect::<Result<Vec<_>, AvengerError>>()
            .map_err(|_| JsError::new("Failed to parse shapes"))?;

        self.inner.shapes = shapes;
        self.inner.shape_index = EncodingValue::Array { values: indices };
        Ok(())
    }
}
