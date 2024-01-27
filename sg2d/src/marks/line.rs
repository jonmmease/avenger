use crate::marks::value::{ColorOrGradient, EncodingValue, Gradient, StrokeCap, StrokeJoin};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct LineMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub gradients: Vec<Gradient>,
    pub x: EncodingValue<f32>,
    pub y: EncodingValue<f32>,
    pub defined: EncodingValue<bool>,
    pub stroke: ColorOrGradient,
    pub stroke_width: f32,
    pub stroke_cap: StrokeCap,
    pub stroke_join: StrokeJoin,
    pub stroke_dash: Option<Vec<f32>>,
}

impl LineMark {
    pub fn x_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x.as_iter(self.len as usize, None)
    }

    pub fn y_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y.as_iter(self.len as usize, None)
    }

    pub fn defined_iter(&self) -> Box<dyn Iterator<Item = &bool> + '_> {
        self.defined.as_iter(self.len as usize, None)
    }
}

impl Default for LineMark {
    fn default() -> Self {
        Self {
            name: "line_mark".to_string(),
            clip: true,
            len: 1,
            gradients: vec![],
            x: EncodingValue::Scalar { value: 0.0 },
            y: EncodingValue::Scalar { value: 0.0 },
            defined: EncodingValue::Scalar { value: true },
            stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]),
            stroke_width: 1.0,
            stroke_cap: Default::default(),
            stroke_join: Default::default(),
            stroke_dash: None,
        }
    }
}
