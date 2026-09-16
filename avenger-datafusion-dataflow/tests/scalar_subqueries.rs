mod common;
use avenger_datafusion_dataflow::{
    arrow::datatypes::{DataType, Field},
    datafusion::{
        common::ScalarValue,
        logical_expr::{col, lit, scalar_subquery, when, Expr, LogicalPlanBuilder},
    },
    DataflowBuilder, Error, Runtime, RuntimeConfig, TableSnapshot,
};
use std::sync::Arc;

#[tokio::test]
async fn scalar_subqueries_enforce_zero_one_and_multiple_row_semantics() {
    let mut graph = DataflowBuilder::new();
    let input = graph.table_input("values", common::schema()).unwrap();
    let node = graph.add_plan("values", input.plan_ref()).unwrap();
    let scalar = graph
        .add_expr("scalar", scalar_subquery(Arc::new(node.plan_ref())))
        .unwrap();
    let output = graph.scalar_output("scalar", &scalar).unwrap();
    let prepared = Runtime::new(RuntimeConfig::default())
        .unwrap()
        .prepare(&graph.finish().unwrap())
        .await
        .unwrap();
    for (table, expected) in [
        (
            TableSnapshot::empty(common::schema()),
            ScalarValue::Int64(None),
        ),
        (common::snapshot(&[42]), ScalarValue::Int64(Some(42))),
    ] {
        let inputs = prepared
            .inputs()
            .table(&input, table)
            .unwrap()
            .finish()
            .unwrap();
        assert_eq!(
            prepared
                .query(&[], &[output], &inputs)
                .await
                .unwrap()
                .scalar(&output)
                .unwrap(),
            &expected
        );
    }
    let inputs = prepared
        .inputs()
        .table(&input, common::snapshot(&[1, 2]))
        .unwrap()
        .finish()
        .unwrap();
    let error = prepared.query(&[], &[output], &inputs).await.unwrap_err();
    assert!(matches!(error, Error::Execution { .. }));
    assert!(error.to_string().to_lowercase().contains("row"), "{error}");
}

#[tokio::test]
async fn nested_subqueries_find_table_and_scalar_dependencies() {
    let mut graph = DataflowBuilder::new();
    let table = graph.table_input("table", common::schema()).unwrap();
    let offset = graph.scalar_input("offset", DataType::Int64).unwrap();
    let value = graph
        .add_expr("offset_plus_one", offset.expr_ref() + lit(1_i64))
        .unwrap();
    let inner = LogicalPlanBuilder::from(table.plan_ref())
        .project(vec![(col("value") + value.expr_ref()).alias("v")])
        .unwrap()
        .build()
        .unwrap();
    let middle = LogicalPlanBuilder::empty(true)
        .project(vec![scalar_subquery(Arc::new(inner)).alias("nested")])
        .unwrap()
        .build()
        .unwrap();
    let scalar = graph
        .add_expr("answer", scalar_subquery(Arc::new(middle)))
        .unwrap();
    let output = graph.scalar_output("answer", &scalar).unwrap();
    let prepared = Runtime::new(RuntimeConfig::default())
        .unwrap()
        .prepare(&graph.finish().unwrap())
        .await
        .unwrap();
    let inputs = prepared
        .inputs()
        .table(&table, common::snapshot(&[10]))
        .unwrap()
        .scalar(&offset, ScalarValue::Int64(Some(2)))
        .unwrap()
        .finish()
        .unwrap();
    let result = prepared.query(&[], &[output], &inputs).await.unwrap();
    assert_eq!(
        result.scalar(&output).unwrap(),
        &ScalarValue::Int64(Some(13))
    );
    assert_eq!(
        result.report().executed_nodes,
        vec!["offset_plus_one", "answer"]
    );
}

#[tokio::test]
async fn strict_named_dependency_fails_even_when_consumer_guard_is_false() {
    let mut graph = DataflowBuilder::new();
    let source = graph.table_input("source", common::schema()).unwrap();
    let bad = graph
        .add_expr("bad", scalar_subquery(Arc::new(source.plan_ref())))
        .unwrap();
    let guarded = graph
        .add_expr(
            "guarded",
            when(lit(false), bad.expr_ref())
                .otherwise(lit(0_i64))
                .unwrap(),
        )
        .unwrap();
    let output = graph.scalar_output("guarded", &guarded).unwrap();
    let prepared = Runtime::new(RuntimeConfig::default())
        .unwrap()
        .prepare(&graph.finish().unwrap())
        .await
        .unwrap();
    let inputs = prepared
        .inputs()
        .table(&source, common::snapshot(&[1, 2]))
        .unwrap()
        .finish()
        .unwrap();
    let error = prepared.query(&[], &[output], &inputs).await.unwrap_err();
    assert!(error.to_string().contains("'bad'"), "{error}");
}

