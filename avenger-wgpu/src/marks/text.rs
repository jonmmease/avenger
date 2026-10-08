use std::collections::HashMap;

use avenger_common::{
    canvas::CanvasDimensions,
    types::{PathTransform, TextAlign, TextBaseline},
};
use avenger_scenegraph::marks::text::text_origin;
use avenger_typst_label::{Label, LabelEngine, TextRasterKey};
use etagere::euclid::{Angle, Point2D, Vector2D};
use image::{DynamicImage, GenericImage, GenericImageView};
use wgpu::Extent3d;

use crate::{
    error::AvengerWgpuError,
    marks::multi::{MultiVertex, TEXT_TEXTURE_CODE, TEXT_TEXTURE_NEAREST_CODE},
};

const DEFAULT_TEXT_ATLAS_EDGE: u32 = 1024;

/// A tile of a label's raster in the text atlas: where it starts in the raster's image and its
/// size, in pixels, and its corners in the atlas, in texture coordinates.
#[derive(Clone, Copy)]
struct PlacedTile {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    tex_coords: [f32; 4],
}

#[derive(Clone)]
pub struct TextAtlasBuilder {
    text_engine: LabelEngine,
    extent: Extent3d,
    next_atlas: image::RgbaImage,
    /// The tiles of the rasters on the current atlas page.
    next_cache: HashMap<TextRasterKey, Vec<PlacedTile>>,
    atlases: Vec<DynamicImage>,
    initialized: bool,
    allocator: etagere::AtlasAllocator,
}

impl TextAtlasBuilder {
    pub fn new(text_engine: LabelEngine) -> Self {
        Self {
            text_engine,
            extent: Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            next_atlas: image::RgbaImage::new(1, 1),
            next_cache: Default::default(),
            atlases: vec![],
            initialized: false,
            allocator: etagere::AtlasAllocator::new(etagere::Size::new(1, 1)),
        }
    }

    pub fn register_text(
        &mut self,
        text: TextInstance,
        dimensions: CanvasDimensions,
    ) -> Result<Vec<TextAtlasRegistration>, AvengerWgpuError> {
        if !self.initialized {
            let limits = wgpu::Limits::downlevel_webgl2_defaults();
            self.extent = Extent3d {
                width: limits.max_texture_dimension_1d.min(DEFAULT_TEXT_ATLAS_EDGE),
                height: limits.max_texture_dimension_2d.min(DEFAULT_TEXT_ATLAS_EDGE),
                depth_or_array_layers: 1,
            };
            self.next_atlas = image::RgbaImage::new(self.extent.width, self.extent.height);
            self.allocator = etagere::AtlasAllocator::new(etagere::Size::new(
                self.extent.width as i32,
                self.extent.height as i32,
            ));
            self.initialized = true;
        }

        let raster = self.text_engine.raster(&text.label, dimensions.scale)?;
        let position = text.position;
        let [box_left, box_top] = text_origin(&raster.bounds, position, text.align, text.baseline);
        let quad = TileQuad {
            scale: dimensions.scale,
            box_left,
            box_top,
            ascent: raster.bounds.ascent,
            raster_x: raster.x,
            raster_y: raster.y,
            physical_x: (raster.x * dimensions.scale).round(),
            angle: text.angle,
            rotation: if text.angle != 0.0 {
                PathTransform::translation(-position[0], -position[1])
                    .then_rotate(Angle::degrees(text.angle))
                    .then_translate(Vector2D::new(position[0], position[1]))
            } else {
                PathTransform::identity()
            },
            texture_code: if text.use_nearest_filter {
                TEXT_TEXTURE_NEAREST_CODE
            } else {
                TEXT_TEXTURE_CODE
            },
        };

        let mut registrations: Vec<TextAtlasRegistration> = Vec::new();
        let mut verts: Vec<MultiVertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        if let Some(tiles) = self.next_cache.get(&raster.key) {
            // The raster is on the current atlas page already.
            for tile in tiles {
                quad.push(&mut verts, &mut indices, tile);
            }
        } else if let Some(image) = &raster.image {
            // A raster wider or taller than an atlas page, less its one pixel border on each
            // side, splits into tiles, each with its own allocation and quad.
            let max_tile_width = self.extent.width.saturating_sub(2).max(1);
            let max_tile_height = self.extent.height.saturating_sub(2).max(1);
            let mut tiles: Vec<PlacedTile> = Vec::new();
            let mut spans_pages = false;
            let mut tile_y = 0u32;
            while tile_y < image.height() {
                let tile_height = max_tile_height.min(image.height() - tile_y);
                let mut tile_x = 0u32;
                while tile_x < image.width() {
                    let tile_width = max_tile_width.min(image.width() - tile_x);
                    let alloc_size =
                        etagere::Size::new((tile_width + 2) as i32, (tile_height + 2) as i32);
                    let allocation = match self.allocator.allocate(alloc_size) {
                        Some(allocation) => allocation,
                        None => {
                            // The page is full: commit its quads and start a new page.
                            registrations.push(TextAtlasRegistration {
                                atlas_index: self.atlases.len(),
                                verts: std::mem::take(&mut verts),
                                indices: std::mem::take(&mut indices),
                            });
                            let full_atlas = std::mem::replace(
                                &mut self.next_atlas,
                                image::RgbaImage::new(self.extent.width, self.extent.height),
                            );
                            self.atlases
                                .push(image::DynamicImage::ImageRgba8(full_atlas));
                            self.next_cache.clear();
                            self.allocator = etagere::AtlasAllocator::new(etagere::Size::new(
                                self.extent.width as i32,
                                self.extent.height as i32,
                            ));
                            // Tiles already placed stay on the previous page, so the raster
                            // can't be cached against the new one.
                            spans_pages = !tiles.is_empty();
                            self.allocator.allocate(alloc_size).ok_or_else(|| {
                                AvengerWgpuError::ImageAllocationError(
                                    "Failed to allocate space for text raster entry".to_string(),
                                )
                            })?
                        }
                    };

                    // The tile starts a pixel in from its allocation's corner, so that linear
                    // filtering doesn't blend it with its neighbors.
                    let corner = allocation.rectangle.min;
                    let (atlas_x0, atlas_y0) = (corner.x as u32 + 1, corner.y as u32 + 1);
                    self.next_atlas
                        .copy_from(
                            &*image.view(tile_x, tile_y, tile_width, tile_height),
                            atlas_x0,
                            atlas_y0,
                        )
                        .expect("a text tile fits its atlas allocation");
                    let (width, height) = (self.extent.width as f32, self.extent.height as f32);
                    let tile = PlacedTile {
                        x: tile_x,
                        y: tile_y,
                        width: tile_width,
                        height: tile_height,
                        tex_coords: [
                            atlas_x0 as f32 / width,
                            atlas_y0 as f32 / height,
                            (atlas_x0 + tile_width) as f32 / width,
                            (atlas_y0 + tile_height) as f32 / height,
                        ],
                    };
                    // The quad goes in now, with the page that holds its tile, even if a later
                    // tile starts a new page.
                    quad.push(&mut verts, &mut indices, &tile);
                    tiles.push(tile);
                    tile_x += tile_width;
                }
                tile_y += tile_height;
            }
            if !spans_pages {
                self.next_cache.insert(raster.key.clone(), tiles);
            }
        }
        registrations.push(TextAtlasRegistration {
            atlas_index: self.atlases.len(),
            verts,
            indices,
        });
        Ok(registrations)
    }

