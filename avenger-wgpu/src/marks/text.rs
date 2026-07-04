use std::{collections::HashMap, sync::Arc};

use avenger_common::{canvas::CanvasDimensions, types::PathTransform};
use avenger_text::{
    engine::TextEngine,
    rasterization::{
        CachedTextRasterization, TextRasterBBox, TextRasterCacheKey, TextRasterPosition,
        TextRasterizationConfig,
    },
    types::{FontStyle, FontWeight, TextAlign, TextBaseline, TextSyntaxMode},
    LabelParams,
};
use etagere::euclid::{Angle, Point2D, Vector2D};
use image::DynamicImage;
use wgpu::Extent3d;

use crate::{
    error::AvengerWgpuError,
    marks::multi::{MultiVertex, TEXT_TEXTURE_CODE, TEXT_TEXTURE_NEAREST_CODE},
};

const DEFAULT_TEXT_ATLAS_EDGE: u32 = 1024;
const TEXT_RASTER_CACHE_CAPACITY: usize = 1024;

#[derive(Clone)]
pub struct TextRasterBBoxAndAtlasCoords {
    pub bbox: TextRasterBBox,
    pub tex_coords: TextAtlasCoords,
}

// Position of a text raster entry in the text atlas.
#[derive(Copy, Clone)]
pub struct TextAtlasCoords {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

pub trait TextAtlasBuilderTrait {
    fn register_text(
        &mut self,
        text: TextInstance,
        dimensions: CanvasDimensions,
    ) -> Result<Vec<TextAtlasRegistration>, AvengerWgpuError>;

    fn build(&self) -> (Extent3d, Vec<DynamicImage>);

    fn reset(&mut self) {}
}

#[derive(Clone)]
pub struct NullTextAtlasBuilder;

impl TextAtlasBuilderTrait for NullTextAtlasBuilder {
    fn register_text(
        &mut self,
        _text: TextInstance,
        _dimensions: CanvasDimensions,
    ) -> Result<Vec<TextAtlasRegistration>, AvengerWgpuError> {
        Err(AvengerWgpuError::TextNotEnabled(
            "Text support is not enabled".to_string(),
        ))
    }

    fn build(&self) -> (Extent3d, Vec<DynamicImage>) {
        (
            Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            vec![DynamicImage::ImageRgba8(image::RgbaImage::new(1, 1))],
        )
    }
}

#[derive(Clone)]
pub struct TextAtlasBuilder {
    text_engine: Arc<TextEngine>,
    extent: Extent3d,
    next_atlas: image::RgbaImage,
    next_cache: HashMap<TextRasterCacheKey, Vec<TextRasterBBoxAndAtlasCoords>>,
    raster_cache: HashMap<TextRasterCacheKey, CachedTextRasterization>,
    atlases: Vec<DynamicImage>,
    initialized: bool,
    allocator: etagere::AtlasAllocator,
}

impl TextAtlasBuilder {
    pub fn new(text_engine: Arc<TextEngine>) -> Self {
        Self {
            text_engine,
            extent: Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            next_atlas: image::RgbaImage::new(1, 1),
            next_cache: Default::default(),
            raster_cache: Default::default(),
            atlases: vec![],
            initialized: false,
            allocator: etagere::AtlasAllocator::new(etagere::Size::new(1, 1)),
        }
    }
}

impl TextAtlasBuilderTrait for TextAtlasBuilder {
    fn reset(&mut self) {
        self.extent = Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        };
        self.next_atlas = image::RgbaImage::new(1, 1);
        self.next_cache.clear();
        self.atlases.clear();
        self.initialized = false;
        self.allocator = etagere::AtlasAllocator::new(etagere::Size::new(1, 1));
    }

