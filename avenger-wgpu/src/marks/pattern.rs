//! Pattern fills as records in a texture. The fill shader reads an item's record to work out how
//! much of each pixel the pattern covers, and paints that coverage in the pattern's ink.

use std::collections::HashMap;

use avenger_color::{ColorOrGradient, Gradient};
use avenger_common::types::PathTransform;
use avenger_scenegraph::{
    marks::pattern::{
        PatternAnchor, PatternFill, PatternLayerKind, PatternLayerOperation, PatternSymbolShape,
        StripeLayer, SymbolLayer, SymbolPaint,
    },
    pattern_geometry::resolve_pattern_ink,
};
use lyon::{
    geom::{point, Angle},
    path::Event,
};
use wgpu::Extent3d;

use crate::error::AvengerWgpuError;

/// The width of the pattern texture in texels. `multi.wgsl` addresses texels by this width.
pub(crate) const PATTERN_TEXTURE_WIDTH: u32 = 2048;

/// The most rows of pattern texels: the largest texture side that WebGL2 guarantees.
const PATTERN_TEXTURE_MAX_ROWS: u32 = 2048;

// Codes that `multi.wgsl` reads.
const STRIPE_LAYER: f32 = 0.0;
const SYMBOL_LAYER: f32 = 1.0;
const CIRCLE_SYMBOL: f32 = 0.0;
const POLYGON_SYMBOL: f32 = 1.0;

/// A frame's pattern records, as `vec4<f32>` texels. A fill vertex names its pattern by the offset
/// of its record's first texel, and offset 0 means no pattern.
///
/// A record is a header texel `[layer count, mark anchor, origin x, origin y]`, the ink texel, and
/// each layer: a header `[kind, operation, parameter texels, 0]` and its kind's parameters.
pub(crate) struct PatternRecords {
    texels: Vec<[f32; 4]>,
    offsets: HashMap<Vec<u32>, u32>,
}

impl Default for PatternRecords {
    fn default() -> Self {
        Self {
            texels: vec![[0.0; 4]],
            offsets: HashMap::new(),
        }
    }
}

impl PatternRecords {
    /// The offset of the record that paints `pattern` over an item filled with `fill`, or 0 when
    /// the pattern paints nothing. `group_origin` is where group-anchored patterns start.
    pub(crate) fn register(
        &mut self,
        pattern: &PatternFill,
        fill: &ColorOrGradient,
        gradients: &[Gradient],
        group_origin: [f32; 2],
    ) -> Result<u32, AvengerWgpuError> {
        if pattern.layers.is_empty() {
            return Ok(0);
        }
        pattern.validate()?;
        let ink = resolve_pattern_ink(&pattern.ink, fill, gradients);
        if ink[3] <= 0.0 {
            return Ok(0);
        }

        // Mark anchors start from each item's bounds, which its vertices carry.
        let (mark_anchor, origin) = match pattern.anchor {
            PatternAnchor::Group => (0.0, group_origin),
            PatternAnchor::Mark => (1.0, [0.0, 0.0]),
            PatternAnchor::Scene => (0.0, [0.0, 0.0]),
        };
        let mut record = vec![
            [
                pattern.layers.len() as f32,
                mark_anchor,
                origin[0],
                origin[1],
            ],
            ink,
        ];
        for layer in &pattern.layers {
            let header = record.len();
            record.push([0.0; 4]);
            let kind = match &layer.kind {
                PatternLayerKind::Stripe(stripe) => {
                    push_stripe(&mut record, stripe);
                    STRIPE_LAYER
                }
                PatternLayerKind::Symbol(symbol) => {
                    push_symbol(&mut record, symbol);
                    SYMBOL_LAYER
                }
            };
            let operation = match layer.operation {
                PatternLayerOperation::Add => 0.0,
                PatternLayerOperation::Subtract => 1.0,
                PatternLayerOperation::Xor => 2.0,
            };
            record[header] = [kind, operation, (record.len() - header - 1) as f32, 0.0];
        }

        let key: Vec<u32> = record
            .iter()
            .flatten()
            .map(|value| value.to_bits())
            .collect();
        if let Some(offset) = self.offsets.get(&key) {
            return Ok(*offset);
        }
        let offset = self.texels.len();
        if offset + record.len() > (PATTERN_TEXTURE_WIDTH * PATTERN_TEXTURE_MAX_ROWS) as usize {
            return Err(AvengerWgpuError::TooManyPatterns);
        }
        self.texels.extend(record);
        self.offsets.insert(key, offset as u32);
        Ok(offset as u32)
    }

    /// The size of the texture that holds the records.
    pub(crate) fn texture_size(&self) -> Extent3d {
        Extent3d {
            width: PATTERN_TEXTURE_WIDTH,
            height: (self.texels.len() as u32).div_ceil(PATTERN_TEXTURE_WIDTH),
            depth_or_array_layers: 1,
        }
    }