    pub fn build(&self) -> (Extent3d, Vec<DynamicImage>) {
        let mut images = self.atlases.clone();
        images.push(image::DynamicImage::ImageRgba8(self.next_atlas.clone()));
        (self.extent, images)
    }
}

/// Where a label's raster tiles go on the canvas: the label's box, the raster's offset in it, and
/// the label's rotation about its position.
struct TileQuad {
    scale: f32,
    box_left: f32,
    box_top: f32,
    ascent: f32,
    raster_x: f32,
    raster_y: f32,
    /// The raster's left edge, on a pixel.
    physical_x: f32,
    angle: f32,
    rotation: PathTransform,
    texture_code: f32,
}

impl TileQuad {
    /// Pushes the quad of one tile: four vertices and six indices. An unrotated label's tiles
    /// start on pixels.
    fn push(&self, verts: &mut Vec<MultiVertex>, indices: &mut Vec<u32>, tile: &PlacedTile) {
        let x0 = if self.angle == 0.0 {
            (self.physical_x + tile.x as f32) / self.scale + self.box_left
        } else {
            self.raster_x + tile.x as f32 / self.scale + self.box_left
        };
        let y0 = self.ascent + self.raster_y + tile.y as f32 / self.scale + self.box_top;
        let x1 = x0 + tile.width as f32 / self.scale;
        let y1 = y0 + tile.height as f32 / self.scale;
        let corner = |x, y| self.rotation.transform_point(Point2D::new(x, y)).to_array();
        let (top_left, bottom_right) = (corner(x0, y0), corner(x1, y1));
        let [tex_x0, tex_y0, tex_x1, tex_y1] = tile.tex_coords;
        let offset = verts.len() as u32;
        for (position, tex_x, tex_y) in [
            (top_left, tex_x0, tex_y0),
            (corner(x0, y1), tex_x0, tex_y1),
            (bottom_right, tex_x1, tex_y1),
            (corner(x1, y0), tex_x1, tex_y0),
        ] {
            verts.push(MultiVertex {
                position,
                color: [self.texture_code, tex_x, tex_y, 0.0],
                top_left,
                bottom_right,
            });
        }
        indices.extend([
            offset,
            offset + 1,
            offset + 2,
            offset,
            offset + 2,
            offset + 3,
        ]);
    }
}

#[derive(Clone)]
pub struct TextAtlasRegistration {
    pub atlas_index: usize,
    pub verts: Vec<MultiVertex>,
    pub indices: Vec<u32>,
}

/// A label to draw: what it shows, and where.
#[derive(Clone, Debug)]
pub struct TextInstance<'a> {
    pub label: Label<'a>,
    /// The position that `align` and `baseline` anchor, before rotation.
    pub position: [f32; 2],
    pub align: TextAlign,
    pub baseline: TextBaseline,
    /// The rotation about the position, in degrees.
    pub angle: f32,
    /// Whether to sample the raster without smoothing, which keeps axis-aligned text sharp.
    pub use_nearest_filter: bool,
}
