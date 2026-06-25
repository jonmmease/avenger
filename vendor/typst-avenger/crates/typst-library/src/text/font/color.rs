//! Minimal color font handling for the avenger-typst math subset.

use ttf_parser::GlyphId;

use crate::layout::{Abs, Frame, FrameItem, Point, Size};
use crate::text::FontInstance;
use crate::visualize::{FixedStroke, Geometry};
use typst_syntax::Span;

/// Whether this glyph should be rendered via simple outlining instead of via
/// `glyph_frame`.
pub fn should_outline(font: &FontInstance, glyph_id: GlyphId) -> bool {
    let ttf = font.ttf();
    ttf.tables().glyf.is_some()
        || ttf.tables().cff.is_some()
        || ttf.tables().cff2.is_some()
        || !ttf.is_color_glyph(glyph_id)
}

/// A frame that can draw a glyph.
#[derive(Clone)]
pub struct GlyphFrame {
    pub upem: Abs,
    pub item: GlyphFrameItem,
}

impl GlyphFrame {
    pub fn size(&self) -> Size {
        Size::splat(self.upem)
    }
}

impl From<GlyphFrame> for Frame {
    fn from(g: GlyphFrame) -> Self {
        let mut frame = Frame::soft(Size::splat(g.upem));
        match g.item {
            GlyphFrameItem::Tofu(pos, shape) => {
                frame.push(pos, FrameItem::Shape(shape, Span::detached()))
            }
        }
        frame
    }
}

#[derive(Clone)]
pub enum GlyphFrameItem {
    Tofu(Point, crate::visualize::Shape),
}

impl GlyphFrameItem {
    pub fn pos(&self) -> Point {
        match *self {
            GlyphFrameItem::Tofu(pos, _) => pos,
        }
    }
}

#[comemo::memoize]
pub fn glyph_frame(font: &FontInstance, glyph_id: u16) -> Option<GlyphFrame> {
    let upem = Abs::pt(font.units_per_em());
    Some(draw_fallback_tofu(font, upem, GlyphId(glyph_id)))
}

fn draw_fallback_tofu(font: &FontInstance, upem: Abs, glyph_id: GlyphId) -> GlyphFrame {
    let advance = font
        .ttf()
        .glyph_hor_advance(glyph_id)
        .map(|advance| Abs::pt(advance as f64))
        .unwrap_or(upem / 3.0);
    let inset = 0.15 * advance;
    let height = 0.7 * upem;
    let pos = Point::new(inset, upem - height);
    let size = Size::new(advance - inset * 2.0, height);
    let thickness = upem / 20.0;
    let stroke = FixedStroke { thickness, ..Default::default() };
    let shape = Geometry::Rect(size).stroked(stroke);
    GlyphFrame { upem, item: GlyphFrameItem::Tofu(pos, shape) }
}
