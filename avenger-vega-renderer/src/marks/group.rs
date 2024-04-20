use crate::marks::rule::RuleMark;
use crate::marks::symbol::SymbolMark;
use crate::marks::text::TextMark;
use avenger::marks::group::{Clip, SceneGroup as RsSceneGroup};
use avenger::marks::mark::SceneMark;
use wasm_bindgen::prelude::wasm_bindgen;

#[wasm_bindgen]
pub struct GroupMark {
    inner: RsSceneGroup,
}

impl GroupMark {
    pub fn build(self) -> RsSceneGroup {
        self.inner
    }
}

#[wasm_bindgen]
impl GroupMark {
    #[wasm_bindgen(constructor)]
    pub fn new(
        origin_x: f32,
        origin_y: f32,
        name: Option<String>,
        width: Option<f32>,
        height: Option<f32>,
    ) -> Self {
        let clip = if let (Some(width), Some(height)) = (width, height) {
            Clip::Rect {
                x: 0.0,
                y: 0.0,
                width: width.clone(),
                height: height.clone(),
            }
        } else {
            Clip::None
        };

        Self {
            inner: RsSceneGroup {
                origin: [origin_x, origin_y],
                name: name.unwrap_or_default(),
                clip,
                ..Default::default()
            },
        }
    }

    pub fn add_symbol_mark(&mut self, mark: SymbolMark) {
        self.inner.marks.push(SceneMark::Symbol(mark.build()));
    }

    pub fn add_rule_mark(&mut self, mark: RuleMark) {
        self.inner.marks.push(SceneMark::Rule(mark.build()));
    }

    pub fn add_text_mark(&mut self, mark: TextMark) {
        self.inner
            .marks
            .push(SceneMark::Text(Box::new(mark.build())));
    }

    pub fn add_group_mark(&mut self, mark: GroupMark) {
        self.inner.marks.push(SceneMark::Group(mark.inner));
    }
}
