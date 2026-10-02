struct ChartUniform {
    size: vec2<f32>,
    scale: f32,
    _pad: f32,
    translation: vec2<f32>,
    _pad2: vec2<f32>,
};

@group(0) @binding(0)
var<uniform> chart_uniforms: ChartUniform;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) top_left: vec2<f32>,
    @location(3) bottom_right: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) top_left: vec2<f32>,
    @location(2) bottom_right: vec2<f32>,
 }

 // Vertex shader
@vertex
fn vs_main(
  model: VertexInput
) -> VertexOutput {
    var out: VertexOutput;

    // Compute absolute position
    let position = model.position + chart_uniforms.translation;

    // Compute vertex coordinates
    let x = 2.0 * position[0] / chart_uniforms.size[0] - 1.0;
    let y = 2.0 * (chart_uniforms.size[1] - position[1]) / chart_uniforms.size[1] - 1.0;
    out.clip_position = vec4<f32>(x, y, 0.0, 1.0);

    out.color = model.color;
    out.top_left = model.top_left * chart_uniforms.scale;
    out.bottom_right = model.bottom_right * chart_uniforms.scale;
    return out;
}

// Fragment shader
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return lookup_color(in.color, in.clip_position, in.top_left, in.bottom_right);
}

// Gradient texture binding
@group(1) @binding(0)
var gradient_texture: texture_2d<f32>;
@group(1) @binding(1)
var gradient_sampler: sampler;

// Image texture binding
@group(2) @binding(0)
var image_texture: texture_2d<f32>;
@group(2) @binding(1)
var image_sampler: sampler;

// Text texture bindings - both samplers in same group
@group(3) @binding(0)
var text_texture: texture_2d<f32>;
@group(3) @binding(1)
var text_sampler_linear: sampler;
@group(3) @binding(2)
var text_sampler_nearest: sampler;

const GRADIENT_TEXTURE_CODE = -1.0;
const IMAGE_TEXTURE_CODE = -2.0;
const TEXT_TEXTURE_CODE = -3.0;
const TEXT_TEXTURE_NEAREST_CODE = -4.0;

const COLORWAY_LENGTH = 250.0;
const GRADIENT_TEXTURE_WIDTH = 256.0;

// Compute final color, potentially computing gradient
fn lookup_color(color: vec4<f32>, clip_position: vec4<f32>, top_left: vec2<f32>, bottom_right: vec2<f32>) -> vec4<f32> {
    // Use textureSampleGrad instead of textureSample to avoid Uniform Control Flow error in WebGPU
    // https://github.com/gpuweb/gpuweb/discussions/2899
    let texXy = vec2<f32>(clip_position[0], clip_position[1]);
    let dx = dpdx(texXy);
    let dy = dpdx(texXy);

    if (color[0] == GRADIENT_TEXTURE_CODE) {
        // If the first color coordinate is a negative value, this indicates that we are computing a color from a texture
        // For gradient texture, the second color component stores the gradient texture y-coordinate
        let tex_coord_y = color[1];

        let row = i32(tex_coord_y * f32(textureDimensions(gradient_texture).y));
        let p0 = vec2<f32>(gradient_control(0, row), gradient_control(1, row));
        let p1 = vec2<f32>(gradient_control(2, row), gradient_control(3, row));
        let r0 = gradient_control(4, row);
        let r1 = gradient_control(5, row);

        let frag_xy = vec2<f32>(clip_position[0], clip_position[1]);
        let width_height = vec2<f32>(bottom_right[0] - top_left[0], bottom_right[1] - top_left[1]);

        if (r0 < 0.0) {
            if (any(width_height == vec2<f32>(0.0))) { return vec4<f32>(0.0); }
           // Convert fragment coordinate into coordinate normalized to rect bounding box
            let norm_xy = (frag_xy - top_left) / width_height;

            let control_dist = distance(p0, p1);
            if (control_dist == 0.0) {
                return textureSampleGrad(gradient_texture, gradient_sampler, vec2<f32>(compute_tex_x_coord(1.0), tex_coord_y), dx, dy);
            }
            let projected_dist = dot(norm_xy - p0, p1 - p0) / control_dist;

            let tex_coord_x = compute_tex_x_coord(projected_dist / control_dist);
            let tex_coords = vec2<f32>(tex_coord_x, tex_coord_y);
            return textureSampleGrad(gradient_texture, gradient_sampler, tex_coords, dx, dy);
        } else {
            // Expand top_left and bottom_right so they form a square
            var square_top_left: vec2<f32>;
            var square_bottom_right: vec2<f32>;
            var side: f32;
            if (width_height[0] > width_height[1]) {
                // wider than tall, push out y coordinates until square
                let delta = (width_height[0] - width_height[1]) / 2.0;
                square_top_left = vec2<f32>(top_left[0], top_left[1] - delta);
                square_bottom_right = vec2<f32>(bottom_right[0], bottom_right[1] + delta);
                side = width_height[0];
            } else if (width_height[0] < width_height[1]) {
                // taller than wide, push out x coordinates until square
                let delta = (width_height[1] - width_height[0]) / 2.0;
                square_top_left = vec2<f32>(top_left[0] - delta, top_left[1]);
                square_bottom_right = vec2<f32>(bottom_right[0] + delta, bottom_right[1]);
                side = width_height[1];
            } else {
                // already square
                square_top_left = top_left;
                square_bottom_right = bottom_right;
                side = width_height[0];
            }

            // Normalize the fragment coordinates to square
            if (side == 0.0) { return vec4<f32>(0.0); }
            let norm_xy = (frag_xy - square_top_left) / side;
            let result = radial_parameter(norm_xy, p0, p1, r0, r1);
            if (result.y == 0.0) { return vec4<f32>(0.0); }
            let grad_dist = result.x;
            let tex_coord_x = compute_tex_x_coord(grad_dist);
            let tex_coords = vec2<f32>(tex_coord_x, tex_coord_y);
            return textureSampleGrad(gradient_texture, gradient_sampler, tex_coords, dx, dy);
        }
    } else if (color[0] == IMAGE_TEXTURE_CODE) {
        // Image texture coordinates are stored in the second and third color components
        let tex_coords = vec2<f32>(color[1], color[2]);
        return straight_image_sample(textureSampleGrad(image_texture, image_sampler, tex_coords, dx, dy));
    } else if (color[0] == TEXT_TEXTURE_CODE) {
        // Text texture coordinates are stored in the second and third color components (Linear filtering)
        let tex_coords = vec2<f32>(color[1], color[2]);
        return textureSampleGrad(text_texture, text_sampler_linear, tex_coords, dx, dy);
    } else if (color[0] == TEXT_TEXTURE_NEAREST_CODE) {
        // Text texture coordinates for nearest filtering
        let tex_coords = vec2<f32>(color[1], color[2]);
        return textureSampleGrad(text_texture, text_sampler_nearest, tex_coords, dx, dy);
    } else {
        return color;
    }
}

