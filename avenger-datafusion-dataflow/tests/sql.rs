#![cfg(feature = "sql")]

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use avenger_datafusion_dataflow::{
    arrow::{
        array::Int64Array,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    datafusion::{
        catalog::MemorySchemaProvider,
        common::{Column, TableReference},
        functions_aggregate::expr_fn::{count, max, sum},
        logical_expr::{
            col, create_udf, lit, scalar_subquery, Expr, JoinType, LogicalPlanBuilder, Volatility,
        },
        prelude::SessionContext,
    },
    DataflowBuilder, Error, Result, TableSnapshot,
};

fn schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]))
}
fn batch(name: &str, values: Vec<i64>) -> RecordBatch {
    RecordBatch::try_from_iter(vec![(name, Arc::new(Int64Array::from(values)) as _)]).unwrap()
}
fn context(namespace: &str) -> SessionContext {
    let ctx = SessionContext::new();
    ctx.catalog("datafusion")
        .unwrap()
        .register_schema(namespace, Arc::new(MemorySchemaProvider::new()))
        .unwrap();
    ctx
}

#[test]
fn stored_nodes_outputs_and_native_expressions_have_distinct_sql() -> Result<()> {
    let mut graph = DataflowBuilder::new();
    let sales = graph.table_input("sales", schema())?;
    let fraction = graph.scalar_input("fraction", DataType::Int64)?;
    let predicate = graph.expr_input("selected", DataType::Boolean)?;
    let totals = graph.add_plan(
        "totals",
        LogicalPlanBuilder::from(sales.plan_ref())
            .aggregate(vec![col("id")], vec![sum(col("id")).alias("total")])?
            .build()?,
    )?;
    let maximum_table = graph.add_plan(
        "maximum_table",
        LogicalPlanBuilder::from(totals.plan_ref())
            .aggregate(Vec::<Expr>::new(), vec![max(col("total")).alias("maximum")])?
            .build()?,
    )?;
    let maximum = graph.add_scalar(
        "maximum",
        scalar_subquery(Arc::new(maximum_table.plan_ref())),
    )?;
    let threshold = graph.add_scalar("threshold", maximum.expr_ref() * fraction.expr_ref())?;
    let visible = graph.add_plan(
        "visible",
        LogicalPlanBuilder::from(totals.plan_ref())
            .filter(
                col("total")
                    .gt(threshold.expr_ref())
                    .and(predicate.expr_ref()),
            )?
            .sort(vec![col("id").sort(true, false)])?
            .build()?,
    )?;
    let table_output = graph.table_output("rows", &visible)?;
    let scalar_output = graph.scalar_output("threshold", &threshold)?;
    assert_eq!(
        graph.sql().scalar(&threshold)?,
        "$scalar__maximum * $input__fraction"
    );
    let graph = graph.finish()?;
    let sql = graph.sql();
    let statement = sql.table(&visible)?;
    assert!(statement.contains("nodes.\"totals\""), "{statement}");
    assert!(
        statement.contains("$scalar__threshold") && statement.contains("$input__selected"),
        "{statement}"
    );
    assert!(
        !statement.contains("sum("),
        "upstream lineage must not be expanded: {statement}"
    );
    assert!(!statement.contains("__avenger_"), "{statement}");
    assert!(sql.table(&totals)?.contains("inputs.sales"));
    assert!(sql.scalar(&maximum)?.contains("nodes.maximum_table"));
    assert!(!sql.scalar(&maximum)?.ends_with(';'));
    assert_eq!(sql.table_output(&table_output)?, statement);
    assert_eq!(sql.scalar_output(&scalar_output)?, sql.scalar(&threshold)?);
    assert_eq!(sql.expr(&threshold.expr_ref())?, "$scalar__threshold");
    assert_eq!(
        sql.expr(&(col("id") + fraction.expr_ref()))?,
        "id + $input__fraction"
    );
    assert!(sql.plan(&totals.plan_ref())?.contains("nodes.\"totals\""));
    Ok(())
}

