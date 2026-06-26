use std::{ops::Range, sync::Arc};

use crate::paths::{MathStroke, MathTransform};
use crate::style::Color;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MathFontResourceId(pub u32);

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MathFontResource {
    pub id: MathFontResourceId,
    pub family: String,
    pub postscript_name: Option<String>,
    pub face_index: u32,
    pub units_per_em: f32,
    pub variations: Vec<MathFontVariation>,
    #[cfg_attr(feature = "serde", serde(skip, default = "empty_font_data"))]
    pub data: Arc<[u8]>,
}

#[cfg(feature = "serde")]
fn empty_font_data() -> Arc<[u8]> {
    Arc::<[u8]>::from([])
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MathFontVariation {
    pub tag: [u8; 4],
    pub value: f32,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MathPdfTextLayer {
    pub logical_width: f32,
    pub logical_height: f32,
    pub semantic_text: String,
    pub glyph_runs: Vec<MathPdfGlyphRun>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MathPdfGlyphRun {
    pub font: MathFontResourceId,
    pub font_size: f32,
    pub fill: Color,
    pub stroke: Option<MathStroke>,
    pub text: String,
    pub glyphs: Vec<MathPdfGlyph>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MathPdfGlyph {
    pub glyph_id: u16,
    pub unicode: String,
    pub text_range: Range<usize>,
    pub x: f32,
    pub y: f32,
    pub x_advance: f32,
    pub y_advance: f32,
    pub transform: MathTransform,
}
