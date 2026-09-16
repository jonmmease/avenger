mod common;
use std::sync::Arc;

use avenger_datafusion_dataflow::{
    arrow::datatypes::DataType,
    datafusion::{
        common::ScalarValue,
        dataframe::DataFrame,
        execution::context::SessionContext,
        functions_aggregate::expr_fn::sum,
        logical_expr::{col, lit, scalar_subquery, Expr, LogicalPlanBuilder},
    },
    Error, ExecutionConfig, GraphBuilder, Runtime, RuntimeConfig, TableSnapshot,
};

#[tokio::test]
async fn mixed_outputs_match_datafusion_and_rebinding_recomputes() {
    let mut graph = GraphBuilder::new();
    let source = graph.table_input("source", common::schema()).unwrap();
    let cutoff = graph.scalar_input("cutoff", DataType::Int64).unwrap();
    let filtered = graph
        .add_plan(
            "filtered",
            LogicalPlanBuilder::from(source.plan_ref())
                .filter(col("value").gt(cutoff.expr_ref()))
                .unwrap()
                .sort(vec![col("value").sort(true, false)])
                .unwrap()
                .build()
                .unwrap(),
        )
        .unwrap();
    let total_plan = graph
        .add_plan(
            "total_plan",
            LogicalPlanBuilder::from(filtered.plan_ref())
                .aggregate(Vec::<Expr>::new(), vec![sum(col("value")).alias("total")])
                .unwrap()
                .build()
                .unwrap(),
        )
        .unwrap();
    let total = graph
        .add_expr("total", scalar_subquery(Arc::new(total_plan.plan_ref())))
        .unwrap();
    let rows = graph.table_output("rows", &filtered).unwrap();
    let scalar = graph.scalar_output("sum", &total).unwrap();
    let prepared = Runtime::new(RuntimeConfig::default())
        .unwrap()
        .prepare(&graph.finish().unwrap())
        .await
        .unwrap();
    let snapshot = common::snapshot(&[4, 1, 3, 2]);
    let inputs = prepared
        .inputs()
        .table(&source, snapshot.clone())
        .unwrap()
        .scalar(&cutoff, ScalarValue::Int64(Some(1)))
        .unwrap()
        .finish()
        .unwrap();
    let first = prepared
        .query(&[rows, rows], &[scalar, scalar], &inputs)
        .await
        .unwrap();
    assert_eq!(common::values(first.table(&rows).unwrap()), vec![2, 3, 4]);
    assert_eq!(first.scalar(&scalar).unwrap(), &ScalarValue::Int64(Some(9)));
    assert_eq!(
        first.report().executed_nodes,
        vec!["filtered", "total_plan", "total"]
    );
    assert_eq!(first.report().physical_plans, 3);

    let context = SessionContext::new();
    let oracle = context
        .read_batch(snapshot.batches()[0].clone())
        .unwrap()
        .filter(col("value").gt(lit(1_i64)))
        .unwrap()
        .sort(vec![col("value").sort(true, false)])
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(first.table(&rows).unwrap().batches(), oracle.as_slice());
    let oracle_plan = LogicalPlanBuilder::from(
        context
            .read_batch(snapshot.batches()[0].clone())
            .unwrap()
            .into_unoptimized_plan(),
    )
    .filter(col("value").gt(lit(1_i64)))
    .unwrap()
    .aggregate(Vec::<Expr>::new(), vec![sum(col("value")).alias("total")])
    .unwrap()
    .build()
    .unwrap();
    let oracle_total = DataFrame::new(context.state(), oracle_plan)
        .collect()
        .await
        .unwrap();
    assert_eq!(
        ScalarValue::try_from_array(oracle_total[0].column(0), 0).unwrap(),
        *first.scalar(&scalar).unwrap()
    );

    let changed = inputs
        .edit()
        .scalar(&cutoff, ScalarValue::Int64(Some(3)))
        .unwrap()
        .finish()
        .unwrap();
    let next = prepared.query(&[rows], &[scalar], &changed).await.unwrap();
    assert_eq!(common::values(next.table(&rows).unwrap()), vec![4]);
    assert_eq!(next.scalar(&scalar).unwrap(), &ScalarValue::Int64(Some(4)));
    let repeated = prepared.query(&[rows], &[scalar], &inputs).await.unwrap();
    assert_eq!(repeated.report().physical_plans, 3);
    assert_eq!(
        repeated.scalar(&scalar).unwrap(),
        &ScalarValue::Int64(Some(9))
    );
    assert_ne!(
        first.report().evaluation_id,
        repeated.report().evaluation_id
    );

    let tables_only = prepared.query(&[rows], &[], &inputs).await.unwrap();
    assert_eq!(tables_only.report().executed_nodes, vec!["filtered"]);
    assert!(matches!(
        tables_only.scalar(&scalar),
        Err(Error::UnrequestedOutput)
    ));
    let scalars_only = prepared.query(&[], &[scalar], &inputs).await.unwrap();
    assert!(matches!(
        scalars_only.table(&rows),
        Err(Error::UnrequestedOutput)
    ));
    let empty = prepared.query(&[], &[], &inputs).await.unwrap();
    assert!(empty.report().executed_nodes.is_empty());

    let no_rows = inputs
        .edit()
        .table(&source, TableSnapshot::empty(common::schema()))
        .unwrap()
        .finish()
        .unwrap();
    let empty = prepared.query(&[rows], &[scalar], &no_rows).await.unwrap();
    assert_eq!(empty.table(&rows).unwrap().schema(), &common::schema());
    assert_eq!(empty.table(&rows).unwrap().num_rows(), 0);
    assert_eq!(empty.scalar(&scalar).unwrap(), &ScalarValue::Int64(None));
}

