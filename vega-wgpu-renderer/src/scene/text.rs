use crate::error::VegaWgpuError;
use crate::scene::value::EncodingValue;
use crate::specs::mark::MarkContainerSpec;
use crate::specs::text::{FontStyleSpec, FontWeightNameSpec, FontWeightSpec, TextAlignSpec, TextBaselineSpec, TextItemSpec};

use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all="kebab-case")]
pub struct TextMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub text: EncodingValue<String>,
    pub x: EncodingValue<f32>,
    pub y: EncodingValue<f32>,
    pub align: EncodingValue<TextAlignSpec>,
    pub baseline: EncodingValue<TextBaselineSpec>,
    pub opacity: EncodingValue<f32>,
    pub angle: EncodingValue<f32>,
    pub color: EncodingValue<[f32; 3]>,
    pub dx: EncodingValue<f32>,
    pub dy: EncodingValue<f32>,
    pub font: EncodingValue<String>,
    pub font_size: EncodingValue<f32>,
    pub font_weight: EncodingValue<FontWeightSpec>,
    pub font_style: EncodingValue<FontStyleSpec>,
    pub limit: EncodingValue<f32>,
}

impl TextMark {
    pub fn text_iter(&self) -> Box<dyn Iterator<Item=&String> + '_> {
        self.text.as_iter(self.len as usize)
    }
    pub fn x_iter(&self) -> Box<dyn Iterator<Item=&f32> + '_> {
        self.x.as_iter(self.len as usize)
    }
    pub fn y_iter(&self) -> Box<dyn Iterator<Item=&f32> + '_> {
        self.y.as_iter(self.len as usize)
    }
    pub fn align_iter(&self) -> Box<dyn Iterator<Item=&TextAlignSpec> + '_> {
        self.align.as_iter(self.len as usize)
    }
    pub fn baseline_iter(&self) -> Box<dyn Iterator<Item=&TextBaselineSpec> + '_> {
        self.baseline.as_iter(self.len as usize)
    }
    pub fn opacity_iter(&self) -> Box<dyn Iterator<Item=&f32> + '_> {
        self.opacity.as_iter(self.len as usize)
    }
    pub fn angle_iter(&self) -> Box<dyn Iterator<Item=&f32> + '_> {
        self.angle.as_iter(self.len as usize)
    }
    pub fn color_iter(&self) -> Box<dyn Iterator<Item=&[f32; 3]> + '_> {
        self.color.as_iter(self.len as usize)
    }
    pub fn dx_iter(&self) -> Box<dyn Iterator<Item=&f32> + '_> {
        self.dx.as_iter(self.len as usize)
    }
    pub fn dy_iter(&self) -> Box<dyn Iterator<Item=&f32> + '_> {
        self.dx.as_iter(self.len as usize)
    }
    pub fn font_iter(&self) -> Box<dyn Iterator<Item=&String> + '_> {
        self.font.as_iter(self.len as usize)
    }
    pub fn font_size_iter(&self) -> Box<dyn Iterator<Item=&f32> + '_> {
        self.font_size.as_iter(self.len as usize)
    }
    pub fn font_weight_iter(&self) -> Box<dyn Iterator<Item=&FontWeightSpec> + '_> {
        self.font_weight.as_iter(self.len as usize)
    }
    pub fn font_style_iter(&self) -> Box<dyn Iterator<Item=&FontStyleSpec> + '_> {
        self.font_style.as_iter(self.len as usize)
    }
    pub fn limit_iter(&self) -> Box<dyn Iterator<Item=&f32> + '_> {
        self.limit.as_iter(self.len as usize)
    }
}

