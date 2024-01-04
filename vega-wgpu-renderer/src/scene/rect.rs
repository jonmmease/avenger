use crate::error::VegaWgpuError;
use crate::scene::value::EncodingValue;
use crate::specs::mark::MarkContainerSpec;
use crate::specs::rect::RectItemSpec;

use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all="kebab-case")]
pub struct RectMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub x: EncodingValue<f32>,
    pub y: EncodingValue<f32>,
    pub width: EncodingValue<f32>,
    pub height: EncodingValue<f32>,
    pub fill: EncodingValue<[f32; 3]>,
}

impl RectMark {
    pub fn x_iter(&self) -> Box<dyn Iterator<Item=&f32> + '_> {
        self.x.as_iter(self.len as usize)
    }

    pub fn y_iter(&self) -> Box<dyn Iterator<Item=&f32> + '_> {
        self.y.as_iter(self.len as usize)
    }

    pub fn width_iter(&self) -> Box<dyn Iterator<Item=&f32> + '_> {
        self.width.as_iter(self.len as usize)
    }

    pub fn height_iter(&self) -> Box<dyn Iterator<Item=&f32> + '_> {
        self.height.as_iter(self.len as usize)
    }

    pub fn fill_iter(&self) -> Box<dyn Iterator<Item=&[f32; 3]> + '_> {
        self.fill.as_iter(self.len as usize)
    }
}

impl Default for RectMark {
    fn default() -> Self {
        Self {
            name: "rule_mark".to_string(),
            clip: true,
            len: 1,
            x: EncodingValue::Scalar { value: 0.0 },
            y: EncodingValue::Scalar { value: 0.0 },
            width: EncodingValue::Scalar { value: 0.0 },
            height: EncodingValue::Scalar { value: 0.0 },
            fill: EncodingValue::Scalar { value: [0.0, 0.0, 0.0] },
        }
    }
}


impl RectMark {
    pub fn from_spec(
        spec: &MarkContainerSpec<RectItemSpec>,
        origin: [f32; 2],
    ) -> Result<Self, VegaWgpuError> {

        // Init mark with scalar defaults
        let mut mark = RectMark::default();
        if let Some(name) = &spec.name {
            mark.name = name.clone();
        }

        // propagate clip
        mark.clip = spec.clip;

        // Init vector for each encoding channel
        let mut x = Vec::<f32>::new();
        let mut y = Vec::<f32>::new();
        let mut width = Vec::<f32>::new();
        let mut height = Vec::<f32>::new();
        let mut fill = Vec::<[f32; 3]>::new();

        // For each item, append explicit values to corresponding vector
        for item in &spec.items {
            x.push(item.x + origin[0]);
            y.push(item.y + origin[1]);
            if let Some(v) = item.width {
                width.push(v);
            }
            if let Some(v) = item.height {
                height.push(v);
            }
            if let Some(v) = &item.fill {
                let c = csscolorparser::parse(v)?;
                fill.push([c.r as f32, c.g as f32, c.b as f32])
            }
        }

        // Override values with vectors
        let len = spec.items.len();
        mark.len = len as u32;

        if x.len() == len {
            mark.x = EncodingValue::Array {values: x };
        }
        if y.len() == len {
            mark.y = EncodingValue::Array {values: y };
        }
        if width.len() == len {
            mark.width = EncodingValue::Array {values: width };
        }
        if height.len() == len {
            mark.height = EncodingValue::Array {values: height };
        }
        if fill.len() == len {
            mark.fill = EncodingValue::Array {values: fill };
        }

        Ok(mark)
    }
}
