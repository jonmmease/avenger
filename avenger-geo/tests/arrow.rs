//! Fitting and projecting Arrow arrays, and loading GeoJSON as a record batch.

use arrow::array::{
    Array, ArrayRef, AsArray, BinaryArray, Float32Array, Float64Array, StringArray,
};
use arrow::datatypes::{DataType, Float64Type};
use avenger_geo::ingest::{geojson_to_record_batch, lon_lat_points};
use avenger_geo::paths::row_parts;
use avenger_geo::projections::Equirectangular;
use avenger_geo::{FitSummary, Projection, Projector};
use geo_traits::to_geo::ToGeoGeometry;
use geo_types::{Geometry, LineString, MultiPoint, Point, Polygon};
use geoarrow_array::array::from_arrow_array;
use geoarrow_array::builder::{
    GeometryBuilder, MultiPointBuilder, PointBuilder, PolygonBuilder, WkbBuilder,
};
use geoarrow_array::{downcast_geoarrow_array, GeoArrowArray, GeoArrowArrayAccessor};
use geoarrow_schema::error::GeoArrowResult;
use geoarrow_schema::{
    CoordType, Dimension, GeoArrowType, GeometryType, Metadata, MultiPointType, PointType,
    PolygonType, WkbType,
};
use lyon_path::{Event, Path};
use std::sync::Arc;

/// A square from `x0, y0` to `x1, y1`, counter-clockwise unless `clockwise`.
fn square([x0, y0]: [f64; 2], [x1, y1]: [f64; 2], clockwise: bool) -> Polygon<f64> {
    let mut ring = vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)];
    if clockwise {
        ring.reverse();
    }
    Polygon::new(LineString::from(ring), vec![])
}

/// Geometry as a WKB array, read back through its `geoarrow.wkb` field, with
/// `None` as a null row.
fn wkb_array(geometry: &[Option<&Polygon<f64>>]) -> Arc<dyn GeoArrowArray> {
    let mut builder = WkbBuilder::<i32>::new(WkbType::default());
    for geometry in geometry {
        builder.push_geometry(*geometry).unwrap();
    }
    let array = builder.finish();
    let field = array.data_type().to_field("geometry", true);
    from_arrow_array(&array.into_array_ref(), &field).unwrap()
}

/// A field whose WKB rows are bytes that don't parse.
fn invalid_wkb() -> Arc<dyn GeoArrowArray> {
    let field = geoarrow_schema::GeoArrowType::Wkb(WkbType::default()).to_field("geometry", true);
    let bytes: ArrayRef = Arc::new(BinaryArray::from_vec(vec![&[1u8, 2, 3][..]]));
    from_arrow_array(&bytes, &field).unwrap()
}

fn subpaths(path: &Path) -> usize {
    path.iter()
        .filter(|event| matches!(event, Event::Begin { .. }))
        .count()
}

fn floats(values: &[Option<f64>]) -> ArrayRef {
    Arc::new(Float64Array::from(values.to_vec()))
}

fn collect_rows<'a>(
    array: &'a impl GeoArrowArrayAccessor<'a>,
) -> GeoArrowResult<Vec<Option<Geometry<f64>>>> {
    let empty = || Geometry::Point(Point::new(f64::NAN, f64::NAN));
    array
        .iter()
        .map(|row| {
            row.map(|value| value.map(|g| g.try_to_geometry().unwrap_or_else(empty)))
                .transpose()
        })
        .collect()
}

/// Each row of `array` as geo-types geometry, or `None` for a null row. An
/// empty point reads as a point with NaN coordinates.
fn rows(array: &dyn GeoArrowArray) -> Vec<Option<Geometry<f64>>> {
    downcast_geoarrow_array!(array, collect_rows).unwrap()
}

/// Twice a ring's signed area, with its closing edge implied.
fn signed_area2(ring: &LineString<f64>) -> f64 {
    ring.0
        .windows(2)
        .map(|pair| pair[0].x * pair[1].y - pair[1].x * pair[0].y)
        .sum()
}

