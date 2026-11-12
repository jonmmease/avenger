//! The charts of a pipe `AGGREGATE`: [`lifts`], which tells a mark call that lifts, the mark
//! planner, a relation planner that plans the `FROM`-position call `__avenger_chart((<input>),
//! <mark call>, partition_by => [...])` of the syntax pass as the lifting of its mark call over its
//! input, and [`lift`], which prints the lifting.
//!
//! The mark planner plans the input one time, for the schema that naming and typing need, then
//! generates the lifting ([`lifting`]) as pipe operators over the reserved table
//! `__avenger_input`, lowers it with the syntax pass, replaces the reserved table with the input,
//! and plans the result one time.

mod call;
mod classify;
mod impute;
mod lifting;
mod roles;
mod stage4;
mod text;

use std::{
    ops::{ControlFlow, Range},
    sync::Arc,
};

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
        sqlparser::{
            ast::{
                Expr, FunctionArg, FunctionArgExpr, FunctionArgumentClause, FunctionArguments,
                Ident, ObjectName, PipeOperator, Query, Statement, TableFactor, Visit, VisitMut,
                Visitor, VisitorMut,
            },
            tokenizer::{Token, Tokenizer},
        },
    },
};

use crate::{
    grammar, layout, pipeline,
    registry::Registry,
    syntax::{self, build, ChartAggregate},
};

/// The name of the call of the mark planner.
pub(crate) const MARKER: &str = "__avenger_chart";

/// The reserved table of the lifting, which the mark planner replaces with the input of the call.
const INPUT: &str = "__avenger_input";

/// The parameter of the call of the mark planner that takes the partition columns, the groups of
/// its `AGGREGATE`.
pub(crate) const PARTITION_BY: &str = "partition_by";

/// The parameter of the call of the mark planner that takes the name of its chart column.
pub(crate) const NAME: &str = "name";

/// Whether the mark call `expr` lifts: it holds a statistic that the stages of its lifting compute,
/// an aggregate, a window function, `stack`, or a discretizer, `bin`, `timeunit`, or `lump`, in an
/// argument or in its draw order, or it takes the `impute` option. Each of these is SQL that
/// DataFusion rejects in the arguments of an aggregate, so a mark call that does not lift is a
/// call of its mark aggregate, and its lifting is itself.
pub(crate) fn lifts(expr: &Expr, registry: &dyn Registry) -> bool {
    let Expr::Function(function) = expr else {
        return false;
    };
    let FunctionArguments::List(list) = &function.args else {
        return false;
    };
    let impute = (list.args.iter()).any(|argument| {
        matches!(argument, FunctionArg::Named { name, .. } if name.value.eq_ignore_ascii_case(call::IMPUTE))
    });
    let order = (list.clauses.iter()).flat_map(|clause| match clause {
        FunctionArgumentClause::OrderBy(items) => items.iter().map(|item| &item.expr).collect(),
        _ => Vec::new(),
    });
    impute
        || (classify::arguments(function).chain(order))
            .any(|value| classify::level(value, registry) > classify::Level::Row)
}

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

/// A query in a form that a user could have typed, which [`lift`] returns: its text, and the byte
/// ranges of the text that `lift` wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lifted {
    /// The text of the query, with each pipe `AGGREGATE` whose mark calls lift in place of the
    /// stages of its lifting, and the rest of the text as written.
    pub text: String,
    /// The byte ranges of `text` that `lift` wrote, sorted and disjoint: the stages of each
    /// lifting, and each copy of the text of the query in a new place, such as the operators before
    /// a lifted `AGGREGATE` in the target of a join.
    pub generated: Vec<Range<usize>>,
}

/// The query `text` with each pipe `AGGREGATE` whose mark calls lift replaced by the stages of
/// their lifting, in pipe form, with the named windows of the mark resolved and the options as
/// written, so that the text is avenger-sql that a user could have typed, names no function of
/// the passes, and runs to the same charts. The rest of the text stays as written, its layout and
/// its comments included, so a query with nothing to lift comes back unchanged, and `lift` lays
/// out the text that it writes. It plans the input of each `AGGREGATE` for its schema.
pub async fn lift(ctx: &SessionContext, text: &str) -> Result<Lifted> {
    let state = ctx.state();
    let mut text = text.to_string();
    // Whether `lift` wrote each byte of the text.
    let mut written = vec![false; text.len()];
    loop {
        let mut statement = pipeline::parse_tokens(&text)?;
        syntax::check_marks(&state, &mut statement)?;
        let ControlFlow::Break(found) =
            Visit::visit(&statement, &mut FirstMarker { registry: &state })
        else {
            break;
        };
        let (prefix, operator, mut chart) = found?;
        chart.check_aggregates(&state)?;
        let mut taken = words(&text);
        chart.name_parts(|base| Ident::new(fresh(base, &mut taken)));
        let mut lowered = prefix.clone();
        lowered.pipe_operators.extend(chart.extend.clone());
        syntax::lower_query(&state, &mut lowered)?;
        let plan = state
            .statement_to_plan(DFStatement::Statement(Box::new(Statement::Query(
                Box::new(lowered),
            ))))
            .await?;
        let schema = plan.schema().as_ref().clone();
        let mut liftings = Vec::new();
        for mark in &chart.marks {
            liftings.push(lifting::generate(
                &mut LiftEnv { state: &state },
                &mark.mark,
                &chart.groups,
                &mark.name,
                &schema,
            )?);
        }
        let source = text::Source::new(&text)?;
        let range = source.operator(&operator)?;
        let mut input = text[source.query_start(&prefix)?..range.start]
            .trim_end()
            .to_string();
        if text::ends_in_comment(&input) {
            input.push('\n');
        }
        let stages = stages(&chart, liftings, &input, &mut taken);
        let indent = source.indent(range.start).to_string();
        let stages = layout::lay_out(&stages).replace('\n', &format!("\n{indent}"));
        written.splice(range.clone(), std::iter::repeat_n(true, stages.len()));
        text.replace_range(range, &stages);
    }
    Ok(Lifted {
        generated: text::ranges(&written),
        text,
    })
}

