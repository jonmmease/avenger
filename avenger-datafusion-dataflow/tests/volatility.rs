mod common;
use std::sync::{
    atomic::{AtomicI64, Ordering},
    Arc,
};

use avenger_datafusion_dataflow::{
    arrow::{array::Int64Array, datatypes::DataType},
    datafusion::{
        common::ScalarValue,
        functions::datetime::expr_fn::now,
        logical_expr::{
            col, create_udf, lit, scalar_subquery, ColumnarValue, LogicalPlanBuilder, Volatility,
        },
    },
    GraphBuilder, ReuseScope, Runtime, RuntimeConfig,
};

#[tokio::test]
async fn named_volatile_scalars_are_shared_within_a_query_and_fresh_across_queries() {
    let calls = Arc::new(AtomicI64::new(0));
    let counter = calls.clone();
    let udf = create_udf(
        "draw",
        vec![],
        DataType::Int64,
        Volatility::Volatile,
        Arc::new(move |_| {
            Ok(ColumnarValue::Scalar(ScalarValue::Int64(Some(
                counter.fetch_add(1, Ordering::SeqCst) + 1,
            ))))
        }),
    );
    let mut graph = GraphBuilder::new();
    let draw = graph.add_expr("draw", udf.call(vec![])).unwrap();
    let next = graph
        .add_expr("next", draw.expr_ref() + lit(1_i64))
        .unwrap();
    let table = graph
        .add_plan(
            "table",
            LogicalPlanBuilder::empty(true)
                .project(vec![
                    draw.expr_ref().alias("a"),
                    draw.expr_ref().alias("b"),
                    next.expr_ref().alias("c"),
                ])
                .unwrap()
                .build()
                .unwrap(),
        )
        .unwrap();
    let table_out = graph.table_output("table", &table).unwrap();
    let draw_out = graph.scalar_output("draw", &draw).unwrap();
    let next_out = graph.scalar_output("next", &next).unwrap();
    let prepared = Runtime::new(RuntimeConfig::default())
        .unwrap()
        .prepare(&graph.finish().unwrap())
        .await
        .unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "construction and preparation must not invoke a volatile function"
    );
    assert!(prepared
        .explain()
        .nodes
        .iter()
        .all(|node| node.reuse_scope == ReuseScope::EvaluationLocal));
    let inputs = prepared.inputs().finish().unwrap();
    for expected in [1, 2] {
        let result = prepared
            .query(&[table_out], &[draw_out, next_out], &inputs)
            .await
            .unwrap();
        assert_eq!(
            result.scalar(&draw_out).unwrap(),
            &ScalarValue::Int64(Some(expected))
        );
        assert_eq!(
            result.scalar(&next_out).unwrap(),
            &ScalarValue::Int64(Some(expected + 1))
        );
        let batch = &result.table(&table_out).unwrap().batches()[0];
        assert_eq!(
            ScalarValue::try_from_array(batch.column(0), 0).unwrap(),
            ScalarValue::Int64(Some(expected))
        );
        assert_eq!(batch.column(0), batch.column(1));
        assert_eq!(calls.load(Ordering::SeqCst), expected);
        assert_eq!(result.report().physical_plans, 3);
    }
    let outputs = [draw_out];
    let (left, right) = tokio::join!(
        prepared.query(&[], &outputs, &inputs),
        prepared.query(&[], &outputs, &inputs)
    );
    let left = left.unwrap();
    let right = right.unwrap();
    assert_ne!(left.report().evaluation_id, right.report().evaluation_id);
    assert_ne!(
        left.scalar(&draw_out).unwrap(),
        right.scalar(&draw_out).unwrap()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn separately_named_volatile_expressions_are_not_merged() {
    let calls = Arc::new(AtomicI64::new(0));
    let counter = calls.clone();
    let udf = create_udf(
        "draw",
        vec![],
        DataType::Int64,
        Volatility::Volatile,
        Arc::new(move |_| {
            Ok(ColumnarValue::Scalar(ScalarValue::Int64(Some(
                counter.fetch_add(1, Ordering::SeqCst),
            ))))
        }),
    );
    let mut graph = GraphBuilder::new();
    let a = graph.add_expr("a", udf.call(vec![])).unwrap();
    let b = graph.add_expr("b", udf.call(vec![])).unwrap();
    let a = graph.scalar_output("a", &a).unwrap();
    let b = graph.scalar_output("b", &b).unwrap();
    let prepared = Runtime::new(RuntimeConfig::default())
        .unwrap()
        .prepare(&graph.finish().unwrap())
        .await
        .unwrap();
    let result = prepared
        .query(&[], &[a, b], &prepared.inputs().finish().unwrap())
        .await
        .unwrap();
    assert_eq!(result.scalar(&a).unwrap(), &ScalarValue::Int64(Some(0)));
    assert_eq!(result.scalar(&b).unwrap(), &ScalarValue::Int64(Some(1)));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn rowwise_volatile_values_are_generated_once_and_replayed_to_consumers() {
    let rows = Arc::new(AtomicI64::new(0));
    let generated = rows.clone();
    let udf = create_udf(
        "row_number_draw",
        vec![DataType::Int64],
        DataType::Int64,
        Volatility::Volatile,
        Arc::new(move |args| {
            let ColumnarValue::Array(values) = &args[0] else {
                panic!("column argument expected");
            };
            let first = generated.fetch_add(values.len() as i64, Ordering::SeqCst);
            Ok(ColumnarValue::Array(Arc::new(
                Int64Array::from_iter_values(first..first + values.len() as i64),
            )))
        }),
    );
    let mut graph = GraphBuilder::new();
    let input = graph.table_input("input", common::schema()).unwrap();
    let producer = graph
        .add_plan(
            "producer",
            LogicalPlanBuilder::from(input.plan_ref())
                .project(vec![udf.call(vec![col("value")]).alias("value")])
                .unwrap()
                .build()
                .unwrap(),
        )
        .unwrap();
    let a = graph.add_plan("a", producer.plan_ref()).unwrap();
    let b = graph.add_plan("b", producer.plan_ref()).unwrap();
    let a = graph.table_output("a", &a).unwrap();
    let b = graph.table_output("b", &b).unwrap();
    let prepared = Runtime::new(RuntimeConfig::default())
        .unwrap()
        .prepare(&graph.finish().unwrap())
        .await
        .unwrap();
    let inputs = prepared
        .inputs()
        .table(&input, common::snapshot(&[10, 20, 30]))
        .unwrap()
        .finish()
        .unwrap();
    let result = prepared.query(&[a, b], &[], &inputs).await.unwrap();
    assert_eq!(common::values(result.table(&a).unwrap()), vec![0, 1, 2]);
    assert_eq!(common::values(result.table(&b).unwrap()), vec![0, 1, 2]);
    assert_eq!(rows.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn volatility_in_nested_subqueries_propagates_to_graph_descendants() {
    let calls = Arc::new(AtomicI64::new(0));
    let counter = calls.clone();
    let udf = create_udf(
        "draw",
        vec![],
        DataType::Int64,
        Volatility::Volatile,
        Arc::new(move |_| {
            Ok(ColumnarValue::Scalar(ScalarValue::Int64(Some(
                counter.fetch_add(1, Ordering::SeqCst),
            ))))
        }),
    );
    let nested = LogicalPlanBuilder::empty(true)
        .project(vec![udf.call(vec![])])
        .unwrap()
        .build()
        .unwrap();
    let mut graph = GraphBuilder::new();
    let node = graph
        .add_expr("nested", scalar_subquery(Arc::new(nested)))
        .unwrap();
    let descendant = graph
        .add_expr("descendant", node.expr_ref() + lit(1_i64))
        .unwrap();
    let output = graph.scalar_output("out", &descendant).unwrap();
    let prepared = Runtime::new(RuntimeConfig::default())
        .unwrap()
        .prepare(&graph.finish().unwrap())
        .await
        .unwrap();
    assert!(prepared
        .explain()
        .nodes
        .iter()
        .all(|node| node.reuse_scope == ReuseScope::EvaluationLocal));
    let inputs = prepared.inputs().finish().unwrap();
    assert_eq!(
        prepared
            .query(&[], &[output], &inputs)
            .await
            .unwrap()
            .scalar(&output)
            .unwrap(),
        &ScalarValue::Int64(Some(1))
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn stable_now_uses_one_query_time_across_independently_planned_nodes() {
    let mut graph = GraphBuilder::new();
    let a = graph.add_expr("a", now()).unwrap();
    let b = graph.add_expr("b", now()).unwrap();
    let a = graph.scalar_output("a", &a).unwrap();
    let b = graph.scalar_output("b", &b).unwrap();
    let prepared = Runtime::new(RuntimeConfig::default())
        .unwrap()
        .prepare(&graph.finish().unwrap())
        .await
        .unwrap();
    assert!(prepared
        .explain()
        .nodes
        .iter()
        .all(|node| node.reuse_scope == ReuseScope::EvaluationLocal));
    let inputs = prepared.inputs().finish().unwrap();
    for _ in 0..2 {
        let result = prepared.query(&[], &[a, b], &inputs).await.unwrap();
        let expected = ScalarValue::TimestampNanosecond(
            result.report().query_start_time.timestamp_nanos_opt(),
            None,
        );
        assert_eq!(result.scalar(&a).unwrap(), &expected);
        assert_eq!(result.scalar(&b).unwrap(), &expected);
    }
}
