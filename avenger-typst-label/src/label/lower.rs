//! Lowers the pipeline's frames, in `f64` with upstream's units, to the public frame.

use super::frame::{
    Curve, CurveItem, DashPattern, FillRule, FontRef, FrameItem, Geometry, Glyph,
    GroupItem, LabelFrame, LineCap, LineJoin, Point, Shape, Size, Stroke, TextItem,
    Transform,
};
use crate::typst_library::layout::{self as layout, Frame};
use crate::typst_library::text;
use crate::typst_library::visualize::{self as visualize, FixedStroke, Paint};

/// Lowers a frame.
pub(crate) fn lower(frame: &Frame) -> LabelFrame {
    LabelFrame {
        size: size(frame.size()),
        baseline: frame.baseline().to_pt() as f32,
        items: frame
            .items()
            .map(|(pos, item)| (point(*pos), lower_item(item)))
            .collect(),
    }
}

fn lower_item(item: &layout::FrameItem) -> FrameItem {
    match item {
        layout::FrameItem::Group(group) => FrameItem::Group(GroupItem {
            frame: lower(&group.frame),
            transform: transform(group.transform),
        }),
        layout::FrameItem::Text(text) => FrameItem::Text(text_item(text)),
        layout::FrameItem::Shape(shape, _) => FrameItem::Shape(lower_shape(shape)),
    }
}

fn text_item(text: &text::TextItem) -> TextItem {
    let glyphs: Vec<_> = text
        .glyphs
        .iter()
        .map(|glyph| Glyph {
            id: glyph.id,
            x_advance: glyph.x_advance.get() as f32,
            x_offset: glyph.x_offset.get() as f32,
            y_advance: glyph.y_advance.get() as f32,
            y_offset: glyph.y_offset.get() as f32,
            range: glyph.range(),
            source: glyph.source.clone(),
        })
        .collect();

    // The union of the glyphs' sources. Glyphs without a source, such as tofus, don't count.
    let mut sources = glyphs.iter().map(|glyph| &glyph.source).filter(|r| !r.is_empty());
    let source = match sources.next() {
        Some(first) => sources.fold(first.clone(), |union, range| {
            union.start.min(range.start)..union.end.max(range.end)
        }),
        None => glyphs.first().map_or(0..0, |glyph| glyph.source.clone()),
    };

    TextItem {
        font: FontRef(text.font.clone()),
        size: text.size.to_pt() as f32,
        fill: color(&text.fill),
        text: text.text.to_string(),
        glyphs,
        source,
    }
}

fn lower_shape(shape: &visualize::Shape) -> Shape {
    Shape {
        geometry: match &shape.geometry {
            visualize::Geometry::Line(to) => Geometry::Line(point(*to)),
            visualize::Geometry::Rect(rect) => Geometry::Rect(size(*rect)),
            visualize::Geometry::Curve(c) => Geometry::Curve(curve(c)),
        },
        fill: shape.fill.as_ref().map(color),
        fill_rule: match shape.fill_rule {
            visualize::FillRule::NonZero => FillRule::NonZero,
            visualize::FillRule::EvenOdd => FillRule::EvenOdd,
        },
        stroke: shape.stroke.as_ref().map(stroke),
    }
}

fn stroke(stroke: &FixedStroke) -> Stroke {
    Stroke {
        paint: color(&stroke.paint),
        thickness: stroke.thickness.to_pt() as f32,
        cap: match stroke.cap {
            visualize::LineCap::Butt => LineCap::Butt,
            visualize::LineCap::Round => LineCap::Round,
            visualize::LineCap::Square => LineCap::Square,
        },
        join: match stroke.join {
            visualize::LineJoin::Miter => LineJoin::Miter,
            visualize::LineJoin::Round => LineJoin::Round,
            visualize::LineJoin::Bevel => LineJoin::Bevel,
        },
        dash: stroke.dash.as_ref().map(|dash| DashPattern {
            array: dash.array.iter().map(|length| length.to_pt() as f32).collect(),
            phase: dash.phase.to_pt() as f32,
        }),
        miter_limit: stroke.miter_limit.get() as f32,
    }
}

fn color(paint: &Paint) -> avenger_color::AbsoluteColor {
    let Paint::Solid(color) = paint;
    *color
}

fn curve(curve: &visualize::Curve) -> Curve {
    Curve(
        curve
            .0
            .iter()
            .map(|item| match *item {
                visualize::CurveItem::Move(p) => CurveItem::Move(point(p)),
                visualize::CurveItem::Line(p) => CurveItem::Line(point(p)),
                visualize::CurveItem::Cubic(a, b, c) => {
                    CurveItem::Cubic(point(a), point(b), point(c))
                }
                visualize::CurveItem::Close => CurveItem::Close,
            })
            .collect(),
    )
}

fn transform(ts: layout::Transform) -> Transform {
    Transform {
        sx: ts.sx.get() as f32,
        ky: ts.ky.get() as f32,
        kx: ts.kx.get() as f32,
        sy: ts.sy.get() as f32,
        tx: ts.tx.to_pt() as f32,
        ty: ts.ty.to_pt() as f32,
    }
}

fn point(point: layout::Point) -> Point {
    Point {
        x: point.x.to_pt() as f32,
        y: point.y.to_pt() as f32,
    }
}

fn size(size: layout::Size) -> Size {
    Size { x: size.x.to_pt() as f32, y: size.y.to_pt() as f32 }
}
