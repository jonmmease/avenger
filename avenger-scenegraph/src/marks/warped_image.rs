use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

use avenger_image::RgbaImage;
use serde::{Deserialize, Serialize};

use super::{
    image::{SceneImageSource, SceneImageUnavailablePolicy},
    mark::SceneMark,
};

/// An image drawn as a textured triangle mesh instead of an axis-aligned
/// rectangle. Each vertex carries a scene-coordinate position and a
/// normalized `[0, 1]` texture coordinate into the source image; `indices`
/// is a triangle list into those vertices.
///
/// This is the rendering primitive for warped raster tiles: a projection
/// that is not the tile grid's native Web Mercator maps a rectangular tile
/// onto a curved region of the plot, which the mesh approximates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SceneWarpedImageMark {
    pub name: String,
    pub interactive: bool,
    pub clip: bool,
    pub smooth: bool,
    pub image: SceneImageSource,
    pub positions: Vec<[f32; 2]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    #[serde(default)]
    pub unavailable_policy: SceneImageUnavailablePolicy,
    pub zindex: Option<i32>,
    /// `Some(edge_px)` routes the image through the renderer's persistent
    /// tile texture-array cache instead of the per-frame image atlas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile_texture_size: Option<u32>,
}

impl SceneWarpedImageMark {
    /// Bounding box of the mesh as `[min_x, min_y, max_x, max_y]` in the
    /// coordinate space of `origin`. `None` when the mesh is empty.
    pub fn bounds(&self, origin: [f32; 2]) -> Option<[f32; 4]> {
        let mut bounds: Option<[f32; 4]> = None;
        for &index in &self.indices {
            let [x, y] = *self.positions.get(index as usize)?;
            let (x, y) = (x + origin[0], y + origin[1]);
            bounds = Some(match bounds {
                None => [x, y, x, y],
                Some([min_x, min_y, max_x, max_y]) => {
                    [min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y)]
                }
            });
        }
        bounds
    }

    /// True when the vertex/index buffers are mutually consistent and
    /// describe at least one triangle.
    pub fn is_valid(&self) -> bool {
        self.positions.len() == self.uvs.len()
            && !self.indices.is_empty()
            && self.indices.len().is_multiple_of(3)
            && self
                .indices
                .iter()
                .all(|index| (*index as usize) < self.positions.len())
    }

    /// Software-rasterize the mesh into an RGBA image for static export
    /// backends (SVG/PDF) that have no textured-mesh primitive.
    ///
    /// `scale` is the supersampling factor (output pixels per scene unit).
    /// Returns the image plus its placement `[min_x, min_y, max_x, max_y]`
    /// in the coordinate space of `origin`. Requires an inline image
    /// source; returns `None` for unresolved resources or empty meshes.
    pub fn rasterize(&self, origin: [f32; 2], scale: f32) -> Option<(RgbaImage, [f32; 4])> {
        if !self.is_valid() {
            return None;
        }
        let source = self.image.inline_image()?;
        if source.width == 0
            || source.height == 0
            || (source.width as usize)
                .checked_mul(source.height as usize)?
                .checked_mul(4)?
                != source.data.len()
            || self
                .positions
                .iter()
                .chain(self.uvs.iter())
                .flatten()
                .any(|value| !value.is_finite())
            || origin.iter().any(|value| !value.is_finite())
        {
            return None;
        }
        let [min_x, min_y, max_x, max_y] = self.bounds(origin)?;
        if ![min_x, min_y, max_x, max_y]
            .iter()
            .all(|value| value.is_finite())
            || max_x <= min_x
            || max_y <= min_y
        {
            return None;
        }
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        let out_width = (((max_x - min_x) * scale).ceil() as usize).clamp(1, 8192);
        let out_height = (((max_y - min_y) * scale).ceil() as usize).clamp(1, 8192);
        // Use the actual output dimensions when the allocation cap reduces resolution.
        let scale_x = out_width as f32 / (max_x - min_x);
        let scale_y = out_height as f32 / (max_y - min_y);
        let mut data = vec![0u8; out_width * out_height * 4];

        for triangle in self.indices.chunks_exact(3) {
            let [a, b, c] = [
                triangle[0] as usize,
                triangle[1] as usize,
                triangle[2] as usize,
            ];
            let pos = |i: usize| -> [f32; 2] {
                [
                    (self.positions[i][0] + origin[0] - min_x) * scale_x,
                    (self.positions[i][1] + origin[1] - min_y) * scale_y,
                ]
            };
            let (pa, pb, pc) = (pos(a), pos(b), pos(c));
            let area = (pb[0] - pa[0]) * (pc[1] - pa[1]) - (pc[0] - pa[0]) * (pb[1] - pa[1]);
            if area.abs() < f32::EPSILON {
                continue;
            }
            let x_start = pa[0].min(pb[0]).min(pc[0]).floor().max(0.0) as usize;
            let x_end = (pa[0].max(pb[0]).max(pc[0]).ceil() as usize).min(out_width);
            let y_start = pa[1].min(pb[1]).min(pc[1]).floor().max(0.0) as usize;
            let y_end = (pa[1].max(pb[1]).max(pc[1]).ceil() as usize).min(out_height);

            for py in y_start..y_end {
                for px in x_start..x_end {
                    let p = [px as f32 + 0.5, py as f32 + 0.5];
                    let w_a =
                        ((pb[0] - p[0]) * (pc[1] - p[1]) - (pc[0] - p[0]) * (pb[1] - p[1])) / area;
                    let w_b =
                        ((pc[0] - p[0]) * (pa[1] - p[1]) - (pa[0] - p[0]) * (pc[1] - p[1])) / area;
                    let w_c = 1.0 - w_a - w_b;
                    // Small negative tolerance keeps shared triangle edges
                    // free of dropout from floating-point rounding.
                    const EDGE_TOLERANCE: f32 = -1e-4;
                    if w_a < EDGE_TOLERANCE || w_b < EDGE_TOLERANCE || w_c < EDGE_TOLERANCE {
                        continue;
                    }
                    let u = w_a * self.uvs[a][0] + w_b * self.uvs[b][0] + w_c * self.uvs[c][0];
                    let v = w_a * self.uvs[a][1] + w_b * self.uvs[b][1] + w_c * self.uvs[c][1];
                    let rgba = if self.smooth {
                        sample_bilinear(source, u, v)
                    } else {
                        sample_nearest(source, u, v)
                    };
                    let offset = (py * out_width + px) * 4;
                    data[offset..offset + 4].copy_from_slice(&rgba);
                }
            }
        }

        Some((
            RgbaImage {
                width: out_width as u32,
                height: out_height as u32,
                data,
            },
            [min_x, min_y, max_x, max_y],
        ))
    }
}

