//! Queries over the events of a chart, which run frame by frame.
//!
//! An [`EventLog`] holds a chart's events in the order of arrival, numbered in the column `seq`,
//! and is the table `events` of avenger-sql's catalog. [`EventQueries`] holds the queries over the
//! log: each frame's events go to the log and to every query, and each query's result equals its
//! batch plan over every event so far.
//!
//! A query that [`avenger_sql::streaming::split`] splits runs incrementally: each frame's events
//! pass through its stages once, the rows that they keep update its front, and the rest of the
//! query runs as a batch plan over the front's output, only on frames where that output changed.
//! The fronts take rows that only grow, so neither needs a retraction:
//!
//! - an `AGGREGATE` keeps DataFusion's accumulators of each group, which only add rows;
//! - an `ORDER BY ... LIMIT` keeps the first rows of its order, at most its skip and its fetch.
//!
//! Another query runs as its batch plan over the log on each frame with events.

mod front;
mod log;
mod matching;
mod query;

use std::collections::{BTreeSet, HashMap};

use avenger_sql::streaming::split;
use datafusion::{
    arrow::array::RecordBatch,
    common::{
        ParamValues, Result, ScalarValue, plan_err,
        tree_node::{TreeNode, TreeNodeRecursion},
    },
    logical_expr::{Expr, LogicalPlan},
    prelude::SessionContext,
};

pub use avenger_sql::streaming::Rejection;
pub use log::{EventLog, SEQ};

use query::StreamingQuery;

/// The name of a query of [`EventQueries`], such as the variable or the chart that it feeds.
pub type QueryId = String;

/// How a query of [`EventQueries`] runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// Incrementally: its cost of a frame doesn't grow with the log.
    Streaming,
    /// As its batch plan over the whole log on each frame with events, for the reason that the
    /// rejection gives.
    Batch(Rejection),
}

/// The queries over the events of one log, which run frame by frame.
pub struct EventQueries {
    ctx: SessionContext,
    log: EventLog,
    /// The queries, in the order of their registration.
    queries: Vec<Entry>,
    /// The values of the parameters, by their names without `$`.
    params: HashMap<String, ScalarValue>,
}

struct Entry {
    id: QueryId,
    mode: Mode,
    /// The names of the parameters that the query reads.
    reads: BTreeSet<String>,
    run: Run,
}

enum Run {
    Streaming(Box<StreamingQuery>),
    Batch(Box<BatchQuery>),
}

/// A query that runs as its batch plan.
struct BatchQuery {
    plan: LogicalPlan,
    /// The plan bound to the parameters, which each run optimizes, so a function of the clock
    /// reads the time of the run.
    bound: LogicalPlan,
    result: Vec<RecordBatch>,
}

impl EventQueries {
    /// The queries over `log`, which run in `ctx`, a session with avenger-sql's functions
    /// (`avenger_sql::register`).
    pub fn new(ctx: SessionContext, log: EventLog) -> EventQueries {
        EventQueries {
            ctx,
            log,
            queries: Vec::new(),
            params: HashMap::new(),
        }
    }

    /// The log of the queries' events.
    pub fn log(&self) -> &EventLog {
        &self.log
    }

    /// Adds the query `plan` under `id`, a plan of avenger-sql whose catalog's `events` is the
    /// log's table. The query runs incrementally when it splits, and otherwise as its batch plan,
    /// and its result is current over the log's events when this returns. The parameters that it
    /// reads need values first, from [`EventQueries::set_params`].
    pub async fn register(&mut self, id: impl Into<QueryId>, plan: LogicalPlan) -> Result<Mode> {
        let id = id.into();
        if self.queries.iter().any(|entry| entry.id == id) {
            return plan_err!("a query is registered as `{id}`");
        }
        let reads = param_names(&plan);
        if let Some(missing) = reads.iter().find(|name| !self.params.contains_key(*name)) {
            return plan_err!(
                "the query `{id}` reads the parameter `{missing}`, which has no value"
            );
        }
        let params = self.param_values(&reads);
        let (mode, run) = match split(&plan, &self.log.table()) {
            Ok(split) => {
                let query =
                    StreamingQuery::new(self.ctx.clone(), self.log.clone(), split, params).await?;
                (Mode::Streaming, Run::Streaming(Box::new(query)))
            }
            Err(rejection) => {
                let bound = self.bind(&plan, &params)?;
                let result = run_batch(&self.ctx, &bound).await?;
                let run = Run::Batch(Box::new(BatchQuery {
                    plan,
                    bound,
                    result,
                }));
                (Mode::Batch(rejection), run)
            }
        };
        self.queries.push(Entry {
            id,
            mode: mode.clone(),
            reads,
            run,
        });
        Ok(mode)
    }

