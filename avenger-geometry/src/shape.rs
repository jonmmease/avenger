//! The shapes that queries compare: an instance's shape at an extent, which its mark's clip shows
//! or hides whole, and a query's own shape. Each is a geometry grown by a distance, which `geo`
//! buffers into polygons where a relation needs the area.

use std::borrow::Cow;

use avenger_scenegraph::{marks::group::Clip, render::fill::filled_contours};
use geo::algorithm::buffer::{BufferStyle, LineCap, LineJoin};
use geo::dimensions::Dimensions;
use geo::{BoundingRect, Buffer, Centroid, HasDimensions, Intersects};
use geo_types::{Coord, Geometry, GeometryCollection, Line, MultiPolygon, Point, Polygon, Rect};
use rstar::{Envelope, AABB};

use crate::query::Extent;
use crate::rtree::GeometryInstance;

/// The largest gap between a round edge, such as a disc's or a stroke's end, and the polygon
/// edges that stand in for it when a shape becomes polygons.
const FLATTENING: f32 = 0.01;

/// The largest gap between a clip path's curves and the polygon edges that stand in for them.
const CLIP_FLATTENING: f32 = 0.1;

/// A shape that a relation compares: every point within `by` of `geometry`, or nothing. The
/// geometry is borrowed from an instance where it can be.
#[derive(Debug, Clone)]
pub(crate) enum Shape<'a> {
    Empty,
    Grown {
        geometry: Cow<'a, Geometry<f32>>,
        by: f32,
    },
}

/// A mark's clip, with a clip path's area found once for every instance that it shows or hides.
#[derive(Debug, Clone)]
pub(crate) enum ClipArea {
    None,
    Rect(AABB<[f32; 2]>),
    Path(MultiPolygon<f32>),
    /// A clip with no area, which hides everything.
    Nothing,
}

impl ClipArea {
    pub(crate) fn new(clip: &Clip) -> Self {
        match clip {
            Clip::None => Self::None,
            Clip::Rect {
                x,
                y,
                width,
                height,
            } => {
                if *width <= 0.0 || *height <= 0.0 {
                    return Self::Nothing;
                }
                Self::Rect(AABB::from_corners([*x, *y], [x + width, y + height]))
            }
            Clip::Path(path) => {
                let contours = filled_contours(path, CLIP_FLATTENING);
                let polygons =
                    MultiPolygon::new(contours.iter().map(|shape| polygon(shape)).collect());
                if polygons.0.is_empty() {
                    return Self::Nothing;
                }
                Self::Path(polygons)
            }
        }
    }

    /// Whether the clip shows any of `envelope`, edge or inside.
    pub(crate) fn meets(&self, envelope: &AABB<[f32; 2]>) -> bool {
        match self {
            Self::None => true,
            Self::Nothing => false,
            Self::Rect(rect) => rect.intersects(envelope),
            Self::Path(area) => area.intersects(&box_geometry(envelope)),
        }
    }
}

/// `envelope` as a `geo` geometry: its rect, or the segment or point that a box with no area is.
pub(crate) fn box_geometry(envelope: &AABB<[f32; 2]>) -> Geometry<f32> {
    rect_or_flat(envelope.lower(), envelope.upper())
}

/// The rectangle between `a` and `b`, or the segment or point between them when it has no area,
/// which `geo` relates as the rect's own shape.
pub(crate) fn rect_or_flat(a: [f32; 2], b: [f32; 2]) -> Geometry<f32> {
    if a == b {
        Point::from(a).into()
    } else if a[0] == b[0] || a[1] == b[1] {
        Line::new(a, b).into()
    } else {
        Rect::new(a, b).into()
    }
}

impl<'a> Shape<'a> {
    /// `geometry` itself, or nothing when it holds no point.
    pub(crate) fn exact(geometry: Cow<'a, Geometry<f32>>) -> Self {
        Self::grown(geometry, 0.0)
    }

    /// Every point within `by` of `geometry`, or nothing when the geometry holds no point.
    pub(crate) fn grown(geometry: Cow<'a, Geometry<f32>>, by: f32) -> Self {
        if geometry.is_empty() || !by.is_finite() || by < 0.0 {
            return Self::Empty;
        }
        Self::Grown { geometry, by }
    }

    /// `instance`'s shape at `extent`, before its clip.
    pub(crate) fn of(instance: &'a GeometryInstance, extent: Extent) -> Self {
        let geometry = &instance.geometry;
        match extent {
            Extent::Drawn => Self::grown(Cow::Borrowed(geometry), instance.reach()),
            Extent::Geometry => Self::exact(Cow::Borrowed(geometry)),
            Extent::Centroid => geometry.centroid().map_or(Self::Empty, |centroid| {
                Self::exact(Cow::Owned(Geometry::from(centroid)))
            }),
            Extent::Fill => match geometry.dimensions() {
                Dimensions::TwoDimensional => match geometry {
                    Geometry::GeometryCollection(collection) => Self::grown(
                        Cow::Owned(Geometry::from(areas(collection))),
                        instance.radius,
                    ),
                    geometry => Self::grown(Cow::Borrowed(geometry), instance.radius),
                },
                // A circle symbol's center fills its disc, and lines and bare points fill nothing.
                Dimensions::ZeroDimensional if instance.radius > 0.0 => {
                    Self::grown(Cow::Borrowed(geometry), instance.radius)
                }
                _ => Self::Empty,
            },
        }
    }

