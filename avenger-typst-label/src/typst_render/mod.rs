//! Raster lowering for compiled label frames.
//!
//! This mirrors the frame-to-pixels role of upstream `typst-render`, but only
//! for the path/image artifacts emitted by a compiled single-line label.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "raster")]
use std::io::Cursor;

#[cfg(feature = "raster")]
use crate::{
    label::LabelError,
    typst_library::Color,
    typst_svg::{
        DashPattern, LineCap, LineJoin, PathArtifact, PathCommand, PathData, PathImageFormat,
        PathImageItem, Transform,
    },
};

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct RasterRequest {
    pub scale: f32,
}

impl Default for RasterRequest {
    fn default() -> Self {
        Self { scale: 1.0 }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct RgbaImageData {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct RasterImage {
    pub image: RgbaImageData,
    pub scale: f32,
    pub logical_width: f32,
    pub logical_height: f32,
    pub origin_x: f32,
    pub origin_y: f32,
}

#[cfg(feature = "raster")]
pub(crate) fn rasterize_path_artifact(
    artifact: &PathArtifact,
    request: RasterRequest,
) -> Result<RasterImage, LabelError> {
    let scale = if request.scale.is_finite() && request.scale > 0.0 {
        request.scale
    } else {
        return Err(LabelError::UnsupportedOutput(
            "raster scale must be finite and positive",
        ));
    };

    let mut draw_items = std::collections::HashMap::new();
    let mut draw_images = Vec::new();
    let mut bounds = RasterBounds::empty();

    for (index, item) in artifact.items.iter().enumerate() {
        if item.clip.is_some() {
            return Err(LabelError::UnsupportedOutput(
                "clipped Typst paths are not supported in raster output yet",
            ));
        }

        let Some(path) = tiny_path_from_math_path(&item.path) else {
            continue;
        };
        let transform = tiny_transform_from_math_transform(item.transform);
        let transformed =
            path.clone()
                .transform(transform)
                .ok_or(LabelError::UnsupportedOutput(
                    "non-finite Typst path transform is not supported in raster output",
                ))?;
        let mut item_bounds = transformed.bounds();

        if let Some(stroke) = &item.stroke {
            let outset = stroke.width / 2.0;
            item_bounds =
                item_bounds
                    .outset(outset, outset)
                    .ok_or(LabelError::UnsupportedOutput(
                        "invalid Typst math stroke bounds in raster output",
                    ))?;
        }

        bounds.include_rect(item_bounds);
        draw_items.insert(index, (path, item));
    }

    for image in &artifact.images {
        let Some(rect) = tiny_skia::Rect::from_xywh(0.0, 0.0, image.width, image.height) else {
            continue;
        };
        let transform = tiny_transform_from_math_transform(image.transform);
        let transformed = rect
            .transform(transform)
            .ok_or(LabelError::UnsupportedOutput(
                "non-finite Typst image glyph transform is not supported in raster output",
            ))?;
        bounds.include_rect(transformed);
        draw_images.push(image);
    }

    if (draw_items.is_empty() && draw_images.is_empty()) || bounds.is_empty() {
        return Ok(empty_raster_artifact(artifact, scale));
    }

    let left_px = (bounds.left * scale).floor() as i32 - 1;
    let top_px = (bounds.top * scale).floor() as i32 - 1;
    let right_px = (bounds.right * scale).ceil() as i32 + 1;
    let bottom_px = (bounds.bottom * scale).ceil() as i32 + 1;
    let width = (right_px - left_px).max(1) as u32;
    let height = (bottom_px - top_px).max(1) as u32;

    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or(LabelError::UnsupportedOutput(
        "raster dimensions are too large",
    ))?;

    for draw in artifact.ordered_items() {
        let index = match draw {
            crate::typst_svg::PathDrawItem::Image(i) => {
                if let Some(image) = artifact.images.get(i) {
                    draw_image_item(&mut pixmap, image, scale, left_px, top_px)?;
                }
                continue;
            }
            crate::typst_svg::PathDrawItem::Path(i) => i,
        };
        let Some((path, item)) = draw_items.get(&index) else {
            continue;
        };
        let item_transform = tiny_transform_from_math_transform(item.transform);
        let draw_transform = item_transform
            .post_scale(scale, scale)
            .post_translate(-(left_px as f32), -(top_px as f32));

        if let Some(fill) = item.fill {
            let paint = paint_from_color(fill);
            pixmap.fill_path(
                path,
                &paint,
                tiny_skia::FillRule::Winding,
                draw_transform,
                None,
            );
        }

        if let Some(stroke) = &item.stroke {
            let paint = paint_from_color(stroke.color);
            // Keep stroke dimensions in path coordinates; draw_transform scales them.
            let tiny_stroke = tiny_skia::Stroke {
                width: stroke.width,
                line_cap: tiny_line_cap(stroke.line_cap),
                line_join: tiny_line_join(stroke.line_join),
                dash: stroke.dash.as_ref().and_then(tiny_dash_pattern),
                miter_limit: stroke.miter_limit,
            };
            pixmap.stroke_path(path, &paint, &tiny_stroke, draw_transform, None);
        }
    }

    Ok(RasterImage {
        image: RgbaImageData {
            width,
            height,
            data: straight_alpha_rgba(&pixmap),
        },
        scale,
        logical_width: artifact.logical_width,
        logical_height: artifact.logical_height,
        origin_x: left_px as f32 / scale,
        origin_y: top_px as f32 / scale,
    })
}

#[cfg(feature = "raster")]
fn tiny_dash_pattern(dash: &DashPattern) -> Option<tiny_skia::StrokeDash> {
    let (array, phase) = tiny_dash_components(dash)?;
    tiny_skia::StrokeDash::new(array, phase)
}

#[cfg(feature = "raster")]
fn tiny_dash_components(dash: &DashPattern) -> Option<(Vec<f32>, f32)> {
    let pattern_len = dash.array.len();
    if pattern_len == 0 {
        return None;
    }
    let len = if pattern_len % 2 == 1 {
        2 * pattern_len
    } else {
        pattern_len
    };
    let array = dash.array.iter().copied().cycle().take(len).collect();
    Some((array, dash.phase))
}

#[cfg(all(test, feature = "raster"))]
mod tests {
    use super::*;
    use crate::typst_svg::{PathItem, PathKind, Stroke};

    fn horizontal_rule(width: f32, dash: Option<DashPattern>) -> PathArtifact {
        PathArtifact {
            logical_width: 48.0,
            logical_height: 20.0,
            items: vec![PathItem {
                path: PathData {
                    commands: vec![
                        PathCommand::MoveTo { x: 0.0, y: 0.0 },
                        PathCommand::LineTo { x: 40.0, y: 0.0 },
                    ],
                },
                kind: PathKind::MathShape,
                fill: None,
                stroke: Some(Stroke {
                    color: Color::BLACK,
                    width,
                    line_cap: LineCap::Butt,
                    line_join: LineJoin::Miter,
                    dash,
                    miter_limit: 4.0,
                }),
                transform: Transform {
                    tx: 4.0,
                    ty: 10.0,
                    ..Transform::IDENTITY
                },
                clip: None,
            }],
            images: Vec::new(),
            draw_order: Vec::new(),
        }
    }

    #[test]
    fn bitmap_glyphs_obey_path_painter_order() {
        use crate::typst_svg::PathDrawItem;
        let mut png_data = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png_data, 8, 8);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[0, 0, 0, 255].repeat(64)).unwrap();
        }
        let mut artifact = horizontal_rule(4.0, None);
        artifact.items[0].stroke.as_mut().unwrap().color = Color::rgba(1.0, 0.0, 0.0, 1.0);
        artifact.images.push(PathImageItem {
            data: png_data,
            format: PathImageFormat::Png,
            width: 8.0,
            height: 8.0,
            transform: Transform {
                tx: 20.0,
                ty: 6.0,
                ..Transform::IDENTITY
            },
        });
        let pixel = |raster: &RasterImage| {
            let x = (24.0 - raster.origin_x) as usize;
            let y = (10.0 - raster.origin_y) as usize;
            raster.image.data[(y * raster.image.width as usize + x) * 4..][..4].to_vec()
        };
        artifact.draw_order = vec![PathDrawItem::Path(0), PathDrawItem::Image(0)];
        let behind = rasterize_path_artifact(&artifact, RasterRequest { scale: 1.0 }).unwrap();
        artifact.draw_order.reverse();
        let foreground = rasterize_path_artifact(&artifact, RasterRequest { scale: 1.0 }).unwrap();
        assert_eq!(pixel(&behind), [0, 0, 0, 255]);
        assert_eq!(pixel(&foreground), [255, 0, 0, 255]);
    }

