use crate::error::AvengerWgpuError;
use etagere::Size;
use image::DynamicImage;
use wgpu::Extent3d;

const IMAGE_ATLAS_GUTTER_PX: u32 = 1;

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
            next_image: image::RgbaImage::new(1, 1),
            images: vec![],
            initialized: false,
            allocator: etagere::AtlasAllocator::new(etagere::Size::new(1, 1)),
        }
    }

    /// Copies an image into the atlas, inside a one-pixel gutter that repeats its edge pixels, so
    /// that smooth sampling at the image's edges doesn't blend in its neighbors.
    pub fn register_image(
        &mut self,
        img: &image::RgbaImage,
    ) -> Result<(usize, ImageAtlasCoords), AvengerWgpuError> {
        let (width, height) = img.dimensions();
        if width == 0 || height == 0 {
            return Err(AvengerWgpuError::ImageAllocationError(format!(
                "Image dimensions ({width}, {height}) must be greater than zero"
            )));
        }
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

        let too_large = || {
            AvengerWgpuError::ImageAllocationError(format!(
                "Image dimensions ({width}, {height}) with {IMAGE_ATLAS_GUTTER_PX}px gutters exceed the maximum size of ({}, {})",
                self.extent.width, self.extent.height
            ))
        };
        let allocated = Size::new(
            width
                .checked_add(2 * IMAGE_ATLAS_GUTTER_PX)
                .and_then(|width| i32::try_from(width).ok())
                .ok_or_else(too_large)?,
            height
                .checked_add(2 * IMAGE_ATLAS_GUTTER_PX)
                .and_then(|height| i32::try_from(height).ok())
                .ok_or_else(too_large)?,
        );

        // Attempt to allocate into the current image
        let allocation = match self.allocator.allocate(allocated) {
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
                self.allocator.allocate(allocated).ok_or_else(|| {
                    if allocated.width as u32 > self.extent.width
                        || allocated.height as u32 > self.extent.height
                    {
                        too_large()
                    } else {
                        AvengerWgpuError::ImageAllocationError("Unknown error".to_string())
                    }
                })?
            }
        };

        let x = allocation.rectangle.min.x as u32 + IMAGE_ATLAS_GUTTER_PX;
        let y = allocation.rectangle.min.y as u32 + IMAGE_ATLAS_GUTTER_PX;
        copy_with_gutter(&mut self.next_image, x, y, img);

        // Compute texture coordinates
        let coords = ImageAtlasCoords {
            x0: x as f32 / self.extent.width as f32,
            x1: (x + width) as f32 / self.extent.width as f32,
            y0: y as f32 / self.extent.height as f32,
            y1: (y + height) as f32 / self.extent.height as f32,
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

/// Copies `image` into `atlas` with its top-left corner at `(x, y)`, and its edge pixels into the
/// one-pixel gutter around it. The atlas holds premultiplied pixels, so that smooth sampling
/// filters in premultiplied space, and the shader converts each sample back to straight alpha.
fn copy_with_gutter(atlas: &mut image::RgbaImage, x: u32, y: u32, image: &image::RgbaImage) {
    let (width, height) = image.dimensions();
    for dy in -1..=height as i64 {
        for dx in -1..=width as i64 {
            let source_x = dx.clamp(0, width as i64 - 1) as u32;
            let source_y = dy.clamp(0, height as i64 - 1) as u32;
            atlas.put_pixel(
                (x as i64 + dx) as u32,
                (y as i64 + dy) as u32,
                premultiplied(*image.get_pixel(source_x, source_y)),
            );
        }
    }
}

fn premultiplied(image::Rgba([r, g, b, a]): image::Rgba<u8>) -> image::Rgba<u8> {
    let scale = |channel: u8| ((u16::from(channel) * u16::from(a) + 127) / 255) as u8;
    image::Rgba([scale(r), scale(g), scale(b), a])
}

#[cfg(test)]
mod tests {
    use image::Rgba;

    use super::*;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const YELLOW: [u8; 4] = [255, 255, 0, 255];

    /// An image's atlas page and the atlas pixel of its top-left corner.
    fn register(image: &image::RgbaImage) -> (image::RgbaImage, u32, u32, ImageAtlasCoords) {
        let mut builder = ImageAtlasBuilder::new();
        let (index, coords) = builder.register_image(image).unwrap();
        let (extent, atlases) = builder.build();
        let x = (coords.x0 * extent.width as f32).round() as u32;
        let y = (coords.y0 * extent.height as f32).round() as u32;
        (atlases[index].to_rgba8(), x, y, coords)
    }

    #[test]
    fn atlas_coords_cover_the_image_inside_its_gutter() {
        let image = image::RgbaImage::from_pixel(2, 3, Rgba([10, 20, 30, 255]));
        let mut builder = ImageAtlasBuilder::new();
        let (_, coords) = builder.register_image(&image).unwrap();
        let (extent, _) = builder.build();
        let [width, height] = [extent.width as f32, extent.height as f32];
        assert!(coords.x0 * width >= 1.0 && coords.y0 * height >= 1.0);
        assert_eq!(((coords.x1 - coords.x0) * width).round(), 2.0);
        assert_eq!(((coords.y1 - coords.y0) * height).round(), 3.0);
    }

    #[test]
    fn atlas_gutters_repeat_edge_pixels() {
        let image = image::RgbaImage::from_fn(2, 2, |x, y| {
            Rgba([[RED, GREEN], [BLUE, YELLOW]][y as usize][x as usize])
        });
        let (atlas, x, y, _) = register(&image);
        // Each row of the 4 x 4 block around the image, gutters included.
        let rows = [
            [RED, RED, GREEN, GREEN],
            [RED, RED, GREEN, GREEN],
            [BLUE, BLUE, YELLOW, YELLOW],
            [BLUE, BLUE, YELLOW, YELLOW],
        ];
        for (dy, row) in rows.iter().enumerate() {
            for (dx, expected) in row.iter().enumerate() {
                let (px, py) = (x + dx as u32 - 1, y + dy as u32 - 1);
                assert_eq!(&atlas.get_pixel(px, py).0, expected, "pixel ({px}, {py})");
            }
        }
    }

    #[test]
    fn atlas_gutters_repeat_a_one_pixel_image() {
        let image = image::RgbaImage::from_pixel(1, 1, Rgba([77, 88, 99, 255]));
        let (atlas, x, y, _) = register(&image);
        for py in y - 1..=y + 1 {
            for px in x - 1..=x + 1 {
                assert_eq!(atlas.get_pixel(px, py).0, [77, 88, 99, 255]);
            }
        }
    }

    #[test]
    fn empty_images_are_errors() {
        let mut builder = ImageAtlasBuilder::new();
        assert!(builder
            .register_image(&image::RgbaImage::new(0, 3))
            .is_err());
    }
}
