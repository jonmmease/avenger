//! Lowers compiled labels to vector drawing items, after upstream's `typst-svg`: glyphs as
//! outlines, bitmap glyphs as images, and shapes as paths.
//!
//! avenger: drawing items in place of an SVG document, which `avenger-svg` writes. Glyph
//! outlines come from the font instances' caches. Color glyphs with COLR or SVG data draw as
//! their outlines. Optionally, text that a viewer draws the same as the label lowers to text
//! runs, which stay selectable.

use std::ops::Range;
use std::sync::Arc;

use avenger_color::AbsoluteColor;

use crate::label::{
    BentRun, BentText, CompiledLabel, Curve, CurveItem, FillRule, FontRef, FrameItem,
    Geometry, Point, Shape, Size, Stroke, TextItem, Transform,
};
use crate::typst_library::text::{FontFlags, FontStyle, FontWeight, OutlineSegment};

/// Options for lowering a label to drawing items.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SvgOptions {
    /// Whether text items that viewers draw as the label does lower to [`TextRun`]s, in place
    /// of their glyphs' outlines.
    pub native_text: bool,
}

/// A label as drawing items.
#[derive(Debug, Clone, PartialEq)]
pub struct SvgLabel {
    /// The label's size.
    pub size: Size,
    /// The items, in drawing order.
    pub items: Vec<SvgItem>,
}

/// A drawing item.
#[derive(Debug, Clone, PartialEq)]
pub enum SvgItem {
    /// A filled or stroked path.
    Path(PathItem),
    /// A raster image.
    Image(ImageItem),
    /// A text item that draws as text.
    Text(TextRun),
}

/// A text item that viewers draw as the label does: shaping its text with its face, in its
/// direction and with no features, gives its glyphs, so a viewer with the face draws the same
/// glyphs in the same places. Its bitmap glyphs also come as images, which a writer can draw
/// instead.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    /// The text the run shows.
    pub text: String,
    /// The run's range in the label source.
    pub source: Range<usize>,
    /// The face the run's glyphs come from.
    pub font: FontRef,
    /// The font size, in points.
    pub size: f32,
    /// The run's fill.
    pub fill: AbsoluteColor,
    /// Where the run's text lies.
    pub place: RunPlace,
    /// The face's weight, from its OS/2 table.
    pub weight: FontWeight,
    /// The face's style, from its OS/2 table.
    pub style: FontStyle,
    /// The run's text item, as an index into the frame's
    /// [`text_items`](crate::LabelFrame::text_items), as [`GlyphRef::text`] counts them. A run
    /// along a path names the first item of its clusters.
    pub text_item: usize,
}

/// Where a run's text lies.
#[derive(Debug, Clone, PartialEq)]
pub enum RunPlace {
    /// In a straight line.
    Line {
        /// The run's left edge, in the label's coordinates.
        x: f32,
        /// The run's baseline, in the label's coordinates.
        baseline: f32,
        /// The run's advance width.
        width: f32,
        /// Whether the run's text runs right to left.
        rtl: bool,
    },
    /// Along a path, for a run of a label on a path, whose text runs left to right.
    Path(RunPath),
}

/// The path that a run of a label on a path follows, as SVG's `textPath` sets text: the
/// baseline of the run's line in the label's coordinates, along which a writer draws the run
/// from `start`. A viewer places each character at the middle of its advance along the path,
/// turned to the path's direction there, as the label does.
#[derive(Debug, Clone, PartialEq)]
pub struct RunPath {
    pub curve: Curve,
    /// The distance along the curve to the run's left edge, in points.
    pub start: f32,
}

/// A filled or stroked path.
#[derive(Debug, Clone, PartialEq)]
pub struct PathItem {
    /// The path, in its own coordinates.
    pub path: Curve,
    /// The transform from the path's coordinates to the label's.
    pub transform: Transform,
    /// The path's fill.
    pub fill: Option<AbsoluteColor>,
    /// The fill rule.
    pub fill_rule: FillRule,
    /// The path's stroke, in the path's coordinates.
    pub stroke: Option<Stroke>,
    /// What the path draws.
    pub kind: PathKind,
}

