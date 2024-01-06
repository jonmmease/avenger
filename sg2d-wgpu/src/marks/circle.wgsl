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
    @location(4) fill_color: vec3<f32>,
    @location(5) stroke_color: vec3<f32>,
    @location(6) stroke_width: f32,
    @location(7) size: f32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(1) center: vec2<f32>,
    @location(2) radius: f32,
    @location(3) color: vec3<f32>,
};


@vertex
fn vs_main(
    model: VertexInput,
    instance: InstanceInput,
) -> VertexOutput {
    var out: VertexOutput;

    // Pass through color
    out.color = instance.fill_color;

    // Compute normalized position of vertex
    let size_scale = sqrt(instance.size);
    let clip_x = 2.0 * (model.position[0] * size_scale + instance.position[0]) / chart_uniforms.size[0] - 1.0;
    let clip_y = 2.0 * (model.position[1] * size_scale + (chart_uniforms.size[1] - instance.position[1])) / chart_uniforms.size[1] - 1.0;
    out.clip_position = vec4<f32>(clip_x, clip_y, 0.0, 1.0);

    // Compute circle center in fragment shader coordinates
    out.center = vec2<f32>(
        instance.position[0],
        instance.position[1]
    );

    // Compute radius in fragment shader coordinates
    out.radius = size_scale / 2.0;
    return out;
}


@fragment
fn fs_main(
    in: VertexOutput,
) -> @location(0) vec4<f32> {
    let buffer = 0.5;
    let dist = length(in.center - vec2<f32>(in.clip_position[0], in.clip_position[1]));
    let alpha_factor = 1.0 - smoothstep(in.radius - buffer, in.radius + buffer, dist);
    if (dist > in.radius + buffer) {
        discard;
    } else {
        return vec4<f32>(in.color, alpha_factor);
    }
}