    fn register_text(
        &mut self,
        text: TextInstance,
        dimensions: CanvasDimensions,
    ) -> Result<Vec<TextAtlasRegistration>, AvengerWgpuError> {
        if !self.initialized {
            let limits = wgpu::Limits::downlevel_webgl2_defaults();

            // Update extent
            self.extent = Extent3d {
                width: limits.max_texture_dimension_1d.min(DEFAULT_TEXT_ATLAS_EDGE),
                height: limits.max_texture_dimension_2d.min(DEFAULT_TEXT_ATLAS_EDGE),
                depth_or_array_layers: 1,
            };

            // Create backing image
            self.next_atlas = image::RgbaImage::new(self.extent.width, self.extent.height);

            // Create allocator
            self.allocator = etagere::AtlasAllocator::new(etagere::Size::new(
                self.extent.width as i32,
                self.extent.height as i32,
            ));

            // Set initialized
            self.initialized = true;
        }

        // Extract values we need from text instance before passing to buffer constructor
        let align = *text.align;
        let baseline = *text.baseline;
        let position = text.position;
        let angle = text.angle;
        let use_nearest_filter = text.use_nearest_filter;

        let buffer = self.text_engine.rasterize_with_plain_fallback(
            &TextRasterizationConfig {
                text: text.text,
                color: *text.color,
                font: text.font,
                font_size: text.font_size,
                font_weight: *text.font_weight,
                font_style: *text.font_style,
                limit: text.limit,
                syntax_mode: text.syntax_mode,
                params: text.params,
                number_locale: text.number_locale,
                number_locale_specs: Some(text.number_locale_specs),
                datetime_locale: text.datetime_locale,
                datetime_timezone: text.datetime_timezone,
                datetime_locale_specs: Some(text.datetime_locale_specs),
            },
            dimensions.scale,
            &self.raster_cache,
        )?;
        self.remember_text_rasters(&buffer);

        let [buffer_left, buffer_top] = buffer
            .text_bounds
            .calculate_origin(position, &align, &baseline);

        // Build rotation_transform
        let rotation_transform = if angle != 0.0 {
            PathTransform::translation(-position[0], -position[1])
                .then_rotate(Angle::degrees(angle))
                .then_translate(Vector2D::new(position[0], position[1]))
        } else {
            PathTransform::identity()
        };

        let mut registrations: Vec<TextAtlasRegistration> = Vec::new();
        let mut verts: Vec<MultiVertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();

        let texture_code = if use_nearest_filter {
            TEXT_TEXTURE_NEAREST_CODE
        } else {
            TEXT_TEXTURE_CODE
        };
        let ascent = buffer.text_bounds.ascent;

        for (entry, entry_pos) in &buffer.entries {
            if let Some(placements) = self.next_cache.get(&entry.cache_key) {
                // Text raster entry has already been written to the current atlas.
                for placement in placements {
                    push_entry_quad(
                        &mut verts,
                        &mut indices,
                        placement,
                        entry_pos,
                        dimensions.scale,
                        buffer_left,
                        buffer_top,
                        ascent,
                        angle,
                        &rotation_transform,
                        texture_code,
                    );
                }
                continue;
            }

            let Some(img) = entry.image.as_ref() else {
                return Err(AvengerWgpuError::TextError(
                    "Expected text raster image to be available on first use".to_string(),
                ));
            };

            // Entries wider or taller than an atlas page (minus the 1 pixel empty
            // border on each side) are split into tiles, each with its own atlas
            // allocation and quad, so a single long line can't fail allocation.
            let max_tile_width = self.extent.width.saturating_sub(2).max(1);
            let max_tile_height = self.extent.height.saturating_sub(2).max(1);

            let mut placements: Vec<TextRasterBBoxAndAtlasCoords> = Vec::new();
            let mut first_tile_page: Option<usize> = None;
            let mut spans_pages = false;

            let mut tile_y = 0u32;
            while tile_y < entry.bbox.height {
                let tile_height = max_tile_height.min(entry.bbox.height - tile_y);
                let mut tile_x = 0u32;
                while tile_x < entry.bbox.width {
                    let tile_width = max_tile_width.min(entry.bbox.width - tile_x);

                    // Allocate space in active atlas image, leaving space for 1 pixel empty border
                    let alloc_size =
                        etagere::Size::new((tile_width + 2) as i32, (tile_height + 2) as i32);
                    let allocation = if let Some(allocation) = self.allocator.allocate(alloc_size) {
                        // Successfully allocated space in the active atlas
                        allocation
                    } else {
                        // No more room in active atlas

                        // Commit current registration
                        let mut full_verts = Vec::new();
                        let mut full_inds = Vec::new();
                        std::mem::swap(&mut full_verts, &mut verts);
                        std::mem::swap(&mut full_inds, &mut indices);

                        registrations.push(TextAtlasRegistration {
                            atlas_index: self.atlases.len(),
                            verts: full_verts,
                            indices: full_inds,
                        });

                        // Store atlas image and create fresh image
                        let mut full_atlas =
                            image::RgbaImage::new(self.extent.width, self.extent.height);
                        std::mem::swap(&mut full_atlas, &mut self.next_atlas);
                        self.atlases
                            .push(image::DynamicImage::ImageRgba8(full_atlas));

                        // Clear cache, since this reflects the current atlas
                        self.next_cache.clear();

                        // Create fresh allocator
                        self.allocator = etagere::AtlasAllocator::new(etagere::Size::new(
                            self.extent.width as i32,
                            self.extent.height as i32,
                        ));

                        // Tiles already placed for this entry stay on the previous
                        // page, so the entry can't be cached against the new page.
                        spans_pages = first_tile_page.is_some();

                        // Try allocation again
                        if let Some(allocation) = self.allocator.allocate(alloc_size) {
                            allocation
                        } else {
                            return Err(AvengerWgpuError::ImageAllocationError(
                                "Failed to allocate space for text raster entry".to_string(),
                            ));
                        }
                    };
                    if first_tile_page.is_none() {
                        first_tile_page = Some(self.atlases.len());
                    }

                    // Write tile to allocated portion of final texture image
                    // Use one pixel offset to avoid aliasing artifacts in linear interpolation
                    let p0 = allocation.rectangle.min;
                    let atlas_x0 = p0.x + 1;
                    let atlas_x1 = atlas_x0 + tile_width as i32;
                    let atlas_y0 = p0.y + 1;
                    let atlas_y1 = atlas_y0 + tile_height as i32;

                    for (src_x, dest_x) in (atlas_x0..atlas_x1).enumerate() {
                        for (src_y, dest_y) in (atlas_y0..atlas_y1).enumerate() {
                            self.next_atlas.put_pixel(
                                dest_x as u32,
                                dest_y as u32,
                                *img.get_pixel(tile_x + src_x as u32, tile_y + src_y as u32),
                            );
                        }
                    }

                    let placement = TextRasterBBoxAndAtlasCoords {
                        bbox: TextRasterBBox {
                            top: entry.bbox.top - tile_y as i32,
                            left: entry.bbox.left + tile_x as i32,
                            width: tile_width,
                            height: tile_height,
                        },
                        tex_coords: TextAtlasCoords {
                            x0: (atlas_x0 as f32) / self.extent.width as f32,
                            y0: (atlas_y0 as f32) / self.extent.height as f32,
                            x1: (atlas_x1 as f32) / self.extent.width as f32,
                            y1: (atlas_y1 as f32) / self.extent.height as f32,
                        },
                    };

                    // Emit the quad now so it lands in the registration for the
                    // page holding this tile, even if a later tile of the same
                    // entry forces a new page.
                    push_entry_quad(
                        &mut verts,
                        &mut indices,
                        &placement,
                        entry_pos,
                        dimensions.scale,
                        buffer_left,
                        buffer_top,
                        ascent,
                        angle,
                        &rotation_transform,
                        texture_code,
                    );
                    placements.push(placement);

                    tile_x += tile_width;
                }
                tile_y += tile_height;
            }

            if !spans_pages {
                self.next_cache.insert(entry.cache_key.clone(), placements);
            }
        }

        // Add final registration
        registrations.push(TextAtlasRegistration {
            atlas_index: self.atlases.len(),
            verts,
            indices,
        });

        Ok(registrations)
    }

