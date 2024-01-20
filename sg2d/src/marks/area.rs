use serde::{Deserialize, Serialize};
use crate::marks::value::{EncodingValue, StrokeCap, StrokeJoin};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct AreaMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub orientation: AreaOrientation,
    pub x: EncodingValue<f32>,
    pub y: EncodingValue<f32>,
    pub x2: EncodingValue<f32>,
    pub y2: EncodingValue<f32>,
    pub defined: EncodingValue<bool>,
    pub fill: [f32; 4],
    pub stroke: [f32; 4],
    pub stroke_width: f32,
    pub stroke_cap: StrokeCap,
    pub stroke_join: StrokeJoin,
    pub stroke_dash: Option<Vec<f32>>,
}

impl AreaMark {
    pub fn x_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x.as_iter(self.len as usize, None)
    }

    pub fn y_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y.as_iter(self.len as usize, None)
    }

    pub fn x2_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x2.as_iter(self.len as usize, None)
    }

    pub fn y2_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y2.as_iter(self.len as usize, None)
    }

    pub fn defined_iter(&self) -> Box<dyn Iterator<Item = &bool> + '_> {
        self.defined.as_iter(self.len as usize, None)
    }
}

impl Default for AreaMark {
    fn default() -> Self {
        Self {
            name: "area_mark".to_string(),
            clip: true,
            len: 1,
            orientation: Default::default(),
            x: EncodingValue::Scalar { value: 0.0 },
            y: EncodingValue::Scalar { value: 0.0 },
            x2: EncodingValue::Scalar { value: 0.0 },
            y2: EncodingValue::Scalar { value: 0.0 },
            defined: EncodingValue::Scalar { value: true },
            fill: [0.0, 0.0, 0.0, 0.0],
            stroke: [0.0, 0.0, 0.0, 0.0],
            stroke_width: 1.0,
            stroke_cap: Default::default(),
            stroke_join: Default::default(),
            stroke_dash: None,
        }
    }
}

#[derive(Default, Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AreaOrientation {
    #[default]
    Vertical,
    Horizontal,
}
