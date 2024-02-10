// Vertex shader
struct ChartUniform {
    size: vec2<f32>,
    origin: vec2<f32>,
    scale: f32,
    _pad: vec2<f32>,
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
    @location(8) relative_scale: f32,
    @location(9) angle: f32,
    @location(10) shape_index: u32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,

    // if 1.0, draw the fragment, otherwise discard
    @location(0) draw_shape: f32,

    // Color of vertex when drawing geometric symbol based on a path
    @location(1) color: vec4<f32>,
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

    let size_scale = sqrt(instance.relative_scale);

    // Compute absolute position
    let position = instance.position + chart_uniforms.origin;

    // Compute scenegraph x and y coordinates
    let angle_rad = PI * instance.angle / 180.0;
    let rot = mat2x2(cos(angle_rad), -sin(angle_rad), sin(angle_rad), cos(angle_rad));
    let rotated_pos = rot * model.position;
    let sg_x = rotated_pos[0] * instance.relative_scale + position[0];
    let sg_y = rotated_pos[1] * instance.relative_scale + (chart_uniforms.size[1] - position[1]);
    let pos = vec2(sg_x, sg_y);

    if (model.kind == 0u) {
        // fill vertex
        out.color = instance.fill_color;

        let normalized_pos = 2.0 * pos / chart_uniforms.size - 1.0;
        out.clip_position = vec4<f32>(normalized_pos, 0.0, 1.0);
    } else if (model.kind == 1u) {
        // stroke vertex
        out.color = instance.stroke_color;

        // Compute scaled stroke width.
        // The shape is tesselated with lyon with a stroke with of 1.0, so this get's scaled down by relative
        // scale factor
        let scaled_stroke_width = instance.relative_scale;

        // Adjust vertex along normal to achieve desired line width
        // The factor of 2.0 here is because the normal vector that lyon
        // returns has length such that moving all stroke vertices by the length
        // of the "normal" vector will increase the line width by 2.
        let normal = rot * model.normal;
        var diff = scaled_stroke_width - instance.stroke_width;
        let adjusted_pos = pos - diff * normal / 2.0;

        let normalized_pos = 2.0 * adjusted_pos / chart_uniforms.size - 1.0;
        out.clip_position = vec4<f32>(normalized_pos, 0.0, 1.0);
    }

    return out;
}

// Fragment shader
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    if (in.draw_shape != 1.0) {
        discard;
    }
    return in.color;
}
