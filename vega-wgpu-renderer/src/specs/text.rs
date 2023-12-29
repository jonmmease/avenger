use crate::specs::mark::MarkItemSpec;
use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextItemSpec {
    // Required
    pub x: f32,
    pub y: f32,
    pub text: String,

    // Optional
    pub align: Option<TextAlignSpec>,
    pub angle: Option<f32>,
    pub baseline: Option<TextBaselineSpec>,
    pub dx: Option<f32>,
    pub dy: Option<f32>,
    pub fill: Option<String>,
    pub fill_opacity: Option<f32>,
    pub font: Option<String>,
    pub font_size: Option<f32>,
    pub font_weight: Option<FontWeightSpec>,
    pub font_style: Option<FontStyleSpec>,
    pub limit: Option<f32>,
}

impl MarkItemSpec for TextItemSpec {}

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
