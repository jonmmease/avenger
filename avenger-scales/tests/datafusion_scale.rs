use std::collections::HashMap;
use std::sync::Arc;

use arrow::array::{
    Array, ArrayRef, AsArray, Date32Array, Float32Array, Float64Array, Int64Array, StringArray,
    StructArray,
};
use arrow::compute::{cast, concat};
use arrow::datatypes::{DataType, Field, Fields, Float64Type, Schema};
use arrow::record_batch::RecordBatch;
use avenger_scales::datafusion::{config_literal, scale_expr, ScaleExtensionCodec};
use avenger_scales::error::AvengerScaleError;
use avenger_scales::scalar::Scalar;
use avenger_scales::scales::band::BandScale;
use avenger_scales::scales::linear::LinearScale;
use avenger_scales::scales::log::LogScale;
use avenger_scales::scales::nested_band::NestedBandScale;
use avenger_scales::scales::ordinal::OrdinalScale;
use avenger_scales::scales::point::PointScale;
use avenger_scales::scales::pow::PowScale;
use avenger_scales::scales::quantile::QuantileScale;
use avenger_scales::scales::quantize::QuantizeScale;
use avenger_scales::scales::symlog::SymlogScale;
use avenger_scales::scales::threshold::ThresholdScale;
use avenger_scales::scales::time::TimeScale;
use avenger_scales::scales::{ConfiguredScale, DomainKind, ScaleConfig, ScaleContext, ScaleImpl};
use datafusion::common::{Result, ScalarValue};
use datafusion::logical_expr::{col, create_udf, lit, ColumnarValue, Expr, ScalarFunctionArgs};
use datafusion::logical_expr::{ScalarUDF, Volatility};
use datafusion::prelude::SessionContext;
use datafusion_proto::logical_plan::from_proto::parse_expr;
use datafusion_proto::logical_plan::to_proto::serialize_expr;
use datafusion_proto::logical_plan::{DefaultLogicalExtensionCodec, LogicalExtensionCodec};
use serde::{Deserialize, Serialize};

fn numbers(values: &[f64]) -> ArrayRef {
    Arc::new(Float64Array::from(values.to_vec()))
}

fn strings(values: &[&str]) -> ArrayRef {
    Arc::new(StringArray::from(values.to_vec()))
}

/// A configuration as a literal.
fn config(domain: ArrayRef, range: ArrayRef, options: &HashMap<String, Scalar>) -> Expr {
    config_literal(&ScaleConfig {
        domain,
        range,
        options: options.clone(),
        context: ScaleContext::default(),
    })
    .unwrap()
}

/// `scale` with a configuration from literals, applied to the column `value`.
fn expr(
    scale: &dyn ScaleImpl,
    domain: ArrayRef,
    range: ArrayRef,
    options: &HashMap<String, Scalar>,
) -> Expr {
    scale_expr(scale, config(domain, range, options), col("value")).unwrap()
}

async fn evaluate(expr: Expr, values: ArrayRef) -> Result<ArrayRef> {
    let schema = Arc::new(Schema::new(vec![Field::new(
        "value",
        values.data_type().clone(),
        true,
    )]));
    let batch = RecordBatch::try_new(schema, vec![values])?;
    let batches = SessionContext::new()
        .read_batch(batch)?
        .select(vec![expr])?
        .collect()
        .await?;
    Ok(batches[0].column(0).clone())
}

async fn evaluate_scalar(expr: Expr) -> Result<ScalarValue> {
    let batches = SessionContext::new()
        .read_empty()?
        .select(vec![expr])?
        .collect()
        .await?;
    ScalarValue::try_from_array(batches[0].column(0), 0)
}

/// The query's result is the scale's own result on the values cast to its input type.
async fn assert_matches_kernel(
    scale: Arc<dyn ScaleImpl>,
    domain: ArrayRef,
    range: ArrayRef,
    options: HashMap<String, Scalar>,
    values: ArrayRef,
) {
    let actual = evaluate(
        expr(scale.as_ref(), domain.clone(), range.clone(), &options),
        values.clone(),
    )
    .await
    .unwrap();
    let values = match scale.domain_kind() {
        DomainKind::Numeric => cast(&values, &DataType::Float64).unwrap(),
        _ => values,
    };
    let expected = ConfiguredScale {
        scale_impl: scale.clone(),
        config: ScaleConfig {
            domain,
            range,
            options,
            context: ScaleContext::default(),
        },
    }
    .scale(&values)
    .unwrap();
    assert_eq!(
        actual.to_data(),
        expected.to_data(),
        "{}",
        scale.scale_type()
    );
}

