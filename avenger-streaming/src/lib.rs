//! Streaming queries over the built-in `events` table: a query that [`avenger_sql::streaming`]
//! splits runs each frame's events through its stages once, applies the rows that they keep to its
//! front, and runs the rest of the query as a batch plan over the front's output, only on frames
//! where that output changed. After every frame, the result equals the batch plan of the whole
//! query over every event so far.
//!
//! The fronts take rows that only grow, so neither needs a retraction:
//!
//! - an `AGGREGATE` keeps DataFusion's accumulators of each group, which only add rows;
//! - an `ORDER BY ... LIMIT` keeps the first rows of its order, at most its skip and its fetch.

mod front;

use std::sync::Arc;

use avenger_sql::streaming::Split;
use datafusion::{
    arrow::{array::RecordBatch, datatypes::SchemaRef},
    common::{
        ParamValues, Result,
        tree_node::{Transformed, TreeNode, TreeNodeRecursion},
    },
    datasource::{MemTable, provider_as_source},
    logical_expr::{Expr, LogicalPlan, LogicalPlanBuilder},
    prelude::SessionContext,
};

use front::FrontState;

/// A query over `events` that runs incrementally, frame by frame.
pub struct StreamingQuery {
    ctx: SessionContext,
    split: Split,
    /// The table of the events of a frame, which the stages read.
    frame: Arc<MemTable>,
    events_schema: SchemaRef,
    /// The stages and the downstream, bound to the parameters and optimized.
    stages: LogicalPlan,
    downstream: LogicalPlan,
    front: FrontState,
    params: Option<ParamValues>,
    /// Every event so far, which a change of a parameter of the stages or the front replays.
    events: Vec<RecordBatch>,
    result: Vec<RecordBatch>,
}

impl StreamingQuery {
    /// The streaming query of `split` with the values of its parameters, whose result is the
    /// result over no events.
    pub async fn new(
        ctx: SessionContext,
        split: Split,
        params: Option<ParamValues>,
    ) -> Result<StreamingQuery> {
        let events_schema = Arc::new(split.scan.projected_schema.as_arrow().clone());
        let frame = Arc::new(MemTable::try_new(events_schema.clone(), vec![vec![]])?);
        let mut query = StreamingQuery {
            ctx,
            split,
            frame,
            events_schema,
            stages: LogicalPlan::default(),
            downstream: LogicalPlan::default(),
            front: FrontState::Empty,
            params,
            events: Vec::new(),
            result: Vec::new(),
        };
        query.prepare(true)?;
        query.run_downstream().await?;
        Ok(query)
    }

    /// The result after the frames so far.
    pub fn result(&self) -> &[RecordBatch] {
        &self.result
    }

    /// Applies the events of a frame, in the order of `seq`, after the events before it, and
    /// returns whether the result changed.
    pub async fn push(&mut self, events: RecordBatch) -> Result<bool> {
        if events.num_rows() == 0 {
            return Ok(false);
        }
        self.events.push(events.clone());
        self.apply(events).await
    }

    /// Binds the query's parameters, as of a script's variables. A parameter that the stages or
    /// the front read rebuilds the front from every event so far; one that only the downstream
    /// reads runs the downstream again.
    pub async fn set_params(&mut self, params: ParamValues) -> Result<()> {
        self.params = Some(params);
        let rebuild = reads_params(&self.split.stages) || self.split.front.reads_params();
        self.prepare(rebuild)?;
        if rebuild {
            for events in self.events.clone() {
                self.apply_front(events).await?;
            }
        }
        self.run_downstream().await
    }

    /// The plans of the parameters, and with `front`, a new front.
    fn prepare(&mut self, front: bool) -> Result<()> {
        let bind = |plan: &LogicalPlan, params: &Option<ParamValues>| match params {
            Some(params) => plan.clone().with_param_values(params.clone()),
            None => Ok(plan.clone()),
        };
        let scan = LogicalPlanBuilder::scan(
            self.split.scan.table_name.clone(),
            provider_as_source(self.frame.clone()),
            None,
        )?
        .build()?;
        let original = LogicalPlan::TableScan(self.split.scan.clone());
        let stages = (self.split.stages.clone())
            .transform_down(|p| match p == original {
                true => Ok(Transformed::new(
                    scan.clone(),
                    true,
                    TreeNodeRecursion::Jump,
                )),
                false => Ok(Transformed::no(p)),
            })?
            .data;
        let state = self.ctx.state();
        let stages = bind(&stages, &self.params)?;
        if front {
            self.front = FrontState::new(
                &self.split.front,
                stages.schema(),
                &self.params,
                state.execution_props(),
            )?;
        }
        self.stages = state.optimize(&stages)?;
        self.downstream = state.optimize(&bind(&self.split.downstream, &self.params)?)?;
        Ok(())
    }

    async fn apply(&mut self, events: RecordBatch) -> Result<bool> {
        if !self.apply_front(events).await? {
            return Ok(false);
        }
        self.run_downstream().await?;
        Ok(true)
    }

    /// Runs `events` through the stages and the front, and returns whether the front's output
    /// changed.
    async fn apply_front(&mut self, events: RecordBatch) -> Result<bool> {
        let events = RecordBatch::try_new(self.events_schema.clone(), events.columns().to_vec())?;
        *self.frame.batches[0].write().await = vec![events];
        let rows: Vec<RecordBatch> = (execute(&self.ctx, &self.stages).await?.into_iter())
            .filter(|batch| batch.num_rows() > 0)
            .collect();
        let mut changed = false;
        for batch in rows {
            changed |= self.front.apply(&batch)?;
        }
        Ok(changed)
    }

    async fn run_downstream(&mut self) -> Result<()> {
        self.split.slot.set(self.front.output()?);
        self.result = execute(&self.ctx, &self.downstream).await?;
        Ok(())
    }
}

/// Runs the physical plan of the optimized `plan`.
async fn execute(ctx: &SessionContext, plan: &LogicalPlan) -> Result<Vec<RecordBatch>> {
    let state = ctx.state();
    let physical = state
        .query_planner()
        .create_physical_plan(plan, &state)
        .await?;
    datafusion::physical_plan::collect(physical, state.task_ctx()).await
}

/// Whether `plan` reads a parameter.
fn reads_params(plan: &LogicalPlan) -> bool {
    let mut found = false;
    let _ = plan.apply(|node| {
        let _ = node.apply_expressions(|expr| {
            found |= expr_reads_params(expr);
            Ok(TreeNodeRecursion::Continue)
        });
        Ok(TreeNodeRecursion::Continue)
    });
    found
}

fn expr_reads_params(expr: &Expr) -> bool {
    expr.exists(|e| Ok(matches!(e, Expr::Placeholder(_))))
        .unwrap_or(false)
}

trait ReadsParams {
    fn reads_params(&self) -> bool;
}

impl ReadsParams for avenger_sql::streaming::Front {
    fn reads_params(&self) -> bool {
        use avenger_sql::streaming::Front;
        match self {
            Front::Aggregate { groups, aggregates } => {
                groups.iter().chain(aggregates).any(expr_reads_params)
            }
            Front::Limit { order, .. } => order.iter().any(|key| expr_reads_params(&key.expr)),
        }
    }
}
