use avenger_color::{relative_luminance_srgb, ColorOrGradient, Gradient};
use avenger_common::types::{PathTransform, SymbolShape};
use lyon_extra::euclid::Vector2D;
use lyon_path::{
    geom::{
        euclid::{Point2D, UnknownUnit},
        point, Angle,
    },
    Event, Path,
};
use lyon_tessellation::{
    geometry_builder::{simple_builder, VertexBuffers},
    LineCap, LineJoin, StrokeOptions, StrokeTessellator,
};

use crate::marks::pattern::{
    PatternAnchor, PatternFill, PatternInk, PatternLayer, PatternLayerOperation, StripeDash,
    StripePatternLayer, SymbolPaint, SymbolPatternLayer,
};

const AUTO_CONTRAST_DARK_INK: [f32; 3] = [0.0, 0.0, 0.0];
const AUTO_CONTRAST_LIGHT_INK: [f32; 3] = [1.0, 1.0, 1.0];
const AUTO_CONTRAST_LUMINANCE_THRESHOLD: f32 = 0.5;
const SYMBOL_LATTICE_DETERMINANT_EPSILON: f32 = 1e-4;

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct PatternRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl PatternRect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
        .normalized()
    }

    pub fn normalized(self) -> Self {
        let x0 = self.x.min(self.x + self.width);
        let x1 = self.x.max(self.x + self.width);
        let y0 = self.y.min(self.y + self.height);
        let y1 = self.y.max(self.y + self.height);

        Self {
            x: x0,
            y: y0,
            width: x1 - x0,
            height: y1 - y0,
        }
    }

    pub fn min_x(self) -> f32 {
        self.x
    }

    pub fn max_x(self) -> f32 {
        self.x + self.width
    }

    pub fn min_y(self) -> f32 {
        self.y
    }

    pub fn max_y(self) -> f32 {
        self.y + self.height
    }

    pub fn is_empty(self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }

    pub fn expanded(self, padding: f32) -> Self {
        Self::new(
            self.x - padding,
            self.y - padding,
            self.width + padding * 2.0,
            self.height + padding * 2.0,
        )
    }

    fn corners(self) -> [[f32; 2]; 4] {
        [
            [self.min_x(), self.min_y()],
            [self.max_x(), self.min_y()],
            [self.max_x(), self.max_y()],
            [self.min_x(), self.max_y()],
        ]
    }
}

#[derive(Debug, Clone)]
pub struct PatternRenderContext<'a> {
    pub chart_bounds: PatternRect,
    pub plot_bounds: Option<PatternRect>,
    pub host_bounds: PatternRect,
    pub host_fill: &'a ColorOrGradient,
    pub gradients: &'a [Gradient],
}

#[derive(Debug, Clone)]
pub struct PatternCoverageLayer {
    pub operation: PatternLayerOperation,
    /// Coverage for this source pattern layer.
    pub coverage_path: Path,
}

