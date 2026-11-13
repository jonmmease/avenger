//! A streaming query: the split of a query over the log's events, which reads the log's events
//! after the ones that it applied.

use std::sync::Arc;

use avenger_sql::streaming::{Front, Split};
use datafusion::{
    arrow::{array::RecordBatch, compute::concat_batches, datatypes::SchemaRef},
    common::{
        ParamValues, Result,
        tree_node::{Transformed, TreeNode, TreeNodeRecursion},
    },
    datasource::{MemTable, provider_as_source},
    logical_expr::{Expr, LogicalPlan, LogicalPlanBuilder},
    prelude::SessionContext,
};

use crate::{EventLog, execute, front::FrontState};

/// The most events that a catch-up runs through the stages at once.
const CHUNK_ROWS: usize = 8_192;

/// A query over the log's events that runs incrementally.
pub(crate) struct StreamingQuery {
    ctx: SessionContext,
    log: EventLog,
    split: Split,
    /// The table of the events of a batch of the log, which the stages read.
    frame: Arc<MemTable>,
    events_schema: SchemaRef,
    /// The stages and the downstream, bound to the parameters and optimized.
    stages: LogicalPlan,
    downstream: LogicalPlan,
    front: FrontState,
    params: Option<ParamValues>,
    /// The number of the log's events that the front holds.
    applied: usize,
    /// The downstream as the latest rows of the front's output, and the rows that it keeps.
    latest: Option<LatestState>,
    result: Vec<RecordBatch>,
}

/// The pieces of [`avenger_sql::streaming::Latest`], bound to the parameters and optimized, and
/// the first rows of the stages' rows of the front's final rows, in the downstream's order.
struct LatestState {
    stages: LogicalPlan,
    top: LogicalPlan,
    rest: LogicalPlan,
    kept: Vec<RecordBatch>,
}

impl StreamingQuery {
    /// The streaming query of `split` with the values of its parameters, over the log's events so
    /// far.
    pub(crate) async fn new(
        ctx: SessionContext,
        log: EventLog,
        split: Split,
        params: Option<ParamValues>,
    ) -> Result<StreamingQuery> {
        let events_schema = Arc::new(split.scan.projected_schema.as_arrow().clone());
        let frame = Arc::new(MemTable::try_new(events_schema.clone(), vec![vec![]])?);
        let mut query = StreamingQuery {
            ctx,
            log,
            split,
            frame,
            events_schema,
            stages: LogicalPlan::default(),
            downstream: LogicalPlan::default(),
            front: FrontState::Empty,
            params,
            applied: 0,
            latest: None,
            result: Vec::new(),
        };
        query.prepare(true)?;
        query.apply_log().await?;
        query.run_downstream().await?;
        Ok(query)
    }

    /// The result over the log's events that the query applied.
    pub(crate) fn result(&self) -> &[RecordBatch] {
        &self.result
    }

    /// Applies the log's events after the ones that the query applied, and returns whether the
    /// result changed.
    pub(crate) async fn catch_up(&mut self) -> Result<bool> {
        if !self.apply_log().await? {
            return Ok(false);
        }
        self.rerun_downstream().await
    }

