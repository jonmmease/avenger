// Vertex shader

struct ChartUniform {
    size: vec2<f32>,
    filler: vec2<f32>,
};
@group(0) @binding(0)
var<uniform> chart_uniforms: ChartUniform;

struct VertexInput {
    @location(0) position: vec2<f32>,
};

struct InstanceInput {
    @location(1) position: vec2<f32>,
    @location(2) color: vec3<f32>,
    @location(3) width: f32,
    @location(4) height: f32,
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
    let x = 2.0 * (model.position[0] * instance.width + instance.position[0]) / chart_uniforms.size[0] - 1.0;
    let y = 2.0 * (model.position[1] * instance.height + (chart_uniforms.size[1] - instance.position[1] - instance.height)) / chart_uniforms.size[1] - 1.0;
    out.clip_position = vec4<f32>(x, y, 0.0, 1.0);
    return out;
}

// Fragment shader

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
