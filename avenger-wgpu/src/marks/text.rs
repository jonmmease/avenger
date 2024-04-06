use crate::canvas::CanvasDimensions;
use crate::error::AvengerWgpuError;
use crate::marks::multi::{MultiVertex, TEXT_TEXTURE_CODE};
use avenger::marks::path::PathTransform;
use avenger::marks::text::{FontStyleSpec, FontWeightSpec, TextAlignSpec, TextBaselineSpec};
use etagere::euclid::{Angle, Point2D, Vector2D};
use image::DynamicImage;
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;
use wgpu::Extent3d;

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
pub struct TextAtlasBuilder<CacheKey: Hash + Eq + Clone> {
    rasterizer: Arc<dyn TextRasterizer<CacheKey = CacheKey>>,
    extent: Extent3d,
    next_atlas: image::RgbaImage,
    next_cache: HashMap<CacheKey, GlyphBBoxAndAtlasCoords>,
    atlases: Vec<DynamicImage>,
    initialized: bool,
    allocator: etagere::AtlasAllocator,
}

impl<CacheKey: Hash + Eq + Clone> TextAtlasBuilder<CacheKey> {
    pub fn new(rasterizer: Arc<dyn TextRasterizer<CacheKey = CacheKey>>) -> Self {
        Self {
            rasterizer,
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

impl<CacheKey: Hash + Eq + Clone> TextAtlasBuilderTrait for TextAtlasBuilder<CacheKey> {
    fn register_text(
        &mut self,
        text: TextInstance,
        dimensions: CanvasDimensions,
    ) -> Result<Vec<TextAtlasRegistration>, AvengerWgpuError> {
        if !self.initialized {
            let limits = wgpu::Limits::downlevel_webgl2_defaults();

            // Update extent
            self.extent = Extent3d {
                width: limits.max_texture_dimension_1d.min(256),
                height: limits.max_texture_dimension_2d.min(256),
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
        let align = text.align.clone();
        let baseline = text.baseline.clone();
        let position = text.position;
        let angle = text.angle;

        let buffer = self.rasterizer.rasterize(
            dimensions,
            &TextRasterizationConfig::from(text),
            &self.next_cache,
        )?;

        let buffer_left = match align {
            TextAlignSpec::Left => position[0],
            TextAlignSpec::Center => position[0] - buffer.buffer_width / 2.0,
            TextAlignSpec::Right => position[0] - buffer.buffer_width,
        };

        let buffer_top = match baseline {
            TextBaselineSpec::Alphabetic => position[1] - buffer.buffer_line_y,
            TextBaselineSpec::Top => position[1],
            TextBaselineSpec::Middle => position[1] - buffer.buffer_height * 0.5,
            TextBaselineSpec::Bottom => position[1] - buffer.buffer_height,
            TextBaselineSpec::LineTop => todo!(),
            TextBaselineSpec::LineBottom => todo!(),
        };

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

        for (glyph_image, phys_position) in &buffer.glyphs {
            let glyph_bbox_and_atlas_coords =
                if let Some(glyph_position) = self.next_cache.get(&glyph_image.cache_key) {
                    // Glyph has already been written to atlas
                    glyph_position
                } else {
                    // Allocate space in active atlas image, leaving space for 1 pixel empty border
                    let allocation = if let Some(allocation) =
                        self.allocator.allocate(etagere::Size::new(
                            (glyph_image.bbox.width + 2) as i32,
                            (glyph_image.bbox.height + 2) as i32,
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
                            (glyph_image.bbox.width + 2) as i32,
                            (glyph_image.bbox.height + 2) as i32,
                        )) {
                            allocation
                        } else {
                            return Err(AvengerWgpuError::ImageAllocationError(
                                "Failed to allocate space for glyph".to_string(),
                            ));
                        }
                    };

                    // Write image to allocated portion of final texture image
                    // Use one pixel offset to avoid aliasing artifacts in linear interpolation
                    let p0 = allocation.rectangle.min;
                    let atlas_x0 = p0.x + 1;
                    let atlas_x1 = atlas_x0 + glyph_image.bbox.width as i32;
                    let atlas_y0 = p0.y + 1;
                    let atlas_y1 = atlas_y0 + glyph_image.bbox.height as i32;

                    let Some(img) = glyph_image.image.as_ref() else {
                        return Err(AvengerWgpuError::TextError(
                            "Expected glyph image to be available on first use".to_string(),
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
                        glyph_image.cache_key.clone(),
                        GlyphBBoxAndAtlasCoords {
                            bbox: glyph_image.bbox,
                            tex_coords: TextAtlasCoords {
                                x0: (atlas_x0 as f32) / self.extent.width as f32,
                                y0: (atlas_y0 as f32) / self.extent.height as f32,
                                x1: (atlas_x1 as f32) / self.extent.width as f32,
                                y1: (atlas_y1 as f32) / self.extent.height as f32,
                            },
                        },
                    );
                    self.next_cache.get(&glyph_image.cache_key).unwrap()
                };

            // Create verts for rectangle around glyph
            let bbox = &glyph_bbox_and_atlas_coords.bbox;
            let x0 = (phys_position.x + bbox.left as f32) / dimensions.scale + buffer_left;
            let y0 = (buffer.buffer_line_y).round()
                + (phys_position.y - bbox.top as f32) / dimensions.scale
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

            let tex_coords = glyph_bbox_and_atlas_coords.tex_coords;
            let tex_x0 = tex_coords.x0;
            let tex_y0 = tex_coords.y0;
            let tex_x1 = tex_coords.x1;
            let tex_y1 = tex_coords.y1;

            let offset = verts.len() as u32;

            verts.push(MultiVertex {
                position: top_left,
                color: [TEXT_TEXTURE_CODE, tex_x0, tex_y0, 0.0],
                top_left,
                bottom_right,
            });
            verts.push(MultiVertex {
                position: bottom_left,
                color: [TEXT_TEXTURE_CODE, tex_x0, tex_y1, 0.0],
                top_left,
                bottom_right,
            });
            verts.push(MultiVertex {
                position: bottom_right,
                color: [TEXT_TEXTURE_CODE, tex_x1, tex_y1, 0.0],
                top_left,
                bottom_right,
            });
            verts.push(MultiVertex {
                position: top_right,
                color: [TEXT_TEXTURE_CODE, tex_x1, tex_y0, 0.0],
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
    pub align: &'a TextAlignSpec,
    pub angle: f32,
    pub baseline: &'a TextBaselineSpec,
    pub font: &'a String,
    pub font_size: f32,
    pub font_weight: &'a FontWeightSpec,
    pub font_style: &'a FontStyleSpec,
    pub limit: f32,
}

// Position of glyph in text buffer
#[derive(Debug, Clone)]
pub struct PhysicalGlyphPosition {
    pub x: f32,
    pub y: f32,
}

// Position of glyph in text atlas
#[derive(Copy, Clone)]
pub struct TextAtlasCoords {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

// Glyph bounding box relative to glyph origin
#[derive(Clone, Copy)]
pub struct GlyphBBox {
    pub top: i32,
    pub left: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone)]
pub struct GlyphImage<CacheKey: Hash + Eq + Clone> {
    pub cache_key: CacheKey,
    // None if image for same CacheKey was already included
    pub image: Option<image::RgbaImage>,
    pub bbox: GlyphBBox,
}

impl<CacheKey: Hash + Eq + Clone> GlyphImage<CacheKey> {
    pub fn without_image(&self) -> Self {
        Self {
            cache_key: self.cache_key.clone(),
            image: None,
            bbox: self.bbox,
        }
    }
}

#[derive(Clone)]
pub struct GlyphBBoxAndAtlasCoords {
    pub bbox: GlyphBBox,
    pub tex_coords: TextAtlasCoords,
}

#[derive(Debug, Clone)]
pub struct TextRasterizationConfig<'a> {
    pub text: &'a String,
    pub color: &'a [f32; 4],
    pub font: &'a String,
    pub font_size: f32,
    pub font_weight: &'a FontWeightSpec,
    pub font_style: &'a FontStyleSpec,
    pub limit: f32,
}

impl<'a> From<TextInstance<'a>> for TextRasterizationConfig<'a> {
    fn from(value: TextInstance<'a>) -> Self {
        Self {
            text: value.text,
            color: value.color,
            font: value.font,
            font_size: value.font_size,
            font_weight: value.font_weight,
            font_style: value.font_style,
            limit: value.limit,
        }
    }
}

#[derive(Clone)]
pub struct TextRasterizationBuffer<CacheKey: Hash + Eq + Clone> {
    pub glyphs: Vec<(GlyphImage<CacheKey>, PhysicalGlyphPosition)>,
    pub buffer_width: f32,
    pub buffer_height: f32,
    pub buffer_line_y: f32,
}

pub trait TextRasterizer {
    type CacheKey: Hash + Eq + Clone;
    fn rasterize(
        &self,
        dimensions: CanvasDimensions,
        config: &TextRasterizationConfig,
        cached_glyphs: &HashMap<Self::CacheKey, GlyphBBoxAndAtlasCoords>,
    ) -> Result<TextRasterizationBuffer<Self::CacheKey>, AvengerWgpuError>;
}