    /// Binds the query's parameters. A parameter that the stages or the front read rebuilds the
    /// front from the log's events; one that only the downstream reads runs the downstream
    /// again. Returns whether the result changed.
    pub(crate) async fn set_params(&mut self, params: ParamValues) -> Result<bool> {
        self.params = Some(params);
        let rebuild = reads_params(&self.split.stages) || front_reads_params(&self.split.front);
        self.prepare(rebuild)?;
        if rebuild {
            self.applied = 0;
            self.apply_log().await?;
        }
        self.rerun_downstream().await
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
                &self.ctx,
                &self.split.front,
                stages.schema(),
                &self.params,
                state.execution_props(),
            )?;
        }
        self.stages = state.optimize(&stages)?;
        self.downstream = state.optimize(&bind(&self.split.downstream, &self.params)?)?;
        // The kept rows follow the front's final rows and the downstream's parameters, so they
        // start again from every final row.
        self.latest = match &self.split.latest {
            Some(latest) => Some(LatestState {
                stages: state.optimize(&bind(&latest.stages, &self.params)?)?,
                top: state.optimize(&bind(&latest.top, &self.params)?)?,
                rest: state.optimize(&bind(&latest.rest, &self.params)?)?,
                kept: Vec::new(),
            }),
            None => None,
        };
        if let FrontState::Match(front) = &mut self.front {
            front.give_finals_again();
        }
        Ok(())
    }

    /// Runs the log's events after the applied ones through the stages and the front, and
    /// returns whether the front's output changed.
    async fn apply_log(&mut self) -> Result<bool> {
        let batches = self.log.events_after(self.applied).await;
        if batches.is_empty() {
            return Ok(false);
        }
        // The stages map each row on its own and the fronts read rows in order, so a catch-up
        // over many batches runs them in chunks. The cursor passes a chunk once it applied, so
        // an event that fails fails again on the next frame, as the batch plan does.
        let mut changed = false;
        let mut chunk: Vec<RecordBatch> = Vec::new();
        let mut rows = 0;
        let count = batches.len();
        for (index, events) in batches.into_iter().enumerate() {
            rows += events.num_rows();
            chunk.push(RecordBatch::try_new(
                self.events_schema.clone(),
                events.columns().to_vec(),
            )?);
            if rows >= CHUNK_ROWS || index + 1 == count {
                let events = concat_batches(&self.events_schema, &chunk)?;
                chunk.clear();
                rows = 0;
                let count = events.num_rows();
                *self.frame.batches[0].write().await = vec![events];
                for kept in execute(&self.ctx, &self.stages).await? {
                    if kept.num_rows() > 0 {
                        changed |= self.front.apply(&kept).await?;
                    }
                }
                self.applied += count;
            }
        }
        Ok(changed)
    }

    /// Runs the downstream, and returns whether the result changed.
    async fn rerun_downstream(&mut self) -> Result<bool> {
        // A downstream that fails keeps the last result.
        let before = self.result.clone();
        self.run_downstream().await?;
        Ok(self.result != before)
    }

    async fn run_downstream(&mut self) -> Result<()> {
        if let (Some(latest), FrontState::Match(front), Some(pieces)) =
            (&mut self.latest, &mut self.front, &self.split.latest)
        {
            // The final rows since the last frame join the kept rows, and the open rows join a
            // copy of them, whose first rows the rest of the downstream reads.
            let finals = front.take_new_finals();
            if !finals.is_empty() {
                let mut candidates = latest.kept.clone();
                candidates
                    .extend(staged(&self.ctx, &self.split.slot, &latest.stages, &finals).await?);
                latest.kept = top(&self.ctx, &pieces.candidates, &latest.top, &candidates).await?;
            }
            let mut candidates = latest.kept.clone();
            let open = front.open_rows();
            candidates.extend(staged(&self.ctx, &self.split.slot, &latest.stages, &open).await?);
            let first = top(&self.ctx, &pieces.candidates, &latest.top, &candidates).await?;
            let rows = concat(&first)?.map(|rows| {
                let skip = pieces.skip.min(rows.num_rows());
                rows.slice(skip, rows.num_rows() - skip)
            });
            match rows {
                Some(rows) => pieces.limited.set(rows.columns().to_vec()),
                None => pieces.limited.set_empty(),
            }
            self.result = execute(&self.ctx, &latest.rest).await?;
            return Ok(());
        }
        self.split.slot.set(self.front.output()?);
        self.result = execute(&self.ctx, &self.downstream).await?;
        Ok(())
    }
}

/// The rows of `batches` as one batch, or `None` without rows.
fn concat(batches: &[RecordBatch]) -> Result<Option<RecordBatch>> {
    let batches: Vec<&RecordBatch> = batches.iter().filter(|b| b.num_rows() > 0).collect();
    match batches.first() {
        None => Ok(None),
        Some(first) => Ok(Some(concat_batches(
            &first.schema(),
            batches.iter().copied(),
        )?)),
    }
}

/// The rows of `stages` over `rows` of the front's output, which `slot` holds.
async fn staged(
    ctx: &SessionContext,
    slot: &avenger_sql::streaming::Slot,
    stages: &LogicalPlan,
    rows: &[RecordBatch],
) -> Result<Vec<RecordBatch>> {
    let Some(rows) = concat(rows)? else {
        return Ok(Vec::new());
    };
    slot.set(rows.columns().to_vec());
    execute(ctx, stages).await
}

/// The first rows of `rows` in the order of `top`, which reads them through `slot`.
async fn top(
    ctx: &SessionContext,
    slot: &avenger_sql::streaming::Slot,
    top: &LogicalPlan,
    rows: &[RecordBatch],
) -> Result<Vec<RecordBatch>> {
    let Some(rows) = concat(rows)? else {
        return Ok(Vec::new());
    };
    slot.set(rows.columns().to_vec());
    execute(ctx, top).await
}

/// Whether `plan` reads a parameter.
fn reads_params(plan: &LogicalPlan) -> bool {
    !crate::param_names(plan).is_empty()
}

fn expr_reads_params(expr: &Expr) -> bool {
    expr.exists(|e| Ok(matches!(e, Expr::Placeholder(_))))
        .unwrap_or(false)
}

fn front_reads_params(front: &Front) -> bool {
    match front {
        Front::Aggregate { groups, aggregates } => {
            groups.iter().chain(aggregates).any(expr_reads_params)
        }
        Front::Limit { order, .. } => order.iter().any(|key| expr_reads_params(&key.expr)),
        Front::MatchRecognize(front) => {
            reads_params(&front.conditions)
                || reads_params(&front.measures)
                || front.partition.iter().any(expr_reads_params)
        }
    }
}
