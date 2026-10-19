//! Projections defined outside the crate, as another crate adds them.

use avenger_geo::{Projection, ProjectionImpl};
use serde::{Deserialize, Serialize};

/// Sinusoidal, which relies on the default numerical inverse.
#[derive(Debug, Serialize, Deserialize)]
struct Sinusoidal;

#[typetag::serde(name = "test_sinusoidal")]
impl ProjectionImpl for Sinusoidal {
    fn project(&self, lambda: f64, phi: f64) -> (f64, f64) {
        (lambda * phi.cos(), phi)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct NonFiniteBounds;

#[typetag::serde(name = "test_non_finite_bounds")]
impl ProjectionImpl for NonFiniteBounds {
    fn project(&self, lambda: f64, phi: f64) -> (f64, f64) {
        (lambda, phi)
    }

    fn bounds(&self) -> Option<[[f64; 2]; 2]> {
        Some([[0.0, 0.0], [f64::NAN, 1.0]])
    }
}

#[test]
fn projections_from_other_crates_fit_serialize_and_invert() {
    let mut projection = Projection::new(Sinusoidal);
    let summary = projection.summarize_sphere().unwrap();
    projection
        .fit([[0.0, 0.0], [640.0, 320.0]], &summary)
        .unwrap();
    let json = serde_json::to_string(&projection).unwrap();
    assert!(json.contains(r#""type":"test_sinusoidal""#), "{json}");
    let projector = serde_json::from_str::<Projection>(&json)
        .unwrap()
        .build()
        .unwrap();

    // The world is twice as wide as it is tall, so it fills the extent.
    let (x, y) = projector.project_point(0.0, 0.0).unwrap();
    assert!(
        (x - 320.0).abs() < 1e-6 && (y - 160.0).abs() < 1e-6,
        "({x}, {y})"
    );
    let (x, _) = projector.project_point(180.0, 0.0).unwrap();
    assert!((x - 640.0).abs() < 1e-6, "{x}");

    let (x, y) = projector.project_point(60.0, 45.0).unwrap();
    let (lon, lat) = projector.invert(x, y).unwrap();
    assert!(
        (lon - 60.0).abs() < 1e-6 && (lat - 45.0).abs() < 1e-6,
        "({lon}, {lat})"
    );
}

#[test]
fn non_finite_bounds_do_not_build() {
    let projection = Projection::new(NonFiniteBounds).with_view(1.0, [0.0, 0.0]);
    assert!(projection.build().is_err());
}
