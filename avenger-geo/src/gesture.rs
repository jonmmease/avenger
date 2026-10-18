//! Pan and zoom: each gesture keeps the place under the cursor at its start under the cursor.
//! [`Projection::pan`] and [`Projection::zoom`] document the modes.

use crate::error::AvengerGeoError;
use crate::math::{DEGREES, PI, RADIANS, TAU};
use crate::projector::{PanMode, Projection, Projector, Surface, View};

/// The bisection steps that find a globe's edge: enough to reach `f64` precision across a map.
const EDGE_STEPS: usize = 60;

pub(crate) fn pan(
    projection: &Projection,
    mode: PanMode,
    from: [f64; 2],
    to: [f64; 2],
) -> Result<Projection, AvengerGeoError> {
    let (view, start) = checked(projection, mode, &[from, to])?;
    match mode {
        PanMode::Translate => Ok(with_view(
            projection,
            view.scale,
            [
                view.translate[0] + to[0] - from[0],
                view.translate[1] + to[1] - from[1],
            ],
        )),
        PanMode::Longitude => {
            let slid = with_view(
                projection,
                view.scale,
                [view.translate[0], view.translate[1] + to[1] - from[1]],
            );
            let turn = match (start.place_at(from), slid.build()?.place_at(to)) {
                (Some(place), Some(cursor)) => wrap(cursor.0 - place.0),
                _ => (to[0] - from[0]) / view.scale,
            };
            Ok(turned_by_longitude(&slid, turn))
        }
        PanMode::Sphere => Ok(turned_to(projection, &start, &start, from, to)),
    }
}

pub(crate) fn zoom(
    projection: &Projection,
    mode: PanMode,
    anchor: [f64; 2],
    factor: f64,
) -> Result<Projection, AvengerGeoError> {
    let (view, start) = checked(projection, mode, &[anchor])?;
    if !(factor.is_finite() && factor > 0.0) {
        return Err(AvengerGeoError::InvalidConfig(format!(
            "a zoom factor must be finite and positive, not {factor}"
        )));
    }
    let scale = view.scale * factor;
    let scaled_y = anchor[1] - factor * (anchor[1] - view.translate[1]);
    match mode {
        PanMode::Translate => Ok(with_view(
            projection,
            scale,
            [
                anchor[0] - factor * (anchor[0] - view.translate[0]),
                scaled_y,
            ],
        )),
        PanMode::Longitude => {
            // The view keeps its horizontal translation, so the antimeridian stays where pans
            // keep it, and the sphere turns instead.
            let scaled = with_view(projection, scale, [view.translate[0], scaled_y]);
            let turn = match (start.place_at(anchor), scaled.build()?.place_at(anchor)) {
                (Some(place), Some(cursor)) => wrap(cursor.0 - place.0),
                _ => (anchor[0] - view.translate[0]) * (1.0 / scale - 1.0 / view.scale),
            };
            Ok(turned_by_longitude(&scaled, turn))
        }
        PanMode::Sphere => {
            let zoomed = with_view(projection, scale, view.translate);
            Ok(turned_to(&zoomed, &start, &zoomed.build()?, anchor, anchor))
        }
    }
}

/// The projection's view and its projector at the start of the gesture, after checking the
/// gesture's mode and points.
fn checked(
    projection: &Projection,
    mode: PanMode,
    points: &[[f64; 2]],
) -> Result<(View, Projector), AvengerGeoError> {
    let view = projection.view.ok_or_else(|| {
        AvengerGeoError::InvalidConfig(
            "the projection has no view: fit it, or set one with `with_view`".into(),
        )
    })?;
    if matches!(projection.surface, Surface::Planar { .. }) && mode != PanMode::Translate {
        return Err(AvengerGeoError::InvalidConfig(format!(
            "a planar projection pans and zooms only with PanMode::Translate, not {mode:?}"
        )));
    }
    if points.iter().flatten().any(|v| !v.is_finite()) {
        return Err(AvengerGeoError::InvalidConfig(
            "a gesture's points must be finite".into(),
        ));
    }
    Ok((view, projection.build()?))
}

