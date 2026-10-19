//! Feeding geo-traits geometry into a [`GeoSink`].
//!
//! Mirrors d3-geo `src/stream.js` (ISC): polygon rings are streamed with the
//! closing (duplicate) point omitted; clip stages re-close rings.

use crate::stream::GeoSink;
use geo_traits::{
    CoordTrait, GeometryCollectionTrait, GeometryTrait, GeometryType, LineStringTrait, LineTrait,
    MultiLineStringTrait, MultiPointTrait, MultiPolygonTrait, PointTrait, PolygonTrait, RectTrait,
    TriangleTrait,
};

/// How polygon rings are streamed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rings {
    /// As written, which RFC 7946 winds: counter-clockwise exteriors and
    /// clockwise holes.
    AsWritten,
    /// Rewound to RFC 7946 by planar signed area, with zero-area rings
    /// dropped. A polygon whose exterior has no area is dropped.
    Rewound,
}

/// Stream `geometry` into `sink`.
pub(crate) fn walk(geometry: &impl GeometryTrait<T = f64>, sink: &mut dyn GeoSink, rings: Rings) {
    match geometry.as_type() {
        GeometryType::Point(point) => {
            if let Some(coord) = point.coord() {
                sink.point(coord.x(), coord.y(), None);
            }
        }
        GeometryType::MultiPoint(multi) => {
            for point in multi.points() {
                if let Some(coord) = point.coord() {
                    sink.point(coord.x(), coord.y(), None);
                }
            }
        }
        GeometryType::LineString(line) => walk_line(line, sink),
        GeometryType::MultiLineString(multi) => {
            for line in multi.line_strings() {
                walk_line(&line, sink);
            }
        }
        GeometryType::Polygon(polygon) => walk_polygon(polygon, sink, rings),
        GeometryType::MultiPolygon(multi) => {
            for polygon in multi.polygons() {
                walk_polygon(&polygon, sink, rings);
            }
        }
        GeometryType::GeometryCollection(collection) => {
            for geometry in collection.geometries() {
                walk(&geometry, sink, rings);
            }
        }
        GeometryType::Line(line) => {
            sink.line_start();
            sink.point(line.start().x(), line.start().y(), None);
            sink.point(line.end().x(), line.end().y(), None);
            sink.line_end();
        }
        GeometryType::Rect(rect) => {
            let (min, max) = (rect.min(), rect.max());
            sink.polygon_start();
            sink.line_start();
            // Counter-clockwise, as RFC 7946 winds exteriors.
            sink.point(min.x(), min.y(), None);
            sink.point(max.x(), min.y(), None);
            sink.point(max.x(), max.y(), None);
            sink.point(min.x(), max.y(), None);
            sink.line_end();
            sink.polygon_end();
        }
        GeometryType::Triangle(triangle) => {
            sink.polygon_start();
            sink.line_start();
            for coord in [triangle.first(), triangle.second(), triangle.third()] {
                sink.point(coord.x(), coord.y(), None);
            }
            sink.line_end();
            sink.polygon_end();
        }
    }
}

fn walk_line(line: &impl LineStringTrait<T = f64>, sink: &mut dyn GeoSink) {
    sink.line_start();
    for coord in line.coords() {
        sink.point(coord.x(), coord.y(), None);
    }
    sink.line_end();
}

fn walk_polygon(polygon: &impl PolygonTrait<T = f64>, sink: &mut dyn GeoSink, rings: Rings) {
    let Some(exterior) = polygon.exterior() else {
        return;
    };
    match rings {
        Rings::AsWritten => {
            sink.polygon_start();
            walk_ring(&exterior, sink, false);
            for interior in polygon.interiors() {
                walk_ring(&interior, sink, false);
            }
            sink.polygon_end();
        }
        Rings::Rewound => {
            let area = ring_signed_area2(&exterior);
            if !has_area(&exterior, area) {
                return;
            }
            sink.polygon_start();
            walk_ring(&exterior, sink, area < 0.0);
            for interior in polygon.interiors() {
                let area = ring_signed_area2(&interior);
                if has_area(&interior, area) {
                    walk_ring(&interior, sink, area > 0.0);
                }
            }
            sink.polygon_end();
        }
    }
}

/// Stream a ring without its closing point, when it repeats the first, in
/// reverse when `reverse`. A reversed ring keeps its first point first, as the
/// pipeline's reversal does, so a ring that the pipeline reverses back streams
/// as written. Where a ring runs along the antimeridian, the clip's result
/// depends on where the ring starts.
fn walk_ring(ring: &impl LineStringTrait<T = f64>, sink: &mut dyn GeoSink, reverse: bool) {
    let n = open_len(ring);
    let point = |sink: &mut dyn GeoSink, i: usize| {
        let coord = ring.coord(i).expect("coord in range");
        sink.point(coord.x(), coord.y(), None);
    };
    sink.line_start();
    if reverse && n > 0 {
        point(sink, 0);
        for i in (1..n).rev() {
            point(sink, i);
        }
    } else {
        for i in 0..n {
            point(sink, i);
        }
    }
    sink.line_end();
}

/// The number of a ring's coordinates without its closing point, when the
/// last coordinate repeats the first.
fn open_len(ring: &impl LineStringTrait<T = f64>) -> usize {
    let n = ring.num_coords();
    let closed = n > 1
        && match (ring.coord(0), ring.coord(n - 1)) {
            (Some(first), Some(last)) => first.x_y() == last.x_y(),
            _ => false,
        };
    if closed {
        n - 1
    } else {
        n
    }
}

/// Whether a ring with twice the signed area `area2` has three or more
/// distinct vertices and nonzero area. Such rings would make spherical
/// clipping emit the ring's complement.
pub(crate) fn has_area(ring: &impl LineStringTrait<T = f64>, area2: f64) -> bool {
    open_len(ring) >= 3 && area2 != 0.0
}

/// Twice a ring's planar signed area, positive when it winds counter-clockwise
/// with y up. The closing edge is implied when the ring isn't closed.
pub(crate) fn ring_signed_area2(ring: &impl LineStringTrait<T = f64>) -> f64 {
    let n = open_len(ring);
    let Some(origin) = ring.coord(0).map(|c| c.x_y()) else {
        return 0.0;
    };
    // Shifting to the first vertex avoids cancellation for small, distant rings.
    let shifted = |i: usize| {
        let (x, y) = ring.coord(i % n).expect("coord in range").x_y();
        (x - origin.0, y - origin.1)
    };
    (0..n)
        .map(|i| {
            let (a, b) = (shifted(i), shifted(i + 1));
            a.0 * b.1 - b.0 * a.1
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::{RecordingSink, StreamEvent};
    use geo_types::{LineString, Polygon};

    /// Rewinding reverses a clockwise exterior and keeps its first point first.
    #[test]
    fn rewound_rings_keep_their_first_point_first() {
        let clockwise = Polygon::new(
            LineString::from(vec![
                (0.0, 0.0),
                (0.0, 1.0),
                (1.0, 1.0),
                (1.0, 0.0),
                (0.0, 0.0),
            ]),
            vec![],
        );
        let mut sink = RecordingSink::default();
        walk(&clockwise, &mut sink, Rings::Rewound);
        let points: Vec<(f64, f64)> = sink
            .events
            .iter()
            .filter_map(|event| match event {
                StreamEvent::Point(x, y) => Some((*x, *y)),
                _ => None,
            })
            .collect();
        assert_eq!(points, [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]);
    }
}