/// The text of the stages of the lifting of `chart`, whose input is the text `input`: the
/// operators of the lifting, or with aggregate items or several charts, the join of the first
/// relation, the aggregate items or the first chart, and the liftings of the others, each over a
/// copy of the input, so the qualifiers of its tables stay valid.
fn stages(
    chart: &ChartAggregate,
    liftings: Vec<Vec<PipeOperator>>,
    input: &str,
    taken: &mut std::collections::HashSet<String>,
) -> String {
    let operators = |operators: &[PipeOperator]| -> String {
        (operators.iter())
            .map(|operator| {
                let mut operator = operator.clone();
                grammar::as_written(&mut operator);
                format!("|> {operator}")
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    let extend: Vec<PipeOperator> = chart.extend.clone().into_iter().collect();
    let join = |parts: Vec<String>| {
        (parts.into_iter())
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    };
    if !chart.joins() {
        let all: Vec<PipeOperator> = extend
            .into_iter()
            .chain(liftings.into_iter().flatten())
            .collect();
        return operators(&all);
    }
    let mut fresh = |base: &str| Ident::new(fresh(base, taken));
    let aggregates = match chart.groups_first() {
        true => fresh("aggregates"),
        false => Ident::new("aggregates"),
    };
    let charts: Vec<Ident> = liftings.iter().map(|_| fresh("charts")).collect();
    let mut liftings = liftings.into_iter();
    let first = match chart.groups_first() {
        true => vec![chart.aggregate()],
        false => liftings.next().expect("a chart"),
    };
    let first_name = chart.first(&aggregates, &charts).clone();
    let mut parts = vec![
        operators(&extend),
        operators(&first),
        format!("|> AS {first_name}"),
    ];
    for (lifting, name) in liftings.zip(chart.joined(&charts)) {
        let target = join(vec![
            input.to_string(),
            operators(&extend),
            operators(&lifting),
        ]);
        parts.push(format!(
            "|> LEFT OUTER JOIN ({target}) AS {name} ON {}",
            chart.join_condition(&first_name, name)
        ));
    }
    let projection: Vec<String> = (chart.projection(&aggregates, &charts).iter())
        .map(ToString::to_string)
        .collect();
    parts.push(format!("|> SELECT {}", projection.join(", ")));
    join(parts)
}

/// The index of the first `AGGREGATE` of `query` whose mark calls lift.
fn marker(query: &Query, registry: &dyn Registry) -> Option<usize> {
    (query.pipe_operators.iter()).position(|operator| syntax::lifts(operator, registry))
}

/// Breaks at the first query with an `AGGREGATE` whose mark calls lift, with the prefix before it,
/// the operator, and its charts.
struct FirstMarker<'a> {
    registry: &'a dyn Registry,
}

impl Visitor for FirstMarker<'_> {
    type Break = Result<(Query, PipeOperator, ChartAggregate)>;

    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<Self::Break> {
        let Some(index) = marker(query, self.registry) else {
            return ControlFlow::Continue(());
        };
        let mut prefix = query.clone();
        let operator = (prefix.pipe_operators.drain(index..).next()).expect("the operator");
        ControlFlow::Break(
            syntax::read_charts(&operator, self.registry)
                .map(|chart| (prefix, operator.clone(), chart)),
        )
    }
}

/// The words of `text`, in lowercase.
fn words(text: &str) -> std::collections::HashSet<String> {
    let dialect = datafusion::sql::sqlparser::dialect::GenericDialect {};
    (Tokenizer::new(&dialect, text)
        .tokenize()
        .unwrap_or_default()
        .into_iter())
    .filter_map(|token| match token {
        Token::Word(word) => Some(word.value.to_lowercase()),
        _ => None,
    })
    .collect()
}

/// The name `base`, or `base` with the smallest free suffix `_2`, `_3`, and so on, that `taken`
/// does not hold, which it then holds.
fn fresh(base: &str, taken: &mut std::collections::HashSet<String>) -> String {
    let name = std::iter::once(base.to_string())
        .chain((2..).map(|n| format!("{base}_{n}")))
        .find(|name| !taken.contains(name))
        .expect("a free name");
    taken.insert(name.clone());
    name
}
