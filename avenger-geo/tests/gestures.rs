//! Pan and zoom keep the place under the cursor at the start of a gesture under the cursor.

use avenger_geo::projections::{
    Aitoff, AzimuthalEqualArea, AzimuthalEquidistant, ConicConformal, ConicEqualArea,
    ConicEquidistant, CylindricalEqualArea, Eckert4, EqualEarth, Equirectangular, Gnomonic, Hammer,
    Mercator, Miller, Mollweide, NaturalEarth1, Orthographic, Robinson, Sinusoidal, Stereographic,
    TransverseMercator, WinkelTripel,
};
use avenger_geo::{PanMode, Projection, RegionFrame, SphericalClip};

const CENTER: [f64; 2] = [400.0, 300.0];

/// Every built-in projection, with whether a pan in its default mode keeps the place under
/// the cursor exactly: a longitude pan does when the projection's y depends on latitude only.
fn projections() -> Vec<(Projection, bool)> {
    let p = |projection: Projection, exact: bool| (projection.with_view(150.0, CENTER), exact);
    vec![
        p(Projection::new(Equirectangular), true),
        p(Projection::new(Mercator), true),
        p(Projection::new(Miller), true),
        p(Projection::new(CylindricalEqualArea::default()), true),
        p(Projection::new(EqualEarth), true),
        p(Projection::new(NaturalEarth1), true),
        p(Projection::new(Robinson), true),
        p(Projection::new(Mollweide), true),
        p(Projection::new(Sinusoidal), true),
        p(Projection::new(Eckert4), true),
        p(Projection::new(WinkelTripel), false),
        p(Projection::new(Hammer::default()), false),
        p(Projection::new(Aitoff), false),
        p(Projection::new(TransverseMercator), false),
        p(Projection::new(ConicEqualArea::albers()), false),
        p(Projection::new(ConicConformal::new([30.0, 60.0])), false),
        p(Projection::new(ConicEquidistant::new([30.0, 60.0])), false),
        p(
            Projection::new(Orthographic).with_rotate([-30.0, -20.0, 0.0]),
            true,
        ),
        p(
            Projection::new(Stereographic).with_rotate([-30.0, -20.0, 0.0]),
            true,
        ),
        p(
            Projection::new(Gnomonic).with_rotate([-30.0, -20.0, 0.0]),
            true,
        ),
        p(Projection::new(AzimuthalEqualArea), true),
        p(Projection::new(AzimuthalEquidistant), true),
    ]
}

fn place(projection: &Projection, point: [f64; 2]) -> (f64, f64) {
    projection
        .build()
        .unwrap()
        .invert(point[0], point[1])
        .unwrap()
}

fn position(projection: &Projection, (lon, lat): (f64, f64)) -> [f64; 2] {
    let (x, y) = projection.build().unwrap().project_point(lon, lat).unwrap();
    [x, y]
}

fn assert_near(a: [f64; 2], b: [f64; 2], tolerance: f64, label: &str) {
    assert!(
        (a[0] - b[0]).abs() < tolerance && (a[1] - b[1]).abs() < tolerance,
        "{label}: {a:?} vs {b:?}"
    );
}

#[test]
fn projections_pan_in_the_mode_of_their_default_clip() {
    // The last five, the azimuthal projections, clip to circles.
    let modes: Vec<PanMode> = projections()
        .iter()
        .map(|(projection, _)| projection.pan_mode())
        .collect();
    let (antimeridian, circle) = modes.split_at(modes.len() - 5);
    assert!(antimeridian.iter().all(|mode| *mode == PanMode::Longitude));
    assert!(circle.iter().all(|mode| *mode == PanMode::Sphere));
    assert_eq!(Projection::planar(true).pan_mode(), PanMode::Translate);
}

#[test]
fn pans_keep_the_place_under_the_cursor() {
    let (from, to) = ([420.0, 290.0], [470.0, 325.0]);
    for (projection, exact) in projections() {
        let label = format!("{:?}", projection.surface);
        let start = place(&projection, from);
        let panned = projection.pan(projection.pan_mode(), from, to).unwrap();
        if exact {
            assert_near(position(&panned, start), to, 1e-6, &label);
        } else {
            // The longitude under the cursor is the place's. The latitude drifts by less than a
            // fifth of the drag's length on the sphere.
            let (lon, lat) = place(&panned, to);
            assert!(
                (lon - start.0).abs() < 1e-6,
                "{label}: {lon} vs {}",
                start.0
            );
            let drag = (to[0] - from[0]).hypot(to[1] - from[1]) / 150.0;
            assert!(
                (lat - start.1).abs() < 0.2 * drag.to_degrees(),
                "{label}: {lat} vs {}",
                start.1
            );
        }
        // A longitude pan turns about the axis and slides, so it never rolls the sphere.
        if projection.pan_mode() == PanMode::Longitude {
            assert_eq!(panned.rotate[2], projection.rotate[2], "{label}");
        }
    }
}

