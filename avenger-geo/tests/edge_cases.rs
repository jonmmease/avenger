use avenger_geo::paths::geometry_path;
use avenger_geo::projections::{
    ConicConformal, ConicEqualArea, ConicEquidistant, CylindricalEqualArea, EqualEarth,
    Equirectangular, Hammer, Mercator,
};
use avenger_geo::{FitSummary, Graticule, Projection, RecordingSink, RegionFrame, SphericalClip};
use common::bounds;
use geo_types::{
    coord, Geometry, GeometryCollection, LineString, MultiLineString, MultiPolygon, Point, Polygon,
    Rect,
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
        // The rectangle clip clamps points to ±1e9, so this extent would keep nothing.
        [[1e10, 0.0], [1e10 + 10.0, 10.0]],
        [[-2e9, -10.0], [10.0, 10.0]],
    ] {
        let projection = Projection::new(Mercator)
            .with_view(1.0, [0.0, 0.0])
            .with_clip_extent(Some(extent));
        assert!(projection.build().is_err(), "{extent:?}");
    }
    let widest = Projection::new(Mercator)
        .with_view(1.0, [0.0, 0.0])
        .with_clip_extent(Some([[-1e9, -1e9], [1e9, 1e9]]));
    assert!(widest.build().is_ok());
}

/// A projection's bounds clip it within its clip extent, so a Mercator map zoomed out inside a
/// larger viewport fills its world square, and one moved off the viewport draws nothing.
#[test]
fn bounds_clip_within_the_clip_extent() {
    let area = |polygon: &Polygon<f64>| {
        let ring = &polygon.exterior().0;
        ring.windows(2)
            .map(|pair| pair[0].x * pair[1].y - pair[1].x * pair[0].y)
            .sum::<f64>()
            / 2.0
    };
    let viewport = Some([[0.0, 0.0], [500.0, 500.0]]);
    // The world square is 2π × 50 ≈ 314 wide and tall, around the viewport's center.
    let zoomed_out = Projection::new(Mercator)
        .with_view(50.0, [250.0, 250.0])
        .with_clip_extent(viewport)
        .build()
        .unwrap();
    let side = std::f64::consts::TAU * 50.0;
    assert_eq!(
        zoomed_out.clip_extent(),
        Some([
            [250.0 - side / 2.0, 250.0 - side / 2.0],
            [250.0 + side / 2.0, 250.0 + side / 2.0]
        ])
    );
    let MultiPolygon(sphere) = zoomed_out.project_sphere();
    assert_eq!(sphere.len(), 1);
    assert!((area(&sphere[0]) - side * side).abs() < 1e-6 * side * side);
    let away = Projection::new(Mercator)
        .with_view(50.0, [2000.0, 250.0])
        .with_clip_extent(viewport)
        .build()
        .unwrap();
    assert!(away.project_sphere().0.is_empty());
}

/// A polygon cut for planar maps, as Natural Earth cuts Antarctica, runs along the antimeridian
/// to the south pole and back. It keeps its area as the sphere turns: the pipeline leaves out
/// the spike, whose pole is the antimeridian clip's test point.
#[test]
fn polygons_cut_at_a_pole_keep_their_area_as_the_sphere_turns() {
    let coast = [
        (-179.94, -84.72),
        (-120.0, -73.0),
        (-60.0, -70.0),
        (0.0, -69.0),
        (60.0, -67.0),
        (120.0, -66.0),
        (178.28, -84.47),
    ];
    let mut ring = vec![
        (180.0, -84.7),
        (180.0, -90.0),
        (-179.99999999999994, -90.0),
        (-179.99999999999994, -84.7),
    ];
    ring.extend(coast);
    // RFC 7946 winds the exterior counter-clockwise, the reverse of the order above.
    ring.reverse();
    ring.push(ring[0]);
    let antarctica = Polygon::new(LineString::from(ring), vec![]);
    let area = |turn: f64| {
        let projector = Projection::new(Mercator)
            .with_view(77.35, [243.0, 243.0])
            .with_clip_extent(Some([[0.0, 0.0], [486.0, 486.0]]))
            .with_rotate([turn, 0.0, 0.0])
            .build()
            .unwrap();
        let Geometry::MultiPolygon(polygons) = projector.project(&antarctica) else {
            panic!("polygons");
        };
        polygons
            .0
            .iter()
            .map(|polygon| {
                let ring = &polygon.exterior().0;
                ring.windows(2)
                    .map(|pair| pair[0].x * pair[1].y - pair[1].x * pair[0].y)
                    .sum::<f64>()
                    / 2.0
            })
            .sum::<f64>()
    };
    let unturned = area(0.0);
    assert!(unturned > 10_000.0, "{unturned}");
    for step in 0..720 {
        let turn = -180.0 + 0.5 * f64::from(step) + 0.137;
        let turned = area(turn);
        assert!(
            (turned - unturned).abs() < 0.05 * unturned,
            "{turn}: {turned} vs {unturned}"
        );
    }
}

