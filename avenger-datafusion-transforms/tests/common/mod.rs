//! Helpers that the tests share: a session that runs the plans of the transforms, tables, the
//! views of planned calls, and expressions in DataFusion's SQL over a query's schema.

#![allow(dead_code)]

use std::sync::Arc;

use avenger_datafusion_transforms::shared::{InlineShared, SharedExtensionPlanner};
use avenger_datafusion_transforms::table_function::TableFunctionExtensionPlanner;
use datafusion::arrow::array::{ArrayRef, RecordBatch};
use datafusion::arrow::datatypes::{Field, Schema};
use datafusion::catalog::MemTable;
use datafusion::catalog::Session;
use datafusion::common::Result;
use datafusion::datasource::ViewTable;
use datafusion::execution::SessionStateBuilder;
use datafusion::execution::context::QueryPlanner;
use datafusion::logical_expr::{Expr, LogicalPlan};
use datafusion::physical_plan::ExecutionPlan;
use datafusion::physical_planner::{DefaultPhysicalPlanner, PhysicalPlanner};
use datafusion::prelude::SessionContext;

/// The query planner of the tests, with the extension planners of the transforms' nodes.
#[derive(Debug)]
struct Planner;

#[async_trait::async_trait]
impl QueryPlanner for Planner {
    async fn create_physical_plan(
        &self,
        plan: &LogicalPlan,
        session: &dyn Session,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        DefaultPhysicalPlanner::with_extension_planners(vec![
            Arc::new(TableFunctionExtensionPlanner),
            Arc::new(SharedExtensionPlanner::default()),
        ])
        .create_physical_plan(plan, session)
        .await
    }
}

/// A session that runs the plans of the transforms, and computes cheap shared inputs again.
pub fn session() -> SessionContext {
    let state = SessionStateBuilder::new()
        .with_default_features()
        .with_query_planner(Arc::new(Planner))
        .build();
    let ctx = SessionContext::new_with_state(state);
    ctx.add_optimizer_rule(Arc::new(InlineShared));
    ctx
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
