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
    @location(1) tex_coords: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
}

@vertex
fn vs_main(
    model: VertexInput,
) -> VertexOutput {
    var out: VertexOutput;
    out.tex_coords = model.tex_coords;

    // Compute absolute position
    let position = model.position + chart_uniforms.origin;

    let normalized_pos = vec2<f32>(
        2.0 * position[0] / chart_uniforms.size[0] - 1.0,
        2.0 * (chart_uniforms.size[1] - position[1]) / chart_uniforms.size[1] - 1.0,
    );
    out.clip_position = vec4<f32>(normalized_pos, 0.0, 1.0);
    return out;
}

// Fragment shader
@group(1) @binding(0)
var texture_atlas: texture_2d<f32>;
@group(1) @binding(1)
var texture_sampler: sampler;
@group(1) @binding(2)
var nearest_sampler: sampler;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(texture_atlas, texture_sampler, in.tex_coords);
}
