//! Helpers that the tests share: a session that runs the plans of the transforms, tables, the
//! views of planned calls, expressions in DataFusion's SQL over a query's schema, the calls of
//! the transforms from short SQL, and the rows of a query as text.

#![allow(dead_code)]

use std::sync::Arc;

use avenger_datafusion_shared::{InlineShared, SharedExtensionPlanner};
use avenger_datafusion_transforms::bin::{self, BinCall, BinOptions};
use avenger_datafusion_transforms::impute::{self, ImputeCall, Named};
use avenger_datafusion_transforms::lump::LumpAggregateFusion;
use avenger_datafusion_transforms::table_function::{ArgError, TableFunctionExtensionPlanner};
use avenger_datafusion_transforms::timeunit::{self, TimeUnitCall, TimeUnitOptions, Unit};
use datafusion::arrow::array::{Array, ArrayRef, RecordBatch};
use datafusion::arrow::datatypes::{Field, Schema};
use datafusion::arrow::util::display::array_value_to_string;
use datafusion::catalog::MemTable;
use datafusion::catalog::Session;
use datafusion::common::{Column, DFSchema, DataFusionError, Result};
use datafusion::datasource::ViewTable;
use datafusion::execution::SessionStateBuilder;
use datafusion::execution::context::QueryPlanner;
use datafusion::logical_expr::{Expr, LogicalPlan};
use datafusion::physical_plan::ExecutionPlan;
use datafusion::physical_planner::{DefaultPhysicalPlanner, PhysicalPlanner};
use datafusion::prelude::{SessionConfig, SessionContext};

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
    session_with(SessionConfig::new())
}

/// [`session`] with the configuration `config`.
pub fn session_with(config: SessionConfig) -> SessionContext {
    let state = SessionStateBuilder::new()
        .with_config(config)
        .with_default_features()
        .with_query_planner(Arc::new(Planner))
        .build();
    let ctx = SessionContext::new_with_state(state);
    ctx.add_optimizer_rule(Arc::new(InlineShared));
    ctx.add_analyzer_rule(Arc::new(LumpAggregateFusion));
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

/// Runs the statement `sql`.
pub async fn exec(ctx: &SessionContext, sql: &str) -> Result<()> {
    ctx.sql(sql).await?.collect().await?;
    Ok(())
}

/// The rows of the query `sql` as text, `NULL` for a null, sorted.
pub async fn rows(ctx: &SessionContext, sql: &str) -> Result<Vec<Vec<String>>> {
    let mut rows = Vec::new();
    for batch in ctx.sql(sql).await?.collect().await? {
        for i in 0..batch.num_rows() {
            let cell = |c: &ArrayRef| match c.is_null(i) {
                true => Ok("NULL".to_string()),
                false => array_value_to_string(c, i),
            };
            rows.push(batch.columns().iter().map(cell).collect::<Result<_, _>>()?);
        }
    }
    rows.sort();
    Ok(rows)
}

/// The options of `timeunit` with the unit `text`, and the calendar that it names.
pub fn unit(text: &str) -> TimeUnitOptions {
    let (unit, calendar) = Unit::parse(text).expect("a unit");
    let mut options = TimeUnitOptions {
        unit: Some(unit),
        ..Default::default()
    };
    options.take_calendar("unit", calendar).expect("a calendar");
    options
}

/// Registers as the view `name` the query `input` with the column `u`, the time unit of `value`
/// by `options` in the window partitions of the columns `partition`.
pub async fn time_units(
    ctx: &SessionContext,
    name: &str,
    input: &str,
    value: &str,
    partition: &[&str],
    options: TimeUnitOptions,
) -> Result<()> {
    let value = expr(ctx, input, value).await?;
    let partition = partition
        .iter()
        .map(|c| datafusion::prelude::col(*c))
        .collect();
    let call = TimeUnitCall {
        value,
        partition,
        options,
    };
    view(ctx, name, input, |plan| {
        timeunit::plan(plan, call, Column::from_name("u"))
    })
    .await
}

/// Registers as the view `name` the query `input` with the column `b`, the bin of `value` by
/// `options` in the window partitions of the columns `partition`.
pub async fn bins(
    ctx: &SessionContext,
    name: &str,
    input: &str,
    value: &str,
    partition: &[&str],
    options: BinOptions,
) -> Result<()> {
    let value = expr(ctx, input, value).await?;
    let partition = partition
        .iter()
        .map(|c| datafusion::prelude::col(*c))
        .collect();
    let call = BinCall {
        value,
        partition,
        options,
    };
    view(ctx, name, input, |plan| {
        bin::plan(plan, call, Column::from_name("b"))
    })
    .await
}

/// The column `name` of `schema`, with its qualifier.
fn column(schema: &DFSchema, name: &str) -> Result<Column> {
    let (qualifier, field) = schema.qualified_field_with_unqualified_name(name)?;
    Ok(Column::new(qualifier.cloned(), field.name()))
}

/// The call of `impute` over the query `input`: the names of its key, group, and value columns,
/// and its named arguments in DataFusion's SQL over the schema of the input. As in a front end,
/// `flag` is a name, `partition_by` is a column or an array of columns, and an array of `frame`,
/// `keys`, or `extra_keys` is a list.
pub async fn impute_call(
    ctx: &SessionContext,
    input: &str,
    key: &str,
    groups: &[&str],
    value: &str,
    named: &[(&str, &str)],
) -> Result<ImputeCall> {
    let plan = self::input(ctx, input).await?;
    let schema = plan.schema();
    let mut args = Vec::new();
    for (name, sql) in named {
        let arg = match *name {
            "flag" => Named::Name(sql.trim_matches('\'').to_string()),
            name => {
                let expr = expr(ctx, input, sql).await?;
                let list = match &expr {
                    Expr::ScalarFunction(f) if f.name() == "make_array" => Some(f.args.clone()),
                    _ => None,
                };
                match (name, list) {
                    ("partition_by", list) => Named::Columns(
                        (list.unwrap_or_else(|| vec![expr]).into_iter())
                            .map(|e| match e {
                                Expr::Column(c) => column(schema, &c.name),
                                e => panic!("a partition column, not {e}"),
                            })
                            .collect::<Result<_>>()?,
                    ),
                    ("frame" | "keys" | "extra_keys", Some(list)) => Named::List(list),
                    _ => Named::Value(expr),
                }
            }
        };
        args.push((name.to_string(), arg));
    }
    Ok(ImputeCall {
        key: column(schema, key)?,
        groups: (groups.iter())
            .map(|g| column(schema, g))
            .collect::<Result<_>>()?,
        value: column(schema, value)?,
        named: args,
    })
}

/// Registers as the view `name` the plan of `call` over the query `input`.
pub async fn imputed(
    ctx: &SessionContext,
    name: &str,
    input: &str,
    call: ImputeCall,
) -> Result<()> {
    view(ctx, name, input, |plan| {
        impute::plan(plan, call).map_err(|e| DataFusionError::Plan(e.message))
    })
    .await
}

/// The error of the plan of `call` over the query `input`.
pub async fn impute_error(ctx: &SessionContext, input: &str, call: ImputeCall) -> Result<ArgError> {
    match impute::plan(self::input(ctx, input).await?, call) {
        Ok(_) => panic!("no error: {input}"),
        Err(e) => Ok(e),
    }
}