    /// The texels of the texture, padded to whole rows.
    pub(crate) fn texture_texels(&self) -> Vec<[f32; 4]> {
        let size = self.texture_size();
        let mut texels = self.texels.clone();
        texels.resize((size.width * size.height) as usize, [0.0; 4]);
        texels
    }
}

/// A stripe layer's parameters: `[normal x, normal y, spacing, half width]`, then
/// `[phase, dash period, dash half length, dash center]`, with period 0 for a solid stripe.
fn push_stripe(record: &mut Vec<[f32; 4]>, stripe: &StripeLayer) {
    let theta = stripe.angle.to_radians();
    record.push([
        -theta.sin(),
        theta.cos(),
        stripe.spacing,
        stripe.stroke_width / 2.0,
    ]);
    record.push(match &stripe.dash {
        Some(dash) => [
            stripe.phase,
            dash.length + dash.gap,
            dash.length / 2.0,
            dash.phase + dash.length / 2.0,
        ],
        None => [stripe.phase, 0.0, 0.0, 0.0],
    });
}

/// A symbol layer's parameters: the reduced lattice steps `[u x, u y, v x, v y]`, the rows of
/// their inverse, `[phase x, phase y, reach, half stroke width]`, and the shape. The shape is
/// `[circle, radius, 0, 0]`, or `[polygons, part count, 0, 0]` followed by each convex part:
/// `[edge count, 0, 0, 0]` and its edges' outward lines `[normal x, normal y, offset, 0]`.
fn push_symbol(record: &mut Vec<[f32; 4]>, layer: &SymbolLayer) {
    let (u, v) = layer.lattice.reduced_steps();
    let det = u[0] * v[1] - u[1] * v[0];
    record.push([u[0], u[1], v[0], v[1]]);
    record.push([v[1] / det, -v[0] / det, -u[1] / det, u[0] / det]);

    let direction = |angle: f32| {
        let theta = angle.to_radians();
        [theta.cos(), theta.sin()]
    };
    let (u_dir, v_dir) = (
        direction(layer.lattice.u_angle),
        direction(layer.lattice.v_angle),
    );
    let phase = [
        u_dir[0] * layer.lattice.u_phase + v_dir[0] * layer.lattice.v_phase,
        u_dir[1] * layer.lattice.u_phase + v_dir[1] * layer.lattice.v_phase,
    ];
    let half_stroke = match layer.paint {
        SymbolPaint::Filled => 0.0,
        SymbolPaint::Open { stroke_width } => stroke_width / 2.0,
    };

    let scale = layer.symbol.size.sqrt();
    let Some(parts) = symbol_parts(layer.symbol.shape) else {
        let radius = scale / 2.0;
        record.push([phase[0], phase[1], radius + half_stroke, half_stroke]);
        record.push([CIRCLE_SYMBOL, radius, 0.0, 0.0]);
        return;
    };

    let transform =
        PathTransform::scale(scale, scale).then_rotate(Angle::degrees(layer.symbol.rotation));
    let parts: Vec<Vec<[f32; 2]>> = parts
        .iter()
        .map(|part| {
            part.iter()
                .map(|p| transform.transform_point(point(p[0], p[1])).to_array())
                .collect()
        })
        .collect();
    let radius = parts
        .iter()
        .flatten()
        .map(|p| p[0].hypot(p[1]))
        .fold(0.0, f32::max);
    record.push([phase[0], phase[1], radius + half_stroke, half_stroke]);
    record.push([POLYGON_SYMBOL, parts.len() as f32, 0.0, 0.0]);
    for part in parts {
        record.push([part.len() as f32, 0.0, 0.0, 0.0]);
        let area: f32 = (0..part.len())
            .map(|k| {
                let (a, b) = (part[k], part[(k + 1) % part.len()]);
                a[0] * b[1] - a[1] * b[0]
            })
            .sum();
        for k in 0..part.len() {
            let (a, b) = (part[k], part[(k + 1) % part.len()]);
            let edge = [b[0] - a[0], b[1] - a[1]];
            let length = edge[0].hypot(edge[1]);
            let normal = if area > 0.0 {
                [edge[1] / length, -edge[0] / length]
            } else {
                [-edge[1] / length, edge[0] / length]
            };
            record.push([
                normal[0],
                normal[1],
                normal[0] * a[0] + normal[1] * a[1],
                0.0,
            ]);
        }
    }
}

