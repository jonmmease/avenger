use std::sync::Arc;

use avenger_common::hash::hash_floats;
use avenger_image::RgbaImage;
use educe::Educe;
use serde::{Deserialize, Serialize};

use super::image::SceneImageSource;
use super::mark::SceneMark;

/// The most pixels a side of a rasterized mesh image mark.
const MAX_CANVAS: usize = 8192;

/// One image of a [`SceneMeshImageMark`], on its own triangle mesh. Each vertex has a position in
/// scene coordinates and a texture coordinate in `[0, 1]` into the image, and `indices` lists the
/// triangles, three vertices each.
#[derive(Debug, Clone, PartialEq, Educe, Serialize, Deserialize)]
#[educe(Hash)]
#[serde(rename_all = "kebab-case")]
pub struct MeshImage {
    /// The image's source. The texture coordinates span whatever pixels it draws.
    pub image: SceneImageSource,
    #[educe(Hash(method(hash_floats)))]
    pub positions: Vec<[f32; 2]>,
    #[educe(Hash(method(hash_floats)))]
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

impl MeshImage {
    /// True when the vertex and index buffers agree and are finite, and describe at least one
    /// triangle. Renderers draw nothing for an invalid instance.
    pub fn is_valid(&self) -> bool {
        self.positions.len() == self.uvs.len()
            && self
                .positions
                .iter()
                .chain(self.uvs.iter())
                .flatten()
                .all(|value| value.is_finite())
            && !self.indices.is_empty()
            && self.indices.len().is_multiple_of(3)
            && self
                .indices
                .iter()
                .all(|index| (*index as usize) < self.positions.len())
    }

    /// Bounding box of the triangles' vertices as `[min_x, min_y, max_x, max_y]` in the coordinate
    /// space of `origin`. `None` for an instance without triangles.
    fn bounds(&self, origin: [f32; 2]) -> Option<[f32; 4]> {
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
}

/// Images drawn on triangle meshes instead of axis-aligned rectangles. Each instance is an image
/// on its own mesh, and the instances draw in order.
///
/// A mesh image can cover a curved region, such as a map tile reprojected from Web Mercator onto
/// another projection, which the mesh approximates. A layer of map tiles is one mark with a tile
/// for each instance, so the tiles draw, hit-test, and export as one layer.
#[derive(Debug, Clone, PartialEq, Educe, Serialize, Deserialize)]
#[educe(Hash)]
#[serde(rename_all = "kebab-case")]
pub struct SceneMeshImageMark {
    pub name: String,
    /// Whether hits find the mark. `None` takes the enclosing group's setting, and marks at the
    /// scene root are interactive.
    pub interactive: Option<bool>,
    pub clip: bool,
    pub smooth: bool,
    pub instances: Vec<MeshImage>,
    pub zindex: Option<i32>,
}

impl SceneMeshImageMark {
    /// Bounding box of the valid instances' meshes as `[min_x, min_y, max_x, max_y]` in the
    /// coordinate space of `origin`. `None` when no instance has a valid mesh.
    pub fn bounds(&self, origin: [f32; 2]) -> Option<[f32; 4]> {
        self.instances
            .iter()
            .filter(|instance| instance.is_valid())
            .filter_map(|instance| instance.bounds(origin))
            .reduce(|[a0, a1, a2, a3], [b0, b1, b2, b3]| {
                [a0.min(b0), a1.min(b1), a2.max(b2), a3.max(b3)]
            })
    }

