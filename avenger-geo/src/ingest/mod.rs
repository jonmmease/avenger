//! Parse GeoJSON into ISO WKB geometry, recomputed longitude/latitude bounds,
//! feature IDs, and JSON properties. Ring winding is normalized to the d3
//! spherical convention: clockwise exteriors and counter-clockwise holes, measured
//! by planar signed area. Already spherical or polar geometry should use
//! [`crate::Streamable`] directly to preserve its authored winding.
//!
//! [`WkbStreamable`] validates WKB once and streams coordinates from its
//! borrowed buffer via geo-traits without materializing geo-types.

use crate::error::AvengerGeoError;
use crate::stream::GeoStream;
use crate::streamable::Streamable;
use geo::algorithm::winding_order::Winding;
use geo_traits::{
    GeometryCollectionTrait, GeometryTrait, GeometryType, LineStringTrait, LineTrait,
    MultiLineStringTrait, MultiPointTrait, MultiPolygonTrait, PointTrait, PolygonTrait, RectTrait,
    TriangleTrait,
};
use geo_types::Geometry;
use serde_json::Value as JsonValue;

/// One ingested GeoJSON feature.
#[derive(Debug, Clone, PartialEq)]
pub struct GeoFeature {
    /// Top-level GeoJSON identifier, separate from any property named `id`.
    pub id: Option<geojson::feature::Id>,
    /// ISO WKB bytes (None for null geometry).
    pub wkb: Option<Vec<u8>>,
    /// Lon/lat bounds recomputed from coordinates (None for null/empty
    /// geometry).
    pub bbox: Option<[f64; 4]>,
    /// Feature properties (JSON object; empty map when absent).
    pub properties: serde_json::Map<String, JsonValue>,
}

/// Parse a GeoJSON string (FeatureCollection, Feature, or bare geometry)
/// into ingested features.
pub fn geojson_to_features(json: &str) -> Result<Vec<GeoFeature>, AvengerGeoError> {
    let parsed: geojson::GeoJson = json.parse()?;
    let mut features = Vec::new();
    match parsed {
        geojson::GeoJson::FeatureCollection(collection) => {
            for feature in collection.features {
                features.push(ingest_feature(feature)?);
            }
        }
        geojson::GeoJson::Feature(feature) => features.push(ingest_feature(feature)?),
        geojson::GeoJson::Geometry(geometry) => features.push(ingest_geometry(
            Some(geometry),
            None,
            serde_json::Map::new(),
        )?),
    }
    Ok(features)
}

fn ingest_feature(feature: geojson::Feature) -> Result<GeoFeature, AvengerGeoError> {
    let properties = feature.properties.unwrap_or_default();
    ingest_geometry(feature.geometry, feature.id, properties)
}

fn ingest_geometry(
    geometry: Option<geojson::Geometry>,
    id: Option<geojson::feature::Id>,
    properties: serde_json::Map<String, JsonValue>,
) -> Result<GeoFeature, AvengerGeoError> {
    let Some(geometry) = geometry else {
        return Ok(GeoFeature {
            id,
            wkb: None,
            bbox: None,
            properties,
        });
    };
    let mut geometry: Geometry<f64> = geometry
        .try_into()
        .map_err(|err: geojson::Error| AvengerGeoError::GeoJson(err))?;
    drop_degenerate_rings(&mut geometry);
    rewind_spherical(&mut geometry);
    let bbox = lonlat_bbox(&geometry);
    let mut wkb_bytes = Vec::new();
    wkb::writer::write_geometry(&mut wkb_bytes, &geometry, &Default::default())
        .map_err(|err| AvengerGeoError::Wkb(err.to_string()))?;
    Ok(GeoFeature {
        id,
        wkb: Some(wkb_bytes),
        bbox,
        properties,
    })
}

