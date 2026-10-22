//! An instance's shape at each extent, cut by its mark's clip: what every query compares.

use avenger_geometry::query::Extent;
use avenger_geometry::rtree::GeometryInstance;
use avenger_scenegraph::marks::{group::Clip, mark::MarkInstance};
use geo::{Area, Contains, Intersects};
use geo_types::{coord, Geometry, LineString, Point, Polygon, Rect};

fn instance(geometry: Geometry<f32>, radius: f32, margin: f32) -> GeometryInstance {
    GeometryInstance {
        mark_instance: MarkInstance {
            name: "probe".into(),
            mark_path: vec![0],
            instance_index: Some(0),
        },
        geometry,
        radius,
        margin,
    }
}

fn area(geometry: &Geometry<f32>) -> f32 {
    geometry.unsigned_area()
}

/// A stroked circle draws its disc and the stroke's outer half, fills its disc, and is its
/// center as geometry and as centroid.
#[test]
fn a_circle_is_its_disc_its_drawn_disc_and_its_center() {
    let circle = instance(Point::new(10.0, 10.0).into(), 5.0, 1.0);
    let drawn = circle.shape(Extent::Drawn, &Clip::None).unwrap();
    let fill = circle.shape(Extent::Fill, &Clip::None).unwrap();
    let pi = std::f32::consts::PI;
    assert!((area(&drawn) - pi * 36.0).abs() < 1.0);
    assert!((area(&fill) - pi * 25.0).abs() < 1.0);
    let center = Geometry::Point(Point::new(10.0, 10.0));
    assert_eq!(
        circle.shape(Extent::Geometry, &Clip::None),
        Some(center.clone())
    );
    assert_eq!(circle.shape(Extent::Centroid, &Clip::None), Some(center));
}

/// A stroked square draws its square grown by half the stroke, with round corners, and fills,
/// and is as geometry, its square.
#[test]
fn a_square_is_its_polygon_and_its_drawn_outline() {
    let square = Rect::new(coord! { x: 0.0, y: 0.0 }, coord! { x: 10.0, y: 10.0 });
    let square = instance(square.into(), 0.0, 1.0);
    let drawn = square.shape(Extent::Drawn, &Clip::None).unwrap();
    let rounded = 100.0 + 4.0 * 10.0 + std::f32::consts::PI;
    assert!((area(&drawn) - rounded).abs() < 1.0, "{}", area(&drawn));
    assert_eq!(
        area(&square.shape(Extent::Fill, &Clip::None).unwrap()),
        100.0
    );
    assert_eq!(
        area(&square.shape(Extent::Geometry, &Clip::None).unwrap()),
        100.0
    );
    assert_eq!(
        square.shape(Extent::Centroid, &Clip::None),
        Some(Point::new(5.0, 5.0).into())
    );
}

/// A stroked line draws a band along its path, fills nothing, is its path as geometry, and has a
/// centroid that can lie off the line.
#[test]
fn a_line_is_its_band_and_its_path_and_fills_nothing() {
    let path = LineString::from(vec![(0.0, 0.0), (10.0, 10.0), (20.0, 0.0)]);
    let line = instance(path.clone().into(), 0.0, 1.0);
    let drawn = line.shape(Extent::Drawn, &Clip::None).unwrap();
    assert!(drawn.contains(&Point::new(10.0, 10.0)));
    assert!(drawn.contains(&Point::new(0.5, 0.0)));
    assert!(line.shape(Extent::Fill, &Clip::None).is_none());
    assert_eq!(line.shape(Extent::Geometry, &Clip::None), Some(path.into()));
    let centroid = line.shape(Extent::Centroid, &Clip::None).unwrap();
    assert_eq!(centroid, Point::new(10.0, 5.0).into());
    assert!(!drawn.intersects(&centroid));
}

/// A clip shows or hides each extent whole: a circle that its clip shows in part keeps its
/// whole disc, a clip that hides an instance leaves no shape, and a shape whose centroid lies
/// outside its clip has no centroid shape though part of it shows.
#[test]
fn clips_show_or_hide_each_extent_whole() {
    let circle = instance(Point::new(10.0, 10.0).into(), 5.0, 0.0);
    let right = Clip::Rect {
        x: 10.0,
        y: 0.0,
        width: 20.0,
        height: 20.0,
    };
    let disc = circle.shape(Extent::Fill, &right).unwrap();
    assert!(
        (area(&disc) - std::f32::consts::PI * 25.0).abs() < 1.0,
        "{}",
        area(&disc)
    );
    // The center lies on the clip's edge, which the clip shows.
    assert_eq!(
        circle.shape(Extent::Geometry, &right),
        Some(Point::new(10.0, 10.0).into())
    );

    let away = Clip::Rect {
        x: 50.0,
        y: 50.0,
        width: 10.0,
        height: 10.0,
    };
    for extent in [
        Extent::Drawn,
        Extent::Fill,
        Extent::Geometry,
        Extent::Centroid,
    ] {
        assert!(circle.shape(extent, &away).is_none(), "{extent:?}");
    }

    // A slice of a pie around (0, 0), from 0° to 90°, whose centroid lies near (4.2, 4.2).
    let slice = Polygon::new(
        LineString::from(vec![
            (0.0, 0.0),
            (10.0, 0.0),
            (7.07, 7.07),
            (0.0, 10.0),
            (0.0, 0.0),
        ]),
        vec![],
    );
    let slice = instance(slice.into(), 0.0, 0.0);
    let corner = Clip::Rect {
        x: 6.0,
        y: 0.0,
        width: 10.0,
        height: 10.0,
    };
    assert!(slice.shape(Extent::Fill, &corner).is_some());
    assert!(slice.shape(Extent::Centroid, &corner).is_none());
}
