//! The outline of a label: closed polygons that cover its glyphs and its shapes, which an effect
//! that follows the shapes of text, such as a halo, grows.

use i_overlay::core::{
    fill_rule::FillRule as OverlayFillRule, overlay_rule::OverlayRule,
};
use i_overlay::float::overlay::FloatOverlay;
use i_overlay::mesh::{
    outline::offset::OutlineOffset,
    style::{LineJoin as OutlineJoin, OutlineStyle},
};
use kurbo::{Affine, BezPath, Cap, Join, PathEl, Rect, Shape as _, StrokeOpts};
use ttf_parser::{GlyphId, OutlineBuilder};

use super::engine::CompiledLabel;
use super::frame::{
    Curve, CurveItem, FillRule, FrameItem, Geometry, Glyph, LineCap, LineJoin, Point,
    Shape, Stroke, TextItem, Transform,
};

/// The outline of a label: closed polygons in the frame of its box, with the box's top left at 0
/// and y down, that cover its glyphs, its fills, and its strokes. The polygons of glyphs and of
/// nonzero fills keep their winding, and the other polygons wind so that holes cancel, so the
/// nonzero rule fills them all as the label draws. A glyph without an outline, such as a color
/// emoji, is its bounding box.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LabelOutline {
    pub contours: Vec<Vec<[f32; 2]>>,
}

impl LabelOutline {
    /// The region within `width` of the region that the outline fills under the nonzero rule,
    /// with round corners: disjoint polygons and their holes, which either fill rule fills alike.
    pub(crate) fn expand(&self, width: f32) -> LabelOutline {
        if self.contours.is_empty() {
            return LabelOutline::default();
        }
        let style = OutlineStyle::new(width).line_join(OutlineJoin::Round(JOIN_RATIO));
        let filled = FloatOverlay::with_subj(&self.contours)
            .overlay(OverlayRule::Subject, OverlayFillRule::NonZero);
        let grown = filled.outline(&style);
        LabelOutline { contours: grown.into_iter().flatten().collect() }
    }
}

/// How finely an expanded outline's round corners follow their circles: the longest segment of a
/// corner over its radius.
const JOIN_RATIO: f32 = 0.25;

/// How far, in pixels, a flattened curve may stray from the curve.
const TOLERANCE: f64 = 0.05;

/// The outline of `compiled`, whose frame stands `top` below the top of its box.
pub(crate) fn label_outline(compiled: &CompiledLabel, top: f32) -> LabelOutline {
    let mut contours = Vec::new();
    let shift = Transform::translate(0.0, top);
    compiled.frame.visit(shift, &mut |transform, item| match item {
        FrameItem::Text(text) => {
            for (origin, glyph) in text.positioned_glyphs() {
                let path = match glyph_path(text, origin, glyph) {
                    Some(path) => path,
                    None => {
                        let Some(rect) =
                            text.font.0.ttf().glyph_bounding_box(GlyphId(glyph.id))
                        else {
                            continue;
                        };
                        let rect = Rect::new(
                            f64::from(rect.x_min),
                            f64::from(rect.y_min),
                            f64::from(rect.x_max),
                            f64::from(rect.y_max),
                        );
                        placement(text, origin) * rect.to_path(TOLERANCE)
                    }
                };
                flatten(&(affine(transform) * path), &mut contours);
            }
        }
        FrameItem::Shape(shape) => shape_outline(shape, affine(transform), &mut contours),
        FrameItem::Group(_) => {}
    });
    LabelOutline { contours }
}

/// The outline of `glyph` of `text`, whose origin lies at `origin` in the item, in the item's
/// coordinates, or `None` for a glyph without one, such as a color emoji's.
pub(crate) fn glyph_path(
    text: &TextItem,
    origin: Point,
    glyph: &Glyph,
) -> Option<BezPath> {
    let mut path = GlyphPath(BezPath::new());
    text.font.0.ttf().outline_glyph(GlyphId(glyph.id), &mut path)?;
    Some(placement(text, origin) * path.0)
}

/// What takes a glyph of `text` from font units, with y up, to its origin at `origin` in the item.
fn placement(text: &TextItem, origin: Point) -> Affine {
    let scale = f64::from(text.size / text.font.units_per_em());
    Affine::new([scale, 0.0, 0.0, -scale, f64::from(origin.x), f64::from(origin.y)])
}

