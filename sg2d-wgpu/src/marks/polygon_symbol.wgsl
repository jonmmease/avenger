// Vertex shader

struct ChartUniform {
    size: vec2<f32>,
    filler: vec2<f32>, // for 16 byte alignment
};
@group(0) @binding(0)
var<uniform> chart_uniforms: ChartUniform;

struct VertexInput {
    @location(0) position: vec2<f32>,
};

struct InstanceInput {
    @location(1) position: vec2<f32>,
    @location(2) color: vec3<f32>,
    @location(3) size: f32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
};


@vertex
fn vs_main(
    model: VertexInput,
    instance: InstanceInput,
) -> VertexOutput {
    var out: VertexOutput;
    out.color = instance.color;
    let size_scale = sqrt(instance.size);
    let x = 2.0 * (model.position[0] * size_scale + instance.position[0]) / chart_uniforms.size[0] - 1.0;
    let y = 2.0 * (model.position[1] * size_scale + (chart_uniforms.size[1] - instance.position[1])) / chart_uniforms.size[1] - 1.0;
    out.clip_position = vec4<f32>(x, y, 0.0, 1.0);
    return out;
}

// Fragment shader

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