impl Default for TextMark {
    fn default() -> Self {
        Self {
            name: "text_mark".to_string(),
            clip: true,
            len: 1,
            text: EncodingValue::Scalar { value: String::new() },
            x: EncodingValue::Scalar { value: 0.0 },
            y: EncodingValue::Scalar { value: 0.0 },
            align: EncodingValue::Scalar { value: TextAlignSpec::Left },
            baseline: EncodingValue::Scalar { value: TextBaselineSpec::Bottom },
            opacity: EncodingValue::Scalar { value: 1.0 },
            angle: EncodingValue::Scalar { value: 0.0 },
            color: EncodingValue::Scalar { value: [0.0, 0.0, 0.0] },
            dx: EncodingValue::Scalar { value: 0.0 },
            dy: EncodingValue::Scalar { value: 0.0 },
            font: EncodingValue::Scalar { value: "sans serif".to_string() },
            font_size: EncodingValue::Scalar { value: 10.0 },
            font_weight: EncodingValue::Scalar { value: FontWeightSpec::Name(FontWeightNameSpec::Normal) },
            font_style: EncodingValue::Scalar { value: FontStyleSpec::Normal },
            limit: EncodingValue::Scalar { value: 0.0 },
        }
    }
}

impl TextMark {
    pub fn from_spec(
        spec: &MarkContainerSpec<TextItemSpec>,
        origin: [f32; 2],
    ) -> Result<Self, VegaWgpuError> {

        // Init mark with scalar defaults
        let mut mark = TextMark::default();
        if let Some(name) = &spec.name {
            mark.name = name.clone();
        }

        // propagate clip
        mark.clip = spec.clip;

        // Init vector for each encoding channel
        let mut text = Vec::<String>::new();
        let mut x = Vec::<f32>::new();
        let mut y = Vec::<f32>::new();
        let mut align = Vec::<TextAlignSpec>::new();
        let mut baseline = Vec::<TextBaselineSpec>::new();
        let mut opacity = Vec::<f32>::new();
        let mut angle = Vec::<f32>::new();
        let mut color = Vec::<[f32; 3]>::new();
        let mut dx = Vec::<f32>::new();
        let mut dy = Vec::<f32>::new();
        let mut font = Vec::<String>::new();
        let mut font_size = Vec::<f32>::new();
        let mut font_weight = Vec::<FontWeightSpec>::new();
        let mut font_style = Vec::<FontStyleSpec>::new();
        let mut limit = Vec::<f32>::new();

        for item in &spec.items {
            x.push(item.x + origin[0]);
            y.push(item.y + origin[1]);
            text.push(item.text.clone());

            if let Some(v) = item.align {
                align.push(v);
            }

            if let Some(v) = item.baseline {
                baseline.push(v);
            }

            if let Some(v) = item.fill_opacity {
                opacity.push(v);
            }

            if let Some(v) = item.angle {
                angle.push(v);
            }

            if let Some(v) = &item.fill {
                let c = csscolorparser::parse(v)?;
                color.push([c.r as f32, c.g as f32, c.b as f32])
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
        if text.len() == len {
            mark.text = EncodingValue::Array {values: text };
        }
        if align.len() == len {
            mark.align = EncodingValue::Array {values: align };
        }
        if baseline.len() == len {
            mark.baseline = EncodingValue::Array {values: baseline };
        }
        if opacity.len() == len {
            mark.opacity = EncodingValue::Array {values: opacity };
        }
        if angle.len() == len {
            mark.angle = EncodingValue::Array {values: angle };
        }
        if color.len() == len {
            mark.color = EncodingValue::Array {values: color };
        }
        if dx.len() == len {
            mark.dx = EncodingValue::Array {values: dx };
        }
        if dy.len() == len {
            mark.dy = EncodingValue::Array {values: dy };
        }
        if font.len() == len {
            mark.font = EncodingValue::Array {values: font };
        }
        if font_size.len() == len {
            mark.font_size = EncodingValue::Array {values: font_size };
        }
        if font_weight.len() == len {
            mark.font_weight = EncodingValue::Array {values: font_weight };
        }
        if font_style.len() == len {
            mark.font_style = EncodingValue::Array {values: font_style };
        }
        if limit.len() == len {
            mark.limit = EncodingValue::Array {values: limit };
        }
        Ok(mark)
    }
}