    #[test]
    fn raster_rule_thickness_scales_once() {
        // Whole physical-pixel widths avoid dependence on subpixel stroke rounding.
        for width in [2.0, 4.0] {
            let artifact = horizontal_rule(width, None);
            for scale in [0.5, 1.0, 1.5, 2.0, 3.0] {
                let raster = rasterize_path_artifact(&artifact, RasterRequest { scale }).unwrap();
                let x = ((24.0 - raster.origin_x) * scale).floor() as usize;
                // Sum coverage through the line, including antialiased edge pixels.
                let physical_thickness: f32 = raster
                    .image
                    .data
                    .chunks_exact(raster.image.width as usize * 4)
                    .map(|row| f32::from(row[x * 4 + 3]) / 255.0)
                    .sum();
                let logical_thickness = physical_thickness / scale;
                assert!(
                    (logical_thickness - width).abs() < 0.05,
                    "rule width {width} at scale {scale} rendered as {logical_thickness} logical pixels"
                );
            }
        }
    }

    #[test]
    fn raster_rule_dashes_and_phase_scale_once() {
        let artifact = horizontal_rule(
            4.0,
            Some(DashPattern {
                array: vec![6.0, 4.0],
                phase: 2.0,
            }),
        );
        for scale in [1.0, 1.5, 2.0, 3.0] {
            let raster = rasterize_path_artifact(&artifact, RasterRequest { scale }).unwrap();
            let y = ((10.0 - raster.origin_y) * scale).floor() as usize;
            for offset in 0..40 {
                let x = ((4.0 + offset as f32 + 0.5 - raster.origin_x) * scale).floor() as usize;
                let alpha = raster.image.data[(y * raster.image.width as usize + x) * 4 + 3];
                let expected = if (offset + 2) % 10 < 6 { 255 } else { 0 };
                assert_eq!(
                    alpha, expected,
                    "dash coverage at offset {offset}, scale {scale}"
                );
            }
        }
    }