/// Equirectangular at a pixel a degree, centered on the origin.
fn unit_projector() -> Projector {
    Projection::new(Equirectangular)
        .with_view(180.0 / std::f64::consts::PI, [0.0, 0.0])
        .build()
        .unwrap()
}

#[test]
fn wkb_and_native_arrays_summarize_like_geo_types() {
    let polygon = square([10.0, 10.0], [40.0, 40.0], false);
    let projection = Projection::new(Equirectangular);
    let native = PolygonBuilder::from_polygons(
        std::slice::from_ref(&polygon),
        PolygonType::new(Dimension::XY, Arc::new(Metadata::default())),
    )
    .finish();
    let expected = projection.summarize(&polygon).unwrap();
    assert_eq!(
        projection
            .summarize_geometry(wkb_array(&[Some(&polygon)]).as_ref())
            .unwrap(),
        expected
    );
    assert_eq!(projection.summarize_geometry(&native).unwrap(), expected);
}

#[test]
fn arrow_rings_are_rewound_and_null_rows_skipped() {
    let ccw = square([10.0, 10.0], [40.0, 40.0], false);
    let cw = square([10.0, 10.0], [40.0, 40.0], true);
    let projection = Projection::new(Equirectangular);
    assert_eq!(
        projection
            .summarize_geometry(wkb_array(&[Some(&cw), None]).as_ref())
            .unwrap(),
        projection.summarize(&ccw).unwrap()
    );
}

#[test]
fn unreadable_rows_and_mismatched_points_are_errors() {
    let projection = Projection::new(Equirectangular);
    assert!(projection
        .summarize_geometry(invalid_wkb().as_ref())
        .is_err());
    let projector = projection
        .clone()
        .with_view(1.0, [0.0, 0.0])
        .build()
        .unwrap();
    assert!(projector.project_geometry(invalid_wkb().as_ref()).is_err());
    assert!(row_parts(invalid_wkb().as_ref()).is_err());

    let lon = floats(&[Some(0.0), Some(10.0)]);
    assert!(lon_lat_points(&lon, &floats(&[Some(0.0)])).is_err());
    let names: ArrayRef = Arc::new(StringArray::from(vec!["a", "b"]));
    assert!(lon_lat_points(&lon, &names).is_err());
}

#[test]
fn lon_lat_points_fit_and_project_like_single_points() {
    let lon = floats(&[Some(-20.0), None, Some(30.0), Some(5.0)]);
    let lat = floats(&[Some(10.0), Some(0.0), Some(-15.0), None]);
    let points = lon_lat_points(&lon, &lat).unwrap();
    assert_eq!(points.logical_null_count(), 2);
    assert!(points
        .data_type()
        .metadata()
        .crs()
        .crs_value()
        .is_some_and(|crs| crs.to_string().contains("OGC:CRS84")));
    let mut projection = Projection::new(Equirectangular);
    let summary = projection.summarize_geometry(&points).unwrap();
    projection
        .fit([[0.0, 0.0], [500.0, 250.0]], &summary)
        .unwrap();
    let projector = projection.build().unwrap();
    let projected = rows(projector.project_geometry(&points).unwrap().as_ref());
    for (row, (lon, lat)) in [(0, (-20.0, 10.0)), (2, (30.0, -15.0))] {
        let (x, y) = projector.project_point(lon, lat).unwrap();
        assert_eq!(projected[row], Some(Geometry::Point(Point::new(x, y))));
    }
    // The fitted points span the range's width, and null rows stay null.
    let Some(Geometry::Point(first)) = projected[0] else {
        panic!("a point");
    };
    assert!(first.x().abs() < 1e-9);
    assert!(projected[1].is_none() && projected[3].is_none());

    // Float32 columns are cast.
    let lon: ArrayRef = Arc::new(Float32Array::from(vec![1.5f32]));
    let lat: ArrayRef = Arc::new(Float32Array::from(vec![-2.0f32]));
    let points = lon_lat_points(&lon, &lat).unwrap();
    assert_eq!(
        rows(&points)[0],
        Some(Geometry::Point(Point::new(1.5, -2.0)))
    );
}