fn with_view(projection: &Projection, scale: f64, translate: [f64; 2]) -> Projection {
    let mut projection = projection.clone();
    projection.view = Some(View { scale, translate });
    projection
}

fn turned_by_longitude(projection: &Projection, turn: f64) -> Projection {
    let mut projection = projection.clone();
    projection.rotate[0] = degrees(projection.rotate[0] * RADIANS + turn);
    projection
}

/// `projection` turned so that the place under `from` in `start` takes the frame position that
/// `target` has under `cursor` in `target`, by the shortest turn of the sphere, as a trackball
/// turns. The turn can roll the sphere. The two projectors share the rotation of `projection`:
/// a zoom's target projector has its new scale. The projection stays unturned when `from`
/// misses the map, or when nothing on the globe lies toward `cursor`. An earth region turns
/// with the place, so only the clips fixed in the frame limit `cursor`.
fn turned_to(
    projection: &Projection,
    start: &Projector,
    target: &Projector,
    from: [f64; 2],
    cursor: [f64; 2],
) -> Projection {
    let Some(place) = start.user_frame_at(from) else {
        return projection.clone();
    };
    let Some(frame) = toward_edge(target, cursor).and_then(|p| target.reachable_frame_at(p)) else {
        return projection.clone();
    };
    let turn = shortest_turn(cartesian(place), cartesian(frame));
    let rotation = product(
        &turn,
        &matrix(projection.rotate.map(|angle| angle * RADIANS)),
    );
    let mut projection = projection.clone();
    projection.rotate = angles(&rotation).map(degrees);
    projection
}

/// `point` when a turn can bring a place there, or else the farthest such point along the line
/// from the frame's center to `point`, or `None` when the center can't take a place either.
fn toward_edge(projector: &Projector, point: [f64; 2]) -> Option<[f64; 2]> {
    if projector.reachable_frame_at(point).is_some() {
        return Some(point);
    }
    let mut inside = projector.frame_origin();
    projector.reachable_frame_at(inside)?;
    let mut outside = point;
    for _ in 0..EDGE_STEPS {
        let middle = [
            (inside[0] + outside[0]) / 2.0,
            (inside[1] + outside[1]) / 2.0,
        ];
        if projector.reachable_frame_at(middle).is_some() {
            inside = middle;
        } else {
            outside = middle;
        }
    }
    Some(inside)
}

type Matrix = [[f64; 3]; 3];

/// A point of the sphere, longitude and latitude in radians, as a unit vector.
fn cartesian((lambda, phi): (f64, f64)) -> [f64; 3] {
    let (sin_phi, cos_phi) = phi.sin_cos();
    [lambda.cos() * cos_phi, lambda.sin() * cos_phi, sin_phi]
}

fn product(a: &Matrix, b: &Matrix) -> Matrix {
    std::array::from_fn(|row| {
        std::array::from_fn(|column| (0..3).map(|k| a[row][k] * b[k][column]).sum())
    })
}

/// The rotation by angles `[λ, φ, γ]` in radians as a matrix of unit vectors, in d3's order: a
/// turn by λ about the z axis, then by φ about the y axis, then by γ about the x axis.
fn matrix([lambda, phi, gamma]: [f64; 3]) -> Matrix {
    let (sl, cl) = lambda.sin_cos();
    let (sp, cp) = phi.sin_cos();
    let (sg, cg) = gamma.sin_cos();
    let a = [[cl, -sl, 0.0], [sl, cl, 0.0], [0.0, 0.0, 1.0]];
    let b = [[cp, 0.0, -sp], [0.0, 1.0, 0.0], [sp, 0.0, cp]];
    let c = [[1.0, 0.0, 0.0], [0.0, cg, -sg], [0.0, sg, cg]];
    product(&c, &product(&b, &a))
}

/// The angles `[λ, φ, γ]` in radians of a rotation [`matrix`]. Where φ is ±90°, λ and γ turn about
/// one axis, and γ is 0.
fn angles(m: &Matrix) -> [f64; 3] {
    let phi = (-m[0][2]).clamp(-1.0, 1.0).asin();
    if m[0][2].abs() < 1.0 - 1e-12 {
        [(-m[0][1]).atan2(m[0][0]), phi, (-m[1][2]).atan2(m[2][2])]
    } else {
        [m[1][0].atan2(m[1][1]), phi, 0.0]
    }
}

