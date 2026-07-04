//! CRS tags and unit-conversion expressions for CRS-tagged rasters.
//!
//! Rasters produced by `Rasterize2D::frame(...)` are binned in their native
//! CRS units; the Geo mark and examples convert between those units and the
//! authored plane (unrotated Mercator raw units: x = λ radians, y =
//! ln tan(π/4 + φ/2), both in [-π, π]). Conversions are closed-form
//! DataFusion built-ins so the expressions survive the chart's proto
//! serialization of channel programs (same rule as [`crate::expr`]).

use avenger_chart_core::AvengerChartError;
use datafusion::functions::expr_fn::{atan, exp, ln, tan};
use datafusion::logical_expr::{Expr, lit};

/// Web-Mercator meters (the projected CRS used by web tiles and most
/// datashader-style pipelines).
pub const EPSG_3857: &str = "epsg:3857";
/// Longitude/latitude degrees.
pub const EPSG_4326: &str = "epsg:4326";
/// Spherical earth radius used by EPSG:3857 (meters). One authored-plane
/// raw unit equals this many 3857 meters.
pub const WEB_MERCATOR_RADIUS_M: f64 = 6_378_137.0;

fn unknown_crs(crs: &str) -> AvengerChartError {
    AvengerChartError::InvalidArgument(format!(
        "unknown CRS tag '{crs}'; supported: '{EPSG_3857}', '{EPSG_4326}'"
    ))
}

/// Authored-plane raw-unit x → CRS x (3857: meters; 4326: degrees longitude).
pub fn from_mercator_units_x(crs: &str, expr: Expr) -> Result<Expr, AvengerChartError> {
    match crs {
        EPSG_3857 => Ok(expr * lit(WEB_MERCATOR_RADIUS_M)),
        EPSG_4326 => Ok(expr * lit(180.0 / std::f64::consts::PI)),
        other => Err(unknown_crs(other)),
    }
}

/// Authored-plane raw-unit y → CRS y (3857: meters; 4326: degrees latitude
/// via the gudermannian, lat = atan(sinh(y))·180/π).
pub fn from_mercator_units_y(crs: &str, expr: Expr) -> Result<Expr, AvengerChartError> {
    match crs {
        EPSG_3857 => Ok(expr * lit(WEB_MERCATOR_RADIUS_M)),
        EPSG_4326 => {
            // sinh(y) = (exp(y) - exp(-y)) / 2 — no sinh built-in.
            let sinh = (exp(expr.clone()) - exp(lit(0.0) - expr)) / lit(2.0);
            Ok(atan(sinh) * lit(180.0 / std::f64::consts::PI))
        }
        other => Err(unknown_crs(other)),
    }
}

/// CRS x → authored-plane raw-unit x (3857: ÷R; 4326: degrees → radians).
pub fn to_mercator_units_x(crs: &str, expr: Expr) -> Result<Expr, AvengerChartError> {
    match crs {
        EPSG_3857 => Ok(expr / lit(WEB_MERCATOR_RADIUS_M)),
        EPSG_4326 => Ok(expr * lit(std::f64::consts::PI / 180.0)),
        other => Err(unknown_crs(other)),
    }
}

