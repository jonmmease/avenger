//! Fitting and projecting Arrow arrays, and loading GeoJSON as a record batch.

use arrow::array::{Array, ArrayRef, AsArray, BinaryArray, Float64Array, StringArray};
use arrow::datatypes::{DataType, Float64Type};
use avenger_geo::ingest::geojson_to_record_batch;
use avenger_geo::projections::Equirectangular;
use avenger_geo::{FitSummary, Projection, Projector};
use geo_types::{LineString, Polygon};
use geoarrow_array::array::from_arrow_array;
use geoarrow_array::builder::{PolygonBuilder, WkbBuilder};
use geoarrow_array::GeoArrowArray;
use geoarrow_schema::{Dimension, Metadata, PolygonType, WkbType};
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
    assert!(projector.project_to_paths(invalid_wkb().as_ref()).is_err());

    let lon = floats(&[Some(0.0), Some(10.0)]);
    assert!(projection
        .summarize_points(&lon, &floats(&[Some(0.0)]))
        .is_err());
    let names: ArrayRef = Arc::new(StringArray::from(vec!["a", "b"]));
    assert!(projection.summarize_points(&lon, &names).is_err());
}

#[test]
fn points_fit_and_project_like_single_points() {
    let lon = floats(&[Some(-20.0), None, Some(30.0), Some(5.0)]);
    let lat = floats(&[Some(10.0), Some(0.0), Some(-15.0), None]);
    let mut projection = Projection::new(Equirectangular);
    let summary = projection.summarize_points(&lon, &lat).unwrap();
    projection
        .fit([[0.0, 0.0], [500.0, 250.0]], &summary)
        .unwrap();
    let projector = projection.build().unwrap();
    let (x, y) = projector.project_points(&lon, &lat).unwrap();
    let (x, y) = (x.as_vec(4, None), y.as_vec(4, None));
    for (i, (lon, lat)) in [(-20.0, 10.0), (30.0, -15.0)].into_iter().enumerate() {
        let row = [0, 2][i];
        let (ex, ey) = projector.project_point(lon, lat).unwrap();
        assert_eq!((x[row], y[row]), (ex as f32, ey as f32));
    }
    // The fitted points span the range's width.
    assert!((x[0] - 0.0).abs() < 1e-3 && (x[2] - 500.0).abs() < 1e-3);
    assert!(x[1].is_nan() && y[1].is_nan() && x[3].is_nan() && y[3].is_nan());

    // A result that overflows f32 isn't finite.
    let overflowing = Projection::new(Equirectangular)
        .with_view(1e308, [0.0, 0.0])
        .build()
        .unwrap();
    let (x, _) = overflowing
        .project_points(&floats(&[Some(90.0)]), &floats(&[Some(0.0)]))
        .unwrap();
    assert!(x.as_vec(1, None)[0].is_nan());
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
    let paths = projector
        .project_to_paths(geometry.as_ref())
        .unwrap()
        .as_vec(3, None);
    assert_eq!(
        paths.iter().map(subpaths).collect::<Vec<_>>(),
        [2, 0, 1],
        "the cut square splits in two, and the null row is empty"
    );
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
    let paths = projector
        .project_to_paths(geometry.as_ref())
        .unwrap()
        .as_vec(batch.num_rows(), None);
    assert_eq!(paths.iter().map(subpaths).collect::<Vec<_>>(), [1, 1]);
}
