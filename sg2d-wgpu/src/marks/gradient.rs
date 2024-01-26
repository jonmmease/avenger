use crate::marks::rect::GRADIENT_TEXTURE_HEIGHT;
use sg2d::marks::value::ColorOrGradient;

pub fn to_color_or_gradient_coord(color_or_gradient: &ColorOrGradient) -> [f32; 4] {
    match color_or_gradient {
        ColorOrGradient::Color(c) => *c,
        ColorOrGradient::GradientIndex(grad_idx) => {
            // Each gradient colorway is written to two rows, starting at texture
            // y-coordinate 0. This results in 128 gradients stored in a 256x256 texture. To
            // avoid interpolation artifacts, we compute the texture coordinate as the
            // position between the two rows
            let num_gradient_rows = GRADIENT_TEXTURE_HEIGHT as f32 / 2.0;
            let grad_coord = (*grad_idx as f32 / num_gradient_rows)
                + 1.0 / (GRADIENT_TEXTURE_HEIGHT as f32 * 2.0);
            [-grad_coord, 0.0, 0.0, 0.0]
        }
    }
}