fn sample_nearest(image: &RgbaImage, u: f32, v: f32) -> [u8; 4] {
    let x = ((u * image.width as f32) as i64).clamp(0, i64::from(image.width) - 1) as usize;
    let y = ((v * image.height as f32) as i64).clamp(0, i64::from(image.height) - 1) as usize;
    pixel_at(image, x, y)
}

fn sample_bilinear(image: &RgbaImage, u: f32, v: f32) -> [u8; 4] {
    let fx = (u * image.width as f32 - 0.5).clamp(0.0, image.width as f32 - 1.0);
    let fy = (v * image.height as f32 - 0.5).clamp(0.0, image.height as f32 - 1.0);
    let x0 = fx.floor() as usize;
    let y0 = fy.floor() as usize;
    let x1 = (x0 + 1).min(image.width as usize - 1);
    let y1 = (y0 + 1).min(image.height as usize - 1);
    let tx = fx - x0 as f32;
    let ty = fy - y0 as f32;

    let (p00, p10) = (pixel_at(image, x0, y0), pixel_at(image, x1, y0));
    let (p01, p11) = (pixel_at(image, x0, y1), pixel_at(image, x1, y1));
    let mut rgba = [0u8; 4];
    for (channel, value) in rgba.iter_mut().enumerate() {
        let top = p00[channel] as f32 * (1.0 - tx) + p10[channel] as f32 * tx;
        let bottom = p01[channel] as f32 * (1.0 - tx) + p11[channel] as f32 * tx;
        *value = (top * (1.0 - ty) + bottom * ty).round().clamp(0.0, 255.0) as u8;
    }
    rgba
}

fn pixel_at(image: &RgbaImage, x: usize, y: usize) -> [u8; 4] {
    let offset = (y * image.width as usize + x) * 4;
    image.data[offset..offset + 4]
        .try_into()
        .unwrap_or([0, 0, 0, 0])
}

impl Hash for SceneWarpedImageMark {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.interactive.hash(state);
        self.clip.hash(state);
        self.smooth.hash(state);
        self.image.hash(state);
        for position in &self.positions {
            position[0].to_bits().hash(state);
            position[1].to_bits().hash(state);
        }
        for uv in &self.uvs {
            uv[0].to_bits().hash(state);
            uv[1].to_bits().hash(state);
        }
        self.indices.hash(state);
        self.unavailable_policy.hash(state);
        self.zindex.hash(state);
        self.tile_texture_size.hash(state);
    }
}

