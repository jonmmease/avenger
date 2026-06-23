use avenger_image::{ImageResourceState, RgbaImage as AvengerRgbaImage};
use avenger_resource::ResourceKey;
use avenger_scenegraph::marks::image::{SceneImageResource, SceneImageSource};
use etagere::Size;
use image::{DynamicImage, Rgba};
use wgpu::Extent3d;

use crate::{
    error::AvengerWgpuError,
    image_resources::{
        WgpuImagePlaceholder, WgpuImageResourceConfig, WgpuImageResourceStatus,
        WgpuMissingImagePolicy,
    },
};

pub struct ImageAtlasBuilder {
    extent: Extent3d,
    entries: Vec<ImageAtlasEntry>,
    initialized: bool,
    allocator: etagere::AtlasAllocator,
    current_atlas_index: usize,
}

#[derive(Copy, Clone)]
pub struct ImageAtlasCoords {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

#[derive(Clone)]
struct ImageAtlasEntry {
    atlas_index: usize,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    source: SceneImageSource,
}

impl Default for ImageAtlasBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ImageAtlasBuilder {
    pub fn new() -> Self {
        Self {
            extent: Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            entries: vec![],
            initialized: false,
            allocator: etagere::AtlasAllocator::new(etagere::Size::new(1, 1)),
            current_atlas_index: 0,
        }
    }

    pub fn register_source(
        &mut self,
        source: SceneImageSource,
    ) -> Result<(usize, ImageAtlasCoords), AvengerWgpuError> {
        let [width, height] = source.intrinsic_size();
        if width == 0 || height == 0 {
            return Err(AvengerWgpuError::ImageAllocationError(format!(
                "Image dimensions ({width}, {height}) must be greater than zero"
            )));
        }

        self.initialize_if_needed();

        let allocation = match self
            .allocator
            .allocate(Size::new(width as i32, height as i32))
        {
            Some(allocation) => allocation,
            None => {
                self.current_atlas_index += 1;
                self.allocator = etagere::AtlasAllocator::new(etagere::Size::new(
                    self.extent.width as i32,
                    self.extent.height as i32,
                ));

                match self
                    .allocator
                    .allocate(Size::new(width as i32, height as i32))
                {
                    Some(allocation) => allocation,
                    None => {
                        if width > self.extent.width || height > self.extent.height {
                            return Err(AvengerWgpuError::ImageAllocationError(format!(
                                "Image dimensions ({width}, {height}) exceed the maximum size of ({}, {})",
                                self.extent.width, self.extent.height
                            )));
                        } else {
                            return Err(AvengerWgpuError::ImageAllocationError(
                                "Unknown error".to_string(),
                            ));
                        }
                    }
                }
            }
        };

        let p0 = allocation.rectangle.min;
        let x = p0.x as u32;
        let y = p0.y as u32;
        let atlas_index = self.current_atlas_index;
        let coords = ImageAtlasCoords {
            x0: x as f32 / self.extent.width as f32,
            x1: (x + width) as f32 / self.extent.width as f32,
            y0: y as f32 / self.extent.height as f32,
            y1: (y + height) as f32 / self.extent.height as f32,
        };

        self.entries.push(ImageAtlasEntry {
            atlas_index,
            x,
            y,
            width,
            height,
            source,
        });

        Ok((atlas_index, coords))
    }

    pub fn build(
        &self,
        config: &WgpuImageResourceConfig,
    ) -> Result<(Extent3d, Vec<DynamicImage>, WgpuImageResourceStatus), AvengerWgpuError> {
        let generation = config
            .resolver
            .as_ref()
            .map(|resolver| resolver.generation())
            .unwrap_or_default();
        let mut status = WgpuImageResourceStatus::default().with_generation(generation);

        if !self.initialized {
            return Ok((
                self.extent,
                vec![DynamicImage::ImageRgba8(image::RgbaImage::new(1, 1))],
                status,
            ));
        }

        let mut images = vec![
            image::RgbaImage::new(self.extent.width, self.extent.height);
            self.current_atlas_index + 1
        ];

        for entry in &self.entries {
            if let Some(image) = self.resolve_entry_image(entry, config, &mut status)? {
                copy_image_to_atlas(&mut images[entry.atlas_index], entry, &image);
            }
        }

        Ok((
            self.extent,
            images.into_iter().map(DynamicImage::ImageRgba8).collect(),
            status,
        ))
    }

