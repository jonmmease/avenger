#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Default, Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum TextAlignSpec {
    #[default]
    Left,
    Center,
    Right,
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Default, Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub enum TextBaselineSpec {
    Alphabetic,
    Top,
    Middle,
    #[default]
    Bottom,
    LineTop,
    LineBottom,
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", serde(untagged))]
pub enum FontWeightSpec {
    Name(FontWeightNameSpec),
    Number(f32),
}

impl Default for FontWeightSpec {
    fn default() -> Self {
        Self::Name(FontWeightNameSpec::Normal)
    }
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Default, Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum FontWeightNameSpec {
    #[default]
    Normal,
    Bold,
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Default, Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum FontStyleSpec {
    #[default]
    Normal,
    Italic,
}
