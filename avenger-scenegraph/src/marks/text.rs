use crate::marks::value::ScalarOrArray;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SceneTextMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub text: ScalarOrArray<String>,
    pub x: ScalarOrArray<f32>,
    pub y: ScalarOrArray<f32>,
    pub align: ScalarOrArray<TextAlignSpec>,
    pub baseline: ScalarOrArray<TextBaselineSpec>,
    pub angle: ScalarOrArray<f32>,
    pub color: ScalarOrArray<[f32; 4]>,
    pub font: ScalarOrArray<String>,
    pub font_size: ScalarOrArray<f32>,
    pub font_weight: ScalarOrArray<FontWeightSpec>,
    pub font_style: ScalarOrArray<FontStyleSpec>,
    pub limit: ScalarOrArray<f32>,
    pub indices: Option<Vec<usize>>,
    pub zindex: Option<i32>,
}

impl SceneTextMark {
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

impl Default for SceneTextMark {
    fn default() -> Self {
        Self {
            name: "text_mark".to_string(),
            clip: true,
            len: 1,
            text: ScalarOrArray::Scalar {
                value: String::new(),
            },
            x: ScalarOrArray::Scalar { value: 0.0 },
            y: ScalarOrArray::Scalar { value: 0.0 },
            align: ScalarOrArray::Scalar {
                value: TextAlignSpec::Left,
            },
            baseline: ScalarOrArray::Scalar {
                value: TextBaselineSpec::Alphabetic,
            },
            angle: ScalarOrArray::Scalar { value: 0.0 },
            color: ScalarOrArray::Scalar {
                value: [0.0, 0.0, 0.0, 1.0],
            },
            font: ScalarOrArray::Scalar {
                value: "sans serif".to_string(),
            },
            font_size: ScalarOrArray::Scalar { value: 10.0 },
            font_weight: ScalarOrArray::Scalar {
                value: FontWeightSpec::Name(FontWeightNameSpec::Normal),
            },
            font_style: ScalarOrArray::Scalar {
                value: FontStyleSpec::Normal,
            },
            limit: ScalarOrArray::Scalar { value: 0.0 },
            indices: None,
            zindex: None,
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
