use wasm_bindgen::prelude::*;
use web_sys::console::group;
use avenger::marks::symbol::{SymbolMark as RsSymbolMark};
use avenger::marks::group::{SceneGroup as RsSceneGroup, SceneGroup};
use avenger::marks::mark::SceneMark;
use avenger::marks::value::{ColorOrGradient, EncodingValue};
use avenger::scene_graph::{SceneGraph as RsSceneGraph};
use crate::log;

#[wasm_bindgen]
pub struct SymbolMark {
    inner: RsSymbolMark
}

#[wasm_bindgen]
impl SymbolMark {
    #[wasm_bindgen(constructor)]
    pub fn new(len: u32, clip: bool, name: Option<String>) -> Self {
        Self {
            inner: RsSymbolMark {
                len,
                clip,
                name: name.unwrap_or_default(),
                fill: EncodingValue::Scalar { value: ColorOrGradient::Color([0.0, 0.0, 1.0, 0.5])},
                ..Default::default()
            }
        }
    }

    pub fn set_xy(&mut self, x: Vec<f32>, y: Vec<f32>) {
        self.inner.x = EncodingValue::Array {values: x};
        self.inner.y = EncodingValue::Array {values: y};
    }

    pub fn set_size(&mut self, size: Vec<f32>) {
        self.inner.size = EncodingValue::Array {values: size};
    }

    pub fn set_angle(&mut self, angle: Vec<f32>) {
        self.inner.angle = EncodingValue::Array {values: angle};
    }

    pub fn set_zindex(&mut self, zindex: Option<i32>) {
        self.inner.zindex = zindex;
    }

    // TODO
    // pub fill: EncodingValue<ColorOrGradient>,
    // pub stroke: EncodingValue<ColorOrGradient>,
    // pub indices: Option<Vec<usize>>,
}


#[wasm_bindgen]
pub struct GroupMark {
    inner: RsSceneGroup
}

#[wasm_bindgen]
impl GroupMark {
    #[wasm_bindgen(constructor)]
    pub fn new(origin_x: f32, origin_y: f32, name: Option<String>) -> Self {
        Self {
            inner: RsSceneGroup {
                origin: [origin_x, origin_y],
                name: name.unwrap_or_default(),
                ..Default::default()
            }
        }
    }

    pub fn add_symbol_mark(&mut self, mark: SymbolMark) {
        self.inner.marks.push(SceneMark::Symbol(mark.inner));
    }
}


#[wasm_bindgen]
pub struct SceneGraph {
    inner: RsSceneGraph
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
            }
        }
    }

    pub fn add_group(&mut self, group: GroupMark) {
        self.inner.groups.push(group.inner)
    }
}

impl SceneGraph {
    pub fn build(self) -> RsSceneGraph {
        self.inner
    }
}