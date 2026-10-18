use avenger_geo::paths::geometry_path;
use avenger_geo::projections::{
    ConicConformal, ConicEqualArea, EqualEarth, Equirectangular, Mercator,
};
use avenger_geo::{FitSummary, Graticule, Projection, RecordingSink};
use common::bounds;
use geo_types::{
    coord, Geometry, GeometryCollection, LineString, MultiLineString, Point, Polygon, Rect,
};
use lyon_path::Event;

mod common;

fn config(projection: &Projection) -> serde_json::Value {
    serde_json::to_value(projection).unwrap()
}

#[test]
fn failed_fit_preserves_configuration() {
    let original = Projection::new(Equirectangular)
        .with_view(42.0, [1.0, 2.0])
        .with_clip_extent(Some([[0.0, 0.0], [10.0, 10.0]]));
    let range = [[0.0, 0.0], [100.0, 100.0]];
    for geometry in [
        Geometry::GeometryCollection(GeometryCollection::<f64>::default()),
        Point::new(2.0, 3.0).into(),
        Point::new(f64::NAN, 3.0).into(),
    ] {
        let mut projection = original.clone();
        let summary = projection.summarize(&geometry).unwrap();
        assert!(projection.fit(range, &summary).is_err());
        assert_eq!(config(&projection), config(&original));
    }
    let sphere = original.summarize_sphere().unwrap();
    for size in [
        [0.0, 10.0],
        [-1.0, 10.0],
        [f64::NAN, 10.0],
        [f64::INFINITY, 10.0],
    ] {
        let mut projection = original.clone();
        assert!(projection.fit([[0.0, 0.0], size], &sphere).is_err());
        assert_eq!(config(&projection), config(&original));
    }
}

#[test]
fn merged_summaries_fit_like_the_summary_of_the_whole() {
    let parts = [
        LineString::from(vec![(-40.0, -10.0), (-5.0, 20.0)]),
        LineString::from(vec![(5.0, -30.0), (60.0, 15.0)]),
    ];
    let projection = Projection::new(EqualEarth);
    let merged = parts.iter().fold(FitSummary::default(), |summary, part| {
        summary.merge(&projection.summarize(part).unwrap())
    });
    let whole = projection
        .summarize(&MultiLineString(parts.to_vec()))
        .unwrap();
    let range = [[0.0, 0.0], [400.0, 300.0]];
    let (mut from_parts, mut from_whole) = (projection.clone(), projection);
    from_parts.fit(range, &merged).unwrap();
    from_whole.fit(range, &whole).unwrap();
    assert_eq!(from_parts.view, from_whole.view);
}

#[test]
fn a_non_finite_vertex_does_not_stop_a_fit() {
    let geometry = MultiLineString(vec![
        LineString::from(vec![(0.0, 0.0), (10.0, 10.0)]),
        LineString::from(vec![(20.0, f64::NAN), (30.0, 5.0)]),
    ]);
    let mut projection = Projection::new(Equirectangular);
    let summary = projection.summarize(&geometry).unwrap();
    projection
        .fit([[0.0, 0.0], [100.0, 100.0]], &summary)
        .unwrap();
}

#[test]
fn fit_handles_one_dimensional_geometry_and_retains_clipping() {
    for points in [vec![(0.0, 1.0), (2.0, 1.0)], vec![(1.0, 0.0), (1.0, 2.0)]] {
        let line = LineString::from(points);
        let clip = Some([[20.0, 20.0], [80.0, 80.0]]);
        let mut projection = Projection::planar(false).with_clip_extent(clip);
        let summary = projection.summarize(&line).unwrap();
        projection
            .fit([[0.0, 0.0], [100.0, 100.0]], &summary)
            .unwrap();
        assert_eq!(projection.clip_extent, clip);
        projection.clip_extent = None;
        let [[x0, y0], [x1, y1]] = bounds(&projection.build().unwrap(), &line).unwrap();
        assert!((x0 + x1 - 100.0).abs() < 1e-9);
        assert!((y0 + y1 - 100.0).abs() < 1e-9);
        assert!(((x1 - x0).max(y1 - y0) - 100.0).abs() < 1e-9);
    }
}

