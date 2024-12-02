use avenger::marks::trail::TrailMark as RsTrailMark;
use avenger::marks::value::ScalarOrArray;
use avenger_vega::marks::values::CssColorOrGradient;
use gloo_utils::format::JsValueSerdeExt;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::{JsError, JsValue};

#[wasm_bindgen]
pub struct TrailMark {
    inner: RsTrailMark,
}

impl TrailMark {
    pub fn build(self) -> RsTrailMark {
        self.inner
    }
}

#[wasm_bindgen]
impl TrailMark {
    #[wasm_bindgen(constructor)]
    pub fn new(len: u32, clip: bool, name: Option<String>, zindex: Option<i32>) -> Self {
        Self {
            inner: RsTrailMark {
                len,
                clip,
                zindex,
                name: name.unwrap_or_default(),
                ..Default::default()
            },
        }
    }

    /// Set fill color
    ///
    /// @param {string|object} color
    /// @param {number} opacity
    #[wasm_bindgen(skip_jsdoc)]
    pub fn set_stroke(&mut self, color: JsValue, opacity: f32) -> Result<(), JsError> {
        let stroke: Option<CssColorOrGradient> = color.into_serde()?;
        if let Some(stroke) = stroke {
            let fill = stroke
                .to_color_or_grad(opacity, &mut self.inner.gradients)
                .map_err(|_| JsError::new("Failed to parse stroke color"))?;
            self.inner.stroke = fill;
        }
        Ok(())
    }

    pub fn set_xy(&mut self, x: Vec<f32>, y: Vec<f32>) {
        self.inner.x = ScalarOrArray::Array { values: x };
        self.inner.y = ScalarOrArray::Array { values: y };
    }

    pub fn set_defined(&mut self, defined: Vec<u8>) -> Result<(), JsError> {
        self.inner.defined = ScalarOrArray::Array {
            values: defined.into_iter().map(|d| d != 0).collect(),
        };
        Ok(())
    }

    pub fn set_size(&mut self, size: Vec<f32>) {
        self.inner.size = ScalarOrArray::Array { values: size };
    }
}
