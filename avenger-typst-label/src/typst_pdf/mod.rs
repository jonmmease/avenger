//! Lowers compiled labels to PDF drawing items: text as glyph runs in their fonts, so that it
//! stays text, bitmap glyphs as images, and shapes as paths.
//!
//! avenger: drawing items in place of a PDF document, which `avenger-pdf` writes, embedding
//! the fonts.

use std::ops::Range;

use avenger_color::AbsoluteColor;

use crate::label::{CompiledLabel, FontRef, FrameItem, Point, Size, TextItem, Transform};
use crate::typst_svg::{GlyphRef, ImageItem, PathItem, bitmap, shape_path};

/// Options for lowering a label to PDF drawing items.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PdfOptions {}

/// A label as PDF drawing items.
#[derive(Debug, Clone, PartialEq)]
pub struct PdfLabel {
    /// The label's size.
    pub size: Size,
    /// The label's text, for text extraction.
    pub semantic_text: String,
    /// The fonts the label's text uses.
    pub fonts: Vec<FontRef>,
    /// The items, in drawing order.
    pub items: Vec<PdfItem>,
}

/// A PDF drawing item.
#[derive(Debug, Clone, PartialEq)]
pub enum PdfItem {
    /// A run of glyphs in one font.
    Text(PdfText),
    /// A filled or stroked path.
    Path(PathItem),
    /// A bitmap glyph.
    Image(ImageItem),
}

/// A run of glyphs in one font, size and fill.
#[derive(Debug, Clone, PartialEq)]
pub struct PdfText {
    /// The font, as an index into the label's fonts.
    pub font: usize,
    /// The font size, in points.
    pub size: f32,
    /// The glyphs' fill.
    pub fill: AbsoluteColor,
    /// The transform from the run's coordinates to the label's. The run's origin lies on its
    /// baseline.
    pub transform: Transform,
    /// The text the glyphs show.
    pub text: String,
    /// The glyphs, in visual order.
    pub glyphs: Vec<PdfGlyph>,
}

/// A glyph in a run.
#[derive(Debug, Clone, PartialEq)]
pub struct PdfGlyph {
    /// The glyph's index in the font.
    pub id: u16,
    /// The glyph's origin relative to the run's, in points.
    pub position: Point,
    /// The horizontal advance, in points.
    pub x_advance: f32,
    /// The glyph's cluster in the run's text.
    pub range: Range<usize>,
    /// The glyph's range in the label source.
    pub source: Range<usize>,
}

/// Lowers a label to PDF drawing items.
pub fn pdf_items(label: &CompiledLabel, _options: &PdfOptions) -> PdfLabel {
    let mut fonts: Vec<FontRef> = vec![];
    let mut items = vec![];
    let mut texts = 0;
    label.frame.visit(Transform::IDENTITY, &mut |ts, item| match item {
        FrameItem::Text(text) => {
            let font = match fonts.iter().position(|font| *font == text.font) {
                Some(index) => index,
                None => {
                    fonts.push(text.font.clone());
                    fonts.len() - 1
                }
            };
            draw_text(&mut items, ts, text, font, texts);
            texts += 1;
        }
        FrameItem::Shape(shape) => items.push(PdfItem::Path(shape_path(ts, shape))),
        FrameItem::Group(_) => {}
    });
    PdfLabel {
        size: label.frame.size,
        semantic_text: label.semantic_text(),
        fonts,
        items,
    }
}

/// Draws a text item: its glyphs as a run, except bitmap glyphs, which follow as images.
fn draw_text(
    items: &mut Vec<PdfItem>,
    ts: Transform,
    text: &TextItem,
    font: usize,
    index: usize,
) {
    let instance = &text.font.0;
    let scale = text.size / instance.units_per_em() as f32;
    let mut glyphs = vec![];
    let mut images = vec![];
    for (glyph_index, (pos, glyph)) in text.positioned_glyphs().enumerate() {
        if let Some(image) = bitmap(instance, glyph.id) {
            images.push(PdfItem::Image(ImageItem {
                data: image.data,
                size: Size::new(image.size.x * scale, image.size.y * scale),
                transform: ts.pre_concat(Transform::translate(
                    pos.x + image.pos.x * scale,
                    pos.y + image.pos.y * scale,
                )),
                glyph: GlyphRef {
                    text: index,
                    glyph: glyph_index,
                    source: glyph.source.clone(),
                },
            }));
            continue;
        }
        glyphs.push(PdfGlyph {
            id: glyph.id,
            position: pos,
            x_advance: glyph.x_advance * text.size,
            range: glyph.range.clone(),
            source: glyph.source.clone(),
        });
    }
    if !glyphs.is_empty() {
        items.push(PdfItem::Text(PdfText {
            font,
            size: text.size,
            fill: text.fill,
            transform: ts,
            text: text.text.clone(),
            glyphs,
        }));
    }
    items.extend(images);
}