/// Remove rings with too few vertices or zero planar area. Such rings can
/// make spherical clipping emit the ring's complement. Nonzero rings are
/// retained regardless of their size.
pub fn drop_degenerate_rings(geometry: &mut Geometry<f64>) {
    fn has_area(ring: &geo_types::LineString<f64>) -> bool {
        let coords = &ring.0;
        if coords.len() < 4 {
            return false;
        }
        // Shifting to the first vertex avoids cancellation for small, distant rings.
        let origin = coords[0];
        let mut area = 0.0;
        for pair in coords.windows(2) {
            let a = pair[0] - origin;
            let b = pair[1] - origin;
            area += a.x * b.y - b.x * a.y;
        }
        area != 0.0
    }
    fn sanitize_polygon(polygon: &geo_types::Polygon<f64>) -> Option<geo_types::Polygon<f64>> {
        if !has_area(polygon.exterior()) {
            return None;
        }
        let interiors: Vec<_> = polygon
            .interiors()
            .iter()
            .filter(|ring| has_area(ring))
            .cloned()
            .collect();
        Some(geo_types::Polygon::new(
            polygon.exterior().clone(),
            interiors,
        ))
    }
    match geometry {
        Geometry::Polygon(polygon) => {
            if let Some(sanitized) = sanitize_polygon(polygon) {
                *polygon = sanitized;
            } else {
                *geometry = Geometry::MultiPolygon(geo_types::MultiPolygon(Vec::new()));
            }
        }
        Geometry::MultiPolygon(multi) => {
            multi.0 = multi.0.iter().filter_map(sanitize_polygon).collect();
        }
        Geometry::GeometryCollection(collection) => {
            for geometry in &mut collection.0 {
                drop_degenerate_rings(geometry);
            }
        }
        _ => {}
    }
}

/// Normalize ring winding to the d3 spherical convention: exterior rings
/// planar-clockwise, holes counter-clockwise (the opposite of RFC 7946).
pub fn rewind_spherical(geometry: &mut Geometry<f64>) {
    match geometry {
        Geometry::Polygon(polygon) => rewind_polygon(polygon),
        Geometry::MultiPolygon(multi) => {
            for polygon in &mut multi.0 {
                rewind_polygon(polygon);
            }
        }
        Geometry::GeometryCollection(collection) => {
            for geometry in &mut collection.0 {
                rewind_spherical(geometry);
            }
        }
        _ => {}
    }
}

fn rewind_polygon(polygon: &mut geo_types::Polygon<f64>) {
    polygon.exterior_mut(|ring| ring.make_cw_winding());
    polygon.interiors_mut(|rings| {
        for ring in rings {
            ring.make_ccw_winding();
        }
    });
}

/// Lon/lat bounds from raw coordinates (ignores any GeoJSON bbox member).
pub fn lonlat_bbox(geometry: &Geometry<f64>) -> Option<[f64; 4]> {
    use geo::algorithm::bounding_rect::BoundingRect;
    geometry
        .bounding_rect()
        .map(|rect| [rect.min().x, rect.min().y, rect.max().x, rect.max().y])
}

/// Validate WKB bytes and stream their coordinates into a [`GeoStream`].
/// Polygon rings are streamed with the closing point omitted, matching
/// [`crate::streamable`] semantics.
pub fn wkb_streamer(wkb_bytes: &[u8], sink: &mut dyn GeoStream) -> Result<(), AvengerGeoError> {
    WkbStreamable::new(wkb_bytes)?.stream(sink);
    Ok(())
}

/// A validated WKB geometry that borrows its coordinate buffer.
/// Reuse this source for fitting and projection without parsing it again.
pub struct WkbStreamable<'a>(wkb::reader::Wkb<'a>);

impl<'a> WkbStreamable<'a> {
    /// Parse and validate WKB, returning an error before any geometry is streamed.
    pub fn new(bytes: &'a [u8]) -> Result<Self, AvengerGeoError> {
        wkb::reader::read_wkb(bytes)
            .map(Self)
            .map_err(|err| AvengerGeoError::Wkb(err.to_string()))
    }
}

impl Streamable for WkbStreamable<'_> {
    fn stream(&self, sink: &mut dyn GeoStream) {
        stream_geometry_trait(&self.0, sink);
    }
}

/// Stream WKB geometry through a projection pipeline into `sink`,
/// validating the bytes first.
pub fn stream_wkb_through(
    projector: &crate::projector::Projector,
    wkb_bytes: &[u8],
    sink: &mut dyn GeoStream,
) -> Result<(), AvengerGeoError> {
    projector.stream(&WkbStreamable::new(wkb_bytes)?, sink);
    Ok(())
}