#[tokio::test]
async fn queries_match_the_scales_and_their_output_types() {
    let sqrt = HashMap::from([("exponent".to_string(), Scalar::from(0.5))]);
    let continuous: [(Arc<dyn ScaleImpl>, HashMap<String, Scalar>); 5] = [
        (Arc::new(LinearScale), HashMap::new()),
        (Arc::new(LogScale), HashMap::new()),
        (Arc::new(PowScale), HashMap::new()),
        (Arc::new(PowScale), sqrt),
        (Arc::new(SymlogScale), HashMap::new()),
    ];
    for (scale, options) in continuous {
        let values: ArrayRef = Arc::new(Int64Array::from(vec![1, 25, 100]));
        assert_matches_kernel(
            scale,
            numbers(&[1.0, 100.0]),
            numbers(&[0.0, 400.0]),
            options,
            values,
        )
        .await;
    }
    let paths = |groups: &[&str], items: &[&str]| -> ArrayRef {
        Arc::new(StructArray::new(
            Fields::from(vec![
                Field::new("group", DataType::Utf8, true),
                Field::new("item", DataType::Utf8, true),
            ]),
            vec![strings(groups), strings(items)],
            None,
        ))
    };
    let discrete: [(Arc<dyn ScaleImpl>, ArrayRef, ArrayRef, ArrayRef); 9] = [
        (
            Arc::new(OrdinalScale),
            strings(&["a", "b"]),
            strings(&["red", "blue"]),
            strings(&["b", "a", "missing"]),
        ),
        (
            Arc::new(BandScale),
            strings(&["a", "b"]),
            numbers(&[0.0, 200.0]),
            strings(&["a", "b", "missing"]),
        ),
        (
            Arc::new(PointScale),
            strings(&["a", "b"]),
            numbers(&[0.0, 200.0]),
            strings(&["a", "b", "missing"]),
        ),
        (
            Arc::new(NestedBandScale),
            paths(&["x", "x", "y"], &["1", "2", "1"]),
            numbers(&[0.0, 300.0]),
            paths(&["y", "x"], &["1", "2"]),
        ),
        (
            Arc::new(ThresholdScale),
            numbers(&[0.5]),
            strings(&["low", "high"]),
            numbers(&[0.25, 0.5, 0.75]),
        ),
        (
            Arc::new(QuantileScale),
            numbers(&[0.0, 0.25, 0.5, 0.75, 1.0]),
            strings(&["low", "high"]),
            numbers(&[0.0, 0.5, 1.0]),
        ),
        (
            Arc::new(QuantizeScale),
            numbers(&[0.0, 1.0]),
            strings(&["low", "high"]),
            numbers(&[0.0, 0.5, 1.0]),
        ),
        (
            Arc::new(TimeScale),
            Arc::new(Date32Array::from(vec![0, 10])),
            numbers(&[0.0, 100.0]),
            Arc::new(Date32Array::from(vec![0, 5, 10])),
        ),
        (
            Arc::new(BandScale),
            Arc::new(Int64Array::from(vec![1, 2])),
            numbers(&[0.0, 200.0]),
            Arc::new(Int64Array::from(vec![2, 1])),
        ),
    ];
    for (scale, domain, range, values) in discrete {
        assert_matches_kernel(scale, domain, range, HashMap::new(), values).await;
    }
}

