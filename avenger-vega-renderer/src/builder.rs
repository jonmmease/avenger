use crate::log;
use avenger::error::AvengerError;
use avenger::marks::group::{Clip, SceneGroup as RsSceneGroup, SceneGroup};
use avenger::marks::mark::SceneMark;
use avenger::marks::symbol::SymbolShape;
use avenger::marks::text::{FontStyleSpec, FontWeightSpec, TextAlignSpec, TextBaselineSpec};
use avenger::marks::value::{ColorOrGradient, EncodingValue};
use avenger::marks::{
    rule::RuleMark as RsRuleMark, symbol::SymbolMark as RsSymbolMark, text::TextMark as RsTextMark,
};
use avenger::scene_graph::SceneGraph as RsSceneGraph;
use avenger_vega::marks::mark::VegaMarkContainer;
use avenger_vega::marks::text::VegaTextItem;
use gloo_utils::format::JsValueSerdeExt;
use wasm_bindgen::prelude::*;
use web_sys::console::group;

#[wasm_bindgen]
pub struct SymbolMark {
    inner: RsSymbolMark,
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
                fill: EncodingValue::Scalar {
                    value: ColorOrGradient::Color([0.0, 0.0, 1.0, 0.5]),
                },
                ..Default::default()
            },
        }
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

    pub fn set_zindex(&mut self, zindex: Option<i32>) {
        self.inner.zindex = zindex;
    }

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

    pub fn set_shape(&mut self, shape_values: JsValue, indices: Vec<usize>) -> Result<(), JsError> {
        let shapes: Vec<String> = shape_values.into_serde()?;
        let shapes = shapes
            .iter()
            .map(|s| SymbolShape::from_vega_str(s))
            .collect::<Result<Vec<_>, AvengerError>>()
            .map_err(|e| JsError::new("Failed to parse shapes"))?;

        self.inner.shapes = shapes;
        self.inner.shape_index = EncodingValue::Array { values: indices };
        Ok(())
    }

    // TODO
    // pub indices: Option<Vec<usize>>,
}

#[wasm_bindgen]
pub struct RuleMark {
    inner: RsRuleMark,
}

#[wasm_bindgen]
impl RuleMark {
    #[wasm_bindgen(constructor)]
    pub fn new(len: u32, clip: bool, name: Option<String>) -> Self {
        Self {
            inner: RsRuleMark {
                len,
                clip,
                name: name.unwrap_or_default(),
                stroke: EncodingValue::Scalar {
                    value: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]),
                },
                ..Default::default()
            },
        }
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
}

#[wasm_bindgen]
pub struct TextMark {
    inner: RsTextMark,
}

#[wasm_bindgen]
impl TextMark {
    #[wasm_bindgen(constructor)]
    pub fn new(len: u32, clip: bool, name: Option<String>) -> Self {
        Self {
            inner: RsTextMark {
                len,
                clip,
                name: name.unwrap_or_default(),
                ..Default::default()
            },
        }
    }

    pub fn set_xy(&mut self, x: Vec<f32>, y: Vec<f32>) {
        self.inner.x = EncodingValue::Array { values: x };
        self.inner.y = EncodingValue::Array { values: y };
    }

    pub fn set_angle(&mut self, angle: Vec<f32>) {
        self.inner.angle = EncodingValue::Array { values: angle };
    }

    pub fn set_font_size(&mut self, font_size: Vec<f32>) {
        self.inner.font_size = EncodingValue::Array { values: font_size };
    }

    pub fn set_font_limit(&mut self, limit: Vec<f32>) {
        self.inner.limit = EncodingValue::Array { values: limit };
    }

    pub fn set_indices(&mut self, indices: Vec<usize>) {
        self.inner.indices = Some(indices);
    }

    pub fn set_zindex(&mut self, zindex: i32) {
        self.inner.zindex = Some(zindex);
    }