/// What a path draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathKind {
    /// A glyph's outline, in font units with y pointing up.
    Glyph(GlyphRef),
    /// A shape.
    Shape,
}

/// A glyph of a label's frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlyphRef {
    /// The glyph's text item, as an index into the frame's
    /// [`text_items`](crate::LabelFrame::text_items).
    pub text: usize,
    /// The glyph's index in its text item.
    pub glyph: usize,
    /// The glyph's range in the label source.
    pub source: Range<usize>,
}

/// A raster image: a bitmap glyph.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageItem {
    /// The image's PNG data.
    pub data: Arc<[u8]>,
    /// The image's size, in its own coordinates.
    pub size: Size,
    /// The transform from the image's coordinates to the label's. The image's top left is at
    /// its origin.
    pub transform: Transform,
    /// The glyph the image draws.
    pub glyph: GlyphRef,
}

/// Lowers a label to drawing items. With native text, the runs of a label on a path that read as
/// runs draw as text along their lines, in place of their clusters.
pub fn svg_items(label: &CompiledLabel, options: &SvgOptions) -> SvgLabel {
    let mut items = vec![];
    let mut texts = 0;
    let along: Vec<(TextRun, Range<usize>)> = match (&label.bent, options.native_text) {
        (Some(bent), true) => bent
            .runs
            .iter()
            .filter_map(|run| Some((run_along(bent, run)?, run.texts.clone())))
            .collect(),
        _ => Vec::new(),
    };
    label.frame.visit(Transform::IDENTITY, &mut |ts, item| match item {
        FrameItem::Text(text) => {
            match along.iter().find(|(_, clusters)| clusters.contains(&texts)) {
                Some((run, clusters)) => {
                    if texts == clusters.start {
                        items.push(SvgItem::Text(run.clone()));
                    }
                }
                None => {
                    let run =
                        options.native_text.then(|| text_run(ts, text, texts)).flatten();
                    draw_text(&mut items, ts, text, texts, run);
                }
            }
            texts += 1;
        }
        FrameItem::Shape(shape) => items.push(SvgItem::Path(shape_path(ts, shape))),
        FrameItem::Group(_) => {}
    });
    SvgLabel { size: label.frame.size, items }
}

/// Draws a text item: as a run, if it has one, and its bitmap glyphs as images; otherwise each
/// glyph as its outline or image.
fn draw_text(
    items: &mut Vec<SvgItem>,
    ts: Transform,
    text: &TextItem,
    index: usize,
    run: Option<TextRun>,
) {
    let native = run.is_some();
    items.extend(run.map(SvgItem::Text));
    let font = &text.font.0;
    let scale = text.size / font.units_per_em() as f32;
    for (glyph_index, (pos, glyph)) in text.positioned_glyphs().enumerate() {
        let glyph_ref = GlyphRef {
            text: index,
            glyph: glyph_index,
            source: glyph.source.clone(),
        };
        let origin = ts.pre_concat(Transform::translate(pos.x, pos.y));
        if let Some(image) = bitmap(font, glyph.id) {
            items.push(SvgItem::Image(ImageItem {
                data: image.data,
                size: Size::new(image.size.x * scale, image.size.y * scale),
                transform: origin.pre_concat(Transform::translate(
                    image.pos.x * scale,
                    image.pos.y * scale,
                )),
                glyph: glyph_ref,
            }));
        } else if native {
            // The run draws the glyph.
        } else if let Some(outline) = font.outline(glyph.id) {
            items.push(SvgItem::Path(PathItem {
                path: curve(&outline.0),
                transform: origin.pre_concat(Transform::scale(scale, -scale)),
                fill: Some(text.fill),
                fill_rule: FillRule::NonZero,
                stroke: None,
                kind: PathKind::Glyph(glyph_ref),
            }));
        }
    }
}

/// A text item as a run that viewers draw as text in a straight line, if it reads as one, as
/// [`run_left`] decides.
fn text_run(ts: Transform, item: &TextItem, index: usize) -> Option<TextRun> {
    let x = run_left(ts, item)?;
    let place = RunPlace::Line {
        x,
        baseline: ts.ty,
        width: item.width(),
        rtl: item.is_rtl(),
    };
    Some(run_at(item, index, place))
}

