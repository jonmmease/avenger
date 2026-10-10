use std::{collections::HashMap, sync::Arc};

use avenger_scales_datafusion::{
    avenger_scales::{
        error::AvengerScaleError,
        scales::{DomainKind, InferDomainFromDataMethod, RangeKind, ScaleConfig, ScaleImpl},
    },
    create_scale_udf, list_literal, options_literal, scale_expr, BuiltinScale, ScaleExtensionCodec,
    ScaleSpec,
};
use datafusion::{
    arrow::{
        array::{ArrayRef, Float64Array},
        datatypes::DataType,
    },
    common::Result,
    logical_expr::{lit, Expr},
    prelude::SessionContext,
};
use datafusion_proto::logical_plan::{
    from_proto::parse_expr, to_proto::serialize_expr, LogicalExtensionCodec,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct OffsetSpec {
    offset: f64,
}

#[derive(Debug)]
struct OffsetKernel(f64);

impl ScaleImpl for OffsetKernel {
    fn scale_type(&self) -> &'static str {
        "offset"
    }
    fn infer_domain_from_data_method(&self) -> InferDomainFromDataMethod {
        InferDomainFromDataMethod::Interval
    }
    fn domain_kind(&self) -> DomainKind {
        DomainKind::Numeric
    }
    fn range_kind(&self) -> RangeKind {
        RangeKind::Continuous
    }
    fn scale(
        &self,
        _config: &ScaleConfig,
        values: &ArrayRef,
    ) -> std::result::Result<ArrayRef, AvengerScaleError> {
        let values = values.as_any().downcast_ref::<Float64Array>().unwrap();
        Ok(Arc::new(
            values
                .iter()
                .map(|v| v.map(|v| v + self.0))
                .collect::<Float64Array>(),
        ))
    }
}

#[typetag::serde(name = "test_offset_v1")]
impl ScaleSpec for OffsetSpec {
    fn create_impl(&self) -> Result<Arc<dyn ScaleImpl>> {
        Ok(Arc::new(OffsetKernel(self.offset)))
    }
    fn output_type(&self, _: &DataType) -> Result<DataType> {
        Ok(DataType::Float64)
    }
    fn input_type(&self, _: &DataType, _: &DataType) -> Result<DataType> {
        Ok(DataType::Float64)
    }
}

fn expr(spec: impl ScaleSpec + 'static) -> Result<Expr> {
    scale_expr(
        spec,
        list_literal(Arc::new(Float64Array::from(vec![0.0, 1.0])))?,
        list_literal(Arc::new(Float64Array::from(vec![0.0, 100.0])))?,
        options_literal(&HashMap::new())?,
        lit(0.25_f64),
    )
}

#[tokio::test]
async fn builtins_and_custom_descriptors_round_trip_without_registration() -> Result<()> {
    let codec = ScaleExtensionCodec::default();
    let original = vec![
        expr(BuiltinScale::Linear)?,
        expr(BuiltinScale::Sqrt)?,
        expr(OffsetSpec { offset: 10.0 })?,
        expr(OffsetSpec { offset: 20.0 })?,
    ];
    let ctx = SessionContext::new();
    let restored = original
        .iter()
        .map(|expression| -> Result<Expr> {
            Ok(parse_expr(
                &serialize_expr(expression, &codec)?,
                ctx.task_ctx().as_ref(),
                &codec,
            )?)
        })
        .collect::<Result<Vec<_>>>()?;
    assert_eq!(original, restored);
    assert_ne!(restored[2], restored[3]);
    let restored: Vec<_> = restored
        .into_iter()
        .enumerate()
        .map(|(i, expr)| expr.alias(format!("x{i}")))
        .collect();
    let result = ctx.read_empty()?.select(restored)?.collect().await?;
    use datafusion::common::ScalarValue;
    let values = result[0]
        .columns()
        .iter()
        .map(|column| ScalarValue::try_from_array(column, 0))
        .collect::<Result<Vec<_>>>()?;
    assert_eq!(
        values,
        vec![
            ScalarValue::Float32(Some(25.0)),
            ScalarValue::Float32(Some(50.0)),
            ScalarValue::Float64(Some(10.25)),
            ScalarValue::Float64(Some(20.25))
        ]
    );
    Ok(())
}

