//! `|> CHART`: the mark planner, a relation planner that plans the `FROM`-position call
//! `__avenger_chart((<input>), <mark call>, partition_by => [...])` of the syntax pass as the
//! lifting of its mark call over its input, and [`lift`], which prints the lifting.
//!
//! The mark planner plans the input one time, for the schema that naming and typing need, then
//! generates the lifting ([`lifting`]) as pipe operators over the reserved table
//! `__avenger_input`, lowers it with the syntax pass, replaces the reserved table with the input,
//! and plans the result one time.

mod call;
mod classify;
mod lifting;
mod roles;
mod stage4;

use std::{ops::ControlFlow, sync::Arc};

use datafusion::{
    arrow::datatypes::DataType,
    common::{DFSchema, Result},
    execution::SessionState,
    logical_expr::{
        planner::{PlannedRelation, RelationPlanner, RelationPlannerContext, RelationPlanning},
        ExprSchemable,
    },
    prelude::SessionContext,
    sql::{
        parser::Statement as DFStatement,
        sqlparser::ast::{
            Expr, FunctionArg, FunctionArgExpr, Ident, ObjectName, PipeOperator, Query, Statement,
            TableFactor, Visit, VisitMut, Visitor, VisitorMut,
        },
    },
};

use crate::{
    pipeline,
    registry::Registry,
    syntax::{self, build, ChartOperator},
};

/// The name of the marker of `|> CHART` and of the call of the mark planner.
pub(crate) const MARKER: &str = "__avenger_chart";

/// The reserved table of the lifting, which the mark planner replaces with the input of the call.
const INPUT: &str = "__avenger_input";

/// The parameter of the call of the mark planner that takes the partition columns, the groups of
/// its `CHART`.
pub(crate) const PARTITION_BY: &str = "partition_by";

/// The parameter of the call of the mark planner that takes the name of its chart column.
pub(crate) const NAME: &str = "name";

/// The name of the chart column of a `CHART` whose mark call has no alias.
pub(crate) const CHART: &str = "chart";

/// Registers the mark planner in `ctx`, one time.
pub(crate) fn register(ctx: &SessionContext) -> Result<()> {
    let registered = (ctx.state().relation_planners().iter())
        .any(|planner| format!("{planner:?}") == "ChartPlanner");
    if !registered {
        ctx.register_relation_planner(Arc::new(ChartPlanner))?;
    }
    Ok(())
}

/// The relation planner of the calls of `__avenger_chart`.
#[derive(Debug)]
struct ChartPlanner;

impl RelationPlanner for ChartPlanner {
    fn plan_relation(
        &self,
        relation: TableFactor,
        ctx: &mut dyn RelationPlannerContext,
    ) -> Result<RelationPlanning> {
        let TableFactor::Table {
            name,
            args: Some(args),
            alias,
            ..
        } = &relation
        else {
            return Ok(RelationPlanning::Original(Box::new(relation)));
        };
        if name.to_string() != MARKER {
            return Ok(RelationPlanning::Original(Box::new(relation)));
        }
        let (input, mark, partition, chart) = read_call(&args.args);
        let alias = alias.clone();
        let input_plan = ctx.plan(build::derived(input.clone()))?;
        let schema = input_plan.schema().as_ref().clone();
        let mut env = PlannerEnv { ctx };
        let operators = lifting::generate(&mut env, &mark, &partition, &chart, &schema)?;
        let mut query = build::query(build::select(vec![build::wildcard()], Some(table(INPUT))));
        query.pipe_operators = operators;
        syntax::lower_query(&env, &mut query)?;
        let _ = VisitMut::visit(&mut query, &mut ReplaceInput { input: &input });
        let plan = env.ctx.plan(build::derived(query))?;
        Ok(RelationPlanning::Planned(Box::new(PlannedRelation::new(
            plan, alias,
        ))))
    }
}

/// The input, the mark call, the partition columns, and the name of the chart column of a call of
/// the mark planner, as the syntax pass writes them.
fn read_call(args: &[FunctionArg]) -> (Query, Expr, Vec<Expr>, Ident) {
    let expr = |index: usize| match args.get(index) {
        Some(
            FunctionArg::Unnamed(FunctionArgExpr::Expr(expr))
            | FunctionArg::Named {
                arg: FunctionArgExpr::Expr(expr),
                ..
            },
        ) => expr.clone(),
        _ => unreachable!("the syntax pass writes the call of the mark planner"),
    };
    let Expr::Subquery(input) = expr(0) else {
        unreachable!("the input of a chart call is a subquery")
    };
    let Expr::Array(partition) = expr(2) else {
        unreachable!("the partition of a chart call is an array")
    };
    let Expr::Identifier(chart) = expr(3) else {
        unreachable!("the name of a chart call is an identifier")
    };
    (*input, expr(1), partition.elem, chart)
}

/// The table `name`.
fn table(name: &str) -> TableFactor {
    TableFactor::Table {
        name: ObjectName::from(vec![Ident::new(name)]),
        alias: None,
        args: None,
        with_hints: Vec::new(),
        version: None,
        with_ordinality: false,
        partitions: Vec::new(),
        json_path: None,
        sample: None,
        index_hints: Vec::new(),
    }
}