#[test]
fn zooms_keep_the_place_under_the_anchor() {
    let anchor = [440.0, 270.0];
    for (projection, exact) in projections() {
        let label = format!("{:?}", projection.surface);
        let start = place(&projection, anchor);
        for factor in [2.5, 0.8] {
            let mode = projection.pan_mode();
            let zoomed = projection.zoom(mode, anchor, factor).unwrap();
            let view = zoomed.view.unwrap();
            assert!((view.scale - 150.0 * factor).abs() < 1e-9, "{label}");
            match mode {
                PanMode::Sphere => assert_eq!(view.translate, CENTER, "{label}"),
                // The antimeridian stays where pans keep it.
                _ => assert_eq!(view.translate[0], CENTER[0], "{label}"),
            }
            if exact {
                assert_near(position(&zoomed, start), anchor, 1e-6, &label);
            } else {
                // As a longitude pan turns, the longitude under the anchor is the place's.
                let (lon, _) = place(&zoomed, anchor);
                assert!(
                    (lon - start.0).abs() < 1e-6,
                    "{label}: {lon} vs {}",
                    start.0
                );
            }
        }
    }
}

#[test]
fn translate_pans_slide_the_view() {
    let projection = Projection::planar(true).with_view(2.0, [10.0, 20.0]);
    let panned = projection
        .pan(PanMode::Translate, [5.0, 5.0], [8.0, 1.0])
        .unwrap();
    assert_eq!(panned.view.unwrap().translate, [13.0, 16.0]);
    let zoomed = projection
        .zoom(PanMode::Translate, [0.0, 0.0], 3.0)
        .unwrap();
    assert_eq!(zoomed.view.unwrap().translate, [30.0, 60.0]);
}

/// Off a world map's outline, a longitude pan turns by the cylindrical relation.
#[test]
fn longitude_pans_off_the_map_turn_by_the_distance() {
    let projection = Projection::new(EqualEarth).with_view(100.0, [0.0, 0.0]);
    let panned = projection
        .pan(PanMode::Longitude, [-390.0, -190.0], [-340.0, -190.0])
        .unwrap();
    assert!((panned.rotate[0] - 50.0_f64.to_degrees() / 100.0).abs() < 1e-9);
    assert_eq!(panned.view, projection.view);
}

/// A cursor off the globe pulls the place to the globe's edge, toward the cursor.
#[test]
fn sphere_pans_off_the_globe_reach_its_edge() {
    let projection = Projection::new(Orthographic).with_view(100.0, [0.0, 0.0]);
    let start = place(&projection, [10.0, 0.0]);
    let panned = projection
        .pan(PanMode::Sphere, [10.0, 0.0], [500.0, 0.0])
        .unwrap();
    assert_near(position(&panned, start), [100.0, 0.0], 1e-6, "edge");
}

#[test]
fn sphere_pans_that_start_off_the_globe_do_not_turn_it() {
    let projection = Projection::new(Orthographic)
        .with_view(100.0, [0.0, 0.0])
        .with_rotate([20.0, 10.0, 5.0]);
    let panned = projection
        .pan(PanMode::Sphere, [150.0, 0.0], [0.0, 0.0])
        .unwrap();
    assert_eq!(panned.rotate, projection.rotate);
}

/// A sphere pan turns the globe as a trackball does, so a place follows the cursor anywhere on
/// the globe: a pole across the globe, and a place far from the equator to the globe's side,
/// which a turn without a roll can't reach.
#[test]
fn sphere_pans_carry_any_place_anywhere_on_the_globe() {
    let projection = Projection::new(Orthographic)
        .with_view(100.0, [0.0, 0.0])
        .with_rotate([0.0, -80.0, 0.0]);
    let pole = position(&projection, (0.0, 90.0));
    for to in [[pole[0] + 60.0, pole[1]], [-70.0, 40.0], [0.0, 95.0]] {
        let panned = projection.pan(PanMode::Sphere, pole, to).unwrap();
        assert_near(position(&panned, (0.0, 90.0)), to, 1e-6, "pole");
    }
    let projection = Projection::new(Orthographic).with_view(100.0, [0.0, 0.0]);
    let north = position(&projection, (0.0, 60.0));
    // A turn without a roll keeps 60°N within half the radius of the vertical center line.
    let side = [80.0, -50.0];
    let panned = projection.pan(PanMode::Sphere, north, side).unwrap();
    assert_near(position(&panned, (0.0, 60.0)), side, 1e-6, "side");
}

