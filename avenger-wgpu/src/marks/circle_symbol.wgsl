struct SymbolUniform {
    size: vec2<f32>,
    origin: vec2<f32>,
    scale: f32,
    _pad: vec2<f32>,
};

@group(0) @binding(0)
var<uniform> chart_uniforms: SymbolUniform;

struct MarkUniform {
    adjustment_scale: vec2<f32>,
    adjustment_offset: vec2<f32>,
}

@group(0) @binding(1)
var<uniform> mark_uniforms: MarkUniform;

struct VertexInput {
    @location(0) position: vec2<f32>,
};

struct InstanceInput {
    @location(4) position: vec2<f32>,
    @location(5) fill_color: vec4<f32>,
    @location(6) stroke_color: vec4<f32>,
    @location(7) radius: f32,
    @location(8) stroke_width: f32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) local_position: vec2<f32>,
    @location(1) fill_color: vec4<f32>,
    @location(2) stroke_color: vec4<f32>,
    @location(3) radius: f32,
    @location(4) stroke_width: f32,
};

@vertex
fn vs_main(
    model: VertexInput,
    instance: InstanceInput,
) -> VertexOutput {
    var out: VertexOutput;

    let stroke_width = max(instance.stroke_width, 0.0);
    let outer_radius = max(instance.radius + stroke_width * 0.5 + 1.0, 1.0);
    let local_position = model.position * outer_radius;

    let position = (
        instance.position * mark_uniforms.adjustment_scale + mark_uniforms.adjustment_offset
    ) + chart_uniforms.origin;

    let screen_position = vec2<f32>(
        position.x + local_position.x,
        chart_uniforms.size.y - position.y + local_position.y,
    );
    let normalized_position = 2.0 * screen_position / chart_uniforms.size - 1.0;

    out.clip_position = vec4<f32>(normalized_position, 0.0, 1.0);
    out.local_position = local_position;
    out.fill_color = instance.fill_color;
    out.stroke_color = instance.stroke_color;
    out.radius = max(instance.radius, 0.0);
    out.stroke_width = stroke_width;

    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let distance = length(in.local_position);
    let stroke_width = select(0.0, in.stroke_width, in.stroke_width > 0.0 && in.stroke_color.a > 0.0);
    let half_stroke = stroke_width * 0.5;
    let has_stroke = stroke_width > 0.0;
    // Cover about one physical pixel, including at high device scales.
    let antialias = max(fwidth(distance) * 0.5, 0.5 / chart_uniforms.scale);

    if (has_stroke) {
        let inner_radius = max(in.radius - half_stroke, 0.0);
        let outer_radius = in.radius + half_stroke;
        let outer_alpha = 1.0 - smoothstep(outer_radius - antialias, outer_radius + antialias, distance);
        let stroke_mix = smoothstep(inner_radius - antialias, inner_radius + antialias, distance);
        // Interpolate premultiplied colors so invisible fill RGB cannot tint
        // the antialiased inner edge of an open circle.
        let mixed_alpha = mix(in.fill_color.a, in.stroke_color.a, stroke_mix);
        let premultiplied = mix(
            in.fill_color.rgb * in.fill_color.a,
            in.stroke_color.rgb * in.stroke_color.a,
            stroke_mix,
        );
        let rgb = premultiplied / max(mixed_alpha, 0.000001);
        let alpha = outer_alpha * mixed_alpha;

        if (alpha <= 0.0) {
            discard;
        }
        return vec4<f32>(rgb, alpha);
    }

    let alpha_mask = 1.0 - smoothstep(in.radius - antialias, in.radius + antialias, distance);
    let alpha = in.fill_color.a * alpha_mask;
    if (alpha <= 0.0) {
        discard;
    }

    return vec4<f32>(in.fill_color.rgb, alpha);
}
