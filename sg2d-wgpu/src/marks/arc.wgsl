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
    @location(1) position: vec2<f32>,
    @location(2) start_angle: f32,
    @location(3) end_angle: f32,
    @location(4) outer_radius: f32,
    @location(5) inner_radius: f32,
    @location(6) pad_angle: f32,
    @location(7) corner_radius: f32,
    @location(8) fill: vec4<f32>,
    @location(9) stroke: vec4<f32>,
    @location(10) stroke_width: f32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(1) position: vec2<f32>,
    @location(2) start_angle: f32,
    @location(3) end_angle: f32,
    @location(4) outer_radius: f32,
    @location(5) inner_radius: f32,
    @location(6) pad_angle: f32,
    @location(7) corner_radius: f32,
    @location(8) fill: vec4<f32>,
    @location(9) stroke: vec4<f32>,
    @location(10) stroke_width: f32,
}

const PI = 3.14159265359;
const TAU = 6.2831853072;

@vertex
fn vs_main(
    model: VertexInput,
    instance: InstanceInput,
) -> VertexOutput {
    var out: VertexOutput;
    out.position = instance.position;
    out.start_angle = instance.start_angle;
    out.end_angle = instance.end_angle;
    out.outer_radius = instance.outer_radius;
    out.inner_radius = instance.inner_radius;
    out.pad_angle = instance.pad_angle;
    out.corner_radius = instance.corner_radius;
    out.fill = instance.fill;
    out.stroke = instance.stroke;
    out.stroke_width = instance.stroke_width;

    let buffer = 0.5;
    let half_stroke = out.stroke_width / 2.0;
    let buffered_radius = instance.outer_radius + half_stroke + buffer;
    let x = 2.0 * (model.position[0] * buffered_radius + instance.position[0]) / chart_uniforms.size[0] - 1.0;
    let y = 2.0 * (model.position[1] * buffered_radius + (chart_uniforms.size[1] - instance.position[1])) / chart_uniforms.size[1] - 1.0;

    out.clip_position = vec4<f32>(x, y, 0.0, 1.0);

    return out;
}

fn mod_tau(v: f32) -> f32 {
    var res: f32 = v;
    while (res < 0.0) {
        res += TAU;
    }
    return res % TAU;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let frag_pos = vec2<f32>(in.clip_position[0], in.clip_position[1]);
    let scaled_center = in.position * chart_uniforms.scale;

    let scaled_outer_radius = in.outer_radius * chart_uniforms.scale;
    let scaled_inner_radius = in.inner_radius * chart_uniforms.scale;
    let mid_radius = (scaled_outer_radius + scaled_inner_radius) / 2.0;

    // Compute position of fragment relative to arc center
    let relative_frag = frag_pos - scaled_center;

    // Compute alpha factor for start/end radius
    let frag_radius = length(relative_frag);
    var radius_alpha_factor: f32 = 1.0;
    let radius_buffer = 0.5 * chart_uniforms.scale;
    if (frag_radius > mid_radius) {
        radius_alpha_factor = 1.0 - smoothstep(scaled_outer_radius - radius_buffer, scaled_outer_radius + radius_buffer, frag_radius);
    } else if (scaled_inner_radius > 0.0) {
        radius_alpha_factor = smoothstep(scaled_inner_radius - radius_buffer, scaled_inner_radius + radius_buffer, frag_radius);
    }

    // Compute angle of current fragment. Normalize to interval [0, TAU), adding PI / 2 to align with Vega
    let frag_angle = mod_tau((PI / 2.0) + atan2(relative_frag[1], relative_frag[0]));

    // Check whether end interval crosses the 0/TAU boundary
    let end_angle_turns = floor(in.end_angle / TAU);
    let turns = end_angle_turns;

    // alpha_factor based on angle
    var angle_alpha_factor: f32 = 1.0;
    let angle_buffer = 0.5 * chart_uniforms.scale / frag_radius;

    if (turns > 0.0 && (in.start_angle <= frag_angle || frag_angle + turns * TAU <= in.end_angle)) {
        angle_alpha_factor = max(
            smoothstep(in.start_angle - angle_buffer, in.start_angle + angle_buffer, frag_angle),
            1.0 - smoothstep(in.end_angle - angle_buffer, in.end_angle + angle_buffer, frag_angle + turns * TAU)
        );
    } else if (turns == 0.0 && (mod_tau(in.start_angle) <= frag_angle && frag_angle <= mod_tau(in.end_angle))) {
        angle_alpha_factor = min(
            smoothstep(in.start_angle - angle_buffer, in.start_angle + angle_buffer, frag_angle),
            1.0 - smoothstep(in.end_angle - angle_buffer, in.end_angle + angle_buffer, frag_angle + turns * TAU)
        );
    } else {
        angle_alpha_factor = 0.0;
    }

    var color: vec4<f32> = in.fill;
    color[3] *= min(radius_alpha_factor, angle_alpha_factor);
    return color;
}
