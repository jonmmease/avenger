//! Parse GeoJSON into ISO WKB geometry, recomputed longitude/latitude bounds,
//! feature IDs, and JSON properties. Ring winding is normalized to RFC 7946:
//! counter-clockwise exteriors and clockwise holes, measured by planar signed
//! area, which repairs files that wind rings the other way. Geometry whose
//! winding is authored on the sphere, such as a polar cap or a ring larger
//! than a hemisphere, should be passed to the projector directly.
//!
//! A feature's WKB reads with `wkb::reader::read_wkb`, whose result the
//! projector takes as it is. [`geojson_to_record_batch`] loads the same
//! features as an Arrow record batch.

use crate::error::AvengerGeoError;
use crate::walk::{has_area as ring_has_area, ring_signed_area2};
use arrow::array::{Array, ArrayRef, AsArray, BooleanArray, Float64Array, StringArray};
use arrow::buffer::NullBuffer;
use arrow::compute::cast;
use arrow::datatypes::{DataType, Field, Float64Type, Schema};
use arrow::record_batch::RecordBatch;
use geo_types::Geometry;
use geoarrow_array::array::{CoordBuffer, PointArray, SeparatedCoordBuffer};
use geoarrow_array::builder::WkbBuilder;
use geoarrow_array::GeoArrowArray;
use geoarrow_schema::{Crs, Dimension, Metadata, WkbType};
use serde_json::Value as JsonValue;
use std::collections::BTreeSet;
use std::sync::Arc;

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

/// Points from longitude and latitude columns in degrees, as a GeoArrow point
/// array with CRS `OGC:CRS84`, for tables that keep coordinates in two
/// columns. A row missing either value is null. Float64 columns share their
/// values with the array, and other numeric columns are cast to Float64.
///
/// # Errors
/// Returns an error when either column isn't numeric or their lengths differ.
pub fn lon_lat_points(lon: &ArrayRef, lat: &ArrayRef) -> Result<PointArray, AvengerGeoError> {
    if lon.len() != lat.len() {
        return Err(AvengerGeoError::Arrow(format!(
            "longitude and latitude have different lengths: {} and {}",
            lon.len(),
            lat.len()
        )));
    }
    let float = |array: &ArrayRef, name: &str| {
        if !array.data_type().is_numeric() {
            return Err(AvengerGeoError::Arrow(format!(
                "{name} must be numeric, not {}",
                array.data_type()
            )));
        }
        cast(array, &DataType::Float64)
            .map(|array| array.as_primitive::<Float64Type>().clone())
            .map_err(|err| AvengerGeoError::Arrow(err.to_string()))
    };
    let (lon, lat): (Float64Array, Float64Array) =
        (float(lon, "longitude")?, float(lat, "latitude")?);
    let coords = SeparatedCoordBuffer::from_vec(
        vec![lon.values().clone(), lat.values().clone()],
        Dimension::XY,
    )
    .map_err(|err| AvengerGeoError::Arrow(err.to_string()))?;
    let crs = Crs::from_authority_code("OGC:CRS84".to_string());
    Ok(PointArray::new(
        CoordBuffer::Separated(coords),
        NullBuffer::union(lon.nulls(), lat.nulls()),
        Arc::new(Metadata::new(crs, None)),
    ))
}

/// Load GeoJSON (a FeatureCollection, Feature, or bare geometry) as an Arrow
/// record batch with one row per feature, parsed and cleaned as
/// [`geojson_to_features`] parses them. Its columns:
///
/// - `geometry`: ISO WKB as a `geoarrow.wkb` field with CRS `OGC:CRS84`,
///   which RFC 7946 specifies, and null for a feature with null geometry.
/// - `id`: the top-level feature id, only when some feature has one.
/// - One column per property key, sorted by name. A column of numbers is
///   Float64, a column of booleans is Boolean, and any other column is Utf8,
///   with non-string values as their JSON text. A missing or null value is
///   null, and a column with only null values is Utf8.
///
/// Ids are typed by the same rule. [`RecordBatch::slice`] splits the batch
/// without copying.
///
/// # Errors
/// Returns an error for GeoJSON that doesn't parse, a property named
/// `geometry`, and a property named `id` when some feature has a top-level id.
pub fn geojson_to_record_batch(json: &str) -> Result<RecordBatch, AvengerGeoError> {
    let features = geojson_to_features(json)?;
    let has_ids = features.iter().any(|feature| feature.id.is_some());
    let keys: Vec<&str> = features
        .iter()
        .flat_map(|feature| feature.properties.keys())
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    for reserved in ["geometry", "id"] {
        if keys.contains(&reserved) && (reserved == "geometry" || has_ids) {
            return Err(AvengerGeoError::Arrow(format!(
                "a property named `{reserved}` collides with the batch's `{reserved}` column"
            )));
        }
    }

    let crs = Crs::from_authority_code("OGC:CRS84".to_string());
    let mut geometry = WkbBuilder::<i32>::new(WkbType::new(Arc::new(Metadata::new(crs, None))));
    for feature in &features {
        geometry
            .push_wkb(feature.wkb.as_deref())
            .map_err(|err| AvengerGeoError::Arrow(err.to_string()))?;
    }
    let geometry = geometry.finish();
    let mut fields = vec![geometry.data_type().to_field("geometry", true)];
    let mut columns = vec![geometry.into_array_ref()];

    let mut push = |name: &str, values: Vec<Option<JsonValue>>| {
        let column = typed_column(&values);
        fields.push(Field::new(name, column.data_type().clone(), true));
        columns.push(column);
    };
    if has_ids {
        push(
            "id",
            features
                .iter()
                .map(|feature| {
                    feature.id.as_ref().map(|id| match id {
                        geojson::feature::Id::String(id) => JsonValue::String(id.clone()),
                        geojson::feature::Id::Number(id) => JsonValue::Number(id.clone()),
                    })
                })
                .collect(),
        );
    }
    for key in &keys {
        push(
            key,
            features
                .iter()
                .map(|feature| feature.properties.get(*key).cloned())
                .collect(),
        );
    }
    RecordBatch::try_new(Arc::new(Schema::new(fields)), columns)
        .map_err(|err| AvengerGeoError::Arrow(err.to_string()))
}

