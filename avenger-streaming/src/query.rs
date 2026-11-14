//! A streaming query: the split of a query over the log's events, whose fronts each read the
//! log's events after the ones that they applied.

use std::sync::Arc;

use avenger_sql::streaming::{Front, FrontSplit, Split};
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
    /// The fronts, in the order of the split's fronts.
    fronts: Vec<FrontRun>,
    /// The table of the events of a batch of the log, which every front's stages read.
    frame: Arc<MemTable>,
    events_schema: SchemaRef,
    /// The downstream, bound to the parameters and optimized.
    downstream: LogicalPlan,
    params: Option<ParamValues>,
    /// The downstream as the latest rows of the front's output, and the rows that it keeps.
    latest: Option<LatestState>,
    result: Vec<RecordBatch>,
}

/// A front of the query and the stages over its read of `events`.
struct FrontRun {
    /// The stages over the frame, bound to the parameters and optimized.
    stages: LogicalPlan,
    state: FrontState,
    /// Whether the stages or the front read a parameter.
    reads_params: bool,
    /// The number of the log's events that the front holds.
    applied: usize,
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
        let events_schema = Arc::new(split.fronts[0].scan.projected_schema.as_arrow().clone());
        let frame = Arc::new(MemTable::try_new(events_schema.clone(), vec![vec![]])?);
        let fronts = (split.fronts.iter())
            .map(|front| FrontRun {
                stages: LogicalPlan::default(),
                state: FrontState::Empty,
                reads_params: reads_params(&front.stages) || front_reads_params(&front.front),
                applied: 0,
            })
            .collect();
        let mut query = StreamingQuery {
            ctx,
            log,
            split,
            fronts,
            frame,
            events_schema,
            downstream: LogicalPlan::default(),
            params,
            latest: None,
            result: Vec::new(),
        };
        query.prepare(|_| true)?;
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

    /// Binds the query's parameters. A parameter that a front or its stages read rebuilds that
    /// front from the log's events; one that only the downstream reads runs the downstream
    /// again. Returns whether the result changed.
    pub(crate) async fn set_params(&mut self, params: ParamValues) -> Result<bool> {
        self.params = Some(params);
        self.prepare(|front| front.reads_params)?;
        self.apply_log().await?;
        self.rerun_downstream().await
    }

    /// The plans of the parameters, and a new front for each front that `rebuild` takes.
    fn prepare(&mut self, rebuild: impl Fn(&FrontRun) -> bool) -> Result<()> {
        let state = self.ctx.state();
        for (run, front) in self.fronts.iter_mut().zip(&self.split.fronts) {
            if !rebuild(run) {
                continue;
            }
            let stages = bind(&frame_stages(front, &self.frame)?, &self.params)?;
            run.state = FrontState::new(
                &self.ctx,
                &front.front,
                stages.schema(),
                &self.params,
                state.execution_props(),
            )?;
            run.stages = state.optimize(&stages)?;
            run.applied = 0;
        }
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
        for run in &mut self.fronts {
            if let FrontState::Match(front) = &mut run.state {
                front.give_finals_again();
            }
        }
        Ok(())
    }

    /// Runs the log's events after the applied ones through each front's stages and the front,
    /// and returns whether a front's output changed.
    async fn apply_log(&mut self) -> Result<bool> {
        let mut changed = false;
        for run in &mut self.fronts {
            changed |=
                apply_front(&self.ctx, &self.log, &self.frame, &self.events_schema, run).await?;
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
        if let (
            Some(latest),
            [
                FrontRun {
                    state: FrontState::Match(front),
                    ..
                },
            ],
            Some(pieces),
            [split],
        ) = (
            &mut self.latest,
            self.fronts.as_mut_slice(),
            &self.split.latest,
            self.split.fronts.as_slice(),
        ) {
            // The final rows since the last frame join the kept rows, and the open rows join a
            // copy of them, whose first rows the rest of the downstream reads.
            let finals = front.take_new_finals();
            if !finals.is_empty() {
                let mut candidates = latest.kept.clone();
                candidates.extend(staged(&self.ctx, &split.slot, &latest.stages, &finals).await?);
                latest.kept = top(&self.ctx, &pieces.candidates, &latest.top, &candidates).await?;
            }
            let mut candidates = latest.kept.clone();
            let open = front.open_rows();
            candidates.extend(staged(&self.ctx, &split.slot, &latest.stages, &open).await?);
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
        for (run, front) in self.fronts.iter().zip(&self.split.fronts) {
            front.slot.set(run.state.output()?);
        }
        self.result = execute(&self.ctx, &self.downstream).await?;
        Ok(())
    }
}

/// Runs the log's events after the ones that `run` applied through its stages, which read them
/// from `frame`, and its front, and returns whether the front's output changed.
async fn apply_front(
    ctx: &SessionContext,
    log: &EventLog,
    frame: &MemTable,
    events_schema: &SchemaRef,
    run: &mut FrontRun,
) -> Result<bool> {
    let batches = log.events_after(run.applied).await;
    // The stages map each row on its own and the fronts read rows in order, so a catch-up over
    // many batches runs them in chunks. The cursor passes a chunk once it applied, so an event
    // that fails fails again on the next frame, as the batch plan does.
    let mut changed = false;
    let mut chunk: Vec<RecordBatch> = Vec::new();
    let mut rows = 0;
    let count = batches.len();
    for (index, events) in batches.into_iter().enumerate() {
        rows += events.num_rows();
        chunk.push(RecordBatch::try_new(
            events_schema.clone(),
            events.columns().to_vec(),
        )?);
        if rows >= CHUNK_ROWS || index + 1 == count {
            let events = concat_batches(events_schema, &chunk)?;
            chunk.clear();
            rows = 0;
            let count = events.num_rows();
            *frame.batches[0].write().await = vec![events];
            for kept in execute(ctx, &run.stages).await? {
                if kept.num_rows() > 0 {
                    changed |= run.state.apply(&kept).await?;
                }
            }
            run.applied += count;
        }
    }
    Ok(changed)
}

/// The stages of `front` over `frame` in place of `events`.
fn frame_stages(front: &FrontSplit, frame: &Arc<MemTable>) -> Result<LogicalPlan> {
    let scan = LogicalPlanBuilder::scan(
        front.scan.table_name.clone(),
        provider_as_source(frame.clone()),
        None,
    )?
    .build()?;
    let original = LogicalPlan::TableScan(front.scan.clone());
    let stages = (front.stages.clone())
        .transform_down(|p| match p == original {
            true => Ok(Transformed::new(
                scan.clone(),
                true,
                TreeNodeRecursion::Jump,
            )),
            false => Ok(Transformed::no(p)),
        })?
        .data;
    Ok(stages)
}

/// `plan` bound to `params`.
fn bind(plan: &LogicalPlan, params: &Option<ParamValues>) -> Result<LogicalPlan> {
    match params {
        Some(params) => plan.clone().with_param_values(params.clone()),
        None => Ok(plan.clone()),
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