#[test]
fn points_that_the_clip_leaves_out_are_empty() {
    let projector = Projection::new(Equirectangular)
        .with_view(180.0 / std::f64::consts::PI, [0.0, 0.0])
        .with_clip_extent(Some([[-10.0, -10.0], [10.0, 10.0]]))
        .build()
        .unwrap();
    let points = lon_lat_points(
        &floats(&[Some(5.0), Some(50.0)]),
        &floats(&[Some(5.0), Some(0.0)]),
    )
    .unwrap();
    let projected = projector.project_geometry(&points).unwrap();
    let rows = rows(projected.as_ref());
    // A point at 5°N is 5 pixels up.
    assert_eq!(rows[0], Some(Geometry::Point(Point::new(5.0, -5.0))));
    let Some(Geometry::Point(clipped)) = rows[1] else {
        panic!("a point");
    };
    assert!(clipped.x().is_nan() && clipped.y().is_nan());
    // The empty point draws no symbol.
    let parts = row_parts(projected.as_ref()).unwrap();
    assert_eq!(parts.point_rows, [0]);
    // A single point projects the same way.
    let Geometry::Point(single) = projector.project(&Point::new(50.0, 0.0)) else {
        panic!("a point");
    };
    assert!(single.x().is_nan());
}

#[test]
fn each_input_type_gives_its_screen_space_type() {
    let metadata = Arc::new(Metadata::default());
    let point = Point::new(1.0, 2.0);
    let polygon = square([10.0, 10.0], [40.0, 40.0], false);
    let points = PointBuilder::from_points(
        [point].iter(),
        PointType::new(Dimension::XY, metadata.clone()),
    )
    .finish();
    let multi_points = MultiPointBuilder::from_multi_points(
        &[MultiPoint::from(vec![(1.0, 2.0), (3.0, 4.0)])],
        MultiPointType::new(Dimension::XY, metadata.clone()),
    )
    .finish();
    let polygons = PolygonBuilder::from_polygons(
        std::slice::from_ref(&polygon),
        PolygonType::new(Dimension::XY, metadata.clone()),
    )
    .finish();
    let mut mixed = GeometryBuilder::new(GeometryType::new(metadata.clone()));
    mixed.push_geometry(Some(&Geometry::Point(point))).unwrap();
    mixed
        .push_geometry(Some(&Geometry::Polygon(polygon.clone())))
        .unwrap();
    let mixed = mixed.finish();
    let projector = unit_projector();
    let output = |array: &dyn GeoArrowArray| projector.project_geometry(array).unwrap();
    let cases: [(Arc<dyn GeoArrowArray>, &str); 5] = [
        (output(&points), "point"),
        (output(&multi_points), "multipoint"),
        (output(&polygons), "multipolygon"),
        (output(wkb_array(&[Some(&polygon)]).as_ref()), "geometry"),
        (output(&mixed), "geometry"),
    ];
    for (array, expected) in cases {
        let typ = array.data_type();
        let name = match &typ {
            GeoArrowType::Point(_) => "point",
            GeoArrowType::MultiPoint(_) => "multipoint",
            GeoArrowType::MultiPolygon(_) => "multipolygon",
            GeoArrowType::Geometry(_) => "geometry",
            other => panic!("unexpected {other:?}"),
        };
        assert_eq!(name, expected);
        assert_eq!(typ.coord_type(), Some(CoordType::Separated));
        assert!(typ.metadata().crs().crs_value().is_none());
    }
    // A mixed array's rows keep their kinds.
    let rows = rows(output(&mixed).as_ref());
    assert!(matches!(rows[0], Some(Geometry::Point(_))));
    assert!(matches!(rows[1], Some(Geometry::MultiPolygon(_))));
}