/// The shortest rotation that carries unit vector `a` to unit vector `b`, about the axis
/// perpendicular to both. Opposite vectors turn half a turn about an axis perpendicular to `a`.
fn shortest_turn(a: [f64; 3], b: [f64; 3]) -> Matrix {
    let cross = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    let sin = cross.iter().map(|v| v * v).sum::<f64>().sqrt();
    let cos = a.iter().zip(&b).map(|(a, b)| a * b).sum::<f64>();
    let axis = if sin > 1e-12 {
        cross.map(|v| v / sin)
    } else {
        // Any axis perpendicular to `a`: its cross product with the coordinate axis it is least
        // along.
        let other = if a[0].abs() < 0.9 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 0.0, 1.0]
        };
        let axis = [
            a[1] * other[2] - a[2] * other[1],
            a[2] * other[0] - a[0] * other[2],
            a[0] * other[1] - a[1] * other[0],
        ];
        let length = axis.iter().map(|v| v * v).sum::<f64>().sqrt();
        axis.map(|v| v / length)
    };
    let angle = sin.atan2(cos);
    let (sin, cos) = angle.sin_cos();
    let [x, y, z] = axis;
    let k = [[0.0, -z, y], [z, 0.0, -x], [-y, x, 0.0]];
    let k2 = product(&k, &k);
    std::array::from_fn(|row| {
        std::array::from_fn(|column| {
            let identity = if row == column { 1.0 } else { 0.0 };
            identity + sin * k[row][column] + (1.0 - cos) * k2[row][column]
        })
    })
}

/// An angle in radians as degrees in (-180, 180].
fn degrees(angle: f64) -> f64 {
    wrap(angle) * DEGREES
}

/// An angle in radians in (-π, π].
fn wrap(angle: f64) -> f64 {
    let angle = angle.rem_euclid(TAU);
    if angle > PI {
        angle - TAU
    } else {
        angle
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rotation::Rotation;

    const ANGLES: [[f64; 3]; 5] = [
        [0.0, 0.0, 0.0],
        [0.4, -0.3, 0.0],
        [-2.0, 1.1, 0.7],
        [3.0, -1.4, -2.5],
        [1.0, PI / 2.0, 0.3],
    ];

    fn near(a: [f64; 3], b: [f64; 3]) -> bool {
        a.iter().zip(&b).all(|(a, b)| (a - b).abs() < 1e-12)
    }

    fn apply(m: &Matrix, v: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|row| (0..3).map(|k| m[row][k] * v[k]).sum())
    }

    /// The matrix turns points as the pipeline's rotation does.
    #[test]
    fn matrices_rotate_as_the_rotation_does() {
        for angles in ANGLES {
            let rotation = Rotation::new(angles[0], angles[1], angles[2]);
            for place in [(0.0, 0.0), (1.2, 0.5), (-2.9, -1.2), (0.3, 1.5)] {
                let (lambda, phi) = rotation.rotate(place.0, place.1);
                let turned = apply(&matrix(angles), cartesian(place));
                assert!(
                    near(turned, cartesian((lambda, phi))),
                    "{angles:?} {place:?}"
                );
            }
        }
    }

    /// Angles read from a matrix give the same matrix, at ±90° too.
    #[test]
    fn angles_round_trip_through_matrices() {
        for angles_in in ANGLES {
            let m = matrix(angles_in);
            let back = matrix(angles(&m));
            for row in 0..3 {
                assert!(near(m[row], back[row]), "{angles_in:?}");
            }
        }
    }

    #[test]
    fn shortest_turns_carry_one_vector_to_another() {
        let a = cartesian((0.3, 0.4));
        for b in [cartesian((1.9, -0.7)), a, a.map(|v| -v)] {
            assert!(near(apply(&shortest_turn(a, b), a), b), "{b:?}");
        }
        let x = [1.0, 0.0, 0.0];
        assert!(near(
            apply(&shortest_turn(x, x.map(|v| -v)), x),
            x.map(|v| -v)
        ));
    }
}
