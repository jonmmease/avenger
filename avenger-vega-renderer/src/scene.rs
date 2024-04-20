// use avenger::error::AvengerError;
//
// use avenger::marks::mark::SceneMark;
// use avenger::marks::symbol::SymbolShape;
// use avenger::marks::text::{FontStyleSpec, FontWeightSpec, TextAlignSpec, TextBaselineSpec};
// use avenger::marks::value::{ColorOrGradient, EncodingValue, Gradient, StrokeCap};
// use avenger::marks::{
//     rule::RuleMark as RsRuleMark, symbol::SymbolMark as RsSymbolMark, text::TextMark as RsTextMark,
// };

// use avenger_vega::error::AvengerVegaError;
// use avenger_vega::marks::values::{CssColorOrGradient, StrokeDashSpec};
// use gloo_utils::format::JsValueSerdeExt;

use crate::marks::group::GroupMark;
use avenger::scene_graph::SceneGraph as RsSceneGraph;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct SceneGraph {
    inner: RsSceneGraph,
}

#[wasm_bindgen]
impl SceneGraph {
    #[wasm_bindgen(constructor)]
    pub fn new(width: f32, height: f32, origin_x: f32, origin_y: f32) -> Self {
        Self {
            inner: RsSceneGraph {
                width,
                height,
                origin: [origin_x, origin_y],
                groups: Vec::new(),
            },
        }
    }

    pub fn add_group(&mut self, group: GroupMark) {
        self.inner.groups.push(group.build())
    }
}

impl SceneGraph {
    pub fn build(self) -> RsSceneGraph {
        self.inner
    }

    pub fn width(&self) -> f32 {
        self.inner.width
    }

    pub fn height(&self) -> f32 {
        self.inner.height
    }

    pub fn origin(&self) -> [f32; 2] {
        self.inner.origin
    }
}