impl Default for SceneWarpedImageMark {
    fn default() -> Self {
        Self {
            name: "warped_image_mark".to_string(),
            interactive: false,
            clip: true,
            smooth: true,
            image: SceneImageSource::default(),
            positions: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
            unavailable_policy: SceneImageUnavailablePolicy::RendererDefault,
            zindex: None,
            tile_texture_size: None,
        }
    }
}

impl From<SceneWarpedImageMark> for SceneMark {
    fn from(mark: SceneWarpedImageMark) -> Self {
        SceneMark::WarpedImage(Arc::new(mark))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checker_image() -> RgbaImage {
        // 2x2: red, green / blue, white
        RgbaImage {
            width: 2,
            height: 2,
            data: vec![
                255, 0, 0, 255, 0, 255, 0, 255, //
                0, 0, 255, 255, 255, 255, 255, 255,
            ],
        }
    }

    fn quad_mark() -> SceneWarpedImageMark {
        SceneWarpedImageMark {
            image: SceneImageSource::inline(checker_image()),
            positions: vec![[10.0, 20.0], [30.0, 20.0], [30.0, 40.0], [10.0, 40.0]],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            indices: vec![0, 1, 2, 0, 2, 3],
            smooth: false,
            ..Default::default()
        }
    }

    #[test]
    fn bounds_cover_indexed_vertices_with_origin() {
        let mark = quad_mark();
        assert_eq!(mark.bounds([5.0, 1.0]), Some([15.0, 21.0, 35.0, 41.0]));
        assert!(mark.is_valid());
    }

    #[test]
    fn rasterize_places_texture_quadrants() {
        let (image, bounds) = quad_mark().rasterize([0.0, 0.0], 1.0).expect("raster");
        assert_eq!(bounds, [10.0, 20.0, 30.0, 40.0]);
        assert_eq!((image.width, image.height), (20, 20));
        // Sample the center of each quadrant of the rasterized quad.
        let pixel = |x: usize, y: usize| -> [u8; 4] {
            let offset = (y * image.width as usize + x) * 4;
            image.data[offset..offset + 4].try_into().unwrap()
        };
        assert_eq!(pixel(5, 5), [255, 0, 0, 255]);
        assert_eq!(pixel(15, 5), [0, 255, 0, 255]);
        assert_eq!(pixel(5, 15), [0, 0, 255, 255]);
        assert_eq!(pixel(15, 15), [255, 255, 255, 255]);
    }

    #[test]
    fn rasterize_leaves_uncovered_pixels_transparent() {
        // Single triangle: lower-left half of the quad only.
        let mark = SceneWarpedImageMark {
            indices: vec![0, 2, 3],
            ..quad_mark()
        };
        let (image, _) = mark.rasterize([0.0, 0.0], 1.0).expect("raster");
        let offset = ((2 * image.width + 17) * 4) as usize; // top-right corner
        assert_eq!(image.data[offset + 3], 0);
    }

    #[test]
    fn rasterize_capped_images_keep_the_complete_texture() {
        let mut mark = quad_mark();
        mark.positions = vec![[0.0, 0.0], [20_000.0, 0.0], [20_000.0, 2.0], [0.0, 2.0]];
        let (image, _) = mark.rasterize([0.0, 0.0], 1.0).unwrap();
        assert_eq!(image.width, 8192);
        assert_eq!(pixel_at(&image, 1, 0), [255, 0, 0, 255]);
        assert_eq!(pixel_at(&image, 8190, 0), [0, 255, 0, 255]);
    }

    #[test]
    fn rasterize_rejects_invalid_image_data_and_nonfinite_meshes() {
        let mut mark = quad_mark();
        mark.image = SceneImageSource::inline(RgbaImage {
            width: 2,
            height: 2,
            data: vec![0; 3],
        });
        assert!(mark.rasterize([0.0, 0.0], 1.0).is_none());
        let mut mark = quad_mark();
        mark.positions[0][0] = f32::NAN;
        assert!(mark.rasterize([0.0, 0.0], 1.0).is_none());
    }

    #[test]
    fn rasterize_requires_inline_image() {
        use avenger_resource::ResourceKey;

        use crate::marks::image::SceneImageResource;

        let mark = SceneWarpedImageMark {
            image: SceneImageSource::Resource(SceneImageResource {
                key: ResourceKey::new("geo/tile"),
                intrinsic_width: 2,
                intrinsic_height: 2,
                fallback_key: None,
            }),
            ..quad_mark()
        };
        assert!(mark.rasterize([0.0, 0.0], 1.0).is_none());
    }
}
