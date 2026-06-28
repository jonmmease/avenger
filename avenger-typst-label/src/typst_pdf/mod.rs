use std::{ops::Range, sync::Arc};

use crate::style::Color;
use crate::typst_svg::{Stroke, Transform};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct FontResourceId(pub u32);

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct FontResource {
    pub id: FontResourceId,
    pub family: String,
    pub postscript_name: Option<String>,
    pub face_index: u32,
    pub units_per_em: f32,
    pub variations: Vec<FontVariation>,
    #[cfg_attr(feature = "serde", serde(skip, default = "empty_font_data"))]
    pub data: Arc<[u8]>,
}

#[cfg(feature = "serde")]
fn empty_font_data() -> Arc<[u8]> {
    Arc::<[u8]>::from([])
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct FontVariation {
    pub tag: [u8; 4],
    pub value: f32,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PdfTextLayer {
    pub logical_width: f32,
    pub logical_height: f32,
    pub semantic_text: String,
    pub glyph_runs: Vec<PdfGlyphRun>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PdfGlyphRun {
    pub font: FontResourceId,
    pub font_size: f32,
    pub fill: Color,
    pub stroke: Option<Stroke>,
    pub text: String,
    pub glyphs: Vec<PdfGlyph>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PdfGlyph {
    pub glyph_id: u16,
    pub unicode: String,
    pub text_range: Range<usize>,
    pub x: f32,
    pub y: f32,
    pub x_advance: f32,
    pub y_advance: f32,
    pub transform: Transform,
}