#[test]
fn rejects_unresolved_correlation_at_a_graph_boundary() {
    let mut graph = DataflowBuilder::new();
    let expr = Expr::OuterReferenceColumn(
        Arc::new(Field::new("outside", DataType::Int64, false)),
        "outside".into(),
    );
    let plan = LogicalPlanBuilder::empty(true)
        .project(vec![expr])
        .unwrap()
        .build()
        .unwrap();
    assert!(matches!(
        graph.add_plan("invalid", plan),
        Err(Error::InvalidExpression(_))
    ));
}

#[tokio::test]
async fn correlated_subquery_inside_a_plan_keeps_its_row_scope() {
    use avenger_datafusion_dataflow::datafusion::common::Column;
    use avenger_datafusion_dataflow::datafusion::functions_aggregate::expr_fn::sum;
    let mut graph = DataflowBuilder::new();
    let outer = graph.table_input("outer_rows", common::schema()).unwrap();
    let inner = graph.table_input("inner_rows", common::schema()).unwrap();
    let outer_value = Expr::OuterReferenceColumn(
        Arc::new(Field::new("value", DataType::Int64, false)),
        Column::new(Some("outer_rows"), "value"),
    );
    let subquery = LogicalPlanBuilder::from(inner.plan_ref())
        .filter(col("inner_rows.value").eq(outer_value))
        .unwrap()
        .aggregate(
            Vec::<Expr>::new(),
            vec![sum(col("inner_rows.value")).alias("matched")],
        )
        .unwrap()
        .build()
        .unwrap();
    let node = graph
        .add_plan(
            "matched",
            LogicalPlanBuilder::from(outer.plan_ref())
                .project(vec![
                    col("outer_rows.value"),
                    scalar_subquery(Arc::new(subquery)).alias("matched"),
                ])
                .unwrap()
                .sort(vec![col("outer_rows.value").sort(true, false)])
                .unwrap()
                .build()
                .unwrap(),
        )
        .unwrap();
    let output = graph.table_output("matched", &node).unwrap();
    let prepared = Runtime::new(RuntimeConfig::default())
        .unwrap()
        .prepare(&graph.finish().unwrap())
        .await
        .unwrap();
    let inputs = prepared
        .inputs()
        .table(&outer, common::snapshot(&[1, 2]))
        .unwrap()
        .table(&inner, common::snapshot(&[2, 2]))
        .unwrap()
        .finish()
        .unwrap();
    let result = prepared.query(&[output], &[], &inputs).await.unwrap();
    let values: Vec<_> = result
        .table(&output)
        .unwrap()
        .batches()
        .iter()
        .flat_map(|batch| {
            (0..batch.num_rows())
                .map(|row| ScalarValue::try_from_array(batch.column(1), row).unwrap())
        })
        .collect();
    assert_eq!(
        values,
        vec![ScalarValue::Int64(None), ScalarValue::Int64(Some(4))]
    );
}

#[tokio::test]
async fn empty_scalar_subquery_in_a_table_plan_remains_nullable() {
    let mut graph = DataflowBuilder::new();
    let source = graph.table_input("source", common::schema()).unwrap();
    let plan = LogicalPlanBuilder::empty(true)
        .project(vec![
            scalar_subquery(Arc::new(source.plan_ref())).alias("value")
        ])
        .unwrap()
        .build()
        .unwrap();
    let node = graph.add_plan("projected", plan).unwrap();
    assert!(node.schema().field(0).is_nullable());
    let output = graph.table_output("projected", &node).unwrap();
    let prepared = Runtime::new(RuntimeConfig::default())
        .unwrap()
        .prepare(&graph.finish().unwrap())
        .await
        .unwrap();
    let inputs = prepared
        .inputs()
        .table(&source, TableSnapshot::empty(common::schema()))
        .unwrap()
        .finish()
        .unwrap();
    let result = prepared.query(&[output], &[], &inputs).await.unwrap();
    let batch = &result.table(&output).unwrap().batches()[0];
    assert_eq!(
        ScalarValue::try_from_array(batch.column(0), 0).unwrap(),
        ScalarValue::Int64(None)
    );
}
