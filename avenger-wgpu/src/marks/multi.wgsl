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
    @location(4) pattern: u32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) top_left: vec2<f32>,
    @location(2) bottom_right: vec2<f32>,
    // The position in scene pixels, which patterns are measured in.
    @location(3) scene_position: vec2<f32>,
    // The offset of the fill's pattern record, or 0 for no pattern.
    @location(4) @interpolate(flat) pattern: u32,
 }

 // Vertex shader
@vertex
fn vs_main(
  model: VertexInput
) -> VertexOutput {
    var out: VertexOutput;

    // Compute absolute position
    let position = model.position;

    // Compute vertex coordinates
    let x = 2.0 * position[0] / chart_uniforms.size[0] - 1.0;
    let y = 2.0 * (chart_uniforms.size[1] - position[1]) / chart_uniforms.size[1] - 1.0;
    out.clip_position = vec4<f32>(x, y, 0.0, 1.0);

    out.color = model.color;
    out.top_left = model.top_left * chart_uniforms.scale;
    out.bottom_right = model.bottom_right * chart_uniforms.scale;
    out.scene_position = position;
    out.pattern = model.pattern;
    return out;
}

// Whether the target texture encodes sRGB itself. Colors arrive sRGB-encoded, so the shader then
// writes them as linear values, which the target encodes back.
override srgb_target: bool = false;

fn target_color(color: vec4<f32>) -> vec4<f32> {
    if (!srgb_target) {
        return color;
    }
    let lower = color.rgb / 12.92;
    let higher = pow((color.rgb + 0.055) / 1.055, vec3<f32>(2.4));
    return vec4<f32>(select(higher, lower, color.rgb <= vec3<f32>(0.04045)), color.a);
}

// Fragment shader
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let fill = target_color(lookup_color(in.color, in.clip_position, in.top_left, in.bottom_right));
    if (in.pattern == 0u) {
        return fill;
    }
    return paint_pattern(fill, in);
}

// Pattern records, which `marks/pattern.rs` encodes.
@group(0) @binding(1)
var pattern_texture: texture_2d<f32>;

const PATTERN_TEXTURE_WIDTH: u32 = 2048u;
const STRIPE_LAYER: u32 = 0u;
const CIRCLE_SYMBOL: u32 = 0u;
const ADD: u32 = 0u;
const SUBTRACT: u32 = 1u;
// Farther than any point of a scene, for a pattern that covers nothing.
const EMPTY: f32 = 1e20;
// The most lattice steps that a pixel searches along each direction for symbols that reach it.
// Validation keeps symbols within four shortest steps, so only a lattice finer than a pixel
// reaches this.
const MAX_LATTICE_SEARCH: i32 = 12;

fn pattern_texel(index: u32) -> vec4<f32> {
    return textureLoad(
        pattern_texture,
        vec2<u32>(index % PATTERN_TEXTURE_WIDTH, index / PATTERN_TEXTURE_WIDTH),
        0,
    );
}

// Paints the pattern's ink over the fill, in proportion to how much of the pixel the pattern
// covers. Each layer's distance to its edge, in scene pixels and negative inside, gives the
// layer's coverage of the pixel, and the layers' coverages combine as the union, difference, or
// symmetric difference of their regions.
fn paint_pattern(fill: vec4<f32>, in: VertexOutput) -> vec4<f32> {
    let header = pattern_texel(in.pattern);
    var origin = header.zw;
    if (header.y > 0.5) {
        // A mark-anchored pattern starts at the item's bounds.
        origin = in.top_left / chart_uniforms.scale;
    }
    let q = in.scene_position - origin;
    let pixel = 1.0 / chart_uniforms.scale;

    var coverage = 0.0;
    var index = in.pattern + 2u;
    for (var layer = 0u; layer < u32(header.x); layer++) {
        let layer_header = pattern_texel(index);
        var distance: f32;
        if (u32(layer_header.x) == STRIPE_LAYER) {
            distance = stripe_distance(index + 1u, q);
        } else {
            distance = symbol_distance(index + 1u, q, pixel);
        }
        let layer_coverage = clamp(0.5 - distance * chart_uniforms.scale, 0.0, 1.0);
        let operation = u32(layer_header.y);
        if (operation == ADD) {
            coverage = coverage + layer_coverage - coverage * layer_coverage;
        } else if (operation == SUBTRACT) {
            coverage = coverage * (1.0 - layer_coverage);
        } else {
            coverage = coverage + layer_coverage - 2.0 * coverage * layer_coverage;
        }
        index += 1u + u32(layer_header.z);
    }

    let ink = target_color(pattern_texel(in.pattern + 1u));
    let alpha = ink.a * coverage;
    if (alpha <= 0.0) {
        return fill;
    }
    // The ink over the fill, as the two would blend if drawn one after the other.
    let out_alpha = alpha + fill.a * (1.0 - alpha);
    let rgb = (ink.rgb * alpha + fill.rgb * fill.a * (1.0 - alpha)) / out_alpha;
    return vec4<f32>(rgb, out_alpha);
}