fn stream_geometry_trait(geometry: &impl GeometryTrait<T = f64>, sink: &mut dyn GeoStream) {
    match geometry.as_type() {
        GeometryType::Point(point) => {
            if let Some(coord) = point.coord() {
                use geo_traits::CoordTrait;
                sink.point(coord.x(), coord.y(), None);
            }
        }
        GeometryType::MultiPoint(multi) => {
            for point in multi.points() {
                if let Some(coord) = point.coord() {
                    use geo_traits::CoordTrait;
                    sink.point(coord.x(), coord.y(), None);
                }
            }
        }
        GeometryType::LineString(line) => stream_line_string(line, sink, false),
        GeometryType::MultiLineString(multi) => {
            for line in multi.line_strings() {
                stream_line_string(&line, sink, false);
            }
        }
        GeometryType::Polygon(polygon) => stream_polygon(polygon, sink),
        GeometryType::MultiPolygon(multi) => {
            for polygon in multi.polygons() {
                stream_polygon(&polygon, sink);
            }
        }
        GeometryType::GeometryCollection(collection) => {
            for geometry in collection.geometries() {
                stream_geometry_trait(&geometry, sink);
            }
        }
        GeometryType::Line(line) => {
            use geo_traits::CoordTrait;
            sink.line_start();
            sink.point(line.start().x(), line.start().y(), None);
            sink.point(line.end().x(), line.end().y(), None);
            sink.line_end();
        }
        GeometryType::Rect(rect) => {
            use geo_traits::CoordTrait;
            let (min, max) = (rect.min(), rect.max());
            sink.polygon_start();
            sink.line_start();
            sink.point(min.x(), min.y(), None);
            sink.point(min.x(), max.y(), None);
            sink.point(max.x(), max.y(), None);
            sink.point(max.x(), min.y(), None);
            sink.line_end();
            sink.polygon_end();
        }
        GeometryType::Triangle(triangle) => {
            use geo_traits::CoordTrait;
            sink.polygon_start();
            sink.line_start();
            sink.point(triangle.first().x(), triangle.first().y(), None);
            sink.point(triangle.second().x(), triangle.second().y(), None);
            sink.point(triangle.third().x(), triangle.third().y(), None);
            sink.line_end();
            sink.polygon_end();
        }
    }
}

fn stream_line_string(
    line: &impl LineStringTrait<T = f64>,
    sink: &mut dyn GeoStream,
    closed: bool,
) {
    use geo_traits::CoordTrait;
    let n = line.num_coords();
    let last = if closed && n > 1 { n - 1 } else { n };
    sink.line_start();
    for i in 0..last {
        let coord = line.coord(i).expect("coord in range");
        sink.point(coord.x(), coord.y(), None);
    }
    sink.line_end();
}

