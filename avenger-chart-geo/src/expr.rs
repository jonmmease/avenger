//! Closed-form projection expressions over DataFusion built-ins.
//!
//! Mark position channels bind `x`/`y` to these expression trees
//! (three-axis rotation + the raw projection forward math, all in terms of
//! `sin`/`cos`/`asin`/`atan2`/`ln`/`power`/`CASE`). Unlike a UDF, built-in
//! expressions survive the chart's proto serialization of channel programs
//! with no session registration anywhere. Numeric parity with
//! [`avenger_geo::Projection::project_raw_units`] is enforced by test.
//!
//! The [`crate::udf::GeoProjectUdf`] remains available as an escape hatch
//! for projections that cannot be expressed with built-ins (it requires
//! registering the UDF on the `SessionContext` before `Plot::compile`).

use avenger_geo::math::{HALF_PI, PI, RADIANS, TAU};
use avenger_geo::projector::Projection;
use avenger_geo::raw::ProjectionKind;
use datafusion::functions::expr_fn::{atan2, cos, ln, power, sin, tan};
use datafusion::logical_expr::{Expr, lit, when};

fn asin_expr(x: Expr) -> Expr {
    // DataFusion's asin; clamp to [-1, 1] like d3 to avoid NaN at the
    // numerical edge.
    let clamped = when(x.clone().gt(lit(1.0)), lit(1.0))
        .when(x.clone().lt(lit(-1.0)), lit(-1.0))
        .otherwise(x)
        .expect("asin clamp case");
    datafusion::functions::expr_fn::asin(clamped)
}

/// Spherical radians after the projection's three-axis rotation.
fn rotated_lambda_phi(projection: &Projection, lon: Expr, lat: Expr) -> (Expr, Expr) {
    let lam = lon * lit(RADIANS);
    let phi = lat * lit(RADIANS);

    let [dl_deg, dp_deg, dg_deg] = projection.rotate;
    let delta_lambda = (dl_deg * RADIANS) % TAU;
    let delta_phi = dp_deg * RADIANS;
    let delta_gamma = dg_deg * RADIANS;

    // λ += Δλ, wrapped into [-π, π] (d3 forwardRotationLambda).
    let lam = if delta_lambda != 0.0 {
        let shifted = lam + lit(delta_lambda);
        when(shifted.clone().gt(lit(PI)), shifted.clone() - lit(TAU))
            .when(shifted.clone().lt(lit(-PI)), shifted.clone() + lit(TAU))
            .otherwise(shifted)
            .expect("lambda wrap case")
    } else {
        lam
    };

    if delta_phi == 0.0 && delta_gamma == 0.0 {
        return (lam, phi);
    }

    // d3 rotationPhiGamma.
    let cos_dp = lit(delta_phi.cos());
    let sin_dp = lit(delta_phi.sin());
    let cos_dg = lit(delta_gamma.cos());
    let sin_dg = lit(delta_gamma.sin());

    let cos_phi = cos(phi.clone());
    let x = cos(lam.clone()) * cos_phi.clone();
    let y = sin(lam) * cos_phi;
    let z = sin(phi);
    let k = z.clone() * cos_dp.clone() + x.clone() * sin_dp.clone();

    let lambda2 = atan2(
        y.clone() * cos_dg.clone() - k.clone() * sin_dg.clone(),
        x * cos_dp - z * sin_dp,
    );
    let phi2 = asin_expr(k * cos_dg + y * sin_dg);
    (lambda2, phi2)
}

/// Build `(x, y)` raw-planar-unit expressions for the projection.
/// Mirrors the raw forward math in `avenger-geo/src/raw/`.
pub fn geo_position_exprs(projection: &Projection, lon: Expr, lat: Expr) -> (Expr, Expr) {
    if projection.kind.is_identity() {
        let reflect = matches!(
            projection.kind,
            ProjectionKind::Identity { reflect_y: true }
        );
        let y = if reflect { lit(-1.0) * lat } else { lat };
        return (lon, y);
    }
    let (lam, phi) = rotated_lambda_phi(projection, lon, lat);
    raw_forward_exprs(&projection.kind, lam, phi)
}

