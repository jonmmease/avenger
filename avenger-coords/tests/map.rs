//! The projection of a map: places through the projector, longitudes that wrap, paths along
//! parallels, and the region of a globe.

use std::f64::consts::PI;

use avenger_coords::{MapProjection, PlotProjection};
use avenger_geo::{projections::Equirectangular, projections::Orthographic, Projection};

/// A flat map of the whole sphere, 360 by 180 pixels, one pixel to a degree.
fn flat() -> MapProjection {
    let projection = Projection::new(Equirectangular).with_view(180.0 / PI, [180.0, 90.0]);
    MapProjection::new(projection.build().unwrap())
}

/// A globe of radius 80 pixels about the point 180, 90.
fn globe() -> MapProjection {
    let projection = Projection::new(Orthographic).with_view(80.0, [180.0, 90.0]);
    MapProjection::new(projection.build().unwrap())
}

fn close(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6
}

#[test]
fn positions_are_places_through_the_projector() {
    let map = flat();
    assert!(close(map.position([180.0, 90.0]).unwrap(), [0.0, 0.0]));
    assert!(close(map.position([270.0, 45.0]).unwrap(), [90.0, 45.0]));
    let pixel = map.pixel([90.0, 45.0]).unwrap();
    assert!((pixel[0] - 270.0).abs() < 1e-4 && (pixel[1] - 45.0).abs() < 1e-4);
    assert_eq!(map.extent(), [[-180.0, 180.0], [-90.0, 90.0]]);
    assert_eq!(map.period(0), Some(360.0));
    assert_eq!(map.period(1), None);
}

#[test]
fn a_globe_shows_no_place_outside_it_or_behind_it() {
    let map = globe();
    assert!(map.position([180.0, 90.0]).is_some());
    assert_eq!(map.position([5.0, 5.0]), None);
    assert_eq!(map.pixel([180.0, 0.0]), None);
    assert!(
        map.enclosed(&[[100.0, 10.0], [260.0, 10.0], [260.0, 170.0], [100.0, 170.0]])
            .len()
            == 1
    );
}

#[test]
fn paths_follow_parallels_and_break_at_the_antimeridian() {
    let map = flat();
    // A parallel on a flat map is a horizontal line at its latitude.
    let parallel = map.path([-90.0, 45.0], [90.0, 45.0]);
    assert_eq!(parallel.len(), 1);
    assert!(parallel[0].iter().all(|[_, y]| (y - 45.0).abs() < 1e-3));
    // A path across the antimeridian breaks into a piece on each edge of the map.
    let across = map.path([170.0, 0.0], [190.0, 0.0]);
    assert_eq!(across.len(), 2);
}

#[test]
fn polygons_cut_at_the_antimeridian() {
    let map = flat();
    let ring = [[170.0, -10.0], [190.0, -10.0], [190.0, 10.0], [170.0, 10.0]];
    let rings = map.polygon(&ring);
    assert_eq!(rings.len(), 2);
    let inside = [[-10.0, -10.0], [10.0, -10.0], [10.0, 10.0], [-10.0, 10.0]];
    assert_eq!(map.polygon(&inside).len(), 1);
    // A ring in wrapped longitudes, as a lasso's, runs the short way across the antimeridian.
    let wrapped = [
        [170.0, -10.0],
        [-170.0, -10.0],
        [-170.0, 10.0],
        [170.0, 10.0],
    ];
    let rings = map.polygon(&wrapped);
    assert_eq!(rings.len(), 2);
    let width: f32 = (rings.iter())
        .map(|ring| {
            let xs = ring.iter().map(|[x, _]| *x);
            xs.clone().fold(f32::MIN, f32::max) - xs.fold(f32::MAX, f32::min)
        })
        .sum();
    assert!((width - 20.0).abs() < 1e-3, "{width}");
}