fn compute_tex_x_coord(grad_dist: f32) -> f32 {
    let col_offset = GRADIENT_TEXTURE_WIDTH - COLORWAY_LENGTH;
    return clamp(grad_dist, 0.0, 1.0) * COLORWAY_LENGTH / GRADIENT_TEXTURE_WIDTH + col_offset / GRADIENT_TEXTURE_WIDTH;
}

// Metadata uses the four bytes of each unfiltered texel to preserve one f32.
fn gradient_control(column: i32, row: i32) -> f32 {
    let bytes = vec4<u32>(round(textureLoad(gradient_texture, vec2<i32>(column, row), 0) * 255.0));
    return bitcast<f32>(bytes.x | (bytes.y << 8u) | (bytes.z << 16u) | (bytes.w << 24u));
}

// The second component indicates whether a circle contributes at this pixel.
fn radial_parameter(p: vec2<f32>, p0: vec2<f32>, p1: vec2<f32>, r0: f32, r1: f32) -> vec2<f32> {
    let d = p1 - p0;
    let dr = r1 - r0;
    if (all(d == vec2<f32>(0.0)) && dr == 0.0) { return vec2<f32>(0.0); }
    let q = p - p0;
    let a = dot(d, d) - dr * dr;
    let b = -2.0 * (dot(q, d) + r0 * dr);
    let c = dot(q, q) - r0 * r0;
    if (abs(a) <= 1e-6 * (dot(d, d) + dr * dr)) {
        if (b == 0.0) { return vec2<f32>(0.0); }
        let t = -c / b;
        return vec2<f32>(t, select(0.0, 1.0, r0 + t * dr >= 0.0));
    }
    let discriminant = b * b - 4.0 * a * c;
    if (discriminant < 0.0) { return vec2<f32>(0.0); }
    let root = sqrt(discriminant);
    // This form avoids subtracting nearly equal terms in one quadratic root.
    let numerator = -0.5 * (b + select(-root, root, b >= 0.0));
    var t0 = -b / (2.0 * a);
    var t1 = t0;
    if (numerator != 0.0) {
        t0 = numerator / a;
        t1 = c / numerator;
    }
    let valid0 = r0 + t0 * dr >= 0.0;
    let valid1 = r0 + t1 * dr >= 0.0;
    if (valid0 && valid1) { return vec2<f32>(max(t0, t1), 1.0); }
    if (valid0) { return vec2<f32>(t0, 1.0); }
    if (valid1) { return vec2<f32>(t1, 1.0); }
    return vec2<f32>(0.0);
}

fn straight_image_sample(sampled: vec4<f32>) -> vec4<f32> {
    if (sampled.a == 0.0) { return vec4<f32>(0.0); }
    return vec4<f32>(sampled.rgb / sampled.a, sampled.a);
}