    pub fn set_text(&mut self, text: JsValue) -> Result<(), JsError> {
        let text: Vec<String> = text.into_serde()?;
        self.inner.text = EncodingValue::Array { values: text };
        Ok(())
    }

    pub fn set_font(&mut self, font_values: JsValue, indices: Vec<usize>) -> Result<(), JsError> {
        let font_values: Vec<String> = font_values.into_serde()?;
        let values = indices
            .iter()
            .map(|ind| font_values[*ind].clone())
            .collect::<Vec<_>>();
        self.inner.font = EncodingValue::Array { values };
        Ok(())
    }

    pub fn set_align(&mut self, align_values: JsValue, indices: Vec<usize>) -> Result<(), JsError> {
        let align_values: Vec<TextAlignSpec> = align_values.into_serde()?;
        let values = indices
            .iter()
            .map(|ind| align_values[*ind].clone())
            .collect::<Vec<_>>();
        self.inner.align = EncodingValue::Array { values };
        Ok(())
    }

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
        self.inner.baseline = EncodingValue::Array { values };
        Ok(())
    }

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
        self.inner.font_weight = EncodingValue::Array { values };
        Ok(())
    }

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
        self.inner.font_style = EncodingValue::Array { values };
        Ok(())
    }

    pub fn set_color(
        &mut self,
        color_values: JsValue,
        indices: Vec<usize>,
        opacity: Vec<f32>,
    ) -> Result<(), JsError> {
        // Parse unique colors
        let color_values: Vec<String> = color_values.into_serde()?;
        let unique_strokes = color_values
            .iter()
            .map(|c| {
                let Ok(c) = csscolorparser::parse(c) else {
                    return [0.0, 0.0, 0.0, 1.0];
                };
                [c.r as f32, c.g as f32, c.b as f32, c.a as f32]
            })
            .collect::<Vec<_>>();

        // Combine with opacity to build
        let colors = indices
            .iter()
            .zip(opacity)
            .map(|(ind, opacity)| {
                let [r, g, b, a] = unique_strokes[*ind as usize];
                [r as f32, g as f32, b as f32, a as f32 * opacity]
            })
            .collect::<Vec<_>>();

        self.inner.color = EncodingValue::Array { values: colors };
        Ok(())
    }
}

fn decode_colors(
    color_values: JsValue,
    indices: Vec<usize>,
    opacity: Vec<f32>,
) -> Result<Vec<ColorOrGradient>, JsError> {
    // Parse unique colors
    let color_values: Vec<String> = color_values.into_serde()?;
    let unique_strokes = color_values
        .iter()
        .map(|c| {
            let Ok(c) = csscolorparser::parse(c) else {
                return [0.0, 0.0, 0.0, 1.0];
            };
            [c.r as f32, c.g as f32, c.b as f32, c.a as f32]
        })
        .collect::<Vec<_>>();

    // Combine with opacity to build
    let colors = indices
        .iter()
        .zip(opacity)
        .map(|(ind, opacity)| {
            let [r, g, b, a] = unique_strokes[*ind as usize];
            ColorOrGradient::Color([r as f32, g as f32, b as f32, a as f32 * opacity])
        })
        .collect::<Vec<_>>();
    Ok(colors)
}

#[wasm_bindgen]
pub struct GroupMark {
    inner: RsSceneGroup,
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
        self.inner.marks.push(SceneMark::Symbol(mark.inner));
    }

    pub fn add_rule_mark(&mut self, mark: RuleMark) {
        self.inner.marks.push(SceneMark::Rule(mark.inner));
    }

    pub fn add_text_mark(&mut self, mark: TextMark) {
        self.inner.marks.push(SceneMark::Text(Box::new(mark.inner)));
    }

    pub fn add_group_mark(&mut self, mark: GroupMark) {
        self.inner.marks.push(SceneMark::Group(mark.inner));
    }
}

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
        self.inner.groups.push(group.inner)
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
