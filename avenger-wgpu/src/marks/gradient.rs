use avenger_color::{ColorOrGradient, Gradient};
use colorgrad::{Color, Gradient as ColorGradGradient, GradientBuilder};
use image::{DynamicImage, Rgba};
use wgpu::Extent3d;

use crate::marks::multi::GRADIENT_TEXTURE_CODE;

const GRADIENT_WIDTH: u32 = 256;
const GRADIENT_HEIGH: u32 = 32;
pub const COLORWAY_LENGTH: u32 = 250;

pub struct GradientAtlasBuilder {
    extent: Extent3d,
    next_image: image::RgbaImage,
    images: Vec<DynamicImage>,
    next_grad_row: usize,
    initialized: bool,
}

impl Default for GradientAtlasBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl GradientAtlasBuilder {
    pub fn new() -> Self {
        // Initialize with single pixel image
        Self {
            extent: Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            next_image: image::RgbaImage::new(1, 1),
            images: vec![],
            next_grad_row: 0,
            initialized: false,
        }
    }

    pub fn register_gradients(&mut self, gradients: &[Gradient]) -> (Option<usize>, Vec<f32>) {
        if gradients.is_empty() {
            return (None, Vec::new());
        }

        // Handle initialization
        if !self.initialized {
            // Initialze next_image that we can write to
            self.next_image = image::RgbaImage::new(GRADIENT_WIDTH, GRADIENT_HEIGH);
            self.extent = Extent3d {
                width: GRADIENT_WIDTH,
                height: GRADIENT_HEIGH,
                depth_or_array_layers: 1,
            };
            self.initialized = true;
        }

        // Handle creation of new image when current image is full
        if self.next_grad_row + gradients.len() > GRADIENT_HEIGH as usize {
            let full_image = std::mem::take(&mut self.next_image);
            self.next_image = image::RgbaImage::new(GRADIENT_WIDTH, GRADIENT_HEIGH);
            self.images
                .push(image::DynamicImage::ImageRgba8(full_image));
            self.next_grad_row = 0;
        }

        // Write gradient values
        for (pos, grad) in gradients.iter().enumerate() {
            let row = (pos + self.next_grad_row) as u32;

            let stops = grad.stops();
            let colors: Vec<_> = stops
                .iter()
                .map(|stop| Color::new(stop.color[0], stop.color[1], stop.color[2], stop.color[3]))
                .collect();
            let ramp = if stops.len() > 1 {
                Some(
                    GradientBuilder::new()
                        .domain(&stops.iter().map(|stop| stop.offset).collect::<Vec<_>>())
                        .colors(&colors)
                        .build::<colorgrad::LinearGradient>()
                        .unwrap(),
                )
            } else {
                None
            };
            let constant = colors.first().map(Color::to_rgba8).unwrap_or([0; 4]);

            // Store 250-bin colorway in pixels 6 through 255.
            let col_offset = GRADIENT_WIDTH - COLORWAY_LENGTH;
            for i in 0..COLORWAY_LENGTH {
                let color = ramp.as_ref().map_or(constant, |ramp| {
                    ramp.at(i as f32 / COLORWAY_LENGTH as f32).to_rgba8()
                });
                self.next_image.put_pixel(i + col_offset, row, Rgba(color));
            }

            // Six metadata texels hold f32 bit patterns independently of the color ramp.
            // A negative starting radius identifies a linear gradient.
            let controls = match grad {
                Gradient::LinearGradient(g) => [g.x0, g.y0, g.x1, g.y1, -1.0, 0.0],
                Gradient::RadialGradient(g) => [g.x0, g.y0, g.x1, g.y1, g.r0, g.r1],
            };
            for (column, value) in controls.into_iter().enumerate() {
                self.next_image
                    .put_pixel(column as u32, row, Rgba(value.to_le_bytes()));
            }
        }

        // Compute texture coords of gradient rows.
        // Add 0.1 of pixel offset so that we don't land on the edge
        let coords = (self.next_grad_row..(self.next_grad_row + gradients.len()))
            .map(|i| (i as f32 + 0.1) / GRADIENT_HEIGH as f32)
            .collect::<Vec<_>>();

        // Update next gradient row (Could be greater than GRADIENT_HEIGH, this will be
        // handled on the next call to register_gradients.
        self.next_grad_row += gradients.len();

        // Compute gradient atlas index (index into the vector if images that will be
        // returned by build).
        let atlas_index = self.images.len();

        (Some(atlas_index), coords)
    }

    pub fn build(&self) -> (Extent3d, Vec<DynamicImage>) {
        let mut images = self.images.clone();
        images.push(image::DynamicImage::ImageRgba8(self.next_image.clone()));
        (self.extent, images)
    }
}

pub fn to_color_or_gradient_coord(
    color_or_gradient: &ColorOrGradient,
    grad_coords: &[f32],
) -> [f32; 4] {
    match color_or_gradient {
        ColorOrGradient::Color(c) => *c,
        ColorOrGradient::GradientIndex(grad_idx) => [
            GRADIENT_TEXTURE_CODE,
            grad_coords[*grad_idx as usize],
            0.0,
            0.0,
        ],
    }
}