/// A shape at size 1 as convex polygons whose union is the shape, or `None` for the circle. Parts
/// overlap, so that every interior point is strictly inside one of them: an open symbol strokes
/// the zero set of the parts' union, which would otherwise include an edge that two parts share.
fn symbol_parts(shape: PatternSymbolShape) -> Option<Vec<Vec<[f32; 2]>>> {
    match shape {
        PatternSymbolShape::Circle => None,
        PatternSymbolShape::Cross => {
            // The cross's two bars, with the cross's arm half-width of 0.2.
            let bar = |w: f32, h: f32| vec![[-w, -h], [w, -h], [w, h], [-w, h]];
            Some(vec![bar(0.5, 0.2), bar(0.2, 0.5)])
        }
        PatternSymbolShape::Square
        | PatternSymbolShape::Diamond
        | PatternSymbolShape::TriangleUp
        | PatternSymbolShape::TriangleDown
        | PatternSymbolShape::TriangleLeft
        | PatternSymbolShape::TriangleRight => {
            let path = shape.symbol_shape().as_path().into_owned();
            let mut outline = Vec::new();
            for event in path.iter() {
                match event {
                    Event::Begin { at } => outline.push(at.to_array()),
                    Event::Line { to, .. } => outline.push(to.to_array()),
                    _ => {}
                }
            }
            if outline.len() > 1 && outline.first() == outline.last() {
                outline.pop();
            }
            Some(vec![outline])
        }
    }
}

#[cfg(test)]
mod tests {
    use avenger_scenegraph::marks::pattern::{PatternSymbol, SymbolLattice2d};
    use lyon::algorithms::hit_test::hit_test_path;
    use lyon::path::FillRule;

    use super::*;

    #[test]
    fn symbol_parts_cover_their_shape() {
        for shape in [
            PatternSymbolShape::Square,
            PatternSymbolShape::Diamond,
            PatternSymbolShape::Cross,
            PatternSymbolShape::TriangleUp,
            PatternSymbolShape::TriangleDown,
            PatternSymbolShape::TriangleLeft,
            PatternSymbolShape::TriangleRight,
        ] {
            let parts = symbol_parts(shape).unwrap();
            let path = shape.symbol_shape().as_path().into_owned();
            // A point is inside a convex part when it is on the same side of every edge.
            let inside_part = |p: [f32; 2]| {
                parts.iter().any(|part| {
                    let sides: Vec<f32> = (0..part.len())
                        .map(|k| {
                            let (a, b) = (part[k], part[(k + 1) % part.len()]);
                            (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
                        })
                        .collect();
                    sides.iter().all(|side| *side > 0.0) || sides.iter().all(|side| *side < 0.0)
                })
            };
            for i in 0..41 {
                for j in 0..41 {
                    let p = [
                        i as f32 / 40.0 - 0.5 + 0.0031,
                        j as f32 / 40.0 - 0.5 + 0.0017,
                    ];
                    let in_path =
                        hit_test_path(&point(p[0], p[1]), path.iter(), FillRule::EvenOdd, 1e-4);
                    assert_eq!(inside_part(p), in_path, "{shape:?} at {p:?}");
                }
            }
        }
    }

    #[test]
    fn equal_patterns_share_a_record() {
        let layer = SymbolLayer {
            lattice: SymbolLattice2d {
                u_spacing: 10.0,
                u_angle: 0.0,
                v_spacing: 10.0,
                v_angle: 90.0,
                u_phase: 0.0,
                v_phase: 0.0,
            },
            symbol: PatternSymbol {
                shape: PatternSymbolShape::Cross,
                size: 30.0,
                rotation: 10.0,
            },
            paint: SymbolPaint::Open { stroke_width: 1.0 },
        };
        let pattern = PatternFill {
            layers: vec![StripeLayer::new(45.0, 8.0, 2.0).into(), layer.into()],
            ..Default::default()
        };
        let dark = ColorOrGradient::Color([0.1, 0.1, 0.1, 1.0]);
        let light = ColorOrGradient::Color([0.9, 0.9, 0.9, 1.0]);
        let mut records = PatternRecords::default();
        let first = records.register(&pattern, &dark, &[], [0.0; 2]).unwrap();
        assert_eq!(first, 1);
        // Header, ink, a stripe (header and two texels), and a cross: a header, four lattice and
        // shape texels, and two parts of four edges.
        assert_eq!(records.texels.len(), 1 + 2 + 3 + 1 + 4 + 2 * 5);
        assert_eq!(
            records.register(&pattern, &dark, &[], [0.0; 2]).unwrap(),
            first
        );
        // AutoContrast paints white ink over the dark fill and black over the light one.
        let second = records.register(&pattern, &light, &[], [0.0; 2]).unwrap();
        assert_ne!(second, first);
        assert_eq!(records.texels[first as usize + 1][0], 1.0);
        assert_eq!(records.texels[second as usize + 1][0], 0.0);
    }
}
