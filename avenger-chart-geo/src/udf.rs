//! The `geo_project` DataFusion UDF: `(lon, lat) -> struct { x, y }` in the
//! authored projection's raw planar units (rotation applied, y-up).
//!
//! Because general projections are not separable, mark position builders
//! bind `x`/`y` to fields of this UDF's output instead of per-channel
//! closed-form expressions (the WebMercator trick). Registered per
//! projection configuration; the UDF name carries a stable hash of the
//! config so distinct projections coexist in one `SessionContext`.

use std::any::Any;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use avenger_geo::projector::Projection;
use datafusion::arrow::array::{Array, ArrayRef, Float64Array, StructArray};
use datafusion::arrow::datatypes::{DataType, Field, Fields};
use datafusion::common::Result as DataFusionResult;
use datafusion::error::DataFusionError;
use datafusion::logical_expr::{
    ColumnarValue, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Volatility,
};
use datafusion::prelude::SessionContext;

fn xy_fields() -> Fields {
    Fields::from(vec![
        Field::new("x", DataType::Float64, true),
        Field::new("y", DataType::Float64, true),
    ])
}

/// Stable name for the UDF of a projection configuration (kind + rotate).
pub fn geo_project_udf_name(projection: &Projection) -> String {
    let mut hasher = DefaultHasher::new();
    serde_json::to_string(&projection.kind)
        .expect("serializable projection kind")
        .hash(&mut hasher);
    for r in projection.rotate {
        r.to_bits().hash(&mut hasher);
    }
    format!("geo_project_{:016x}", hasher.finish())
}

#[derive(Debug)]
pub struct GeoProjectUdf {
    name: String,
    projection: Projection,
    signature: Signature,
}

impl GeoProjectUdf {
    pub fn new(projection: &Projection) -> Self {
        GeoProjectUdf {
            name: geo_project_udf_name(projection),
            projection: projection.clone(),
            signature: Signature::exact(
                vec![DataType::Float64, DataType::Float64],
                Volatility::Immutable,
            ),
        }
    }
}

impl ScalarUDFImpl for GeoProjectUdf {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> DataFusionResult<DataType> {
        Ok(DataType::Struct(xy_fields()))
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DataFusionResult<ColumnarValue> {
        let arrays = ColumnarValue::values_to_arrays(&args.args)?;
        let lon = arrays[0]
            .as_any()
            .downcast_ref::<Float64Array>()
            .ok_or_else(|| {
                DataFusionError::Internal("geo_project expects Float64 longitude".to_string())
            })?;
        let lat = arrays[1]
            .as_any()
            .downcast_ref::<Float64Array>()
            .ok_or_else(|| {
                DataFusionError::Internal("geo_project expects Float64 latitude".to_string())
            })?;

        let len = lon.len();
        let mut xs = Vec::with_capacity(len);
        let mut ys = Vec::with_capacity(len);
        for i in 0..len {
            if lon.is_null(i) || lat.is_null(i) {
                xs.push(None);
                ys.push(None);
                continue;
            }
            let (x, y) = self
                .projection
                .project_raw_units(lon.value(i), lat.value(i));
            if x.is_finite() && y.is_finite() {
                xs.push(Some(x));
                ys.push(Some(y));
            } else {
                xs.push(None);
                ys.push(None);
            }
        }

        let x_array: ArrayRef = Arc::new(Float64Array::from(xs));
        let y_array: ArrayRef = Arc::new(Float64Array::from(ys));
        let fields = xy_fields();
        let strct = StructArray::new(fields, vec![x_array, y_array], None);
        Ok(ColumnarValue::Array(Arc::new(strct)))
    }
}

/// Register the projection's UDF on the context (idempotent by name) and
/// return it.
pub fn register_geo_project_udf(ctx: &SessionContext, projection: &Projection) -> ScalarUDF {
    let udf = ScalarUDF::from(GeoProjectUdf::new(projection));
    ctx.register_udf(udf.clone());
    udf
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_geo::raw::ProjectionKind;
    use datafusion::arrow::array::Float64Array;

    #[tokio::test]
    async fn udf_matches_projector_and_propagates_nulls() {
        let projection = Projection::new(ProjectionKind::albers()).with_rotate([96.0, 0.0, 0.0]);
        let udf = ScalarUDF::from(GeoProjectUdf::new(&projection));

        let lon = Float64Array::from(vec![Some(-98.0), None, Some(-75.0)]);
        let lat = Float64Array::from(vec![Some(38.5), Some(10.0), None]);
        let args = ScalarFunctionArgs {
            args: vec![
                ColumnarValue::Array(Arc::new(lon)),
                ColumnarValue::Array(Arc::new(lat)),
            ],
            arg_fields: vec![
                Arc::new(Field::new("lon", DataType::Float64, true)),
                Arc::new(Field::new("lat", DataType::Float64, true)),
            ],
            number_rows: 3,
            return_field: Arc::new(Field::new("out", DataType::Struct(xy_fields()), true)),
        };
        let result = udf.inner().invoke_with_args(args).expect("invoke");
        let ColumnarValue::Array(array) = result else {
            panic!("expected array result");
        };
        let strct = array.as_any().downcast_ref::<StructArray>().unwrap();
        let xs = strct
            .column(0)
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        let ys = strct
            .column(1)
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();

        let (ex, ey) = projection.project_raw_units(-98.0, 38.5);
        assert!((xs.value(0) - ex).abs() < 1e-12);
        assert!((ys.value(0) - ey).abs() < 1e-12);
        assert!(xs.is_null(1) && ys.is_null(1));
        assert!(xs.is_null(2) && ys.is_null(2));
    }

    #[test]
    fn udf_name_is_stable_and_config_sensitive() {
        let a = Projection::new(ProjectionKind::EqualEarth);
        let b = Projection::new(ProjectionKind::EqualEarth);
        let c = Projection::new(ProjectionKind::EqualEarth).with_rotate([15.0, 0.0, 0.0]);
        assert_eq!(geo_project_udf_name(&a), geo_project_udf_name(&b));
        assert_ne!(geo_project_udf_name(&a), geo_project_udf_name(&c));
    }
}
