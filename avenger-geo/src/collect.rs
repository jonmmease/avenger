//! Projected geometry collected from the pipeline's output stream.
//!
//! The stream carries points, lines, and polygon rings, not polygons: a cut or
//! a clip can split one polygon into several, and the rings of one polygon
//! block can belong to more than one. The pipeline gives exterior rings
//! positive signed area in output coordinates and holes negative, so the
//! collector assembles each block's polygons by sign, as d3-geo-projection's
//! `geoProject` does. A block holds the rings of one input polygon, so a hole
//! never joins another polygon's exterior.

use crate::error::AvengerGeoError;
use crate::stream::GeoSink;
use geo_traits::{GeometryTrait, GeometryType};
use geo_types::{
    Coord, Geometry, GeometryCollection, LineString, MultiLineString, MultiPoint, MultiPolygon,
    Point, Polygon,
};
use geoarrow_array::builder::{
    GeometryBuilder, MultiLineStringBuilder, MultiPointBuilder, MultiPolygonBuilder, PointBuilder,
};
use geoarrow_array::GeoArrowArray;
use geoarrow_schema::{
    CoordType, Dimension, GeoArrowType, GeometryType as MixedType, Metadata, MultiLineStringType,
    MultiPointType, MultiPolygonType, PointType,
};
use std::sync::Arc;

/// The kind of geometry that a geometry projects to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Point,
    MultiPoint,
    Lines,
    Polygons,
    Collection,
}

impl Kind {
    pub(crate) fn of(geometry: &impl GeometryTrait<T = f64>) -> Self {
        match geometry.as_type() {
            GeometryType::Point(_) => Kind::Point,
            GeometryType::MultiPoint(_) => Kind::MultiPoint,
            GeometryType::LineString(_)
            | GeometryType::MultiLineString(_)
            | GeometryType::Line(_) => Kind::Lines,
            GeometryType::Polygon(_)
            | GeometryType::MultiPolygon(_)
            | GeometryType::Rect(_)
            | GeometryType::Triangle(_) => Kind::Polygons,
            GeometryType::GeometryCollection(_) => Kind::Collection,
        }
    }
}

/// Collects a projected stream: points outside lines, lines outside polygons,
/// and polygons.
#[derive(Debug, Default)]
pub(crate) struct Collector {
    points: Vec<Coord<f64>>,
    lines: Vec<LineString<f64>>,
    polygons: Vec<Polygon<f64>>,
    /// The rings of the current polygon block.
    rings: Vec<Vec<Coord<f64>>>,
    current: Vec<Coord<f64>>,
    in_line: bool,
    in_polygon: bool,
}

impl Collector {
    /// The collected geometry as `kind`. A point that the clip left out is a
    /// point with NaN coordinates, which GeoArrow and WKB read as empty.
    pub(crate) fn into_geometry(self, kind: Kind) -> Geometry<f64> {
        let Collector {
            points,
            lines,
            polygons,
            ..
        } = self;
        match kind {
            Kind::Point => Geometry::Point(
                points
                    .first()
                    .map_or(Point::new(f64::NAN, f64::NAN), |coord| Point(*coord)),
            ),
            Kind::MultiPoint => Geometry::MultiPoint(multi_point(points)),
            Kind::Lines => Geometry::MultiLineString(MultiLineString(lines)),
            Kind::Polygons => Geometry::MultiPolygon(MultiPolygon(polygons)),
            Kind::Collection => {
                let mut parts = Vec::new();
                if !points.is_empty() {
                    parts.push(Geometry::MultiPoint(multi_point(points)));
                }
                if !lines.is_empty() {
                    parts.push(Geometry::MultiLineString(MultiLineString(lines)));
                }
                if !polygons.is_empty() {
                    parts.push(Geometry::MultiPolygon(MultiPolygon(polygons)));
                }
                Geometry::GeometryCollection(GeometryCollection(parts))
            }
        }
    }

    /// The collected polygons.
    pub(crate) fn into_multi_polygon(self) -> MultiPolygon<f64> {
        MultiPolygon(self.polygons)
    }

    /// Ends the current line piece, which needs two points.
    fn end_piece(&mut self) {
        let piece = std::mem::take(&mut self.current);
        if piece.len() >= 2 {
            self.lines.push(LineString(piece));
        }
    }
}

impl GeoSink for Collector {
    fn point(&mut self, x: f64, y: f64, _m: Option<f64>) {
        let finite = x.is_finite() && y.is_finite();
        match (self.in_line, self.in_polygon) {
            // A non-finite vertex breaks a line.
            (true, false) if !finite => self.end_piece(),
            // A ring leaves out its non-finite vertices.
            (true, _) if finite => self.current.push(Coord { x, y }),
            (false, _) if finite => self.points.push(Coord { x, y }),
            _ => {}
        }
    }

    fn line_start(&mut self) {
        self.in_line = true;
        self.current.clear();
    }

    fn line_end(&mut self) {
        self.in_line = false;
        if self.in_polygon {
            let ring = std::mem::take(&mut self.current);
            self.rings.push(ring);
        } else {
            self.end_piece();
        }
    }