#[test]
fn planar_reflection_matches_display_coordinates() {
    for (reflect_y, expected_y) in [(false, 23.0), (true, 17.0)] {
        let projector = Projection::planar(reflect_y)
            .with_view(1.0, [10.0, 20.0])
            .build()
            .unwrap();
        assert_eq!(projector.project_point(2.0, 3.0), Some((12.0, expected_y)));
        assert_eq!(projector.invert(12.0, expected_y), Some((2.0, 3.0)));
        assert_eq!(
            bounds(&projector, &Point::new(2.0, 3.0)),
            Some([[12.0, expected_y], [12.0, expected_y]])
        );
    }
}

#[test]
fn inverse_rejects_non_finite_values() {
    for projection in [
        Projection::new(Equirectangular),
        Projection::new(EqualEarth),
        Projection::planar(false),
    ] {
        let projector = projection.with_view(150.0, [480.0, 250.0]).build().unwrap();
        for point in [(f64::NAN, 0.0), (0.0, f64::INFINITY)] {
            assert_eq!(projector.invert(point.0, point.1), None);
        }
    }
}

#[test]
fn invalid_clip_extents_do_not_build() {
    for extent in [
        [[10.0, 0.0], [0.0, 10.0]],
        [[0.0, 0.0], [10.0, 0.0]],
        [[0.0, f64::NAN], [10.0, 10.0]],
    ] {
        let projection = Projection::new(Mercator)
            .with_view(1.0, [0.0, 0.0])
            .with_clip_extent(Some(extent));
        assert!(projection.build().is_err(), "{extent:?}");
    }
}

#[test]
fn invalid_standard_parallels_do_not_build() {
    for parallels in [[f64::NAN, 10.0], [0.0, 120.0]] {
        for projection in [
            Projection::new(ConicEqualArea::new(parallels)),
            Projection::new(ConicConformal::new(parallels)),
        ] {
            let projection = projection.with_view(1.0, [0.0, 0.0]);
            assert!(projection.build().is_err(), "{parallels:?}");
        }
    }
    // A decoded projection is checked too.
    let mut json = config(&Projection::new(ConicEqualArea::albers()).with_view(1.0, [0.0, 0.0]));
    json["surface"]["spherical"]["fields"]["parallels"] = serde_json::json!([0.0, 120.0]);
    let decoded: Projection = serde_json::from_value(json).unwrap();
    assert!(decoded.build().is_err());
}

/// Exterior rings wind counter-clockwise, as RFC 7946 specifies, so a rect and a
/// counter-clockwise polygon project as the area they enclose, and a clockwise
/// ring encloses the rest of the sphere.
#[test]
fn counter_clockwise_rings_enclose_their_own_area() {
    let projector = Projection::new(Equirectangular)
        .with_view(1.0, [0.0, 0.0])
        .build()
        .unwrap();
    let rect = Rect::new(coord! { x: 0.0, y: 0.0 }, coord! { x: 10.0, y: 10.0 });
    let ccw = rect.to_polygon();
    let mut cw = ccw.clone();
    cw.exterior_mut(|ring| ring.0.reverse());
    let ten = 10f64.to_radians();
    for geometry in [Geometry::Rect(rect), Geometry::Polygon(ccw)] {
        let [[x0, y0], [x1, y1]] = bounds(&projector, &geometry).unwrap();
        assert!(x0.abs() < 1e-9 && (x1 - ten).abs() < 1e-9);
        assert!((y0 + ten).abs() < 1e-9 && y1.abs() < 1e-9);
    }
    let [[x0, _], [x1, _]] = bounds(&projector, &cw).unwrap();
    assert!((x1 - x0 - 2.0 * std::f64::consts::PI).abs() < 1e-9);
}