/// A roll angle and a center move the frame that a sphere gesture turns in, and the place still
/// stays under the cursor.
#[test]
fn sphere_gestures_keep_the_place_with_a_roll_and_a_center() {
    let projection = Projection::new(Orthographic)
        .with_view(150.0, CENTER)
        .with_rotate([-30.0, -20.0, 25.0])
        .with_center(5.0, -8.0);
    let (from, to) = ([420.0, 290.0], [470.0, 325.0]);
    let start = place(&projection, from);
    let panned = projection.pan(PanMode::Sphere, from, to).unwrap();
    assert_near(position(&panned, start), to, 1e-6, "pan");
    let anchor = [440.0, 270.0];
    let start = place(&projection, anchor);
    for factor in [2.5, 0.8] {
        let zoomed = projection.zoom(PanMode::Sphere, anchor, factor).unwrap();
        assert_near(position(&zoomed, start), anchor, 1e-6, "zoom");
    }
}

/// A longitude pan keeps the place exactly with a center too.
#[test]
fn longitude_pans_keep_the_place_with_a_center() {
    let projection = Projection::new(EqualEarth)
        .with_view(150.0, CENTER)
        .with_rotate([40.0, 0.0, 0.0])
        .with_center(10.0, 20.0);
    let (from, to) = ([420.0, 290.0], [470.0, 325.0]);
    let start = place(&projection, from);
    let panned = projection.pan(PanMode::Longitude, from, to).unwrap();
    assert_near(position(&panned, start), to, 1e-6, "pan");
}

/// A small drag turns the globe a little, whatever its roll.
#[test]
fn small_sphere_drags_turn_the_globe_a_little() {
    for rotate in [
        [-30.0, -20.0, 0.0],
        [-30.0, -20.0, 25.0],
        [100.0, 60.0, 90.0],
    ] {
        let projection = Projection::new(Orthographic)
            .with_view(150.0, CENTER)
            .with_rotate(rotate);
        let panned = projection
            .pan(PanMode::Sphere, [420.0, 290.0], [420.5, 290.5])
            .unwrap();
        let turned = panned
            .rotate
            .iter()
            .zip(&rotate)
            .map(|(after, before)| ((after - before + 540.0) % 360.0 - 180.0).abs())
            .fold(0.0, f64::max);
        assert!(turned < 1.0, "{rotate:?} -> {:?}", panned.rotate);
    }
}

/// A zoom out that leaves the anchor off the globe pulls the place to the globe's edge, toward
/// the anchor.
#[test]
fn sphere_zooms_out_pull_the_place_to_the_edge() {
    let projection = Projection::new(Orthographic).with_view(100.0, [0.0, 0.0]);
    let start = place(&projection, [90.0, 0.0]);
    let zoomed = projection.zoom(PanMode::Sphere, [90.0, 0.0], 0.5).unwrap();
    assert_near(position(&zoomed, start), [50.0, 0.0], 1e-6, "edge");
}

/// An earth region turns with the globe, so it doesn't stop a sphere pan from carrying a place
/// in it under the cursor.
#[test]
fn sphere_pans_turn_earth_regions_with_the_globe() {
    let region = SphericalClip::lon_lat_box(RegionFrame::Earth, 10.0, 10.0, 30.0, 30.0).unwrap();
    let projection = Projection::new(Orthographic)
        .with_view(100.0, [0.0, 0.0])
        .with_clip(Some(region));
    let from = position(&projection, (20.0, 20.0));
    let to = [from[0] + 20.0, from[1]];
    let panned = projection.pan(PanMode::Sphere, from, to).unwrap();
    assert_near(position(&panned, (20.0, 20.0)), to, 1e-6, "pan");
}

#[test]
fn invalid_gestures_are_errors() {
    let projection = Projection::new(EqualEarth).with_view(100.0, [0.0, 0.0]);
    assert!(Projection::new(EqualEarth)
        .pan(PanMode::Longitude, [0.0, 0.0], [1.0, 1.0])
        .is_err());
    assert!(projection
        .pan(PanMode::Longitude, [f64::NAN, 0.0], [1.0, 1.0])
        .is_err());
    for factor in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(projection
            .zoom(PanMode::Longitude, [0.0, 0.0], factor)
            .is_err());
    }
    // A projection that doesn't build fails in every mode.
    let unbuilt = projection.clone().with_precision(f64::NAN);
    for mode in [PanMode::Translate, PanMode::Longitude, PanMode::Sphere] {
        assert!(unbuilt.pan(mode, [0.0, 0.0], [1.0, 1.0]).is_err());
        assert!(unbuilt.zoom(mode, [0.0, 0.0], 2.0).is_err());
    }
    let planar = Projection::planar(false).with_view(1.0, [0.0, 0.0]);
    for mode in [PanMode::Longitude, PanMode::Sphere] {
        assert!(planar.pan(mode, [0.0, 0.0], [1.0, 1.0]).is_err());
        assert!(planar.zoom(mode, [0.0, 0.0], 2.0).is_err());
    }
}
