//! Frame serialization.

use serde_json::{Value as Json, json};
use typst::{
    layout::{Frame, FrameItem, Point, Transform},
    text::{FontInstance, TextItem},
    visualize::{Curve, CurveItem, FixedStroke, Geometry, Paint, Shape},
};

use crate::world::SpanMapper;

/// The wrapper's `#box`: the first hard frame below the page.
pub fn find_label_box(page: &Frame) -> Option<&Frame> {
    page.items().find_map(|(_, item)| match item {
        FrameItem::Group(group) if group.frame.kind().is_hard() => Some(&group.frame),
        FrameItem::Group(group) => find_label_box(&group.frame),
        _ => None,
    })
}

/// Serializes a frame. Fonts go into `fonts`, which text items reference by index.
pub fn dump_frame(frame: &Frame, mapper: &SpanMapper, fonts: &mut Vec<Json>) -> Json {
    let items = frame
        .items()
        .filter_map(|(pos, item)| {
            let mut entry = point(*pos);
            let (key, value) = match item {
                FrameItem::Group(group) => (
                    "group",
                    json!({
                        "transform": transform(group.transform),
                        "clip": group.clip.as_ref().map(curve),
                        "frame": dump_frame(&group.frame, mapper, fonts),
                    }),
                ),
                FrameItem::Text(text) => ("text", text_item(text, mapper, fonts)),
                FrameItem::Shape(shape, span) => {
                    let mut value = shape_json(shape);
                    value["span"] = mapper.json(*span);
                    ("shape", value)
                }
                FrameItem::Image(_, size, span) => (
                    "image",
                    json!({
                        "width": size.x.to_pt(),
                        "height": size.y.to_pt(),
                        "span": mapper.json(*span),
                    }),
                ),
                FrameItem::Link(..) | FrameItem::Tag(_) => return None,
            };
            entry[key] = value;
            Some(entry)
        })
        .collect::<Vec<_>>();
    json!({
        "width": frame.width().to_pt(),
        "height": frame.height().to_pt(),
        "baseline": frame.baseline().to_pt(),
        "kind": if frame.kind().is_hard() { "hard" } else { "soft" },
        "items": items,
    })
}

fn point(pos: Point) -> Json {
    json!({ "x": pos.x.to_pt(), "y": pos.y.to_pt() })
}

fn text_item(text: &TextItem, mapper: &SpanMapper, fonts: &mut Vec<Json>) -> Json {
    // One tuple per glyph: [id, x_advance, x_offset, y_advance, y_offset, range, span,
    // span_offset]. Advances and offsets are in em, `range` indexes `text`, and `span` is the
    // label-relative range of the glyph's source node (null when detached).
    let glyphs = text
        .glyphs
        .iter()
        .map(|glyph| {
            json!([
                glyph.id,
                glyph.x_advance.get(),
                glyph.x_offset.get(),
                glyph.y_advance.get(),
                glyph.y_offset.get(),
                [glyph.range.start, glyph.range.end],
                mapper.json(glyph.span.0),
                glyph.span.1,
            ])
        })
        .collect::<Vec<_>>();
    let font = font(&text.font);
    let index = fonts
        .iter()
        .position(|known| *known == font)
        .unwrap_or_else(|| {
            fonts.push(font);
            fonts.len() - 1
        });
    let mut value = json!({
        "font": index,
        "size": text.size.to_pt(),
        "fill": paint(&text.fill),
        "lang": text.lang.as_str(),
        "text": text.text.as_str(),
        "glyphs": glyphs,
    });
    if let Some(stroke) = &text.stroke {
        value["stroke"] = self::stroke(stroke);
    }
    if let Some(region) = text.region {
        value["region"] = json!(region.as_str());
    }
    value
}

pub fn font(instance: &FontInstance) -> Json {
    let font = instance.font();
    let variations = instance
        .variations()
        .0
        .iter()
        .map(|(tag, value)| json!([String::from_utf8_lossy(&tag.to_bytes()), value.0]))
        .collect::<Vec<_>>();
    json!({
        "family": font.info().family.as_str(),
        "postscript": font.post_script_name(),
        "index": font.index(),
        "variations": variations,
    })
}

pub fn paint(paint: &Paint) -> Json {
    match paint {
        Paint::Solid(color) => {
            let [r, g, b, a] = color.to_vec4_u8();
            json!(format!("#{r:02x}{g:02x}{b:02x}{a:02x}"))
        }
        Paint::Gradient(_) => json!("gradient"),
        Paint::Tiling(_) => json!("tiling"),
    }
}

pub fn stroke(stroke: &FixedStroke) -> Json {
    json!({
        "paint": paint(&stroke.paint),
        "thickness": stroke.thickness.to_pt(),
        "cap": format!("{:?}", stroke.cap),
        "join": format!("{:?}", stroke.join),
        "dash": stroke.dash.as_ref().map(|dash| json!({
            "array": dash.array.iter().map(|len| len.to_pt()).collect::<Vec<_>>(),
            "phase": dash.phase.to_pt(),
        })),
        "miter_limit": stroke.miter_limit.get(),
    })
}

fn shape_json(shape: &Shape) -> Json {
    let geometry = match &shape.geometry {
        Geometry::Line(to) => json!({ "line": [to.x.to_pt(), to.y.to_pt()] }),
        Geometry::Rect(size) => json!({ "rect": [size.x.to_pt(), size.y.to_pt()] }),
        Geometry::Curve(path) => json!({ "curve": curve(path) }),
    };
    json!({
        "geometry": geometry,
        "fill": shape.fill.as_ref().map(paint),
        "fill_rule": format!("{:?}", shape.fill_rule),
        "stroke": shape.stroke.as_ref().map(stroke),
    })
}

fn curve(curve: &Curve) -> Json {
    let pt = |p: &Point| [p.x.to_pt(), p.y.to_pt()];
    curve
        .0
        .iter()
        .map(|item| match item {
            CurveItem::Move(p) => json!(["move", pt(p)]),
            CurveItem::Line(p) => json!(["line", pt(p)]),
            CurveItem::Cubic(a, b, c) => json!(["cubic", pt(a), pt(b), pt(c)]),
            CurveItem::Close => json!(["close"]),
        })
        .collect()
}

fn transform(t: Transform) -> Json {
    json!([
        t.sx.get(),
        t.ky.get(),
        t.kx.get(),
        t.sy.get(),
        t.tx.to_pt(),
        t.ty.to_pt()
    ])
}
