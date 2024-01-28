use crate::marks::value::{ColorOrGradient, EncodingValue, Gradient};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct RectMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub gradients: Vec<Gradient>,
    pub x: EncodingValue<f32>,
    pub y: EncodingValue<f32>,
    pub width: EncodingValue<f32>,
    pub height: EncodingValue<f32>,
    pub fill: EncodingValue<ColorOrGradient>,
    pub stroke: EncodingValue<ColorOrGradient>,
    pub stroke_width: EncodingValue<f32>,
    pub corner_radius: EncodingValue<f32>,
    pub indices: Option<Vec<usize>>,
}

impl RectMark {
    pub fn x_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn y_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn width_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.width.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn height_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.height
            .as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn fill_iter(&self) -> Box<dyn Iterator<Item = &ColorOrGradient> + '_> {
        self.fill.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn stroke_iter(&self) -> Box<dyn Iterator<Item = &ColorOrGradient> + '_> {
        self.stroke
            .as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn stroke_width_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.stroke_width
            .as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn corner_radius_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.corner_radius
            .as_iter(self.len as usize, self.indices.as_ref())
    }
}

impl Default for RectMark {
    fn default() -> Self {
        Self {
            name: "rule_mark".to_string(),
            clip: true,
            len: 1,
            gradients: vec![],
            x: EncodingValue::Scalar { value: 0.0 },
            y: EncodingValue::Scalar { value: 0.0 },
            width: EncodingValue::Scalar { value: 0.0 },
            height: EncodingValue::Scalar { value: 0.0 },
            fill: EncodingValue::Scalar {
                value: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]),
            },
            stroke: EncodingValue::Scalar {
                value: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]),
            },
            stroke_width: EncodingValue::Scalar { value: 0.0 },
            corner_radius: EncodingValue::Scalar { value: 0.0 },
            indices: None,
        }
    }
}