#[tokio::test]
async fn results_keep_nulls_scalars_and_colors() {
    let colors = Arc::new(StringArray::from(vec![Some("red"), None]));
    let ordinal = expr(
        &OrdinalScale,
        strings(&["a", "b"]),
        colors.clone(),
        &HashMap::new(),
    );
    let actual = evaluate(ordinal, strings(&["a", "b"])).await.unwrap();
    assert_eq!(actual.to_data(), colors.to_data());
    for (input, expected) in [("a", Some("red")), ("b", None), ("missing", None)] {
        let config = config(strings(&["a", "b"]), colors.clone(), &HashMap::new());
        let ordinal = scale_expr(&OrdinalScale, config, lit(input)).unwrap();
        assert_eq!(
            evaluate_scalar(ordinal).await.unwrap(),
            ScalarValue::Utf8(expected.map(str::to_owned))
        );
    }
    let clamp = HashMap::from([("clamp".to_string(), Scalar::from_bool(true))]);
    let config = config(numbers(&[0.0, 1.0]), numbers(&[0.0, 100.0]), &clamp);
    let clamped = scale_expr(&LinearScale, config, lit(2.0_f64)).unwrap();
    assert_eq!(
        evaluate_scalar(clamped).await.unwrap(),
        ScalarValue::Float32(Some(100.0))
    );
    let linear = expr(
        &LinearScale,
        numbers(&[0.0, 1.0]),
        numbers(&[0.0, 100.0]),
        &HashMap::new(),
    );
    let actual = evaluate(linear, Arc::new(Float64Array::from(vec![None, Some(0.5)])))
        .await
        .unwrap();
    assert!(actual.is_null(0));
    assert_eq!(
        actual
            .as_primitive::<arrow::datatypes::Float32Type>()
            .value(1),
        50.0
    );
    for scale in [&LinearScale as &dyn ScaleImpl, &PowScale] {
        let colored = expr(
            scale,
            numbers(&[0.0, 1.0]),
            strings(&["red", "blue"]),
            &HashMap::new(),
        );
        let actual = evaluate(colored, numbers(&[0.0, 1.0])).await.unwrap();
        assert_eq!(
            actual.data_type(),
            &DataType::new_list(DataType::Float32, true)
        );
        assert_eq!(
            actual.as_list::<i32>().value(0).to_data(),
            Float32Array::from(vec![1.0, 0.0, 0.0, 1.0]).to_data()
        );
    }
}

/// Invokes the linear scale function on arguments that may vary within a batch.
fn invoke(args: Vec<ColumnarValue>, rows: usize) -> Result<ColumnarValue> {
    let linear = expr(
        &LinearScale,
        numbers(&[0.0, 1.0]),
        numbers(&[0.0, 1.0]),
        &HashMap::new(),
    );
    let Expr::ScalarFunction(function) = linear else {
        panic!("a scale applies through a scalar function")
    };
    let arg_fields = args
        .iter()
        .map(|arg| Arc::new(Field::new("", arg.data_type(), true)))
        .collect();
    function.func.invoke_with_args(ScalarFunctionArgs {
        args,
        arg_fields,
        number_rows: rows,
        return_field: Arc::new(Field::new("", DataType::Float32, true)),
        config_options: Arc::default(),
    })
}

fn literal(expr: Expr) -> ScalarValue {
    let Expr::Literal(value, _) = expr else {
        panic!("a literal")
    };
    value
}