    fn build(&self) -> (Extent3d, Vec<DynamicImage>) {
        let mut images = self.atlases.clone();
        images.push(image::DynamicImage::ImageRgba8(self.next_atlas.clone()));
        (self.extent, images)
    }
}

impl TextAtlasBuilder {
    fn remember_text_rasters(
        &mut self,
        buffer: &avenger_text::rasterization::TextRasterizationBuffer<TextRasterCacheKey>,
    ) {
        for (entry, position) in &buffer.entries {
            if entry.image.is_none() || self.raster_cache.contains_key(&entry.cache_key) {
                continue;
            }
            if self.raster_cache.len() >= TEXT_RASTER_CACHE_CAPACITY {
                self.raster_cache.clear();
            }
            self.raster_cache.insert(
                entry.cache_key.clone(),
                CachedTextRasterization {
                    entries: vec![(entry.clone(), position.clone())],
                    text_bounds: buffer.text_bounds.clone(),
                },
            );
        }
    }
}

#[derive(Clone)]
pub struct TextAtlasRegistration {
    pub atlas_index: usize,
    pub verts: Vec<MultiVertex>,
    pub indices: Vec<u32>,
}

/// Push the quad (4 verts, 6 indices) for one placed text raster tile.
#[allow(clippy::too_many_arguments)]
fn push_entry_quad(
    verts: &mut Vec<MultiVertex>,
    indices: &mut Vec<u32>,
    placement: &TextRasterBBoxAndAtlasCoords,
    entry_pos: &TextRasterPosition,
    scale: f32,
    buffer_left: f32,
    buffer_top: f32,
    ascent: f32,
    angle: f32,
    rotation_transform: &PathTransform,
    texture_code: f32,
) {
    let bbox = &placement.bbox;
    let x0 = if angle == 0.0 {
        (entry_pos.physical_x + bbox.left as f32) / scale + buffer_left
    } else {
        entry_pos.x + bbox.left as f32 / scale + buffer_left
    };
    let y0 = ascent + entry_pos.y - bbox.top as f32 / scale + buffer_top;
    let x1 = x0 + bbox.width as f32 / scale;
    let y1 = y0 + bbox.height as f32 / scale;

    let top_left = rotation_transform
        .transform_point(Point2D::new(x0, y0))
        .to_array();
    let bottom_left = rotation_transform
        .transform_point(Point2D::new(x0, y1))
        .to_array();
    let bottom_right = rotation_transform
        .transform_point(Point2D::new(x1, y1))
        .to_array();
    let top_right = rotation_transform
        .transform_point(Point2D::new(x1, y0))
        .to_array();

    let tex_coords = placement.tex_coords;
    let tex_x0 = tex_coords.x0;
    let tex_y0 = tex_coords.y0;
    let tex_x1 = tex_coords.x1;
    let tex_y1 = tex_coords.y1;

    let offset = verts.len() as u32;

    verts.push(MultiVertex {
        position: top_left,
        color: [texture_code, tex_x0, tex_y0, 0.0],
        top_left,
        bottom_right,
    });
    verts.push(MultiVertex {
        position: bottom_left,
        color: [texture_code, tex_x0, tex_y1, 0.0],
        top_left,
        bottom_right,
    });
    verts.push(MultiVertex {
        position: bottom_right,
        color: [texture_code, tex_x1, tex_y1, 0.0],
        top_left,
        bottom_right,
    });
    verts.push(MultiVertex {
        position: top_right,
        color: [texture_code, tex_x1, tex_y0, 0.0],
        top_left,
        bottom_right,
    });

    indices.extend([
        offset,
        offset + 1,
        offset + 2,
        offset,
        offset + 2,
        offset + 3,
    ])
}

#[derive(Clone, Debug)]
pub struct TextInstance<'a> {
    pub position: [f32; 2],
    pub text: &'a String,
    pub color: &'a [f32; 4],
    pub align: &'a TextAlign,
    pub angle: f32,
    pub baseline: &'a TextBaseline,
    pub font: &'a String,
    pub font_size: f32,
    pub font_weight: &'a FontWeight,
    pub font_style: &'a FontStyle,
    pub limit: f32,
    pub syntax_mode: TextSyntaxMode,
    pub params: &'a LabelParams,
    pub number_locale: Option<&'a str>,
    pub number_locale_specs: &'a avenger_text::NumberLocaleSpecs,
    pub datetime_locale: Option<&'a str>,
    pub datetime_timezone: Option<&'a str>,
    pub datetime_locale_specs: &'a avenger_text::DateTimeLocaleSpecs,
    pub use_nearest_filter: bool,
}
