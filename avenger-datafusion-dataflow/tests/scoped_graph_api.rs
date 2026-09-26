mod common;
#[path = "common/scoped.rs"]
mod scoped;
use avenger_datafusion_dataflow::{
    arrow::datatypes::{DataType, Field, Schema},
    datafusion::{
        common::ScalarValue,
        logical_expr::{col, lit, scalar_subquery, LogicalPlanBuilder},
    },
    DataflowBuilder, Error, Result, Runtime, RuntimeConfig,
};
use std::{cell::Cell, sync::Arc};

#[test]
fn callbacks_run_once_and_visibility_is_lexical() -> Result<()> {
    let mut graph = DataflowBuilder::new();
    let input = graph.table_input("data", common::schema())?;
    let global = graph.add_scalar("value", lit(5_i64))?;
    let calls = Cell::new(0);
    let (outer, (inner, parameter, expression, rows)) =
        graph.partition_by("outer", input.plan_ref(), vec![col("value")], |scope| {
            calls.set(calls.get() + 1);
            let parameter = scope.scalar_input("value", DataType::Int64)?;
            let expression = scope.add_scalar("value", global.expr_ref() + parameter.expr_ref())?;
            let rows = scope.rows();
            assert_eq!(rows.plan_ref(), scope.rows().plan_ref());
            scope.table_output("rows", &rows)?;
            let (inner, _) =
                scope.partition_by("inner", rows.plan_ref(), vec![col("value")], |scope| {
                    calls.set(calls.get() + 1);
                    scope.add_scalar("value", expression.expr_ref())?;
                    Ok(())
                })?;
            Ok((inner, parameter, expression, rows))
        })?;
    assert_eq!(calls.get(), 2);
    assert!(matches!(
        graph.add_scalar("bad", parameter.expr_ref()),
        Err(Error::OutOfScope(_))
    ));
    assert!(matches!(
        graph.add_plan("bad", rows.plan_ref()),
        Err(Error::OutOfScope(_))
    ));
    assert!(matches!(
        graph.add_scalar("bad", scalar_subquery(Arc::new(rows.plan_ref()))),
        Err(Error::OutOfScope(_))
    ));
    assert!(matches!(
        graph.scalar_output("bad", &expression),
        Err(Error::OutOfScope(_))
    ));
    graph.partition_by("sibling", input.plan_ref(), vec![col("value")], |scope| {
        assert!(matches!(
            scope.add_scalar("bad", expression.expr_ref()),
            Err(Error::OutOfScope(_))
        ));
        assert!(matches!(
            scope.partition_by("bad", rows.plan_ref(), vec![col("value")], |_| Ok(())),
            Err(Error::OutOfScope(_))
        ));
        Ok(())
    })?;
    assert!(matches!(
        inner.instance([ScalarValue::from(1_i64)]),
        Err(Error::InvalidScopeAddress(_))
    ));
    let parent = outer.instance([ScalarValue::from(1_i64)])?;
    assert!(parent.child(&inner, [ScalarValue::from(2_i64)]).is_ok());
    assert!(matches!(
        parent.child(&outer, [ScalarValue::from(2_i64)]),
        Err(Error::InvalidScopeAddress(_))
    ));
    assert!(matches!(
        outer.key([ScalarValue::from(1_i32)]),
        Err(Error::InvalidKey(_))
    ));
    assert!(matches!(outer.key([]), Err(Error::InvalidKey(_))));
    assert!(matches!(
        graph.partition_by("outer", input.plan_ref(), vec![col("value")], |_| Ok(())),
        Err(Error::DuplicateName { .. })
    ));
    graph.finish()?;
    Ok(())
}

#[tokio::test]
async fn unsupported_key_types_fail_preparation_without_execution() -> Result<()> {
    for data_type in [
        DataType::Float64,
        DataType::List(Arc::new(Field::new("item", DataType::Int64, true))),
    ] {
        let mut graph = DataflowBuilder::new();
        let input = graph.table_input(
            "data",
            Arc::new(Schema::new(vec![Field::new("key", data_type, true)])),
        )?;
        graph.partition_by("panels", input.plan_ref(), vec![col("key")], |scope| {
            scope.table_output("rows", &scope.rows())
        })?;
        assert!(matches!(
            Runtime::new(RuntimeConfig::default())?
                .prepare(&graph.finish()?)
                .await,
            Err(Error::UnsupportedKeyType(_))
        ));
    }
    Ok(())
}

#[test]
fn local_keys_are_portable_but_addresses_validate_graph_and_parent() -> Result<()> {
    let mut a = DataflowBuilder::new();
    let a_fixture = scoped::build(&mut a)?;
    let mut b = DataflowBuilder::new();
    let b_fixture = scoped::build(&mut b)?;
    let key = a_fixture.regions.key([ScalarValue::from("East")])?;
    assert_eq!(key, b_fixture.regions.key([ScalarValue::from("East")])?);
    let east = a_fixture.regions.instance([ScalarValue::from("East")])?;
    let west = a_fixture.regions.instance([ScalarValue::from("West")])?;
    assert_ne!(
        east.child(&a_fixture.region.years, [ScalarValue::from(2025_i32)])?,
        west.child(&a_fixture.region.years, [ScalarValue::from(2025_i32)])?
    );
    assert!(matches!(
        east.child(&b_fixture.region.years, [ScalarValue::from(2025_i32)]),
        Err(Error::ForeignHandle)
    ));
    Ok(())
}

#[test]
fn failed_callback_cannot_publish_a_partial_graph() -> Result<()> {
    let mut graph = DataflowBuilder::new();
    let source = LogicalPlanBuilder::empty(true)
        .project(vec![lit(1_i64).alias("key")])?
        .build()?;
    let result = graph.partition_by("bad", source, vec![col("key")], |scope| -> Result<()> {
        scope.scalar_input("partial", DataType::Int64)?;
        Err(Error::InvalidExpression("test callback failed".into()))
    });
    assert!(result.is_err());
    assert!(graph.finish().is_err());
    Ok(())
}