fn stream_polygon(polygon: &impl PolygonTrait<T = f64>, sink: &mut dyn GeoStream) {
    sink.polygon_start();
    if let Some(exterior) = polygon.exterior() {
        stream_line_string(&exterior, sink, true);
    }
    for interior in polygon.interiors() {
        stream_line_string(&interior, sink, true);
    }
    sink.polygon_end();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::{RecordingSink, StreamEvent};

    #[test]
    fn parses_feature_collection_with_properties_and_bbox() {
        let json = r#"{
            "type": "FeatureCollection",
            "features": [
                {
                    "type": "Feature",
                    "id": "region-a",
                    "properties": {"id": "property-id", "name": "box", "value": 3.5},
                    "geometry": {
                        "type": "Polygon",
                        "coordinates": [[[0,0],[10,0],[10,10],[0,10],[0,0]]]
                    }
                },
                {
                    "type": "Feature",
                    "id": 7,
                    "properties": {"name": "empty"},
                    "geometry": null
                }
            ]
        }"#;
        let features = geojson_to_features(json).expect("parse");
        assert_eq!(features.len(), 2);
        assert_eq!(
            features[0].id,
            Some(geojson::feature::Id::String("region-a".into()))
        );
        assert_eq!(features[1].id, Some(geojson::feature::Id::Number(7.into())));
        assert_eq!(features[0].properties["id"], "property-id");
        assert_eq!(features[0].bbox, Some([0.0, 0.0, 10.0, 10.0]));
        assert_eq!(
            features[0].properties.get("name"),
            Some(&JsonValue::String("box".to_string()))
        );
        assert!(features[0].wkb.is_some());
        assert!(features[1].wkb.is_none() && features[1].bbox.is_none());
    }

    #[test]
    fn retains_small_rings_and_removes_degenerate_rings() {
        for (x, y) in [(0.0, 0.0), (120.0, 45.0)] {
            let outer = vec![
                [x, y],
                [x + 5e-6, y],
                [x + 5e-6, y + 5e-6],
                [x, y + 5e-6],
                [x, y],
            ];
            let hole = vec![
                [x + 1e-6, y + 1e-6],
                [x + 1e-6, y + 2e-6],
                [x + 2e-6, y + 2e-6],
                [x + 2e-6, y + 1e-6],
                [x + 1e-6, y + 1e-6],
            ];
            let degenerate = vec![[x, y], [x + 1e-6, y], [x + 2e-6, y], [x, y]];
            let json = serde_json::json!({
                "type": "GeometryCollection",
                "geometries": [
                    {"type": "Polygon", "coordinates": [outer, hole, degenerate]},
                    {"type": "MultiPolygon", "coordinates": [[degenerate]]}
                ]
            });
            let features = geojson_to_features(&json.to_string()).unwrap();
            assert_eq!(features[0].id, None);
            assert_eq!(features[0].bbox, Some([x, y, x + 5e-6, y + 5e-6]));
            let mut sink = RecordingSink::default();
            wkb_streamer(features[0].wkb.as_ref().unwrap(), &mut sink).unwrap();
            let rings = sink.polylines();
            assert_eq!(rings.len(), 2);
            for (ring, source) in rings.iter().zip([&outer, &hole]) {
                assert_eq!(ring.len(), 4);
                for point in &source[..4] {
                    assert!(ring.contains(point), "missing {point:?}");
                }
            }
        }
    }

    #[test]
    fn wkb_sources_are_reusable_and_reject_invalid_bytes_before_streaming() {
        let features =
            geojson_to_features(r#"{"type":"LineString","coordinates":[[0,0],[10,20]]}"#).unwrap();
        let bytes = features[0].wkb.as_ref().unwrap();
        let source = WkbStreamable::new(bytes).unwrap();
        let projector = crate::Projection::new(crate::ProjectionKind::Equirectangular).build();
        let mut direct = RecordingSink::default();
        stream_wkb_through(&projector, bytes, &mut direct).unwrap();
        for _ in 0..2 {
            let mut reused = RecordingSink::default();
            projector.stream(&source, &mut reused);
            assert_eq!(reused, direct);
        }
        let before = direct.clone();
        let truncated = &bytes[..bytes.len() - 1];
        assert!(WkbStreamable::new(truncated).is_err());
        assert!(wkb_streamer(truncated, &mut direct).is_err());
        assert!(stream_wkb_through(&projector, truncated, &mut direct).is_err());
        assert_eq!(direct, before);
    }

    #[test]
    fn rewinds_rfc7946_exteriors_to_spherical_clockwise() {
        // RFC 7946 counter-clockwise exterior ring.
        let json = r#"{
            "type": "Polygon",
            "coordinates": [[[0,0],[10,0],[10,10],[0,10],[0,0]]]
        }"#;
        let features = geojson_to_features(json).expect("parse");
        let wkb_bytes = features[0].wkb.as_ref().unwrap();

        // Verify winding via the streamed ring order (negative planar
        // signed area = clockwise).
        let mut sink = RecordingSink::default();
        wkb_streamer(wkb_bytes, &mut sink).unwrap();
        let ring: Vec<[f64; 2]> = sink
            .events
            .iter()
            .filter_map(|e| match e {
                StreamEvent::Point(x, y) => Some([*x, *y]),
                _ => None,
            })
            .collect();
        // Planar shoelace over the open ring (closing edge implied).
        let mut area = 0.0;
        for i in 0..ring.len() {
            let a = ring[i];
            let b = ring[(i + 1) % ring.len()];
            area += a[0] * b[1] - b[0] * a[1];
        }
        assert!(area < 0.0, "exterior must be clockwise, area {area}");
    }

    #[test]
    fn wkb_streams_polygon_without_closing_point() {
        let json = r#"{
            "type": "Polygon",
            "coordinates": [[[0,0],[0,10],[10,10],[10,0],[0,0]]]
        }"#;
        let features = geojson_to_features(json).expect("parse");
        let mut sink = RecordingSink::default();
        wkb_streamer(features[0].wkb.as_ref().unwrap(), &mut sink).unwrap();
        let points = sink
            .events
            .iter()
            .filter(|e| matches!(e, StreamEvent::Point(..)))
            .count();
        assert_eq!(points, 4, "closing point omitted");
        assert!(sink
            .events
            .iter()
            .any(|e| matches!(e, StreamEvent::PolygonStart)));
    }
}