    /// The shape's geometry and how far it grows, or `None` when it is empty.
    pub(crate) fn parts(&self) -> Option<(&Geometry<f32>, f32)> {
        match self {
            Self::Empty => None,
            Self::Grown { geometry, by } => Some((geometry, *by)),
        }
    }

    /// The box around the shape, or `None` when it is empty.
    pub(crate) fn envelope(&self) -> Option<AABB<[f32; 2]>> {
        let Self::Grown { geometry, by } = self else {
            return None;
        };
        let bounds = geometry.bounding_rect()?;
        Some(AABB::from_corners(
            [bounds.min().x - by, bounds.min().y - by],
            [bounds.max().x + by, bounds.max().y + by],
        ))
    }

    /// The shape, or nothing when `clip` shows none of its box.
    pub(crate) fn visible(self, clip: &ClipArea) -> Self {
        match self.envelope() {
            Some(envelope) if clip.meets(&envelope) => self,
            _ => Self::Empty,
        }
    }

    /// The shape as polygons, with round edges within [`FLATTENING`] of the shape's, or no
    /// polygons when the shape has no area.
    pub(crate) fn polygons(&self) -> MultiPolygon<f32> {
        match self {
            Self::Empty => MultiPolygon::new(Vec::new()),
            Self::Grown { geometry, by } if *by > 0.0 => buffered(geometry, *by),
            Self::Grown { geometry, .. } => match geometry.as_ref() {
                Geometry::Polygon(polygon) => MultiPolygon::new(vec![polygon.clone()]),
                Geometry::MultiPolygon(polygons) => polygons.clone(),
                Geometry::Rect(rect) => MultiPolygon::new(vec![rect.to_polygon()]),
                Geometry::Triangle(triangle) => MultiPolygon::new(vec![triangle.to_polygon()]),
                Geometry::GeometryCollection(collection) => areas(collection),
                _ => MultiPolygon::new(Vec::new()),
            },
        }
    }

    /// The shape as one `geo` geometry: its polygons when it grows, and else its geometry.
    pub(crate) fn into_geometry(self) -> Option<Geometry<f32>> {
        match self {
            Self::Empty => None,
            Self::Grown { by, .. } if by > 0.0 => Some(self.polygons().into()),
            Self::Grown { geometry, .. } => Some(geometry.into_owned()),
        }
    }
}

/// `geometry` grown by `by`, with round edges within [`FLATTENING`] of the true ones.
fn buffered(geometry: &Geometry<f32>, by: f32) -> MultiPolygon<f32> {
    let angle = arc_angle(by);
    let style = BufferStyle::new(by)
        .line_cap(LineCap::Round(angle))
        .line_join(LineJoin::Round(angle));
    geometry.buffer_with_style(style)
}

/// The angle that each segment of a round edge of `radius` spans, so the segments stay within
/// [`FLATTENING`] of the arc, for radii up to 20,000 pixels.
fn arc_angle(radius: f32) -> f32 {
    if radius <= FLATTENING {
        return std::f32::consts::FRAC_PI_4;
    }
    (2.0 * (1.0 - FLATTENING / radius).acos()).clamp(0.002, std::f32::consts::FRAC_PI_4)
}

/// The areas of `collection`, which lyon gives for a filled path with degenerate parts.
pub(crate) fn areas(collection: &GeometryCollection<f32>) -> MultiPolygon<f32> {
    let mut polygons: Vec<Polygon<f32>> = Vec::new();
    for geometry in collection.iter() {
        match geometry {
            Geometry::Polygon(polygon) => polygons.push(polygon.clone()),
            Geometry::MultiPolygon(more) => polygons.extend(more.0.iter().cloned()),
            Geometry::Rect(rect) => polygons.push(rect.to_polygon()),
            Geometry::Triangle(triangle) => polygons.push(triangle.to_polygon()),
            _ => {}
        }
    }
    MultiPolygon::new(polygons)
}

/// A polygon from an exterior contour followed by its holes, each without its closing point.
pub(crate) fn polygon(shape: &[Vec<[f32; 2]>]) -> Polygon<f32> {
    let ring = |contour: &Vec<[f32; 2]>| {
        let mut coords: Vec<Coord<f32>> =
            contour.iter().map(|p| Coord { x: p[0], y: p[1] }).collect();
        if let Some(first) = coords.first().copied() {
            coords.push(first);
        }
        geo_types::LineString::new(coords)
    };
    let mut rings = shape.iter();
    let exterior = rings
        .next()
        .map(ring)
        .unwrap_or_else(|| geo_types::LineString::new(Vec::new()));
    Polygon::new(exterior, rings.map(ring).collect())
}