// The distance to the edge of a layer of stripes, which can be dashed.
fn stripe_distance(params: u32, q: vec2<f32>) -> f32 {
    // [normal x, normal y, spacing, half width]
    let line = pattern_texel(params);
    // [phase, dash period, dash half length, dash center], with period 0 for solid stripes
    let dash = pattern_texel(params + 1u);
    let across = dot(q, line.xy) - dash.x;
    var distance = abs(across - line.z * round(across / line.z)) - line.w;
    if (dash.y > 0.0) {
        let along = dot(q, vec2<f32>(line.y, -line.x)) - dash.w;
        distance = max(distance, abs(along - dash.y * round(along / dash.y)) - dash.z);
    }
    return distance;
}

// The distance to the edge of the nearest symbols of a lattice.
fn symbol_distance(params: u32, q: vec2<f32>, pixel: f32) -> f32 {
    // The reduced lattice steps [u, v], and the rows of their inverse.
    let steps = pattern_texel(params);
    let inverse = pattern_texel(params + 1u);
    // [phase x, phase y, reach, half stroke width], with 0 for filled symbols
    let placement = pattern_texel(params + 2u);
    let p = q - placement.xy;
    let a = dot(inverse.xy, p);
    let b = dot(inverse.zw, p);

    // The lattice points whose symbols, with their anti-aliased edges, can reach p.
    let reach = placement.z + pixel;
    let a_reach = reach * length(inverse.xy);
    let b_reach = reach * length(inverse.zw);
    let a_center = i32(round(a));
    let b_center = i32(round(b));
    let i_start = max(i32(ceil(a - a_reach)), a_center - MAX_LATTICE_SEARCH);
    let i_end = min(i32(floor(a + a_reach)), a_center + MAX_LATTICE_SEARCH);
    let j_start = max(i32(ceil(b - b_reach)), b_center - MAX_LATTICE_SEARCH);
    let j_end = min(i32(floor(b + b_reach)), b_center + MAX_LATTICE_SEARCH);

    var distance = EMPTY;
    for (var i = i_start; i <= i_end; i++) {
        for (var j = j_start; j <= j_end; j++) {
            let local = p - f32(i) * steps.xy - f32(j) * steps.zw;
            if (dot(local, local) > reach * reach) {
                continue;
            }
            var symbol = shape_distance(params + 3u, local);
            if (placement.w > 0.0) {
                symbol = abs(symbol) - placement.w;
            }
            distance = min(distance, symbol);
        }
    }
    return distance;
}

// The distance to the edge of a symbol's shape: a circle, or the union of convex parts, each the
// intersection of its edges' half-planes. Half-plane distances keep the corners of an open
// symbol's stroke mitered.
fn shape_distance(shape_index: u32, q: vec2<f32>) -> f32 {
    // [circle, radius, 0, 0] or [polygons, part count, 0, 0]
    let shape = pattern_texel(shape_index);
    if (u32(shape.x) == CIRCLE_SYMBOL) {
        return length(q) - shape.y;
    }
    var distance = EMPTY;
    var index = shape_index + 1u;
    for (var part = 0u; part < u32(shape.y); part++) {
        let edge_count = u32(pattern_texel(index).x);
        var part_distance = -EMPTY;
        for (var edge = 0u; edge < edge_count; edge++) {
            // [normal x, normal y, offset, 0]
            let line = pattern_texel(index + 1u + edge);
            part_distance = max(part_distance, dot(line.xy, q) - line.z);
        }
        distance = min(distance, part_distance);
        index += 1u + edge_count;
    }
    return distance;
}