    fn polygon_start(&mut self) {
        self.in_polygon = true;
    }

    fn polygon_end(&mut self) {
        self.in_polygon = false;
        let rings = std::mem::take(&mut self.rings);
        self.polygons.extend(assemble(rings));
    }
}

/// Builds the screen-space array for an input array's type: native, with
/// separated XY coordinates and no CRS.
pub(crate) enum ScreenBuilder {
    Point(PointBuilder),
    MultiPoint(MultiPointBuilder),
    Lines(MultiLineStringBuilder),
    Polygons(MultiPolygonBuilder),
    Mixed(Box<GeometryBuilder>),
}

impl ScreenBuilder {
    pub(crate) fn for_type(input: &GeoArrowType) -> Self {
        let metadata = Arc::new(Metadata::default());
        let (xy, separated) = (Dimension::XY, CoordType::Separated);
        match input {
            GeoArrowType::Point(_) => Self::Point(PointBuilder::new(
                PointType::new(xy, metadata).with_coord_type(separated),
            )),
            GeoArrowType::MultiPoint(_) => Self::MultiPoint(MultiPointBuilder::new(
                MultiPointType::new(xy, metadata).with_coord_type(separated),
            )),
            GeoArrowType::LineString(_) | GeoArrowType::MultiLineString(_) => {
                Self::Lines(MultiLineStringBuilder::new(
                    MultiLineStringType::new(xy, metadata).with_coord_type(separated),
                ))
            }
            GeoArrowType::Polygon(_) | GeoArrowType::MultiPolygon(_) | GeoArrowType::Rect(_) => {
                Self::Polygons(MultiPolygonBuilder::new(
                    MultiPolygonType::new(xy, metadata).with_coord_type(separated),
                ))
            }
            _ => Self::Mixed(Box::new(GeometryBuilder::new(
                MixedType::new(metadata).with_coord_type(separated),
            ))),
        }
    }

    /// Push a row, or a null row for `None`.
    pub(crate) fn push(&mut self, geometry: Option<&Geometry<f64>>) -> Result<(), AvengerGeoError> {
        match self {
            Self::Point(builder) => builder.push_geometry(geometry),
            Self::MultiPoint(builder) => builder.push_geometry(geometry),
            Self::Lines(builder) => builder.push_geometry(geometry),
            Self::Polygons(builder) => builder.push_geometry(geometry),
            Self::Mixed(builder) => builder.push_geometry(geometry),
        }
        .map_err(|err| AvengerGeoError::Arrow(err.to_string()))
    }

    pub(crate) fn finish(self) -> Arc<dyn GeoArrowArray> {
        match self {
            Self::Point(builder) => Arc::new(builder.finish()),
            Self::MultiPoint(builder) => Arc::new(builder.finish()),
            Self::Lines(builder) => Arc::new(builder.finish()),
            Self::Polygons(builder) => Arc::new(builder.finish()),
            Self::Mixed(builder) => Arc::new((*builder).finish()),
        }
    }
}

fn multi_point(points: Vec<Coord<f64>>) -> MultiPoint<f64> {
    MultiPoint(points.into_iter().map(Point).collect())
}

/// Polygons from rings: each ring with positive signed area is an exterior,
/// and each hole joins the smallest exterior that contains its first vertex.
/// Rings with fewer than three distinct vertices or no area are left out, and
/// so is a hole that no exterior contains.
fn assemble(rings: Vec<Vec<Coord<f64>>>) -> Vec<Polygon<f64>> {
    struct Exterior {
        ring: Vec<Coord<f64>>,
        area: f64,
        holes: Vec<LineString<f64>>,
    }
    let mut exteriors = Vec::new();
    let mut holes = Vec::new();
    for ring in rings {
        let ring = open_ring(ring);
        if ring.len() < 3 {
            continue;
        }
        let area = signed_area(&ring);
        if area > 0.0 {
            exteriors.push(Exterior {
                ring,
                area,
                holes: Vec::new(),
            });
        } else if area < 0.0 {
            holes.push(ring);
        }
    }
    for hole in holes {
        let smallest = exteriors
            .iter_mut()
            .filter(|exterior| contains(&exterior.ring, hole[0]))
            .min_by(|a, b| a.area.total_cmp(&b.area));
        if let Some(exterior) = smallest {
            exterior.holes.push(LineString(hole));
        }
    }
    exteriors
        .into_iter()
        .map(|exterior| Polygon::new(LineString(exterior.ring), exterior.holes))
        .collect()
}

/// The ring without repeated consecutive vertices or a closing vertex.
fn open_ring(mut ring: Vec<Coord<f64>>) -> Vec<Coord<f64>> {
    ring.dedup();
    if ring.len() > 1 && ring.first() == ring.last() {
        ring.pop();
    }
    ring
}

