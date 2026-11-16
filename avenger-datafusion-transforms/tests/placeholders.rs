//! The options of the transforms as values of the plan: each transform with a placeholder in an
//! option, bound by `with_param_values`, gives the result of the same call with the literal,
//! and a bound value that fails a check errors with the option's name.

mod common;

use std::sync::Arc;

use avenger_datafusion_transforms::impute::{self, ImputeCall, Named};
use avenger_datafusion_transforms::lump::{self, Keep, LumpCall, Nulls};
use avenger_datafusion_transforms::{bin, plan_windows, stack, timeunit};
use common::{exec, input, session};
use datafusion::arrow::array::{Array, ArrayRef, RecordBatch};
use datafusion::arrow::datatypes::{DataType, Field};
use datafusion::arrow::util::display::array_value_to_string;
use datafusion::common::{Column, ParamValues, Result, ScalarValue};
use datafusion::logical_expr::expr::Placeholder;
use datafusion::logical_expr::{Expr, ExprFunctionExt, LogicalPlan, LogicalPlanBuilder};
use datafusion::prelude::{SessionContext, col, lit};

/// The rows of `c`: groups `g`, numbers `x`, the order `i`, and dates `d` with gaps.
const C: &str = "CREATE TABLE c AS SELECT * FROM (VALUES
    ('a', 1.0, 1, DATE '2024-01-15'), ('a', 7.0, 2, DATE '2024-03-02'),
    ('b', 3.0, 3, DATE '2024-01-20'), ('b', 18.0, 4, DATE '2024-04-09'),
    ('c', 5.0, 5, DATE '2024-02-11')) AS t(g, x, i, d)";

/// The placeholder `$id` of the type `ty`.
fn placeholder(id: &str, ty: DataType) -> Expr {
    let field = Arc::new(Field::new(id, ty, true));
    Expr::Placeholder(Placeholder::new_with_field(format!("${id}"), Some(field)))
}

/// The rows of `plan` with the parameters `values`, as text, sorted.
async fn run(
    ctx: &SessionContext,
    plan: LogicalPlan,
    values: Vec<(&str, ScalarValue)>,
) -> Result<Vec<String>> {
    let values = ParamValues::from(
        values
            .into_iter()
            .map(|(id, v)| (id.to_string(), v))
            .collect::<Vec<_>>(),
    );
    let batches = ctx
        .execute_logical_plan(plan.with_param_values(values)?)
        .await?
        .collect()
        .await?;
    Ok(lines(&batches))
}

fn lines(batches: &[RecordBatch]) -> Vec<String> {
    let mut lines = Vec::new();
    for batch in batches {
        for i in 0..batch.num_rows() {
            let cell = |c: &ArrayRef| match c.is_null(i) {
                true => "NULL".to_string(),
                false => array_value_to_string(c, i).unwrap(),
            };
            let cells: Vec<String> = batch.columns().iter().map(cell).collect();
            lines.push(cells.join(" | "));
        }
    }
    lines.sort();
    lines
}

/// The plan of `c` with the placeholder windows `windows` as columns, and the column `g`.
async fn windows(ctx: &SessionContext, windows: Vec<Expr>) -> Result<LogicalPlan> {
    let plan = input(ctx, "SELECT * FROM c").await?;
    let outputs: Vec<Expr> = (windows.iter())
        .map(|w| Expr::Column(Column::from_name(w.schema_name().to_string())))
        .collect();
    let (plan, rest) = plan_windows(plan, windows)?;
    assert!(rest.is_empty());
    LogicalPlanBuilder::from(plan)
        .project([col("g")].into_iter().chain(outputs))?
        .build()
}

#[tokio::test]
async fn the_options_of_bin_and_timeunit_are_values() -> Result<()> {
    let ctx = session();
    exec(&ctx, C).await?;
    // A bin with a placeholder step, and a time unit with a placeholder unit, against literals.
    let calls = |step: Expr, unit: Expr| -> Result<Vec<Expr>> {
        let bins =
            bin::window(false).call(vec![col("x"), bin::options(vec![("step".into(), step)])?]);
        let units = timeunit::window(true).call(vec![
            col("d"),
            timeunit::options(vec![("unit".into(), unit)])?,
        ]);
        Ok(vec![bins, units])
    };
    let literal = windows(&ctx, calls(lit(5.0), lit("week(2)"))?).await?;
    let bound = windows(
        &ctx,
        calls(
            placeholder("step", DataType::Float64),
            placeholder("unit", DataType::Utf8),
        )?,
    )
    .await?;
    let want = run(&ctx, literal, vec![]).await?;
    let values = vec![
        ("step", ScalarValue::from(5.0)),
        ("unit", ScalarValue::from("week(2)")),
    ];
    assert_eq!(run(&ctx, bound.clone(), values).await?, want);
    // A bound value that fails the check of its option.
    let values = vec![
        ("step", ScalarValue::from(-1.0)),
        ("unit", ScalarValue::from("week")),
    ];
    let error = run(&ctx, bound.clone(), values)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("The step of bin must be a positive number, not -1"),
        "{error}"
    );
    let values = vec![
        ("step", ScalarValue::from(5.0)),
        ("unit", ScalarValue::from("week(9)")),
    ];
    let error = run(&ctx, bound, values).await.unwrap_err().to_string();
    assert!(
        error.contains("from 1 for Sunday to 7 for Saturday, not 9"),
        "{error}"
    );
    Ok(())
}

