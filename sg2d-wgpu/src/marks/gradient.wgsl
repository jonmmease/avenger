// Gradient color logic that provides the `lookup_color` function.
// This is intended to be concatenated to the end of shader files that support
// gradients

const GRADIENT_LINEAR = 1.0;
const GRADIENT_RADIAL = 2.0;

const COLORWAY_LENGTH = 250.0;
const GRADIENT_TEXTURE_WIDTH = 256.0;
const GRADIENT_TEXTURE_HEIGHT = 256.0;

@group(1) @binding(0)
var gradient_texture: texture_2d<f32>;
@group(1) @binding(1)
var linear_sampler: sampler;
@group(1) @binding(2)
var nearest_sampler: sampler;

// Compute final color, potentially computing gradient
fn lookup_color(color: vec4<f32>, clip_position: vec4<f32>, top_left: vec2<f32>, bottom_right: vec2<f32>) -> vec4<f32> {
    if (color[0] < 0.0) {
        // If the first color coordinate is negative, this indicates that we need to compute a gradient.
        // The negative of this value is the y-coordinate into the gradient texture where the gradient control
        // points and gradient colorway are stored.
        let tex_coord_y = -color[0];

        // Extract gradient type from fist pixel using nearest sampler (so that not interpolation is performed)
        let control0 = textureSample(gradient_texture, nearest_sampler, vec2<f32>(0.0, tex_coord_y));
        let gradient_type = control0[0];

        // Extract x/y control points from second pixel
        let control1 = textureSample(gradient_texture, nearest_sampler, vec2<f32>(1.0 / GRADIENT_TEXTURE_WIDTH, tex_coord_y));
        let x0 = control1[0];
        let y0 = control1[1];
        let x1 = control1[2];
        let y1 = control1[3];

        if (gradient_type == GRADIENT_LINEAR) {
           // Convert fragment coordinate into coordinate normalized to rect bounding box
            let frag_xy = vec2<f32>(clip_position[0], clip_position[1]);
            let width_height = vec2<f32>(bottom_right[0] - top_left[0], bottom_right[1] - top_left[1]);
            let norm_xy = (frag_xy - top_left) / width_height;

            let p0 = vec2<f32>(x0, y0);
            let p1 = vec2<f32>(x1, y1);
            let control_dist = distance(p0, p1);
            let projected_dist = dot(norm_xy - p0, p1 - p0) / control_dist;
            let col_offset = GRADIENT_TEXTURE_WIDTH - COLORWAY_LENGTH;
            let tex_coord_x = clamp(projected_dist / control_dist, 0.0, 1.0) * COLORWAY_LENGTH / GRADIENT_TEXTURE_WIDTH + col_offset / GRADIENT_TEXTURE_WIDTH;

            return textureSample(gradient_texture, linear_sampler, vec2<f32>(tex_coord_x, tex_coord_y));
        } else {
           // Extract additional radius gradient control points from third pixel
            let control2 = textureSample(gradient_texture, nearest_sampler, vec2<f32>(2.0 / GRADIENT_TEXTURE_WIDTH, tex_coord_y));
            let r0 = control2[0];
            let r1 = control2[1];

            // TODO: compute radial gradinet
            return vec4<f32>(1.0, 0.0, 0.0, 1.0);
        }
    } else {
        return color;
    }
}