/// A projection whose center projects to infinity has no view transform.
#[test]
fn centers_that_project_to_infinity_do_not_build() {
    for projection in [
        Projection::new(Mercator).with_center(0.0, -90.0),
        Projection::new(ConicConformal::new([-90.0, -90.0])),
    ] {
        let projection = projection.with_view(100.0, [0.0, 0.0]);
        assert!(projection.build().is_err(), "{projection:?}");
    }
    let overflow = Projection::new(EqualEarth)
        .with_view(1e308, [0.0, 0.0])
        .with_center(180.0, 0.0);
    assert!(overflow.build().is_err());
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

#[test]
fn invalid_projection_parameters_do_not_build() {
    for projection in [
        Projection::new(ConicEquidistant::new([0.0, 120.0])),
        Projection::new(Hammer::new(0.0)),
        Projection::new(Hammer::new(0.5)),
        Projection::new(Hammer::new(f64::NAN)),
        Projection::new(CylindricalEqualArea::new(90.0)),
    ] {
        let projection = projection.with_view(1.0, [0.0, 0.0]);
        assert!(projection.build().is_err(), "{projection:?}");
    }
}

#[test]
fn invalid_spherical_clips_do_not_build() {
    let ring = |coords: &[(f64, f64)]| {
        LineString::from(
            coords
                .iter()
                .map(|&(x, y)| coord! { x: x, y: y })
                .collect::<Vec<_>>(),
        )
    };
    let square = ring(&[
        (0.0, 0.0),
        (10.0, 0.0),
        (10.0, 10.0),
        (0.0, 10.0),
        (0.0, 0.0),
    ]);
    let hole = ring(&[(2.0, 2.0), (2.0, 4.0), (4.0, 4.0), (4.0, 2.0), (2.0, 2.0)]);
    let regions = [
        MultiPolygon(vec![]),
        MultiPolygon(vec![Polygon::new(square, vec![hole])]),
        MultiPolygon(vec![Polygon::new(
            ring(&[(0.0, 0.0), (10.0, 0.0), (0.0, 0.0)]),
            vec![],
        )]),
        MultiPolygon(vec![Polygon::new(
            ring(&[(0.0, 0.0), (f64::NAN, 0.0), (10.0, 10.0), (0.0, 0.0)]),
            vec![],
        )]),
    ];
    let clips = [0.0, 180.0, -10.0, f64::NAN]
        .map(|angle| SphericalClip::Circle { angle })
        .into_iter()
        .chain(regions.map(|polygon| SphericalClip::Region {
            polygon,
            frame: RegionFrame::Earth,
        }));
    for clip in clips {
        let projection = Projection::new(EqualEarth)
            .with_view(1.0, [0.0, 0.0])
            .with_clip(Some(clip.clone()));
        assert!(projection.build().is_err(), "{clip:?}");
    }
}

#[test]
fn invalid_boxes_are_errors() {
    for [west, south, east, north] in [
        [f64::NEG_INFINITY, -10.0, 10.0, 10.0],
        [-10.0, -10.0, f64::INFINITY, 10.0],
        [-200.0, -10.0, 10.0, 10.0],
        [10.0, -10.0, 10.0, 10.0],
        [-10.0, 10.0, 10.0, -10.0],
        [-10.0, -95.0, 10.0, 10.0],
        [-10.0, f64::NAN, 10.0, 10.0],
    ] {
        assert!(
            SphericalClip::lon_lat_box(RegionFrame::Earth, west, south, east, north).is_err(),
            "{west}, {south}, {east}, {north}"
        );
    }
}

/// A deserialized region's ring need not repeat its first vertex, and it keeps every vertex.
#[test]
fn open_region_rings_clip_as_closed_ones() {
    let ring = |closed: bool| {
        let mut coords = vec![[-20.0, -20.0], [20.0, -20.0], [20.0, 20.0], [-20.0, 20.0]];
        if closed {
            coords.push(coords[0]);
        }
        let exterior: Vec<_> = coords
            .iter()
            .map(|[x, y]| serde_json::json!({"x": x, "y": y}))
            .collect();
        serde_json::from_value::<SphericalClip>(serde_json::json!({
            "region": {"polygon": [{"exterior": exterior, "interiors": []}], "frame": "earth"}
        }))
        .unwrap()
    };
    let line = LineString::from(vec![(-40.0, 0.0), (40.0, 5.0)]);
    let project = |clip: SphericalClip| {
        Projection::new(Equirectangular)
            .with_view(100.0, [0.0, 0.0])
            .with_clip(Some(clip))
            .build()
            .unwrap()
            .project(&line)
    };
    let closed = project(ring(true));
    assert_eq!(project(ring(false)), closed);
    let Geometry::MultiLineString(lines) = closed else {
        panic!("lines");
    };
    // The line keeps the part inside the square: from 20°W to 20°E.
    let xs: Vec<f64> = lines.0[0].coords().map(|c| c.x).collect();
    let (min, max) = xs
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), x| {
            (a.min(*x), b.max(*x))
        });
    assert!(
        (min + 20f64.to_radians() * 100.0).abs() < 1e-6
            && (max - 20f64.to_radians() * 100.0).abs() < 1e-6,
        "{min} {max}"
    );
}

