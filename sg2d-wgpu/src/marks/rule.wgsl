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
};

struct InstanceInput {
    @location(1) x0: f32,
    @location(2) y0: f32,
    @location(3) x1: f32,
    @location(4) y1: f32,
    @location(5) stroke: vec3<f32>,
    @location(6) stroke_width: f32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
};

const PI = 3.14159265359;

@vertex
fn vs_main(
    model: VertexInput,
    instance: InstanceInput,
) -> VertexOutput {
    var out: VertexOutput;
    out.color = instance.stroke;

    let p0 = vec2(instance.x0, instance.y0);
    let p1 = vec2(instance.x1, instance.y1);
    let mid = (p0 + p1) / 2.0;
    let len = distance(p0, p1);
    let width = instance.stroke_width;

    let normed = normalize(p1 - p0);
    let angle = (PI / 2.0) + atan2(normed[1], normed[0]);
    let rot = mat2x2(cos(angle), -sin(angle), sin(angle), cos(angle));

    let rot_pos = rot * vec2(model.position[0] * width, model.position[1] * len);
    let x = 2.0 * (rot_pos[0] + mid[0]) / chart_uniforms.size[0] - 1.0;
    let y = 2.0 * (rot_pos[1] + (chart_uniforms.size[1] - mid[1])) / chart_uniforms.size[1] - 1.0;

    out.clip_position = vec4<f32>(x, y, 0.0, 1.0);
    return out;
}

// Fragment shader

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