/// CRS y → authored-plane raw-unit y (3857: ÷R; 4326: degrees latitude →
/// ln tan(π/4 + φ/2)).
pub fn to_mercator_units_y(crs: &str, expr: Expr) -> Result<Expr, AvengerChartError> {
    match crs {
        EPSG_3857 => Ok(expr / lit(WEB_MERCATOR_RADIUS_M)),
        EPSG_4326 => {
            let phi = expr * lit(std::f64::consts::PI / 180.0);
            Ok(ln(tan(lit(std::f64::consts::FRAC_PI_4) + phi / lit(2.0))))
        }
        other => Err(unknown_crs(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::array::{ArrayRef, AsArray, Float64Array};
    use datafusion::arrow::datatypes::{DataType, Field, Float64Type, Schema};
    use datafusion::arrow::record_batch::RecordBatch;
    use datafusion::prelude::{SessionContext, col};
    use std::sync::Arc;

    async fn eval(expr: Expr, values: &[f64]) -> Vec<f64> {
        let ctx = SessionContext::new();
        let schema = Arc::new(Schema::new(vec![Field::new("v", DataType::Float64, false)]));
        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(Float64Array::from(values.to_vec())) as ArrayRef],
        )
        .unwrap();
        let df = ctx.read_batch(batch).unwrap();
        let batches = df
            .select(vec![expr.alias("out")])
            .unwrap()
            .collect()
            .await
            .unwrap();
        batches
            .iter()
            .flat_map(|batch| {
                batch
                    .column(0)
                    .as_primitive::<Float64Type>()
                    .values()
                    .iter()
                    .copied()
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn assert_rel_close(actual: f64, expected: f64) {
        let scale = expected.abs().max(1.0);
        assert!(
            (actual - expected).abs() <= 1e-6 * scale,
            "expected {expected}, got {actual}"
        );
    }

    /// §2 worked numbers: raw x −1.292 ↔ 3857 meters −8 240 553.004
    /// (taxi-parquet magnitudes), symmetric for y.
    #[tokio::test]
    async fn epsg_3857_scales_by_earth_radius() {
        let raw = [-1.292, 0.779];
        let meters: Vec<f64> = raw.iter().map(|v| v * WEB_MERCATOR_RADIUS_M).collect();
        assert_rel_close(meters[0], -8_240_553.004);

        let forward_x = eval(from_mercator_units_x(EPSG_3857, col("v")).unwrap(), &raw).await;
        let forward_y = eval(from_mercator_units_y(EPSG_3857, col("v")).unwrap(), &raw).await;
        for (actual, expected) in forward_x
            .iter()
            .chain(&forward_y)
            .zip(meters.iter().chain(&meters))
        {
            assert_rel_close(*actual, *expected);
        }

        let back_x = eval(to_mercator_units_x(EPSG_3857, col("v")).unwrap(), &meters).await;
        let back_y = eval(to_mercator_units_y(EPSG_3857, col("v")).unwrap(), &meters).await;
        for (actual, expected) in back_x.iter().chain(&back_y).zip(raw.iter().chain(&raw)) {
            assert_rel_close(*actual, *expected);
        }
    }

    /// 4326 x is linear degrees↔radians; 4326 y is the (nonlinear)
    /// Mercator/gudermannian pair. Checked against std-lib closed forms and
    /// by round-trip.
    #[tokio::test]
    async fn epsg_4326_converts_degrees() {
        let lons = [-74.0, 0.0, 140.39];
        let raw_x: Vec<f64> = lons.iter().map(|&lon: &f64| lon.to_radians()).collect();
        let actual = eval(to_mercator_units_x(EPSG_4326, col("v")).unwrap(), &lons).await;
        for (actual, expected) in actual.iter().zip(&raw_x) {
            assert_rel_close(*actual, *expected);
        }
        let actual = eval(from_mercator_units_x(EPSG_4326, col("v")).unwrap(), &raw_x).await;
        for (actual, expected) in actual.iter().zip(&lons) {
            assert_rel_close(*actual, *expected);
        }

        let lats = [-35.5, 0.0, 40.75, 66.6];
        let raw_y: Vec<f64> = lats
            .iter()
            .map(|&lat: &f64| {
                (std::f64::consts::FRAC_PI_4 + lat.to_radians() / 2.0)
                    .tan()
                    .ln()
            })
            .collect();
        let actual = eval(to_mercator_units_y(EPSG_4326, col("v")).unwrap(), &lats).await;
        for (actual, expected) in actual.iter().zip(&raw_y) {
            assert_rel_close(*actual, *expected);
        }
        let actual = eval(from_mercator_units_y(EPSG_4326, col("v")).unwrap(), &raw_y).await;
        for (actual, expected) in actual.iter().zip(&lats) {
            assert_rel_close(*actual, *expected);
        }
    }

    #[test]
    fn unknown_tags_error_with_supported_list() {
        for result in [
            from_mercator_units_x("epsg:32618", col("v")),
            from_mercator_units_y("epsg:32618", col("v")),
            to_mercator_units_x("epsg:32618", col("v")),
            to_mercator_units_y("epsg:32618", col("v")),
        ] {
            let err = result.err().expect("unknown CRS must error").to_string();
            assert!(err.contains("unknown CRS tag 'epsg:32618'"), "{err}");
            assert!(
                err.contains("epsg:3857") && err.contains("epsg:4326"),
                "{err}"
            );
        }
    }
}
