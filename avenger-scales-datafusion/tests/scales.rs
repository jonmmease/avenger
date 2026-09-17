use std::{collections::HashMap, sync::Arc};

use avenger_scales_datafusion::{
    avenger_scales::{
        scalar::Scalar,
        scales::{ConfiguredScale, ScaleConfig, ScaleContext},
    },
    create_scale_udf, list_literal, options_literal, scale_expr, BuiltinScale, ScaleSpec,
};
use datafusion::{
    arrow::{
        array::{Array, ArrayRef, Date32Array, Float32Array, Float64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    common::{Result, ScalarValue},
    logical_expr::{col, lit, ColumnarValue, Expr, ScalarFunctionArgs},
    prelude::SessionContext,
};

fn numbers(values: &[f64]) -> ArrayRef {
    Arc::new(Float64Array::from(values.to_vec()))
}
fn strings(values: &[&str]) -> ArrayRef {
    Arc::new(StringArray::from(values.to_vec()))
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

#[tokio::test]
async fn numeric_kernels_match_direct_execution_and_keep_declared_output_type() -> Result<()> {
    for kind in [
        BuiltinScale::Linear,
        BuiltinScale::Log,
        BuiltinScale::Pow,
        BuiltinScale::Sqrt,
        BuiltinScale::Symlog,
    ] {
        let domain = numbers(&[1.0, 100.0]);
        let range = numbers(&[0.0, 400.0]);
        let values = numbers(&[1.0, 25.0, 100.0]);
        let expression = scale_expr(
            kind,
            list_literal(domain.clone())?,
            list_literal(range.clone())?,
            options_literal(&HashMap::new())?,
            col("value"),
        )?;
        let actual = evaluate(expression, values.clone()).await?;
        let scale = ConfiguredScale {
            scale_impl: kind.create_impl()?,
            config: ScaleConfig {
                domain,
                range,
                options: kind.default_options(),
                context: ScaleContext::default(),
            },
        };
        let expected = scale.scale(&values).unwrap();
        assert_eq!(actual.to_data(), expected.to_data(), "{kind:?}");
        assert_eq!(actual.data_type(), &DataType::Float32);
    }
    Ok(())
}

#[tokio::test]
async fn categorical_discrete_and_temporal_scales_match_kernels() -> Result<()> {
    for kind in [
        BuiltinScale::Ordinal,
        BuiltinScale::Band,
        BuiltinScale::Point,
        BuiltinScale::Threshold,
        BuiltinScale::Quantile,
        BuiltinScale::Quantize,
        BuiltinScale::Time,
    ] {
        let (domain, range, values): (ArrayRef, ArrayRef, ArrayRef) = match kind {
            BuiltinScale::Ordinal => (
                strings(&["a", "b"]),
                strings(&["red", "blue"]),
                strings(&["b", "a", "missing"]),
            ),
            BuiltinScale::Band | BuiltinScale::Point => (
                strings(&["a", "b"]),
                numbers(&[0.0, 200.0]),
                strings(&["a", "b", "missing"]),
            ),
            BuiltinScale::Threshold => (
                numbers(&[0.5]),
                strings(&["low", "high"]),
                numbers(&[0.25, 0.5, 0.75]),
            ),
            BuiltinScale::Quantile => (
                numbers(&[0.0, 0.25, 0.5, 0.75, 1.0]),
                strings(&["low", "high"]),
                numbers(&[0.0, 0.5, 1.0]),
            ),
            BuiltinScale::Quantize => (
                numbers(&[0.0, 1.0]),
                strings(&["low", "high"]),
                numbers(&[0.0, 0.5, 1.0]),
            ),
            BuiltinScale::Time => (
                Arc::new(Date32Array::from(vec![0, 10])),
                numbers(&[0.0, 100.0]),
                Arc::new(Date32Array::from(vec![0, 5, 10])),
            ),
            _ => unreachable!(),
        };
        let expression = scale_expr(
            kind,
            list_literal(domain.clone())?,
            list_literal(range.clone())?,
            options_literal(&HashMap::new())?,
            col("value"),
        )?;
        let actual = evaluate(expression, values.clone()).await?;
        let input_type = kind.input_type(domain.data_type(), values.data_type())?;
        let values = datafusion::arrow::compute::cast(&values, &input_type)?;
        let scale = ConfiguredScale {
            scale_impl: kind.create_impl()?,
            config: ScaleConfig {
                domain,
                range,
                options: kind.default_options(),
                context: ScaleContext::default(),
            },
        };
        assert_eq!(
            actual.to_data(),
            scale.scale(&values).unwrap().to_data(),
            "{kind:?}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn scalar_null_options_and_color_results() -> Result<()> {
    let expression = scale_expr(
        BuiltinScale::Ordinal,
        list_literal(strings(&["a", "b"]))?,
        list_literal(Arc::new(StringArray::from(vec![Some("red"), None])))?,
        options_literal(&HashMap::new())?,
        col("value"),
    )?;
    let actual = evaluate(expression, strings(&["a", "b"])).await?;
    assert_eq!(
        actual.data_type(),
        &DataType::Dictionary(Box::new(DataType::Int16), Box::new(DataType::Utf8))
    );
    assert_eq!(
        datafusion::arrow::compute::cast(&actual, &DataType::Utf8)?.to_data(),
        StringArray::from(vec![Some("red"), None]).to_data()
    );
    let expression = scale_expr(
        BuiltinScale::Linear,
        list_literal(numbers(&[0.0, 1.0]))?,
        list_literal(numbers(&[0.0, 100.0]))?,
        options_literal(&HashMap::from([("clamp".into(), Scalar::from_bool(true))]))?,
        lit(2.0_f64),
    )?;
    let batches = SessionContext::new()
        .read_empty()?
        .select(vec![expression])?
        .collect()
        .await?;
    assert_eq!(
        ScalarValue::try_from_array(batches[0].column(0), 0)?,
        ScalarValue::Float32(Some(100.0))
    );
    let expression = scale_expr(
        BuiltinScale::Linear,
        list_literal(numbers(&[0.0, 1.0]))?,
        list_literal(numbers(&[0.0, 100.0]))?,
        options_literal(&HashMap::new())?,
        col("value"),
    )?;
    let actual = evaluate(
        expression,
        Arc::new(Float64Array::from(vec![None, Some(0.5)])),
    )
    .await?;
    assert!(actual.is_null(0));
    assert_eq!(
        ScalarValue::try_from_array(&actual, 1)?,
        ScalarValue::Float32(Some(50.0))
    );

    for kind in [BuiltinScale::Linear, BuiltinScale::Pow] {
        let expression = scale_expr(
            kind,
            list_literal(numbers(&[0.0, 1.0]))?,
            list_literal(strings(&["red", "blue"]))?,
            options_literal(&HashMap::new())?,
            col("value"),
        )?;
        let actual = evaluate(expression, numbers(&[0.0, 1.0])).await?;
        assert_eq!(
            actual.data_type(),
            &DataType::new_list(DataType::Float32, true)
        );
        let list = actual
            .as_any()
            .downcast_ref::<datafusion::arrow::array::ListArray>()
            .unwrap();
        assert_eq!(
            list.value(0).to_data(),
            Float32Array::from(vec![1.0, 0.0, 0.0, 1.0]).to_data()
        );
    }
    Ok(())
}

#[tokio::test]
async fn scalar_dictionary_results_preserve_values_and_logical_nulls() -> Result<()> {
    for (input, expected) in [("a", Some("red")), ("b", None), ("missing", None)] {
        let expression = scale_expr(
            BuiltinScale::Ordinal,
            list_literal(strings(&["a", "b"]))?,
            list_literal(Arc::new(StringArray::from(vec![Some("red"), None])))?,
            options_literal(&HashMap::new())?,
            lit(input),
        )?;
        let batches = SessionContext::new()
            .read_empty()?
            .select(vec![expression])?
            .collect()
            .await?;
        assert_eq!(
            ScalarValue::try_from_array(batches[0].column(0), 0)?,
            ScalarValue::Dictionary(
                Box::new(DataType::Int16),
                Box::new(ScalarValue::Utf8(expected.map(str::to_owned))),
            )
        );
    }
    Ok(())
}

fn invoke(args: Vec<ColumnarValue>, rows: usize) -> Result<ColumnarValue> {
    let udf = create_scale_udf(BuiltinScale::Linear)?;
    let fields = args
        .iter()
        .map(|arg| Arc::new(Field::new("", arg.data_type(), true)))
        .collect();
    udf.invoke_with_args(ScalarFunctionArgs {
        args,
        arg_fields: fields,
        number_rows: rows,
        return_field: Arc::new(Field::new("", DataType::Float32, true)),
        config_options: Arc::default(),
    })
}

fn literal(expr: Expr) -> ScalarValue {
    let Expr::Literal(value, _) = expr else {
        panic!("expected literal")
    };
    value
}

#[test]
fn broadcast_configuration_is_validated_and_empty_batches_preserve_type() -> Result<()> {
    let domain = literal(list_literal(numbers(&[0.0, 1.0]))?);
    let range = literal(list_literal(numbers(&[0.0, 100.0]))?);
    let options = literal(options_literal(&HashMap::new())?);
    let args = vec![
        ColumnarValue::Array(domain.to_array_of_size(2)?),
        ColumnarValue::Array(range.to_array_of_size(2)?),
        ColumnarValue::Array(options.to_array_of_size(2)?),
        ColumnarValue::Scalar(0.5_f64.into()),
    ];
    let ColumnarValue::Array(result) = invoke(args, 2)? else {
        panic!("array")
    };
    assert_eq!(
        result.to_data(),
        Float32Array::from(vec![50.0, 50.0]).to_data()
    );

    let different = literal(list_literal(numbers(&[0.0, 2.0]))?);
    let domains = datafusion::arrow::compute::concat(&[
        domain.to_array()?.as_ref(),
        different.to_array()?.as_ref(),
    ])?;
    let args = vec![
        ColumnarValue::Array(domains),
        ColumnarValue::Scalar(range.clone()),
        ColumnarValue::Scalar(options.clone()),
        ColumnarValue::Scalar(0.5_f64.into()),
    ];
    assert!(invoke(args, 2)
        .unwrap_err()
        .to_string()
        .contains("constant within a record batch"));

    let args = vec![
        ColumnarValue::Array(domain.to_array_of_size(0)?),
        ColumnarValue::Scalar(range),
        ColumnarValue::Scalar(options),
        ColumnarValue::Array(Arc::new(Float64Array::from(Vec::<f64>::new()))),
    ];
    let ColumnarValue::Array(result) = invoke(args, 0)? else {
        panic!("array")
    };
    assert_eq!(result.len(), 0);
    assert_eq!(result.data_type(), &DataType::Float32);
    assert!(invoke(vec![], 1).is_err());
    Ok(())
}

#[tokio::test]
async fn invalid_configuration_returns_errors() -> Result<()> {
    for (domain, range, options) in [
        (numbers(&[0.0]), numbers(&[0.0, 1.0]), HashMap::new()),
        (numbers(&[0.0, 1.0]), numbers(&[]), HashMap::new()),
        (
            numbers(&[0.0, 1.0]),
            numbers(&[0.0, 1.0]),
            HashMap::from([("unknown".into(), Scalar::from_bool(true))]),
        ),
    ] {
        let expr = scale_expr(
            BuiltinScale::Linear,
            list_literal(domain)?,
            list_literal(range)?,
            options_literal(&options)?,
            col("value"),
        )?;
        assert!(evaluate(expr, numbers(&[0.0])).await.is_err());
    }
    let expr = scale_expr(
        BuiltinScale::Ordinal,
        list_literal(Arc::new(datafusion::arrow::array::Int64Array::from(vec![
            1_i64,
        ])))?,
        list_literal(strings(&["a"]))?,
        options_literal(&HashMap::new())?,
        col("value"),
    )?;
    assert!(evaluate(expr, numbers(&[1.0])).await.is_err());
    Ok(())
}
