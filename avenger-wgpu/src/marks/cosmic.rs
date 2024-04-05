use crate::canvas::CanvasDimensions;
use crate::error::AvengerWgpuError;
use crate::marks::multi::{MultiVertex, TEXT_TEXTURE_CODE};
use crate::marks::text::{TextAtlasBuilder, TextAtlasRegistration, TextInstance};
use avenger::marks::path::PathTransform;
use avenger::marks::text::{FontStyleSpec, FontWeightNameSpec, FontWeightSpec, TextAlignSpec, TextBaselineSpec};
use cosmic_text::fontdb::Database;
use cosmic_text::{
    Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent, Weight,
};
use image::DynamicImage;
use lazy_static;
use lyon::geom::euclid::{Point2D, Vector2D};
use lyon::geom::Angle;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use wgpu::Extent3d;

lazy_static! {
    static ref FONT_SYSTEM: Mutex<FontSystem> = Mutex::new(build_font_system());
    static ref SWASH_CACHE: Mutex<SwashCache> = Mutex::new(SwashCache::new());
}

fn build_font_system() -> FontSystem {
    let mut font_system = FontSystem::new();

    // Override default families based on what system fonts are available
    let fontdb = font_system.db_mut();
    setup_default_fonts(fontdb);
    font_system
}

fn setup_default_fonts(fontdb: &mut Database) {
    let families: HashSet<String> = fontdb
        .faces()
        .flat_map(|face| {
            face.families
                .iter()
                .map(|(fam, _lang)| fam.clone())
                .collect::<Vec<_>>()
        })
        .collect();

    // Set default sans serif
    for family in ["Helvetica", "Arial", "Liberation Sans"] {
        if families.contains(family) {
            fontdb.set_sans_serif_family(family);
            break;
        }
    }

    // Set default monospace font family
    for family in [
        "Courier New",
        "Courier",
        "Liberation Mono",
        "DejaVu Sans Mono",
    ] {
        if families.contains(family) {
            fontdb.set_monospace_family(family);
            break;
        }
    }

    // Set default serif font family
    for family in [
        "Times New Roman",
        "Times",
        "Liberation Serif",
        "DejaVu Serif",
    ] {
        if families.contains(family) {
            fontdb.set_serif_family(family);
            break;
        }
    }
}

pub fn register_font_directory(dir: &str) {
    let mut font_system = FONT_SYSTEM
        .lock()
        .expect("Failed to acquire lock on FONT_SYSTEM");
    let fontdb = font_system.db_mut();
    fontdb.load_fonts_dir(dir);
    setup_default_fonts(fontdb);
}