/// The left edge of a text item in the label's coordinates, if the item reads as a run that
/// viewers draw as text: it is unrotated and unscaled, its face is static and not a math face,
/// and shaping its text with the face, in its direction and with no features, gives its glyphs:
/// the same glyphs and clusters, advances and no vertical offsets. Horizontal offsets may differ
/// by a constant, as a synthesized script's do, which moves the run, and the trailing space of a
/// wrapped line keeps no advance. Math faces stay outlines, so that documents don't embed them
/// for a few glyphs.
fn run_left(ts: Transform, item: &TextItem) -> Option<f32> {
    let font = &item.font.0;
    if (ts.sx, ts.ky, ts.kx, ts.sy) != (1.0, 0.0, 0.0, 1.0)
        || !font.variations().0.is_empty()
        || font.font().info().flags.contains(FontFlags::MATH)
    {
        return None;
    }
    let rtl = item.is_rtl();
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str(&item.text);
    buffer.set_direction(if rtl {
        rustybuzz::Direction::RightToLeft
    } else {
        rustybuzz::Direction::LeftToRight
    });
    buffer.guess_segment_properties();
    // As the label engine shapes: default ignorables draw nothing.
    buffer.set_flags(rustybuzz::BufferFlags::REMOVE_DEFAULT_IGNORABLES);
    let shaped = rustybuzz::shape(font.rusty(), &[], buffer);
    if shaped.glyph_infos().len() != item.glyphs.len() {
        return None;
    }

    let units = font.units_per_em() as f32;
    let em = |value: i32| value as f32 / units;
    let mut shift = None;
    for ((glyph, info), position) in item
        .glyphs
        .iter()
        .zip(shaped.glyph_infos())
        .zip(shaped.glyph_positions())
    {
        let trimmed = glyph.x_advance == 0.0
            && item.text[glyph.range.clone()].chars().all(char::is_whitespace);
        let offset = glyph.x_offset - em(position.x_offset);
        if u32::from(glyph.id) != info.glyph_id
            || glyph.range.start != info.cluster as usize
            || (!trimmed
                && (glyph.x_advance - em(position.x_advance)).abs() > EM_TOLERANCE)
            || position.y_offset != 0
            || position.y_advance != 0
            || (offset - *shift.get_or_insert(offset)).abs() > EM_TOLERANCE
        {
            return None;
        }
    }

    Some(ts.tx + shift.unwrap_or(0.0) * item.size)
}

/// The run of the text item `index`, as [`GlyphRef::text`] counts them, lying at `place`.
fn run_at(item: &TextItem, index: usize, place: RunPlace) -> TextRun {
    let ttf = item.font.0.ttf();
    TextRun {
        text: item.text.clone(),
        source: item.source.clone(),
        font: item.font.clone(),
        size: item.size,
        fill: item.fill,
        place,
        weight: FontWeight::from_number(ttf.weight().to_number()),
        style: match ttf.style() {
            ttf_parser::Style::Normal => FontStyle::Normal,
            ttf_parser::Style::Italic => FontStyle::Italic,
            ttf_parser::Style::Oblique => FontStyle::Oblique,
        },
        text_item: index,
    }
}

/// The run of a bent label's straight text item that draws as text along its line's baseline,
/// if the item reads as a run and runs left to right, with no bitmap glyphs.
fn run_along(bent: &BentText, run: &BentRun) -> Option<TextRun> {
    let font = &run.item.font.0;
    if run.item.is_rtl()
        || run.item.glyphs.iter().any(|glyph| bitmap(font, glyph.id).is_some())
    {
        return None;
    }
    let left = run_left(run.ts, &run.item)?;
    let (curve, anchor) = &bent.lines[run.line];
    let start = anchor + left - bent.anchor_x;
    let place = RunPlace::Path(RunPath { curve: curve.clone(), start });
    Some(run_at(&run.item, run.texts.start, place))
}