#[test]
fn rotations_and_centers_that_are_not_finite_do_not_build() {
    let projection = Projection::new(EqualEarth).with_view(1.0, [0.0, 0.0]);
    for rotate in [[f64::NAN, 0.0, 0.0], [0.0, f64::INFINITY, 0.0]] {
        assert!(
            projection.clone().with_rotate(rotate).build().is_err(),
            "{rotate:?}"
        );
    }
    assert!(projection.with_center(f64::NAN, 0.0).build().is_err());
}

#[test]
fn spherical_clips_round_trip_through_serialization() {
    for clip in [
        None,
        Some(SphericalClip::Antimeridian),
        Some(SphericalClip::Circle { angle: 28.0 }),
        Some(SphericalClip::lon_lat_box(RegionFrame::Earth, 150.0, -50.0, -150.0, 10.0).unwrap()),
        Some(SphericalClip::lon_lat_box(RegionFrame::View, 150.0, -50.0, -150.0, 10.0).unwrap()),
    ] {
        let projection = Projection::new(EqualEarth)
            .with_view(100.0, [0.0, 0.0])
            .with_clip(clip.clone());
        let decoded: Projection = serde_json::from_value(config(&projection)).unwrap();
        assert_eq!(decoded.clip, clip);
        let line = LineString::from(vec![(-179.0, -20.0), (0.0, 5.0), (179.0, 30.0)]);
        assert_eq!(
            projection.build().unwrap().project(&line),
            decoded.build().unwrap().project(&line),
            "{clip:?}"
        );
    }
}

