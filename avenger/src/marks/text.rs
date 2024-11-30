use crate::marks::value::EncodingValue;
use itertools::izip;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct TextMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub indices: Option<Vec<usize>>,
    pub zindex: Option<i32>,

    // Encodings
    pub text: EncodingValue<String>,
    pub x: EncodingValue<f32>,
    pub y: EncodingValue<f32>,
    pub align: EncodingValue<TextAlignSpec>,
    pub baseline: EncodingValue<TextBaselineSpec>,
    pub angle: EncodingValue<f32>,
    pub color: EncodingValue<[f32; 4]>,
    pub font: EncodingValue<String>,
    pub font_size: EncodingValue<f32>,
    pub font_weight: EncodingValue<FontWeightSpec>,
    pub font_style: EncodingValue<FontStyleSpec>,
    pub limit: EncodingValue<f32>,
}

impl TextMark {
    pub fn instances(&self) -> Box<dyn Iterator<Item = TextMarkInstance> + '_> {
        let n = self.len as usize;
        let inds = self.indices.as_ref();
        Box::new(
            izip!(
                self.text.as_iter(n, inds),
                self.x.as_iter(n, inds),
                self.y.as_iter(n, inds),
                self.align.as_iter(n, inds),
                self.baseline.as_iter(n, inds),
                self.angle.as_iter(n, inds),
                self.color.as_iter(n, inds),
                self.font.as_iter(n, inds),
                self.font_size.as_iter(n, inds),
                self.font_weight.as_iter(n, inds),
                self.font_style.as_iter(n, inds),
                self.limit.as_iter(n, inds)
            )
            .map(
                |(
                    text,
                    x,
                    y,
                    align,
                    baseline,
                    angle,
                    color,
                    font,
                    font_size,
                    font_weight,
                    font_style,
                    limit,
                )| {
                    TextMarkInstance {
                        text: text.clone(),
                        x: *x,
                        y: *y,
                        align: *align,
                        baseline: *baseline,
                        angle: *angle,
                        color: *color,
                        font: font.clone(),
                        font_size: *font_size,
                        font_weight: *font_weight,
                        font_style: *font_style,
                        limit: *limit,
                    }
                },
            ),
        )
    }
}

impl Default for TextMark {
    fn default() -> Self {
        let default_instance = TextMarkInstance::default();
        Self {
            name: "text_mark".to_string(),
            clip: true,
            len: 1,
            text: EncodingValue::Scalar {
                value: default_instance.text,
            },
            x: EncodingValue::Scalar {
                value: default_instance.x,
            },
            y: EncodingValue::Scalar {
                value: default_instance.y,
            },
            align: EncodingValue::Scalar {
                value: default_instance.align,
            },
            baseline: EncodingValue::Scalar {
                value: default_instance.baseline,
            },
            angle: EncodingValue::Scalar {
                value: default_instance.angle,
            },
            color: EncodingValue::Scalar {
                value: default_instance.color,
            },
            font: EncodingValue::Scalar {
                value: default_instance.font,
            },
            font_size: EncodingValue::Scalar {
                value: default_instance.font_size,
            },
            font_weight: EncodingValue::Scalar {
                value: default_instance.font_weight,
            },
            font_style: EncodingValue::Scalar {
                value: default_instance.font_style,
            },
            limit: EncodingValue::Scalar {
                value: default_instance.limit,
            },
            indices: None,
            zindex: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextMarkInstance {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub align: TextAlignSpec,
    pub baseline: TextBaselineSpec,
    pub angle: f32,
    pub color: [f32; 4],
    pub font: String,
    pub font_size: f32,
    pub font_weight: FontWeightSpec,
    pub font_style: FontStyleSpec,
    pub limit: f32,
}

impl Default for TextMarkInstance {
    fn default() -> Self {
        Self {
            text: String::new(),
            x: 0.0,
            y: 0.0,
            align: TextAlignSpec::Left,
            baseline: TextBaselineSpec::Alphabetic,
            angle: 0.0,
            color: [0.0, 0.0, 0.0, 1.0],
            font: "sans serif".to_string(),
            font_size: 10.0,
            font_weight: FontWeightSpec::Name(FontWeightNameSpec::Normal),
            font_style: FontStyleSpec::Normal,
            limit: 0.0,
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
