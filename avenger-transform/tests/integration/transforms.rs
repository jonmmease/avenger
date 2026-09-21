use super::common;
use avenger_transform::{self as t, expr_fn as tf, BinOptions};
use common::*;
use datafusion::{
    arrow::{
        array::{Array, ArrayRef, AsArray, BooleanArray, Float64Array, Int32Array, StringArray},
        datatypes::{DataType, Float64Type, Int32Type},
        record_batch::RecordBatch,
    },
    common::{Column, Result, ScalarValue},
    functions_nested::expr_fn::make_array,
    logical_expr::{col, lit, Expr, LogicalPlanBuilder},
    prelude::SessionContext,
};
use std::sync::Arc;

#[tokio::test]
async fn formula_replaces_in_order_and_preserves_literal_names() -> Result<()> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_from_iter([
        ("x", Arc::new(Int32Array::from(vec![1, 2])) as ArrayRef),
        ("a.b", Arc::new(Int32Array::from(vec![10, 20])) as ArrayRef),
    ])?;
    let plan = LogicalPlanBuilder::from(ctx.read_batch(batch)?.into_unoptimized_plan())
        .alias("rows")?
        .build()?;
    let plan = t::formula(plan, col("rows.x") + lit(2), "x")?;
    let plan = t::formula(plan, col("x") * lit(3), "z")?;
    let plan = t::formula(
        plan,
        Expr::Column(Column::from_name("a.b")) + col("z"),
        "a.b",
    )?;
    let b = collect(&ctx, plan).await?;
    assert_eq!(
        b.schema()
            .fields()
            .iter()
            .map(|f| f.name().as_str())
            .collect::<Vec<_>>(),
        ["x", "a.b", "z"]
    );
    assert_eq!(b.column(1).as_primitive::<Int32Type>().values(), &[19, 32]);
    Ok(())
}

#[tokio::test]
async fn filter_truthiness() -> Result<()> {
    let ctx = SessionContext::new();
    for (values, expected) in [
        (
            Arc::new(Float64Array::from(vec![
                None,
                Some(f64::NAN),
                Some(0.0),
                Some(-0.0),
                Some(1.0),
                Some(f64::INFINITY),
            ])) as ArrayRef,
            2,
        ),
        (
            Arc::new(StringArray::from(vec![
                None,
                Some(""),
                Some("false"),
                Some("0"),
            ])) as ArrayRef,
            2,
        ),
        (
            Arc::new(BooleanArray::from(vec![None, Some(false), Some(true)])) as ArrayRef,
            1,
        ),
    ] {
        let plan = ctx
            .read_batch(RecordBatch::try_from_iter([("x", values)])?)?
            .into_unoptimized_plan();
        assert_eq!(
            collect(&ctx, t::filter(plan, col("x"))?).await?.num_rows(),
            expected
        );
    }
    Ok(())
}

#[tokio::test]
async fn extent_empty_invalid_and_nonfinite() -> Result<()> {
    let ctx = SessionContext::new();
    for (values, expected) in [
        (vec![], vec![None, None]),
        (vec![None, Some(f64::NAN)], vec![None, None]),
        (
            vec![Some(-1.0), None, Some(9.0), Some(f64::NAN)],
            vec![Some(-1.0), Some(9.0)],
        ),
        (vec![Some(1.0), Some(f64::INFINITY)], vec![None, None]),
        (vec![Some(f64::NEG_INFINITY), Some(1.0)], vec![None, None]),
        (vec![Some(f64::INFINITY)], vec![None, None]),
        (vec![Some(f64::NEG_INFINITY)], vec![None, None]),
    ] {
        let b = collect(&ctx, t::extent(source(&ctx, values), col("x"))?).await?;
        assert_eq!(b.num_rows(), 1);
        assert_eq!(
            struct_values(&ScalarValue::try_from_array(b.column(0), 0)?),
            expected
        );
    }
    Ok(())
}