    #[test]
    fn tiny_dash_components_repeat_odd_arrays_and_preserve_phase() {
        let dash = DashPattern {
            array: vec![1.0, 2.0, 3.0],
            phase: 0.5,
        };

        let (array, phase) = tiny_dash_components(&dash).unwrap();

        assert_eq!(array, vec![1.0, 2.0, 3.0, 1.0, 2.0, 3.0]);
        assert_eq!(phase, 0.5);
    }
}

#[cfg(feature = "raster")]
fn draw_image_item(
    pixmap: &mut tiny_skia::Pixmap,
    image: &PathImageItem,
    scale: f32,
    left_px: i32,
    top_px: i32,
) -> Result<(), LabelError> {
    let PathImageFormat::Png = image.format;
    let decoded = decode_png_to_rgba(&image.data)?;
    let width = decoded.width;
    let height = decoded.height;
    let Some(size) = tiny_skia::IntSize::from_wh(width, height) else {
        return Err(LabelError::UnsupportedOutput(
            "Typst PNG glyph dimensions are too large",
        ));
    };
    let source = tiny_skia::Pixmap::from_vec(premultiply_rgba(decoded.data), size).ok_or(
        LabelError::UnsupportedOutput("Typst PNG glyph data did not match its dimensions"),
    )?;
    if image.transform.sx != 1.0
        || image.transform.ky != 0.0
        || image.transform.kx != 0.0
        || image.transform.sy != 1.0
    {
        return Err(LabelError::UnsupportedOutput(
            "transformed Typst PNG glyphs are not supported in raster output yet",
        ));
    }
    let sx = image.width * scale / width as f32;
    let sy = image.height * scale / height as f32;
    let transform = tiny_skia::Transform::from_scale(sx, sy).post_translate(
        image.transform.tx * scale - left_px as f32,
        image.transform.ty * scale - top_px as f32,
    );

    pixmap.draw_pixmap(
        0,
        0,
        source.as_ref(),
        &tiny_skia::PixmapPaint::default(),
        transform,
        None,
    );
    Ok(())
}

#[cfg(feature = "raster")]
fn decode_png_to_rgba(data: &[u8]) -> Result<RgbaImageData, LabelError> {
    let mut decoder = png::Decoder::new(Cursor::new(data));
    decoder.set_transformations(png::Transformations::ALPHA | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|_| LabelError::UnsupportedOutput("failed to decode Typst PNG glyph"))?;
    let buffer_size = reader
        .output_buffer_size()
        .ok_or(LabelError::UnsupportedOutput(
            "failed to decode Typst PNG glyph",
        ))?;
    let mut buffer = vec![0; buffer_size];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|_| LabelError::UnsupportedOutput("failed to decode Typst PNG glyph"))?;
    let decoded = &buffer[..info.buffer_size()];

    if info.bit_depth != png::BitDepth::Eight {
        return Err(LabelError::UnsupportedOutput(
            "unsupported Typst PNG glyph color format",
        ));
    }

    let rgba = match info.color_type {
        png::ColorType::Rgba => decoded.to_vec(),
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity(rgba_byte_len(info.width, info.height)?);
            for pixel in decoded.chunks_exact(3) {
                out.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
            out
        }
        png::ColorType::GrayscaleAlpha => {
            let mut out = Vec::with_capacity(rgba_byte_len(info.width, info.height)?);
            for pixel in decoded.chunks_exact(2) {
                out.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
            }
            out
        }
        png::ColorType::Grayscale => {
            let mut out = Vec::with_capacity(rgba_byte_len(info.width, info.height)?);
            for gray in decoded {
                out.extend_from_slice(&[*gray, *gray, *gray, 255]);
            }
            out
        }
        png::ColorType::Indexed => {
            return Err(LabelError::UnsupportedOutput(
                "unsupported Typst PNG glyph color format",
            ));
        }
    };

    let expected_len = rgba_byte_len(info.width, info.height)?;
    if rgba.len() != expected_len {
        return Err(LabelError::UnsupportedOutput(
            "Typst PNG glyph data did not match its dimensions",
        ));
    }

    Ok(RgbaImageData {
        width: info.width,
        height: info.height,
        data: rgba,
    })
}

