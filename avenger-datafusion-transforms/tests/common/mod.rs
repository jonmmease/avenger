//! Helpers that the tests share: a session that runs the plans of the transforms, tables, the
//! views of planned calls, and expressions in DataFusion's SQL over a query's schema.

#![allow(dead_code)]

use std::sync::Arc;

use avenger_datafusion_transforms::table_function::TransformsQueryPlanner;
use datafusion::arrow::array::{ArrayRef, RecordBatch};
use datafusion::arrow::datatypes::{Field, Schema};
use datafusion::catalog::MemTable;
use datafusion::common::Result;
use datafusion::datasource::ViewTable;
use datafusion::execution::SessionStateBuilder;
use datafusion::logical_expr::{Expr, LogicalPlan};
use datafusion::prelude::SessionContext;

/// A session that runs the plans of the transforms.
pub fn session() -> SessionContext {
    let state = SessionStateBuilder::new()
        .with_default_features()
        .with_query_planner(Arc::new(TransformsQueryPlanner))
        .build();
    SessionContext::new_with_state(state)
}

/// Registers the columns as the table `name`, in place of any table of that name.
pub fn register(ctx: &SessionContext, name: &str, columns: Vec<(&str, ArrayRef)>) -> Result<()> {
    let schema = Arc::new(Schema::new(
        columns
            .iter()
            .map(|(n, a)| Field::new(*n, a.data_type().clone(), true))
            .collect::<Vec<_>>(),
    ));
    let batch = RecordBatch::try_new(
        schema.clone(),
        columns.into_iter().map(|(_, a)| a).collect(),
    )?;
    ctx.deregister_table(name)?;
    ctx.register_table(
        name,
        Arc::new(MemTable::try_new(schema, vec![vec![batch]])?),
    )?;
    Ok(())
}

/// The logical plan of the query `input`.
pub async fn input(ctx: &SessionContext, input: &str) -> Result<LogicalPlan> {
    Ok(ctx.sql(input).await?.into_unoptimized_plan())
}

/// The expression `sql` in DataFusion's SQL over the schema of the query `input`, for the
/// arguments of a call, such as `sum(tons)`.
pub async fn expr(ctx: &SessionContext, input: &str, sql: &str) -> Result<Expr> {
    let plan = self::input(ctx, input).await?;
    ctx.state().create_logical_expr(sql, plan.schema())
}

/// Registers as the view `name`, in place of any table of that name, the plan that `plan` makes
/// of the plan of the query `input`.
pub async fn view(
    ctx: &SessionContext,
    name: &str,
    input: &str,
    plan: impl FnOnce(LogicalPlan) -> Result<LogicalPlan>,
) -> Result<()> {
    let plan = plan(self::input(ctx, input).await?)?;
    ctx.deregister_table(name)?;
    ctx.register_table(name, Arc::new(ViewTable::new(plan, None)))?;
    Ok(())
}
