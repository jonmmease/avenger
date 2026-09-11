//! Feeding geometry into a [`GeoStream`].
//!
//! Mirrors d3-geo `src/stream.js` (ISC): polygon rings are streamed with the
//! closing (duplicate) point omitted; clip stages re-close rings.

use crate::stream::GeoStream;
use geo_types::{Geometry, LineString, MultiLineString, MultiPoint, MultiPolygon, Point, Polygon};

pub trait Streamable {
    fn stream(&self, sink: &mut dyn GeoStream);
}

/// The whole globe (d3's `{type: "Sphere"}`): projects to the projection's
/// outline via the clip stage's boundary interpolation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sphere;

impl Streamable for Sphere {
    fn stream(&self, sink: &mut dyn GeoStream) {
        sink.sphere();
    }
}

fn stream_line(points: &[(f64, f64)], sink: &mut dyn GeoStream, closed: bool) {
    let n = if closed {
        points.len().saturating_sub(1)
    } else {
        points.len()
    };
    sink.line_start();
    for p in &points[..n] {
        sink.point(p.0, p.1, None);
    }
    sink.line_end();
}

fn coords(ls: &LineString<f64>) -> Vec<(f64, f64)> {
    ls.0.iter().map(|c| (c.x, c.y)).collect()
}

fn stream_polygon(poly: &Polygon<f64>, sink: &mut dyn GeoStream) {
    sink.polygon_start();
    stream_line(&coords(poly.exterior()), sink, true);
    for interior in poly.interiors() {
        stream_line(&coords(interior), sink, true);
    }
    sink.polygon_end();
}

impl Streamable for Point<f64> {
    fn stream(&self, sink: &mut dyn GeoStream) {
        sink.point(self.x(), self.y(), None);
    }
}

impl Streamable for MultiPoint<f64> {
    fn stream(&self, sink: &mut dyn GeoStream) {
        for p in &self.0 {
            sink.point(p.x(), p.y(), None);
        }
    }
}

impl Streamable for LineString<f64> {
    fn stream(&self, sink: &mut dyn GeoStream) {
        stream_line(&coords(self), sink, false);
    }
}

impl Streamable for MultiLineString<f64> {
    fn stream(&self, sink: &mut dyn GeoStream) {
        for ls in &self.0 {
            stream_line(&coords(ls), sink, false);
        }
    }
}

impl Streamable for Polygon<f64> {
    fn stream(&self, sink: &mut dyn GeoStream) {
        stream_polygon(self, sink);
    }
}

impl Streamable for MultiPolygon<f64> {
    fn stream(&self, sink: &mut dyn GeoStream) {
        for poly in &self.0 {
            stream_polygon(poly, sink);
        }
    }
}

impl Streamable for Geometry<f64> {
    fn stream(&self, sink: &mut dyn GeoStream) {
        match self {
            Geometry::Point(g) => g.stream(sink),
            Geometry::MultiPoint(g) => g.stream(sink),
            Geometry::Line(g) => {
                stream_line(&[(g.start.x, g.start.y), (g.end.x, g.end.y)], sink, false)
            }
            Geometry::LineString(g) => g.stream(sink),
            Geometry::MultiLineString(g) => g.stream(sink),
            Geometry::Polygon(g) => g.stream(sink),
            Geometry::MultiPolygon(g) => g.stream(sink),
            Geometry::GeometryCollection(gc) => {
                for g in gc {
                    g.stream(sink);
                }
            }
            Geometry::Rect(r) => stream_polygon(&r.to_polygon(), sink),
            Geometry::Triangle(t) => stream_polygon(&t.to_polygon(), sink),
        }
    }
}

/// A bare multi-polyline (used by graticules).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MultiLine(pub Vec<Vec<[f64; 2]>>);

impl Streamable for MultiLine {
    fn stream(&self, sink: &mut dyn GeoStream) {
        for line in &self.0 {
            sink.line_start();
            for p in line {
                sink.point(p[0], p[1], None);
            }
            sink.line_end();
        }
    }
}

/// A collection of streamables streamed in sequence (for fit over multiple
/// layers).
pub struct StreamableSeq<'a>(pub Vec<&'a dyn Streamable>);

impl Streamable for StreamableSeq<'_> {
    fn stream(&self, sink: &mut dyn GeoStream) {
        for s in &self.0 {
            s.stream(sink);
        }
    }
}