/// An earth region keeps the same places under rotation, so a box across the antimeridian keeps
/// the points inside it, with or without a rotation that moves it to the frame's center.
#[test]
fn earth_regions_keep_the_points_inside_them() {
    let region =
        SphericalClip::lon_lat_box(RegionFrame::Earth, 150.0, -50.0, -150.0, 10.0).unwrap();
    for rotate in [[0.0, 0.0, 0.0], [180.0, 0.0, 0.0], [160.0, 20.0, 10.0]] {
        let projector = Projection::new(Equirectangular)
            .with_view(100.0, [0.0, 0.0])
            .with_rotate(rotate)
            .with_clip(Some(region.clone()))
            .build()
            .unwrap();
        let visible = |lon: f64, lat: f64| match projector.project(&Point::new(lon, lat)) {
            Geometry::Point(point) => !point.x().is_nan(),
            other => panic!("{other:?}"),
        };
        for (lon, lat) in [(170.0, -20.0), (-170.0, -20.0), (155.0, 5.0)] {
            assert!(visible(lon, lat), "({lon}, {lat}) at {rotate:?}");
        }
        for (lon, lat) in [(0.0, 0.0), (170.0, 30.0), (-140.0, -20.0)] {
            assert!(!visible(lon, lat), "({lon}, {lat}) at {rotate:?}");
        }
    }
}

/// A polygon that holds a region without crossing it fills the region, through a clip extent
/// too: Mercator always has one.
#[test]
fn polygons_around_a_region_fill_it() {
    let region = SphericalClip::lon_lat_box(RegionFrame::Earth, -30.0, -40.0, 60.0, 40.0).unwrap();
    let around = Polygon::new(
        LineString::from(vec![
            (-60.0, -60.0),
            (90.0, -60.0),
            (90.0, 60.0),
            (-60.0, 60.0),
            (-60.0, -60.0),
        ]),
        vec![],
    );
    for projection in [Projection::new(Mercator), Projection::new(Equirectangular)] {
        let projector = projection
            .with_view(100.0, [0.0, 0.0])
            .with_clip(Some(region.clone()))
            .build()
            .unwrap();
        let Geometry::MultiPolygon(filled) = projector.project(&around) else {
            panic!("a multipolygon");
        };
        assert_eq!(filled.0.len(), 1, "{:?}", projector.clip_extent());
        // The fill is the region's box: its corners project to the box's corners.
        let (x0, y0) = projector.project_point(-30.0, 40.0).unwrap();
        let (x1, y1) = projector.project_point(60.0, -40.0).unwrap();
        for coord in filled.0[0].exterior().coords() {
            assert!(coord.x >= x0 - 1e-6 && coord.x <= x1 + 1e-6, "{coord:?}");
            assert!(coord.y >= y0 - 1e-6 && coord.y <= y1 + 1e-6, "{coord:?}");
        }
    }
}

/// A region of two polygons clips a polygon that holds both, with a hole in one, into a polygon
/// for each, the hole kept, and through a clip extent too.
#[test]
fn regions_of_several_polygons_clip_each_in_turn() {
    let rect = |x0: f64, y0: f64, x1: f64, y1: f64| {
        Polygon::new(
            LineString::from(vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)]),
            vec![],
        )
    };
    let region = SphericalClip::Region {
        polygon: MultiPolygon(vec![
            rect(-40.0, -10.0, -20.0, 10.0),
            rect(20.0, -10.0, 40.0, 10.0),
        ]),
        frame: RegionFrame::View,
    };
    let hole = LineString::from(vec![
        (25.0, -5.0),
        (25.0, 5.0),
        (35.0, 5.0),
        (35.0, -5.0),
        (25.0, -5.0),
    ]);
    let around = Polygon::new(rect(-60.0, -30.0, 60.0, 30.0).into_inner().0, vec![hole]);
    for clip_extent in [None, Some([[-1000.0, -1000.0], [1000.0, 1000.0]])] {
        let projector = Projection::new(Equirectangular)
            .with_view(100.0, [0.0, 0.0])
            .with_precision(0.0)
            .with_clip(Some(region.clone()))
            .with_clip_extent(clip_extent)
            .build()
            .unwrap();
        let Geometry::MultiPolygon(clipped) = projector.project(&around) else {
            panic!("a multipolygon");
        };
        let mut holes: Vec<usize> = clipped
            .0
            .iter()
            .map(|polygon| polygon.interiors().len())
            .collect();
        holes.sort();
        assert_eq!(holes, [0, 1], "{clip_extent:?}: {clipped:?}");
    }
}

