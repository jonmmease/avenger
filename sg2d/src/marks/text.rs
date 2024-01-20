use crate::marks::value::EncodingValue;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct TextMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub text: EncodingValue<String>,
    pub x: EncodingValue<f32>,
    pub y: EncodingValue<f32>,
    pub align: EncodingValue<TextAlignSpec>,
    pub baseline: EncodingValue<TextBaselineSpec>,
    pub angle: EncodingValue<f32>,
    pub color: EncodingValue<[f32; 4]>,
    pub dx: EncodingValue<f32>,
    pub dy: EncodingValue<f32>,
    pub font: EncodingValue<String>,
    pub font_size: EncodingValue<f32>,
    pub font_weight: EncodingValue<FontWeightSpec>,
    pub font_style: EncodingValue<FontStyleSpec>,
    pub limit: EncodingValue<f32>,
    pub indices: Option<Vec<usize>>,
}

impl TextMark {
    pub fn text_iter(&self) -> Box<dyn Iterator<Item = &String> + '_> {
        self.text.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn x_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn y_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn align_iter(&self) -> Box<dyn Iterator<Item = &TextAlignSpec> + '_> {
        self.align.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn baseline_iter(&self) -> Box<dyn Iterator<Item = &TextBaselineSpec> + '_> {
        self.baseline
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn angle_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.angle.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn color_iter(&self) -> Box<dyn Iterator<Item = &[f32; 4]> + '_> {
        self.color.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn dx_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.dx.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn dy_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.dx.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn font_iter(&self) -> Box<dyn Iterator<Item = &String> + '_> {
        self.font.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn font_size_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.font_size
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn font_weight_iter(&self) -> Box<dyn Iterator<Item = &FontWeightSpec> + '_> {
        self.font_weight
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn font_style_iter(&self) -> Box<dyn Iterator<Item = &FontStyleSpec> + '_> {
        self.font_style
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn limit_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.limit.as_iter(self.len as usize, self.indices.as_ref())
    }
}

impl Default for TextMark {
    fn default() -> Self {
        Self {
            name: "text_mark".to_string(),
            clip: true,
            len: 1,
            text: EncodingValue::Scalar {
                value: String::new(),
            },
            x: EncodingValue::Scalar { value: 0.0 },
            y: EncodingValue::Scalar { value: 0.0 },
            align: EncodingValue::Scalar {
                value: TextAlignSpec::Left,
            },
            baseline: EncodingValue::Scalar {
                value: TextBaselineSpec::Bottom,
            },
            angle: EncodingValue::Scalar { value: 0.0 },
            color: EncodingValue::Scalar {
                value: [0.0, 0.0, 0.0, 1.0],
            },
            dx: EncodingValue::Scalar { value: 0.0 },
            dy: EncodingValue::Scalar { value: 0.0 },
            font: EncodingValue::Scalar {
                value: "sans serif".to_string(),
            },
            font_size: EncodingValue::Scalar { value: 10.0 },
            font_weight: EncodingValue::Scalar {
                value: FontWeightSpec::Name(FontWeightNameSpec::Normal),
            },
            font_style: EncodingValue::Scalar {
                value: FontStyleSpec::Normal,
            },
            limit: EncodingValue::Scalar { value: 0.0 },
            indices: None,
        }
    }
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextAlignSpec {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TextBaselineSpec {
    Alphabetic,
    Top,
    Middle,
    #[default]
    Bottom,
    LineTop,
    LineBottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FontWeightSpec {
    Name(FontWeightNameSpec),
    Number(f32),
}

impl Default for FontWeightSpec {
    fn default() -> Self {
        Self::Name(FontWeightNameSpec::Normal)
    }
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FontWeightNameSpec {
    #[default]
    Normal,
    Bold,
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FontStyleSpec {
    #[default]
    Normal,
    Italic,
}
