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

    // Compute absolute position
    let position = instance.position + chart_uniforms.origin;

    out.position = position;
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
    let x = 2.0 * (model.position[0] * buffered_radius + position[0]) / chart_uniforms.size[0] - 1.0;
    let y = 2.0 * (model.position[1] * buffered_radius + (chart_uniforms.size[1] - position[1])) / chart_uniforms.size[1] - 1.0;

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

    // Compute scaled values position
    let scaled_center = in.position * chart_uniforms.scale;
    let outer_radius = in.outer_radius * chart_uniforms.scale;
    let inner_radius = in.inner_radius * chart_uniforms.scale;
    let half_stroke = in.stroke_width * chart_uniforms.scale / 2.0;
    let outer_stroke_radius = outer_radius + half_stroke;
    let inner_stroke_radius = inner_radius - half_stroke;
    let mid_radius = (outer_radius + inner_radius) / 2.0;

    let top_left = scaled_center - outer_radius;
    let bottom_right = scaled_center + outer_radius;
    let fill = lookup_color(in.fill, in.clip_position, top_left, bottom_right);
    let stroke = lookup_color(in.stroke, in.clip_position, top_left, bottom_right);

    // Compute position of fragment relative to arc center
    let frag_pos = vec2<f32>(in.clip_position[0], in.clip_position[1]);
    let relative_frag = frag_pos - scaled_center;

    // Initialize alpha nand color mix factors for start/end radius boundaries
    var radius_alpha_factor: f32 = 1.0;
    var radius_mix_factor: f32 = 1.0;

    // antialias buffer distance
    let distance_buffer = 0.5 * chart_uniforms.scale;

    let frag_radius = length(relative_frag);
    if (frag_radius > mid_radius) {
        radius_alpha_factor = 1.0 - smoothstep(outer_stroke_radius - distance_buffer, outer_stroke_radius + distance_buffer, frag_radius);
        radius_mix_factor = 1.0 - smoothstep(
            outer_stroke_radius - 2.0 * half_stroke - distance_buffer,
            outer_stroke_radius - 2.0 * half_stroke + distance_buffer,
            frag_radius
        );
    } else if (inner_radius > 0.0) {
        radius_alpha_factor = smoothstep(inner_stroke_radius - distance_buffer, inner_stroke_radius + distance_buffer, frag_radius);
        radius_mix_factor = smoothstep(
            inner_stroke_radius + 2.0 * half_stroke - distance_buffer,
            inner_stroke_radius + 2.0 * half_stroke + distance_buffer,
            frag_radius
        );
    }

    // Compute angle of current fragment. Normalize to interval [0, TAU), adding PI / 2 to align with Vega
    let frag_angle = mod_tau((PI / 2.0) + atan2(relative_frag[1], relative_frag[0]));

    // compute projected distance from fragment position to lines from center
    // point at start and end angles
    let start_theta = min(
        abs(frag_angle - in.start_angle),
        abs(frag_angle - mod_tau(in.start_angle)),
    );
    let end_theta = min(
        abs(frag_angle - in.end_angle),
        abs(frag_angle - mod_tau(in.end_angle)),
    );

    // Compute distance of fragment to line from center along start_theta.
    // Use a large value if the fragment is on the oposite side
    var start_proj_dist: f32;
    if (start_theta < PI / 2.0) {
        start_proj_dist = abs(sin(start_theta) * length(relative_frag));
    } else {
        start_proj_dist = 1000000.0;
    }

    var end_proj_dist: f32;
    if (end_theta < PI / 2.0) {
        end_proj_dist = abs(sin(end_theta) * length(relative_frag));
    } else {
        end_proj_dist = 1000000.0;
    }

    // Check whether end interval crosses the 0/TAU boundary
    let end_angle_turns = floor(in.end_angle / TAU);
    let turns = end_angle_turns;

    // alpha_factor based on angle
    var angle_alpha_factor: f32;
    var angle_mix_factor: f32;

    let in_straddle_arc = turns > 0.0 && (in.start_angle <= frag_angle || frag_angle + turns * TAU <= in.end_angle);
    let in_non_straddle_arc = turns == 0.0 && (mod_tau(in.start_angle) <= frag_angle && frag_angle <= mod_tau(in.end_angle));
    if (in_straddle_arc || in_non_straddle_arc) {
        // Inside an arc
        angle_mix_factor = smoothstep(
            half_stroke - distance_buffer,
            half_stroke + distance_buffer,
            min(start_proj_dist, end_proj_dist)
        );
        angle_alpha_factor = 1.0;
    } else {
        // Outside of the arc, apply anit-aliasing
        angle_alpha_factor = 1.0 - smoothstep(
            half_stroke - distance_buffer,
            half_stroke + distance_buffer,
            min(start_proj_dist, end_proj_dist)
        );
        angle_mix_factor = 0.0;
    }

    var mixed_color: vec4<f32>;
    if (in.stroke_width > 0.0) {
        mixed_color = mix(stroke, fill, min(radius_mix_factor, angle_mix_factor));
    } else {
        mixed_color = fill;
    }

    mixed_color[3] *= min(radius_alpha_factor, angle_alpha_factor);
    return mixed_color;
}
