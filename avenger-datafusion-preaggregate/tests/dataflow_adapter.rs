#![cfg(feature = "dataflow")]
#[path = "../../avenger-datafusion-aggregate-state/tests/common/mod.rs"]
mod common;

use avenger_datafusion_dataflow::{
    CachePolicy, DataflowBuilder, Runtime, RuntimeConfig, TableSnapshot,
};
use avenger_datafusion_preaggregate::{
    dataflow::Query, DirectReason, FilterQuery, PreaggregatePlanner, QueryPolicy, QueryStrategy,
};
use datafusion::{
    arrow::{
        array::{Float64Array, Int32Array, StringArray},
        datatypes::DataType,
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    functions_aggregate::expr_fn::count,
    logical_expr::{col, create_udf, lit, Expr, LogicalPlan, LogicalPlanBuilder as LP, Volatility},
    prelude::SessionContext,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn batch() -> RecordBatch {
    RecordBatch::try_from_iter(vec![
        (
            "g",
            Arc::new(StringArray::from(vec!["a", "b", "a", "b", "a", "b"])) as _,
        ),
        (
            "cell",
            Arc::new(Int32Array::from(vec![0, 0, 1, 1, 2, 2])) as _,
        ),
        (
            "x",
            Arc::new(Float64Array::from(vec![
                Some(1.),
                None,
                Some(3.),
                Some(4.),
                None,
                Some(6.),
            ])) as _,
        ),
    ])
    .unwrap()
}
fn snapshot(batch: RecordBatch) -> TableSnapshot {
    TableSnapshot::from_batches(batch.schema(), vec![batch]).unwrap()
}
fn aggregate(rows: LogicalPlan) -> datafusion::common::Result<LogicalPlan> {
    LP::from(rows)
        .aggregate(vec![col("g")], vec![count(col("x")).alias("n")])?
        .sort(vec![col("g").sort(true, true)])?
        .build()
}
fn install(b: &mut DataflowBuilder, source: LogicalPlan, name: &str) -> Result<Query> {
    let prepared = PreaggregatePlanner::default()
        .prepare(FilterQuery::new(source, aggregate)?, vec![col("cell")])?;
    Ok(Query::install(b, name, prepared)?)
}
async fn reference(batch: RecordBatch, predicate: Expr) -> Result<Vec<RecordBatch>> {
    let ctx = SessionContext::new();
    let rows = ctx
        .read_batch(batch)?
        .filter(predicate)?
        .into_unoptimized_plan();
    Ok(ctx
        .execute_logical_plan(aggregate(rows)?)
        .await?
        .collect()
        .await?)
}
fn equal(a: &[RecordBatch], b: &[RecordBatch]) -> Result {
    assert_eq!(common::rows(a)?, common::rows(b)?);
    Ok(())
}

#[tokio::test]
async fn checked_bindings_select_outputs_without_executing_and_preserve_filters() -> Result {
    let calls = Arc::new(AtomicUsize::new(0));
    let invocations = calls.clone();
    let f = create_udf(
        "observed",
        vec![DataType::Float64],
        DataType::Float64,
        Volatility::Immutable,
        Arc::new(move |args| {
            invocations.fetch_add(1, Ordering::Relaxed);
            Ok(args[0].clone())
        }),
    );
    let mut b = DataflowBuilder::new();
    let table = b.table_snapshot("rows", snapshot(batch()))?;
    let prepared = PreaggregatePlanner::default().prepare(
        FilterQuery::new(table.plan_ref(), |rows| {
            LP::from(rows)
                .filter(col("g").eq(lit("a")))?
                .aggregate(
                    vec![col("g")],
                    vec![count(f.call(vec![col("x")])).alias("n")],
                )?
                .sort(vec![col("g").sort(true, true)])?
                .build()
        })?,
        vec![col("cell")],
    )?;
    let q = Query::install(&mut b, "counts", prepared)?;
    let brush = col("cell").lt(lit(2));
    let bound = q.clone().bind(brush.clone())?;
    assert_eq!(bound.diagnostics().strategy, QueryStrategy::Preaggregated);
    assert_eq!(bound.materialization_output(), q.materialization_output());
    assert!(q.bind(col("missing").eq(lit(1))).is_err());
    assert!(q.bind(lit(1)).is_err());
    let forced = q.bind_with_policy(brush.clone(), QueryPolicy::ForceDirect)?;
    assert_eq!(
        forced.diagnostics().direct_reason,
        Some(DirectReason::Forced)
    );
    assert!(forced.materialization_output().is_none());
    let unretained = q.bind(col("x").gt(lit(2.)))?;
    assert_eq!(
        unretained.diagnostics().direct_reason,
        Some(DirectReason::PredicateNeedsUnretainedExpression)
    );
    let flow = Runtime::new(Default::default())?
        .prepare(&b.finish()?)
        .await?;
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    assert!(flow.inputs().finish().is_err());
    let foreign = Runtime::new(Default::default())?
        .prepare(&DataflowBuilder::new().finish()?)
        .await?;
    assert!(bound.apply(foreign.inputs()).is_err());
    let inputs = bound.apply(flow.inputs())?.finish()?;
    let result = flow.query(&[bound.output()], &[], &inputs).await?;
    equal(
        result.table(&bound.output())?.batches(),
        &reference(batch(), brush.clone().and(col("g").eq(lit("a")))).await?,
    )?;
    assert!(calls.load(Ordering::Relaxed) > 0);
    for (binding, predicate) in [(forced, brush), (unretained, col("x").gt(lit(2.)))] {
        let updated = binding.clone().apply(inputs.edit())?.finish()?;
        let result = flow.query(&[binding.output()], &[], &updated).await?;
        equal(
            result.table(&binding.output())?.batches(),
            &reference(batch(), predicate.and(col("g").eq(lit("a")))).await?,
        )?;
        assert!(!result
            .report()
            .executed_nodes
            .iter()
            .any(|n| n.ends_with("_states")));
        // Inspect the otherwise-unused rollup to verify the old retained predicate was cleared.
        let neutral = q.bind(lit(true))?;
        let result = flow.query(&[neutral.output()], &[], &updated).await?;
        equal(
            result.table(&neutral.output())?.batches(),
            &reference(batch(), col("g").eq(lit("a"))).await?,
        )?;
    }
    Ok(())
}

#[tokio::test]
async fn direct_only_queries_and_empty_aggregates() -> Result {
    let ctx = SessionContext::new();
    let source = ctx.read_batch(batch())?.into_unoptimized_plan();
    for grouped in [true, false] {
        let query = FilterQuery::new(source.clone(), |rows| {
            LP::from(rows)
                .aggregate(
                    if grouped { vec![col("g")] } else { vec![] },
                    vec![count(col("x")).alias("n")],
                )?
                .build()
        })?;
        let mut b = DataflowBuilder::new();
        let q = Query::install(
            &mut b,
            "empty",
            PreaggregatePlanner::default().prepare(query, vec![col("cell")])?,
        )?;
        let flow = Runtime::new(Default::default())?
            .prepare(&b.finish()?)
            .await?;
        let mut results = vec![];
        for policy in [QueryPolicy::Auto, QueryPolicy::ForceDirect] {
            let binding = q.bind_with_policy(lit(false), policy)?;
            let inputs = binding.apply(flow.inputs())?.finish()?;
            let result = flow.query(&[binding.output()], &[], &inputs).await?;
            let table = result.table(&binding.output())?;
            results.push((table.schema().clone(), common::rows(table.batches())?));
        }
        assert_eq!(results[0], results[1]);
        assert_eq!(results[0].1.len(), usize::from(!grouped));
        if !grouped {
            assert_eq!(results[0].1[0][0], ScalarValue::Int64(Some(0)));
        }
    }
    let query = FilterQuery::new(source, |rows| {
        LP::from(rows).project(vec![col("x")])?.build()
    })?;
    let mut b = DataflowBuilder::new();
    let q = Query::install(
        &mut b,
        "raw",
        PreaggregatePlanner::default().prepare(query, vec![col("cell")])?,
    )?;
    assert!(q.materialization_output().is_none());
    let binding = q.bind(lit(true))?;
    assert_eq!(
        binding.diagnostics().direct_reason,
        Some(DirectReason::UnsupportedQueryShape)
    );
    let graph = b.finish()?;
    assert_eq!(graph.num_inputs(), 1);
    let flow = Runtime::new(Default::default())?.prepare(&graph).await?;
    let inputs = binding.apply(flow.inputs())?.finish()?;
    let result = flow.query(&[binding.output()], &[], &inputs).await?;
    equal(
        result.table(&binding.output())?.batches(),
        &[batch().project(&[2])?],
    )?;
    Ok(())
}

#[tokio::test]
async fn warm_states_follow_source_and_fixed_inputs_but_not_brush_or_finishing_inputs() -> Result {
    for cache in [CachePolicy::default(), CachePolicy::Disabled] {
        let cached = !matches!(cache, CachePolicy::Disabled);
        let mut b = DataflowBuilder::new();
        let input = b.table_input("table", batch().schema())?;
        let fixed = b.expr_input("fixed", DataType::Boolean)?;
        let limit = b.scalar_input("limit", DataType::Int64)?;
        let source = LP::from(input.plan_ref())
            .filter(fixed.expr_ref())?
            .build()?;
        let query = FilterQuery::new(source, |rows| {
            LP::from(aggregate(rows)?)
                .limit_by_expr(None, Some(limit.expr_ref()))?
                .build()
        })?;
        let q = Query::install(
            &mut b,
            "counts",
            PreaggregatePlanner::default().prepare(query, vec![col("cell")])?,
        )?;
        let flow = Runtime::new(RuntimeConfig {
            cache,
            ..Default::default()
        })?
        .prepare(&b.finish()?)
        .await?;
        let idle = q.bind(lit(true))?;
        let initial = idle
            .apply(
                flow.inputs()
                    .table(&input, snapshot(batch()))?
                    .expr(&fixed, lit(true))?
                    .scalar(&limit, 2_i64.into())?,
            )?
            .finish()?;
        let warm = flow
            .query(&[idle.materialization_output().unwrap()], &[], &initial)
            .await?;
        assert!(warm
            .report()
            .executed_nodes
            .iter()
            .any(|n| n == "counts_states"));
        let mut observed = vec![];
        for high in [1, 2] {
            let predicate = col("cell").lt(lit(high));
            let bound = q.bind(predicate.clone())?;
            let inputs = bound.apply(initial.edit())?.finish()?;
            let result = flow.query(&[bound.output()], &[], &inputs).await?;
            assert_eq!(
                result
                    .report()
                    .executed_nodes
                    .iter()
                    .any(|n| n == "counts_states"),
                !cached
            );
            observed.push((predicate, result.table(&bound.output())?.clone()));
        }
        let bound = q.bind(col("cell").lt(lit(2)))?;
        let inputs = bound
            .apply(initial.edit().scalar(&limit, 1_i64.into())?)?
            .finish()?;
        let result = flow.query(&[bound.output()], &[], &inputs).await?;
        assert_eq!(
            result
                .report()
                .executed_nodes
                .iter()
                .any(|n| n == "counts_states"),
            !cached
        );
        assert_eq!(
            common::rows(result.table(&bound.output())?.batches())?.len(),
            1
        );
        let inputs = bound
            .apply(initial.edit().expr(&fixed, col("g").eq(lit("a")))?)?
            .finish()?;
        let result = flow.query(&[bound.output()], &[], &inputs).await?;
        assert!(result
            .report()
            .executed_nodes
            .iter()
            .any(|n| n == "counts_states"));
        equal(
            result.table(&bound.output())?.batches(),
            &reference(batch(), col("cell").lt(lit(2)).and(col("g").eq(lit("a")))).await?,
        )?;
        let inputs = bound
            .apply(
                initial
                    .edit()
                    .table(&input, snapshot(batch().slice(0, 2)))?,
            )?
            .finish()?;
        let result = flow.query(&[bound.output()], &[], &inputs).await?;
        assert!(result
            .report()
            .executed_nodes
            .iter()
            .any(|n| n == "counts_states"));
        equal(
            result.table(&bound.output())?.batches(),
            &reference(batch().slice(0, 2), col("cell").lt(lit(2))).await?,
        )?;
        for (p, result) in observed {
            equal(result.batches(), &reference(batch(), p).await?)?;
        }
    }
    Ok(())
}

#[tokio::test]
async fn additional_queries_preserve_base_identity_and_separate_bindings() -> Result {
    let mut b = DataflowBuilder::new();
    let input = b.table_input("table", batch().schema())?;
    let node = b.add_plan("source", input.plan_ref())?;
    let output = b.table_output("source", &node)?;
    let base = Runtime::new(Default::default())?
        .prepare(&b.finish()?)
        .await?;
    let base_inputs = base.inputs().table(&input, snapshot(batch()))?.finish()?;
    base.query(&[output], &[], &base_inputs).await?;
    for name in ["first", "replacement"] {
        let mut b = DataflowBuilder::with_base(&base.interface());
        let source = b.import_table("source", &output)?;
        let q = install(&mut b, source.plan_ref(), name)?;
        let extension = base.prepare_extension(&b.finish()?).await?;
        let bound = q.bind(col("cell").eq(lit(1)))?;
        assert!(bound.apply(base.inputs()).is_err());
        let inputs = bound.apply(extension.inputs())?.finish()?;
        let result = extension
            .query(&[bound.output()], &[], &base_inputs, &inputs)
            .await?;
        assert!(!result
            .report()
            .executed_nodes
            .iter()
            .any(|n| n == "base::source"));
        assert!(result
            .report()
            .executed_nodes
            .iter()
            .any(|n| n == &format!("additional::{name}_states")));
        equal(
            result.table(&bound.output())?.batches(),
            &reference(batch(), col("cell").eq(lit(1))).await?,
        )?;
    }
    Ok(())
}

#[tokio::test]
async fn scoped_bindings_preserve_other_instances_and_capture_dependencies() -> Result {
    let mut b = DataflowBuilder::new();
    let rows = b.table_snapshot("rows", snapshot(batch()))?;
    let fixed = b.expr_input("fixed", DataType::Boolean)?;
    let (scope, q) = b.partition_by("groups", rows.plan_ref(), vec![col("g")], |s| {
        let source = LP::from(s.rows().plan_ref())
            .filter(fixed.expr_ref())?
            .build()?;
        let prepared = PreaggregatePlanner::default()
            .prepare(FilterQuery::new(source, aggregate)?, vec![col("cell")])?;
        Query::install_scoped(s, "counts", prepared)
    })?;
    let (other_scope, ()) = b.partition_by("other", rows.plan_ref(), vec![col("g")], |_| Ok(()))?;
    let flow = Runtime::new(Default::default())?
        .prepare(&b.finish()?)
        .await?;
    let idle = q.bind(lit(true))?;
    assert!(idle.apply(flow.inputs()).is_err());
    assert!(flow
        .inputs()
        .scope_defaults(&other_scope, |b| idle.apply_scoped(b))
        .is_err());
    let initial = flow
        .inputs()
        .expr(&fixed, lit(true))?
        .scope_defaults(&scope, |b| idle.apply_scoped(b))?
        .finish()?;
    let all = flow.query(&[idle.output()], &[], &initial).await?;
    let key_b = scope.key(["b".into()])?;
    let before = all
        .scope(&scope)?
        .get(&key_b)
        .unwrap()
        .table(&idle.output())?
        .clone();
    let bound = q.bind(col("cell").lt(lit(1)))?;
    let instance = scope.instance(["a".into()])?;
    let updated = initial
        .edit()
        .at(&instance, |b| bound.apply_scoped(b))?
        .finish()?;
    let result = flow.query(&[bound.output()], &[], &updated).await?;
    assert_eq!(result.report().executed_nodes.len(), 1);
    assert!(result.report().executed_nodes[0].ends_with("counts_rollup"));
    let panels = result.scope(&scope)?;
    equal(
        before.batches(),
        panels
            .get(&key_b)
            .unwrap()
            .table(&bound.output())?
            .batches(),
    )?;
    let key_a = scope.key(["a".into()])?;
    equal(
        panels
            .get(&key_a)
            .unwrap()
            .table(&bound.output())?
            .batches(),
        &reference(batch(), col("g").eq(lit("a")).and(col("cell").lt(lit(1)))).await?,
    )?;
    let inputs = updated
        .edit()
        .expr(&fixed, col("x").gt(lit(2.)))?
        .finish()?;
    let result = flow.query(&[bound.output()], &[], &inputs).await?;
    assert_eq!(
        result
            .report()
            .executed_nodes
            .iter()
            .filter(|n| n.ends_with("counts_states"))
            .count(),
        2
    );
    Ok(())
}