#[tokio::test]
async fn the_offset_of_stack_is_a_value() -> Result<()> {
    let ctx = session();
    exec(&ctx, C).await?;
    let c = input(&ctx, "SELECT * FROM c").await?;
    let plan = |offset: Expr| -> Result<LogicalPlan> {
        let window = stack::window()
            .call(vec![col("x"), offset])
            .order_by(vec![col("i").sort(true, false)])
            .build()?;
        LogicalPlanBuilder::from(c.clone())
            .window(vec![window.alias("s")])?
            .project([col("i"), col("s")])?
            .build()
    };
    let want = run(&ctx, plan(lit("normalize"))?, vec![]).await?;
    let bound = plan(placeholder("offset", DataType::Utf8))?;
    assert_eq!(
        run(
            &ctx,
            bound.clone(),
            vec![("offset", ScalarValue::from("normalize"))]
        )
        .await?,
        want
    );
    let error = run(&ctx, bound, vec![("offset", ScalarValue::from("up"))])
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("The offset of stack is 'zero', 'normalize', or 'center', not 'up'"),
        "{error}"
    );
    Ok(())
}

#[tokio::test]
async fn the_options_of_lump_are_values() -> Result<()> {
    let ctx = session();
    exec(&ctx, C).await?;
    let c = input(&ctx, "SELECT * FROM c").await?;
    let plan = |n: Expr, other: Expr| -> Result<LogicalPlan> {
        let call = LumpCall {
            category: col("g"),
            partition: Vec::new(),
            weight: None,
            keep: Keep::First(n),
            other,
            null_category: Nulls::Keep.expr(),
        };
        let plan = lump::plan(c.clone(), call, Column::from_name("l"))?;
        LogicalPlanBuilder::from(plan)
            .project([col("g"), col("l")])?
            .build()
    };
    let want = run(&ctx, plan(lit(1i64), lit("Rest"))?, vec![]).await?;
    let bound = plan(
        placeholder("n", DataType::Int64),
        placeholder("other", DataType::Utf8),
    )?;
    let values = vec![
        ("n", ScalarValue::from(1i64)),
        ("other", ScalarValue::from("Rest")),
    ];
    assert_eq!(run(&ctx, bound, values).await?, want);
    Ok(())
}

#[tokio::test]
async fn the_values_of_impute_are_values() -> Result<()> {
    let ctx = session();
    exec(&ctx, "CREATE TABLE counts AS SELECT * FROM (VALUES ('a', 1, 10), ('a', 3, 30), ('b', 2, 20)) AS t(s, k, n)").await?;
    let counts = input(&ctx, "SELECT * FROM counts").await?;
    let plan = |value: Expr| -> Result<LogicalPlan> {
        let call = ImputeCall {
            key: Column::from_name("k"),
            groups: vec![Column::from_name("s")],
            value: Column::from_name("n"),
            named: vec![("value".to_string(), Named::Value(value))],
        };
        impute::plan(counts.clone(), call)
            .map_err(|error| datafusion::common::DataFusionError::Plan(error.message))
    };
    let want = run(&ctx, plan(lit(0i64))?, vec![]).await?;
    let bound = plan(placeholder("fill", DataType::Int64))?;
    assert_eq!(
        run(&ctx, bound, vec![("fill", ScalarValue::from(0i64))]).await?,
        want
    );
    Ok(())
}

#[tokio::test]
async fn an_aggregate_grouped_by_a_lump_binds_its_placeholders() {
    // The fusion of the aggregate with the lump reads copies of the call and of its input, so it
    // leaves out a lump with a placeholder, which plans bound without the fusion.
    let ctx = session();
    exec(&ctx, C).await.unwrap();
    let c = input(&ctx, "SELECT * FROM c").await.unwrap();
    let plan = |since: Expr, n: Expr| -> Result<LogicalPlan> {
        let input = LogicalPlanBuilder::from(c.clone())
            .filter(col("i").gt_eq(since))?
            .build()?;
        let call = LumpCall {
            category: col("g"),
            partition: Vec::new(),
            weight: None,
            keep: Keep::First(n),
            other: lit("Rest"),
            null_category: Nulls::Keep.expr(),
        };
        let plan = lump::plan(input, call, Column::from_name("l"))?;
        LogicalPlanBuilder::from(plan)
            .aggregate(
                vec![col("l")],
                vec![datafusion::functions_aggregate::count::count(lit(1))],
            )?
            .build()
    };
    let want = run(&ctx, plan(lit(2i64), lit(1i64)).unwrap(), vec![])
        .await
        .unwrap();
    let bound = plan(
        placeholder("since", DataType::Int64),
        placeholder("n", DataType::Int64),
    )
    .unwrap();
    let values = vec![
        ("since", ScalarValue::from(2i64)),
        ("n", ScalarValue::from(1i64)),
    ];
    assert_eq!(run(&ctx, bound, values).await.unwrap(), want);
}