#[test]
fn rows_scale_with_their_own_configurations() {
    let ends = |end: f64| {
        literal(config(
            numbers(&[0.0, end]),
            numbers(&[0.0, 100.0]),
            &HashMap::new(),
        ))
    };
    let (one, two) = (ends(1.0), ends(2.0));
    let missing = ScalarValue::try_from(one.data_type()).unwrap();
    let configs = [&one, &two, &missing, &one].map(|config| config.to_array().unwrap());
    let configs = concat(
        &configs
            .iter()
            .map(|config| config.as_ref())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let args = vec![
        ColumnarValue::Array(configs),
        ColumnarValue::Scalar(0.5_f64.into()),
    ];
    let ColumnarValue::Array(result) = invoke(args, 4).unwrap() else {
        panic!("an array")
    };
    // A row without a configuration has no result.
    assert_eq!(
        result.to_data(),
        Float32Array::from(vec![Some(50.0), Some(25.0), None, Some(50.0)]).to_data()
    );
    let args = vec![
        ColumnarValue::Array(one.to_array_of_size(0).unwrap()),
        ColumnarValue::Array(numbers(&[])),
    ];
    let ColumnarValue::Array(result) = invoke(args, 0).unwrap() else {
        panic!("an array")
    };
    assert_eq!((result.len(), result.data_type()), (0, &DataType::Float32));
    assert!(invoke(vec![], 1).is_err());
}

#[tokio::test]
async fn invalid_configurations_are_errors() {
    let unknown = HashMap::from([("unknown".to_string(), Scalar::from_bool(true))]);
    for (domain, range, options) in [
        (numbers(&[0.0]), numbers(&[0.0, 1.0]), HashMap::new()),
        (numbers(&[0.0, 1.0]), numbers(&[]), HashMap::new()),
        (numbers(&[0.0, 1.0]), numbers(&[0.0, 1.0]), unknown),
    ] {
        let linear = expr(&LinearScale, domain, range, &options);
        assert!(evaluate(linear, numbers(&[0.0])).await.is_err());
    }
    // Categorical values have the domain's type.
    let ordinal = expr(
        &OrdinalScale,
        Arc::new(Int64Array::from(vec![1])),
        strings(&["a"]),
        &HashMap::new(),
    );
    assert!(evaluate(ordinal, numbers(&[1.0])).await.is_err());
    // A band scale maps onto a numeric range.
    let band = expr(
        &BandScale,
        strings(&["a"]),
        strings(&["red"]),
        &HashMap::new(),
    );
    assert!(evaluate(band, strings(&["a"])).await.is_err());
}

/// A scale defined outside avenger-scales whose configuration is a field: it adds an offset.
#[derive(Debug, Serialize, Deserialize)]
struct OffsetScale {
    offset: f64,
}

#[typetag::serde(name = "offset")]
impl ScaleImpl for OffsetScale {
    fn scale_type(&self) -> &'static str {
        "offset"
    }

    fn domain_kind(&self) -> DomainKind {
        DomainKind::Numeric
    }

    fn output_type(&self, _range_type: &DataType) -> Result<DataType, AvengerScaleError> {
        Ok(DataType::Float64)
    }

    fn scale(
        &self,
        _config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError> {
        let values = values.as_primitive::<Float64Type>();
        Ok(Arc::new(
            values.unary::<_, Float64Type>(|v| v + self.offset),
        ))
    }
}

fn round_trip(expr: &Expr, codec: &dyn LogicalExtensionCodec) -> Result<Expr> {
    let ctx = SessionContext::new();
    let proto = serialize_expr(expr, codec)?;
    Ok(parse_expr(&proto, ctx.task_ctx().as_ref(), codec)?)
}

#[tokio::test]
async fn plans_decode_their_scales_without_registration() {
    let codec = ScaleExtensionCodec::default();
    let at = |scale: &dyn ScaleImpl| {
        let config = config(
            numbers(&[0.0, 1.0]),
            numbers(&[0.0, 100.0]),
            &HashMap::new(),
        );
        scale_expr(scale, config, lit(0.25_f64)).unwrap()
    };
    let original = [
        at(&LinearScale),
        at(&OffsetScale { offset: 10.0 }),
        at(&OffsetScale { offset: 20.0 }),
    ];
    let restored: Vec<Expr> = original
        .iter()
        .map(|expr| round_trip(expr, &codec).unwrap())
        .collect();
    assert_eq!(original.as_slice(), restored.as_slice());
    // The configuration of a scale is part of its function's identity.
    assert_ne!(restored[1], restored[2]);
    let mut values = Vec::new();
    for expr in restored {
        values.push(evaluate_scalar(expr).await.unwrap());
    }
    assert_eq!(
        values,
        [
            ScalarValue::Float32(Some(25.0)),
            ScalarValue::Float64(Some(10.25)),
            ScalarValue::Float64(Some(20.25))
        ]
    );
}

#[test]
fn payloads_of_other_versions_scales_and_functions_are_errors() {
    let codec = ScaleExtensionCodec::default();
    let linear = expr(
        &LinearScale,
        numbers(&[0.0, 1.0]),
        numbers(&[0.0, 1.0]),
        &HashMap::new(),
    );
    let Expr::ScalarFunction(function) = linear else {
        panic!("a scale applies through a scalar function")
    };
    let mut bytes = Vec::new();
    codec.try_encode_udf(&function.func, &mut bytes).unwrap();
    assert!(codec.try_decode_udf("linear_scale", &bytes).is_ok());
    assert!(codec.try_decode_udf("log_scale", &bytes).is_err());
    assert!(codec
        .try_decode_udf("linear_scale", &bytes[..bytes.len() - 1])
        .is_err());
    let split = bytes.iter().position(|byte| *byte == b'{').unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&bytes[split..]).unwrap();
    let with = |change: &dyn Fn(&mut serde_json::Value)| {
        let mut payload = payload.clone();
        change(&mut payload);
        [&bytes[..split], &serde_json::to_vec(&payload).unwrap()].concat()
    };
    assert!(codec
        .try_decode_udf(
            "linear_scale",
            &with(&|p| p["version"] = "avenger-scales/2".into())
        )
        .is_err());
    assert!(codec
        .try_decode_udf(
            "linear_scale",
            &with(&|p| p["scale"]["type"] = "unknown".into())
        )
        .is_err());
}

#[test]
fn other_functions_go_to_the_fallback_codec() {
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
            schema: arrow::datatypes::SchemaRef,
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
    // A function named like a scale function is still another function.
    let mut bytes = Vec::new();
    codec
        .try_encode_udf(&function("linear_scale"), &mut bytes)
        .unwrap();
    assert_eq!(bytes, b"fallback");
    assert_eq!(
        codec.try_decode_udf("linear_scale", &bytes).unwrap().name(),
        "linear_scale"
    );
}