#[tokio::test]
async fn concurrent_queries_keep_distinct_scalar_bindings() {
    let mut graph = GraphBuilder::new();
    let parameter = graph.scalar_input("parameter", DataType::Int64).unwrap();
    let node = graph
        .add_expr("twice", parameter.expr_ref() * lit(2_i64))
        .unwrap();
    let output = graph.scalar_output("twice", &node).unwrap();
    let prepared = Runtime::new(RuntimeConfig::default())
        .unwrap()
        .prepare(&graph.finish().unwrap())
        .await
        .unwrap();
    let one = prepared
        .inputs()
        .scalar(&parameter, ScalarValue::Int64(Some(1)))
        .unwrap()
        .finish()
        .unwrap();
    let two = one
        .edit()
        .scalar(&parameter, ScalarValue::Int64(Some(2)))
        .unwrap()
        .finish()
        .unwrap();
    let outputs = [output];
    let (a, b) = tokio::join!(
        prepared.query(&[], &outputs, &one),
        prepared.query(&[], &outputs, &two)
    );
    assert_eq!(
        a.unwrap().scalar(&output).unwrap(),
        &ScalarValue::Int64(Some(2))
    );
    assert_eq!(
        b.unwrap().scalar(&output).unwrap(),
        &ScalarValue::Int64(Some(4))
    );
}

#[tokio::test]
async fn resource_failure_is_an_error_and_does_not_poison_the_runtime() {
    let runtime = Runtime::new(RuntimeConfig {
        execution: ExecutionConfig {
            max_active_queries: 1,
            max_materialized_bytes: 1,
        },
    })
    .unwrap();
    let mut graph = GraphBuilder::new();
    let node = graph.add_expr("value", lit(1_i64)).unwrap();
    let output = graph.scalar_output("value", &node).unwrap();
    let prepared = runtime.prepare(&graph.finish().unwrap()).await.unwrap();
    let inputs = prepared.inputs().finish().unwrap();
    assert!(matches!(
        prepared.query(&[], &[output], &inputs).await,
        Err(Error::ResourceExhausted { .. })
    ));
    assert!(prepared.query(&[], &[], &inputs).await.is_ok());
}

#[tokio::test]
async fn rejects_foreign_outputs_and_inputs() {
    let runtime = Runtime::new(RuntimeConfig::default()).unwrap();
    let mut one = GraphBuilder::new();
    let node = one.add_expr("one", lit(1)).unwrap();
    let output = one.scalar_output("one", &node).unwrap();
    let one = runtime.prepare(&one.finish().unwrap()).await.unwrap();
    let mut two = GraphBuilder::new();
    let node = two.add_expr("two", lit(2)).unwrap();
    let foreign = two.scalar_output("two", &node).unwrap();
    let two = runtime.prepare(&two.finish().unwrap()).await.unwrap();
    let inputs = one.inputs().finish().unwrap();
    assert!(matches!(
        one.query(&[], &[foreign], &inputs).await,
        Err(Error::ForeignHandle)
    ));
    assert!(matches!(
        one.query(&[], &[output], &two.inputs().finish().unwrap())
            .await,
        Err(Error::ForeignHandle)
    ));
    assert!(matches!(
        one.query(&[], &[output], &inputs)
            .await
            .unwrap()
            .scalar(&foreign),
        Err(Error::ForeignHandle)
    ));
}
