// Tile texture-array variant of multi.wgsl: identical vertex transform
// and vertex buffer layout, but the fragment shader samples a
// texture_2d_array. Vertex `color` is reinterpreted: color[0] carries the
// array layer index (identical across each triangle; decoded with
// round()), color[1..2] the tile-space uv, and color[3] an opacity
// multiplier (1.0 today; the fade-in hook).

struct ChartUniform {
    size: vec2<f32>,
    scale: f32,
    _pad: f32,
};

@group(0) @binding(0)
var<uniform> chart_uniforms: ChartUniform;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) top_left: vec2<f32>,
    @location(3) bottom_right: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(model: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    let x = 2.0 * model.position[0] / chart_uniforms.size[0] - 1.0;
    let y = 2.0 * (chart_uniforms.size[1] - model.position[1]) / chart_uniforms.size[1] - 1.0;
    out.clip_position = vec4<f32>(x, y, 0.0, 1.0);
    out.color = model.color;
    return out;
}

@group(2) @binding(0)
var tile_texture: texture_2d_array<f32>;
@group(2) @binding(1)
var tile_sampler: sampler;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let layer = i32(round(in.color[0]));
    let uv = vec2<f32>(in.color[1], in.color[2]);
    // Gradient arguments mirror multi.wgsl's image branch (they only
    // matter for mip selection, and tile arrays have a single mip).
    let texXy = vec2<f32>(in.clip_position[0], in.clip_position[1]);
    let dx = dpdx(texXy);
    let dy = dpdx(texXy);
    let sampled = textureSampleGrad(tile_texture, tile_sampler, uv, layer, dx, dy);
    return sampled * vec4<f32>(1.0, 1.0, 1.0, in.color[3]);
}