#[tokio::test]
async fn vega_bin_fixtures() -> Result<()> {
    let ctx = SessionContext::new();
    let fixtures: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/vega-6.2.0.json")).unwrap();
    for case in fixtures["bins"].as_array().unwrap() {
        let o = &case["options"];
        let scalar = |key: &str| {
            o.get(key).map(|v| {
                if let Some(b) = v.as_bool() {
                    lit(b)
                } else {
                    lit(v.as_f64().unwrap())
                }
            })
        };
        let list = |key: &str| {
            o.get(key).map(|v| {
                make_array(
                    v.as_array()
                        .unwrap()
                        .iter()
                        .map(|v| lit(v.as_f64().unwrap()))
                        .collect(),
                )
            })
        };
        let p = t::bin_parameters(
            extent(number(&case["extent"][0]), number(&case["extent"][1])),
            BinOptions {
                maxbins: scalar("maxbins"),
                base: scalar("base"),
                divide: list("divide"),
                span: scalar("span"),
                step: scalar("step"),
                steps: list("steps"),
                minstep: scalar("minstep"),
                nice: scalar("nice"),
                anchor: scalar("anchor"),
            },
        )?;
        let resolved = evaluate(&ctx, p.clone()).await?;
        eprintln!("bin fixture {}", case["name"]);
        for (a, e) in struct_values(&resolved)
            .into_iter()
            .zip(case["parameters"].as_array().unwrap())
        {
            assert_number(a, number(e));
        }
        let input = source(
            &ctx,
            case["values"]
                .as_array()
                .unwrap()
                .iter()
                .map(number)
                .collect(),
        );
        let result = collect(&ctx, t::bin(input, col("x"), p, ["x", "hi"])?).await?;
        assert_eq!(result.num_columns(), 2);
        for (row, expected) in case["bounds"].as_array().unwrap().iter().enumerate() {
            for column in 0..2 {
                let a = result.column(column).as_primitive::<Float64Type>();
                assert_number(
                    (!a.is_null(row)).then(|| a.value(row)),
                    number(&expected[column]),
                );
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn bins_validate_options_and_empty_extents() -> Result<()> {
    let ctx = SessionContext::new();
    for options in [
        BinOptions {
            step: Some(lit(0.0)),
            ..Default::default()
        },
        BinOptions {
            base: Some(lit(1.0)),
            ..Default::default()
        },
        BinOptions {
            maxbins: Some(lit(0.5)),
            ..Default::default()
        },
        BinOptions {
            steps: Some(make_array(vec![lit(2.0), lit(1.0)])),
            ..Default::default()
        },
        BinOptions {
            nice: Some(lit(ScalarValue::Boolean(None))),
            ..Default::default()
        },
        BinOptions {
            step: Some(lit(f64::INFINITY)),
            ..Default::default()
        },
    ] {
        assert!(evaluate(
            &ctx,
            t::bin_parameters(extent(Some(0.0), Some(29.0)), options)?
        )
        .await
        .is_err());
    }
    for (min, max, options) in [
        (10.0, 0.0, BinOptions::default()),
        (-f64::MAX, f64::MAX, BinOptions::default()),
        (
            f64::MAX,
            f64::MAX,
            BinOptions {
                step: Some(lit(1.0)),
                nice: Some(lit(false)),
                ..Default::default()
            },
        ),
        (
            0.0,
            1.0,
            BinOptions {
                step: Some(lit(f64::from_bits(1))),
                ..Default::default()
            },
        ),
    ] {
        assert!(evaluate(
            &ctx,
            t::bin_parameters(extent(Some(min), Some(max)), options)?
        )
        .await
        .is_err());
    }
    // A base close to one is valid if its finite arithmetic converges.
    evaluate(
        &ctx,
        t::bin_parameters(
            extent(Some(0.0), Some(29.0)),
            BinOptions {
                base: Some(lit(1.0000000001)),
                ..Default::default()
            },
        )?,
    )
    .await?;
    let params = t::bin_parameters(extent(None, None), BinOptions::default())?;
    assert!(evaluate(&ctx, params.clone()).await?.is_null());
    let b = collect(
        &ctx,
        t::bin(
            source(&ctx, vec![Some(2.0)]),
            col("x"),
            params,
            ["lo", "hi"],
        )?,
    )
    .await?;
    assert!(b.column(1).is_null(0));
    let p = t::bin_parameters(extent(Some(0.0), Some(10.0)), BinOptions::default())?;
    assert_eq!(
        collect(
            &ctx,
            t::bin(source(&ctx, vec![]), col("x"), p, ["lo", "hi"])?
        )
        .await?
        .num_rows(),
        0
    );
    Ok(())
}

#[tokio::test]
async fn vega_aggregate_fixtures() -> Result<()> {
    let ctx = SessionContext::new();
    let fixtures: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/vega-6.2.0.json")).unwrap();
    for case in fixtures["aggregates"].as_array().unwrap() {
        let b = collect(
            &ctx,
            t::aggregate(
                source(
                    &ctx,
                    case["values"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(number)
                        .collect(),
                ),
                vec![],
                measures(),
            )?,
        )
        .await?;
        assert_eq!(b.num_rows(), case["rows"].as_array().unwrap().len());
        if b.num_rows() == 0 {
            continue;
        }
        for (field, a) in b.schema().fields().iter().zip(b.columns()) {
            let a = datafusion::arrow::compute::cast(a, &DataType::Float64)?;
            let a = a.as_primitive::<Float64Type>();
            assert_number(
                (!a.is_null(0)).then(|| a.value(0)),
                number(&case["rows"][0][field.name()]),
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn aggregate_grouping_aliases_and_strings() -> Result<()> {
    let ctx = SessionContext::new();
    let b = RecordBatch::try_from_iter([
        ("g", Arc::new(Int32Array::from(vec![1, 1, 2])) as ArrayRef),
        (
            "s",
            Arc::new(StringArray::from(vec![Some(""), None, Some("z")])),
        ),
    ])?;
    let rows = LogicalPlanBuilder::from(ctx.read_batch(b)?.into_unoptimized_plan())
        .alias("rows")?
        .build()?;
    let plan = t::aggregate(
        rows.clone(),
        vec![col("g")],
        vec![
            tf::min(col("s")).alias("min"),
            tf::valid(col("s")).alias("valid"),
            tf::missing(col("s")).alias("missing"),
            tf::count().alias("n"),
            tf::count().alias("again"),
        ],
    )?;
    assert_eq!(plan.schema().field(0).name(), "g");
    let b = collect(
        &ctx,
        LogicalPlanBuilder::from(plan)
            .sort(vec![col("g").sort(true, true)])?
            .build()?,
    )
    .await?;
    assert!(b.column(1).is_null(0));
    assert_eq!(
        ScalarValue::try_from_array(b.column(3), 0)?,
        ScalarValue::Int64(Some(2))
    );
    let plan = t::aggregate(
        rows.clone(),
        vec![(col("g") + lit(1)).alias("group")],
        vec![(tf::count() + lit(1_i64)).alias("plus")],
    )?;
    assert_eq!(collect(&ctx, plan).await?.num_rows(), 2);
    assert!(t::aggregate(rows.clone(), vec![], vec![tf::count()]).is_err());
    assert!(t::aggregate(rows, vec![col("g")], vec![tf::count().alias("g")]).is_err());
    Ok(())
}

#[tokio::test]
async fn uniform_array_configuration_and_vector_boundaries() -> Result<()> {
    use datafusion::{
        arrow::datatypes::Field,
        logical_expr::{ColumnarValue, ScalarFunctionArgs},
    };
    let ctx = SessionContext::new();
    let expression = t::bin_parameters(
        extent(Some(0.0), Some(29.0)),
        BinOptions {
            step: Some(lit(5.0)),
            nice: Some(lit(false)),
            ..Default::default()
        },
    )?;
    let Expr::ScalarFunction(f) = expression.clone() else {
        unreachable!()
    };
    let extent = evaluate(&ctx, f.args[0].clone()).await?;
    let options = evaluate(&ctx, f.args[1].clone()).await?;
    let invoke =
        |function: &datafusion::logical_expr::ScalarUDF, args: Vec<ColumnarValue>, rows| {
            let types = args
                .iter()
                .map(ColumnarValue::data_type)
                .collect::<Vec<_>>();
            let arg_fields = types
                .iter()
                .map(|t| Arc::new(Field::new("", t.clone(), true)))
                .collect();
            function.invoke_with_args(ScalarFunctionArgs {
                args,
                arg_fields,
                number_rows: rows,
                return_field: Arc::new(Field::new("", function.return_type(&types)?, true)),
                config_options: Arc::default(),
            })
        };
    let p = invoke(
        &f.func,
        vec![
            ColumnarValue::Array(extent.to_array_of_size(4)?),
            ColumnarValue::Array(options.to_array_of_size(4)?),
        ],
        4,
    )?;
    let empty = invoke(
        &f.func,
        vec![
            ColumnarValue::Array(extent.to_array_of_size(0)?),
            ColumnarValue::Scalar(options.clone()),
        ],
        0,
    )?;
    assert_eq!(empty.into_array(0)?.len(), 0);
    let other = evaluate(&ctx, common::extent(Some(0.0), Some(30.0))).await?;
    let mixed = datafusion::arrow::compute::concat(&[
        extent.to_array()?.as_ref(),
        other.to_array()?.as_ref(),
    ])?;
    assert!(invoke(
        &f.func,
        vec![ColumnarValue::Array(mixed), ColumnarValue::Scalar(options)],
        2
    )
    .is_err());
    // Extract the bounds UDF from the get_field wrapper; exercise its array path directly.
    let Expr::ScalarFunction(get) = tf::bin_start(col("x"), expression) else {
        unreachable!()
    };
    let Expr::ScalarFunction(bounds) = &get.args[0] else {
        unreachable!()
    };
    let result = invoke(
        &bounds.func,
        vec![
            ColumnarValue::Array(Arc::new(Int32Array::from(vec![
                Some(5),
                Some(29),
                Some(31),
                None,
            ]))),
            p,
        ],
        4,
    )?;
    let result = result.into_array(4)?;
    let result = result.as_struct();
    let start = result.column(0).as_primitive::<Float64Type>();
    assert_eq!(
        start.iter().collect::<Vec<_>>(),
        vec![Some(5.0), Some(25.0), Some(f64::INFINITY), None]
    );
    let scalar = evaluate(
        &ctx,
        tf::bin_start(
            lit(29),
            lit(evaluate(
                &ctx,
                t::bin_parameters(
                    common::extent(Some(0.0), Some(29.0)),
                    BinOptions {
                        step: Some(lit(5.0)),
                        nice: Some(lit(false)),
                        ..Default::default()
                    },
                )?,
            )
            .await?),
        ),
    )
    .await?;
    assert_eq!(scalar, ScalarValue::Float64(Some(25.0)));
    Ok(())
}

#[tokio::test]
async fn multiple_batches_types_and_schema_errors() -> Result<()> {
    use datafusion::datasource::MemTable;
    let ctx = SessionContext::new();
    let a =
        RecordBatch::try_from_iter([("x", Arc::new(Int32Array::from(vec![1, 2])) as ArrayRef)])?;
    let b =
        RecordBatch::try_from_iter([("x", Arc::new(Int32Array::from(vec![5, 10])) as ArrayRef)])?;
    let rows = ctx
        .read_table(Arc::new(MemTable::try_new(
            a.schema(),
            vec![vec![a], vec![b]],
        )?))?
        .into_unoptimized_plan();
    let extent = t::extent(rows.clone(), col("x"))?;
    let parameters = t::bin_parameters(
        datafusion::logical_expr::scalar_subquery(Arc::new(extent)),
        BinOptions::default(),
    )?;
    let bins = t::bin(rows.clone(), col("x"), parameters.clone(), ["lo", "hi"])?;
    let result = collect(
        &ctx,
        t::aggregate(
            bins,
            vec![],
            vec![
                tf::count().alias("n"),
                tf::min(col("x")).alias("min"),
                tf::sum(col("x")).alias("sum"),
            ],
        )?,
    )
    .await?;
    assert_eq!(
        ScalarValue::try_from_array(result.column(0), 0)?,
        ScalarValue::Int64(Some(4))
    );
    assert_eq!(result.column(1).data_type(), &DataType::Int32);
    assert_eq!(
        ScalarValue::try_from_array(result.column(2), 0)?,
        ScalarValue::Float64(Some(18.0))
    );
    assert!(t::bin(rows.clone(), col("x"), parameters, ["lo", "lo"]).is_err());
    assert!(t::extent(rows.clone(), lit("12")).is_err());
    let left = LogicalPlanBuilder::from(rows.clone()).alias("a")?.build()?;
    let right = LogicalPlanBuilder::from(rows).alias("b")?.build()?;
    let joined = LogicalPlanBuilder::from(left).cross_join(right)?.build()?;
    assert!(t::formula(joined, lit(1), "x").is_err());
    Ok(())
}

#[tokio::test]
async fn bin_temporary_name_avoids_both_input_and_output_fields() -> Result<()> {
    let ctx = SessionContext::new();
    let rows = source(&ctx, vec![Some(2.0)]);
    let rows = t::formula(rows, lit(17), "__avenger_bin_")?;
    let parameters = t::bin_parameters(
        extent(Some(0.0), Some(10.0)),
        BinOptions {
            step: Some(lit(5.0)),
            ..Default::default()
        },
    )?;
    let result = collect(
        &ctx,
        t::bin(rows, col("x"), parameters, ["__avenger_bin", "end"])?,
    )
    .await?;
    assert_eq!(result.num_columns(), 4);
    assert_eq!(
        ScalarValue::try_from_array(result.column(1), 0)?,
        ScalarValue::Int32(Some(17))
    );
    assert_eq!(
        ScalarValue::try_from_array(result.column(2), 0)?,
        ScalarValue::Float64(Some(0.0))
    );
    Ok(())
}

#[tokio::test]
async fn zero_stack_preserves_rows_and_separates_signs() -> Result<()> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_from_iter([
        (
            "v",
            Arc::new(Float64Array::from(vec![
                Some(4.),
                Some(-3.),
                None,
                Some(6.),
                Some(-2.),
                Some(f64::NAN),
            ])) as ArrayRef,
        ),
        (
            "order",
            Arc::new(Int32Array::from(vec![0, 1, 2, 3, 4, 5])) as ArrayRef,
        ),
    ])?;
    let input = ctx.read_batch(batch)?.into_unoptimized_plan();
    let plan = t::stack_zero(
        input.clone(),
        vec![],
        col("v"),
        vec![col("order").sort(true, true)],
        ["start", "end"],
    )?;
    let plan = LogicalPlanBuilder::from(plan)
        .sort(vec![col("order").sort(true, true)])?
        .build()?;
    let b = collect(&ctx, plan).await?;
    assert_eq!(
        b.column_by_name("start")
            .unwrap()
            .as_primitive::<Float64Type>()
            .values()
            .as_ref(),
        &[0., 0., 4., 4., -3., 10.]
    );
    assert_eq!(
        b.column_by_name("end")
            .unwrap()
            .as_primitive::<Float64Type>()
            .values()
            .as_ref(),
        &[4., -3., 4., 10., -5., 10.]
    );
    assert!(t::stack_zero(input.clone(), vec![], col("v"), vec![], ["start", "end"]).is_err());
    assert!(t::stack_zero(
        input,
        vec![],
        col("v"),
        vec![col("order").sort(true, true)],
        ["v", "end"]
    )
    .is_err());
    Ok(())
}
