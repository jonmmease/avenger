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
    @location(1) color: vec4<f32>,
    @location(2) top_left: vec2<f32>,
    @location(3) bottom_right: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) top_left: vec2<f32>,
    @location(2) bottom_right: vec2<f32>,
 }

 // Vertex shader
@vertex
fn vs_main(
  model: VertexInput
) -> VertexOutput {
    var out: VertexOutput;

    // Compute absolute position
    let position = model.position + chart_uniforms.origin;

    // Compute vertex coordinates
    let x = 2.0 * position[0] / chart_uniforms.size[0] - 1.0;
    let y = 2.0 * (chart_uniforms.size[1] - position[1]) / chart_uniforms.size[1] - 1.0;
    out.clip_position = vec4<f32>(x, y, 0.0, 1.0);

    out.color = model.color;
    out.top_left = (model.top_left + chart_uniforms.origin) * chart_uniforms.scale;
    out.bottom_right = (model.bottom_right + chart_uniforms.origin) * chart_uniforms.scale;
    return out;
}

// Fragment shader
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return lookup_color(in.color, in.clip_position, in.top_left, in.bottom_right);
}