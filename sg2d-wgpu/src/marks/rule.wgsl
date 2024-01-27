// Vertex shader

struct ChartUniform {
    size: vec2<f32>,
    origin: vec2<f32>,
    group_size: vec2<f32>,
    scale: f32,
    clip: f32,
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
    var p0: vec2<f32> = vec2(instance.x0, instance.y0) + chart_uniforms.origin;
    var p1: vec2<f32> = vec2(instance.x1, instance.y1) + chart_uniforms.origin;
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

    let should_anitalias = instance.stroke_cap == STROKE_CAP_ROUND || (p0[0] != p1[0] && p0[1] != p1[1]);
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