/// Appends the contours that cover `shape`'s fill and stroke, through `transform`.
fn shape_outline(shape: &Shape, transform: Affine, contours: &mut Vec<Vec<[f32; 2]>>) {
    let path = match &shape.geometry {
        Geometry::Line(to) => {
            let mut path = BezPath::new();
            path.move_to((0.0, 0.0));
            path.line_to((f64::from(to.x), f64::from(to.y)));
            path
        }
        Geometry::Rect(size) => {
            Rect::new(0.0, 0.0, f64::from(size.x), f64::from(size.y)).to_path(TOLERANCE)
        }
        Geometry::Curve(curve) => bez_path(curve),
    };
    // A line has no inside to fill.
    if shape.fill.is_some() && !matches!(shape.geometry, Geometry::Line(_)) {
        let filled = transform * path.clone();
        match shape.fill_rule {
            FillRule::NonZero => flatten(&filled, contours),
            FillRule::EvenOdd => {
                let mut own = Vec::new();
                flatten(&filled, &mut own);
                let resolved = FloatOverlay::with_subj(&own)
                    .overlay(OverlayRule::Subject, OverlayFillRule::EvenOdd);
                contours.extend(resolved.into_iter().flatten());
            }
        }
    }
    if let Some(stroke) = shape.stroke.as_ref().filter(|stroke| stroke.thickness > 0.0) {
        let stroked =
            kurbo::stroke(path, &kurbo_stroke(stroke), &StrokeOpts::default(), TOLERANCE);
        flatten(&(transform * stroked), contours);
    }
}

/// The kurbo path of `curve`.
pub(crate) fn bez_path(curve: &Curve) -> BezPath {
    let point = |point: Point| (f64::from(point.x), f64::from(point.y));
    let mut path = BezPath::new();
    for item in &curve.0 {
        match *item {
            CurveItem::Move(to) => path.move_to(point(to)),
            CurveItem::Line(to) => path.line_to(point(to)),
            CurveItem::Quad(control, to) => path.quad_to(point(control), point(to)),
            CurveItem::Cubic(a, b, to) => path.curve_to(point(a), point(b), point(to)),
            CurveItem::Close => path.close_path(),
        }
    }
    path
}

/// The kurbo style of `stroke`. A dash pattern without length draws solid, as the outputs do.
fn kurbo_stroke(stroke: &Stroke) -> kurbo::Stroke {
    let style = kurbo::Stroke::new(f64::from(stroke.thickness))
        .with_caps(match stroke.cap {
            LineCap::Butt => Cap::Butt,
            LineCap::Round => Cap::Round,
            LineCap::Square => Cap::Square,
        })
        .with_join(match stroke.join {
            LineJoin::Miter => Join::Miter,
            LineJoin::Round => Join::Round,
            LineJoin::Bevel => Join::Bevel,
        })
        .with_miter_limit(f64::from(stroke.miter_limit));
    match &stroke.dash {
        Some(dash)
            if dash.array.iter().all(|length| length.is_finite() && *length >= 0.0)
                && dash.array.iter().sum::<f32>() > 0.0 =>
        {
            let pattern = dash.array.iter().map(|length| f64::from(*length));
            style.with_dashes(f64::from(dash.phase), pattern)
        }
        _ => style,
    }
}

/// The kurbo transformation of `transform`.
fn affine(transform: Transform) -> Affine {
    let Transform { sx, ky, kx, sy, tx, ty } = transform;
    Affine::new([sx, ky, kx, sy, tx, ty].map(f64::from))
}

/// Appends `path`, flattened to lines within [`TOLERANCE`], as contours that close implicitly.
fn flatten(path: &BezPath, contours: &mut Vec<Vec<[f32; 2]>>) {
    let mut contour = Vec::new();
    kurbo::flatten(path, TOLERANCE, |element| {
        if let PathEl::MoveTo(_) | PathEl::ClosePath = element {
            finish(&mut contour, contours);
        }
        if let PathEl::MoveTo(to) | PathEl::LineTo(to) = element {
            contour.push([to.x as f32, to.y as f32]);
        }
    });
    finish(&mut contour, contours);
}

/// Moves `contour` to `contours` if it encloses an area, without a repeat of its first point.
fn finish(contour: &mut Vec<[f32; 2]>, contours: &mut Vec<Vec<[f32; 2]>>) {
    let mut done = std::mem::take(contour);
    if done.len() > 1 && done.first() == done.last() {
        done.pop();
    }
    if done.len() >= 3 {
        contours.push(done);
    }
}

/// A glyph's outline as a path in font units, with y up.
struct GlyphPath(BezPath);

impl OutlineBuilder for GlyphPath {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((f64::from(x), f64::from(y)));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to((f64::from(x), f64::from(y)));
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.0
            .quad_to((f64::from(x1), f64::from(y1)), (f64::from(x), f64::from(y)));
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0.curve_to(
            (f64::from(x1), f64::from(y1)),
            (f64::from(x2), f64::from(y2)),
            (f64::from(x), f64::from(y)),
        );
    }

    fn close(&mut self) {
        self.0.close_path();
    }
}