/// The winding number of a point in rings that a stream drew.
fn winding(sink: &RecordingSink, [px, py]: [f64; 2]) -> i32 {
    let mut winding = 0;
    for ring in sink.polylines() {
        for (i, &[ax, ay]) in ring.iter().enumerate() {
            let [bx, by] = ring[(i + 1) % ring.len()];
            let cross = (bx - ax) * (py - ay) - (px - ax) * (by - ay);
            if ay <= py {
                if by > py && cross > 0.0 {
                    winding += 1;
                }
            } else if by <= py && cross < 0.0 {
                winding -= 1;
            }
        }
    }
    winding
}

/// A hole stays empty under the nonzero rule when a clip extent lies inside the polygon's
/// exterior, for spherical and planar data, with and without a reflected y axis.
#[test]
fn holes_stay_empty_inside_clip_extents() {
    let square = |x0: f64, x1: f64, counter_clockwise: bool| {
        let mut ring = vec![(x0, x0), (x1, x0), (x1, x1), (x0, x1), (x0, x0)];
        if !counter_clockwise {
            ring.reverse();
        }
        LineString::from(ring)
    };
    // Spherical: degrees in, about degrees out, with y down from 50.
    let spherical = Projection::new(Equirectangular)
        .with_view(180.0 / std::f64::consts::PI, [0.0, 50.0])
        .with_clip_extent(Some([[5.0, 15.0], [35.0, 45.0]]));
    let polygon = Polygon::new(square(0.0, 40.0, true), vec![square(15.0, 25.0, false)]);
    let mut cases = vec![(spherical, polygon, [20.0, 30.0], [10.0, 20.0])];
    for reflect_y in [false, true] {
        let planar = Projection::planar(reflect_y)
            .with_view(1.0, [0.0, if reflect_y { 100.0 } else { 0.0 }])
            .with_clip_extent(Some([[20.0, 20.0], [80.0, 80.0]]));
        let polygon = Polygon::new(square(0.0, 100.0, true), vec![square(40.0, 60.0, false)]);
        cases.push((planar, polygon, [50.0, 50.0], [30.0, 30.0]));
    }
    for (projection, polygon, in_hole, in_polygon) in cases {
        let mut sink = RecordingSink::default();
        projection
            .build()
            .unwrap()
            .project_to_sink(&polygon, &mut sink);
        assert_eq!(winding(&sink, in_hole), 0, "{:?}", projection.surface);
        assert_ne!(winding(&sink, in_polygon), 0, "{:?}", projection.surface);
    }
}

#[test]
fn projected_collections_close_rings_and_leave_lines_open() {
    let open = vec![(20., 20.), (30., 30.)];
    // RFC 7946 winding: a counter-clockwise exterior and a clockwise hole.
    let outer = vec![(0., 0.), (10., 0.), (10., 10.), (0., 10.), (0., 0.)];
    let hole = vec![(2., 2.), (2., 4.), (4., 4.), (4., 2.), (2., 2.)];
    let geometry = Geometry::GeometryCollection(GeometryCollection(vec![
        LineString::from(open.clone()).into(),
        Polygon::new(
            LineString::from(outer.clone()),
            vec![LineString::from(hole.clone())],
        )
        .into(),
        LineString::from(open.clone()).into(),
    ]));
    let projector = Projection::planar(false)
        .with_view(1., [0., 0.])
        .build()
        .unwrap();
    // The collection's lines come before its polygons.
    let closures: Vec<_> = geometry_path(&projector.project(&geometry))
        .iter()
        .filter_map(|event| match event {
            Event::End { close, .. } => Some(close),
            _ => None,
        })
        .collect();
    assert_eq!(closures, [false, false, true, true]);
}

#[test]
fn graticule_rejects_invalid_and_unbounded_allocations() {
    for step in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e-300] {
        assert!(Graticule::default().with_step(step).lines().is_err());
    }
    for precision in [0.0, 1e-300] {
        assert!(Graticule {
            precision,
            ..Default::default()
        }
        .lines()
        .is_err());
    }
    let invalid_extent = Graticule {
        extent_minor: [[0.0, 0.0], [f64::INFINITY, 90.0]],
        ..Default::default()
    };
    assert!(invalid_extent.lines().is_err());
}