#[test]
fn projected_polygons_keep_their_holes_and_wind_exteriors_positive() {
    let ring = |points: &[(f64, f64)]| LineString::from(points.to_vec());
    // RFC 7946: a counter-clockwise exterior and a clockwise hole.
    let polygon = Polygon::new(
        ring(&[
            (0.0, 0.0),
            (40.0, 0.0),
            (40.0, 40.0),
            (0.0, 40.0),
            (0.0, 0.0),
        ]),
        vec![ring(&[
            (10.0, 10.0),
            (10.0, 20.0),
            (20.0, 20.0),
            (20.0, 10.0),
            (10.0, 10.0),
        ])],
    );
    let Geometry::MultiPolygon(projected) = unit_projector().project(&polygon) else {
        panic!("a multipolygon");
    };
    let [polygon] = &projected.0[..] else {
        panic!("one polygon: {projected:?}");
    };
    assert_eq!(polygon.interiors().len(), 1);
    assert!(signed_area2(polygon.exterior()) > 0.0);
    assert!(signed_area2(&polygon.interiors()[0]) < 0.0);
}

/// A row of a polygon and a line puts the closed ring in the polygon path and
/// the open line in the line path.
#[test]
fn a_row_splits_its_polygons_from_its_lines() {
    let row = Geometry::GeometryCollection(geo_types::GeometryCollection(vec![
        Geometry::Polygon(square([0.0, 0.0], [10.0, 10.0], false)),
        Geometry::LineString(LineString::from(vec![(0.0, 20.0), (10.0, 30.0)])),
    ]));
    let mut builder = GeometryBuilder::new(GeometryType::new(Arc::new(Metadata::default())));
    builder.push_geometry(Some(&row)).unwrap();
    let parts = row_parts(&builder.finish()).unwrap();
    let closes = |path: &Path| -> Vec<bool> {
        path.iter()
            .filter_map(|event| match event {
                Event::End { close, .. } => Some(close),
                _ => None,
            })
            .collect()
    };
    assert_eq!(closes(&parts.polygons[0]), [true]);
    assert_eq!(closes(&parts.lines[0]), [false]);
}

#[test]
fn rows_project_to_one_path_each() {
    let left = square([-10.0, -10.0], [10.0, 10.0], false);
    let right = square([20.0, -10.0], [40.0, 10.0], false);
    let geometry = wkb_array(&[Some(&left), None, Some(&right)]);
    // Rotating by 180° puts the antimeridian at 0°, through the first square.
    let projector = Projection::new(Equirectangular)
        .with_rotate([180.0, 0.0, 0.0])
        .with_view(100.0, [0.0, 0.0])
        .build()
        .unwrap();
    let projected = projector.project_geometry(geometry.as_ref()).unwrap();
    let rows = rows(projected.as_ref());
    let polygons = |row: &Option<Geometry<f64>>| match row {
        Some(Geometry::MultiPolygon(multi)) => multi.0.len(),
        _ => 0,
    };
    assert_eq!(
        rows.iter().map(polygons).collect::<Vec<_>>(),
        [2, 0, 1],
        "the cut square splits in two, and the null row stays null"
    );
    assert!(rows[1].is_none());
    let parts = row_parts(projected.as_ref()).unwrap();
    assert_eq!(
        parts.polygons.iter().map(subpaths).collect::<Vec<_>>(),
        [2, 0, 1]
    );
    assert!(parts.lines.iter().all(|path| subpaths(path) == 0));
}

