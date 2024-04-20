use crate::marks::util::{decode_colors, decode_gradients, zindex_to_indices};
use avenger::marks::rule::RuleMark as RsRuleMark;
use avenger::marks::value::{EncodingValue, StrokeCap};
use avenger_vega::error::AvengerVegaError;
use avenger_vega::marks::values::StrokeDashSpec;
use gloo_utils::format::JsValueSerdeExt;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::{JsError, JsValue};

#[wasm_bindgen]
pub struct RuleMark {
    inner: RsRuleMark,
}

impl RuleMark {
    pub fn build(self) -> RsRuleMark {
        self.inner
    }
}

#[wasm_bindgen]
impl RuleMark {
    #[wasm_bindgen(constructor)]
    pub fn new(len: u32, clip: bool, name: Option<String>, zindex: Option<i32>) -> Self {
        Self {
            inner: RsRuleMark {
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

    pub fn set_xy(&mut self, x0: Vec<f32>, y0: Vec<f32>, x1: Vec<f32>, y1: Vec<f32>) {
        self.inner.x0 = EncodingValue::Array { values: x0 };
        self.inner.y0 = EncodingValue::Array { values: y0 };
        self.inner.x1 = EncodingValue::Array { values: x1 };
        self.inner.y1 = EncodingValue::Array { values: y1 };
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

    /// Set stroke cap
    ///
    /// @param {("butt"|"round"|"square")[]} values
    #[wasm_bindgen(skip_jsdoc)]
    pub fn set_stroke_cap(&mut self, values: JsValue) -> Result<(), JsError> {
        let values: Vec<StrokeCap> = values.into_serde()?;
        self.inner.stroke_cap = EncodingValue::Array { values };
        Ok(())
    }

    /// Set stroke dash
    ///
    /// @param {string|number[]} values
    #[wasm_bindgen(skip_jsdoc)]
    pub fn set_stroke_dash(&mut self, values: JsValue) -> Result<(), JsError> {
        let values: Vec<StrokeDashSpec> = values.into_serde()?;
        let values = values
            .into_iter()
            .map(|s| Ok(s.to_array()?.to_vec()))
            .collect::<Result<Vec<_>, AvengerVegaError>>()
            .map_err(|_| JsError::new("Failed to parse dash spec"))?;
        self.inner.stroke_dash = Some(EncodingValue::Array { values });
        Ok(())
    }
}