#[test]
fn polygons_stop_short_of_the_poles() {
    let map = flat();
    let band = [
        [-180.0, 60.0],
        [-90.0, 60.0],
        [0.0, 60.0],
        [90.0, 60.0],
        [180.0, 60.0],
        [180.0, 90.0],
        [90.0, 90.0],
        [0.0, 90.0],
        [-90.0, 90.0],
        [-180.0, 90.0],
    ];
    let rings = map.polygon(&band);
    assert_eq!(rings.len(), 1);
    let top = rings[0].iter().map(|[_, y]| *y).fold(f32::MAX, f32::min);
    assert!(top.abs() < 1e-3, "{top}");
}

#[test]
fn an_enclosed_path_cuts_to_the_globe_and_winds_around_its_inside() {
    let map = globe();
    // A box over the globe's center that runs past its right edge.
    let path = [[170.0, 80.0], [300.0, 80.0], [300.0, 100.0], [170.0, 100.0]];
    let polygons = map.enclosed(&path);
    assert_eq!(polygons.len(), 1);
    // The place under the box's middle on the globe lies inside, and the place at the center of
    // the far side does not.
    let inside = map.position([200.0, 90.0]).unwrap();
    assert!(avenger_geo::polygon_contains(&polygons, inside));
    assert!(!avenger_geo::polygon_contains(&polygons, [180.0, 0.0]));
}

/// Whether the polygon `ring` holds `point` on the plane, by the even-odd rule.
fn holds(ring: &[[f64; 2]], [x, y]: [f64; 2]) -> bool {
    let mut inside = false;
    for (index, &[xi, yi]) in ring.iter().enumerate() {
        let [xj, yj] = ring[(index + ring.len() - 1) % ring.len()];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
    }
    inside
}

#[test]
fn a_lasso_across_the_antimeridian_splits_there_on_the_plane() {
    let projection = Projection::new(Equirectangular)
        .with_view(180.0 / PI, [180.0, 90.0])
        .with_rotate([180.0, 0.0, 0.0]);
    let map = MapProjection::new(projection.build().unwrap());
    // A square over the antimeridian, at the middle of the map, from 170 east to 170 west.
    let path = [[170.0, 80.0], [190.0, 80.0], [190.0, 100.0], [170.0, 100.0]];
    let polygons = map.enclosed(&path);
    assert_eq!(polygons.len(), 2);
    let held = |place: [f64; 2]| polygons.iter().any(|ring| holds(ring, place));
    assert!(held([175.0, 0.0]) && held([-175.0, 0.0]));
    assert!(!held([0.0, 0.0]));
    for place in [[175.0, 0.0], [-175.0, 0.0]] {
        assert!(
            avenger_geo::polygon_contains(&polygons[..1], place)
                || avenger_geo::polygon_contains(&polygons[1..], place)
        );
    }
}

#[test]
fn a_lasso_around_a_pole_closes_through_it_on_the_plane() {
    let projection = Projection::new(Orthographic)
        .with_view(80.0, [180.0, 90.0])
        .with_rotate([0.0, -60.0, 0.0]);
    let projector = projection.build().unwrap();
    let (x, y) = projector.project_point(0.0, 90.0).unwrap();
    let map = MapProjection::new(projector);
    let [x, y] = [x as f32, y as f32];
    let path = [
        [x - 25.0, y - 25.0],
        [x + 25.0, y - 25.0],
        [x + 25.0, y + 25.0],
        [x - 25.0, y + 25.0],
    ];
    for path in [path.to_vec(), path.iter().rev().copied().collect()] {
        let polygons = map.enclosed(&path);
        let held = |place: [f64; 2]| polygons.iter().any(|ring| holds(ring, place));
        // Every longitude holds the places near the pole, and none at the equator.
        for lon in [-170.0, -90.0, 0.0, 90.0, 170.0] {
            assert!(held([lon, 88.0]), "{lon}");
            assert!(!held([lon, 0.0]), "{lon}");
            let spherical = polygons
                .iter()
                .any(|ring| avenger_geo::polygon_contains(std::slice::from_ref(ring), [lon, 88.0]));
            assert!(spherical, "{lon}");
        }
    }
}
