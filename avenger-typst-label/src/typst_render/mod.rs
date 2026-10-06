//! Rasterizes compiled labels, after upstream's `typst-render`.
//!
//! avenger: rasterizes a label's drawing items (`typst_svg`), so raster and vector output
//! draw the same paths and images. The image covers the drawn items rather than the line's
//! box, and records where its top left lies in the label.

use std::io::Cursor;

use avenger_color::AbsoluteColor;
use thiserror::Error;

use crate::label::{
    CompiledLabel, Curve, CurveItem, DashPattern, FillRule, LineCap, LineJoin, Size,
    Transform,
};
use crate::typst_svg::{ImageItem, PathItem, SvgItem, SvgOptions, svg_items};

/// Options for rasterizing a label.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RasterOptions {
    /// Pixels per point.
    pub scale: f32,
}

impl Default for RasterOptions {
    fn default() -> Self {
        Self { scale: 1.0 }
    }
}

/// Why a label couldn't be rasterized.
#[derive(Debug, Clone, Error, PartialEq)]
pub enum RasterError {
    /// The scale isn't finite and positive.
    #[error("raster scale must be finite and positive, not {0}")]
    InvalidScale(f32),
    /// The image would be too large to allocate.
    #[error("a raster image of {width}x{height} pixels is too large")]
    TooLarge { width: u32, height: u32 },
}

/// A rasterized label.
#[derive(Debug, Clone, PartialEq)]
pub struct RasterImage {
    /// The pixels.
    pub image: RgbaImageData,
    /// Pixels per point.
    pub scale: f32,
    /// The label's width, in points.
    pub logical_width: f32,
    /// The label's height, in points.
    pub logical_height: f32,
    /// Where the image's left edge lies in the label, in points.
    pub origin_x: f32,
    /// Where the image's top edge lies in the label, in points.
    pub origin_y: f32,
}

