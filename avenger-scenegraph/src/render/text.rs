//! The shapes a label draws, as lyon paths with scene paint.

use avenger_common::types::{FillRule, StrokeCap, StrokeJoin};
use avenger_typst_label::{Curve, CurveItem, LineCap, LineJoin, PathItem, Transform};
use lyon_path::{geom::point, Path, PathEvent};

/// A filled or stroked shape: a glyph's outline, or a shape such as a fraction line or a text
/// decoration. Its path and stroke are in the label's coordinates.
#[derive(Debug, Clone)]
pub struct TextShape {
    pub path: Path,
    /// The fill's straight-alpha color.
    pub fill: Option<[f32; 4]>,
    pub fill_rule: FillRule,
    pub stroke: Option<TextStroke>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextStroke {
    /// The stroke's straight-alpha color.
    pub color: [f32; 4],
    pub width: f32,
    pub cap: StrokeCap,
    pub join: StrokeJoin,
    /// The miter limit, as SVG measures it.
    pub miter_limit: f32,
    pub dash: Option<Vec<f32>>,
    /// Where the dash pattern starts along the path.
    pub dash_offset: f32,
}

impl TextShape {
    /// A path item's shape, in the label's coordinates.
    pub fn new(item: &PathItem) -> Self {
        let transform = item.transform;
        // The stroke is in the path's coordinates, so its lengths scale with the transform.
        let scale = (transform.sx * transform.sy - transform.kx * transform.ky)
            .abs()
            .sqrt();
        Self {
            path: lyon_path(&item.path, transform),
            fill: item.fill.map(|fill| fill.to_rgba()),
            fill_rule: match item.fill_rule {
                avenger_typst_label::FillRule::NonZero => FillRule::NonZero,
                avenger_typst_label::FillRule::EvenOdd => FillRule::EvenOdd,
            },
            stroke: item.stroke.as_ref().map(|stroke| TextStroke {
                color: stroke.paint.to_rgba(),
                width: stroke.thickness * scale,
                cap: match stroke.cap {
                    LineCap::Butt => StrokeCap::Butt,
                    LineCap::Round => StrokeCap::Round,
                    LineCap::Square => StrokeCap::Square,
                },
                join: match stroke.join {
                    LineJoin::Miter => StrokeJoin::Miter,
                    LineJoin::Round => StrokeJoin::Round,
                    LineJoin::Bevel => StrokeJoin::Bevel,
                },
                miter_limit: stroke.miter_limit,
                dash: stroke
                    .dash
                    .as_ref()
                    .map(|dash| dash.array.iter().map(|length| length * scale).collect()),
                dash_offset: stroke.dash.as_ref().map_or(0.0, |dash| dash.phase * scale),
            }),
        }
    }
}

/// The label crate's curve of a lyon path, one item for each of its events but the ends of open
/// subpaths.
pub(crate) fn lyon_curve(path: &Path) -> Curve {
    let point = |p: lyon_path::math::Point| avenger_typst_label::Point::new(p.x, p.y);
    let mut items = Vec::new();
    for event in path.iter() {
        match event {
            PathEvent::Begin { at } => items.push(CurveItem::Move(point(at))),
            PathEvent::Line { to, .. } => items.push(CurveItem::Line(point(to))),
            PathEvent::Quadratic { ctrl, to, .. } => {
                items.push(CurveItem::Quad(point(ctrl), point(to)))
            }
            PathEvent::Cubic {
                ctrl1, ctrl2, to, ..
            } => items.push(CurveItem::Cubic(point(ctrl1), point(ctrl2), point(to))),
            PathEvent::End { close: true, .. } => items.push(CurveItem::Close),
            PathEvent::End { .. } => {}
        }
    }
    Curve(items)
}

/// The lyon path of a label's curve, in the curve's coordinates.
pub fn curve_path(curve: &Curve) -> Path {
    lyon_path(curve, Transform::IDENTITY)
}

fn lyon_path(curve: &Curve, transform: Transform) -> Path {
    let to_point = |p: avenger_typst_label::Point| {
        let p = transform.apply(p);
        point(p.x, p.y)
    };
    let mut builder = Path::builder().with_svg();
    for item in &curve.0 {
        match *item {
            CurveItem::Move(to) => {
                builder.move_to(to_point(to));
            }
            CurveItem::Line(to) => {
                builder.line_to(to_point(to));
            }
            CurveItem::Quad(control, to) => {
                builder.quadratic_bezier_to(to_point(control), to_point(to));
            }
            CurveItem::Cubic(first, second, to) => {
                builder.cubic_bezier_to(to_point(first), to_point(second), to_point(to));
            }
            CurveItem::Close => builder.close(),
        }
    }
    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_color::AbsoluteColor;
    use avenger_typst_label::{DashPattern, Point, Stroke};

    #[test]
    fn strokes_scale_with_the_shape_transform() {
        let item = PathItem {
            path: Curve(vec![
                CurveItem::Move(Point::new(0.0, 0.0)),
                CurveItem::Line(Point::new(4.0, 0.0)),
            ]),
            transform: Transform {
                sx: 2.0,
                ky: 0.0,
                kx: 0.0,
                sy: 2.0,
                tx: 1.0,
                ty: 3.0,
            },
            fill: None,
            fill_rule: avenger_typst_label::FillRule::EvenOdd,
            stroke: Some(Stroke {
                paint: AbsoluteColor::from_rgba([0.1, 0.2, 0.3, 1.0]),
                thickness: 1.0,
                cap: LineCap::Round,
                join: LineJoin::Bevel,
                dash: Some(DashPattern {
                    array: vec![2.0, 1.0],
                    phase: 0.5,
                }),
                miter_limit: 2.0,
            }),
            kind: avenger_typst_label::PathKind::Shape,
        };
        let shape = TextShape::new(&item);
        let endpoints: Vec<_> = shape
            .path
            .iter()
            .filter_map(|event| match event {
                lyon_path::Event::Begin { at } => Some(at.to_array()),
                lyon_path::Event::Line { to, .. } => Some(to.to_array()),
                _ => None,
            })
            .collect();
        assert_eq!(endpoints, [[1.0, 3.0], [9.0, 3.0]]);
        assert_eq!(shape.fill_rule, FillRule::EvenOdd);
        assert_eq!(
            shape.stroke,
            Some(TextStroke {
                color: [0.1, 0.2, 0.3, 1.0],
                width: 2.0,
                cap: StrokeCap::Round,
                join: StrokeJoin::Bevel,
                miter_limit: 2.0,
                dash: Some(vec![4.0, 2.0]),
                dash_offset: 1.0,
            })
        );
    }
}
