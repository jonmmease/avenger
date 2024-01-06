// Vertex shader

struct ChartUniform {
    size: vec2<f32>,
    filler: vec2<f32>, // for 16 byte alignment
};
@group(0) @binding(0)
var<uniform> chart_uniforms: ChartUniform;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) normal: vec2<f32>,
    @location(2) kind: u32,
};

struct InstanceInput {
    @location(3) position: vec2<f32>,
    @location(4) fill_color: vec4<f32>,
    @location(5) stroke_color: vec4<f32>,
    @location(6) stroke_width: f32,
    @location(7) size: f32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};


@vertex
fn vs_main(
    model: VertexInput,
    instance: InstanceInput,
) -> VertexOutput {
    var out: VertexOutput;
    let size_scale = sqrt(instance.size);

    // Compute scenegraph x and y coordinates
    let sg_x = model.position[0] * size_scale + instance.position[0];
    let sg_y = model.position[1] * size_scale + (chart_uniforms.size[1] - instance.position[1]);
    let pos = vec2(sg_x, sg_y);

    if (model.kind == 0u) {
        // fill vertex
        out.color = instance.fill_color;

        let normalized_pos = 2.0 * pos / chart_uniforms.size - 1.0;
        out.clip_position = vec4<f32>(normalized_pos, 0.0, 1.0);
    } else {
        // stroke vertex
        out.color = instance.stroke_color;

        // Compute scaled stroke width.
        // The 0.1 here is the width that lyon used to compute the stroke tesselation
        let scaled_stroke_width = 0.1 * size_scale;

        // Adjust vertex along normal to achieve desired line width
        // The factor of 2.0 here is because the normal vector that lyon
        // returns has length such that moving all stroke vertices by the length
        // of the "normal" vector will increase the line width by 2.
        var diff = scaled_stroke_width - instance.stroke_width;
        let adjusted_pos = pos - diff * model.normal / 2.0;

        let normalized_pos = 2.0 * adjusted_pos / chart_uniforms.size - 1.0;
        out.clip_position = vec4<f32>(normalized_pos, 0.0, 1.0);
    }

    return out;
}

// Fragment shader

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
