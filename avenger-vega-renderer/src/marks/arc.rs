use crate::marks::util::{decode_colors, decode_gradients, zindex_to_indices};
use avenger::marks::arc::ArcMark as RsArcMark;
use avenger::marks::value::ScalarOrArray;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::{JsError, JsValue};

#[wasm_bindgen]
pub struct ArcMark {
    inner: RsArcMark,
}

impl ArcMark {
    pub fn build(self) -> RsArcMark {
        self.inner
    }
}

#[wasm_bindgen]
impl ArcMark {
    #[wasm_bindgen(constructor)]
    pub fn new(len: u32, clip: bool, name: Option<String>, zindex: Option<i32>) -> Self {
        Self {
            inner: RsArcMark {
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
        self.inner.x = ScalarOrArray::Array { values: x };
        self.inner.y = ScalarOrArray::Array { values: y };
    }

    pub fn set_start_angle(&mut self, start_angle: Vec<f32>) {
        self.inner.start_angle = ScalarOrArray::Array {
            values: start_angle,
        };
    }

    pub fn set_end_angle(&mut self, end_angle: Vec<f32>) {
        self.inner.end_angle = ScalarOrArray::Array { values: end_angle };
    }

    pub fn set_outer_radius(&mut self, outer_radius: Vec<f32>) {
        self.inner.outer_radius = ScalarOrArray::Array {
            values: outer_radius,
        };
    }

    pub fn set_inner_radius(&mut self, inner_radius: Vec<f32>) {
        self.inner.inner_radius = ScalarOrArray::Array {
            values: inner_radius,
        };
    }

    pub fn set_corner_radius(&mut self, corner_radius: Vec<f32>) {
        self.inner.corner_radius = ScalarOrArray::Array {
            values: corner_radius,
        };
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
        self.inner.stroke = ScalarOrArray::Array {
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
        self.inner.stroke = ScalarOrArray::Array {
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
        self.inner.fill = ScalarOrArray::Array {
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
        self.inner.fill = ScalarOrArray::Array {
            values: decode_gradients(values, opacity, &mut self.inner.gradients)?,
        };
        Ok(())
    }

    pub fn set_stroke_width(&mut self, width: Vec<f32>) {
        self.inner.stroke_width = ScalarOrArray::Array { values: width }
    }
}
