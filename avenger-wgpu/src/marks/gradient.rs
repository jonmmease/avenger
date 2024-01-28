use crate::marks::rect::{
    COLORWAY_LENGTH, GRADIENT_LINEAR, GRADIENT_RADIAL, GRADIENT_TEXTURE_WIDTH,
};
use avenger::marks::value::{ColorOrGradient, Gradient};
use colorgrad::Color;
use image::{DynamicImage, Rgba};
use wgpu::Extent3d;

pub fn to_color_or_gradient_coord(
    color_or_gradient: &ColorOrGradient,
    texture_size: Extent3d,
) -> [f32; 4] {
    match color_or_gradient {
        ColorOrGradient::Color(c) => *c,
        ColorOrGradient::GradientIndex(grad_idx) => {
            // Each gradient colorway is written to two rows, starting at texture
            // y-coordinate 0. This results in 128 gradients stored in a 256x256 texture. To
            // avoid interpolation artifacts, we compute the texture coordinate as the
            // position between the two rows
            let num_gradient_rows = texture_size.height as f32 / 2.0;
            let grad_coord =
                (*grad_idx as f32 / num_gradient_rows) + 1.0 / (texture_size.height as f32 * 2.0);
            [-grad_coord, 0.0, 0.0, 0.0]
        }
    }
}

pub fn build_gradients_image(gradients: &[Gradient]) -> (Option<DynamicImage>, Extent3d) {
    if gradients.is_empty() {
        return (None, Extent3d::default());
    }

    // Write gradients
    let limits = wgpu::Limits::downlevel_webgl2_defaults();
    assert!(
        gradients.len() < (limits.max_texture_dimension_2d / 2) as usize,
        "Exceeded max number of unique gradients"
    );

    let texture_size = Extent3d {
        width: GRADIENT_TEXTURE_WIDTH,
        height: (gradients.len() * 2) as u32,
        depth_or_array_layers: 1,
    };

    let mut img = image::RgbaImage::new(texture_size.width, texture_size.height);
    for (pos, grad) in gradients.iter().enumerate() {
        let row0 = (pos * 2) as u32;

        // Build gradient colorway using colorgrad
        let s = grad.stops();
        let mut binding = colorgrad::CustomGradient::new();
        let offsets = s.iter().map(|stop| stop.offset as f64).collect::<Vec<_>>();
        let colors = s
            .iter()
            .map(|stop| {
                Color::new(
                    stop.color[0] as f64,
                    stop.color[1] as f64,
                    stop.color[2] as f64,
                    stop.color[3] as f64,
                )
            })
            .collect::<Vec<_>>();

        let builder = binding.domain(offsets.as_slice()).colors(colors.as_slice());
        let b = builder.build().unwrap();

        // Fill leading pixels with start color so that linear interpolation doesn't pick
        // up the empty pixels between control pixels and gradient pixels
        let start_color = Rgba::from(b.at(0.0).to_rgba8());
        let col_offset = GRADIENT_TEXTURE_WIDTH - COLORWAY_LENGTH;
        for i in 0..col_offset {
            img.put_pixel(i, row0, start_color);
            img.put_pixel(i, row0 + 1, start_color);
        }

        // Store 250-bin colorway in pixels 6 through 255
        for i in 0..COLORWAY_LENGTH {
            let p = (i as f64) / (COLORWAY_LENGTH as f64 - 1.0);
            let c = b.at(p).to_rgba8();

            // Write color to row0 and row0 + 1
            img.put_pixel(i + col_offset, row0, Rgba::from(c));
            img.put_pixel(i + col_offset, row0 + 1, Rgba::from(c));
        }

        // We encode the gradient control points in the first two or three pixels of the texture
        match grad {
            Gradient::LinearGradient(grad) => {
                // Write gradient type to column 0
                let control_color0 = Rgba::from([(GRADIENT_LINEAR * 255.0) as u8, 0, 0, 0]);
                img.put_pixel(0, row0, control_color0);
                img.put_pixel(0, row0 + 1, control_color0);

                // Write x/y control points to column 1
                let control_color1 = Rgba::from([
                    (grad.x0 * 255.0) as u8,
                    (grad.y0 * 255.0) as u8,
                    (grad.x1 * 255.0) as u8,
                    (grad.y1 * 255.0) as u8,
                ]);
                img.put_pixel(1, row0, control_color1);
                img.put_pixel(1, row0 + 1, control_color1);
            }
            Gradient::RadialGradient(grad) => {
                // Write gradient type to column 0
                let control_color0 = Rgba::from([(GRADIENT_RADIAL * 255.0) as u8, 0, 0, 0]);
                img.put_pixel(0, row0, control_color0);
                img.put_pixel(0, row0 + 1, control_color0);

                // Write x/y control points to column 1
                let control_color1 = Rgba::from([
                    (grad.x0 * 255.0) as u8,
                    (grad.y0 * 255.0) as u8,
                    (grad.x1 * 255.0) as u8,
                    (grad.y1 * 255.0) as u8,
                ]);
                img.put_pixel(1, row0, control_color1);
                img.put_pixel(1, row0 + 1, control_color1);

                // Write radius control points to column 2
                let control_color2 =
                    Rgba::from([(grad.r0 * 255.0) as u8, (grad.r1 * 255.0) as u8, 0, 0]);
                img.put_pixel(2, row0, control_color2);
                img.put_pixel(2, row0 + 1, control_color2);
            }
        };
    }
    (Some(DynamicImage::ImageRgba8(img)), texture_size)
}
