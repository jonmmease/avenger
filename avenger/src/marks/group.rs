use crate::marks::mark::SceneMark;
use crate::marks::rect::RectMark;
use crate::marks::value::{ColorOrGradient, EncodingValue, Gradient};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct GroupBounds {
    pub x: f32,
    pub y: f32,
    pub width: Option<f32>,
    pub height: Option<f32>,
}

impl Default for GroupBounds {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: None,
            height: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneGroup {
    pub name: String,
    pub bounds: GroupBounds,
    pub marks: Vec<SceneMark>,
    pub gradients: Vec<Gradient>,
    pub fill: Option<ColorOrGradient>,
    pub stroke: Option<ColorOrGradient>,
    pub stroke_width: Option<f32>,
    pub stroke_offset: Option<f32>,
    pub corner_radius: Option<f32>,
}

impl SceneGroup {
    pub fn make_rect(&self) -> Option<RectMark> {
        if self.fill.is_none() && self.stroke.is_none() {
            return None;
        }
        let stroke_width =
            self.stroke_width
                .unwrap_or(if self.stroke.is_some() { 1.0 } else { 0.0 });
        let stroke_offset = if let Some(stroke_offset) = self.stroke_offset {
            stroke_offset
        } else {
            // From Vega's default stroke offset logic
            if self.stroke.is_some() && stroke_width > 0.5 && stroke_width < 1.5 {
                0.5 - (stroke_width - 1.0).abs()
            } else {
                0.0
            }
        };
        Some(RectMark {
            name: format!("rect_{}", self.name),
            clip: false,
            len: 1,
            gradients: self.gradients.clone(),
            x: EncodingValue::Scalar {
                value: self.bounds.x + stroke_offset,
            },
            y: EncodingValue::Scalar {
                value: self.bounds.y + stroke_offset,
            },
            width: EncodingValue::Scalar {
                value: self.bounds.width.unwrap_or(0.0),
            },
            height: EncodingValue::Scalar {
                value: self.bounds.height.unwrap_or(0.0),
            },
            fill: EncodingValue::Scalar {
                value: self
                    .fill
                    .clone()
                    .unwrap_or(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            },
            stroke: EncodingValue::Scalar {
                value: self
                    .stroke
                    .clone()
                    .unwrap_or(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            },
            stroke_width: EncodingValue::Scalar {
                value: stroke_width,
            },
            corner_radius: EncodingValue::Scalar {
                value: self.corner_radius.unwrap_or(0.0),
            },
            indices: None,
        })
    }
}