fn raw_forward_exprs(kind: &ProjectionKind, lam: Expr, phi: Expr) -> (Expr, Expr) {
    match kind {
        ProjectionKind::Equirectangular => (lam, phi),
        ProjectionKind::Mercator => {
            // y = ln(tan((π/2 + φ) / 2))
            let y = ln(tan((lit(HALF_PI) + phi) / lit(2.0)));
            (lam, y)
        }
        ProjectionKind::EqualEarth => {
            const A1: f64 = 1.340264;
            const A2: f64 = -0.081106;
            const A3: f64 = 0.000893;
            const A4: f64 = 0.003796;
            let m = 3.0_f64.sqrt() / 2.0;
            let l = asin_expr(lit(m) * sin(phi));
            let l2 = l.clone() * l.clone();
            let l6 = l2.clone() * l2.clone() * l2.clone();
            let x_den = lit(m)
                * (lit(A1)
                    + lit(3.0 * A2) * l2.clone()
                    + l6.clone() * (lit(7.0 * A3) + lit(9.0 * A4) * l2.clone()));
            let x = lam * cos(l.clone()) / x_den;
            let y = l * (lit(A1) + lit(A2) * l2.clone() + l6 * (lit(A3) + lit(A4) * l2));
            (x, y)
        }
        ProjectionKind::NaturalEarth1 => {
            let phi2 = phi.clone() * phi.clone();
            let phi4 = phi2.clone() * phi2.clone();
            let x = lam
                * (lit(0.8707) - lit(0.131979) * phi2.clone()
                    + phi4.clone()
                        * (lit(-0.013791)
                            + phi4.clone()
                                * (lit(0.003971) * phi2.clone() - lit(0.001529) * phi4.clone())));
            let y = phi
                * (lit(1.007226)
                    + phi2.clone()
                        * (lit(0.015085)
                            + phi4.clone()
                                * (lit(-0.044475) + lit(0.028874) * phi2 - lit(0.005916) * phi4)));
            (x, y)
        }
        ProjectionKind::WinkelTripel => {
            // Mean of aitoff and equirectangular-with-standard-parallel.
            let cos_phi = cos(phi.clone());
            let half_lam = lam.clone() / lit(2.0);
            // alpha = acos(cos φ · cos(λ/2)); sinci(alpha) = alpha / sin(alpha)
            let ca = cos_phi.clone() * cos(half_lam.clone());
            let ca = when(ca.clone().gt(lit(1.0)), lit(1.0))
                .when(ca.clone().lt(lit(-1.0)), lit(-1.0))
                .otherwise(ca)
                .expect("acos clamp case");
            let alpha = datafusion::functions::expr_fn::acos(ca);
            let sinci = when(alpha.clone().eq(lit(0.0)), lit(1.0))
                .otherwise(alpha.clone() / sin(alpha.clone()))
                .expect("sinci case");
            let aitoff_x = lit(2.0) * cos_phi * sin(half_lam) * sinci.clone();
            let aitoff_y = sin(phi.clone()) * sinci;
            let x = (aitoff_x + lam / lit(HALF_PI)) / lit(2.0);
            let y = (aitoff_y + phi) / lit(2.0);
            (x, y)
        }
        ProjectionKind::ConicEqualArea { parallels } => {
            let y0 = parallels.0 * RADIANS;
            let y1 = parallels.1 * RADIANS;
            let sy0 = y0.sin();
            let n = (sy0 + y1.sin()) / 2.0;
            if n.abs() < avenger_geo::math::EPSILON {
                // Degenerate: cylindrical equal-area.
                let cos_phi0 = y0.cos();
                return (lam * lit(cos_phi0), sin(phi) / lit(cos_phi0));
            }
            let c = 1.0 + sy0 * (2.0 * n - sy0);
            let r0 = c.sqrt() / n;
            // r = sqrt(max(c − 2n·sinφ, 0)) / n
            let under = lit(c) - lit(2.0 * n) * sin(phi);
            let under = when(under.clone().lt(lit(0.0)), lit(0.0))
                .otherwise(under)
                .expect("sqrt clamp case");
            let r = datafusion::functions::expr_fn::sqrt(under) / lit(n);
            let nl = lam * lit(n);
            (r.clone() * sin(nl.clone()), lit(r0) - r * cos(nl))
        }
        ProjectionKind::ConicConformal { parallels } => {
            let y0 = parallels.0 * RADIANS;
            let y1 = parallels.1 * RADIANS;
            let cy0 = y0.cos();
            let tany = |y: f64| ((HALF_PI + y) / 2.0).tan();
            let n = if y0 == y1 {
                y0.sin()
            } else {
                (cy0 / y1.cos()).ln() / (tany(y1) / tany(y0)).ln()
            };
            if n == 0.0 || !n.is_finite() {
                // Degenerate: mercator.
                let y = ln(tan((lit(HALF_PI) + phi) / lit(2.0)));
                return (lam, y);
            }
            let f = cy0 * tany(y0).powf(n) / n;
            // Latitude clamp keeps the antipodal pole finite (matches the
            // Rust raw).
            let eps = avenger_geo::math::EPSILON;
            let phi = if f > 0.0 {
                when(phi.clone().lt(lit(-HALF_PI + eps)), lit(-HALF_PI + eps))
                    .otherwise(phi)
                    .expect("phi clamp case")
            } else {
                when(phi.clone().gt(lit(HALF_PI - eps)), lit(HALF_PI - eps))
                    .otherwise(phi)
                    .expect("phi clamp case")
            };
            let r = lit(f) / power(tan((lit(HALF_PI) + phi) / lit(2.0)), lit(n));
            let nl = lam * lit(n);
            (r.clone() * sin(nl.clone()), lit(f) - r * cos(nl))
        }
        ProjectionKind::Identity { .. } => unreachable!("handled by caller"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::array::Float64Array;
    use datafusion::arrow::datatypes::{DataType, Field, Schema};
    use datafusion::arrow::record_batch::RecordBatch;
    use datafusion::prelude::{SessionContext, col};
    use std::sync::Arc;

    /// Expression math must match the engine within tight tolerance.
    #[tokio::test]
    async fn exprs_match_projector() {
        let mut lons = Vec::new();
        let mut lats = Vec::new();
        let mut lon = -170.0_f64;
        while lon <= 170.0 {
            let mut lat = -80.0_f64;
            while lat <= 80.0 {
                lons.push(lon);
                lats.push(lat);
                lat += 20.0;
            }
            lon += 30.0;
        }

        let ctx = SessionContext::new();
        let schema = Arc::new(Schema::new(vec![
            Field::new("lon", DataType::Float64, false),
            Field::new("lat", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(lons.clone())),
                Arc::new(Float64Array::from(lats.clone())),
            ],
        )
        .unwrap();

        for (kind, rotate) in [
            (ProjectionKind::Equirectangular, [0.0, 0.0, 0.0]),
            (ProjectionKind::Mercator, [15.0, 0.0, 0.0]),
            (ProjectionKind::EqualEarth, [0.0, 0.0, 0.0]),
            (ProjectionKind::EqualEarth, [15.0, -30.0, 12.0]),
            (ProjectionKind::NaturalEarth1, [96.0, 0.0, 0.0]),
            (ProjectionKind::WinkelTripel, [0.0, 0.0, 0.0]),
            (ProjectionKind::albers(), [96.0, 0.0, 0.0]),
            (
                ProjectionKind::ConicConformal {
                    parallels: (35.0, 65.0),
                },
                [-15.0, 0.0, 0.0],
            ),
            (
                ProjectionKind::Identity { reflect_y: true },
                [0.0, 0.0, 0.0],
            ),
        ] {
            let projection = Projection::new(kind.clone()).with_rotate(rotate);
            let (x_expr, y_expr) = geo_position_exprs(&projection, col("lon"), col("lat"));
            let df = ctx
                .read_batch(batch.clone())
                .unwrap()
                .select(vec![x_expr.alias("x"), y_expr.alias("y")])
                .unwrap();
            let results = df.collect().await.unwrap();
            let result = &results[0];
            let xs = result
                .column(0)
                .as_any()
                .downcast_ref::<Float64Array>()
                .unwrap();
            let ys = result
                .column(1)
                .as_any()
                .downcast_ref::<Float64Array>()
                .unwrap();
            for i in 0..lons.len() {
                let (ex, ey) = projection.project_raw_units(lons[i], lats[i]);
                assert!(
                    (xs.value(i) - ex).abs() < 1e-9 && (ys.value(i) - ey).abs() < 1e-9,
                    "{kind:?} rotate {rotate:?} at ({}, {}): expr ({}, {}), engine ({ex}, {ey})",
                    lons[i],
                    lats[i],
                    xs.value(i),
                    ys.value(i),
                );
            }
        }
    }
}