// Gradient texture binding
@group(1) @binding(0)
var gradient_texture: texture_2d<f32>;
@group(1) @binding(1)
var gradient_sampler: sampler;

// Image texture binding
@group(2) @binding(0)
var image_texture: texture_2d<f32>;
@group(2) @binding(1)
var image_sampler: sampler;

// Text texture bindings
@group(3) @binding(0)
var text_texture: texture_2d<f32>;
@group(3) @binding(1)
var text_sampler_linear: sampler;
@group(3) @binding(2)
var text_sampler_nearest: sampler;

const GRADIENT_TEXTURE_CODE = -1.0;
const IMAGE_TEXTURE_CODE = -2.0;
const TEXT_TEXTURE_CODE = -3.0;
const TEXT_TEXTURE_NEAREST_CODE = -4.0;

const GRADIENT_LINEAR = 0.0;
const GRADIENT_RADIAL = 1.0;
const COLORWAY_LENGTH = 250.0;
const GRADIENT_TEXTURE_WIDTH = 256.0;
const GRADIENT_TEXTURE_HEIGHT = 256.0;

// Compute final color, potentially computing gradient
fn lookup_color(color: vec4<f32>, clip_position: vec4<f32>, top_left: vec2<f32>, bottom_right: vec2<f32>) -> vec4<f32> {
    // Use textureSampleGrad instead of textureSample to avoid Uniform Control Flow error in WebGPU
    // https://github.com/gpuweb/gpuweb/discussions/2899
    let texXy = vec2<f32>(clip_position[0], clip_position[1]);
    let dx = dpdx(texXy);
    let dy = dpdx(texXy);

    if (color[0] == GRADIENT_TEXTURE_CODE) {
        // If the first color coordinate is a negative value, this indicates that we are computing a color from a texture
        // For gradient texture, the second color component stores the gradient texture y-coordinate
        let tex_coord_y = color[1];

        // Extract gradient type from fist pixel
        let control0 = textureSampleGrad(gradient_texture, gradient_sampler, vec2<f32>(0.0, tex_coord_y), dx, dy);
        let gradient_type = control0[0];

        // Extract x/y control points from second pixel
        let control1 = textureSampleGrad(gradient_texture, gradient_sampler, vec2<f32>(1.0 / GRADIENT_TEXTURE_WIDTH, tex_coord_y), dx, dy);
        let x0 = control1[0];
        let y0 = control1[1];
        let x1 = control1[2];
        let y1 = control1[3];
        let p0 = vec2<f32>(x0, y0);
        let p1 = vec2<f32>(x1, y1);

        let frag_xy = vec2<f32>(clip_position[0], clip_position[1]);
        let width_height = vec2<f32>(bottom_right[0] - top_left[0], bottom_right[1] - top_left[1]);

        if (gradient_type == GRADIENT_LINEAR) {
           // Convert fragment coordinate into coordinate normalized to rect bounding box
            let norm_xy = (frag_xy - top_left) / width_height;

            let control_dist = distance(p0, p1);
            let projected_dist = dot(norm_xy - p0, p1 - p0) / control_dist;

            let tex_coord_x = compute_tex_x_coord(projected_dist / control_dist);
            let tex_coords = vec2<f32>(tex_coord_x, tex_coord_y);
            return textureSampleGrad(gradient_texture, gradient_sampler, tex_coords, dx, dy);
        } else {
           // Extract additional radius gradient control points from third pixel
            let control2 = textureSampleGrad(gradient_texture, gradient_sampler, vec2<f32>(2.0 / GRADIENT_TEXTURE_WIDTH, tex_coord_y), dx, dy);
            let r0 = control2[0];
            let r1 = control2[1];

            // Expand top_left and bottom_right so they form a square
            var square_top_left: vec2<f32>;
            var square_bottom_right: vec2<f32>;
            var side: f32;
            if (width_height[0] > width_height[1]) {
                // wider than tall, push out y coordinates until square
                let delta = (width_height[0] - width_height[1]) / 2.0;
                square_top_left = vec2<f32>(top_left[0], top_left[1] - delta);
                square_bottom_right = vec2<f32>(bottom_right[0], bottom_right[1] + delta);
                side = width_height[0];
            } else if (width_height[0] < width_height[1]) {
                // taller than wide, push out x coordinates until square
                let delta = (width_height[1] - width_height[0]) / 2.0;
                square_top_left = vec2<f32>(top_left[0] - delta, top_left[1]);
                square_bottom_right = vec2<f32>(bottom_right[0] + delta, bottom_right[1]);
                side = width_height[1];
            } else {
                // already square
                square_top_left = top_left;
                square_bottom_right = bottom_right;
                side = width_height[0];
            }

            // Normalize the fragment coordinates to square
            let norm_xy = (frag_xy - square_top_left) / side;
            let r_delta = r1 - r0;
            var frag_radius: f32;
            if (p0[0] == p1[0] && p0[1] == p1[1]) {
                // Concentric circles, compute radius to p0
                frag_radius = distance(norm_xy, p0);
            } else {
                // Offset circles,
                // In this case the radius we're computing is not to p0, but to a point between
                // p0 and p1.
                //
                // Define the following variables:
                //  t: Free variable such that as t scales from 0 to 1, the radius center point
                //     scales from p0 to p1 while the radius scales from 0 to r.
                //  x: Component of norm_xy along line from p0 to p1
                //  y: Component of norm_xy perpendicular to the line from p0 to p1
                //  d: Distance from p0 to p1,
                //
                // The equation we need to solve is:
                //      r1 * t = sqrt((x - d*t) & 2 + y^2).
                //
                // The solution below was obtained using sympy
                //      >>> from sympy.solvers import solve
                //      >>> from sympy import symbols, sqrt
                //      >>> r1, t, d, x, y = symbols("r1,t,d,x,y")
                //      >>> solutions = solve(r1 * t - sqrt((x - d * t) ** 2 + y**2), t)
                //
                // Take the position solution, which corresponds to positive t values
                //      >>> print(solutions[1])
                //      (-d*x + sqrt(-d**2*y**2 + r1**2*x**2 + r1**2*y**2))/(-d**2 + r1**2)
                //
                let centers = p1 - p0;
                let d = length(centers);
                let relative_xy = norm_xy - p0;
                let x = dot(relative_xy, centers) / d;
                let y = length(relative_xy - normalize(centers) * x);
                let t = (
                    -d * x + sqrt(-pow(d,2.0)*pow(y,2.0) + pow(r1,2.0)*pow(x,2.0) + pow(r1,2.0)*pow(y,2.0))
                ) / (
                    -pow(d,2.0) + pow(r1,2.0)
                );
                frag_radius = r1 * t;
            }

            let grad_dist = (frag_radius - r0) / r_delta;
            let tex_coord_x = compute_tex_x_coord(grad_dist);
            let tex_coords = vec2<f32>(tex_coord_x, tex_coord_y);
            return textureSampleGrad(gradient_texture, gradient_sampler, tex_coords, dx, dy);
        }
    } else if (color[0] == IMAGE_TEXTURE_CODE) {
        // Image texture coordinates are stored in the second and third color components
        let tex_coords = vec2<f32>(color[1], color[2]);
        return textureSampleGrad(image_texture, image_sampler, tex_coords, dx, dy);
    } else if (color[0] == TEXT_TEXTURE_CODE) {
        // Text texture coordinates are stored in the second and third color components (Linear filtering)
        let tex_coords = vec2<f32>(color[1], color[2]);
        return textureSampleGrad(text_texture, text_sampler_linear, tex_coords, dx, dy);
    } else if (color[0] == TEXT_TEXTURE_NEAREST_CODE) {
        // Text texture coordinates for nearest filtering
        let tex_coords = vec2<f32>(color[1], color[2]);
        return textureSampleGrad(text_texture, text_sampler_nearest, tex_coords, dx, dy);
    } else {
        return color;
    }
}

fn compute_tex_x_coord(grad_dist: f32) -> f32 {
    let col_offset = GRADIENT_TEXTURE_WIDTH - COLORWAY_LENGTH;
    return clamp(grad_dist, 0.0, 1.0) * COLORWAY_LENGTH / GRADIENT_TEXTURE_WIDTH + col_offset / GRADIENT_TEXTURE_WIDTH;
}