/// A column of JSON values, typed by the rule in [`geojson_to_record_batch`].
fn typed_column(values: &[Option<JsonValue>]) -> ArrayRef {
    fn present(value: &Option<JsonValue>) -> Option<&JsonValue> {
        value.as_ref().filter(|value| !value.is_null())
    }
    let all = |test: fn(&JsonValue) -> bool| {
        let mut present_values = values.iter().filter_map(present).peekable();
        present_values.peek().is_some() && present_values.all(test)
    };
    if all(JsonValue::is_number) {
        Arc::new(Float64Array::from_iter(
            values
                .iter()
                .map(|v| present(v).and_then(JsonValue::as_f64)),
        ))
    } else if all(JsonValue::is_boolean) {
        Arc::new(BooleanArray::from_iter(
            values
                .iter()
                .map(|v| present(v).and_then(JsonValue::as_bool)),
        ))
    } else {
        Arc::new(StringArray::from_iter(values.iter().map(|v| {
            present(v).map(|value| match value {
                JsonValue::String(text) => text.clone(),
                value => value.to_string(),
            })
        })))
    }
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
    check_positions(&geometry.value)?;
    let mut geometry: Geometry<f64> = geometry
        .try_into()
        .map_err(|err: geojson::Error| AvengerGeoError::GeoJson(err))?;
    drop_degenerate_rings(&mut geometry);
    rewind(&mut geometry);
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

/// Reject a position with fewer than two values. The geojson crate rejects a
/// short point position as it parses, but indexes the positions of other
/// geometry when it converts them, and panics.
fn check_positions(value: &geojson::GeometryValue) -> Result<(), geojson::Error> {
    use geojson::GeometryValue;
    fn check<'a>(
        positions: impl IntoIterator<Item = &'a geojson::Position>,
    ) -> Result<(), geojson::Error> {
        match positions.into_iter().find(|position| position.len() < 2) {
            Some(position) => Err(geojson::Error::PositionTooShort(position.len())),
            None => Ok(()),
        }
    }
    match value {
        GeometryValue::Point { coordinates } => check([coordinates]),
        GeometryValue::MultiPoint { coordinates } | GeometryValue::LineString { coordinates } => {
            check(coordinates)
        }
        GeometryValue::MultiLineString { coordinates } | GeometryValue::Polygon { coordinates } => {
            check(coordinates.iter().flatten())
        }
        GeometryValue::MultiPolygon { coordinates } => {
            check(coordinates.iter().flatten().flatten())
        }
        GeometryValue::GeometryCollection { geometries } => geometries
            .iter()
            .try_for_each(|geometry| check_positions(&geometry.value)),
    }
}