/// How far a run's advances and offsets may stray from shaping's, in ems.
const EM_TOLERANCE: f32 = 1e-4;

/// A bitmap glyph's image, positioned in font units relative to the glyph's origin.
pub(crate) struct Bitmap {
    pub data: Arc<[u8]>,
    pub pos: Point,
    pub size: Size,
}

/// The PNG image of a bitmap glyph, as upstream places it: in font units relative to the
/// glyph's origin, with y pointing down.
// upstream: crates/typst-library/src/text/font/color.rs::draw_raster_glyph @ v0.15.1
pub(crate) fn bitmap(
    font: &crate::typst_library::text::FontInstance,
    id: u16,
) -> Option<Bitmap> {
    let raster = font
        .ttf()
        .glyph_raster_image(ttf_parser::GlyphId(id), u16::MAX)
        .filter(|image| image.format == ttf_parser::RasterImageFormat::PNG)?;
    let upem = font.units_per_em() as f32;
    let scale = upem / f32::from(raster.pixels_per_em);
    let width = scale * f32::from(raster.width);
    let height = scale * f32::from(raster.height);
    let x_offset = scale * f32::from(raster.x);
    let mut y_offset = scale * f32::from(raster.y);
    // Apple Color emoji doesn't provide offset information (or at least
    // not in a way ttf-parser understands), so we artificially shift their
    // baseline to make it look good.
    if font.info().family.to_lowercase() == "apple color emoji" {
        // This factor is just taken from krilla.
        y_offset -= 0.128 * upem;
    }
    Some(Bitmap {
        data: raster.data.into(),
        pos: Point::new(-x_offset, -(height + y_offset)),
        size: Size::new(width, height),
    })
}

/// A glyph outline as a curve, with its quadratic segments as cubic ones.
fn curve(outline: &[OutlineSegment]) -> Curve {
    let mut items = Vec::with_capacity(outline.len());
    let mut current = Point::ZERO;
    let mut start = Point::ZERO;
    for &segment in outline {
        match segment {
            OutlineSegment::Move(x, y) => {
                current = Point::new(x, y);
                start = current;
                items.push(CurveItem::Move(current));
            }
            OutlineSegment::Line(x, y) => {
                current = Point::new(x, y);
                items.push(CurveItem::Line(current));
            }
            OutlineSegment::Quad(x1, y1, x, y) => {
                // The cubic with the quadratic's shape.
                let control = Point::new(x1, y1);
                let end = Point::new(x, y);
                let toward = |from: Point| {
                    Point::new(
                        from.x + 2.0 / 3.0 * (control.x - from.x),
                        from.y + 2.0 / 3.0 * (control.y - from.y),
                    )
                };
                items.push(CurveItem::Cubic(toward(current), toward(end), end));
                current = end;
            }
            OutlineSegment::Cubic(x1, y1, x2, y2, x, y) => {
                current = Point::new(x, y);
                items.push(CurveItem::Cubic(
                    Point::new(x1, y1),
                    Point::new(x2, y2),
                    current,
                ));
            }
            OutlineSegment::Close => {
                current = start;
                items.push(CurveItem::Close);
            }
        }
    }
    Curve(items)
}

/// A shape as a path.
pub(crate) fn shape_path(ts: Transform, shape: &Shape) -> PathItem {
    let path = match &shape.geometry {
        Geometry::Line(to) => {
            Curve(vec![CurveItem::Move(Point::ZERO), CurveItem::Line(*to)])
        }
        // Upstream's rectangle winding.
        Geometry::Rect(size) => Curve(vec![
            CurveItem::Move(Point::ZERO),
            CurveItem::Line(Point::new(0.0, size.y)),
            CurveItem::Line(Point::new(size.x, size.y)),
            CurveItem::Line(Point::new(size.x, 0.0)),
            CurveItem::Close,
        ]),
        Geometry::Curve(curve) => curve.clone(),
    };
    PathItem {
        path,
        transform: ts,
        fill: shape.fill,
        fill_rule: shape.fill_rule,
        stroke: shape.stroke.clone(),
        kind: PathKind::Shape,
    }
}