#[tokio::test]
async fn aggregate_queries_preserve_column_order_with_sort_and_limit() -> Result<()> {
    let mut graph = DataflowBuilder::new();
    let sales = graph.table_input("sales", schema())?;
    let grouped = LogicalPlanBuilder::from(sales.plan_ref())
        .aggregate(vec![col("id")], vec![sum(col("id")).alias("total")])?
        .build()?;
    let sorted = LogicalPlanBuilder::from(grouped.clone())
        .sort(vec![col("id").sort(false, false)])?
        .limit(0, Some(1))?
        .build()?;
    let graph = graph.finish()?;
    let ctx = context("inputs");
    ctx.register_batch(
        TableReference::partial("inputs", "sales"),
        batch("id", vec![1, 2, 2]),
    )?;
    for (plan, expected_rows) in [(grouped, 2), (sorted, 1)] {
        let statement = graph.sql().plan(&plan)?;
        let rows = ctx.sql(&statement).await?.collect().await?;
        assert_eq!(rows[0].schema().field(0).name(), "id", "{statement}");
        assert_eq!(rows[0].schema().field(1).name(), "total", "{statement}");
        assert_eq!(
            rows.iter().map(|b| b.num_rows()).sum::<usize>(),
            expected_rows
        );
        if expected_rows == 1 {
            let totals = rows[0]
                .column(1)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap();
            assert_eq!(totals.value(0), 4, "{statement}");
        }
    }
    Ok(())
}

#[tokio::test]
async fn joins_and_materialized_duplicate_columns_keep_their_identities() -> Result<()> {
    let mut graph = DataflowBuilder::new();
    let left = graph.table_input("left", schema())?;
    let right = graph.table_input("right", schema())?;
    let joined = graph.add_plan(
        "joined",
        LogicalPlanBuilder::from(left.plan_ref())
            .join(
                right.plan_ref(),
                JoinType::Inner,
                (vec!["id"], vec!["id"]),
                None,
            )?
            .build()?,
    )?;
    let result = graph.add_plan(
        "result",
        LogicalPlanBuilder::from(joined.plan_ref())
            .project(vec![
                col("left.id").alias("left_id"),
                col("right.id").alias("right_id"),
            ])?
            .build()?,
    )?;
    let graph = graph.finish()?;
    let joined_sql = graph.sql().table(&joined)?;
    let ctx = context("inputs");
    ctx.register_batch(
        TableReference::partial("inputs", "left"),
        batch("id", vec![1, 2]),
    )?;
    ctx.register_batch(
        TableReference::partial("inputs", "right"),
        batch("id", vec![2, 3]),
    )?;
    let joined_rows = ctx.sql(&joined_sql).await?.collect().await?;
    assert_eq!(
        joined_rows.iter().map(|b| b.num_rows()).sum::<usize>(),
        1,
        "{joined_sql}"
    );

    let result_sql = graph.sql().table(&result)?;
    assert!(
        result_sql.contains("\"left.id\"") && result_sql.contains("\"right.id\""),
        "{result_sql}"
    );
    let ctx = context("nodes");
    ctx.register_batch(
        TableReference::partial("nodes", "joined"),
        RecordBatch::try_from_iter(vec![
            ("left.id", Arc::new(Int64Array::from(vec![2])) as _),
            ("right.id", Arc::new(Int64Array::from(vec![2])) as _),
        ])?,
    )?;
    let rows = ctx.sql(&result_sql).await?.collect().await?;
    assert_eq!(rows[0].schema().field(0).name(), "left_id");
    assert_eq!(rows[0].schema().field(1).name(), "right_id");
    assert_eq!(rows[0].num_rows(), 1);
    Ok(())
}

