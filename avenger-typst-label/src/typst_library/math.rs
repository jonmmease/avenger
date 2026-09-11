pub(crate) mod call;
pub(crate) mod item;

use super::{Color, FontWeight, MathFontSpec};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MathStyle {
    pub font: MathFontSpec,
    pub font_size: f32,
    pub fill: Color,
    pub font_weight: FontWeight,
}

impl Default for MathStyle {
    fn default() -> Self {
        Self {
            font: MathFontSpec::LeteSansMath,
            font_size: 12.0,
            fill: Color::BLACK,
            font_weight: FontWeight::Normal,
        }
    }
}
