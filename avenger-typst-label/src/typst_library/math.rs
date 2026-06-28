pub(crate) mod item;

use super::{Color, FontWeight, MathFontSpec};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum MathDisplayStyle {
    Inline,
    Display,
    PreserveTypstDelimiterWhitespace,
}

impl Default for MathDisplayStyle {
    fn default() -> Self {
        Self::Inline
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MathStyle {
    pub font: MathFontSpec,
    pub font_size: f32,
    pub fill: Color,
    pub font_weight: FontWeight,
    pub display_style: MathDisplayStyle,
}

impl Default for MathStyle {
    fn default() -> Self {
        Self {
            font: MathFontSpec::LeteSansMath,
            font_size: 12.0,
            fill: Color::BLACK,
            font_weight: FontWeight::Normal,
            display_style: MathDisplayStyle::Inline,
        }
    }
}