    /// Removes the query `id`, and returns whether it was registered.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.queries.len();
        self.queries.retain(|entry| entry.id != id);
        self.queries.len() != before
    }

    /// How the query `id` runs.
    pub fn mode(&self, id: &str) -> Option<&Mode> {
        self.entry(id).map(|entry| &entry.mode)
    }

    /// The current result of the query `id`: its batch plan's rows over every event so far.
    pub fn result(&self, id: &str) -> Option<&[RecordBatch]> {
        self.entry(id).map(|entry| match &entry.run {
            Run::Streaming(query) => query.result(),
            Run::Batch(query) => query.result.as_slice(),
        })
    }

    /// Appends a frame's events to the log, with the columns of [`EventLog::input_schema`], and
    /// applies them to every query. Returns the queries whose result changed, in the order of
    /// their registration.
    pub async fn push(&mut self, events: RecordBatch) -> Result<Vec<QueryId>> {
        if events.num_rows() == 0 {
            return Ok(Vec::new());
        }
        self.log.append(events).await?;
        let mut changed = Vec::new();
        for entry in &mut self.queries {
            let differs = match &mut entry.run {
                Run::Streaming(query) => query.catch_up().await?,
                Run::Batch(query) => {
                    let rows = run_batch(&self.ctx, &query.bound).await?;
                    let differs = rows != query.result;
                    query.result = rows;
                    differs
                }
            };
            if differs {
                changed.push(entry.id.clone());
            }
        }
        Ok(changed)
    }

    /// Sets the values of parameters, by their names without `$`, over the values so far. Each
    /// query that reads a parameter whose value changed rebuilds its front from the log, or runs
    /// the rest of its plan again. Returns the queries whose result changed.
    pub async fn set_params(
        &mut self,
        values: impl IntoIterator<Item = (String, ScalarValue)>,
    ) -> Result<Vec<QueryId>> {
        let mut changed_names = BTreeSet::new();
        for (name, value) in values {
            let name = name.trim_start_matches('$').to_string();
            if self.params.get(&name) != Some(&value) {
                self.params.insert(name.clone(), value);
                changed_names.insert(name);
            }
        }
        let mut changed = Vec::new();
        for index in 0..self.queries.len() {
            if self.queries[index].reads.is_disjoint(&changed_names) {
                continue;
            }
            let params = self.param_values(&self.queries[index].reads);
            let bound = match &self.queries[index].run {
                Run::Batch(query) => Some(self.bind(&query.plan, &params)?),
                Run::Streaming(_) => None,
            };
            let entry = &mut self.queries[index];
            let differs = match (&mut entry.run, bound) {
                (Run::Streaming(query), _) => match params {
                    Some(params) => query.set_params(params).await?,
                    None => false,
                },
                (Run::Batch(query), Some(rebound)) => {
                    query.bound = rebound;
                    let rows = run_batch(&self.ctx, &query.bound).await?;
                    let differs = rows != query.result;
                    query.result = rows;
                    differs
                }
                (Run::Batch(_), None) => unreachable!("a batch query's plan is bound"),
            };
            if differs {
                changed.push(entry.id.clone());
            }
        }
        Ok(changed)
    }

    fn entry(&self, id: &str) -> Option<&Entry> {
        self.queries.iter().find(|entry| entry.id == id)
    }

    /// The values of the parameters `names`, or `None` without parameters.
    fn param_values(&self, names: &BTreeSet<String>) -> Option<ParamValues> {
        if names.is_empty() {
            return None;
        }
        let values = (names.iter())
            .filter_map(|name| {
                let value = self.params.get(name)?;
                Some((name.clone(), value.clone().into()))
            })
            .collect();
        Some(ParamValues::Map(values))
    }

    /// `plan` bound to `params`.
    fn bind(&self, plan: &LogicalPlan, params: &Option<ParamValues>) -> Result<LogicalPlan> {
        match params {
            Some(params) => plan.clone().with_param_values(params.clone()),
            None => Ok(plan.clone()),
        }
    }
}

/// Optimizes and runs `plan`, a batch plan.
async fn run_batch(ctx: &SessionContext, plan: &LogicalPlan) -> Result<Vec<RecordBatch>> {
    ctx.execute_logical_plan(plan.clone())
        .await?
        .collect()
        .await
}

/// Runs the physical plan of the optimized `plan`.
pub(crate) async fn execute(ctx: &SessionContext, plan: &LogicalPlan) -> Result<Vec<RecordBatch>> {
    let state = ctx.state();
    let physical = state
        .query_planner()
        .create_physical_plan(plan, &state)
        .await?;
    datafusion::physical_plan::collect(physical, state.task_ctx()).await
}

/// The names of the parameters that `plan` reads, in its subqueries too, without `$`.
pub(crate) fn param_names(plan: &LogicalPlan) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let _ = plan.apply_with_subqueries(|node| {
        let _ = node.apply_expressions(|expr| {
            let _ = expr.apply(|e| {
                if let Expr::Placeholder(placeholder) = e {
                    names.insert(placeholder.id.trim_start_matches('$').to_string());
                }
                Ok(TreeNodeRecursion::Continue)
            });
            Ok(TreeNodeRecursion::Continue)
        });
        Ok(TreeNodeRecursion::Continue)
    });
    names
}
