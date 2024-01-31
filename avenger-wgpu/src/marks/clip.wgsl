// Helper function to compute clipping.
// Assumes a chart_uniforms is available with origin, scale, group_size, and clip properties

fn should_clip(clip_position: vec4<f32>) -> bool {
    let scaled_top_left = chart_uniforms.origin * chart_uniforms.scale;
    let scaled_bottom_right = scaled_top_left + chart_uniforms.group_size * chart_uniforms.scale;
    return chart_uniforms.clip == 1.0 && (
        clip_position[0] < scaled_top_left[0]
            || clip_position[1] < scaled_top_left[1]
            || clip_position[0] > scaled_bottom_right[0]
            || clip_position[1] > scaled_bottom_right[1]
        );
}