/// Remove rings with too few vertices or zero planar area. Such rings can
/// make spherical clipping emit the ring's complement. Nonzero rings are
/// retained regardless of their size.
pub fn drop_degenerate_rings(geometry: &mut Geometry<f64>) {
    fn has_area(ring: &geo_types::LineString<f64>) -> bool {
        ring_has_area(ring, ring_signed_area2(ring))
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

/// Normalize ring winding to RFC 7946: counter-clockwise exterior rings and
/// clockwise holes, by planar signed area.
pub fn rewind(geometry: &mut Geometry<f64>) {
    fn wind(ring: &mut geo_types::LineString<f64>, counter_clockwise: bool) {
        if (ring_signed_area2(ring) > 0.0) != counter_clockwise {
            ring.0.reverse();
        }
    }
    fn rewind_polygon(polygon: &mut geo_types::Polygon<f64>) {
        polygon.exterior_mut(|ring| wind(ring, true));
        polygon.interiors_mut(|rings| {
            for ring in rings {
                wind(ring, false);
            }
        });
    }
    match geometry {
        Geometry::Polygon(polygon) => rewind_polygon(polygon),
        Geometry::MultiPolygon(multi) => multi.0.iter_mut().for_each(rewind_polygon),
        Geometry::GeometryCollection(collection) => collection.0.iter_mut().for_each(rewind),
        _ => {}
    }
}

/// Lon/lat bounds from raw coordinates (ignores any GeoJSON bbox member), or
/// `None` for empty geometry.
pub fn lonlat_bbox(geometry: &Geometry<f64>) -> Option<[f64; 4]> {
    fn extend(
        bounds: &mut Option<[f64; 4]>,
        coords: impl IntoIterator<Item = geo_types::Coord<f64>>,
    ) {
        for c in coords {
            let b = bounds.get_or_insert([c.x, c.y, c.x, c.y]);
            *b = [b[0].min(c.x), b[1].min(c.y), b[2].max(c.x), b[3].max(c.y)];
        }
    }
    fn visit(geometry: &Geometry<f64>, bounds: &mut Option<[f64; 4]>) {
        match geometry {
            Geometry::Point(p) => extend(bounds, [p.0]),
            Geometry::Line(l) => extend(bounds, [l.start, l.end]),
            Geometry::LineString(l) => extend(bounds, l.0.iter().copied()),
            Geometry::Polygon(p) => extend(bounds, p.exterior().0.iter().copied()),
            Geometry::MultiPoint(m) => extend(bounds, m.0.iter().map(|p| p.0)),
            Geometry::MultiLineString(m) => {
                extend(bounds, m.0.iter().flat_map(|l| l.0.iter().copied()))
            }
            Geometry::MultiPolygon(m) => extend(
                bounds,
                m.0.iter().flat_map(|p| p.exterior().0.iter().copied()),
            ),
            Geometry::GeometryCollection(c) => c.0.iter().for_each(|g| visit(g, bounds)),
            Geometry::Rect(r) => extend(bounds, [r.min(), r.max()]),
            Geometry::Triangle(t) => extend(bounds, t.to_array()),
        }
    }
    let mut bounds = None;
    visit(geometry, &mut bounds);
    bounds
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::{RecordingSink, StreamEvent};
    use crate::walk::{walk, Rings};

    /// The events of a feature's WKB, streamed as written.
    fn wkb_events(wkb: &[u8]) -> RecordingSink {
        let mut sink = RecordingSink::default();
        walk(
            &wkb::reader::read_wkb(wkb).unwrap(),
            &mut sink,
            Rings::AsWritten,
        );
        sink
    }

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
    fn rejects_short_positions_in_any_geometry() {
        for geometry in [
            r#"{"type": "LineString", "coordinates": [[0, 0], [1]]}"#,
            r#"{"type": "Polygon", "coordinates": [[[0, 0], [1, 0], [1], [0, 0]]]}"#,
            r#"{"type": "MultiPolygon", "coordinates": [[[[0, 0], [1, 0], [1, 1], []]]]}"#,
            r#"{"type": "GeometryCollection", "geometries": [{"type": "MultiPoint", "coordinates": [[0]]}]}"#,
        ] {
            let error = geojson_to_features(geometry).unwrap_err();
            assert!(
                matches!(
                    error,
                    AvengerGeoError::GeoJson(geojson::Error::PositionTooShort(_))
                ),
                "{geometry}: {error}"
            );
        }
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
            let rings = wkb_events(features[0].wkb.as_ref().unwrap()).polylines();
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
    fn rewinds_exteriors_counter_clockwise_and_holes_clockwise() {
        // A clockwise exterior with a counter-clockwise hole, the opposite of RFC 7946.
        let json = r#"{
            "type": "Polygon",
            "coordinates": [
                [[0,0],[0,10],[10,10],[10,0],[0,0]],
                [[2,2],[8,2],[8,8],[2,8],[2,2]]
            ]
        }"#;
        let features = geojson_to_features(json).expect("parse");
        let areas: Vec<f64> = wkb_events(features[0].wkb.as_ref().unwrap())
            .polylines()
            .iter()
            .map(|ring| {
                // Planar shoelace over the open ring (closing edge implied).
                (0..ring.len())
                    .map(|i| {
                        let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                        a[0] * b[1] - b[0] * a[1]
                    })
                    .sum()
            })
            .collect();
        assert!(areas[0] > 0.0 && areas[1] < 0.0, "{areas:?}");
    }

    #[test]
    fn wkb_streams_polygon_without_closing_point() {
        let json = r#"{
            "type": "Polygon",
            "coordinates": [[[0,0],[0,10],[10,10],[10,0],[0,0]]]
        }"#;
        let features = geojson_to_features(json).expect("parse");
        let sink = wkb_events(features[0].wkb.as_ref().unwrap());
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
