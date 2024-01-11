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
    @location(1) normal: vec2<f32>,
    @location(2) kind: u32,
    @location(3) shape_index: u32,
};

struct InstanceInput {
    @location(4) position: vec2<f32>,
    @location(5) fill_color: vec4<f32>,
    @location(6) stroke_color: vec4<f32>,
    @location(7) stroke_width: f32,
    @location(8) size: f32,
    @location(9) angle: f32,
    @location(10) shape_index: u32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,

    // if 1.0, draw the fragment, otherwise discard
    @location(0) draw_shape: f32,

    // If 1.0, the vertex is part of the bounding box of a circle symbol
    // otherwise, vertex is part of a geometric (path-based) symbol
    @location(1) is_circle: f32,

    // Color of vertex when drawing geometric symbol based on a path
    @location(2) geom_color: vec4<f32>,

    // Properties of circle symbol. Circles are drawn in the fragment shader,
    // so more info must be passed through
    @location(3) circle_center: vec2<f32>,
    @location(4) circle_radius: f32,
    @location(5) circle_fill_color: vec4<f32>,
    @location(6) circle_stroke_color: vec4<f32>,
    @location(7) circle_stroke_width: f32,
};

const PI = 3.14159265359;

@vertex
fn vs_main(
    model: VertexInput,
    instance: InstanceInput,
) -> VertexOutput {
    var out: VertexOutput;

    if (instance.shape_index != model.shape_index) {
        out.draw_shape = 0.0;
        return out;
    } else {
        out.draw_shape = 1.0;
    }

    let size_scale = sqrt(instance.size);

    // Compute scenegraph x and y coordinates
    let angle_rad = PI * instance.angle / 180.0;
    let rot = mat2x2(cos(angle_rad), -sin(angle_rad), sin(angle_rad), cos(angle_rad));
    let rotated_pos = rot * model.position;
    let sg_x = rotated_pos[0] * size_scale + instance.position[0];
    let sg_y = rotated_pos[1] * size_scale + (chart_uniforms.size[1] - instance.position[1]);
    let pos = vec2(sg_x, sg_y);

    if (model.kind == 0u) {
        // fill vertex
        out.geom_color = instance.fill_color;

        let normalized_pos = 2.0 * pos / chart_uniforms.size - 1.0;
        out.clip_position = vec4<f32>(normalized_pos, 0.0, 1.0);
        out.is_circle = 0.0;
    } else if (model.kind == 1u) {
        // stroke vertex
        out.geom_color = instance.stroke_color;

        // Compute scaled stroke width.
        // The 0.1 here is the width that lyon used to compute the stroke tesselation
        let scaled_stroke_width = 0.1 * size_scale;

        // Adjust vertex along normal to achieve desired line width
        // The factor of 2.0 here is because the normal vector that lyon
        // returns has length such that moving all stroke vertices by the length
        // of the "normal" vector will increase the line width by 2.
        let normal = rot * model.normal;
        var diff = scaled_stroke_width - instance.stroke_width;
        let adjusted_pos = pos - diff * normal / 2.0;

        let normalized_pos = 2.0 * adjusted_pos / chart_uniforms.size - 1.0;
        out.clip_position = vec4<f32>(normalized_pos, 0.0, 1.0);
        out.is_circle = 0.0;
    } else if (model.kind == 2u) {
        // circle symbol. Circles are drawn in the fragment shader, so
        // we compute the center and radius and pass through the stroke and
        // fill specifications.
        out.is_circle = 1.0;

        // Pass through colors and stroke_width
        out.circle_fill_color = instance.fill_color;
        out.circle_stroke_color = instance.stroke_color;
        out.circle_stroke_width = instance.stroke_width;

        // Compute normalized position
        let normalized_pos = 2.0 * pos / chart_uniforms.size - 1.0;
        out.clip_position = vec4<f32>(normalized_pos, 0.0, 1.0);

        // Compute circle center in fragment shader coordinates
        out.circle_center = vec2<f32>(
            instance.position[0] * chart_uniforms.scale,
            instance.position[1] * chart_uniforms.scale,
        );

        // Compute radius in fragment shader coordinates
        out.circle_radius = size_scale * chart_uniforms.scale / 2.0;
    }

    return out;
}

// Fragment shader
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    if (in.draw_shape != 1.0) {
        discard;
    } if (in.is_circle == 1.0) {
        // Draw anti-aliased circle
        let buffer = 0.5 * chart_uniforms.scale;
        let dist = length(in.circle_center - vec2<f32>(in.clip_position[0], in.clip_position[1]));

        if (in.circle_stroke_width > 0.0) {
            let inner_radius = in.circle_radius - in.circle_stroke_width * chart_uniforms.scale / 2.0;
            let outer_radius = in.circle_radius + in.circle_stroke_width * chart_uniforms.scale / 2.0;
            if (dist > outer_radius + buffer * 2.0) {
                discard;
            } else {
                let alpha_factor = 1.0 - smoothstep(outer_radius - buffer, outer_radius + buffer, dist);
                let mix_factor = 1.0 - smoothstep(inner_radius - buffer, inner_radius + buffer, dist);
                var mixed_color: vec4<f32> = mix(in.circle_stroke_color, in.circle_fill_color, mix_factor);
                mixed_color[3] *= alpha_factor;
                return mixed_color;
            }
        } else {
            let alpha_factor = 1.0 - smoothstep(in.circle_radius - buffer, in.circle_radius + buffer, dist);
            var mixed_color: vec4<f32> = in.circle_fill_color;
            mixed_color[3] *= alpha_factor;
            if (dist > in.circle_radius + buffer) {
                discard;
            } else {
                return mixed_color;
            }
        }
    } else {
        return in.geom_color;
    }
}
