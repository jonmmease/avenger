#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const BLACK: Self = Self::rgba(0.0, 0.0, 0.0, 1.0);

    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }
}

impl Default for Color {
    fn default() -> Self {
        Self::BLACK
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum FontWeight {
    Normal,
    Bold,
    Number(u16),
}

impl Default for FontWeight {
    fn default() -> Self {
        Self::Normal
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum FontStyle {
    Normal,
    Italic,
    Oblique,
}

impl Default for FontStyle {
    fn default() -> Self {
        Self::Normal
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PlainTextStyle {
    pub font_family: String,
    pub font_size: f32,
    pub fill: Color,
    pub font_weight: FontWeight,
    pub font_style: FontStyle,
}

impl Default for PlainTextStyle {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MathFontBytesId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum MathFontSpec {
    NewComputerModernMath,
    Family(String),
    FontBytes(MathFontBytesId),
}

impl Default for MathFontSpec {
    fn default() -> Self {
        Self::NewComputerModernMath
    }
}

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
    pub display_style: MathDisplayStyle,
}

impl Default for MathStyle {
    fn default() -> Self {
        Self {
            font: MathFontSpec::NewComputerModernMath,
            font_size: 12.0,
            fill: Color::BLACK,
            display_style: MathDisplayStyle::Inline,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MathFontConfig {
    pub extra_font_families: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum MathStrictness {
    Strict,
}

impl Default for MathStrictness {
    fn default() -> Self {
        Self::Strict
    }
}