    fn initialize_if_needed(&mut self) {
        if self.initialized {
            return;
        }

        let limits = wgpu::Limits::downlevel_webgl2_defaults();
        self.extent = Extent3d {
            width: limits.max_texture_dimension_1d,
            height: limits.max_texture_dimension_2d,
            depth_or_array_layers: 1,
        };
        self.allocator = etagere::AtlasAllocator::new(etagere::Size::new(
            self.extent.width as i32,
            self.extent.height as i32,
        ));
        self.initialized = true;
    }

    fn resolve_entry_image(
        &self,
        entry: &ImageAtlasEntry,
        config: &WgpuImageResourceConfig,
        status: &mut WgpuImageResourceStatus,
    ) -> Result<Option<image::RgbaImage>, AvengerWgpuError> {
        match &entry.source {
            SceneImageSource::Inline(image) => image.to_image().map(Some).ok_or_else(|| {
                AvengerWgpuError::ConversionError(
                    "Failed to convert raw image to rgba image".to_string(),
                )
            }),
            SceneImageSource::Resource(resource) => {
                self.resolve_resource_image(entry, resource, config, status)
            }
        }
    }

    fn resolve_resource_image(
        &self,
        entry: &ImageAtlasEntry,
        resource: &SceneImageResource,
        config: &WgpuImageResourceConfig,
        status: &mut WgpuImageResourceStatus,
    ) -> Result<Option<image::RgbaImage>, AvengerWgpuError> {
        let Some(resolver) = config.resolver.as_ref() else {
            push_unique(&mut status.missing, resource.key.clone());
            return unavailable_image(entry, config, "No WGPU image resource resolver configured");
        };

        match resolver.image_state(&resource.key) {
            ImageResourceState::Ready(image) => {
                let image = image.to_image().ok_or_else(|| {
                    AvengerWgpuError::ConversionError(format!(
                        "Failed to convert ready resource image {:?} to rgba image",
                        resource.key
                    ))
                })?;
                if image.width() == entry.width && image.height() == entry.height {
                    Ok(Some(image))
                } else {
                    let message = format!(
                        "Ready resource image {:?} has dimensions ({}, {}), expected ({}, {})",
                        resource.key,
                        image.width(),
                        image.height(),
                        entry.width,
                        entry.height
                    );
                    push_unique_failed(status, resource.key.clone(), message.clone());
                    unavailable_image(entry, config, &message)
                }
            }
            ImageResourceState::Pending => {
                push_unique(&mut status.pending, resource.key.clone());
                self.resolve_fallback_or_unavailable(entry, resource, config, status, "pending")
            }
            ImageResourceState::Missing => {
                push_unique(&mut status.missing, resource.key.clone());
                self.resolve_fallback_or_unavailable(entry, resource, config, status, "missing")
            }
            ImageResourceState::Failed(error) => {
                push_unique_failed(status, resource.key.clone(), error.to_string());
                self.resolve_fallback_or_unavailable(
                    entry,
                    resource,
                    config,
                    status,
                    error.as_ref(),
                )
            }
        }
    }

