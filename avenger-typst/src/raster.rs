#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "raster")]
use crate::{
    error::MathTypesetError,
    paths::{MathPathArtifact, MathPathCommand, MathPathData, MathTransform},
    style::Color,
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
pub struct MathRasterArtifact {
    pub image: RgbaImageData,
    pub scale: f32,
    pub logical_width: f32,
    pub logical_height: f32,
    pub origin_x: f32,
    pub origin_y: f32,
}

#[cfg(feature = "raster")]
pub(crate) fn rasterize_path_artifact(
    artifact: &MathPathArtifact,
    request: RasterRequest,
) -> Result<MathRasterArtifact, MathTypesetError> {
    let scale = if request.scale.is_finite() && request.scale > 0.0 {
        request.scale
    } else {
        return Err(MathTypesetError::UnsupportedOutput(
            "math raster scale must be finite and positive",
        ));
    };

    let mut draw_items = Vec::new();
    let mut bounds = RasterBounds::empty();

    for item in &artifact.items {
        if item.clip.is_some() {
            return Err(MathTypesetError::UnsupportedOutput(
                "clipped Typst math paths are not supported in raster output yet",
            ));
        }

        let Some(path) = tiny_path_from_math_path(&item.path) else {
            continue;
        };
        let transform = tiny_transform_from_math_transform(item.transform);
        let transformed =
            path.clone()
                .transform(transform)
                .ok_or(MathTypesetError::UnsupportedOutput(
                    "non-finite Typst math path transform is not supported in raster output",
                ))?;
        let mut item_bounds = transformed.bounds();

        if let Some(stroke) = &item.stroke {
            let outset = stroke.width / 2.0;
            item_bounds =
                item_bounds
                    .outset(outset, outset)
                    .ok_or(MathTypesetError::UnsupportedOutput(
                        "invalid Typst math stroke bounds in raster output",
                    ))?;
        }

        bounds.include_rect(item_bounds);
        draw_items.push((path, item));
    }

    if draw_items.is_empty() || bounds.is_empty() {
        return Ok(empty_raster_artifact(artifact, scale));
    }

    let left_px = (bounds.left * scale).floor() as i32 - 1;
    let top_px = (bounds.top * scale).floor() as i32 - 1;
    let right_px = (bounds.right * scale).ceil() as i32 + 1;
    let bottom_px = (bounds.bottom * scale).ceil() as i32 + 1;
    let width = (right_px - left_px).max(1) as u32;
    let height = (bottom_px - top_px).max(1) as u32;

    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or(
        MathTypesetError::UnsupportedOutput("math raster dimensions are too large"),
    )?;

    for (path, item) in draw_items {
        let item_transform = tiny_transform_from_math_transform(item.transform);
        let draw_transform = item_transform
            .post_scale(scale, scale)
            .post_translate(-(left_px as f32), -(top_px as f32));

        if let Some(fill) = item.fill {
            let paint = paint_from_color(fill);
            pixmap.fill_path(
                &path,
                &paint,
                tiny_skia::FillRule::Winding,
                draw_transform,
                None,
            );
        }

        if let Some(stroke) = &item.stroke {
            let paint = paint_from_color(stroke.color);
            let tiny_stroke = tiny_skia::Stroke {
                width: stroke.width * scale,
                ..Default::default()
            };
            pixmap.stroke_path(&path, &paint, &tiny_stroke, draw_transform, None);
        }
    }

    Ok(MathRasterArtifact {
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
fn empty_raster_artifact(artifact: &MathPathArtifact, scale: f32) -> MathRasterArtifact {
    MathRasterArtifact {
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
fn tiny_path_from_math_path(path: &MathPathData) -> Option<tiny_skia::Path> {
    let mut builder = tiny_skia::PathBuilder::new();

    for command in &path.commands {
        match *command {
            MathPathCommand::MoveTo { x, y } => builder.move_to(x, y),
            MathPathCommand::LineTo { x, y } => builder.line_to(x, y),
            MathPathCommand::QuadTo { x1, y1, x, y } => builder.quad_to(x1, y1, x, y),
            MathPathCommand::CubicTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => builder.cubic_to(x1, y1, x2, y2, x, y),
            MathPathCommand::Close => builder.close(),
        }
    }

    builder.finish()
}

#[cfg(feature = "raster")]
fn tiny_transform_from_math_transform(transform: MathTransform) -> tiny_skia::Transform {
    tiny_skia::Transform::from_row(
        transform.xx,
        transform.yx,
        transform.xy,
        transform.yy,
        transform.dx,
        transform.dy,
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
