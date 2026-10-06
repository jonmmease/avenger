//! Glyph outlines, cached per font instance.
//!
//! avenger: lowering a frame to drawing items outlines its glyphs, which upstream's exporters
//! do through ttf-parser directly. A label's instance caches each glyph's outline instead, so
//! that lowering a label again doesn't outline its glyphs again.

use std::sync::Arc;

/// A glyph's outline, in font units with y pointing up.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GlyphOutline(pub Vec<OutlineSegment>);

/// A segment of a glyph's outline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutlineSegment {
    Move(f32, f32),
    Line(f32, f32),
    Quad(f32, f32, f32, f32),
    Cubic(f32, f32, f32, f32, f32, f32),
    Close,
}

impl GlyphOutline {
    /// Outlines a glyph of a face, if it has an outline.
    pub(super) fn new(ttf: &ttf_parser::Face, id: u16) -> Option<Arc<Self>> {
        let mut outline = Self::default();
        ttf.outline_glyph(ttf_parser::GlyphId(id), &mut outline)?;
        Some(Arc::new(outline))
    }
}

impl ttf_parser::OutlineBuilder for GlyphOutline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.push(OutlineSegment::Move(x, y));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.0.push(OutlineSegment::Line(x, y));
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.0.push(OutlineSegment::Quad(x1, y1, x, y));
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0.push(OutlineSegment::Cubic(x1, y1, x2, y2, x, y));
    }

    fn close(&mut self) {
        self.0.push(OutlineSegment::Close);
    }
}
