struct ChartUniform {
    size: vec2<f32>,
    scale: f32,
    _pad: f32, // for 16 byte alignment
};

@group(0) @binding(0)
var<uniform> chart_uniforms: ChartUniform;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
 }

 // Vertex shader
  @vertex
  fn vs_main(
      model: VertexInput
  ) -> VertexOutput {
     var out: VertexOutput;

     // Compute vertex coordinates
     let x = 2.0 * model.position[0] / chart_uniforms.size[0] - 1.0;
     let y = 2.0 * (chart_uniforms.size[1] - model.position[1]) / chart_uniforms.size[1] - 1.0;
     out.clip_position = vec4<f32>(x, y, 0.0, 1.0);

     out.color = model.color;
     return out;
  }

   // Fragment shader
   @fragment
   fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
      return in.color;
   }