#[derive(Debug, Clone)]
pub struct LayeredPatternGeometry {
    /// Straight-alpha RGBA pattern ink. Opacity has already been applied.
    pub ink: [f32; 4],
    /// One coverage path per non-empty source layer.
    pub layers: Vec<PatternCoverageLayer>,
    /// Source coverage bounding the final paint through the composed layer mask.
    /// This path does not encode subtraction or XOR and must not be filled directly.
    pub merged_coverage_path: Path,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PatternGeometryError {
    InvalidPattern,
    MissingPlotReferenceFrame,
}

/// Build pattern layers in composition order. Apply their operations to an empty
/// coverage mask, intersect it with the host fill, and paint the ink once.
pub fn build_layered_pattern_geometry(
    pattern: &PatternFill,
    context: &PatternRenderContext<'_>,
) -> Result<Option<LayeredPatternGeometry>, PatternGeometryError> {
    if pattern.layers.is_empty() || context.host_bounds.is_empty() {
        return Ok(None);
    }

    pattern
        .validate()
        .map_err(|_| PatternGeometryError::InvalidPattern)?;

    let origin = resolve_origin(pattern.anchor.clone(), context)?;
    let mut merged_builder = Path::builder();
    let mut layers = Vec::with_capacity(pattern.layers.len());

    for layer in &pattern.layers {
        let mut layer_builder = Path::builder();
        match layer {
            PatternLayer::Stripe(stripe) => {
                append_stripe_layer(&mut layer_builder, stripe, origin, context.host_bounds);
            }
            PatternLayer::Symbol(symbol) => {
                append_symbol_layer(&mut layer_builder, symbol, origin, context.host_bounds)?;
            }
        }

        let coverage_path = layer_builder.build();
        if path_is_empty(&coverage_path) {
            continue;
        }

        append_path(&mut merged_builder, &coverage_path);
        layers.push(PatternCoverageLayer {
            operation: layer.operation(),
            coverage_path,
        });
    }

    if layers.is_empty() {
        return Ok(None);
    }

    Ok(Some(LayeredPatternGeometry {
        ink: resolve_pattern_ink(&pattern.ink, context.host_fill, context.gradients),
        layers,
        merged_coverage_path: merged_builder.build(),
    }))
}

pub fn resolve_pattern_ink(
    ink: &PatternInk,
    host_fill: &ColorOrGradient,
    gradients: &[Gradient],
) -> [f32; 4] {
    match ink {
        PatternInk::AutoContrast { opacity } => {
            let host = representative_host_color(host_fill, gradients);
            let luminance = relative_luminance_srgb(host[0], host[1], host[2]);
            let neutral = if luminance >= AUTO_CONTRAST_LUMINANCE_THRESHOLD {
                AUTO_CONTRAST_DARK_INK
            } else {
                AUTO_CONTRAST_LIGHT_INK
            };

            [neutral[0], neutral[1], neutral[2], *opacity]
        }
        PatternInk::Solid { color, opacity } => {
            let mut color = *color;
            color[3] *= *opacity;
            color
        }
    }
}

fn representative_host_color(host_fill: &ColorOrGradient, gradients: &[Gradient]) -> [f32; 4] {
    match host_fill {
        ColorOrGradient::Color(color) => *color,
        ColorOrGradient::GradientIndex(index) => gradients
            .get(*index as usize)
            .and_then(average_gradient_color)
            .unwrap_or([1.0, 1.0, 1.0, 1.0]),
    }
}

fn average_gradient_color(gradient: &Gradient) -> Option<[f32; 4]> {
    let stops = gradient.stops();
    if stops.is_empty() {
        return None;
    }

    let mut color = [0.0; 4];
    for stop in stops {
        for (i, component) in stop.color.iter().enumerate() {
            color[i] += component;
        }
    }

    let denom = stops.len() as f32;
    for component in &mut color {
        *component /= denom;
    }

    Some(color)
}

fn resolve_origin(
    anchor: PatternAnchor,
    context: &PatternRenderContext<'_>,
) -> Result<[f32; 2], PatternGeometryError> {
    match anchor {
        PatternAnchor::Mark => Ok([context.host_bounds.min_x(), context.host_bounds.min_y()]),
        PatternAnchor::Plot => context
            .plot_bounds
            .map(|bounds| [bounds.min_x(), bounds.min_y()])
            .ok_or(PatternGeometryError::MissingPlotReferenceFrame),
        PatternAnchor::Chart => Ok([context.chart_bounds.min_x(), context.chart_bounds.min_y()]),
    }
}

fn append_stripe_layer(
    builder: &mut lyon_path::path::Builder,
    layer: &StripePatternLayer,
    origin: [f32; 2],
    bounds: PatternRect,
) {
    let theta = layer.angle.to_radians();
    let d = [theta.cos(), theta.sin()];
    let n = [-theta.sin(), theta.cos()];
    let half_width = layer.stroke_width / 2.0;

    let corners = bounds.corners();
    let (min_n, max_n) = projection_range(&corners, origin, n);
    let (min_d, max_d) = projection_range(&corners, origin, d);
    let t_start = min_d - half_width;
    let t_end = max_d + half_width;

    let k_start = ((min_n - half_width - layer.phase) / layer.spacing).ceil() as i32;
    let k_end = ((max_n + half_width - layer.phase) / layer.spacing).floor() as i32;

    for k in k_start..=k_end {
        let stripe_offset = layer.phase + k as f32 * layer.spacing;

        if let Some(dash) = &layer.dash {
            append_dashed_stripe(
                builder,
                origin,
                d,
                n,
                stripe_offset,
                half_width,
                t_start,
                t_end,
                dash,
            );
        } else {
            append_stripe_quad(
                builder,
                origin,
                d,
                n,
                stripe_offset,
                half_width,
                t_start,
                t_end,
            );
        }
    }
}

fn append_symbol_layer(
    builder: &mut lyon_path::path::Builder,
    layer: &SymbolPatternLayer,
    origin: [f32; 2],
    bounds: PatternRect,
) -> Result<(), PatternGeometryError> {
    let shape = SymbolShape::from_vega_str(&layer.symbol.shape)
        .map_err(|_| PatternGeometryError::InvalidPattern)?;
    let symbol_path = shape.as_path();
    let scale = layer.symbol.size.sqrt();
    let rotation = Angle::degrees(layer.symbol.rotation);

    let u_dir = direction(layer.lattice.u_angle);
    let v_dir = direction(layer.lattice.v_angle);
    let u_vec = [
        u_dir[0] * layer.lattice.u_spacing,
        u_dir[1] * layer.lattice.u_spacing,
    ];
    let v_vec = [
        v_dir[0] * layer.lattice.v_spacing,
        v_dir[1] * layer.lattice.v_spacing,
    ];
    let det = cross(u_vec, v_vec);
    if !det.is_finite() || det.abs() < SYMBOL_LATTICE_DETERMINANT_EPSILON {
        return Err(PatternGeometryError::InvalidPattern);
    }

    let stroke_padding = match layer.paint {
        SymbolPaint::Filled => 0.0,
        SymbolPaint::Open { stroke_width } => stroke_width / 2.0,
    };
    let expanded_bounds = bounds.expanded(scale + stroke_padding + 1.0);
    let offset = [
        u_dir[0] * layer.lattice.u_phase + v_dir[0] * layer.lattice.v_phase,
        u_dir[1] * layer.lattice.u_phase + v_dir[1] * layer.lattice.v_phase,
    ];
    let (min_i, max_i, min_j, max_j) =
        lattice_index_ranges(expanded_bounds, origin, offset, u_vec, v_vec, det);

    for i in min_i..=max_i {
        for j in min_j..=max_j {
            let position = [
                origin[0] + offset[0] + i as f32 * u_vec[0] + j as f32 * v_vec[0],
                origin[1] + offset[1] + i as f32 * u_vec[1] + j as f32 * v_vec[1],
            ];
            let transform = PathTransform::scale(scale, scale)
                .then_rotate(rotation)
                .then_translate(Vector2D::new(position[0], position[1]));
            let transformed_path = symbol_path.as_ref().clone().transformed(&transform);

            match layer.paint {
                SymbolPaint::Filled => append_path(builder, &transformed_path),
                SymbolPaint::Open { stroke_width } => {
                    append_stroked_path_as_triangles(builder, &transformed_path, stroke_width)?;
                }
            }
        }
    }

    Ok(())
}

fn direction(angle_degrees: f32) -> [f32; 2] {
    let theta = angle_degrees.to_radians();
    [theta.cos(), theta.sin()]
}

fn lattice_index_ranges(
    bounds: PatternRect,
    origin: [f32; 2],
    offset: [f32; 2],
    u_vec: [f32; 2],
    v_vec: [f32; 2],
    det: f32,
) -> (i32, i32, i32, i32) {
    let mut min_i = f32::INFINITY;
    let mut max_i = f32::NEG_INFINITY;
    let mut min_j = f32::INFINITY;
    let mut max_j = f32::NEG_INFINITY;

    for corner in bounds.corners() {
        let q = [
            corner[0] - origin[0] - offset[0],
            corner[1] - origin[1] - offset[1],
        ];
        let i = cross(q, v_vec) / det;
        let j = cross(u_vec, q) / det;
        min_i = min_i.min(i);
        max_i = max_i.max(i);
        min_j = min_j.min(j);
        max_j = max_j.max(j);
    }

    (
        min_i.floor() as i32 - 1,
        max_i.ceil() as i32 + 1,
        min_j.floor() as i32 - 1,
        max_j.ceil() as i32 + 1,
    )
}

fn append_path(builder: &mut lyon_path::path::Builder, path: &Path) {
    for event in path.iter() {
        match event {
            Event::Begin { at } => {
                builder.begin(at);
            }
            Event::Line { to, .. } => {
                builder.line_to(to);
            }
            Event::Quadratic { ctrl, to, .. } => {
                builder.quadratic_bezier_to(ctrl, to);
            }
            Event::Cubic {
                ctrl1, ctrl2, to, ..
            } => {
                builder.cubic_bezier_to(ctrl1, ctrl2, to);
            }
            Event::End { close, .. } => {
                builder.end(close);
            }
        }
    }
}

fn path_is_empty(path: &Path) -> bool {
    !path
        .iter()
        .any(|event| matches!(event, Event::Begin { .. }))
}

fn append_stroked_path_as_triangles(
    builder: &mut lyon_path::path::Builder,
    path: &Path,
    stroke_width: f32,
) -> Result<(), PatternGeometryError> {
    let mut tessellator = StrokeTessellator::new();
    let mut buffers: VertexBuffers<Point2D<f32, UnknownUnit>, u16> = VertexBuffers::new();
    let options = StrokeOptions::default()
        .with_tolerance(0.05)
        .with_line_width(stroke_width)
        .with_line_join(LineJoin::Miter)
        .with_line_cap(LineCap::Butt);
    tessellator
        .tessellate_path(path, &options, &mut simple_builder(&mut buffers))
        .map_err(|_| PatternGeometryError::InvalidPattern)?;

    for triangle in buffers.indices.chunks_exact(3) {
        let p0 = buffers.vertices[triangle[0] as usize];
        let p1 = buffers.vertices[triangle[1] as usize];
        let p2 = buffers.vertices[triangle[2] as usize];
        builder.begin(p0);
        builder.line_to(p1);
        builder.line_to(p2);
        builder.close();
    }

    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "Keep the explicit inputs of the existing layout and rendering pipeline."
)]
fn append_dashed_stripe(
    builder: &mut lyon_path::path::Builder,
    origin: [f32; 2],
    d: [f32; 2],
    n: [f32; 2],
    stripe_offset: f32,
    half_width: f32,
    t_start: f32,
    t_end: f32,
    dash: &StripeDash,
) {
    let period = dash.length + dash.gap;
    let j_start = ((t_start - dash.phase) / period).floor() as i32;
    let j_end = ((t_end - dash.phase) / period).ceil() as i32;

    for j in j_start..=j_end {
        let start = dash.phase + j as f32 * period;
        let end = start + dash.length;
        let clipped_start = start.max(t_start);
        let clipped_end = end.min(t_end);

        if clipped_end > clipped_start {
            append_stripe_quad(
                builder,
                origin,
                d,
                n,
                stripe_offset,
                half_width,
                clipped_start,
                clipped_end,
            );
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "Keep the explicit inputs of the existing layout and rendering pipeline."
)]
fn append_stripe_quad(
    builder: &mut lyon_path::path::Builder,
    origin: [f32; 2],
    d: [f32; 2],
    n: [f32; 2],
    stripe_offset: f32,
    half_width: f32,
    t_start: f32,
    t_end: f32,
) {
    let p0 = stripe_point(origin, d, n, t_start, stripe_offset - half_width);
    let p1 = stripe_point(origin, d, n, t_end, stripe_offset - half_width);
    let p2 = stripe_point(origin, d, n, t_end, stripe_offset + half_width);
    let p3 = stripe_point(origin, d, n, t_start, stripe_offset + half_width);

    builder.begin(point(p0[0], p0[1]));
    builder.line_to(point(p1[0], p1[1]));
    builder.line_to(point(p2[0], p2[1]));
    builder.line_to(point(p3[0], p3[1]));
    builder.close();
}

fn stripe_point(origin: [f32; 2], d: [f32; 2], n: [f32; 2], t: f32, s: f32) -> [f32; 2] {
    [
        origin[0] + d[0] * t + n[0] * s,
        origin[1] + d[1] * t + n[1] * s,
    ]
}

fn projection_range(corners: &[[f32; 2]; 4], origin: [f32; 2], axis: [f32; 2]) -> (f32, f32) {
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;

    for corner in corners {
        let projection = dot(axis, [corner[0] - origin[0], corner[1] - origin[1]]);
        min = min.min(projection);
        max = max.max(projection);
    }

    (min, max)
}

fn dot(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}

fn cross(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[1] - a[1] * b[0]
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_color::{GradientStop, LinearGradient};
    use lyon_path::Event;

    fn context<'a>(
        host_bounds: PatternRect,
        host_fill: &'a ColorOrGradient,
        gradients: &'a [Gradient],
    ) -> PatternRenderContext<'a> {
        PatternRenderContext {
            chart_bounds: PatternRect::new(0.0, 0.0, 100.0, 80.0),
            plot_bounds: Some(PatternRect::new(10.0, 20.0, 50.0, 40.0)),
            host_bounds,
            host_fill,
            gradients,
        }
    }

    fn stripe_pattern(layer: StripePatternLayer) -> PatternFill {
        PatternFill {
            anchor: PatternAnchor::Mark,
            layers: vec![PatternLayer::Stripe(layer)],
            ..Default::default()
        }
    }

    fn begin_count(path: &Path) -> usize {
        path.iter()
            .filter(|event| matches!(event, Event::Begin { .. }))
            .count()
    }

    fn begins(path: &Path) -> Vec<[f32; 2]> {
        path.iter()
            .filter_map(|event| match event {
                Event::Begin { at } => Some([at.x, at.y]),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn horizontal_stripes_generate_one_quad_per_centerline() {
        let fill = stripe_pattern(StripePatternLayer::new(0.0, 16.0, 2.0));
        let host_fill = ColorOrGradient::Color([1.0, 1.0, 1.0, 1.0]);
        let geometry = build_layered_pattern_geometry(
            &fill,
            &context(PatternRect::new(0.0, 0.0, 32.0, 32.0), &host_fill, &[]),
        )
        .unwrap()
        .unwrap();

        assert_eq!(begin_count(&geometry.layers[0].coverage_path), 3);
        assert_eq!(begins(&geometry.layers[0].coverage_path)[0], [-1.0, -1.0]);
    }

    #[test]
    fn stripe_phase_offsets_centerlines_along_normal() {
        let mut layer = StripePatternLayer::new(0.0, 16.0, 2.0);
        layer.phase = 8.0;
        let fill = stripe_pattern(layer);
        let host_fill = ColorOrGradient::Color([1.0, 1.0, 1.0, 1.0]);
        let geometry = build_layered_pattern_geometry(
            &fill,
            &context(PatternRect::new(0.0, 0.0, 32.0, 32.0), &host_fill, &[]),
        )
        .unwrap()
        .unwrap();

        assert_eq!(begin_count(&geometry.layers[0].coverage_path), 2);
        assert_eq!(begins(&geometry.layers[0].coverage_path)[0], [-1.0, 7.0]);
    }

    #[test]
    fn dash_phase_uses_global_stripe_coordinate() {
        let mut layer = StripePatternLayer::new(0.0, 32.0, 2.0);
        layer.dash = Some(StripeDash {
            length: 4.0,
            gap: 4.0,
            phase: 2.0,
        });
        let fill = stripe_pattern(layer);
        let host_fill = ColorOrGradient::Color([1.0, 1.0, 1.0, 1.0]);
        let geometry = build_layered_pattern_geometry(
            &fill,
            &context(PatternRect::new(0.0, 0.0, 32.0, 1.0), &host_fill, &[]),
        )
        .unwrap()
        .unwrap();

        assert_eq!(begins(&geometry.layers[0].coverage_path)[0], [2.0, -1.0]);
    }

    #[test]
    fn layered_geometry_preserves_source_layer_operations() {
        let first = StripePatternLayer::new(0.0, 16.0, 2.0);
        let mut second = StripePatternLayer::new(90.0, 16.0, 2.0);
        second.operation = PatternLayerOperation::Subtract;
        let mut third = StripePatternLayer::new(45.0, 16.0, 2.0);
        third.operation = PatternLayerOperation::Xor;
        let fill = PatternFill {
            anchor: PatternAnchor::Mark,
            layers: vec![
                PatternLayer::Stripe(first),
                PatternLayer::Stripe(second),
                PatternLayer::Stripe(third),
            ],
            ..Default::default()
        };
        let host_fill = ColorOrGradient::Color([1.0, 1.0, 1.0, 1.0]);
        let geometry = build_layered_pattern_geometry(
            &fill,
            &context(PatternRect::new(0.0, 0.0, 32.0, 32.0), &host_fill, &[]),
        )
        .unwrap()
        .unwrap();

        assert_eq!(geometry.layers.len(), 3);
        assert_eq!(geometry.layers[2].operation, PatternLayerOperation::Xor);
        assert_eq!(geometry.layers[0].operation, PatternLayerOperation::Add);
        assert_eq!(
            geometry.layers[1].operation,
            PatternLayerOperation::Subtract
        );
        assert_eq!(
            begin_count(&geometry.merged_coverage_path),
            begin_count(&geometry.layers[0].coverage_path)
                + begin_count(&geometry.layers[1].coverage_path)
                + begin_count(&geometry.layers[2].coverage_path)
        );
    }

    #[test]
    fn layered_geometry_returns_none_for_empty_patterns() {
        let fill = PatternFill {
            anchor: PatternAnchor::Mark,
            layers: Vec::new(),
            ..Default::default()
        };
        let host_fill = ColorOrGradient::Color([1.0, 1.0, 1.0, 1.0]);

        assert!(build_layered_pattern_geometry(
            &fill,
            &context(PatternRect::new(0.0, 0.0, 32.0, 32.0), &host_fill, &[]),
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn plot_and_mark_anchors_produce_different_origins() {
        let mut plot_pattern = stripe_pattern(StripePatternLayer::new(0.0, 16.0, 2.0));
        plot_pattern.anchor = PatternAnchor::Plot;
        let mark_pattern = stripe_pattern(StripePatternLayer::new(0.0, 16.0, 2.0));
        let host_fill = ColorOrGradient::Color([1.0, 1.0, 1.0, 1.0]);
        let ctx = context(PatternRect::new(20.0, 25.0, 32.0, 15.0), &host_fill, &[]);

        let plot_geometry = build_layered_pattern_geometry(&plot_pattern, &ctx)
            .unwrap()
            .unwrap();
        let mark_geometry = build_layered_pattern_geometry(&mark_pattern, &ctx)
            .unwrap()
            .unwrap();

        assert_eq!(begins(&plot_geometry.layers[0].coverage_path)[0][1], 35.0);
        assert_eq!(begins(&mark_geometry.layers[0].coverage_path)[0][1], 24.0);
    }

    #[test]
    fn plot_anchor_requires_plot_bounds() {
        let mut pattern = stripe_pattern(StripePatternLayer::new(0.0, 16.0, 2.0));
        pattern.anchor = PatternAnchor::Plot;
        let host_fill = ColorOrGradient::Color([1.0, 1.0, 1.0, 1.0]);
        let mut ctx = context(PatternRect::new(0.0, 0.0, 32.0, 32.0), &host_fill, &[]);
        ctx.plot_bounds = None;

        assert_eq!(
            build_layered_pattern_geometry(&pattern, &ctx).unwrap_err(),
            PatternGeometryError::MissingPlotReferenceFrame
        );
    }

    #[test]
    fn auto_contrast_selects_dark_ink_for_light_fill() {
        let ink = resolve_pattern_ink(
            &PatternInk::AutoContrast { opacity: 0.18 },
            &ColorOrGradient::Color([1.0, 1.0, 1.0, 1.0]),
            &[],
        );

        assert_eq!(ink, [0.0, 0.0, 0.0, 0.18]);
    }

    #[test]
    fn auto_contrast_selects_light_ink_for_dark_fill() {
        let ink = resolve_pattern_ink(
            &PatternInk::AutoContrast { opacity: 0.18 },
            &ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]),
            &[],
        );

        assert_eq!(ink, [1.0, 1.0, 1.0, 0.18]);
    }

    #[test]
    fn auto_contrast_uses_rgb_channels_for_transparent_fill() {
        let light_transparent_ink = resolve_pattern_ink(
            &PatternInk::AutoContrast { opacity: 0.18 },
            &ColorOrGradient::Color([1.0, 1.0, 1.0, 0.0]),
            &[],
        );
        let dark_transparent_ink = resolve_pattern_ink(
            &PatternInk::AutoContrast { opacity: 0.18 },
            &ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]),
            &[],
        );

        assert_eq!(light_transparent_ink, [0.0, 0.0, 0.0, 0.18]);
        assert_eq!(dark_transparent_ink, [1.0, 1.0, 1.0, 0.18]);
    }

    #[test]
    fn solid_ink_multiplies_color_alpha_by_opacity() {
        let ink = resolve_pattern_ink(
            &PatternInk::Solid {
                color: [0.2, 0.4, 0.6, 0.5],
                opacity: 0.2,
            },
            &ColorOrGradient::Color([1.0, 1.0, 1.0, 1.0]),
            &[],
        );

        assert_eq!(ink, [0.2, 0.4, 0.6, 0.1]);
    }

    #[test]
    fn auto_contrast_uses_average_gradient_stop_color() {
        let gradients = vec![Gradient::LinearGradient(LinearGradient {
            x0: 0.0,
            y0: 0.0,
            x1: 1.0,
            y1: 0.0,
            stops: vec![
                GradientStop {
                    offset: 0.0,
                    color: [0.0, 0.0, 0.0, 1.0],
                },
                GradientStop {
                    offset: 1.0,
                    color: [0.0, 0.0, 0.0, 1.0],
                },
            ],
        })];
        let ink = resolve_pattern_ink(
            &PatternInk::AutoContrast { opacity: 0.18 },
            &ColorOrGradient::GradientIndex(0),
            &gradients,
        );

        assert_eq!(ink, [1.0, 1.0, 1.0, 0.18]);
    }

    #[test]
    fn symbol_layers_generate_lattice_coverage() {
        let pattern = PatternFill {
            anchor: PatternAnchor::Mark,
            layers: vec![PatternLayer::Symbol(
                crate::marks::pattern::SymbolPatternLayer {
                    operation: crate::marks::pattern::PatternLayerOperation::Add,
                    lattice: crate::marks::pattern::SymbolLattice2d {
                        u_spacing: 8.0,
                        u_angle: 0.0,
                        v_spacing: 8.0,
                        v_angle: 90.0,
                        u_phase: 0.0,
                        v_phase: 0.0,
                    },
                    symbol: crate::marks::pattern::PatternSymbol {
                        shape: "circle".to_string(),
                        size: 4.0,
                        rotation: 0.0,
                    },
                    paint: crate::marks::pattern::SymbolPaint::Filled,
                },
            )],
            ..Default::default()
        };
        let host_fill = ColorOrGradient::Color([1.0, 1.0, 1.0, 1.0]);
        let ctx = context(PatternRect::new(0.0, 0.0, 32.0, 32.0), &host_fill, &[]);

        let geometry = build_layered_pattern_geometry(&pattern, &ctx)
            .unwrap()
            .unwrap();

        assert!(begin_count(&geometry.layers[0].coverage_path) > 4);
        assert_ne!(begins(&geometry.layers[0].coverage_path)[0], [-1.0, -1.0]);
    }

    #[test]
    fn open_symbol_layers_generate_stroked_coverage() {
        let pattern = PatternFill {
            anchor: PatternAnchor::Mark,
            layers: vec![PatternLayer::Symbol(
                crate::marks::pattern::SymbolPatternLayer {
                    operation: crate::marks::pattern::PatternLayerOperation::Add,
                    lattice: crate::marks::pattern::SymbolLattice2d {
                        u_spacing: 16.0,
                        u_angle: 0.0,
                        v_spacing: 16.0,
                        v_angle: 90.0,
                        u_phase: 0.0,
                        v_phase: 0.0,
                    },
                    symbol: crate::marks::pattern::PatternSymbol {
                        shape: "square".to_string(),
                        size: 9.0,
                        rotation: 0.0,
                    },
                    paint: crate::marks::pattern::SymbolPaint::Open { stroke_width: 1.0 },
                },
            )],
            ..Default::default()
        };
        let host_fill = ColorOrGradient::Color([1.0, 1.0, 1.0, 1.0]);
        let ctx = context(PatternRect::new(0.0, 0.0, 32.0, 32.0), &host_fill, &[]);

        let geometry = build_layered_pattern_geometry(&pattern, &ctx)
            .unwrap()
            .unwrap();

        assert!(begin_count(&geometry.layers[0].coverage_path) > 8);
    }

    #[test]
    fn parallel_symbol_lattice_vectors_are_invalid() {
        let pattern = PatternFill {
            layers: vec![PatternLayer::Symbol(
                crate::marks::pattern::SymbolPatternLayer {
                    operation: crate::marks::pattern::PatternLayerOperation::Add,
                    lattice: crate::marks::pattern::SymbolLattice2d {
                        u_spacing: 8.0,
                        u_angle: 0.0,
                        v_spacing: 8.0,
                        v_angle: 0.0,
                        u_phase: 0.0,
                        v_phase: 0.0,
                    },
                    symbol: crate::marks::pattern::PatternSymbol {
                        shape: "circle".to_string(),
                        size: 4.0,
                        rotation: 0.0,
                    },
                    paint: crate::marks::pattern::SymbolPaint::Filled,
                },
            )],
            ..Default::default()
        };
        let host_fill = ColorOrGradient::Color([1.0, 1.0, 1.0, 1.0]);
        let ctx = context(PatternRect::new(0.0, 0.0, 32.0, 32.0), &host_fill, &[]);

        assert_eq!(
            build_layered_pattern_geometry(&pattern, &ctx).unwrap_err(),
            PatternGeometryError::InvalidPattern
        );
    }
}