/// A view region stays in the same part of the map as the sphere turns, so a band of latitudes
/// along the frame's antimeridian keeps a Mercator map's sphere one rectangle at every
/// longitude.
/// A non-finite vertex breaks a line before the clip sees it, so a region keeps only the parts
/// of the pieces inside it, and a ring leaves the vertex out.
#[test]
fn non_finite_vertices_break_lines_before_the_clip() {
    let region = SphericalClip::lon_lat_box(RegionFrame::View, -20.0, -20.0, 20.0, 20.0).unwrap();
    let projector = Projection::new(Equirectangular)
        .with_view(180.0 / std::f64::consts::PI, [0.0, 0.0])
        .with_precision(0.0)
        .with_clip(Some(region))
        .build()
        .unwrap();
    let xs = |line: &[(f64, f64)]| -> Vec<Vec<f64>> {
        let Geometry::MultiLineString(lines) = projector.project(&LineString::from(line.to_vec()))
        else {
            panic!("lines");
        };
        lines
            .0
            .iter()
            .map(|line| line.0.iter().map(|point| point.x.round()).collect())
            .collect()
    };
    let nan = (f64::NAN, f64::NAN);
    assert_eq!(
        xs(&[(0.0, 0.0), (10.0, 0.0), nan, (30.0, 0.0), (40.0, 0.0)]),
        [[0.0, 10.0]]
    );
    assert_eq!(
        xs(&[
            (-30.0, 0.0),
            (-25.0, 0.0),
            nan,
            (-10.0, 0.0),
            (0.0, 0.0),
            (10.0, 0.0)
        ]),
        [[-10.0, 0.0, 10.0]]
    );
    let square = [
        (-10.0, -10.0),
        (10.0, -10.0),
        (10.0, 10.0),
        (-10.0, 10.0),
        (-10.0, -10.0),
    ];
    let mut broken = square.to_vec();
    broken.insert(2, nan);
    let polygon = |ring: Vec<(f64, f64)>| Polygon::new(LineString::from(ring), vec![]);
    assert_eq!(
        projector.project(&polygon(broken)),
        projector.project(&polygon(square.to_vec()))
    );
}

#[test]
fn view_regions_stay_on_the_map_as_the_sphere_turns() {
    let band = SphericalClip::lon_lat_box(RegionFrame::View, -180.0, -60.0, 180.0, 80.0).unwrap();
    let sphere_bounds = |rotate: [f64; 3]| {
        let sphere = Projection::new(Mercator)
            .with_view(100.0, [0.0, 0.0])
            .with_rotate(rotate)
            .with_clip(Some(band.clone()))
            .build()
            .unwrap()
            .project_sphere();
        assert_eq!(sphere.0.len(), 1, "{rotate:?}");
        let coords = sphere.0[0].exterior().coords();
        coords.fold(
            [f64::INFINITY, f64::INFINITY, -f64::INFINITY, -f64::INFINITY],
            |b, c| [b[0].min(c.x), b[1].min(c.y), b[2].max(c.x), b[3].max(c.y)],
        )
    };
    let unturned = sphere_bounds([0.0, 0.0, 0.0]);
    for rotate in [[-150.0, 0.0, 0.0], [60.0, 0.0, 0.0]] {
        let turned = sphere_bounds(rotate);
        for (a, b) in unturned.iter().zip(&turned) {
            assert!(
                (a - b).abs() < 1e-6,
                "{rotate:?}: {turned:?}, unturned {unturned:?}"
            );
        }
    }
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