#[derive(Copy, Clone)]
pub struct TextAtlasCoords {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

#[derive(Clone)]
pub struct GlyphPosition {
    top: f32,
    left: f32,
    width: f32,
    height: f32,
    tex_coords: TextAtlasCoords,
}

pub struct CosmicTextAtlasBuilder {
    extent: Extent3d,
    next_atlas: image::RgbaImage,
    next_cache: HashMap<(cosmic_text::CacheKey, [u8; 4]), GlyphPosition>,
    atlases: Vec<DynamicImage>,
    initialized: bool,
    allocator: etagere::AtlasAllocator,
}

impl Default for CosmicTextAtlasBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl CosmicTextAtlasBuilder {
    pub fn new() -> Self {
        Self {
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

impl TextAtlasBuilder for CosmicTextAtlasBuilder {
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

        let buffer = TextRasterizationBuffer::new(dimensions, &TextRasterizationBufferConfig::from(text))?;

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

        for (raster_details, phys_position) in &buffer.glyphs {
            let glyph_position = if let Some(glyph_position) = self.next_cache.get(&raster_details.cache_key) {
                // Glyph has already been written to atlas
                glyph_position
            } else {
                // Allocate space in active atlas image, leaving space for 1 pixel empty border
                let allocation = if let Some(allocation) = self
                    .allocator
                    .allocate(etagere::Size::new((raster_details.width + 2) as i32, (raster_details.height + 2) as i32))
                {
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
                    if let Some(allocation) = self
                        .allocator
                        .allocate(etagere::Size::new((raster_details.width + 2) as i32, (raster_details.height + 2) as i32))
                    {
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
                let atlas_x1 = atlas_x0 + raster_details.width as i32;
                let atlas_y0 = p0.y + 1;
                let atlas_y1 = atlas_y0 + raster_details.height as i32;

                let img = &buffer.images[raster_details.image_index];
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
                    raster_details.cache_key,
                    GlyphPosition {
                        top: raster_details.top as f32,
                        left: raster_details.left as f32,
                        width: raster_details.width as f32,
                        height: raster_details.height as f32,
                        tex_coords: TextAtlasCoords {
                            x0: (atlas_x0 as f32) / self.extent.width as f32,
                            y0: (atlas_y0 as f32) / self.extent.height as f32,
                            x1: (atlas_x1 as f32) / self.extent.width as f32,
                            y1: (atlas_y1 as f32) / self.extent.height as f32,
                        },
                    },
                );
                self.next_cache.get(&raster_details.cache_key).unwrap()
            };

            // Create verts for rectangle around glyph
            let x0 = (phys_position.x as f32 + glyph_position.left) / dimensions.scale + buffer_left;
            let y0 = (buffer.buffer_line_y).round()
                + (phys_position.y as f32 - glyph_position.top) / dimensions.scale + buffer_top;
            let x1 = x0 + glyph_position.width / dimensions.scale;
            let y1 = y0 + glyph_position.height / dimensions.scale;

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

            let tex_x0 = glyph_position.tex_coords.x0;
            let tex_y0 = glyph_position.tex_coords.y0;
            let tex_x1 = glyph_position.tex_coords.x1;
            let tex_y1 = glyph_position.tex_coords.y1;

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

pub fn measure(buffer: &Buffer) -> (f32, f32, f32) {
    let (width, line_y, total_lines) =
        buffer
            .layout_runs()
            .fold((0.0, 0.0, 0usize), |(width, line_y, total_lines), run| {
                (
                    run.line_w.max(width),
                    run.line_y.max(line_y),
                    total_lines + 1,
                )
            });
    (
        width,
        line_y,
        total_lines as f32 * buffer.metrics().line_height,
    )
}

#[derive(Debug, Clone)]
struct TextRasterizationGlyphDetails {
    pub cache_key: (cosmic_text::CacheKey, [u8; 4]),
    pub image_index: usize,
    pub top: i32,
    pub left: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone)]
struct PhysicalGlyphPosition {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone)]
struct TextRasterizationBufferConfig<'a> {
    pub text: &'a String,
    pub color: &'a [f32; 4],
    pub font: &'a String,
    pub font_size: f32,
    pub font_weight: &'a FontWeightSpec,
    pub font_style: &'a FontStyleSpec,
    pub limit: f32,
}

impl <'a> From<TextInstance<'a>> for TextRasterizationBufferConfig<'a> {
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

#[derive(Debug, Clone)]
struct TextRasterizationBuffer {
    pub images: Vec<image::RgbaImage>,
    pub glyphs: Vec<(TextRasterizationGlyphDetails, PhysicalGlyphPosition)>,
    pub buffer_width: f32,
    pub buffer_height: f32,
    pub buffer_line_y: f32,
}

impl TextRasterizationBuffer {
    pub fn new(dimensions: CanvasDimensions, config: &TextRasterizationBufferConfig) -> Result<Self, AvengerWgpuError> {

        let mut font_system = FONT_SYSTEM
            .lock()
            .expect("Failed to acquire lock on FONT_SYSTEM");
        let mut cache = SWASH_CACHE
            .lock()
            .expect("Failed to acquire lock on SWASH_CACHE");

        // Build image cache
        let mut next_cache: HashMap<(cosmic_text::CacheKey, [u8; 4]), TextRasterizationGlyphDetails> = HashMap::new();

        // Build cosmic-text Buffer
        let mut buffer = Buffer::new(
            &mut font_system,
            Metrics::new(config.font_size, config.font_size),
        );
        let family = match config.font.to_lowercase().as_str() {
            "serif" => Family::Serif,
            "sans serif" | "sans-serif" => Family::SansSerif,
            "cursive" => Family::Cursive,
            "fantasy" => Family::Fantasy,
            "monospace" => Family::Monospace,
            _ => Family::Name(config.font.as_str()),
        };
        let weight = match config.font_weight {
            FontWeightSpec::Name(FontWeightNameSpec::Bold) => Weight::BOLD,
            FontWeightSpec::Name(FontWeightNameSpec::Normal) => Weight::NORMAL,
            FontWeightSpec::Number(w) => Weight(*w as u16),
        };

        buffer.set_text(
            &mut font_system,
            config.text,
            Attrs::new().family(family).weight(weight),
            Shaping::Advanced,
        );

        buffer.set_size(&mut font_system, dimensions.size[0], dimensions.size[1]);
        buffer.shape_until_scroll(&mut font_system, false);

        let (buffer_width, line_y, buffer_height) = measure(&buffer);

        let text_color = [
            (config.color[0] * 255.0).round() as u8,
            (config.color[1] * 255.0).round() as u8,
            (config.color[2] * 255.0).round() as u8,
            (config.color[3] * 255.0).round() as u8,
        ];

        // Initialize
        let mut images: Vec<image::RgbaImage> = Vec::new();
        let mut glyphs: Vec<(TextRasterizationGlyphDetails, PhysicalGlyphPosition)> = Vec::new();

        for run in buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                let physical_glyph = glyph.physical(
                    (0.0, 0.0),
                    dimensions.scale,
                );

                let phys_pos = PhysicalGlyphPosition {
                    x: physical_glyph.x as f32,
                    y: physical_glyph.y as f32,
                };

                // Compute cache key which combines glyph and color
                let cache_key = (physical_glyph.cache_key, text_color);

                // Compute next image index
                let next_image_index = images.len();

                if let Some(details) = next_cache.get(&cache_key) {
                    // Glyph has already been rasterize, add again with new physical position
                    glyphs.push((details.clone(), phys_pos));
                } else {
                    // We need to rasterize glyph and write it to next_atlas
                    let Some(image) = cache
                        .get_image(&mut font_system, physical_glyph.cache_key)
                        .as_ref()
                        else {
                            return Err(AvengerWgpuError::ImageAllocationError(
                                "Failed to create glyph image".to_string(),
                            ));
                        };

                    let width = image.placement.width as usize;
                    let height = image.placement.height as usize;
                    let should_rasterize = width > 0 && height > 0;

                    if !should_rasterize {
                        continue;
                    }

                    let img = match image.content {
                        SwashContent::Color => {
                            // Image is rgba (like an emoji)
                            let Some(img) = image::RgbaImage::from_vec(
                                width as u32,
                                height as u32,
                                image.data.clone(),
                            ) else {
                                return Err(AvengerWgpuError::ImageAllocationError(
                                    "Failed to parse text rasterization as Rgba image".to_string(),
                                ));
                            };
                            img
                        }
                        SwashContent::Mask | SwashContent::SubpixelMask => {
                            // Image is monochrome (like regular text)
                            let Some(monochrome_img) = image::GrayImage::from_vec(
                                width as u32,
                                height as u32,
                                image.data.clone(),
                            ) else {
                                return Err(AvengerWgpuError::ImageAllocationError(
                                    "Failed to parse text rasterization as Grayscale image"
                                        .to_string(),
                                ));
                            };

                            // Initialize empty rgba image
                            let mut img = image::RgbaImage::new(monochrome_img.width(), monochrome_img.height());

                            // Write colored image
                            for x in 0..monochrome_img.width() {
                                for y in 0..monochrome_img.height() {
                                    let pixel_lum = monochrome_img.get_pixel(x as u32, y as u32).0[0];
                                    // Compute pixel color, adjusting alpha by pixel luminance
                                    let mut pixel_color = text_color;
                                    pixel_color[3] =
                                        ((text_color[3] as f32) * (pixel_lum as f32 / 255.0))
                                            .round() as u8;

                                    // Write pixel to rgba image
                                    let pixel = image::Rgba::from(pixel_color);
                                    img.put_pixel(x, y, pixel);
                                }
                            }
                            img
                        }
                    };

                    // Add new image
                    images.push(img);

                    // Create and add details and physical position
                    let details = TextRasterizationGlyphDetails {
                        image_index: next_image_index,
                        cache_key: (physical_glyph.cache_key, text_color),
                        top: image.placement.top,
                        left: image.placement.left,
                        width: image.placement.width,
                        height: image.placement.height,
                    };
                    glyphs.push((details.clone(), phys_pos));

                    // Update cache
                    next_cache.insert(cache_key, details);
                };
            }
        }

        Ok(Self {
            images,
            glyphs,
            buffer_width: buffer_width,
            buffer_height: buffer_height,
            buffer_line_y: line_y,
        })
    }
}