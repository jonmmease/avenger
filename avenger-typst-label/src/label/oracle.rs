//! The upstream reference fixtures for the crate's own tests: the reader and comparison that
//! the integration tests share, and the flattening of the pipeline's internal frames.

#[path = "../../tests/common/oracle.rs"]
mod common;

pub(crate) use self::common::*;

use crate::typst_library::layout::{Frame, FrameItem};
use crate::typst_library::visualize::{
    Color, CurveItem, FixedStroke, Geometry, Paint, Shape,
};

/// Positions and metrics must agree within this many points.
pub(crate) const TOLERANCE: f64 = 1e-3;

/// Flattens a frame as [`Reference::flat`] flattens a reference. Every glyph carries its
/// source range, so the comparison checks the range wherever the reference has one: at the
/// clusters that came from source text verbatim.
pub(crate) fn flatten(frame: &Frame) -> Flat {
    let mut flat = Flat {
        width: frame.width().to_pt(),
        height: frame.height().to_pt(),
        baseline: frame.baseline().to_pt(),
        ..Flat::default()
    };
    flatten_into(frame, Affine::IDENTITY, &mut flat);
    flat
}

fn flatten_into(frame: &Frame, transform: Affine, flat: &mut Flat) {
    for (pos, item) in frame.items() {
        let at = transform.then(Affine::translate(pos.x.to_pt(), pos.y.to_pt()));
        match item {
            FrameItem::Group(group) => {
                let t = group.transform;
                let inner = at.then(Affine::new([
                    t.sx.get(),
                    t.ky.get(),
                    t.kx.get(),
                    t.sy.get(),
                    t.tx.to_pt(),
                    t.ty.to_pt(),
                ]));
                flatten_into(&group.frame, inner, flat);
            }
            FrameItem::Text(text) => {
                let font = text.font.font();
                let name =
                    font.post_script_name().unwrap_or_else(|| font.info().family.clone());
                let size = text.size.to_pt();
                let fill = paint(&text.fill);
                let mut pen = 0.0;
                for glyph in &text.glyphs {
                    let (x, y) = at.apply(
                        pen + glyph.x_offset.get() * size,
                        -glyph.y_offset.get() * size,
                    );
                    pen += glyph.x_advance.get() * size;
                    flat.glyphs.push(FlatGlyph {
                        font: name.clone(),
                        id: glyph.id,
                        size,
                        x,
                        y,
                        fill: fill.clone(),
                        source: Some([glyph.source.start, glyph.source.end]),
                    });
                }
            }
            FrameItem::Shape(shape, _) => {
                if let Some(rule) = shape_rule(&ref_shape(shape), at) {
                    flat.rules.push(rule);
                }
            }
        }
    }
}

/// A shape as the probe writes it.
fn ref_shape(shape: &Shape) -> RefShape {
    let geometry = match &shape.geometry {
        Geometry::Line(to) => RefGeometry::Line([to.x.to_pt(), to.y.to_pt()]),
        Geometry::Rect(size) => RefGeometry::Rect([size.x.to_pt(), size.y.to_pt()]),
        Geometry::Curve(curve) => {
            let pt = |p: &crate::typst_library::layout::Point| {
                serde_json::json!([p.x.to_pt(), p.y.to_pt()])
            };
            let items = curve.0.iter().map(|item| match item {
                CurveItem::Move(p) => serde_json::json!(["move", pt(p)]),
                CurveItem::Line(p) => serde_json::json!(["line", pt(p)]),
                CurveItem::Cubic(a, b, c) => {
                    serde_json::json!(["cubic", pt(a), pt(b), pt(c)])
                }
                CurveItem::Close => serde_json::json!(["close"]),
            });
            RefGeometry::Curve(items.collect())
        }
    };
    RefShape {
        geometry,
        fill: shape.fill.as_ref().map(paint),
        fill_rule: format!("{:?}", shape.fill_rule),
        stroke: shape.stroke.as_ref().map(stroke),
        span: None,
    }
}

fn stroke(stroke: &FixedStroke) -> RefStroke {
    RefStroke {
        paint: paint(&stroke.paint),
        thickness: stroke.thickness.to_pt(),
        cap: format!("{:?}", stroke.cap),
        join: format!("{:?}", stroke.join),
        dash: None,
        miter_limit: stroke.miter_limit.get(),
    }
}

/// `#rrggbbaa`, rounded to bytes as upstream's `Color::to_vec4_u8` rounds.
fn paint(paint: &Paint) -> String {
    let Paint::Solid(color) = paint;
    hex(color)
}

fn hex(color: &Color) -> String {
    let [r, g, b, a] = color.to_rgba().map(|c| (c * 255.0).round() as u8);
    format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
}
