mod common;
use avenger_datafusion_dataflow::{
    arrow::datatypes::DataType,
    datafusion::logical_expr::{col, lit, placeholder, scalar_subquery, LogicalPlanBuilder},
    Error, GraphBuilder, Runtime, RuntimeConfig,
};
use std::sync::Arc;

#[test]
fn registers_all_plan_expression_edges_and_separate_namespaces() {
    let mut graph = GraphBuilder::new();
    let table = graph.table_input("table", common::schema()).unwrap();
    let scalar = graph.scalar_input("scalar", DataType::Int64).unwrap();
    let first = graph.add_plan("first", table.plan_ref()).unwrap();
    let second = graph
        .add_plan(
            "second",
            LogicalPlanBuilder::from(first.plan_ref())
                .limit(0, Some(1))
                .unwrap()
                .build()
                .unwrap(),
        )
        .unwrap();
    let expr = graph
        .add_expr("expression", scalar_subquery(Arc::new(second.plan_ref())))
        .unwrap();
    let combined = graph
        .add_expr("combined", expr.expr_ref() + scalar.expr_ref())
        .unwrap();
    let final_plan = graph
        .add_plan(
            "final",
            LogicalPlanBuilder::from(first.plan_ref())
                .filter(col("value").gt(combined.expr_ref()))
                .unwrap()
                .build()
                .unwrap(),
        )
        .unwrap();
    graph.table_output("final", &final_plan).unwrap();
    graph.scalar_output("expression", &expr).unwrap();
    let graph = graph.finish().unwrap();
    assert_eq!(graph.num_nodes(), 5);
    assert_eq!(graph.num_inputs(), 2);
    assert_eq!(graph.num_outputs(), 2);
}

#[test]
fn rejects_foreign_references_duplicate_names_and_unknown_placeholders() {
    let mut one = GraphBuilder::new();
    let table = one.table_input("table", common::schema()).unwrap();
    assert!(matches!(
        one.scalar_input("table", DataType::Int64),
        Err(Error::DuplicateName {
            namespace: "input",
            ..
        })
    ));
    let node = one.add_plan("node", table.plan_ref()).unwrap();
    assert!(matches!(
        one.add_expr("node", lit(1)),
        Err(Error::DuplicateName {
            namespace: "computation",
            ..
        })
    ));
    one.table_output("out", &node).unwrap();
    assert!(matches!(
        one.table_output("out", &node),
        Err(Error::DuplicateName {
            namespace: "output",
            ..
        })
    ));
    let mut two = GraphBuilder::new();
    assert!(matches!(
        two.add_plan("foreign", node.plan_ref()),
        Err(Error::ForeignHandle)
    ));
    assert!(matches!(
        two.table_output("foreign", &node),
        Err(Error::ForeignHandle)
    ));
    assert!(two.add_expr("unknown", placeholder("$unknown")).is_err());
    // An unsuccessful registration does not reserve its name.
    two.add_expr("unknown", lit(42)).unwrap();
}

#[test]
fn rejects_free_columns_and_multicolumn_scalar_subqueries() {
    let mut graph = GraphBuilder::new();
    assert!(matches!(
        graph.add_expr("bad", col("value")),
        Err(Error::InvalidExpression(_))
    ));
    let plan = LogicalPlanBuilder::empty(true)
        .project(vec![lit(1).alias("a"), lit(2).alias("b")])
        .unwrap()
        .build()
        .unwrap();
    assert!(graph
        .add_expr("bad", scalar_subquery(Arc::new(plan)))
        .is_err());
}

#[tokio::test]
async fn prepare_reports_only_nodes_reachable_from_outputs() {
    let mut graph = GraphBuilder::new();
    let reachable = graph.add_expr("reachable", lit(1)).unwrap();
    graph.add_expr("unused", lit(2)).unwrap();
    graph.scalar_output("answer", &reachable).unwrap();
    let prepared = Runtime::new(RuntimeConfig::default())
        .unwrap()
        .prepare(&graph.finish().unwrap())
        .await
        .unwrap();
    assert!(prepared.explain().replans_on_query);
    assert_eq!(
        prepared
            .explain()
            .nodes
            .iter()
            .map(|node| node.name.as_str())
            .collect::<Vec<_>>(),
        vec!["reachable"]
    );
}
