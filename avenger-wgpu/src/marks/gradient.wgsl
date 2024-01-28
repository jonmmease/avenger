// Gradient color logic that provides the `lookup_color` function.
// This is intended to be concatenated to the end of shader files that support
// gradients

const GRADIENT_LINEAR = 0.0;
const GRADIENT_RADIAL = 1.0;

const COLORWAY_LENGTH = 250.0;
const GRADIENT_TEXTURE_WIDTH = 256.0;
const GRADIENT_TEXTURE_HEIGHT = 256.0;

@group(1) @binding(0)
var gradient_texture: texture_2d<f32>;
@group(1) @binding(1)
var gradient_sampler: sampler;

// Compute final color, potentially computing gradient
fn lookup_color(color: vec4<f32>, clip_position: vec4<f32>, top_left: vec2<f32>, bottom_right: vec2<f32>) -> vec4<f32> {
    if (color[0] < 0.0) {
        // If the first color coordinate is negative, this indicates that we need to compute a gradient.
        // The negative of this value is the y-coordinate into the gradient texture where the gradient control
        // points and gradient colorway are stored.
        let tex_coord_y = -color[0];

        // Extract gradient type from fist pixel using nearest sampler (so that not interpolation is performed)
        let control0 = textureSample(gradient_texture, gradient_sampler, vec2<f32>(0.0, tex_coord_y));
        let gradient_type = control0[0];

        // Extract x/y control points from second pixel
        let control1 = textureSample(gradient_texture, gradient_sampler, vec2<f32>(1.0 / GRADIENT_TEXTURE_WIDTH, tex_coord_y));
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

            return textureSample(gradient_texture, gradient_sampler, vec2<f32>(tex_coord_x, tex_coord_y));
        } else {
           // Extract additional radius gradient control points from third pixel
            let control2 = textureSample(gradient_texture, gradient_sampler, vec2<f32>(2.0 / GRADIENT_TEXTURE_WIDTH, tex_coord_y));
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
            return textureSample(gradient_texture, gradient_sampler, vec2<f32>(tex_coord_x, tex_coord_y));
        }
    } else {
        return color;
    }
}

fn compute_tex_x_coord(grad_dist: f32) -> f32 {
    let col_offset = GRADIENT_TEXTURE_WIDTH - COLORWAY_LENGTH;
    return clamp(grad_dist, 0.0, 1.0) * COLORWAY_LENGTH / GRADIENT_TEXTURE_WIDTH + col_offset / GRADIENT_TEXTURE_WIDTH;
}
