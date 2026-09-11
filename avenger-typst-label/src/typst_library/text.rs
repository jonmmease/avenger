pub(crate) mod call;
pub(crate) mod content;
pub(crate) mod smartquote;

use super::{Color, FontStyle, FontWeight};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TextStyle {
    pub font_family: String,
    pub font_size: f32,
    pub fill: Color,
    pub font_weight: FontWeight,
    pub font_style: FontStyle,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font_family: "sans-serif".to_string(),
            font_size: 12.0,
            fill: Color::BLACK,
            font_weight: FontWeight::Normal,
            font_style: FontStyle::Normal,
        }
    }
}