/// Replaces the reserved table of the lifting with the derived table of the input.
struct ReplaceInput<'a> {
    input: &'a Query,
}

impl VisitorMut for ReplaceInput<'_> {
    type Break = ();

    fn pre_visit_table_factor(&mut self, table: &mut TableFactor) -> ControlFlow<()> {
        if matches!(table, TableFactor::Table { name, args: None, .. } if name.to_string() == INPUT)
        {
            *table = build::derived(self.input.clone());
        }
        ControlFlow::Continue(())
    }
}

/// The environment of the mark planner: the functions of its context provider, and the types of
/// its expression planner.
struct PlannerEnv<'a> {
    ctx: &'a mut dyn RelationPlannerContext,
}

impl Registry for PlannerEnv<'_> {
    fn parameter_names(&self, name: &str) -> Option<Vec<String>> {
        let provider = self.ctx.context_provider();
        if let Some(function) = provider.get_function_meta(name) {
            function.signature().parameter_names.clone()
        } else if let Some(function) = provider.get_aggregate_meta(name) {
            function.signature().parameter_names.clone()
        } else {
            (provider.get_window_meta(name)?.signature().parameter_names).clone()
        }
    }

    fn is_aggregate(&self, name: &str) -> bool {
        self.ctx
            .context_provider()
            .get_aggregate_meta(name)
            .is_some()
    }

    fn is_window(&self, name: &str) -> bool {
        self.ctx.context_provider().get_window_meta(name).is_some()
    }
}

impl lifting::Env for PlannerEnv<'_> {
    fn data_type(&mut self, expr: Expr, schema: &DFSchema) -> Result<DataType> {
        self.ctx.sql_to_expr(expr, schema)?.get_type(schema)
    }
}

/// The environment of [`lift`]: the functions of the session, and the types of its expression
/// planner.
struct LiftEnv<'a> {
    state: &'a SessionState,
}

impl Registry for LiftEnv<'_> {
    fn parameter_names(&self, name: &str) -> Option<Vec<String>> {
        self.state.parameter_names(name)
    }

    fn is_aggregate(&self, name: &str) -> bool {
        self.state.is_aggregate(name)
    }

    fn is_window(&self, name: &str) -> bool {
        self.state.is_window(name)
    }
}

impl lifting::Env for LiftEnv<'_> {
    fn data_type(&mut self, expr: Expr, schema: &DFSchema) -> Result<DataType> {
        (self.state.create_logical_expr(&expr.to_string(), schema)?).get_type(schema)
    }
}

/// The statement `text` with each `|> CHART` call replaced by its lifting in pipe form, with the
/// named windows of the mark resolved and the options as written, so that the text is valid
/// avenger-sql and runs to the same chart. It plans the input of each call for its schema.
pub async fn lift(ctx: &SessionContext, text: &str) -> Result<String> {
    let state = ctx.state();
    let mut statement = pipeline::parse_tokens(text)?;
    while let ControlFlow::Break(found) = Visit::visit(&statement, &mut FirstMarker) {
        let (mut prefix, chart) = found?;
        prefix.pipe_operators.extend(chart.extend.clone());
        let mut lowered = prefix;
        syntax::lower_query(&state, &mut lowered)?;
        let plan = state
            .statement_to_plan(DFStatement::Statement(Box::new(Statement::Query(
                Box::new(lowered),
            ))))
            .await?;
        let schema = plan.schema().as_ref().clone();
        let operators = lifting::generate(
            &mut LiftEnv { state: &state },
            &chart.mark,
            &chart.groups,
            &chart.name,
            &schema,
        )?;
        let operators = (chart.extend.into_iter()).chain(operators).collect();
        let _ = VisitMut::visit(
            &mut statement,
            &mut SpliceMarker {
                operators: Some(operators),
            },
        );
    }
    Ok(statement.to_string())
}

/// The index of the first `|> CHART` of `query`.
fn marker(query: &Query) -> Option<usize> {
    (query.pipe_operators.iter()).position(syntax::is_chart)
}

/// Breaks at the first query with a `|> CHART`, with the prefix before it and the operator.
struct FirstMarker;

impl Visitor for FirstMarker {
    type Break = Result<(Query, ChartOperator)>;

    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<Self::Break> {
        let Some(index) = marker(query) else {
            return ControlFlow::Continue(());
        };
        let mut prefix = query.clone();
        let operator = (prefix.pipe_operators.drain(index..).next()).expect("the operator");
        ControlFlow::Break(syntax::read_chart(&operator).map(|chart| (prefix, chart)))
    }
}

/// Replaces the first `|> CHART` with the operators of its lifting, which compute its group
/// expressions first.
struct SpliceMarker {
    operators: Option<Vec<PipeOperator>>,
}

impl VisitorMut for SpliceMarker {
    type Break = ();

    fn pre_visit_query(&mut self, query: &mut Query) -> ControlFlow<()> {
        let Some(index) = marker(query) else {
            return ControlFlow::Continue(());
        };
        let rest = query.pipe_operators.split_off(index + 1);
        query.pipe_operators.pop();
        query
            .pipe_operators
            .extend(self.operators.take().unwrap_or_default());
        query.pipe_operators.extend(rest);
        ControlFlow::Break(())
    }
}