/// Straight-alpha RGBA pixels, row by row.
#[derive(Debug, Clone, PartialEq)]
pub struct RgbaImageData {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// Rasterizes a label.
pub fn rasterize(
    label: &CompiledLabel,
    options: &RasterOptions,
) -> Result<RasterImage, RasterError> {
    let items = svg_items(label, &SvgOptions::default()).items;
    rasterize_items(label.frame.size, &items, options.scale)
}

/// Rasterizes drawing items, at `scale` pixels per point.
fn rasterize_items(
    size: Size,
    items: &[SvgItem],
    scale: f32,
) -> Result<RasterImage, RasterError> {
    if !(scale.is_finite() && scale > 0.0) {
        return Err(RasterError::InvalidScale(scale));
    }

    // The items that draw something, with their bounds in the label.
    let mut bounds = Bounds::EMPTY;
    let mut draws = vec![];
    for item in items {
        let draw = match item {
            SvgItem::Path(path) => path_draw(path),
            SvgItem::Image(image) => image_draw(image),
        };
        if let Some((draw, rect)) = draw {
            bounds.include(rect);
            draws.push(draw);
        }
    }

    let empty = RasterImage {
        image: RgbaImageData { width: 1, height: 1, data: vec![0; 4] },
        scale,
        logical_width: size.x,
        logical_height: size.y,
        origin_x: 0.0,
        origin_y: 0.0,
    };
    if draws.is_empty() || bounds.is_empty() {
        return Ok(empty);
    }

    // A pixel of margin around the drawn items.
    let left = (bounds.left * scale).floor() as i32 - 1;
    let top = (bounds.top * scale).floor() as i32 - 1;
    let right = (bounds.right * scale).ceil() as i32 + 1;
    let bottom = (bounds.bottom * scale).ceil() as i32 + 1;
    let width = (right - left).max(1) as u32;
    let height = (bottom - top).max(1) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or(RasterError::TooLarge { width, height })?;

    // From the label's coordinates to the pixmap's.
    let canvas = tiny_skia::Transform::from_scale(scale, scale)
        .post_translate(-(left as f32), -(top as f32));
    for draw in &draws {
        match draw {
            Draw::Path { path, item } => {
                let ts = canvas.pre_concat(sk_transform(item.transform));
                if let Some(fill) = item.fill {
                    let rule = match item.fill_rule {
                        FillRule::NonZero => tiny_skia::FillRule::Winding,
                        FillRule::EvenOdd => tiny_skia::FillRule::EvenOdd,
                    };
                    pixmap.fill_path(path, &sk_paint(fill), rule, ts, None);
                }
                if let Some(stroke) = &item.stroke {
                    let sk_stroke = sk_stroke(stroke);
                    pixmap.stroke_path(
                        path,
                        &sk_paint(stroke.paint),
                        &sk_stroke,
                        ts,
                        None,
                    );
                }
            }
            Draw::Image { pixels, item } => {
                let ts = canvas.pre_concat(sk_transform(item.transform)).pre_scale(
                    item.size.x / pixels.width() as f32,
                    item.size.y / pixels.height() as f32,
                );
                let paint = tiny_skia::PixmapPaint {
                    quality: tiny_skia::FilterQuality::Bicubic,
                    ..Default::default()
                };
                pixmap.draw_pixmap(0, 0, pixels.as_ref(), &paint, ts, None);
            }
        }
    }

    Ok(RasterImage {
        image: RgbaImageData { width, height, data: straight_alpha(&pixmap) },
        scale,
        logical_width: size.x,
        logical_height: size.y,
        origin_x: left as f32 / scale,
        origin_y: top as f32 / scale,
    })
}

/// An item to draw, prepared for tiny-skia.
enum Draw<'a> {
    Path { path: tiny_skia::Path, item: &'a PathItem },
    Image { pixels: tiny_skia::Pixmap, item: &'a ImageItem },
}

/// A path to draw and its bounds in the label, including its stroke.
fn path_draw(item: &PathItem) -> Option<(Draw<'_>, tiny_skia::Rect)> {
    if item.fill.is_none() && item.stroke.is_none() {
        return None;
    }
    let path = sk_path(&item.path)?;
    let ts = sk_transform(item.transform);
    let mut rect = path.clone().transform(ts)?.bounds();
    let stroked = item.stroke.as_ref().and_then(|stroke| {
        path.stroke(&sk_stroke(stroke), 1.0)?
            .transform(ts)
            .map(|path| path.bounds())
    });
    if let Some(outer) = stroked {
        rect = tiny_skia::Rect::from_ltrb(
            rect.left().min(outer.left()),
            rect.top().min(outer.top()),
            rect.right().max(outer.right()),
            rect.bottom().max(outer.bottom()),
        )?;
    }
    Some((Draw::Path { path, item }, rect))
}

/// An image to draw and its bounds in the label. Images that don't decode are skipped, as
/// upstream skips them.
fn image_draw(item: &ImageItem) -> Option<(Draw<'_>, tiny_skia::Rect)> {
    let pixels = decode_png(&item.data)?;
    let rect = tiny_skia::Rect::from_xywh(0.0, 0.0, item.size.x, item.size.y)?
        .transform(sk_transform(item.transform))?;
    Some((Draw::Image { pixels, item }, rect))
}

/// Decodes PNG data into a premultiplied pixmap.
fn decode_png(data: &[u8]) -> Option<tiny_skia::Pixmap> {
    let mut decoder = png::Decoder::new(Cursor::new(data));
    decoder.set_transformations(
        png::Transformations::ALPHA
            | png::Transformations::STRIP_16
            | png::Transformations::EXPAND,
    );
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buffer).ok()?;
    let decoded = &buffer[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => decoded.to_vec(),
        png::ColorType::GrayscaleAlpha => decoded
            .chunks_exact(2)
            .flat_map(|pixel| [pixel[0], pixel[0], pixel[0], pixel[1]])
            .collect(),
        _ => return None,
    };
    let premultiplied = rgba
        .chunks_exact(4)
        .flat_map(|pixel| {
            let alpha = u16::from(pixel[3]);
            let premultiply =
                |channel: u8| ((u16::from(channel) * alpha + 127) / 255) as u8;
            [
                premultiply(pixel[0]),
                premultiply(pixel[1]),
                premultiply(pixel[2]),
                pixel[3],
            ]
        })
        .collect();
    let size = tiny_skia::IntSize::from_wh(info.width, info.height)?;
    tiny_skia::Pixmap::from_vec(premultiplied, size)
}

fn sk_path(curve: &Curve) -> Option<tiny_skia::Path> {
    let mut builder = tiny_skia::PathBuilder::new();
    for item in &curve.0 {
        match *item {
            CurveItem::Move(p) => builder.move_to(p.x, p.y),
            CurveItem::Line(p) => builder.line_to(p.x, p.y),
            CurveItem::Cubic(a, b, c) => builder.cubic_to(a.x, a.y, b.x, b.y, c.x, c.y),
            CurveItem::Close => builder.close(),
        }
    }
    builder.finish()
}

fn sk_transform(ts: Transform) -> tiny_skia::Transform {
    tiny_skia::Transform::from_row(ts.sx, ts.ky, ts.kx, ts.sy, ts.tx, ts.ty)
}

fn sk_paint(color: AbsoluteColor) -> tiny_skia::Paint<'static> {
    let [r, g, b, a] = color.to_rgba8();
    let mut paint = tiny_skia::Paint::default();
    paint.set_color_rgba8(r, g, b, a);
    paint.anti_alias = true;
    paint
}

