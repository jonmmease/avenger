use std::{collections::HashMap, sync::Arc};

use avenger_common::{canvas::CanvasDimensions, types::PathTransform};
use avenger_text::{
    engine::TextEngine,
    rasterization::{TextRasterBBox, TextRasterCacheKey, TextRasterizationConfig},
    types::{FontStyle, FontWeight, TextAlign, TextBaseline, TextSyntaxMode},
};
use etagere::euclid::{Angle, Point2D, Vector2D};
use image::DynamicImage;
use wgpu::Extent3d;

use crate::{
    error::AvengerWgpuError,
    marks::multi::{MultiVertex, TEXT_TEXTURE_CODE, TEXT_TEXTURE_NEAREST_CODE},
};

const DEFAULT_TEXT_ATLAS_EDGE: u32 = 1024;

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
    next_cache: HashMap<TextRasterCacheKey, TextRasterBBoxAndAtlasCoords>,
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
            atlases: vec![],
            initialized: false,
            allocator: etagere::AtlasAllocator::new(etagere::Size::new(1, 1)),
        }
    }
}

impl TextAtlasBuilderTrait for TextAtlasBuilder {
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
            },
            dimensions.scale,
            &self.next_cache,
        )?;

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

        for (entry, entry_pos) in &buffer.entries {
            let entry_bbox_and_atlas_coords =
                if let Some(entry_position) = self.next_cache.get(&entry.cache_key) {
                    // Text raster entry has already been written to atlas.
                    entry_position
                } else {
                    // Allocate space in active atlas image, leaving space for 1 pixel empty border
                    let allocation = if let Some(allocation) =
                        self.allocator.allocate(etagere::Size::new(
                            (entry.bbox.width + 2) as i32,
                            (entry.bbox.height + 2) as i32,
                        )) {
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

                        // Try allocation again
                        if let Some(allocation) = self.allocator.allocate(etagere::Size::new(
                            (entry.bbox.width + 2) as i32,
                            (entry.bbox.height + 2) as i32,
                        )) {
                            allocation
                        } else {
                            return Err(AvengerWgpuError::ImageAllocationError(
                                "Failed to allocate space for text raster entry".to_string(),
                            ));
                        }
                    };

                    // Write image to allocated portion of final texture image
                    // Use one pixel offset to avoid aliasing artifacts in linear interpolation
                    let p0 = allocation.rectangle.min;
                    let atlas_x0 = p0.x + 1;
                    let atlas_x1 = atlas_x0 + entry.bbox.width as i32;
                    let atlas_y0 = p0.y + 1;
                    let atlas_y1 = atlas_y0 + entry.bbox.height as i32;

                    let Some(img) = entry.image.as_ref() else {
                        return Err(AvengerWgpuError::TextError(
                            "Expected text raster image to be available on first use".to_string(),
                        ));
                    };

                    for (src_x, dest_x) in (atlas_x0..atlas_x1).enumerate() {
                        for (src_y, dest_y) in (atlas_y0..atlas_y1).enumerate() {
                            self.next_atlas.put_pixel(
                                dest_x as u32,
                                dest_y as u32,
                                *img.get_pixel(src_x as u32, src_y as u32),
                            );
                        }
                    }

                    self.next_cache.insert(
                        entry.cache_key.clone(),
                        TextRasterBBoxAndAtlasCoords {
                            bbox: entry.bbox,
                            tex_coords: TextAtlasCoords {
                                x0: (atlas_x0 as f32) / self.extent.width as f32,
                                y0: (atlas_y0 as f32) / self.extent.height as f32,
                                x1: (atlas_x1 as f32) / self.extent.width as f32,
                                y1: (atlas_y1 as f32) / self.extent.height as f32,
                            },
                        },
                    );
                    self.next_cache.get(&entry.cache_key).unwrap()
                };

            // Create verts for rectangle around the text raster entry.
            let bbox = &entry_bbox_and_atlas_coords.bbox;
            let x0 = if angle == 0.0 {
                (entry_pos.physical_x + bbox.left as f32) / dimensions.scale + buffer_left
            } else {
                entry_pos.x + bbox.left as f32 / dimensions.scale + buffer_left
            };
            let y0 = buffer.text_bounds.ascent + entry_pos.y - bbox.top as f32 / dimensions.scale
                + buffer_top;
            let x1 = x0 + bbox.width as f32 / dimensions.scale;
            let y1 = y0 + bbox.height as f32 / dimensions.scale;

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

            let tex_coords = entry_bbox_and_atlas_coords.tex_coords;
            let tex_x0 = tex_coords.x0;
            let tex_y0 = tex_coords.y0;
            let tex_x1 = tex_coords.x1;
            let tex_y1 = tex_coords.y1;

            let offset = verts.len() as u32;

            let texture_code = if use_nearest_filter {
                TEXT_TEXTURE_NEAREST_CODE
            } else {
                TEXT_TEXTURE_CODE
            };

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

#[derive(Clone)]
pub struct TextAtlasRegistration {
    pub atlas_index: usize,
    pub verts: Vec<MultiVertex>,
    pub indices: Vec<u32>,
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
    pub use_nearest_filter: bool,
}
