use crate::error::VegaWgpuError;
use crate::scene::value::{EncodingValue, StrokeCap};
use crate::specs::mark::MarkContainerSpec;
use crate::specs::rule::RuleItemSpec;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all="kebab-case")]
pub struct RuleMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub x0: EncodingValue<f32>,
    pub y0: EncodingValue<f32>,
    pub x1: EncodingValue<f32>,
    pub y1: EncodingValue<f32>,
    pub stroke: EncodingValue<[f32; 3]>,
    pub stroke_width: EncodingValue<f32>,
    pub stroke_cap: EncodingValue<StrokeCap>,
}

impl RuleMark {
    pub fn x0_iter(&self) -> Box<dyn Iterator<Item=&f32> + '_> {
        self.x0.as_iter(self.len as usize)
    }
    pub fn y0_iter(&self) -> Box<dyn Iterator<Item=&f32> + '_> {
        self.y0.as_iter(self.len as usize)
    }
    pub fn x1_iter(&self) -> Box<dyn Iterator<Item=&f32> + '_> {
        self.x1.as_iter(self.len as usize)
    }
    pub fn y1_iter(&self) -> Box<dyn Iterator<Item=&f32> + '_> {
        self.y1.as_iter(self.len as usize)
    }
    pub fn stroke_iter(&self) -> Box<dyn Iterator<Item=&[f32; 3]> + '_> {
        self.stroke.as_iter(self.len as usize)
    }
    pub fn stroke_width_iter(&self) -> Box<dyn Iterator<Item=&f32> + '_> {
        self.stroke_width.as_iter(self.len as usize)
    }
    pub fn stroke_cap_iter(&self) -> Box<dyn Iterator<Item=&StrokeCap> + '_> {
        self.stroke_cap.as_iter(self.len as usize)
    }
}

impl Default for RuleMark {
    fn default() -> Self {
        Self {
            name: "rule_mark".to_string(),
            clip: true,
            len: 1,
            x0: EncodingValue::Scalar { value: 0.0 },
            y0: EncodingValue::Scalar { value: 0.0 },
            x1: EncodingValue::Scalar { value: 0.0 },
            y1: EncodingValue::Scalar { value: 0.0 },
            stroke: EncodingValue::Scalar { value: [0.0, 0.0, 0.0] },
            stroke_width: EncodingValue::Scalar { value: 1.0 },
            stroke_cap: EncodingValue::Scalar { value: StrokeCap::Butt },
        }
    }
}


impl RuleMark {
    pub fn from_spec(
        spec: &MarkContainerSpec<RuleItemSpec>,
        origin: [f32; 2],
    ) -> Result<Self, VegaWgpuError> {

        // Init mark with scalar defaults
        let mut mark = RuleMark::default();
        if let Some(name) = &spec.name {
            mark.name = name.clone();
        }

        // propagate clip
        mark.clip = spec.clip;

        // Init vector for each encoding channel
        let mut x0 = Vec::<f32>::new();
        let mut y0 = Vec::<f32>::new();
        let mut x1 = Vec::<f32>::new();
        let mut y1 = Vec::<f32>::new();
        let mut stroke = Vec::<[f32; 3]>::new();
        let mut stroke_width = Vec::<f32>::new();
        let mut stroke_cap = Vec::<StrokeCap>::new();

        // For each item, append explicit values to corresponding vector
        for item in &spec.items {
            x0.push(item.x + origin[0]);
            y0.push(item.y + origin[1]);
            x1.push(item.x2.unwrap_or(item.x) + origin[0]);
            y1.push(item.y2.unwrap_or(item.y) + origin[1]);

            if let Some(s) = &item.stroke {
                let c = csscolorparser::parse(s)?;
                stroke.push([c.r as f32, c.g as f32, c.b as f32])
            }

            if let Some(s) = item.stroke_width {
                stroke_width.push(s);
            }

            if let Some(s) = item.stroke_cap {
                stroke_cap.push(s);
            }
        }

        // Override values with vectors
        let len = spec.items.len();
        mark.len = len as u32;

        if x0.len() == len {
            mark.x0 = EncodingValue::Array {values: x0};
        }
        if y0.len() == len {
            mark.y0 = EncodingValue::Array {values: y0};
        }
        if x1.len() == len {
            mark.x1 = EncodingValue::Array {values: x1};
        }
        if y1.len() == len {
            mark.y1 = EncodingValue::Array {values: y1};
        }
        if stroke.len() == len {
            mark.stroke = EncodingValue::Array {values: stroke};
        }
        if stroke_width.len() == len {
            mark.stroke_width = EncodingValue::Array {values: stroke_width};
        }
        if stroke_cap.len() == len {
            mark.stroke_cap = EncodingValue::Array {values: stroke_cap};
        }

        Ok(mark)
    }
}