    fn resolve_fallback_or_unavailable(
        &self,
        entry: &ImageAtlasEntry,
        resource: &SceneImageResource,
        config: &WgpuImageResourceConfig,
        status: &mut WgpuImageResourceStatus,
        reason: &str,
    ) -> Result<Option<image::RgbaImage>, AvengerWgpuError> {
        if let (Some(resolver), Some(fallback_key)) =
            (config.resolver.as_ref(), resource.fallback_key.as_ref())
        {
            if let ImageResourceState::Ready(image) = resolver.image_state(fallback_key) {
                let image = image.to_image().ok_or_else(|| {
                    AvengerWgpuError::ConversionError(format!(
                        "Failed to convert fallback resource image {fallback_key:?} to rgba image"
                    ))
                })?;
                if image.width() == entry.width && image.height() == entry.height {
                    return Ok(Some(image));
                }
                push_unique_failed(
                    status,
                    fallback_key.clone(),
                    format!(
                        "Fallback resource image {fallback_key:?} has dimensions ({}, {}), expected ({}, {})",
                        image.width(),
                        image.height(),
                        entry.width,
                        entry.height
                    ),
                );
            }
        }

        unavailable_image(entry, config, reason)
    }
}

fn unavailable_image(
    entry: &ImageAtlasEntry,
    config: &WgpuImageResourceConfig,
    reason: &str,
) -> Result<Option<image::RgbaImage>, AvengerWgpuError> {
    match config.missing_policy {
        WgpuMissingImagePolicy::DrawPlaceholder => Ok(Some(make_placeholder(
            entry.width,
            entry.height,
            &config.placeholder,
        )?)),
        WgpuMissingImagePolicy::Skip => Ok(None),
        WgpuMissingImagePolicy::Error => {
            Err(AvengerWgpuError::ImageResourceError(reason.to_string()))
        }
    }
}

fn make_placeholder(
    width: u32,
    height: u32,
    placeholder: &WgpuImagePlaceholder,
) -> Result<image::RgbaImage, AvengerWgpuError> {
    match placeholder {
        WgpuImagePlaceholder::Transparent => Ok(image::RgbaImage::new(width, height)),
        WgpuImagePlaceholder::Solid(color) => {
            Ok(image::RgbaImage::from_pixel(width, height, Rgba(*color)))
        }
        WgpuImagePlaceholder::Checkerboard => {
            let mut image = image::RgbaImage::new(width, height);
            for y in 0..height {
                for x in 0..width {
                    let light = ((x / 8) + (y / 8)) % 2 == 0;
                    let value = if light { 208 } else { 160 };
                    image.put_pixel(x, y, Rgba([value, value, value, 255]));
                }
            }
            Ok(image)
        }
        WgpuImagePlaceholder::Inline(image) => scale_placeholder(image, width, height),
    }
}

fn scale_placeholder(
    image: &AvengerRgbaImage,
    width: u32,
    height: u32,
) -> Result<image::RgbaImage, AvengerWgpuError> {
    let Some(source) = image.to_image() else {
        return Err(AvengerWgpuError::ConversionError(
            "Failed to convert placeholder image to rgba image".to_string(),
        ));
    };
    if source.width() == width && source.height() == height {
        return Ok(source);
    }

    let mut scaled = image::RgbaImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let src_x = (x as u64 * source.width() as u64 / width as u64) as u32;
            let src_y = (y as u64 * source.height() as u64 / height as u64) as u32;
            scaled.put_pixel(x, y, *source.get_pixel(src_x, src_y));
        }
    }
    Ok(scaled)
}

fn copy_image_to_atlas(
    atlas: &mut image::RgbaImage,
    entry: &ImageAtlasEntry,
    image: &image::RgbaImage,
) {
    let copy_width = entry.width.min(image.width());
    let copy_height = entry.height.min(image.height());
    for src_y in 0..copy_height {
        for src_x in 0..copy_width {
            atlas.put_pixel(
                entry.x + src_x,
                entry.y + src_y,
                *image.get_pixel(src_x, src_y),
            );
        }
    }
}

fn push_unique(values: &mut Vec<ResourceKey>, key: ResourceKey) {
    if !values.contains(&key) {
        values.push(key);
    }
}

fn push_unique_failed(status: &mut WgpuImageResourceStatus, key: ResourceKey, message: String) {
    if !status.failed.iter().any(|(existing, _)| existing == &key) {
        status.failed.push((key, message));
    }
}
