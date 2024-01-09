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
    @location(1) normal: vec2<f32>,
    @location(2) kind: u32,
};

struct InstanceInput {
    @location(3) position: vec2<f32>,
    @location(4) fill_color: vec4<f32>,
    @location(5) stroke_color: vec4<f32>,
    @location(6) stroke_width: f32,
    @location(7) size: f32,
    @location(8) angle: f32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(1) center: vec2<f32>,
    @location(2) radius: f32,
    @location(3) fill_color: vec4<f32>,
    @location(4) stroke_color: vec4<f32>,
    @location(5) stroke_width: f32,
};


@vertex
fn vs_main(
    model: VertexInput,
    instance: InstanceInput,
) -> VertexOutput {
    var out: VertexOutput;

    // Pass through colors and stroke_width
    out.fill_color = instance.fill_color;
    out.stroke_color = instance.stroke_color;
    out.stroke_width = instance.stroke_width;

    // Compute normalized position of vertex
    let size_scale = sqrt(instance.size);
    let clip_x = 2.0 * (model.position[0] * size_scale + instance.position[0]) / chart_uniforms.size[0] - 1.0;
    let clip_y = 2.0 * (model.position[1] * size_scale + (chart_uniforms.size[1] - instance.position[1])) / chart_uniforms.size[1] - 1.0;
    out.clip_position = vec4<f32>(clip_x, clip_y, 0.0, 1.0);

    // Compute circle center in fragment shader coordinates
    out.center = vec2<f32>(
        instance.position[0] * chart_uniforms.scale,
        instance.position[1] * chart_uniforms.scale,
    );

    // Compute radius in fragment shader coordinates
    out.radius = size_scale * chart_uniforms.scale / 2.0;
    return out;
}


@fragment
fn fs_main(
    in: VertexOutput,
) -> @location(0) vec4<f32> {
    let buffer = 0.5 * chart_uniforms.scale;
    let dist = length(in.center - vec2<f32>(in.clip_position[0], in.clip_position[1]));

    if (in.stroke_width > 0.0) {
        let inner_radius = in.radius - in.stroke_width * chart_uniforms.scale / 2.0;
        let outer_radius = in.radius + in.stroke_width * chart_uniforms.scale / 2.0;
        if (dist > outer_radius + buffer * 2.0) {
            discard;
        } else {
            let alpha_factor = 1.0 - smoothstep(outer_radius - buffer, outer_radius + buffer, dist);
            let mix_factor = 1.0 - smoothstep(inner_radius - buffer, inner_radius + buffer, dist);
            var mixed_color: vec4<f32> = mix(in.stroke_color, in.fill_color, mix_factor);
            mixed_color[3] *= alpha_factor;
            return mixed_color;
        }
    } else {
        let alpha_factor = 1.0 - smoothstep(in.radius - buffer, in.radius + buffer, dist);
        var mixed_color: vec4<f32> = in.fill_color;
        mixed_color[3] *= alpha_factor;
        if (dist > in.radius + buffer) {
            discard;
        } else {
            return mixed_color;
        }
    }
}
