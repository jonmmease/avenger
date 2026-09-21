use super::common;
use avenger_datafusion_preaggregate::{
    dataflow::Query, BoundQuery, FilterQuery, PreaggregatePlanner, QueryPolicy,
};
use avenger_transform as t;
use datafusion::{
    arrow::{
        array::{ArrayRef, AsArray, Float64Array, Int32Array},
        datatypes::{DataType, Float64Type},
        record_batch::RecordBatch,
    },
    common::Result,
    logical_expr::{col, lit, LogicalPlanBuilder},
    prelude::SessionContext,
};
use std::sync::Arc;

#[tokio::test]
async fn native_states_preserve_all_helper_results_and_fallback() -> Result<()> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_from_iter([
        (
            "x",
            Arc::new(Float64Array::from(vec![
                Some(1.0),
                Some(2.0),
                Some(3.0),
                None,
                Some(f64::NAN),
            ])) as ArrayRef,
        ),
        (
            "cell",
            Arc::new(Int32Array::from(vec![0, 1, 1, 2, 2])) as ArrayRef,
        ),
        (
            "other",
            Arc::new(Int32Array::from(vec![1, 2, 3, 4, 5])) as ArrayRef,
        ),
    ])?;
    let source = ctx.read_batch(batch)?.into_unoptimized_plan();
    for grouped in [false, true] {
        let query = FilterQuery::new(source.clone(), |rows| {
            t::aggregate(
                rows,
                if grouped {
                    vec![(col("cell") / lit(2)).alias("g")]
                } else {
                    vec![]
                },
                common::measures(),
            )
        })?;
        let prepared = PreaggregatePlanner::default().prepare(query, vec![col("cell")])?;
        let materialization = prepared
            .materialization_plan()
            .unwrap_or_else(|| panic!("{:?}", prepared.explain()));
        let stored = common::collect(&ctx, materialization.clone()).await?;
        let stored = ctx.read_batch(stored)?.into_unoptimized_plan();
        for predicate in [
            lit(true),
            col("cell").eq(lit(0)),
            col("cell").eq(lit(2)),
            col("cell").lt(lit(2)),
            lit(false),
        ] {
            let BoundQuery::Preaggregated { rollup, .. } = prepared.bind(predicate.clone())? else {
                panic!("expected preaggregation")
            };
            let BoundQuery::Direct { plan: direct, .. } =
                prepared.bind_with_policy(predicate, QueryPolicy::ForceDirect)?
            else {
                unreachable!()
            };
            let sort = |p| {
                if grouped {
                    LogicalPlanBuilder::from(p)
                        .sort(vec![col("g").sort(true, true)])
                        .unwrap()
                        .build()
                        .unwrap()
                } else {
                    p
                }
            };
            let actual =
                common::collect(&ctx, sort(rollup.with_materialization(stored.clone())?)).await?;
            let expected = common::collect(&ctx, sort(direct)).await?;
            assert_eq!(actual.schema(), expected.schema());
            assert_eq!(actual.num_rows(), expected.num_rows());
            for (a, e) in actual.columns().iter().zip(expected.columns()) {
                if a.data_type() == &DataType::Float64 {
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
        assert!(matches!(
            prepared.bind(col("other").gt(lit(2)))?,
            BoundQuery::Direct { .. }
        ));
    }
    Ok(())
}

#[tokio::test]
async fn warm_states_reuse_across_brushes_and_track_source_dependencies(
) -> avenger_datafusion_dataflow::Result<()> {
    use avenger_datafusion_dataflow::{
        CacheConfig, CachePolicy, DataflowBuilder, Runtime, RuntimeConfig, TableSnapshot,
    };
    let batch = RecordBatch::try_from_iter([
        (
            "x",
            Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0])) as ArrayRef,
        ),
        (
            "cell",
            Arc::new(Int32Array::from(vec![0, 0, 1, 1])) as ArrayRef,
        ),
    ])?;
    let mut b = DataflowBuilder::new();
    let source = b.table_input("source", batch.schema())?;
    let fixed = b.expr_input("fixed", DataType::Boolean)?;
    let step = b.scalar_input("step", DataType::Float64)?;
    let params = b.add_scalar(
        "parameters",
        t::bin_parameters(
            common::extent(Some(0.0), Some(4.0)),
            t::BinOptions {
                step: Some(step.expr_ref()),
                ..Default::default()
            },
        )?,
    )?;
    let bins = b.add_plan(
        "bins",
        t::bin(source.plan_ref(), col("x"), params.expr_ref(), ["lo", "hi"])?,
    )?;
    let query = FilterQuery::new(t::filter(bins.plan_ref(), fixed.expr_ref())?, |rows| {
        t::aggregate(rows, vec![col("lo")], vec![t::expr_fn::count().alias("n")])
    })?;
    let family = PreaggregatePlanner::default().prepare(query, vec![col("cell")])?;
    let query = Query::install(&mut b, "histogram", family)?;
    let warm = query.materialization_output().unwrap();
    let idle = query.bind(lit(true))?;
    let output = idle.output();
    let prepared = Runtime::new(RuntimeConfig {
        cache: CachePolicy::Lru(CacheConfig {
            max_bytes: 16 * 1024 * 1024,
            max_entries: 256,
        }),
        ..Default::default()
    })?
    .prepare(&b.finish()?)
    .await?;
    let inputs = idle
        .apply(
            prepared
                .inputs()
                .table(
                    &source,
                    TableSnapshot::from_batches(batch.schema(), vec![batch.clone()])?,
                )?
                .scalar(&step, 2.0.into())?
                .expr(&fixed, lit(true))?,
        )?
        .finish()?;
    let result = prepared.query(&[warm], &[], &inputs).await?;
    assert!(result
        .report()
        .executed_nodes
        .iter()
        .any(|n| n == "histogram_states"));
    for cell in [0, 1] {
        let binding = query.bind(col("cell").eq(lit(cell)))?;
        let inputs = binding.apply(inputs.edit())?.finish()?;
        let result = prepared.query(&[binding.output()], &[], &inputs).await?;
        assert_eq!(result.report().executed_nodes, vec!["histogram_rollup"]);
    }
    let changed_fixed = inputs
        .edit()
        .expr(&fixed, col("x").gt(lit(1.0)))?
        .finish()?;
    let result = prepared.query(&[output], &[], &changed_fixed).await?;
    assert!(result
        .report()
        .executed_nodes
        .iter()
        .any(|n| n == "histogram_states"));
    assert!(!result.report().executed_nodes.iter().any(|n| n == "bins"));
    let changed_bins = inputs.edit().scalar(&step, 1.0.into())?.finish()?;
    let result = prepared.query(&[output], &[], &changed_bins).await?;
    assert!(result
        .report()
        .executed_nodes
        .iter()
        .any(|n| n == "histogram_states"));
    assert!(result.report().executed_nodes.iter().any(|n| n == "bins"));
    let changed_source = inputs
        .edit()
        .table(
            &source,
            TableSnapshot::from_batches(batch.schema(), vec![batch])?,
        )?
        .finish()?;
    let result = prepared.query(&[output], &[], &changed_source).await?;
    assert!(result
        .report()
        .executed_nodes
        .iter()
        .any(|n| n == "histogram_states"));
    Ok(())
}