#[test]
fn geojson_loads_as_typed_columns() {
    let json = r#"{"type": "FeatureCollection", "features": [
        {"type": "Feature", "id": 1, "geometry": {"type": "Point", "coordinates": [1, 2]},
         "properties": {"name": "a", "count": 3, "flag": true, "mixed": 1, "nested": {"k": 1}, "empty": null}},
        {"type": "Feature", "id": 2, "geometry": null,
         "properties": {"name": "b", "count": 4.5, "flag": false, "mixed": "x", "only_here": 7}}
    ]}"#;
    let batch = geojson_to_record_batch(json).unwrap();
    let schema = batch.schema();
    let names: Vec<_> = schema
        .fields()
        .iter()
        .map(|field| field.name().as_str())
        .collect();
    assert_eq!(
        names,
        [
            "geometry",
            "id",
            "count",
            "empty",
            "flag",
            "mixed",
            "name",
            "nested",
            "only_here"
        ]
    );
    let types: Vec<_> = schema
        .fields()
        .iter()
        .skip(1)
        .map(|field| field.data_type().clone())
        .collect();
    use DataType::{Boolean, Float64, Utf8};
    assert_eq!(
        types,
        [Float64, Float64, Utf8, Boolean, Utf8, Utf8, Utf8, Float64]
    );

    let column = |name: &str| batch.column(schema.index_of(name).unwrap()).clone();
    assert_eq!(
        column("mixed")
            .as_string::<i32>()
            .iter()
            .collect::<Vec<_>>(),
        [Some("1"), Some("x")]
    );
    assert_eq!(column("nested").as_string::<i32>().value(0), r#"{"k":1}"#);
    assert_eq!(column("empty").null_count(), 2);
    assert!(column("only_here").is_null(0));
    assert_eq!(column("count").as_primitive::<Float64Type>().value(1), 4.5);

    let geometry = schema.field(0);
    assert_eq!(geometry.extension_type_name(), Some("geoarrow.wkb"));
    assert!(geometry
        .metadata()
        .get("ARROW:extension:metadata")
        .is_some_and(|metadata| metadata.contains("OGC:CRS84")));
    assert!(batch.column(0).is_null(1));
}

#[test]
fn geojson_ids_appear_only_when_present_and_reserve_their_names() {
    let collection =
        |features: &str| format!(r#"{{"type": "FeatureCollection", "features": [{features}]}}"#);
    let feature = |id: &str, properties: &str| {
        format!(r#"{{"type": "Feature", {id} "geometry": null, "properties": {{{properties}}}}}"#)
    };

    // Without top-level ids, a property may be named `id`.
    let batch = geojson_to_record_batch(&collection(&feature("", r#""id": "p""#))).unwrap();
    assert_eq!(batch.schema().field(1).name(), "id");
    assert_eq!(batch.num_columns(), 2);

    // Mixed ids are text.
    let mixed = [feature(r#""id": 1,"#, ""), feature(r#""id": "b","#, "")].join(",");
    let batch = geojson_to_record_batch(&collection(&mixed)).unwrap();
    assert_eq!(batch.schema().field(1).data_type(), &DataType::Utf8);

    for features in [
        feature("", r#""geometry": 1"#),
        feature(r#""id": 1,"#, r#""id": "p""#),
    ] {
        assert!(
            geojson_to_record_batch(&collection(&features)).is_err(),
            "{features}"
        );
    }
}

#[test]
fn a_geojson_batch_fits_and_projects() {
    let json = r#"{"type": "FeatureCollection", "features": [
        {"type": "Feature", "properties": {"name": "a"},
         "geometry": {"type": "Polygon", "coordinates": [[[0, 0], [20, 0], [20, 10], [0, 10], [0, 0]]]}},
        {"type": "Feature", "properties": {"name": "b"},
         "geometry": {"type": "LineString", "coordinates": [[30, -5], [50, 15]]}}
    ]}"#;
    let batch = geojson_to_record_batch(json).unwrap();
    let geometry = from_arrow_array(batch.column(0), batch.schema().field(0)).unwrap();
    let mut projection = Projection::new(Equirectangular);
    let summary = [geometry.slice(0, 1), geometry.slice(1, 1)]
        .iter()
        .fold(FitSummary::default(), |summary, part| {
            summary.merge(&projection.summarize_geometry(part.as_ref()).unwrap())
        });
    projection
        .fit([[0.0, 0.0], [400.0, 200.0]], &summary)
        .unwrap();
    let projector: Projector = projection.build().unwrap();
    let parts = row_parts(
        projector
            .project_geometry(geometry.as_ref())
            .unwrap()
            .as_ref(),
    )
    .unwrap();
    // The polygon's row has a ring to fill, and the line's row a line to stroke.
    assert_eq!(
        parts.polygons.iter().map(subpaths).collect::<Vec<_>>(),
        [1, 0]
    );
    assert_eq!(parts.lines.iter().map(subpaths).collect::<Vec<_>>(), [0, 1]);
}
