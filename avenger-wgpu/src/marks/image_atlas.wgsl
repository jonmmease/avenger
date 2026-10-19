// Images in atlas pages. The texture coordinates are the color's second and third components.
@group(2) @binding(0)
var image_texture: texture_2d<f32>;
@group(2) @binding(1)
var image_sampler: sampler;

fn sample_image(color: vec4<f32>, dx: vec2<f32>, dy: vec2<f32>) -> vec4<f32> {
    let tex_coords = vec2<f32>(color[1], color[2]);
    return textureSampleGrad(image_texture, image_sampler, tex_coords, dx, dy);
}
