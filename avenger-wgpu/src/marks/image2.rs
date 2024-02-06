use crate::error::AvengerWgpuError;
use etagere::Size;
use image::DynamicImage;
use wgpu::Extent3d;

pub struct ImageAtlasBuilder {
    extent: Extent3d,
    next_image: image::RgbaImage,
    images: Vec<DynamicImage>,
    initialized: bool,
    allocator: etagere::AtlasAllocator,
}

#[derive(Copy, Clone)]
pub struct ImageAtlasCoords {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl ImageAtlasBuilder {
    pub fn new() -> Self {
        Self {
            extent: Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            next_image: image::RgbaImage::new(1, 1),
            images: vec![],
            initialized: false,
            allocator: etagere::AtlasAllocator::new(etagere::Size::new(1, 1)),
        }
    }

    pub fn register_image(
        &mut self,
        img: &image::RgbaImage,
    ) -> Result<(usize, ImageAtlasCoords), AvengerWgpuError> {
        if !self.initialized {
            let limits = wgpu::Limits::downlevel_webgl2_defaults();

            // Update extent
            self.extent = Extent3d {
                width: limits.max_texture_dimension_1d,
                height: limits.max_texture_dimension_2d,
                depth_or_array_layers: 1,
            };

            // Create backing image
            self.next_image = image::RgbaImage::new(self.extent.width, self.extent.height);

            // Create allocator
            self.allocator = etagere::AtlasAllocator::new(etagere::Size::new(
                self.extent.width as i32,
                self.extent.height as i32,
            ));

            // Set initialized
            self.initialized = true;
        }

        // Attempt to allocate into the current image
        let allocation = match self
            .allocator
            .allocate(Size::new(img.width() as i32, img.height() as i32))
        {
            Some(allocation) => allocation,
            None => {
                // Allocation failed, create new image
                let full_image = std::mem::take(&mut self.next_image);
                self.next_image = image::RgbaImage::new(self.extent.width, self.extent.height);
                self.images
                    .push(image::DynamicImage::ImageRgba8(full_image));
                self.allocator = etagere::AtlasAllocator::new(etagere::Size::new(
                    self.extent.width as i32,
                    self.extent.height as i32,
                ));

                // Try allocating again with new allocator
                match self
                    .allocator
                    .allocate(Size::new(img.width() as i32, img.height() as i32))
                {
                    Some(allocation) => allocation,
                    None => {
                        if img.width() > self.extent.width || img.height() > self.extent.height {
                            return Err(AvengerWgpuError::ImageAllocationError(format!(
                                "Image dimensions ({}, {}) exceed the maximum size of ({}, {})",
                                img.width(),
                                img.height(),
                                self.extent.width,
                                self.extent.height
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

        // Write image to allocated portion of final texture image
        let p0 = allocation.rectangle.min;
        let p1 = allocation.rectangle.max;

        let x0 = p0.x;
        let x1 = p1.x.min(x0 + img.width() as i32);
        let y0 = p0.y;
        let y1 = p1.y.min(y0 + img.height() as i32);

        for (src_x, dest_x) in (x0..x1).enumerate() {
            for (src_y, dest_y) in (y0..y1).enumerate() {
                self.next_image.put_pixel(
                    dest_x as u32,
                    dest_y as u32,
                    *img.get_pixel(src_x as u32, src_y as u32),
                );
            }
        }

        // Compute texture coordinates
        let coords = ImageAtlasCoords {
            x0: x0 as f32 / self.extent.width as f32,
            x1: x1 as f32 / self.extent.width as f32,
            y0: y0 as f32 / self.extent.height as f32,
            y1: y1 as f32 / self.extent.height as f32,
        };

        // Compute image atlas index
        let atlas_index = self.images.len();

        Ok((atlas_index, coords))
    }

    pub fn build(&self) -> (Extent3d, Vec<DynamicImage>) {
        let mut images = self.images.clone();
        images.push(image::DynamicImage::ImageRgba8(self.next_image.clone()));
        (self.extent, images)
    }
}
