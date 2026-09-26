//! Filled contours shared by renderers and geometry queries.

use avenger_common::types::FillRule;
use i_overlay::{core::overlay_rule::OverlayRule, float::overlay::FloatOverlay};
use lyon_path::{iterator::PathIterator, math::point, Path, PathEvent};
use lyon_tessellation::{
    BuffersBuilder, LineCap, LineJoin, StrokeOptions, StrokeTessellator, StrokeVertex,
    TessellationError, VertexBuffers,
};

/// Disjoint polygons, each containing an exterior followed by its interior rings.
pub type FilledContours = Vec<Vec<Vec<[f32; 2]>>>;

/// Reference rectangle for normalized gradient coordinates, independent of painted coverage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientBounds {
    pub min: [f32; 2],
    pub max: [f32; 2],
}

impl GradientBounds {
    /// Use the unexpanded path bounds for gradient placement.
    pub fn from_path(path: &Path) -> Self {
        let bounds = lyon_algorithms::aabb::bounding_box(path);
        Self {
            min: bounds.min.to_array(),
            max: bounds.max.to_array(),
        }
    }

    /// Symbols use their nominal square regardless of shape and rotation.
    pub fn symbol(center: [f32; 2], size: f32) -> Self {
        let half = size.sqrt() / 2.0;
        Self {
            min: [center[0] - half, center[1] - half],
            max: [center[0] + half, center[1] + half],
        }
    }

    /// Radial gradients use a centered enclosing square.
    pub fn radial(self) -> Self {
        let width = self.max[0] - self.min[0];
        let height = self.max[1] - self.min[1];
        let side = width.max(height);
        let min = [
            self.min[0] - (side - width) / 2.0,
            self.min[1] - (side - height) / 2.0,
        ];
        Self {
            min,
            max: [min[0] + side, min[1] + side],
        }
    }
}

/// Resolve a compound path's filled region, including intersections and holes.
pub fn filled_contours(path: &Path, tolerance: f32, rule: FillRule) -> FilledContours {
    let mut contours = Vec::new();
    let mut contour = Vec::new();
    for event in path.iter().flattened(tolerance) {
        match event {
            PathEvent::Begin { at } => contour.push(at.to_array()),
            PathEvent::Line { to, .. } => contour.push(to.to_array()),
            PathEvent::End { .. } => {
                // Overlay contours close implicitly.
                if contour.first() == contour.last() {
                    contour.pop();
                }
                if contour.len() >= 3 {
                    contours.push(std::mem::take(&mut contour));
                }
                contour.clear();
            }
            _ => unreachable!("flattened paths contain only line segments"),
        }
    }
    resolve_contours(&contours, rule)
}

fn resolve_contours(contours: &[Vec<[f32; 2]>], rule: FillRule) -> FilledContours {
    if contours.is_empty() {
        return Vec::new();
    }
    let rule = match rule {
        FillRule::NonZero => i_overlay::core::fill_rule::FillRule::NonZero,
        FillRule::EvenOdd => i_overlay::core::fill_rule::FillRule::EvenOdd,
    };
    FloatOverlay::with_subj(contours).overlay(OverlayRule::Subject, rule)
}

/// Build one nonzero-filled outline of a variable-width round stroke.
pub fn trail_outline(
    centerline: &Path,
    tolerance: f32,
    size_attribute_index: usize,
) -> Result<Path, TessellationError> {
    let mut buffers: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
    StrokeTessellator::new().tessellate_path(
        centerline,
        &StrokeOptions::default()
            .with_tolerance(tolerance)
            .with_line_cap(LineCap::Round)
            .with_line_join(LineJoin::Round)
            .with_variable_line_width(size_attribute_index),
        &mut BuffersBuilder::new(&mut buffers, |vertex: StrokeVertex<'_, '_>| {
            vertex.position().to_array()
        }),
    )?;
    let mut isolated = None;
    let mut has_segment = false;
    for event in centerline.iter_with_attributes() {
        match event {
            lyon_path::Event::Begin {
                at: (at, attributes),
            } => {
                isolated = Some((at, attributes[size_attribute_index]));
                has_segment = false;
            }
            lyon_path::Event::Line { from, to } => {
                has_segment |= from.0 != to.0;
                if let Some((_, size)) = &mut isolated {
                    *size = size.max(to.1[size_attribute_index]);
                }
            }
            lyon_path::Event::End { .. } => {
                if let Some((at, size)) = isolated.take().filter(|_| !has_segment) {
                    if size > 0.0 {
                        lyon_tessellation::FillTessellator::new().tessellate_circle(
                            at,
                            size / 2.0,
                            &lyon_tessellation::FillOptions::default().with_tolerance(tolerance),
                            &mut BuffersBuilder::new(
                                &mut buffers,
                                |vertex: lyon_tessellation::FillVertex<'_>| {
                                    vertex.position().to_array()
                                },
                            ),
                        )?;
                    }
                }
            }
            _ => has_segment = true,
        }
    }
    let triangles: Vec<_> = buffers
        .indices
        .chunks_exact(3)
        .filter_map(|indices| {
            let [a, mut b, mut c] = [
                buffers.vertices[indices[0] as usize],
                buffers.vertices[indices[1] as usize],
                buffers.vertices[indices[2] as usize],
            ];
            let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            if area == 0.0 {
                return None;
            }
            // Consistent winding makes overlapping stroke triangles contribute union coverage.
            if area < 0.0 {
                std::mem::swap(&mut b, &mut c);
            }
            Some(vec![a, b, c])
        })
        .collect();
    let shapes = resolve_contours(&triangles, FillRule::NonZero);
    Ok(contours_to_path(&shapes))
}

fn contours_to_path(shapes: &FilledContours) -> Path {
    let mut builder = Path::builder();
    for contour in shapes.iter().flatten() {
        if let Some(first) = contour.first() {
            builder.begin(point(first[0], first[1]));
            for p in &contour[1..] {
                builder.line_to(point(p[0], p[1]));
            }
            builder.close();
        }
    }
    builder.build()
}
