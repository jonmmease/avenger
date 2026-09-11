use avenger_geo::{
    BoundsSink, GeoStream, Graticule, LyonPathSink, PolylineSink, Projection, ProjectionKind,
    RecordingSink, Sphere, Streamable,
};
use geo_types::{Geometry, GeometryCollection, LineString, Point};
use lyon_path::Event;

#[test]
fn failed_fit_preserves_configuration() {
    let original = Projection::new(ProjectionKind::Equirectangular)
        .with_scale(42.0)
        .with_translate([1.0, 2.0])
        .with_clip_extent(Some([[0.0, 0.0], [10.0, 10.0]]));
    let empty = Geometry::GeometryCollection(GeometryCollection::<f64>::default());
    for geometry in [
        &empty as &dyn Streamable,
        &Point::new(2.0, 3.0),
        &Point::new(f64::NAN, 3.0),
    ] {
        let mut projection = original.clone();
        assert!(projection.fit_size([100.0, 100.0], geometry).is_err());
        assert_eq!(projection, original);
    }
    for size in [
        [0.0, 10.0],
        [-1.0, 10.0],
        [f64::NAN, 10.0],
        [f64::INFINITY, 10.0],
    ] {
        let mut projection = original.clone();
        assert!(projection.fit_size(size, &Sphere).is_err());
        assert_eq!(projection, original);
    }
}

#[test]
fn fit_handles_one_dimensional_geometry_and_retains_clipping() {
    for points in [vec![(0.0, 1.0), (2.0, 1.0)], vec![(1.0, 0.0), (1.0, 2.0)]] {
        let line = LineString::from(points);
        let clip = Some([[20.0, 20.0], [80.0, 80.0]]);
        let mut projection =
            Projection::new(ProjectionKind::Identity { reflect_y: false }).with_clip_extent(clip);
        projection.fit_size([100.0, 100.0], &line).unwrap();
        assert_eq!(projection.clip_extent, clip);
        projection.clip_extent = None;
        let mut bounds = BoundsSink::default();
        projection.build().stream(&line, &mut bounds);
        let [[x0, y0], [x1, y1]] = bounds.result().unwrap();
        assert!((x0 + x1 - 100.0).abs() < 1e-9);
        assert!((y0 + y1 - 100.0).abs() < 1e-9);
        assert!(((x1 - x0).max(y1 - y0) - 100.0).abs() < 1e-9);
    }
}

#[test]
fn identity_reflection_matches_display_coordinates() {
    for (reflect_y, expected_y) in [(false, 23.0), (true, 17.0)] {
        let config = Projection::new(ProjectionKind::Identity { reflect_y })
            .with_scale(1.0)
            .with_translate([10.0, 20.0]);
        let projector = config.build();
        assert_eq!(projector.project(2.0, 3.0), Some((12.0, expected_y)));
        assert_eq!(projector.invert(12.0, expected_y), Some((2.0, 3.0)));
        let mut bounds = BoundsSink::default();
        projector.stream(&Point::new(2.0, 3.0), &mut bounds);
        assert_eq!(
            bounds.result(),
            Some([[12.0, expected_y], [12.0, expected_y]])
        );
        let raw = config.project_raw_units(2.0, 3.0);
        assert_eq!(config.invert_raw_units(raw.0, raw.1), Some((2.0, 3.0)));
    }
}

#[test]
fn inverse_rejects_non_finite_values_and_zero_scale() {
    for kind in [
        ProjectionKind::Equirectangular,
        ProjectionKind::EqualEarth,
        ProjectionKind::Identity { reflect_y: false },
    ] {
        let projection = Projection::new(kind);
        for point in [(f64::NAN, 0.0), (0.0, f64::INFINITY)] {
            assert_eq!(projection.build().invert(point.0, point.1), None);
            assert_eq!(projection.invert_raw_units(point.0, point.1), None);
        }
        assert_eq!(
            projection.with_scale(0.0).build().invert(480.0, 250.0),
            None
        );
    }
}

#[test]
fn mercator_view_outside_world_emits_no_geometry() {
    let projector =
        Projection::new(ProjectionKind::Mercator).build_view((0.0, 100.0), 0.01, 100.0, 100.0);
    let mut sink = RecordingSink::default();
    projector.stream(&Sphere, &mut sink);
    assert!(sink.events.is_empty());
}

#[test]
fn line_sinks_ignore_isolated_points_and_break_at_invalid_vertices() {
    let mut path = LyonPathSink::stroke();
    let mut lines = PolylineSink::default();
    for sink in [&mut path as &mut dyn GeoStream, &mut lines] {
        sink.point(99.0, 99.0, None);
        sink.line_start();
        sink.point(0.0, 0.0, None);
        sink.point(1.0, 1.0, None);
        sink.point(f64::INFINITY, 2.0, None);
        sink.point(3.0, 3.0, None);
        sink.point(4.0, 4.0, None);
        sink.line_end();
        sink.point(99.0, 99.0, None);
        sink.line_start();
        sink.point(5.0, 5.0, None);
        sink.line_end();
    }
    let path = path.finish();
    assert_eq!(
        path.iter()
            .filter(|e| matches!(e, Event::Begin { .. }))
            .count(),
        3
    );
    assert_eq!(
        path.iter()
            .filter(|e| matches!(e, Event::Line { .. }))
            .count(),
        2
    );
    assert_eq!(lines.x, vec![0.0, 1.0, 1.0, 3.0, 4.0, 4.0, 5.0]);
    assert_eq!(
        lines.defined,
        vec![true, true, false, true, true, false, true]
    );
}

#[test]
fn graticule_rejects_invalid_and_unbounded_allocations() {
    for step in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e-300] {
        assert!(Graticule::default().with_step(step).try_lines().is_err());
    }
    for precision in [0.0, 1e-300] {
        assert!(Graticule {
            precision,
            ..Default::default()
        }
        .try_lines()
        .is_err());
    }
    let invalid_extent = Graticule {
        extent_minor: [[0.0, 0.0], [f64::INFINITY, 90.0]],
        ..Default::default()
    };
    assert!(invalid_extent.try_lines().is_err());
}