/// The ring's signed area, with its closing edge implied: positive when it
/// winds counter-clockwise with y up, which is clockwise on a y-down screen.
fn signed_area(ring: &[Coord<f64>]) -> f64 {
    let origin = ring[0];
    // Shifting to the first vertex avoids cancellation for small, distant rings.
    let doubled: f64 = (0..ring.len())
        .map(|i| {
            let (a, b) = (ring[i] - origin, ring[(i + 1) % ring.len()] - origin);
            a.x * b.y - b.x * a.y
        })
        .sum();
    doubled / 2.0
}

/// Whether `point` lies inside the ring, by the even-odd rule.
fn contains(ring: &[Coord<f64>], point: Coord<f64>) -> bool {
    let mut inside = false;
    let mut previous = ring[ring.len() - 1];
    for &vertex in ring {
        if (vertex.y > point.y) != (previous.y > point.y)
            && point.x
                < (previous.x - vertex.x) * (point.y - vertex.y) / (previous.y - vertex.y)
                    + vertex.x
        {
            inside = !inside;
        }
        previous = vertex;
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring(sink: &mut Collector, points: &[(f64, f64)]) {
        sink.line_start();
        for &(x, y) in points {
            sink.point(x, y, None);
        }
        sink.line_end();
    }

    const SQUARE: [(f64, f64); 4] = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];

    #[test]
    fn rings_assemble_into_polygons_by_sign() {
        let mut sink = Collector::default();
        sink.polygon_start();
        ring(&mut sink, &SQUARE);
        ring(
            &mut sink,
            &[(20.0, 0.0), (30.0, 0.0), (30.0, 10.0), (20.0, 10.0)],
        );
        // A hole in the first square, and one in no square.
        ring(&mut sink, &[(2.0, 2.0), (2.0, 4.0), (4.0, 4.0), (4.0, 2.0)]);
        ring(
            &mut sink,
            &[(50.0, 2.0), (50.0, 4.0), (52.0, 4.0), (52.0, 2.0)],
        );
        // A ring without area.
        ring(&mut sink, &[(0.0, 0.0), (1.0, 1.0), (2.0, 2.0)]);
        sink.polygon_end();
        let MultiPolygon(polygons) = sink.into_multi_polygon();
        assert_eq!(polygons.len(), 2);
        assert_eq!(polygons[0].interiors().len(), 1);
        assert!(polygons[1].interiors().is_empty());
        // Rings close.
        let exterior = &polygons[0].exterior().0;
        assert_eq!(exterior.first(), exterior.last());
    }

    #[test]
    fn a_hole_joins_the_smallest_exterior_around_it() {
        let mut sink = Collector::default();
        sink.polygon_start();
        ring(
            &mut sink,
            &[(-10.0, -10.0), (20.0, -10.0), (20.0, 20.0), (-10.0, 20.0)],
        );
        ring(&mut sink, &SQUARE);
        ring(&mut sink, &[(2.0, 2.0), (2.0, 4.0), (4.0, 4.0), (4.0, 2.0)]);
        sink.polygon_end();
        let MultiPolygon(polygons) = sink.into_multi_polygon();
        assert!(polygons[0].interiors().is_empty());
        assert_eq!(polygons[1].interiors().len(), 1);
    }

    /// A hole stays with its own polygon when another polygon's exterior is
    /// smaller and around it too.
    #[test]
    fn a_hole_stays_in_its_polygon_block() {
        let mut sink = Collector::default();
        sink.polygon_start();
        ring(
            &mut sink,
            &[(-10.0, -10.0), (20.0, -10.0), (20.0, 20.0), (-10.0, 20.0)],
        );
        ring(&mut sink, &[(2.0, 2.0), (2.0, 4.0), (4.0, 4.0), (4.0, 2.0)]);
        sink.polygon_end();
        sink.polygon_start();
        ring(&mut sink, &SQUARE);
        sink.polygon_end();
        let MultiPolygon(polygons) = sink.into_multi_polygon();
        assert_eq!(polygons[0].interiors().len(), 1);
        assert!(polygons[1].interiors().is_empty());
    }

    #[test]
    fn lines_split_at_non_finite_vertices() {
        let mut sink = Collector::default();
        ring(
            &mut sink,
            &[
                (0.0, 0.0),
                (1.0, 1.0),
                (f64::NAN, 2.0),
                (3.0, 3.0),
                (4.0, 4.0),
                (f64::INFINITY, 0.0),
                (5.0, 5.0),
            ],
        );
        sink.point(7.0, 8.0, None);
        let Geometry::GeometryCollection(GeometryCollection(parts)) =
            sink.into_geometry(Kind::Collection)
        else {
            panic!("a collection");
        };
        let [Geometry::MultiPoint(points), Geometry::MultiLineString(lines)] = &parts[..] else {
            panic!("points and lines: {parts:?}");
        };
        assert_eq!(points.0.len(), 1);
        // The last piece has one point, which isn't a line.
        assert_eq!(lines.0.len(), 2);
    }

    #[test]
    fn a_point_that_the_clip_leaves_out_is_empty() {
        let Geometry::Point(point) = Collector::default().into_geometry(Kind::Point) else {
            panic!("a point");
        };
        assert!(point.x().is_nan() && point.y().is_nan());
    }
}
