// Vertex shader

struct ChartUniform {
    size: vec2<f32>,
    scale: f32,
    _pad: f32, // for 16 byte alignment
};

@group(0) @binding(0)
var<uniform> chart_uniforms: ChartUniform;

struct VertexInput {
    @location(0) position: vec2<f32>,
};

struct InstanceInput {
    @location(1) x0: f32,
    @location(2) y0: f32,
    @location(3) x1: f32,
    @location(4) y1: f32,
    @location(5) stroke: vec4<f32>,
    @location(6) stroke_width: f32,
    @location(7) stroke_cap: u32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) p0: vec2<f32>,
    @location(2) p1: vec2<f32>,
    @location(3) radius: f32,
    @location(4) stroke_half_width: f32,
};

const PI = 3.14159265359;

const STROKE_CAP_BUTT: u32 = 0u;
const STROKE_CAP_SQUARE: u32 = 1u;
const STROKE_CAP_ROUND: u32 = 2u;

@vertex
fn vs_main(
    model: VertexInput,
    instance: InstanceInput,
) -> VertexOutput {
    var out: VertexOutput;
    out.color = instance.stroke;

    var width: f32 = instance.stroke_width;
    var p0: vec2<f32> = vec2(instance.x0, instance.y0);
    var p1: vec2<f32> = vec2(instance.x1, instance.y1);
    let mid = (p0 + p1) / 2.0;
    var len: f32 = distance(p0, p1);
    if (instance.stroke_cap == STROKE_CAP_ROUND) {
        // extend length, but leave p0 and p1 at the center of the circleular end caps
        len += width;
    } else if (instance.stroke_cap == STROKE_CAP_SQUARE) {
        // Extend length and move p0 and p1 to the outer edge of the square
        len += width;
        let p0p1_norm = normalize(p1 - p0);
        p0 -= p0p1_norm * (width / 2.0);
        p1 += p0p1_norm * (width / 2.0);
    }

    let should_anitalias = instance.stroke_cap == STROKE_CAP_ROUND || (instance.x0 != instance.x1 && instance.y0 != instance.y1);
    if (should_anitalias) {
        // Add anti-aliasing buffer for rules with rounded caps and all diagonal rules.
        // Non-round rules that are vertical or horizontal don't get anti-aliasing.
        len += chart_uniforms.scale;
        width += chart_uniforms.scale;
    }

    let normed = normalize(p1 - p0);
    let angle = (PI / 2.0) + atan2(normed[1], normed[0]);
    let rot = mat2x2(cos(angle), -sin(angle), sin(angle), cos(angle));

    let rot_pos = rot * vec2(model.position[0] * width, model.position[1] * len);
    let x = 2.0 * (rot_pos[0] + mid[0]) / chart_uniforms.size[0] - 1.0;
    let y = 2.0 * (rot_pos[1] + (chart_uniforms.size[1] - mid[1])) / chart_uniforms.size[1] - 1.0;

    out.clip_position = vec4<f32>(x, y, 0.0, 1.0);

    out.p0 = p0 * chart_uniforms.scale;
    out.p1 = p1 * chart_uniforms.scale;

    out.stroke_half_width = instance.stroke_width * chart_uniforms.scale / 2.0;
    if (instance.stroke_cap == STROKE_CAP_ROUND) {
        out.radius = instance.stroke_width * chart_uniforms.scale / 2.0;
    } else {
        out.radius = 0.0;
    }

    return out;
}

// Fragment shader
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {

    let top_left = vec2<f32>(
        min(in.p0[0], in.p1[0]),
        min(in.p0[1], in.p1[1]),
    );
    let bottom_right = vec2<f32>(
       max(in.p0[0], in.p1[0]),
       max(in.p0[1], in.p1[1]),
    );
    let color = lookup_color(in.color, in.clip_position, top_left, bottom_right);

    let should_antialias = in.radius > 0.0 || (in.p0[0] != in.p1[0] && in.p0[1] != in.p1[1]);
    if (!should_antialias) {
        // This is a butt or square cap and fully vertical or horizontal
        // vertex boundary matches desired rule area and we don't need to do any
        // anti-aliasing.
        return color;
    }

    let frag_pos = vec2<f32>(in.clip_position[0], in.clip_position[1]);
    let relative_frag_pos = frag_pos - in.p0;

    let relative_p1 = in.p1 - in.p0;
    let relative_p0 = vec2<f32>(0.0, 0.0);

    let len = length(relative_p1);
    let projected_frag_dist = dot(relative_frag_pos, relative_p1) / length(relative_p1);
    let perpendicular_frag_pos = relative_frag_pos - normalize(relative_p1) * projected_frag_dist;

    // Compute fragment distance for anit-aliasing
    var dist: f32 = 0.0;
    if (in.radius > 0.0) {
        // rounded cap
        if (projected_frag_dist < 0.0) {
            // distance to p0
            dist = distance(relative_frag_pos, relative_p0);
        } else if (projected_frag_dist > len) {
            // distance to p1
            dist = distance(relative_frag_pos, relative_p1);
        } else {
            // distance to line connecting p0 and p1
            dist = length(perpendicular_frag_pos);
        }
    } else {
        // rule square or butt cap on a diagonal
        if (projected_frag_dist < in.stroke_half_width) {
            dist = max(-projected_frag_dist + in.stroke_half_width, length(perpendicular_frag_pos));
        } else if (projected_frag_dist > len - in.stroke_half_width) {
            dist = max(projected_frag_dist - len + in.stroke_half_width, length(perpendicular_frag_pos));
        } else {
            dist = length(perpendicular_frag_pos);
        }
    }

    let buffer = chart_uniforms.scale / 2.0;
    let alpha_factor = 1.0 - smoothstep(in.stroke_half_width - buffer, in.stroke_half_width + buffer, dist);
    var adjusted_color = color;
    adjusted_color[3] *= alpha_factor;
    return adjusted_color;
}

// Gradient color
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