    /// Software-rasterize the instances into one RGBA image for static export backends (SVG/PDF)
    /// that have no textured-mesh primitive. One image for every instance means adjacent
    /// instances, such as map tiles, meet without a seam.
    ///
    /// `scale` is the supersampling factor (output pixels per scene unit). The image's pixels lie
    /// on the grid of `1 / scale` scene units from the scene's origin, so they line up with the
    /// page's pixels. An image that would be wider or taller than 8192 pixels halves the scale
    /// until it fits, which keeps it on the grid. `resolved` finds the images of resource
    /// instances by URL, as [`SceneImageSource::pixels`] does. Returns the image plus its
    /// placement `[min_x, min_y, max_x, max_y]` in the coordinate space of `origin`, or `None`
    /// when no instance has a valid mesh and pixels.
    pub fn rasterize(
        &self,
        origin: [f32; 2],
        scale: f32,
        resolved: &dyn Fn(&str) -> Option<Arc<RgbaImage>>,
    ) -> Option<(RgbaImage, [f32; 4])> {
        let instances: Vec<(&MeshImage, Arc<RgbaImage>)> = self
            .instances
            .iter()
            .filter(|instance| instance.is_valid())
            .filter_map(|instance| {
                let image = instance.image.pixels(resolved)?;
                image.has_pixels().then_some((instance, image))
            })
            .collect();
        if instances.is_empty() || origin.iter().any(|value| !value.is_finite()) {
            return None;
        }
        let [min_x, min_y, max_x, max_y] = instances
            .iter()
            .filter_map(|(instance, _)| instance.bounds(origin))
            .reduce(|[a0, a1, a2, a3], [b0, b1, b2, b3]| {
                [a0.min(b0), a1.min(b1), a2.max(b2), a3.max(b3)]
            })?;
        if ![min_x, min_y, max_x, max_y]
            .iter()
            .all(|value| value.is_finite())
            || max_x <= min_x
            || max_y <= min_y
        {
            return None;
        }
        let mut scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        // The image's first pixel on the grid, and its size in pixels.
        let canvas = |scale: f32| {
            let [left, top] = [(min_x * scale).floor(), (min_y * scale).floor()];
            let width = ((max_x * scale).ceil() - left).max(1.0) as usize;
            let height = ((max_y * scale).ceil() - top).max(1.0) as usize;
            (left, top, width, height)
        };
        let (mut left, mut top, mut out_width, mut out_height) = canvas(scale);
        while out_width > MAX_CANVAS || out_height > MAX_CANVAS {
            scale /= 2.0;
            (left, top, out_width, out_height) = canvas(scale);
        }
        let mut data = vec![0u8; out_width * out_height * 4];

        for (instance, image) in instances {
            let source = image.as_ref();
            // The instance draws into its own layer over its bounds, which then composites over
            // the instances below it. Pixels on the edges that its triangles share draw twice
            // into the layer, but composite once.
            let pixel =
                |p: &[f32; 2], axis: usize, offset: f32| (p[axis] + origin[axis]) * scale - offset;
            let bound = |axis: usize, offset: f32, limit: usize| {
                let (min, max) = instance.positions.iter().fold(
                    (f32::INFINITY, f32::NEG_INFINITY),
                    |(min, max), p| {
                        let v = pixel(p, axis, offset);
                        (min.min(v), max.max(v))
                    },
                );
                let start = min.floor().max(0.0) as usize;
                (start.min(limit), (max.ceil().max(0.0) as usize).min(limit))
            };
            let (layer_x, layer_x_end) = bound(0, left, out_width);
            let (layer_y, layer_y_end) = bound(1, top, out_height);
            let layer_width = layer_x_end - layer_x;
            let mut layer = vec![0u8; layer_width * (layer_y_end - layer_y) * 4];
            for triangle in instance.indices.chunks_exact(3) {
                let [a, b, c] = [
                    triangle[0] as usize,
                    triangle[1] as usize,
                    triangle[2] as usize,
                ];
                let pos = |i: usize| -> [f32; 2] {
                    [
                        (instance.positions[i][0] + origin[0]) * scale - left,
                        (instance.positions[i][1] + origin[1]) * scale - top,
                    ]
                };
                let (pa, pb, pc) = (pos(a), pos(b), pos(c));
                let area = (pb[0] - pa[0]) * (pc[1] - pa[1]) - (pc[0] - pa[0]) * (pb[1] - pa[1]);
                if area.abs() < f32::EPSILON {
                    continue;
                }
                let x_start = (pa[0].min(pb[0]).min(pc[0]).floor().max(0.0) as usize).max(layer_x);
                let x_end = (pa[0].max(pb[0]).max(pc[0]).ceil() as usize).min(layer_x_end);
                let y_start = (pa[1].min(pb[1]).min(pc[1]).floor().max(0.0) as usize).max(layer_y);
                let y_end = (pa[1].max(pb[1]).max(pc[1]).ceil() as usize).min(layer_y_end);

                for py in y_start..y_end {
                    for px in x_start..x_end {
                        let p = [px as f32 + 0.5, py as f32 + 0.5];
                        let w_a = ((pb[0] - p[0]) * (pc[1] - p[1])
                            - (pc[0] - p[0]) * (pb[1] - p[1]))
                            / area;
                        let w_b = ((pc[0] - p[0]) * (pa[1] - p[1])
                            - (pa[0] - p[0]) * (pc[1] - p[1]))
                            / area;
                        let w_c = 1.0 - w_a - w_b;
                        // Small negative tolerance keeps shared triangle edges
                        // free of dropout from floating-point rounding.
                        const EDGE_TOLERANCE: f32 = -1e-4;
                        if w_a < EDGE_TOLERANCE || w_b < EDGE_TOLERANCE || w_c < EDGE_TOLERANCE {
                            continue;
                        }
                        let uvs = &instance.uvs;
                        let u = w_a * uvs[a][0] + w_b * uvs[b][0] + w_c * uvs[c][0];
                        let v = w_a * uvs[a][1] + w_b * uvs[b][1] + w_c * uvs[c][1];
                        let rgba = if self.smooth {
                            sample_bilinear(source, u, v)
                        } else {
                            sample_nearest(source, u, v)
                        };
                        let offset = ((py - layer_y) * layer_width + px - layer_x) * 4;
                        layer[offset..offset + 4].copy_from_slice(&rgba);
                    }
                }
            }
            for (row, line) in layer.chunks_exact(layer_width.max(1) * 4).enumerate() {
                let start = ((layer_y + row) * out_width + layer_x) * 4;
                let below = &mut data[start..start + line.len()];
                for (dst, src) in below.chunks_exact_mut(4).zip(line.chunks_exact(4)) {
                    source_over(dst, src);
                }
            }
        }

        Some((
            RgbaImage {
                width: out_width as u32,
                height: out_height as u32,
                data,
            },
            [
                left / scale,
                top / scale,
                (left + out_width as f32) / scale,
                (top + out_height as f32) / scale,
            ],
        ))
    }
}

/// Composite the straight-alpha pixel `src` over `dst`.
fn source_over(dst: &mut [u8], src: &[u8]) {
    match src[3] {
        0 => {}
        255 => dst.copy_from_slice(src),
        alpha => {
            let src_alpha = f32::from(alpha) / 255.0;
            let dst_alpha = f32::from(dst[3]) / 255.0 * (1.0 - src_alpha);
            let alpha = src_alpha + dst_alpha;
            for channel in 0..3 {
                let color =
                    f32::from(src[channel]) * src_alpha + f32::from(dst[channel]) * dst_alpha;
                dst[channel] = (color / alpha).round() as u8;
            }
            dst[3] = (alpha * 255.0).round() as u8;
        }
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
    let weights = [
        (1.0 - tx) * (1.0 - ty),
        tx * (1.0 - ty),
        (1.0 - tx) * ty,
        tx * ty,
    ];
    let samples = [p00, p10, p01, p11];
    let alpha: f32 = samples
        .iter()
        .zip(weights)
        .map(|(p, w)| p[3] as f32 * w)
        .sum();
    let mut rgba = [0; 4];
    if alpha > 0.0 {
        for channel in 0..3 {
            let premultiplied: f32 = samples
                .iter()
                .zip(weights)
                .map(|(p, w)| p[channel] as f32 * p[3] as f32 * w)
                .sum();
            rgba[channel] = (premultiplied / alpha).round().clamp(0.0, 255.0) as u8;
        }
        rgba[3] = alpha.round().clamp(0.0, 255.0) as u8;
    }

    rgba
}

fn pixel_at(image: &RgbaImage, x: usize, y: usize) -> [u8; 4] {
    let offset = (y * image.width as usize + x) * 4;
    image.data[offset..offset + 4]
        .try_into()
        .unwrap_or([0, 0, 0, 0])
}

impl Default for SceneMeshImageMark {
    fn default() -> Self {
        Self {
            name: "mesh_image_mark".to_string(),
            interactive: Some(false),
            clip: true,
            smooth: true,
            instances: Vec::new(),
            zindex: None,
        }
    }
}

impl From<SceneMeshImageMark> for SceneMark {
    fn from(mark: SceneMeshImageMark) -> Self {
        SceneMark::MeshImage(Arc::new(mark))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::marks::image::SceneImageResource;

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

    fn quad(x: f32) -> MeshImage {
        MeshImage {
            image: checker_image().into(),
            positions: vec![[x, 20.0], [x + 20.0, 20.0], [x + 20.0, 40.0], [x, 40.0]],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            indices: vec![0, 1, 2, 0, 2, 3],
        }
    }

    fn quad_mark() -> SceneMeshImageMark {
        SceneMeshImageMark {
            instances: vec![quad(10.0)],
            smooth: false,
            ..Default::default()
        }
    }

    #[test]
    fn bilinear_sampling_ignores_hidden_rgb_and_returns_straight_alpha() {
        for hidden in [[0, 0, 255], [0, 255, 0]] {
            let image = RgbaImage {
                width: 2,
                height: 1,
                data: vec![255, 0, 0, 255, hidden[0], hidden[1], hidden[2], 0],
            };
            assert_eq!(sample_bilinear(&image, 0.5, 0.5), [255, 0, 0, 128]);
            assert_eq!(sample_bilinear(&image, 1.0, 0.5), [0; 4]);
            assert_eq!(sample_nearest(&image, 0.0, 0.5), [255, 0, 0, 255]);
        }
        let image = RgbaImage {
            width: 2,
            height: 1,
            data: vec![200, 40, 100, 128, 40, 200, 100, 64],
        };
        assert_eq!(sample_bilinear(&image, 0.5, 0.5), [147, 93, 100, 96]);
    }

    #[test]
    fn bounds_cover_indexed_vertices_with_origin() {
        let mark = quad_mark();
        assert_eq!(mark.bounds([5.0, 1.0]), Some([15.0, 21.0, 35.0, 41.0]));
        assert!(mark.instances[0].is_valid());
    }

    /// Two instances side by side rasterize into one image, which their bounds cover, with no
    /// uncovered column where they meet.
    #[test]
    fn instances_rasterize_into_one_image() {
        let mark = SceneMeshImageMark {
            instances: vec![quad(10.0), quad(30.0)],
            smooth: false,
            ..Default::default()
        };
        assert_eq!(mark.bounds([0.0, 0.0]), Some([10.0, 20.0, 50.0, 40.0]));
        let (image, bounds) = mark.rasterize([0.0, 0.0], 1.0, &|_| None).expect("raster");
        assert_eq!(bounds, [10.0, 20.0, 50.0, 40.0]);
        assert_eq!((image.width, image.height), (40, 20));
        for x in 0..40 {
            assert_eq!(pixel_at(&image, x, 10)[3], 255, "column {x}");
        }
        // The second instance's first quadrant starts where the first instance ends.
        assert_eq!(pixel_at(&image, 19, 5), [0, 255, 0, 255]);
        assert_eq!(pixel_at(&image, 20, 5), [255, 0, 0, 255]);
    }

    /// An instance composites over the instances below it, as the GPU blends them: a clear
    /// pixel leaves the one below, and a translucent pixel mixes with it. A translucent pixel
    /// on the diagonal that the quad's two triangles share composites once.
    #[test]
    fn overlapping_instances_composite_over_those_below() {
        let flat = |pixel: [u8; 4]| MeshImage {
            image: RgbaImage {
                width: 1,
                height: 1,
                data: pixel.to_vec(),
            }
            .into(),
            positions: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            indices: vec![0, 1, 2, 0, 2, 3],
        };
        for (top, expected) in [
            ([0, 0, 255, 0], [255, 0, 0, 255]),
            ([0, 0, 255, 128], [127, 0, 128, 255]),
        ] {
            let mark = SceneMeshImageMark {
                instances: vec![flat([255, 0, 0, 255]), flat(top)],
                smooth: false,
                ..Default::default()
            };
            let (image, _) = mark.rasterize([0.0, 0.0], 1.0, &|_| None).unwrap();
            for (x, y) in [(3, 5), (5, 5), (8, 2)] {
                assert_eq!(pixel_at(&image, x, y), expected, "{top:?} at ({x}, {y})");
            }
        }
        let clear_below = SceneMeshImageMark {
            instances: vec![flat([0, 0, 0, 0]), flat([0, 0, 255, 128])],
            smooth: false,
            ..Default::default()
        };
        let (image, _) = clear_below.rasterize([0.0, 0.0], 1.0, &|_| None).unwrap();
        assert_eq!(pixel_at(&image, 5, 5), [0, 0, 255, 128]);
    }

    #[test]
    fn rasterize_places_texture_quadrants() {
        let (image, bounds) = quad_mark()
            .rasterize([0.0, 0.0], 1.0, &|_| None)
            .expect("raster");
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
        let mut mark = quad_mark();
        mark.instances[0].indices = vec![0, 2, 3];
        let (image, _) = mark.rasterize([0.0, 0.0], 1.0, &|_| None).expect("raster");
        let offset = ((2 * image.width + 17) * 4) as usize; // top-right corner
        assert_eq!(image.data[offset + 3], 0);
    }

    #[test]
    fn rasterize_capped_images_keep_the_complete_texture() {
        let mut mark = quad_mark();
        mark.instances[0].positions =
            vec![[0.0, 0.0], [20_000.0, 0.0], [20_000.0, 8.0], [0.0, 8.0]];
        let (image, bounds) = mark.rasterize([0.0, 0.0], 1.0, &|_| None).unwrap();
        // A quarter of the scale fits, on a grid of 4 scene units.
        assert_eq!((image.width, image.height), (5000, 2));
        assert_eq!(bounds, [0.0, 0.0, 20_000.0, 8.0]);
        assert_eq!(pixel_at(&image, 1, 0), [255, 0, 0, 255]);
        assert_eq!(pixel_at(&image, 4998, 0), [0, 255, 0, 255]);
    }

    /// The image starts and ends on the grid of the scale's pixels, measured from the scene's
    /// origin, whatever the mesh's bounds and the mark's origin.
    #[test]
    fn rasterize_aligns_to_the_pixel_grid() {
        let mut mark = quad_mark();
        mark.instances[0].positions = vec![[0.3, 0.2], [10.7, 0.2], [10.7, 5.1], [0.3, 5.1]];
        let (image, bounds) = mark.rasterize([100.25, 7.0], 2.0, &|_| None).unwrap();
        assert_eq!(bounds, [100.5, 7.0, 111.0, 12.5]);
        assert_eq!((image.width, image.height), (21, 11));
    }

    #[test]
    fn rasterize_rejects_invalid_image_data_and_nonfinite_meshes() {
        let mut mark = quad_mark();
        mark.instances[0].image = RgbaImage {
            width: 2,
            height: 2,
            data: vec![0; 3],
        }
        .into();
        assert!(mark.rasterize([0.0, 0.0], 1.0, &|_| None).is_none());
        let mut mark = quad_mark();
        mark.instances[0].positions[0][0] = f32::NAN;
        assert!(!mark.instances[0].is_valid());
        assert!(mark.rasterize([0.0, 0.0], 1.0, &|_| None).is_none());
    }

    /// A resource instance draws the image that the lookup finds, or else its placeholder, or
    /// else nothing.
    #[test]
    fn resource_instances_draw_their_lookup_or_placeholder() {
        let resource = |placeholder: Option<Arc<RgbaImage>>| {
            let mut mark = quad_mark();
            mark.instances[0].image = SceneImageResource {
                url: "tile.png".to_string(),
                width: 2,
                height: 2,
                placeholder,
            }
            .into();
            mark
        };
        let gray = Arc::new(RgbaImage {
            width: 1,
            height: 1,
            data: vec![128, 128, 128, 255],
        });
        let checker = Arc::new(checker_image());
        let found = |url: &str| (url == "tile.png").then(|| checker.clone());

        let (image, _) = resource(Some(gray.clone()))
            .rasterize([0.0, 0.0], 1.0, &found)
            .unwrap();
        assert_eq!(pixel_at(&image, 5, 5), [255, 0, 0, 255]);
        let (image, _) = resource(Some(gray))
            .rasterize([0.0, 0.0], 1.0, &|_| None)
            .unwrap();
        assert_eq!(pixel_at(&image, 5, 5), [128, 128, 128, 255]);
        assert!(resource(None)
            .rasterize([0.0, 0.0], 1.0, &|_| None)
            .is_none());
    }
}
