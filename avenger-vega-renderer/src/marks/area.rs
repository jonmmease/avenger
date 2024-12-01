use avenger_scenegraph::marks::area::{SceneAreaMark as RsAreaMark, AreaOrientation};
use avenger_scenegraph::marks::value::{EncodingValue, StrokeCap, StrokeJoin};
use avenger_vega::marks::values::{CssColorOrGradient, StrokeDashSpec};
use gloo_utils::format::JsValueSerdeExt;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::{JsError, JsValue};

#[wasm_bindgen]
pub struct AreaMark {
    inner: RsAreaMark,
}

impl AreaMark {
    pub fn build(self) -> RsAreaMark {
        self.inner
    }
}

#[wasm_bindgen]
impl AreaMark {
    #[wasm_bindgen(constructor)]
    pub fn new(len: u32, clip: bool, name: Option<String>, zindex: Option<i32>) -> Self {
        Self {
            inner: RsAreaMark {
                len,
                clip,
                zindex,
                name: name.unwrap_or_default(),
                ..Default::default()
            },
        }
    }

    pub fn set_stroke_width(&mut self, width: f32) {
        self.inner.stroke_width = width;
    }

    /// Set stroke cap
    ///
    /// @param {"butt"|"round"|"square"} cap
    pub fn set_stroke_cap(&mut self, cap: JsValue) -> Result<(), JsError> {
        let cap: Option<StrokeCap> = cap.into_serde()?;
        if let Some(cap) = cap {
            self.inner.stroke_cap = cap;
        }
        Ok(())
    }

    /// Set stroke cap
    ///
    /// @param {"bevel"|"miter"|"round"} join
    pub fn set_stroke_join(&mut self, join: JsValue) -> Result<(), JsError> {
        let join: Option<StrokeJoin> = join.into_serde()?;
        if let Some(join) = join {
            self.inner.stroke_join = join;
        }
        Ok(())
    }

    /// Set stroke cap
    ///
    /// @param {"vertical"|"horizontal"} orient
    pub fn set_orient(&mut self, orient: JsValue) -> Result<(), JsError> {
        let orient: Option<AreaOrientation> = orient.into_serde()?;
        if let Some(orient) = orient {
            self.inner.orientation = orient;
        }
        Ok(())
    }

    /// Set stroke color
    ///
    /// @param {string|object} color
    /// @param {number} opacity
    #[wasm_bindgen(skip_jsdoc)]
    pub fn set_stroke(&mut self, color: JsValue, opacity: f32) -> Result<(), JsError> {
        let stroke: Option<CssColorOrGradient> = color.into_serde()?;
        if let Some(stroke) = stroke {
            let stroke = stroke
                .to_color_or_grad(opacity, &mut self.inner.gradients)
                .map_err(|_| JsError::new("Failed to parse stroke color"))?;
            self.inner.stroke = stroke;
        }
        Ok(())
    }

    /// Set fill color
    ///
    /// @param {string|object} color
    /// @param {number} opacity
    #[wasm_bindgen(skip_jsdoc)]
    pub fn set_fill(&mut self, color: JsValue, opacity: f32) -> Result<(), JsError> {
        let fill: Option<CssColorOrGradient> = color.into_serde()?;
        if let Some(fill) = fill {
            let fill = fill
                .to_color_or_grad(opacity, &mut self.inner.gradients)
                .map_err(|_| JsError::new("Failed to parse stroke color"))?;
            self.inner.fill = fill;
        }
        Ok(())
    }

    /// Set stroke dash
    ///
    /// @param {string|number[]} values
    pub fn set_stroke_dash(&mut self, dash: JsValue) -> Result<(), JsError> {
        let dash: Option<StrokeDashSpec> = dash.into_serde()?;
        if let Some(dash) = dash {
            let dash_array = dash
                .to_array()
                .map(|a| a.to_vec())
                .map_err(|_| JsError::new("Failed to parse dash spec"))?;
            self.inner.stroke_dash = Some(dash_array);
        }
        Ok(())
    }

    pub fn set_xy(&mut self, x: Vec<f32>, y: Vec<f32>) {
        self.inner.x = EncodingValue::Array { values: x };
        self.inner.y = EncodingValue::Array { values: y };
    }

    pub fn set_x2(&mut self, x2: Vec<f32>) {
        self.inner.x2 = EncodingValue::Array { values: x2 };
    }

    pub fn set_y2(&mut self, y2: Vec<f32>) {
        self.inner.y2 = EncodingValue::Array { values: y2 };
    }

    pub fn set_defined(&mut self, defined: Vec<u8>) -> Result<(), JsError> {
        self.inner.defined = EncodingValue::Array {
            values: defined.into_iter().map(|d| d != 0).collect(),
        };
        Ok(())
    }
}