#[test]
fn malformed_unknown_and_incompatible_payloads_fail() -> Result<()> {
    assert!(create_scale_udf(OffsetSpec { offset: f64::NAN }).is_err());
    let codec = ScaleExtensionCodec::default();
    let udf = create_scale_udf(BuiltinScale::Linear)?;
    let mut bytes = Vec::new();
    codec.try_encode_udf(&udf, &mut bytes)?;
    assert!(codec.try_decode_udf("other", &bytes).is_err());
    assert!(codec
        .try_decode_udf("scale", &bytes[..bytes.len() - 1])
        .is_err());
    assert!(codec.try_decode_udf("scale", b"unknown").is_err());
    let split = bytes.iter().position(|byte| *byte == b'{').unwrap();
    let mut payload: serde_json::Value = serde_json::from_slice(&bytes[split..]).unwrap();
    let mut bad = bytes[..split].to_vec();
    for version in ["avenger-scales-datafusion/1", "future"] {
        payload["version"] = version.into();
        bad.truncate(split);
        bad.extend_from_slice(&serde_json::to_vec(&payload).unwrap());
        assert!(codec.try_decode_udf("scale", &bad).is_err());
    }
    payload["version"] = avenger_scales_datafusion::SCALE_FUNCTION_VERSION.into();
    payload["spec"]["type"] = "missing_custom_type".into();
    bad.truncate(split);
    bad.extend_from_slice(&serde_json::to_vec(&payload).unwrap());
    assert!(codec.try_decode_udf("scale", &bad).is_err());
    Ok(())
}

#[test]
fn codec_delegates_unrelated_functions() -> Result<()> {
    use datafusion::{
        common::ScalarValue,
        logical_expr::{create_udf, ColumnarValue, ScalarUDF, Volatility},
    };
    use datafusion_proto::logical_plan::DefaultLogicalExtensionCodec;
    #[derive(Debug)]
    struct Fallback;
    impl LogicalExtensionCodec for Fallback {
        fn try_encode(
            &self,
            node: &datafusion::logical_expr::Extension,
            buf: &mut Vec<u8>,
        ) -> Result<()> {
            DefaultLogicalExtensionCodec {}.try_encode(node, buf)
        }
        fn try_decode(
            &self,
            buf: &[u8],
            inputs: &[datafusion::logical_expr::LogicalPlan],
            ctx: &datafusion::execution::TaskContext,
        ) -> Result<datafusion::logical_expr::Extension> {
            DefaultLogicalExtensionCodec {}.try_decode(buf, inputs, ctx)
        }
        fn try_encode_table_provider(
            &self,
            reference: &datafusion::common::TableReference,
            table: Arc<dyn datafusion::catalog::TableProvider>,
            buf: &mut Vec<u8>,
        ) -> Result<()> {
            DefaultLogicalExtensionCodec {}.try_encode_table_provider(reference, table, buf)
        }
        fn try_decode_table_provider(
            &self,
            buf: &[u8],
            reference: &datafusion::common::TableReference,
            schema: datafusion::arrow::datatypes::SchemaRef,
            ctx: &datafusion::execution::TaskContext,
        ) -> Result<Arc<dyn datafusion::catalog::TableProvider>> {
            DefaultLogicalExtensionCodec {}.try_decode_table_provider(buf, reference, schema, ctx)
        }
        fn try_encode_udf(&self, _: &ScalarUDF, buf: &mut Vec<u8>) -> Result<()> {
            buf.extend_from_slice(b"fallback");
            Ok(())
        }
        fn try_decode_udf(&self, name: &str, buf: &[u8]) -> Result<Arc<ScalarUDF>> {
            assert_eq!(buf, b"fallback");
            Ok(Arc::new(function(name)))
        }
    }
    fn function(name: &str) -> ScalarUDF {
        create_udf(
            name,
            vec![],
            DataType::Int64,
            Volatility::Immutable,
            Arc::new(|_| Ok(ColumnarValue::Scalar(ScalarValue::Int64(Some(1))))),
        )
    }
    let codec = ScaleExtensionCodec::with_fallback(Arc::new(Fallback));
    // Matching the function name is insufficient: only our implementation is encoded.
    let udf = function("scale");
    let mut bytes = Vec::new();
    codec.try_encode_udf(&udf, &mut bytes)?;
    assert_eq!(bytes, b"fallback");
    assert_eq!(codec.try_decode_udf("scale", &bytes)?.name(), "scale");
    Ok(())
}
