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
    @location(2) fill: vec4<f32>,
    @location(3) width: f32,
    @location(4) height: f32,
    @location(5) stroke: vec4<f32>,
    @location(6) stroke_width: f32,
    @location(7) corner_radius: f32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) fill: vec4<f32>,
    @location(1) stroke: vec4<f32>,
    @location(2) stroke_width: f32,
    @location(3) corner_radius: f32,

    // Outer points are outside of stroke
    @location(4) outer_top_left: vec2<f32>,
    @location(5) outer_bottom_right: vec2<f32>,

    // Inner points are centers of the corner radius
    @location(6) inner_top_left: vec2<f32>,
    @location(7) inner_bottom_right: vec2<f32>,
};

@vertex
fn vs_main(
    model: VertexInput,
    instance: InstanceInput,
) -> VertexOutput {
    var out: VertexOutput;
    // Pass through values
    out.fill = instance.fill;
    out.stroke = instance.stroke;
    out.stroke_width = instance.stroke_width;

    // corner_radius may not be less than half the rect height
    let corner_radius = min(instance.corner_radius, instance.height / 2.0);
    out.corner_radius = corner_radius;

    // Compute absolute position
    let position = instance.position + chart_uniforms.origin;

    // Compute corner points in fragment shader coordinates
    let half_stroke = instance.stroke_width / 2.0;
    let width_height = vec2<f32>(instance.width, instance.height);
    out.outer_top_left = (position - half_stroke) * chart_uniforms.scale;
    out.outer_bottom_right = (position + width_height + half_stroke) * chart_uniforms.scale;

    // Compute corner radius center points in fragment shader coordinates
    out.inner_top_left = (position + corner_radius) * chart_uniforms.scale;
    out.inner_bottom_right = (position + width_height - corner_radius) * chart_uniforms.scale;

    // Compute vertex coordinates
    let x = 2.0 * (model.position[0] * (instance.width + instance.stroke_width) + position[0] - half_stroke) / chart_uniforms.size[0] - 1.0;
    let y = 2.0 * (model.position[1] * (instance.height + instance.stroke_width) + (chart_uniforms.size[1] - position[1] - instance.height - half_stroke)) / chart_uniforms.size[1] - 1.0;
    out.clip_position = vec4<f32>(x, y, 0.0, 1.0);
    return out;
}

// Fragment shader
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let scaled_radius = in.corner_radius * chart_uniforms.scale;
    let scaled_stroke_width = in.stroke_width * chart_uniforms.scale;
    let frag_xy = vec2<f32>(in.clip_position[0], in.clip_position[1]);

    // Compute fill and stroke, potentially based on gradient
    let fill = lookup_color(in.fill, in.clip_position, in.outer_top_left, in.outer_bottom_right);
    let stroke = lookup_color(in.stroke, in.clip_position, in.outer_top_left, in.outer_bottom_right);

    if (scaled_radius > 0.0) {
        // has rounded corners
        let inner_bottom_left = vec2<f32>(in.inner_top_left[0], in.inner_bottom_right[1]);
        let inner_top_right = vec2<f32>(in.inner_bottom_right[0], in.inner_top_left[1]);

        let in_top_left = in.clip_position[0] < in.inner_top_left[0] && in.clip_position[1] < in.inner_top_left[1];
        let in_top_right = inner_top_right[0] < in.clip_position[0] && in.clip_position[1] < inner_top_right[1];
        let in_bottom_right = in.inner_bottom_right[0] < in.clip_position[0] && in.inner_bottom_right[1] < in.clip_position[1];
        let in_bottom_left = in.clip_position[0] < inner_bottom_left[0] && inner_bottom_left[1] < in.clip_position[1];

        let buffer = 0.5 * chart_uniforms.scale;
        if (scaled_stroke_width > 0.0) {
            var dist: f32 = scaled_radius;
            if (in_top_left) {
                dist = distance(in.inner_top_left, frag_xy);
            } else if (in_bottom_right) {
                dist = distance(in.inner_bottom_right, frag_xy);
            } else if (in_bottom_left) {
                dist = distance(inner_bottom_left, frag_xy);
            } else if (in_top_right) {
                dist = distance(inner_top_right, frag_xy);
            } else {
                let right_dist = frag_xy[0] - inner_top_right[0];
                let left_dist = in.inner_top_left[0] - frag_xy[0];
                let top_dist = in.inner_top_left[1] - frag_xy[1];
                let bottom_dist = frag_xy[1] - in.inner_bottom_right[1];
                dist = max(max(right_dist, left_dist), max(bottom_dist, top_dist));
            }

            let stroke_radius = scaled_radius + scaled_stroke_width / 2.0;
            let outer_factor = 1.0 - smoothstep(stroke_radius - buffer, stroke_radius + buffer, dist);

            let inner_radius = scaled_radius - scaled_stroke_width / 2.0;
            let inner_factor = 1.0 - smoothstep(inner_radius - buffer, inner_radius + buffer, dist);

            var mixed_color: vec4<f32>;

            if (fill[3] == 0.0) {
                mixed_color = stroke;
                mixed_color[3] *= outer_factor * (1.0 - inner_factor);
            } else {
                mixed_color = mix(stroke, fill, inner_factor);
                mixed_color[3] *= outer_factor;
            }

            return mixed_color;
        } else {
            var dist: f32 = scaled_radius;
            if (in_top_left) {
                dist = distance(in.inner_top_left, frag_xy);
            } else if (in_bottom_right) {
                dist = distance(in.inner_bottom_right, frag_xy);
            } else if (in_bottom_left) {
                dist = distance(inner_bottom_left, frag_xy);
            } else if (in_top_right) {
                dist = distance(inner_top_right, frag_xy);
            } else {
                // skip anit-aliasing when not in a corner
                return fill;
            }

            let alpha_factor = 1.0 - smoothstep(scaled_radius - buffer, scaled_radius + buffer, dist);
            var color: vec4<f32> = fill;
            color[3] *= alpha_factor;
            return color;
        }
    } else {
        // no rounded corners
        if (scaled_stroke_width > 0.0) {
            // has stroke
            let in_left_stroke = in.clip_position[0] - in.outer_top_left[0] < scaled_stroke_width;
            let in_right_stroke = in.outer_bottom_right[0] - in.clip_position[0]  < scaled_stroke_width;
            let in_top_stroke = in.clip_position[1] - in.outer_top_left[1] < scaled_stroke_width;
            let in_bottom_stroke = in.outer_bottom_right[1] - in.clip_position[1] < scaled_stroke_width;

            let in_stroke = in_left_stroke || in_right_stroke || in_bottom_stroke || in_top_stroke;
            if (in_stroke) {
                return stroke;
            } else {
                return fill;
            }
        } else {
            // no stroke
            return fill;
        }
    }
}
