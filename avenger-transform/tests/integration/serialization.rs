use super::common;
use avenger_datafusion_dataflow::{
    DataflowBuilder, Result, Runtime, RuntimeConfig, SemanticConfig, TableSnapshot,
};
use avenger_scales_datafusion::{options_literal, scale_expr, BuiltinScale, ScaleExtensionCodec};
use avenger_transform::{self as t, BinOptions, TransformExtensionCodec};
use datafusion::{
    arrow::{compute::concat_batches, datatypes::DataType},
    common::ScalarValue,
    functions_nested::expr_fn::make_array,
    logical_expr::{col, lit, scalar_subquery, Expr, LogicalPlanBuilder},
    prelude::SessionContext,
};
use datafusion_proto::logical_plan::{
    from_proto::parse_expr, to_proto::serialize_expr, LogicalExtensionCodec,
};
use std::{collections::HashMap, sync::Arc};

#[tokio::test]
async fn generated_preaggregations_round_trip_with_composed_codecs() -> Result<()> {
    use avenger_datafusion_aggregate_state::{self as states, AggregateStateExtensionCodec};
    use avenger_datafusion_preaggregate::{dataflow::Query, FilterQuery, PreaggregatePlanner};
    let mut versions = t::function_versions();
    versions.extend(states::function_versions());
    let batch = common::batch(vec![Some(1.0), None, Some(2.0), Some(4.0), Some(8.0)]);
    let snapshot = TableSnapshot::from_batches(batch.schema(), vec![batch])?;
    let mut builder = DataflowBuilder::with_semantics(SemanticConfig {
        function_versions: versions.clone(),
        ..Default::default()
    });
    let source = builder.table_input("source", snapshot.schema().clone())?;
    let query = PreaggregatePlanner::default().prepare(
        FilterQuery::new(source.plan_ref(), |rows| {
            t::aggregate(rows, vec![], common::measures())
        })?,
        vec![col("x")],
    )?;
    let predicates = [lit(true), col("x").gt(lit(1.0)), lit(false)]
        .into_iter()
        .map(|predicate| query.bind(predicate).map(|b| b.predicates().clone()))
        .collect::<datafusion::common::Result<Vec<_>>>()?;
    let query = Query::install(&mut builder, "summary", query)?;
    assert!(query.materialization_output().is_some());
    let graph = builder.finish()?;
    let codecs: Vec<Arc<dyn LogicalExtensionCodec>> = vec![
        Arc::new(TransformExtensionCodec::with_fallback(Arc::new(
            AggregateStateExtensionCodec::default(),
        ))),
        Arc::new(AggregateStateExtensionCodec::with_fallback(Arc::new(
            TransformExtensionCodec::default(),
        ))),
    ];
    for codec in codecs {
        let bytes = graph.to_bytes_with_codec(codec.clone())?;
        assert!(Runtime::new(RuntimeConfig::default())?
            .decode_dataflow(&bytes)
            .is_err());
        let runtime = Runtime::with_session_state_and_codec(
            SessionContext::new().state(),
            RuntimeConfig {
                function_versions: versions.clone(),
                ..Default::default()
            },
            codec,
        )?;
        let decoded = runtime.decode_dataflow(&bytes)?;
        let root = decoded.interface().root();
        let flow = runtime.prepare(&decoded).await?;
        let direct = root.table_output("summary_direct")?;
        let rollup = root.table_output("summary_rollup")?;
        let warm = root.table_output("summary_states")?;
        let mut inputs = flow
            .inputs()
            .table(&root.table_input("source")?, snapshot.clone())?
            .expr(&root.expr_input("summary_source")?, lit(true))?
            .expr(&root.expr_input("summary_retained")?, lit(true))?
            .finish()?;
        flow.query(&[warm], &[], &inputs).await?;
        for predicate in &predicates {
            inputs = inputs
                .edit()
                .expr(
                    &root.expr_input("summary_source")?,
                    predicate.source().clone(),
                )?
                .expr(
                    &root.expr_input("summary_retained")?,
                    predicate.retained().unwrap().clone(),
                )?
                .finish()?;
            let result = flow.query(&[rollup, direct], &[], &inputs).await?;
            assert!(!result
                .report()
                .executed_nodes
                .iter()
                .any(|n| n == "summary_states"));
            let a = concat_batches(
                result.table(&rollup)?.schema(),
                result.table(&rollup)?.batches(),
            )?;
            let e = concat_batches(
                result.table(&direct)?.schema(),
                result.table(&direct)?.batches(),
            )?;
            assert_eq!(a.schema(), e.schema());
            assert_eq!(a.num_rows(), e.num_rows());
            for (a, e) in a.columns().iter().zip(e.columns()) {
                if a.data_type() == &DataType::Float64 {
                    use datafusion::arrow::{array::AsArray, datatypes::Float64Type};
                    for (a, e) in a
                        .as_primitive::<Float64Type>()
                        .iter()
                        .zip(e.as_primitive::<Float64Type>())
                    {
                        common::assert_number(a, e);
                    }
                } else {
                    assert_eq!(a.to_data(), e.to_data());
                }
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn transform_graph_decodes_in_fresh_runtime() -> Result<()> {
    let mut b = DataflowBuilder::with_semantics(SemanticConfig {
        function_versions: t::function_versions(),
        ..Default::default()
    });
    let batch = common::batch(vec![
        Some(0.0),
        Some(1.0),
        Some(2.0),
        None,
        Some(f64::NAN),
        Some(f64::INFINITY),
    ]);
    let rows = b.table_snapshot(
        "rows",
        TableSnapshot::from_batches(batch.schema(), vec![batch])?,
    )?;
    let selected = b.expr_input("selected", DataType::Boolean)?;
    let step = b.scalar_input("step", DataType::Float64)?;
    let finite = t::filter(rows.plan_ref(), col("x").lt(lit(f64::INFINITY)))?;
    let extent = b.add_plan("extent", t::extent(finite, col("x"))?)?;
    let params = b.add_scalar(
        "parameters",
        t::bin_parameters(
            scalar_subquery(Arc::new(extent.plan_ref())),
            BinOptions {
                step: Some(step.expr_ref()),
                ..Default::default()
            },
        )?,
    )?;
    let bins = t::bin(rows.plan_ref(), col("x"), params.expr_ref(), ["lo", "hi"])?;
    let bins = t::formula(bins, t::expr_fn::truthy(col("x")), "truthy")?;
    let output = b.add_plan(
        "result",
        LogicalPlanBuilder::from(t::aggregate(
            t::filter(bins, selected.expr_ref())?,
            vec![col("lo")],
            common::measures(),
        )?)
        .sort(vec![col("lo").sort(true, true)])?
        .build()?,
    )?;
    let output = b.table_output("result", &output)?;
    let graph = b.finish()?;
    let codec = Arc::new(TransformExtensionCodec::default());
    let runtime = || {
        Runtime::with_session_state_and_codec(
            SessionContext::new().state(),
            RuntimeConfig {
                function_versions: t::function_versions(),
                ..Default::default()
            },
            codec.clone(),
        )
    };
    let prepared = runtime()?.prepare(&graph).await?;
    let inputs = prepared
        .inputs()
        .scalar(&step, 1.0.into())?
        .expr(&selected, col("x").lt(lit(10.0)))?
        .finish()?;
    let expected = prepared.query(&[output], &[], &inputs).await?;
    let bytes = graph.to_bytes_with_codec(codec.clone())?;
    let fresh = runtime()?;
    let decoded = fresh.decode_dataflow(&bytes)?;
    let interface = decoded.interface();
    let root = interface.root();
    let prepared = fresh.prepare(&decoded).await?;
    let output2 = root.table_output("result")?;
    let inputs = prepared
        .inputs()
        .scalar(&root.scalar_input("step")?, 1.0.into())?
        .expr(&root.expr_input("selected")?, col("x").lt(lit(10.0)))?
        .finish()?;
    let actual = prepared.query(&[output2], &[], &inputs).await?;
    let expected = expected.table(&output)?;
    let actual = actual.table(&output2)?;
    assert_eq!(
        concat_batches(expected.schema(), expected.batches())?,
        concat_batches(actual.schema(), actual.batches())?
    );
    Ok(())
}

#[tokio::test]
async fn delegates_scale_codec_and_preserves_nonfinite_literals() -> datafusion::common::Result<()>
{
    let ctx = SessionContext::new();
    let codec = TransformExtensionCodec::with_fallback(Arc::new(ScaleExtensionCodec::default()));
    let scale = scale_expr(
        BuiltinScale::Linear,
        make_array(vec![lit(0.0), lit(10.0)]),
        make_array(vec![lit(0.0), lit(100.0)]),
        options_literal(&HashMap::new())?,
        lit(5.0),
    )?;
    for expr in [
        t::expr_fn::truthy(scale),
        t::expr_fn::truthy(lit(f64::NAN)),
        t::expr_fn::bin_end(
            lit(f64::INFINITY),
            t::bin_parameters(common::extent(Some(0.0), Some(10.0)), BinOptions::default())?,
        ),
    ] {
        let decoded = parse_expr(
            &serialize_expr(&expr, &codec)?,
            ctx.task_ctx().as_ref(),
            &codec,
        )?;
        let a = common::evaluate(&ctx, expr).await?;
        let b = common::evaluate(&ctx, decoded).await?;
        assert_eq!(a, b);
    }
    Ok(())
}

#[test]
fn rejects_unknown_version_and_does_not_claim_foreign_names() -> datafusion::common::Result<()> {
    let codec = TransformExtensionCodec::default();
    let Expr::ScalarFunction(f) = t::expr_fn::truthy(lit(1)) else {
        unreachable!()
    };
    let mut bytes = vec![];
    codec.try_encode_udf(&f.func, &mut bytes)?;
    assert!(codec.try_decode_udf("other", &bytes).is_err());
    let i = bytes.iter().position(|b| *b == b'/').unwrap() + 1;
    bytes[i] = b'9';
    assert!(codec.try_decode_udf(f.func.name(), &bytes).is_err());
    let foreign = datafusion::logical_expr::create_udf(
        "avenger_truthy",
        vec![DataType::Boolean],
        DataType::Boolean,
        datafusion::logical_expr::Volatility::Volatile,
        Arc::new(|_| {
            Ok(datafusion::logical_expr::ColumnarValue::Scalar(
                ScalarValue::Boolean(Some(true)),
            ))
        }),
    );
    let mut bytes = vec![];
    codec.try_encode_udf(&foreign, &mut bytes)?;
    assert!(!bytes.starts_with(b"AVENGER_TRANSFORM"));
    Ok(())
}
