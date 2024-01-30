use crate::error::AvengerVegaError;
use crate::marks::mark::{VegaMarkContainer, VegaMarkItem};
use avenger::marks::mark::SceneMark;
use avenger::marks::text::{
    FontStyleSpec, FontWeightSpec, TextAlignSpec, TextBaselineSpec, TextMark,
};
use avenger::marks::value::EncodingValue;
use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VegaTextItem {
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    pub text: String,

    // Optional
    pub align: Option<TextAlignSpec>,
    pub angle: Option<f32>,
    pub baseline: Option<TextBaselineSpec>,
    pub dx: Option<f32>,
    pub dy: Option<f32>,
    pub fill: Option<String>,
    pub opacity: Option<f32>,
    pub fill_opacity: Option<f32>,
    pub font: Option<String>,
    pub font_size: Option<f32>,
    pub font_weight: Option<FontWeightSpec>,
    pub font_style: Option<FontStyleSpec>,
    pub limit: Option<f32>,
    pub zindex: Option<i32>,
}

impl VegaMarkItem for VegaTextItem {}

impl VegaMarkContainer<VegaTextItem> {
    pub fn to_scene_graph(&self) -> Result<SceneMark, AvengerVegaError> {
        // Init mark with scalar defaults
        let mut mark = TextMark {
            clip: self.clip,
            ..Default::default()
        };
        if let Some(name) = &self.name {
            mark.name = name.clone();
        }

        // Init vector for each encoding channel
        let mut text = Vec::<String>::new();
        let mut x = Vec::<f32>::new();
        let mut y = Vec::<f32>::new();
        let mut align = Vec::<TextAlignSpec>::new();
        let mut baseline = Vec::<TextBaselineSpec>::new();
        let mut angle = Vec::<f32>::new();
        let mut color = Vec::<[f32; 4]>::new();
        let mut dx = Vec::<f32>::new();
        let mut dy = Vec::<f32>::new();
        let mut font = Vec::<String>::new();
        let mut font_size = Vec::<f32>::new();
        let mut font_weight = Vec::<FontWeightSpec>::new();
        let mut font_style = Vec::<FontStyleSpec>::new();
        let mut limit = Vec::<f32>::new();
        let mut zindex = Vec::<i32>::new();

        for item in &self.items {
            x.push(item.x);
            y.push(item.y);
            text.push(item.text.clone());

            if let Some(v) = item.align {
                align.push(v);
            }

            if let Some(v) = item.baseline {
                baseline.push(v);
            }

            if let Some(v) = item.angle {
                angle.push(v);
            }

            if let Some(v) = &item.fill {
                let c = csscolorparser::parse(v)?;
                let opacity =
                    c.a as f32 * item.fill_opacity.unwrap_or(1.0) * item.opacity.unwrap_or(1.0);
                color.push([c.r as f32, c.g as f32, c.b as f32, opacity])
            }

            if let Some(v) = item.dx {
                dx.push(v);
            }

            if let Some(v) = item.dy {
                dy.push(v);
            }

            if let Some(v) = &item.font {
                font.push(v.clone());
            }

            if let Some(v) = item.font_size {
                font_size.push(v);
            }

            if let Some(v) = item.font_weight {
                font_weight.push(v);
            }

            if let Some(v) = item.font_style {
                font_style.push(v);
            }

            if let Some(v) = item.limit {
                limit.push(v);
            }

            if let Some(v) = item.zindex {
                zindex.push(v);
            }
        }

        // Override values with vectors
        let len = self.items.len();
        mark.len = len as u32;

        if x.len() == len {
            mark.x = EncodingValue::Array { values: x };
        }
        if y.len() == len {
            mark.y = EncodingValue::Array { values: y };
        }
        if text.len() == len {
            mark.text = EncodingValue::Array { values: text };
        }
        if align.len() == len {
            mark.align = EncodingValue::Array { values: align };
        }
        if baseline.len() == len {
            mark.baseline = EncodingValue::Array { values: baseline };
        }
        if angle.len() == len {
            mark.angle = EncodingValue::Array { values: angle };
        }
        if color.len() == len {
            mark.color = EncodingValue::Array { values: color };
        }
        if dx.len() == len {
            mark.dx = EncodingValue::Array { values: dx };
        }
        if dy.len() == len {
            mark.dy = EncodingValue::Array { values: dy };
        }
        if font.len() == len {
            mark.font = EncodingValue::Array { values: font };
        }
        if font_size.len() == len {
            mark.font_size = EncodingValue::Array { values: font_size };
        }
        if font_weight.len() == len {
            mark.font_weight = EncodingValue::Array {
                values: font_weight,
            };
        }
        if font_style.len() == len {
            mark.font_style = EncodingValue::Array { values: font_style };
        }
        if limit.len() == len {
            mark.limit = EncodingValue::Array { values: limit };
        }
        if zindex.len() == len {
            let mut indices: Vec<usize> = (0..len).collect();
            indices.sort_by_key(|i| zindex[*i]);
            mark.indices = Some(indices);
        }
        Ok(SceneMark::Text(Box::new(mark)))
    }
}