#[tokio::test]
async fn join_keys_resolve_against_their_own_input_when_lineage_is_shared() -> Result<()> {
    let mut graph = DataflowBuilder::new();
    let sales = graph.table_input("sales", schema())?;
    let lower = graph.add_plan(
        "lower",
        LogicalPlanBuilder::from(sales.plan_ref())
            .filter(col("id").lt_eq(lit(2_i64)))?
            .build()?,
    )?;
    let upper = graph.add_plan(
        "upper",
        LogicalPlanBuilder::from(sales.plan_ref())
            .filter(col("id").gt_eq(lit(2_i64)))?
            .build()?,
    )?;
    let joined = graph.add_plan(
        "joined",
        LogicalPlanBuilder::from(lower.plan_ref())
            .join(
                upper.plan_ref(),
                JoinType::LeftSemi,
                (vec!["id"], vec!["id"]),
                None,
            )?
            .build()?,
    )?;
    let graph = graph.finish()?;
    let statement = graph.sql().table(&joined)?;
    let ctx = context("nodes");
    ctx.register_batch(
        TableReference::partial("nodes", "lower"),
        batch("id", vec![1, 2]),
    )?;
    ctx.register_batch(
        TableReference::partial("nodes", "upper"),
        batch("id", vec![2, 3]),
    )?;
    let rows = ctx.sql(&statement).await?.collect().await?;
    let ids = rows[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap();
    assert_eq!(ids.values().as_ref(), &[2], "{statement}");
    Ok(())
}

#[tokio::test]
async fn aliases_and_correlated_subqueries_rebind_columns_in_each_scope() -> Result<()> {
    let mut graph = DataflowBuilder::new();
    let left = graph.table_input("left", schema())?;
    let right = graph.table_input("right", schema())?;
    let outer = Expr::OuterReferenceColumn(
        Arc::new(Field::new("id", DataType::Int64, false)),
        Column::new(Some("left"), "id"),
    );
    let subquery = LogicalPlanBuilder::from(right.plan_ref())
        .filter(col("right.id").eq(outer))?
        .aggregate(Vec::<Expr>::new(), vec![count(lit(1_i64)).alias("n")])?
        .build()?;
    let result = graph.add_plan(
        "matches",
        LogicalPlanBuilder::from(left.plan_ref())
            .project(vec![
                col("left.id"),
                scalar_subquery(Arc::new(subquery)).alias("n"),
            ])?
            .sort(vec![col("id").sort(true, true)])?
            .build()?,
    )?;
    let aliased = graph.add_plan(
        "aliased",
        LogicalPlanBuilder::from(left.plan_ref())
            .alias("x")?
            .project(vec![col("x.id")])?
            .build()?,
    )?;
    let graph = graph.finish()?;
    let statement = graph.sql().table(&result)?;
    let ctx = context("inputs");
    ctx.register_batch(
        TableReference::partial("inputs", "left"),
        batch("id", vec![1, 2]),
    )?;
    ctx.register_batch(
        TableReference::partial("inputs", "right"),
        batch("id", vec![2, 2, 3]),
    )?;
    let rows = ctx.sql(&statement).await?.collect().await?;
    let counts = rows
        .iter()
        .flat_map(|b| {
            b.column(1)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .values()
                .to_vec()
        })
        .collect::<Vec<_>>();
    assert_eq!(counts, vec![0, 2], "{statement}");
    assert_eq!(
        ctx.sql(&graph.sql().table(&aliased)?)
            .await?
            .collect()
            .await?
            .iter()
            .map(|b| b.num_rows())
            .sum::<usize>(),
        2
    );
    Ok(())
}

#[test]
fn scoped_rows_and_same_named_parameters_are_distinct() -> Result<()> {
    let mut graph = DataflowBuilder::new();
    let rows = graph.table_input("rows", schema())?;
    let root_fraction = graph.scalar_input("fraction", DataType::Int64)?;
    let (_, (local_rows, local, nested)) =
        graph.partition_by("regions", rows.plan_ref(), vec![col("id")], |scope| {
            let fraction = scope.scalar_input("fraction", DataType::Int64)?;
            let local =
                scope.add_scalar("threshold", root_fraction.expr_ref() + fraction.expr_ref())?;
            let (_, nested) = scope.partition_by(
                "panels",
                scope.rows().plan_ref(),
                vec![col("id")],
                |child| {
                    let fraction = child.scalar_input("fraction", DataType::Int64)?;
                    child.add_scalar("threshold", local.expr_ref() + fraction.expr_ref())
                },
            )?;
            Ok((scope.rows().clone(), local, nested))
        })?;
    let graph = graph.finish()?;
    assert_eq!(
        graph.sql().scalar(&local)?,
        "$input__fraction + $input__regions__fraction"
    );
    assert_eq!(
        graph.sql().scalar(&nested)?,
        "$scalar__regions__threshold + $input__regions__panels__fraction"
    );
    let local_sql = graph.sql().table(&local_rows)?;
    assert!(
        local_sql.contains("regions.\"rows\".\"local\""),
        "{local_sql}"
    );
    Ok(())
}

#[test]
fn base_imports_parameters_and_foreign_handles_are_checked() -> Result<()> {
    let mut base = DataflowBuilder::new();
    let input = base.table_input("sales", schema())?;
    let amount = base.scalar_input("amount", DataType::Int64)?;
    let node = base.add_plan("rows", input.plan_ref())?;
    let output = base.table_output("sales", &node)?;
    let scalar = base.add_scalar("amount", amount.expr_ref() + lit(1_i64))?;
    let scalar_output = base.scalar_output("amount", &scalar)?;
    let base = base.finish()?;
    let mut extra = DataflowBuilder::with_base(&base.interface());
    let imported = extra.import_table("sales", &output)?;
    let imported_scalar = extra.import_scalar("amount", &scalar_output)?;
    let combined = extra.add_scalar("combined", imported_scalar.expr_ref() + amount.expr_ref())?;
    let extra = extra.finish()?;
    assert!(extra.sql().table(&imported)?.contains("base.outputs.sales"));
    assert!(extra
        .sql()
        .plan(&input.plan_ref())?
        .contains("base.inputs.sales"));
    assert_eq!(
        extra.sql().scalar(&imported_scalar)?,
        "$base_output__amount"
    );
    assert_eq!(
        extra.sql().scalar(&combined)?,
        "$scalar__amount + $base_input__amount"
    );
    assert!(matches!(
        extra.sql().table(&node),
        Err(Error::ForeignHandle)
    ));
    assert!(matches!(
        extra.sql().scalar(&scalar),
        Err(Error::ForeignHandle)
    ));
    assert!(matches!(
        extra.sql().table_output(&output),
        Err(Error::ForeignHandle)
    ));
    assert!(matches!(
        extra.sql().scalar_output(&scalar_output),
        Err(Error::ForeignHandle)
    ));
    let empty = DataflowBuilder::new().finish()?;
    assert!(empty.sql().plan(&input.plan_ref()).is_err());
    assert!(empty.sql().expr(&amount.expr_ref()).is_err());
    Ok(())
}

#[test]
fn formatting_keeps_udfs_symbolic_and_quotes_unusual_names() -> Result<()> {
    let mut graph = DataflowBuilder::new();
    let data = batch("id", vec![1]);
    let rows = graph.table_snapshot(
        "a.b\"table",
        TableSnapshot::from_batches(data.schema(), vec![data])?,
    )?;
    let ordinary = graph.scalar_input("a_b", DataType::Int64)?;
    let escaped = graph.scalar_input("a b", DataType::Int64)?;
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let udf = create_udf(
        "inspect_only",
        vec![DataType::Int64],
        DataType::Int64,
        Volatility::Immutable,
        Arc::new(move |args| {
            counter.fetch_add(1, Ordering::Relaxed);
            Ok(args[0].clone())
        }),
    );
    let scalar = graph.add_scalar("result", udf.call(vec![ordinary.expr_ref()]))?;
    let graph = graph.finish()?;
    assert_eq!(graph.sql().scalar(&scalar)?, "inspect_only($input__a_5fb)");
    assert_eq!(graph.sql().expr(&escaped.expr_ref())?, "$input__a_20b");
    let statement = graph.sql().table(&rows)?;
    assert!(statement.contains("assets.\"a.b\"\"table\""), "{statement}");
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    Ok(())
}