fn sk_stroke(stroke: &crate::label::Stroke) -> tiny_skia::Stroke {
    tiny_skia::Stroke {
        width: stroke.thickness,
        line_cap: match stroke.cap {
            LineCap::Butt => tiny_skia::LineCap::Butt,
            LineCap::Round => tiny_skia::LineCap::Round,
            LineCap::Square => tiny_skia::LineCap::Square,
        },
        line_join: match stroke.join {
            LineJoin::Miter => tiny_skia::LineJoin::Miter,
            LineJoin::Round => tiny_skia::LineJoin::Round,
            LineJoin::Bevel => tiny_skia::LineJoin::Bevel,
        },
        dash: stroke.dash.as_ref().and_then(sk_dash),
        miter_limit: stroke.miter_limit,
    }
}

/// A dash pattern for tiny-skia, which needs an even number of lengths: an odd pattern
/// repeats once, as SVG and PDF repeat it.
fn sk_dash(dash: &DashPattern) -> Option<tiny_skia::StrokeDash> {
    let len = dash.array.len();
    if len == 0 {
        return None;
    }
    let len = if len % 2 == 1 { 2 * len } else { len };
    tiny_skia::StrokeDash::new(
        dash.array.iter().copied().cycle().take(len).collect(),
        dash.phase,
    )
}

fn straight_alpha(pixmap: &tiny_skia::Pixmap) -> Vec<u8> {
    pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        })
        .collect()
}

/// A bounding rectangle, in points.
#[derive(Debug, Clone, Copy)]
struct Bounds {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

impl Bounds {
    const EMPTY: Self = Self {
        left: f32::INFINITY,
        top: f32::INFINITY,
        right: f32::NEG_INFINITY,
        bottom: f32::NEG_INFINITY,
    };

    fn is_empty(self) -> bool {
        self.left >= self.right || self.top >= self.bottom
    }

    fn include(&mut self, rect: tiny_skia::Rect) {
        self.left = self.left.min(rect.left());
        self.top = self.top.min(rect.top());
        self.right = self.right.max(rect.right());
        self.bottom = self.bottom.max(rect.bottom());
    }
}

#[cfg(test)]
mod tests;
