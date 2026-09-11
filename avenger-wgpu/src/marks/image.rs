use avenger_image::{ImageResourceState, RgbaImage as AvengerRgbaImage};
use avenger_resource::ResourceKey;
use avenger_scenegraph::marks::image::{
    SceneImageResource, SceneImageSource, SceneImageUnavailablePolicy,
};
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

const IMAGE_ATLAS_GUTTER_PX: u32 = 1;

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
    unavailable_policy: SceneImageUnavailablePolicy,
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
        unavailable_policy: SceneImageUnavailablePolicy,
    ) -> Result<(usize, ImageAtlasCoords), AvengerWgpuError> {
        let [width, height] = source.intrinsic_size();
        if width == 0 || height == 0 {
            return Err(AvengerWgpuError::ImageAllocationError(format!(
                "Image dimensions ({width}, {height}) must be greater than zero"
            )));
        }

        self.initialize_if_needed();
        let allocated_width = width
            .checked_add(2 * IMAGE_ATLAS_GUTTER_PX)
            .ok_or_else(|| {
                AvengerWgpuError::ImageAllocationError(format!(
                    "Image dimensions ({width}, {height}) exceed atlas allocation limits"
                ))
            })?;
        let allocated_height = height
            .checked_add(2 * IMAGE_ATLAS_GUTTER_PX)
            .ok_or_else(|| {
                AvengerWgpuError::ImageAllocationError(format!(
                    "Image dimensions ({width}, {height}) exceed atlas allocation limits"
                ))
            })?;

        let allocation = match self
            .allocator
            .allocate(Size::new(allocated_width as i32, allocated_height as i32))
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
                    .allocate(Size::new(allocated_width as i32, allocated_height as i32))
                {
                    Some(allocation) => allocation,
                    None => {
                        if allocated_width > self.extent.width
                            || allocated_height > self.extent.height
                        {
                            return Err(AvengerWgpuError::ImageAllocationError(format!(
                                "Image dimensions ({width}, {height}) with {IMAGE_ATLAS_GUTTER_PX}px gutters exceed the maximum atlas allocation size of ({}, {})",
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
        let content_x = x + IMAGE_ATLAS_GUTTER_PX;
        let content_y = y + IMAGE_ATLAS_GUTTER_PX;
        let atlas_index = self.current_atlas_index;
        let coords = ImageAtlasCoords {
            x0: content_x as f32 / self.extent.width as f32,
            x1: (content_x + width) as f32 / self.extent.width as f32,
            y0: content_y as f32 / self.extent.height as f32,
            y1: (content_y + height) as f32 / self.extent.height as f32,
        };

        self.entries.push(ImageAtlasEntry {
            atlas_index,
            x,
            y,
            width,
            height,
            source,
            unavailable_policy,
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
            SceneImageSource::SharedInline(image) => image.to_image().map(Some).ok_or_else(|| {
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
                    Ok(Some(image::imageops::resize(
                        &image,
                        entry.width,
                        entry.height,
                        image::imageops::FilterType::CatmullRom,
                    )))
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
    let missing_policy = match entry.unavailable_policy {
        SceneImageUnavailablePolicy::RendererDefault => config.missing_policy,
        SceneImageUnavailablePolicy::Skip => WgpuMissingImagePolicy::Skip,
        SceneImageUnavailablePolicy::DrawPlaceholder => WgpuMissingImagePolicy::DrawPlaceholder,
        SceneImageUnavailablePolicy::Error => WgpuMissingImagePolicy::Error,
    };
    match missing_policy {
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

pub(crate) fn make_placeholder(
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
    if copy_width == 0 || copy_height == 0 {
        return;
    }

    let content_x = entry.x + IMAGE_ATLAS_GUTTER_PX;
    let content_y = entry.y + IMAGE_ATLAS_GUTTER_PX;
    for src_y in 0..copy_height {
        for src_x in 0..copy_width {
            atlas.put_pixel(
                content_x + src_x,
                content_y + src_y,
                *image.get_pixel(src_x, src_y),
            );
        }
    }

    let left_x = content_x - 1;
    let right_x = content_x + copy_width;
    let top_y = content_y - 1;
    let bottom_y = content_y + copy_height;

    for src_y in 0..copy_height {
        let dst_y = content_y + src_y;
        atlas.put_pixel(left_x, dst_y, *image.get_pixel(0, src_y));
        atlas.put_pixel(right_x, dst_y, *image.get_pixel(copy_width - 1, src_y));
    }

    for src_x in 0..copy_width {
        let dst_x = content_x + src_x;
        atlas.put_pixel(dst_x, top_y, *image.get_pixel(src_x, 0));
        atlas.put_pixel(dst_x, bottom_y, *image.get_pixel(src_x, copy_height - 1));
    }

    atlas.put_pixel(left_x, top_y, *image.get_pixel(0, 0));
    atlas.put_pixel(right_x, top_y, *image.get_pixel(copy_width - 1, 0));
    atlas.put_pixel(left_x, bottom_y, *image.get_pixel(0, copy_height - 1));
    atlas.put_pixel(
        right_x,
        bottom_y,
        *image.get_pixel(copy_width - 1, copy_height - 1),
    );
}

pub(crate) fn push_unique(values: &mut Vec<ResourceKey>, key: ResourceKey) {
    if !values.contains(&key) {
        values.push(key);
    }
}

pub(crate) fn push_unique_failed(
    status: &mut WgpuImageResourceStatus,
    key: ResourceKey,
    message: String,
) {
    if !status.failed.iter().any(|(existing, _)| existing == &key) {
        status.failed.push((key, message));
    }
}

#[cfg(test)]
mod tests {
    use image::Rgba;

    use super::*;

    #[test]
    fn atlas_coords_are_inset_to_content_inside_gutter() {
        let image = image::RgbaImage::from_pixel(2, 3, Rgba([10, 20, 30, 255]));
        let mut builder = ImageAtlasBuilder::new();
        let (_atlas_index, coords) = builder
            .register_source(
                inline_source(&image),
                SceneImageUnavailablePolicy::RendererDefault,
            )
            .expect("register image source");

        let entry = &builder.entries[0];
        let expected_x0 = (entry.x + IMAGE_ATLAS_GUTTER_PX) as f32 / builder.extent.width as f32;
        let expected_x1 =
            (entry.x + IMAGE_ATLAS_GUTTER_PX + entry.width) as f32 / builder.extent.width as f32;
        let expected_y0 = (entry.y + IMAGE_ATLAS_GUTTER_PX) as f32 / builder.extent.height as f32;
        let expected_y1 =
            (entry.y + IMAGE_ATLAS_GUTTER_PX + entry.height) as f32 / builder.extent.height as f32;

        assert_eq!(coords.x0, expected_x0);
        assert_eq!(coords.x1, expected_x1);
        assert_eq!(coords.y0, expected_y0);
        assert_eq!(coords.y1, expected_y1);
    }

    #[test]
    fn atlas_copy_duplicates_edge_pixels_into_gutter() {
        let image = test_image_2x2();
        let mut builder = ImageAtlasBuilder::new();
        builder
            .register_source(
                inline_source(&image),
                SceneImageUnavailablePolicy::RendererDefault,
            )
            .expect("register image source");
        let entry = builder.entries[0].clone();
        let (_extent, atlases, _status) = builder
            .build(&WgpuImageResourceConfig::default())
            .expect("build image atlas");
        let atlas = atlases[entry.atlas_index].to_rgba8();
        let x = entry.x + IMAGE_ATLAS_GUTTER_PX;
        let y = entry.y + IMAGE_ATLAS_GUTTER_PX;

        assert_pixel(&atlas, x, y, RED);
        assert_pixel(&atlas, x + 1, y, GREEN);
        assert_pixel(&atlas, x, y + 1, BLUE);
        assert_pixel(&atlas, x + 1, y + 1, YELLOW);

        assert_pixel(&atlas, x - 1, y - 1, RED);
        assert_pixel(&atlas, x, y - 1, RED);
        assert_pixel(&atlas, x + 1, y - 1, GREEN);
        assert_pixel(&atlas, x + 2, y - 1, GREEN);
        assert_pixel(&atlas, x - 1, y, RED);
        assert_pixel(&atlas, x + 2, y, GREEN);
        assert_pixel(&atlas, x - 1, y + 1, BLUE);
        assert_pixel(&atlas, x + 2, y + 1, YELLOW);
        assert_pixel(&atlas, x - 1, y + 2, BLUE);
        assert_pixel(&atlas, x, y + 2, BLUE);
        assert_pixel(&atlas, x + 1, y + 2, YELLOW);
        assert_pixel(&atlas, x + 2, y + 2, YELLOW);
    }

    #[test]
    fn atlas_copy_duplicates_one_pixel_image_into_all_gutters() {
        let image = image::RgbaImage::from_pixel(1, 1, Rgba([77, 88, 99, 255]));
        let mut builder = ImageAtlasBuilder::new();
        builder
            .register_source(
                inline_source(&image),
                SceneImageUnavailablePolicy::RendererDefault,
            )
            .expect("register image source");
        let entry = builder.entries[0].clone();
        let (_extent, atlases, _status) = builder
            .build(&WgpuImageResourceConfig::default())
            .expect("build image atlas");
        let atlas = atlases[entry.atlas_index].to_rgba8();

        for y in entry.y..entry.y + 3 {
            for x in entry.x..entry.x + 3 {
                assert_pixel(&atlas, x, y, [77, 88, 99, 255]);
            }
        }
    }

    const RED: [u8; 4] = [255, 0, 0, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const YELLOW: [u8; 4] = [255, 255, 0, 255];

    fn inline_source(image: &image::RgbaImage) -> SceneImageSource {
        SceneImageSource::Inline(AvengerRgbaImage::from_image(image))
    }

    fn test_image_2x2() -> image::RgbaImage {
        image::RgbaImage::from_fn(2, 2, |x, y| match (x, y) {
            (0, 0) => Rgba(RED),
            (1, 0) => Rgba(GREEN),
            (0, 1) => Rgba(BLUE),
            (1, 1) => Rgba(YELLOW),
            _ => unreachable!(),
        })
    }

    fn assert_pixel(image: &image::RgbaImage, x: u32, y: u32, expected: [u8; 4]) {
        assert_eq!(
            image.get_pixel(x, y).0,
            expected,
            "unexpected pixel at ({x}, {y})"
        );
    }
}