#[cfg(feature = "raster")]
fn rgba_byte_len(width: u32, height: u32) -> Result<usize, LabelError> {
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(LabelError::UnsupportedOutput(
            "Typst PNG glyph dimensions are too large",
        ))
}

#[cfg(feature = "raster")]
fn premultiply_rgba(mut data: Vec<u8>) -> Vec<u8> {
    for pixel in data.chunks_mut(4) {
        let alpha = pixel[3] as u16;
        pixel[0] = ((pixel[0] as u16 * alpha + 127) / 255) as u8;
        pixel[1] = ((pixel[1] as u16 * alpha + 127) / 255) as u8;
        pixel[2] = ((pixel[2] as u16 * alpha + 127) / 255) as u8;
    }
    data
}

#[cfg(feature = "raster")]
fn empty_raster_artifact(artifact: &PathArtifact, scale: f32) -> RasterImage {
    RasterImage {
        image: RgbaImageData {
            width: 1,
            height: 1,
            data: vec![0, 0, 0, 0],
        },
        scale,
        logical_width: artifact.logical_width,
        logical_height: artifact.logical_height,
        origin_x: 0.0,
        origin_y: 0.0,
    }
}

#[cfg(feature = "raster")]
fn tiny_path_from_math_path(path: &PathData) -> Option<tiny_skia::Path> {
    let mut builder = tiny_skia::PathBuilder::new();

    for command in &path.commands {
        match *command {
            PathCommand::MoveTo { x, y } => builder.move_to(x, y),
            PathCommand::LineTo { x, y } => builder.line_to(x, y),
            PathCommand::QuadTo { x1, y1, x, y } => builder.quad_to(x1, y1, x, y),
            PathCommand::CubicTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => builder.cubic_to(x1, y1, x2, y2, x, y),
            PathCommand::Close => builder.close(),
        }
    }

    builder.finish()
}

#[cfg(feature = "raster")]
fn tiny_transform_from_math_transform(transform: Transform) -> tiny_skia::Transform {
    tiny_skia::Transform::from_row(
        transform.sx,
        transform.ky,
        transform.kx,
        transform.sy,
        transform.tx,
        transform.ty,
    )
}

#[cfg(feature = "raster")]
fn paint_from_color(color: Color) -> tiny_skia::Paint<'static> {
    let mut paint = tiny_skia::Paint::default();
    paint.set_color_rgba8(
        channel(color.r),
        channel(color.g),
        channel(color.b),
        channel(color.a),
    );
    paint
}

#[cfg(feature = "raster")]
fn tiny_line_cap(cap: LineCap) -> tiny_skia::LineCap {
    match cap {
        LineCap::Butt => tiny_skia::LineCap::Butt,
        LineCap::Round => tiny_skia::LineCap::Round,
        LineCap::Square => tiny_skia::LineCap::Square,
    }
}

#[cfg(feature = "raster")]
fn tiny_line_join(join: LineJoin) -> tiny_skia::LineJoin {
    match join {
        LineJoin::Bevel => tiny_skia::LineJoin::Bevel,
        LineJoin::Miter => tiny_skia::LineJoin::Miter,
        LineJoin::Round => tiny_skia::LineJoin::Round,
    }
}

#[cfg(feature = "raster")]
fn channel(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(feature = "raster")]
fn straight_alpha_rgba(pixmap: &tiny_skia::Pixmap) -> Vec<u8> {
    pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        })
        .collect()
}

#[cfg(feature = "raster")]
#[derive(Debug, Clone, Copy)]
struct RasterBounds {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

#[cfg(feature = "raster")]
impl RasterBounds {
    fn empty() -> Self {
        Self {
            left: f32::INFINITY,
            top: f32::INFINITY,
            right: f32::NEG_INFINITY,
            bottom: f32::NEG_INFINITY,
        }
    }

    fn is_empty(self) -> bool {
        self.left >= self.right || self.top >= self.bottom
    }

    fn include_rect(&mut self, rect: tiny_skia::Rect) {
        self.left = self.left.min(rect.left());
        self.top = self.top.min(rect.top());
        self.right = self.right.max(rect.right());
        self.bottom = self.bottom.max(rect.bottom());
    }
}
